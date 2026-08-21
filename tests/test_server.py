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

    exact, _maybe, skipped = GalleryHandler.search(GalleryHandler, "tentacles", "")
    assert exact == [str(hit)]
    assert skipped == 1, "the unarchived one is reported, not silently dropped"


def test_search_is_inert_when_the_feature_is_off(monkeypatch):
    from publicator.webui.server import GalleryHandler
    monkeypatch.setattr(GalleryHandler, "archive", None)
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [])
    assert GalleryHandler.search(GalleryHandler, "nothing", "") == ([], [], 0)
