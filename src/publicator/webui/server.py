"""The human-review gallery: a ThreadingHTTPServer on 127.0.0.1 plus its
durable phase endpoints."""

import json
import logging
import os
import subprocess
import urllib.parse
import threading
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from jsonschema import ValidationError

from publicator.config import validate_publications
from publicator import platforms, store, workflow
from publicator.platforms import publish_selected
from publicator.images import (
    THUMB_CACHE,
    collect_images,
    compute_sha512,
    ensure_thumb,
    guess_mime,
    index_by_basename,
    load_publicated_hashes,
    Search,
    find_candidates,
    thumb_for_ai,
    thumb_name,
)
from publicator.llm_meta import (
    DEFAULT_MODEL, TODO_GENERATE, VISION_MODELS,
    generate_metadata, local_model_status,
)
from publicator.prompts import Exact, Nearest
from publicator.prompts import from_config as prompt_archive_from_config
from publicator.scheduling import schedule_data, zone
from publicator.webui.calendar_view import timeline
from publicator.webui.page import render_page
from publicator.webui.prompt_view import render_prompt

log = logging.getLogger("publicator.gallery")

# ponytail: a filtered view walks every publicable image; this caps how many
# survivors get sha512-hashed for the already-published check. Raise it if a
# lineage ever legitimately has more art than this.
SEARCH_LIMIT = 200


# ---------------------------------------------------------------------------
# HTTP: gallery + queue endpoints
# ---------------------------------------------------------------------------


