import json
import threading
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer

from publicator.images import thumb_name
from publicator.webui.server import GalleryHandler, thumb_maps


def test_thumb_maps_pairs_gallery_art_with_calendar_rows(tmp_path):
    picked = tmp_path / "picked"
    picked.mkdir()
    pick = picked / "new.png"
    pick.write_bytes(b"\x89PNG-new")
    old = picked / "old.webp"          # published art, still on disk
    old.write_bytes(b"RIFF-old")
    rows = [{"basename": "old.webp", "sha": "ab" * 64},
            {"basename": "gone.webp", "sha": "cd" * 64}]

    thumb_map, thumb_src = thumb_maps(str(tmp_path), [str(pick)], rows)

    assert thumb_map == {str(pick): thumb_name(str(pick))}       # cards -> cache names
    assert thumb_src[thumb_name(str(pick))] == str(pick)         # and back, for serving
    assert thumb_src["ab" * 16 + ".webp"] == str(old)            # stored sha, no re-hash
    assert "cd" * 16 + ".webp" not in thumb_src                  # art no longer on disk
    assert rows[0]["thumb"] == "ab" * 16 + ".webp"               # stamped for the view
    assert rows[1]["thumb"] == ""                                # ...which shows text instead
    assert rows[0]["path"] == str(old)   # how the calendar looks its prompt up
    assert rows[1]["path"] == ""         # no file, so no prompt to resolve


def test_thumb_maps_hashes_a_row_the_json_never_recorded_a_sha_for(tmp_path):
    art = tmp_path / "old.webp"
    art.write_bytes(b"RIFF-old")
    rows = [{"basename": "old.webp", "sha": ""}]

    _, thumb_src = thumb_maps(str(tmp_path), [], rows)

    assert rows[0]["thumb"] == thumb_name(str(art))   # hashed from the file it found
    assert thumb_src[rows[0]["thumb"]] == str(art)


def _serve_once(**attrs):
    for k, v in attrs.items():
        setattr(GalleryHandler, k, v)
    server = ThreadingHTTPServer(("127.0.0.1", 0), GalleryHandler)
    threading.Thread(target=server.handle_request, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}"


def _get(url):
    try:
        with urllib.request.urlopen(url, timeout=5) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, b""


def _post(url, data):
    request = urllib.request.Request(url + "/ai", data=json.dumps(data).encode(),
                                     headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=5) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


def test_ai_accepts_only_catalog_objects_and_source(monkeypatch, tmp_path):
    from publicator.llm_meta import VISION_MODELS
    from publicator.webui import server as gallery
    calls = []
    monkeypatch.setattr(gallery, "thumb_for_ai", lambda *a: "thumb.png")
    monkeypatch.setattr(gallery, "generate_metadata", lambda *a, **kw: (calls.append((a, kw)) or ("t", "d")))
    model = VISION_MODELS[0]
    base = dict(cache_dir=str(tmp_path), thumb_map={}, ai_model="claude-custom-opus",
                openrouter_model=model["openrouter_name"], ai_timeout=3,
                local_model_statuses={model["ollama_name"]: {"fit": True, "installed": False,
                                                               "available": True, "reason": ""}})
    url = _serve_once(**base)
    assert _post(url, {"path": "a.png", "source": "openrouter", "model": model})[0] == 200
    assert calls[-1][0][1:] == (model,)
    assert calls[-1][1]["source"] == "openrouter"
    url = _serve_once(**base)
    assert _post(url, {"path": "a.png", "source": "openrouter",
                       "model": {**model, "llmfit_name": "forged"}})[0] == 400
    url = _serve_once(**base)
    assert _post(url, {"path": "a.png", "source": "unknown", "model": model})[0] == 400
    url = _serve_once(**base)
    assert _post(url, {"path": "a.png", "source": "claude", "model": None})[0] == 200
    assert calls[-1][1]["source"] == "claude"
    assert calls[-1][1]["claude_model"] == "claude-custom-opus"
    assert calls[-1][0][1] is None


def test_ai_refuses_unfit_local_and_allows_install_on_use(monkeypatch, tmp_path):
    from publicator.llm_meta import VISION_MODELS
    from publicator.webui import server as gallery
    calls = []
    monkeypatch.setattr(gallery, "thumb_for_ai", lambda *a: "thumb.png")
    monkeypatch.setattr(gallery, "generate_metadata", lambda *a, **kw: (calls.append((a, kw)) or ("t", "d")))
    model = VISION_MODELS[0]
    url = _serve_once(cache_dir=str(tmp_path), thumb_map={}, ai_model="claude-cli-opus",
                      openrouter_model=model["openrouter_name"], ai_timeout=3,
                      local_model_statuses={model["ollama_name"]: {"fit": False, "installed": False,
                                                                     "available": False, "reason": "Too Tight"}})
    request = {"path": "a.png", "source": "local", "model": model}
    assert _post(url, request)[0] == 400
    assert not calls

    url = _serve_once(cache_dir=str(tmp_path), thumb_map={}, ai_model="claude-cli-opus",
                      openrouter_model=model["openrouter_name"], ai_timeout=3,
                      local_model_statuses={model["ollama_name"]: {"fit": True, "installed": False,
                                                                     "available": True, "reason": ""}})
    assert _post(url, request)[0] == 200
    assert calls[-1][1]["source"] == "local" and calls[-1][1]["install_missing"] is True


