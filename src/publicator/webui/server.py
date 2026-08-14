"""The human-review gallery: a ThreadingHTTPServer on 127.0.0.1 plus its
JSON endpoints (/delete, /ai, /stage, /update, /publish)."""

import json
import logging
import os
import subprocess
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from publicator.config import validate_publications
from publicator.deviantart import publish_batch
from publicator.images import guess_mime
from publicator.llm_meta import DEFAULT_MODEL, generate_metadata
from publicator.scheduling import existing_ts as compute_existing_ts, schedule_data, zone
from publicator.store import apply_update, atomic_write_json, write_publications
from publicator.webui.page import render_page

log = logging.getLogger("publicator.gallery")


# ---------------------------------------------------------------------------
# HTTP: gallery + queue endpoints
# ---------------------------------------------------------------------------


class GalleryHandler(BaseHTTPRequestHandler):
    thumb_dir = ""
    thumb_map: dict[str, str] = {}
    candidate_paths: list[str] = []      # new picks from picked/, add-able
    pending: list[dict] = []             # already-queued entries from publications.json
    existing_ts: list[int] = []
    ai_model = DEFAULT_MODEL
    openrouter_model = ""
    ai_timeout = 300
    json_path = "publications.json"
    config: dict = {}
    schedules: list = []

    def _build_page(self) -> str:
        return render_page(
            thumb_dir=self.thumb_dir, thumb_map=self.thumb_map,
            candidates=self.candidate_paths, pending=self.pending,
            existing_ts=self.existing_ts, schedules=self.schedules,
            config=self.config, ai_model=self.ai_model,
            openrouter_model=self.openrouter_model)

    def _json_body(self) -> dict:
        length = int(self.headers.get("Content-length", 0))
        return json.loads(self.rfile.read(length) or b"{}")

    def _send(self, code: int, obj: dict) -> None:
        body = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-type", "application/json")
        self.send_header("Content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_file(self, path: str) -> None:
        if path and os.path.isfile(path):
            self.send_response(200)
            self.send_header("Content-type", guess_mime(path))
            self.end_headers()
            with open(path, "rb") as f:
                self.wfile.write(f.read())
        else:
            self.send_response(404); self.end_headers()

    def do_GET(self):
        if self.path in ("/", "/index.html"):
            page = self._build_page().encode("utf-8")
            self.send_response(200)
            self.send_header("Content-type", "text/html; charset=utf-8")
            self.send_header("Content-length", str(len(page)))
            self.end_headers()
            self.wfile.write(page)
        elif self.path.startswith("/thumbs/"):
            rel_path = self.path[len("/thumbs/"):]
            full_path = os.path.normpath(os.path.join(self.thumb_dir, rel_path))
            if full_path.startswith(os.path.normpath(self.thumb_dir)):
                self._send_file(full_path)
            else:
                self.send_response(404); self.end_headers()
        elif self.path.startswith("/original?"):
            # Full-res original for a card's thumbnail, opened in a new tab.
            # Allow-list = thumb_map keys (exactly the candidate + pending
            # originals); anything else 404s, so no arbitrary-path read.
            qs = urllib.parse.urlparse(self.path).query
            req_path = urllib.parse.parse_qs(qs).get("path", [""])[0]
            if req_path in self.thumb_map:
                self._send_file(req_path)
            else:
                self.send_response(404); self.end_headers()
        else:
            self.send_response(404); self.end_headers()

    def do_POST(self):
        try:
            if self.path == "/delete":
                data = self._json_body()
                path = data.get("path", "")
                try:
                    os.remove(path)
                except OSError as e:
                    self._send(200, {"ok": False, "error": str(e)}); return
                # ponytail: gallery renders once; concurrent refresh vs. delete
                # would race this dict, but in practice the tab is opened once.
                self.__class__.thumb_map.pop(path, None)
                self._send(200, {"ok": True})

            elif self.path == "/ai":
                data = self._json_body()
                path = data.get("path", "")
                model = data.get("model") or self.ai_model
                if model not in (self.ai_model, self.openrouter_model):
                    self._send(400, {"error": f"model not allowed: {model}"}); return
                # Send the thumbnail to Claude, not the full-res original — same
                # visual info for a fraction of the tokens/latency.
                image_for_ai = self.thumb_map.get(path, path)
                log.debug("AI metadata: %s (model=%s, timeout=%ss)",
                          image_for_ai, model, self.ai_timeout)
                try:
                    title, desc = generate_metadata(image_for_ai, model, timeout=self.ai_timeout)
                except RuntimeError as e:
                    log.debug("AI metadata failed: %s", e)
                    self._send(400, {"error": str(e)}); return
                self._send(200, {"title": title, "description": desc})

            elif self.path == "/publish":
                data = self._json_body()
                entries = data.get("entries", []) or []
                if not entries:
                    self._send(400, {"error": "empty queue"}); return
                # Entries with a uuid are already in publications.json (the
                # pre-queued pending set) — publish them as-is. The rest are new
                # picks: append them first, then publish. Existing go first.
                existing = [e for e in entries if e.get("uuid")]
                new = [e for e in entries if not e.get("uuid")]
                try:
                    new_uuids = (write_publications(new, self.json_path, self.config)
                                 if new else [])
                except Exception as e:
                    self._send(400, {"error": f"schema/write failed: {e}"}); return

                all_entries = existing + new
                all_uuids = [e["uuid"] for e in existing] + new_uuids
                published, failed, err = publish_batch(all_entries, all_uuids, self.json_path)
                result = {"published": published, "failed": failed}
                if err:
                    result["error"] = err
                self.server.publish_done = result
                self._send(200, result)

            elif self.path == "/stage":
                data = self._json_body()
                entries = data.get("entries", []) or []
                if not entries:
                    self._send(400, {"error": "nothing to stage"}); return
                try:
                    uuids = write_publications(entries, self.json_path, self.config)
                except Exception as e:
                    self._send(400, {"error": f"schema/write failed: {e}"}); return
                # No publish_done set → serve loop keeps running, session stays alive.
                self._send(200, {"staged": len(uuids), "uuids": uuids})

            elif self.path == "/update":
                data = self._json_body()
                u = data.get("uuid")
                if not u:
                    self._send(400, {"error": "missing uuid"}); return
                with open(self.json_path) as f:
                    pubs = json.load(f)
                try:
                    apply_update(pubs, u, data, zone(self.config.get("schedule", {})))
                except KeyError:
                    self._send(404, {"error": f"uuid not found: {u}"}); return
                try:
                    validate_publications(pubs, self.config)
                except Exception as e:
                    self._send(400, {"error": f"invalid: {e}"}); return
                atomic_write_json(self.json_path, pubs)
                self._send(200, {"ok": True})
            else:
                self.send_response(404); self.end_headers()
        except Exception as e:
            try:
                self._send(500, {"error": str(e)})
            except Exception:
                pass

    def log_message(self, format, *args):
        log.debug("http %s - %s", self.address_string(), format % args)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def serve(thumb_dir: str, thumb_map: dict[str, str], candidate_paths: list[str],
          pending: list[dict], args, config: dict) -> dict | None:
    GalleryHandler.thumb_dir = thumb_dir
    GalleryHandler.thumb_map = thumb_map
    GalleryHandler.candidate_paths = candidate_paths
    GalleryHandler.pending = pending
    GalleryHandler.existing_ts = compute_existing_ts(args.json)
    GalleryHandler.ai_model = args.ai_model
    GalleryHandler.openrouter_model = args.openrouter_model
    GalleryHandler.ai_timeout = args.ai_timeout
    GalleryHandler.json_path = args.json
    GalleryHandler.config = config
    GalleryHandler.schedules = schedule_data(config["schedule"])

    server = ThreadingHTTPServer(("127.0.0.1", args.port), GalleryHandler)
    server.publish_done = None
    server.timeout = 0.5
    url = f"http://127.0.0.1:{args.port}"
    print(f"Gallery: {url}")
    try:
        subprocess.Popen(
            ["firefox", "--private-window", "--no-remote", url],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
    except FileNotFoundError:
        print("(open the URL yourself; firefox not found)")

    try:
        while server.publish_done is None:
            server.handle_request()
    except KeyboardInterrupt:
        print("\nShutting down...")
    finally:
        server.server_close()
    return server.publish_done