class GalleryHandler(BaseHTTPRequestHandler):
    cache_dir = ""                       # <data-dir>/.thumbs, shared by both tabs
    thumb_map: dict[str, str] = {}       # art path -> cache name (gallery cards)
    thumb_src: dict[str, str] = {}       # cache name -> art path (what /thumbs may serve)
    candidate_paths: list[str] = []      # new picks from picked/, add-able
    data_dir = ""
    rendered_candidates: set[str] = set()
    search_generation = 0
    ai_busy: set[str] = set()
    operation_lock = threading.RLock()
    publish_lock = threading.Lock()
    timeline: list[dict] = []            # every stored apparition, for the calendar tab
    ai_model = DEFAULT_MODEL
    openrouter_model = ""
    ai_timeout = 300
    vision_models = VISION_MODELS
    local_model_statuses: dict[str, dict] = {}
    json_path = "publications.json"
    config: dict = {}
    archive = None                       # PromptArchive | None; None = feature off
    publicable_dirs: list[str] = []
    search_state = None

    def _prompt_html(self, paths) -> dict[str, str]:
        """{art path: prompt block} for everything this page will show — the
        gallery's cards AND the calendar's thumbnails, resolved once and shared,
        so a published image's prompt renders exactly like a candidate's.
        Deduped because a queued entry is both a pending card and a calendar
        row, and resolving a Nearest runs difflib over every candidate path."""
        if self.archive is None:
            return {}
        tz = zone(self.config.get("schedule", {}))
        wanted = dict.fromkeys(
            list(paths) + [r["path"] for r in self.timeline if r.get("path")])
        return {p: block for p in wanted
                if (block := render_prompt(self.archive.resolve_path(p),
                                           near=os.path.dirname(p), tz=tz))}

    def search(self, needle: str, lineage: str) -> tuple[list[str], list[str], int, bool]:
        """(exact hits, lineage-hint hits, count skipped as unarchived, truncated).

        `find_candidates` shuffles and caps, so filtering its sample would
        return near-nothing — a filtered view walks the publicable dirs itself.
        Order matters: resolve and filter first (regex + dict lookups, cheap),
        cap, and only then sha512-hash the survivors for the already-published
        check, which is the expensive step. `truncated` is True when either
        list had more matches than SEARCH_LIMIT before the cap — the page
        must say so, or a broad needle silently shrinks the result set with
        no sign anything was cut (the same gap-stays-visible principle as
        `skipped`)."""
        if self.archive is None:
            return [], [], 0, False
        exact, maybe, skipped = [], [], 0
        referenced = self._referenced_paths()
        for d in self.publicable_dirs:
            for path in collect_images(d):
                if os.path.realpath(path) in referenced:
                    continue
                match = self.archive.resolve_path(path)
                if isinstance(match, Nearest):
                    # A Nearest shown under "possibly from this prompt" is not
                    # also "skipped" — the banner and the maybe-list would
                    # otherwise contradict each other on a ?lineage= page.
                    if lineage and any(c.path == lineage for c in match.candidates):
                        maybe.append(path)
                    else:
                        skipped += 1
                    continue
                if not isinstance(match, Exact):
                    continue
                if needle and needle.lower() not in match.version.text.lower():
                    continue
                if lineage and lineage not in match.version.paths:
                    continue
                exact.append(path)
        published = load_publicated_hashes(self.json_path)
        display_limit = min(SEARCH_LIMIT, getattr(self.search_state, "limit", 25))
        truncated = (len(exact) + len(maybe) > display_limit
                     or len(exact) > SEARCH_LIMIT or len(maybe) > SEARCH_LIMIT)

        def unpublished(paths, remaining):
            out = []
            for p in paths[:SEARCH_LIMIT]:
                if len(out) >= remaining:
                    break
                try:
                    if compute_sha512(p) not in published:
                        out.append(p)
                except OSError as e:
                    log.debug("hashing %s: %s", p, e)
            return out

        exact_hits = unpublished(exact, display_limit)
        maybe_hits = unpublished(maybe, display_limit - len(exact_hits))
        return exact_hits, maybe_hits, skipped, truncated

    def _register_thumbs(self, paths: list[str]) -> None:
        """Extend the /thumbs allow-list with search results. The allow-list is
        'names this page rendered', so it must grow when a filtered page
        renders art the initial sample never included."""
        for p in paths:
            if p not in self.__class__.thumb_map:
                name = thumb_name(p)
                self.__class__.thumb_map[p] = name
                self.__class__.thumb_src[name] = p

    def _publications(self) -> list[dict]:
        data = store.load_publications(self.json_path)
        validate_publications(data, self.config)
        return data

    def _publication(self, uuid_: str) -> dict:
        if not isinstance(uuid_, str) or not uuid_:
            raise ValueError("uuid required")
        return store._find(self._publications(), uuid_)

    def _referenced_paths(self) -> set[str]:
        # A referenced file remains protected even if its bytes no longer match its saved hash.
        index = index_by_basename(self.data_dir)
        return {os.path.realpath(index[f['basename']]) for pub in self._publications()
                for f in pub.get('files', []) + pub.get('previewFilesIfNotFree', [])
                if f.get('basename') in index}

    def _candidates(self) -> list[str]:
        hashes = load_publicated_hashes(self.json_path)
        referenced = self._referenced_paths()
        return [p for p in dict.fromkeys(self.candidate_paths + list(self.rendered_candidates))
                if os.path.realpath(p) not in referenced and os.path.isfile(p) and compute_sha512(p) not in hashes]

    def _build_page(self, needle: str = "", lineage: str = "") -> str:
        cls = self.__class__
        with cls.operation_lock:
            search_generation = cls.search_generation
        publications = self._publications()
        index = index_by_basename(self.data_dir)
        for pub in publications:
            pub["path"] = next((index[f["basename"]] for f in pub.get("files", [])
                                if f.get("basename") in index), "")
        self.timeline = timeline(self.json_path)
        paths = self._candidates()
        candidates, maybe, skipped, truncated = paths, [], 0, False
        if needle or lineage:
            candidates, maybe, skipped, truncated = self.search(needle, lineage)
        with cls.operation_lock:
            if cls.search_generation == search_generation:
                cls.rendered_candidates.update(candidates + maybe)
        self._register_thumbs(candidates + maybe + [p["path"] for p in publications if p["path"]])
        _, calendar_thumbs = thumb_maps(self.data_dir, [], self.timeline)
        self.thumb_src.update(calendar_thumbs)
        return render_page(
            thumb_map=self.thumb_map, candidates=candidates, maybe_candidates=maybe,
            publications=publications, timeline=self.timeline,
            schedules=schedule_data(self.config.get("schedule", {}), allowed_tiers=self.config.get("tiers", [])),
            config=self.config, platforms={k: p.label for k, p in platforms.PLATFORMS.items()},
            ai_model=self.ai_model, openrouter_model=self.openrouter_model,
            prompt_html=self._prompt_html(candidates + maybe),
            query=needle, skipped=skipped, truncated=truncated,
            search_enabled=self.archive is not None,
            lineage_active=bool(lineage), vision_models=self.vision_models,
            local_model_statuses=self.local_model_statuses, search=self.search_state,
            publicable_choices=self.config.get("publicable", []))

    def _json_body(self) -> dict:
        length = int(self.headers.get("Content-length", 0))
        if not 0 <= length <= 1_000_000:
            raise ValueError("request body too large")
        data = json.loads(self.rfile.read(length) or b"{}")
        if not isinstance(data, dict):
            raise ValueError("JSON object required")
        return data

    @staticmethod
    def _fields(data: dict, allowed: set[str]) -> None:
        if set(data) - allowed:
            raise ValueError("fields not allowed: " + ", ".join(sorted(set(data) - allowed)))

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
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path in ("/", "/index.html"):
            q = urllib.parse.parse_qs(parsed.query)
            try:
                page = self._build_page(
                    needle=q.get("prompt", [""])[0],
                    lineage=q.get("lineage", [""])[0]).encode("utf-8")
            except Exception as exc:
                log.exception("Cannot render publications")
                self._send(500, {"error": str(exc)}); return
            self.send_response(200)
            self.send_header("Content-type", "text/html; charset=utf-8")
            self.send_header("Content-length", str(len(page)))
            self.end_headers()
            self.wfile.write(page)
        elif self.path.startswith("/thumbs/"):
            # Allow-list = the cache names this page rendered, so no path can be
            # traversed in. Thumbnails are built on the first request that wants
            # one and then live in the data dir, not per run in /tmp.
            name = self.path[len("/thumbs/"):]
            src = self.thumb_src.get(name)
            try:
                self._send_file(ensure_thumb(src, self.cache_dir, name) if src else "")
            except OSError as e:
                log.debug("thumbnail %s: %s", name, e)
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

    def _ai(self, data: dict) -> None:
        self._fields(data, {"uuid", "source", "model"})
        pub = self._publication(data.get("uuid"))
        workflow.require_metadata_generation(pub)
        source, posted_model = data.get("source"), data.get("model")
        model = None
        if source == "claude":
            if posted_model is not None:
                raise ValueError("Claude does not take a model object")
        elif source in ("local", "openrouter"):
            model = next((m for m in VISION_MODELS if m == posted_model), None)
            if model is None:
                raise ValueError("model object not allowed")
            if source == "local":
                status = self.local_model_statuses.get(model["ollama_name"], {})
                if not status.get("fit") or not status.get("available"):
                    raise ValueError(status.get("reason") or "Local model unavailable")

        else:
            raise ValueError("source not allowed")
        index = index_by_basename(self.data_dir)
        path = next((index[f["basename"]] for f in pub.get("files", [])
                     if f.get("basename") in index), "")
        if not path or not os.path.isfile(path):
            raise ValueError("stored artwork not found")
        with self.operation_lock:
            if pub["uuid"] in self.ai_busy:
                self._send(409, {"error": "metadata generation already running"}); return
            workflow.require_metadata_generation(self._publication(pub["uuid"]))
            self.ai_busy.add(pub["uuid"])
        try:
            image = thumb_for_ai(path, self.cache_dir, self.thumb_map.get(path))
            error = None
            error_status = 400
            try:
                title, description = generate_metadata(
                    image, model, source=source, timeout=self.ai_timeout,
                    install_missing=(source == "local"),
                    **({"claude_model": self.ai_model} if source == "claude" else {}))
                if any(not isinstance(v, str) or not v.strip() for v in (title, description)):
                    raise RuntimeError("invalid generated metadata")
            except Exception as exc:
                title = description = TODO_GENERATE
                error = str(exc)
                if not isinstance(exc, (RuntimeError, OSError)):
                    log.exception("Unexpected generation error")
                    error_status = 500
            # Provider work has no mutation lock; the application operation saves both fields.
            workflow.save_generated_metadata(self.json_path, pub["uuid"], title, description,
                                            config=self.config)
            saved = self._publication(pub["uuid"])
            result = {"saved": True, "title": saved["title"], "description": saved["description"]}
            if error is not None:
                result["error"] = error
            self._send(error_status if error is not None else 200, result)
        finally:
            with self.operation_lock:
                self.ai_busy.discard(pub["uuid"])

    def _publish(self, data: dict) -> None:
        self._fields(data, {"uuid"})
        if not self.publish_lock.acquire(blocking=False):
            self._send(409, {"error": "publication batch already running"}); return
        try:
            publications = self._publications()
            if "uuid" in data:
                pub = self._publication(data["uuid"])
                if pub["state"] not in ("DESCRIBED", "APPEARED"):
                    raise ValueError("publication is not metadata-approved")
                publications = [pub]
            registered = list(platforms.PLATFORMS)
            targets = [p for p in publications if p["state"] == "DESCRIBED" and
                       any(a["state"] == "unpublished" and a["platformName"] in registered
                           for a in p.get("apparitions", []))]
            for pub in targets:
                if any(not pub.get(k, "").strip() or TODO_GENERATE in pub[k] for k in ("title", "description")):
                    raise ValueError("AI metadata incomplete")
                for app in pub.get("apparitions", []):
                    if app["state"] == "unpublished" and app["platformName"] in registered:
                        if app["platformName"] == "deviantart" and len(pub["title"]) > 50:
                            raise ValueError("DeviantArt title must be at most 50 characters")
            published, failed, error = publish_selected(
                self.json_path, self.data_dir, uuids=[p["uuid"] for p in targets],
                platform_names=registered, config=self.config)
            remaining = [{"uuid": p["uuid"], "platform": a["platformName"]}
                         for p in self._publications() if p["uuid"] in {t["uuid"] for t in targets}
                         for a in p.get("apparitions", [])
                         if a["state"] == "unpublished" and a["platformName"] in registered]
            self._send(200, {"published": published, "failed": failed,
                             "unattempted": max(0, len(remaining) - failed),
                             "remaining": remaining, "error": error})
        finally:
            self.publish_lock.release()

    def _search(self, data: dict) -> dict:
        self._fields(data, {"limit", "paths"})
        limit, paths = data.get("limit"), data.get("paths")
        if type(limit) is not int or limit not in (0, 25, 50, 75, 100):
            raise ValueError("limit must be 0, 25, 50, 75, or 100")
        configured = self.config.get("publicable", [])
        if (not isinstance(paths, list) or any(not isinstance(p, str) for p in paths)
                or len(paths) != len(set(paths))
                or any(p not in configured for p in paths)):
            raise ValueError("paths must be unique configured publicable paths")
        directories = [str((Path(self.data_dir) / p).resolve()) for p in paths]
        candidates = find_candidates(Search(limit, tuple(directories)), self.json_path)
        with self.operation_lock:
            cls = self.__class__
            cls.search_state = Search(limit, tuple(paths))
            cls.publicable_dirs = directories
            cls.candidate_paths = candidates
            cls.rendered_candidates.clear()
            cls.search_generation += 1
        return {"limit": limit, "paths": paths}

    def _mutate(self, data: dict) -> dict:
        endpoint = self.path
        if endpoint in ("/pick", "/delete"):
            self._fields(data, {"path"})
            path = data.get("path")
            if path not in self._candidates():
                # Repeat picking is idempotent, but deletion of referenced art is forbidden.
                if endpoint != "/pick" or path not in self.candidate_paths + list(self.rendered_candidates):
                    raise ValueError("unpicked candidate path required")
            if endpoint == "/pick":
                uuid_ = workflow.pick_publication(path, self.json_path, self.config)
                workflow.repack_publications(self.json_path, self.config)
                return {"uuid": uuid_, "saved": True}
            os.remove(path)
            self.candidate_paths[:] = [p for p in self.candidate_paths if p != path]
            self.rendered_candidates.discard(path)
            name = self.thumb_map.pop(path, "")
            self.thumb_src.pop(name, None)
            return {"ok": True}
        if endpoint in ("/selection", "/selection/all"):
            self._fields(data, {"selected"} if endpoint.endswith("/all") else {"uuid", "platform", "selected"})
            if endpoint == "/selection":
                self._publication(data.get("uuid"))
                if not isinstance(data.get("platform"), str):
                    raise ValueError("platform required")
            workflow.select_publication_platforms(self.json_path, selected=data.get("selected"),
                registered_platforms=list(platforms.PLATFORMS), uuid_=data.get("uuid"),
                platform=data.get("platform"), config=self.config)
            return {"saved": True}
        allowed = {
            "/schedule": {"uuid", "schedule", "scheduleTs", "scheduleProfile", "scheduleIsManual"},
            "/schedule/approve": {"uuid"}, "/schedule/return": {"uuid"},
            "/metadata": {"uuid", "title", "description"},
            "/metadata/approve": {"uuid"}, "/metadata/return": {"uuid"},
            "/options": {"uuid", "platform", "price", "tiers", "galleries"},
        }
        if endpoint not in allowed:
            raise LookupError("endpoint not found")
        self._fields(data, allowed[endpoint])
        pub = self._publication(data.get("uuid"))
        uuid_ = pub["uuid"]
        if endpoint == "/schedule":
            if "scheduleIsManual" in data and type(data["scheduleIsManual"]) is not bool:
                raise ValueError("scheduleIsManual must be boolean")
            workflow.edit_publication_date(self.json_path, uuid_, data, self.config)
        elif endpoint == "/schedule/approve":
            workflow.approve_publication_date(self.json_path, uuid_, self.config)
        elif endpoint == "/schedule/return":
            workflow.return_to_scheduling(self.json_path, uuid_, self.config)
        elif endpoint == "/metadata":
            fields = {k: data[k] for k in ("title", "description") if k in data}
            if not fields or any(not isinstance(v, str) for v in fields.values()):
                raise ValueError("metadata string fields required")
            workflow.save_publication_metadata(self.json_path, uuid_, config=self.config, **fields)
        elif endpoint == "/metadata/approve":
            workflow.approve_publication_metadata(self.json_path, uuid_, self.config)
        elif endpoint == "/metadata/return":
            workflow.return_to_metadata(self.json_path, uuid_, self.config)
        elif endpoint == "/options":
            if data.get("platform") not in platforms.PLATFORMS:
                raise ValueError("unregistered platform")
            workflow.save_platform_options(self.json_path, uuid_, data["platform"],
                {k: data[k] for k in ("price", "tiers", "galleries") if k in data}, self.config)
        return {"saved": True}

    def do_POST(self):
        try:
            if self.headers.get_content_type() != "application/json":
                self._send(415, {"error": "application/json required"}); return
            origin = self.headers.get("Origin")
            host, port = self.server.server_address
            # Trust the bound address, not the request's attacker-controlled Host.
            if origin is not None and origin not in {f"http://{host}:{port}", f"http://localhost:{port}"}:
                self._send(403, {"error": "foreign origin forbidden"}); return
            data = self._json_body()
            if self.path == "/ai":
                self._ai(data)
            elif self.path == "/publish":
                self._publish(data)
            elif self.path == "/search":
                self._send(200, self._search(data))
            else:
                with self.operation_lock:
                    if self.path in ("/metadata/approve", "/schedule/return") and data.get("uuid") in self.ai_busy:
                        self._send(409, {"error": "wait for metadata generation to save"}); return
                    if not self.publish_lock.acquire(blocking=False):
                        self._send(409, {"error": "publication batch already running"}); return
                    try:
                        result = self._mutate(data)
                    finally:
                        self.publish_lock.release()
                self._send(200, result)
        except KeyError as exc:
            self._send(404, {"error": f"uuid not found: {exc}"})
        except LookupError as exc:
            self._send(404, {"error": str(exc)})
        except (ValueError, TypeError, OSError, ValidationError) as exc:
            self._send(400, {"error": str(exc)})
        except Exception as exc:
            log.exception("HTTP mutation failed")
            self._send(500, {"error": str(exc)})

    def log_message(self, format, *args):
        log.debug("http %s - %s", self.address_string(), format % args)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def thumb_maps(data_dir: str, paths: list[str], rows: list[dict]) -> tuple[dict, dict]:
    """({art path: cache name}, {cache name: art path}) for everything the page
    can show, and each calendar row stamped with its cache name and its file on
    disk (both empty when its art has left the disk). Naming happens here, where
    the files are: gallery art is hashed, the back-catalogue reuses the sha512
    publications.json stores, and one walk of the data dir resolves basenames to
    paths. `path` is that same walk's answer, which is what lets the calendar
    look a published image's prompt up by path like any gallery card."""
    thumb_map = {p: thumb_name(p) for p in paths}
    thumb_src = {name: p for p, name in thumb_map.items()}
    index = index_by_basename(data_dir)
    for r in rows:
        src = index.get(r["basename"])
        # thumb_name() off `src`, not the basename: the oldest entries have no
        # stored sha at all, and then it hashes the file it just found.
        r["thumb"] = thumb_name(src, r["sha"]) if src else ""
        r["path"] = src or ""
        if src:
            thumb_src[r["thumb"]] = src
    return thumb_map, thumb_src