def test_thumbs_route_serves_a_rendered_name(tmp_path):
    art = tmp_path / "a.png"
    art.write_bytes(b"\x89PNG-art")
    url = _serve_once(cache_dir=str(tmp_path / ".thumbs"),
                      thumb_src={"beef.png": str(art)}, thumb_map={})
    status, body = _get(url + "/thumbs/beef.png")
    assert status == 200 and body                       # generated into the cache on demand
    assert (tmp_path / ".thumbs" / "beef.png").exists()  # ...and kept there


def test_thumbs_route_refuses_a_name_the_page_never_rendered(tmp_path):
    url = _serve_once(cache_dir=str(tmp_path / ".thumbs"), thumb_src={}, thumb_map={})
    assert _get(url + "/thumbs/../../etc/passwd")[0] == 404


def test_search_matches_exact_prompt_text_only(tmp_path, monkeypatch):
    """Matching a Nearest's text would return images whose prompt merely
    resembles the query - the conflation the whole feature exists to prevent."""
    import subprocess
    from publicator.prompts import PromptArchive
    from publicator.webui.server import GalleryHandler
    import re, hashlib

    repo = tmp_path / "hf"; repo.mkdir()
    run = lambda *a: subprocess.run(["git", "-C", str(repo), *a], check=True,
                                    capture_output=True)
    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@e"); run("config", "user.name", "t")
    (repo / "a").write_text("tentacles everywhere\n")
    run("add", "--", "a"); run("commit", "-q", "-m", "s")
    digest = hashlib.sha1(b"tentacles everywhere\n").hexdigest()

    pattern = re.compile(r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_"
                         r"[0-9a-f-]{36}\.[^.]+$")
    uuid = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"
    picked = tmp_path / "picked"; picked.mkdir()
    hit = picked / f"a_{digest}_{uuid}.webp"
    miss = picked / f"a_{'f' * 40}_{uuid}.webp"      # never archived -> Nearest
    hit.write_bytes(b"1"); miss.write_bytes(b"2")

    # monkeypatch, not plain assignment: these are CLASS attributes and would
    # otherwise leak into every other test in this file.
    monkeypatch.setattr(GalleryHandler, "archive",
                        PromptArchive(str(repo), pattern, "sha1"))
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [str(picked)])
    monkeypatch.setattr(GalleryHandler, "json_path",
                        str(tmp_path / "publications.json"))

    exact, _maybe, skipped, truncated = GalleryHandler.search(GalleryHandler, "tentacles", "")
    assert exact == [str(hit)]
    assert skipped == 1, "the unarchived one is reported, not silently dropped"
    assert truncated is False


def test_search_is_inert_when_the_feature_is_off(monkeypatch):
    from publicator.webui.server import GalleryHandler
    monkeypatch.setattr(GalleryHandler, "archive", None)
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [])
    assert GalleryHandler.search(GalleryHandler, "nothing", "") == ([], [], 0, False)


def test_prompt_html_is_empty_when_the_feature_is_off(monkeypatch):
    from publicator.webui.server import GalleryHandler
    monkeypatch.setattr(GalleryHandler, "archive", None)
    assert GalleryHandler._prompt_html(GalleryHandler, ["a.webp"]) == {}


def test_skipped_excludes_a_nearest_already_shown_as_a_lineage_hint(tmp_path, monkeypatch):
    """A Nearest whose candidates match the requested lineage is shown under
    'possibly from this prompt' on a ?lineage= page — it must not ALSO
    inflate the 'skipped' banner, or the page would simultaneously claim
    those images were never archived and show them right there."""
    import subprocess
    from publicator.prompts import PromptArchive
    from publicator.webui.server import GalleryHandler
    import re

    repo = tmp_path / "hf"; repo.mkdir()
    run = lambda *a: subprocess.run(["git", "-C", str(repo), *a], check=True,
                                    capture_output=True)
    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@e"); run("config", "user.name", "t")
    (repo / "a").write_text("original prompt\n")
    run("add", "--", "a"); run("commit", "-q", "-m", "s")

    pattern = re.compile(r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_"
                         r"[0-9a-f-]{36}\.[^.]+$")
    uuid = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"
    picked = tmp_path / "picked"; picked.mkdir()
    miss = picked / f"a_{'f' * 40}_{uuid}.webp"      # never archived -> Nearest, lineage "a" known
    miss.write_bytes(b"1")

    monkeypatch.setattr(GalleryHandler, "archive",
                        PromptArchive(str(repo), pattern, "sha1"))
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [str(picked)])
    monkeypatch.setattr(GalleryHandler, "json_path",
                        str(tmp_path / "publications.json"))

    exact, maybe, skipped, truncated = GalleryHandler.search(GalleryHandler, "", "a")
    assert maybe == [str(miss)]
    assert skipped == 0, "already shown under 'possibly from this prompt' - not also 'skipped'"
