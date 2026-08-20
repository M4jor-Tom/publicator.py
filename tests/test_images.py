import json
from pathlib import Path

from publicator.images import (
    compute_sha512,
    ensure_thumb,
    find_candidates,
    guess_mime,
    index_by_basename,
    thumb_name,
)


def test_find_candidates_skips_already_published_hashes(tmp_path):
    picked = tmp_path / "picked"
    picked.mkdir()
    new, done = picked / "new.png", picked / "done.png"
    new.write_bytes(b"\x89PNG-new")
    done.write_bytes(b"\x89PNG-done")
    jp = tmp_path / "publications.json"
    jp.write_text(json.dumps([{"files": [{"sha512sum": compute_sha512(str(done)).upper()}]}]))

    got = find_candidates([str(picked)], str(jp), 10)

    assert got == [str(new)]  # case-insensitive hash match drops done.png


def test_find_candidates_honours_the_limit(tmp_path):
    picked = tmp_path / "picked"
    picked.mkdir()
    for i in range(5):
        (picked / f"{i}.png").write_bytes(f"img{i}".encode())
    assert len(find_candidates([str(picked)], str(tmp_path / "none.json"), 3)) == 3


def test_find_candidates_tolerates_a_missing_directory(tmp_path):
    assert find_candidates([str(tmp_path / "nope")], str(tmp_path / "none.json"), 5) == []


def test_thumb_name_is_content_addressed(tmp_path):
    a, b = tmp_path / "a.png", tmp_path / "b.png"
    a.write_bytes(b"same"); b.write_bytes(b"same")
    assert thumb_name(str(a)) == thumb_name(str(b))   # one cache entry per content
    assert thumb_name(str(a)).endswith(".png")        # ext kept, so mime stays right


def test_thumb_name_accepts_a_known_sha_without_reading_the_file():
    # publications.json already stores the sha512; no re-hash, no file needed.
    assert thumb_name("/gone/art.webp", sha="ab" * 64) == "ab" * 16 + ".webp"


def test_ensure_thumb_reuses_the_cached_file(tmp_path):
    src = tmp_path / "a.png"
    src.write_bytes(b"\x89PNG-art")
    cache = tmp_path / ".thumbs"

    first = ensure_thumb(str(src), str(cache))

    assert Path(first).parent == cache
    Path(first).write_bytes(b"CACHED")                # stamp it
    assert ensure_thumb(str(src), str(cache)) == first
    assert Path(first).read_bytes() == b"CACHED"      # second call did not regenerate


def test_ensure_thumb_can_be_told_the_cache_name(tmp_path):
    # The server already knows the name (it is the URL it was asked for) and the
    # sha it came from, so it must not have to re-hash the art to serve it.
    src = tmp_path / "a.png"
    src.write_bytes(b"\x89PNG-art")
    got = ensure_thumb(str(src), str(tmp_path / ".thumbs"), name="beef.png")
    assert Path(got).name == "beef.png" and Path(got).exists()


def test_index_by_basename_finds_art_under_the_data_dir(tmp_path):
    (tmp_path / "picked").mkdir()
    art = tmp_path / "picked" / "art.webp"
    art.write_bytes(b"x")
    (tmp_path / ".thumbs").mkdir()
    (tmp_path / ".thumbs" / "cached.webp").write_bytes(b"x")

    index = index_by_basename(str(tmp_path))

    assert index["art.webp"] == str(art)
    assert "cached.webp" not in index      # hidden dirs (cache, browser profiles) skipped


def test_guess_mime_falls_back_to_octet_stream():
    assert guess_mime("/a/b.jpg") == "image/jpeg"
    assert guess_mime("/a/b.jpeg") == "image/jpeg"
    assert guess_mime("/a/b.png") == "image/png"
    assert guess_mime("/a/b.webp") == "image/webp"
    assert guess_mime("/a/b.gif") == "image/gif"
    assert guess_mime("/a/b.mp4") == "video/mp4"
    assert guess_mime("/a/b.unknown") == "application/octet-stream"