def create_handler(data_dir: str, candidate_paths: list[str], args, config: dict,
                   publicable_dirs: list[str] | None = None) -> type[GalleryHandler]:
    """Isolated handler configuration; every request rereads the durable database."""
    class Handler(GalleryHandler):
        pass
    Handler.data_dir = str(Path(data_dir).resolve())
    Handler.cache_dir = os.path.join(data_dir, THUMB_CACHE)
    Handler.candidate_paths = list(candidate_paths)
    Handler.rendered_candidates = set()
    Handler.search_generation = 0
    Handler.thumb_map, Handler.thumb_src = {}, {}
    Handler.ai_busy = set()
    Handler.operation_lock = threading.RLock()
    Handler.publish_lock = threading.Lock()
    Handler.ai_model = args.ai_model
    Handler.openrouter_model = args.openrouter_model
    Handler.ai_timeout = args.ai_timeout
    Handler.local_model_statuses = {
        m["ollama_name"]: local_model_status(m) for m in VISION_MODELS}
    Handler.json_path = args.json
    Handler.config = config
    Handler.archive = prompt_archive_from_config(data_dir, config)
    Handler.search_state = Search(25, tuple(config.get("publicable", [])))
    Handler.publicable_dirs = list(publicable_dirs or [])
    schedule_data(config.get("schedule", {}), allowed_tiers=config.get("tiers", []))
    return Handler


def serve(data_dir: str, candidate_paths: list[str], args, config: dict,
          publicable_dirs: list[str] | None = None) -> None:
    handler = create_handler(data_dir, candidate_paths, args, config, publicable_dirs)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler)
    url = f"http://127.0.0.1:{server.server_address[1]}"
    print(f"Gallery: {url}", flush=True)
    try:
        subprocess.Popen(["firefox", "--private-window", "--no-remote", url],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except FileNotFoundError:
        print("(open the URL yourself; firefox not found)")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down...")
    finally:
        server.server_close()
