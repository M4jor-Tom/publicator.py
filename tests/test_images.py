import json

from publicator.images import compute_sha512, find_candidates, guess_mime


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


def test_guess_mime_falls_back_to_octet_stream():
    assert guess_mime("/a/b.jpg") == "image/jpeg"
    assert guess_mime("/a/b.jpeg") == "image/jpeg"
    assert guess_mime("/a/b.png") == "image/png"
    assert guess_mime("/a/b.webp") == "image/webp"
    assert guess_mime("/a/b.gif") == "image/gif"
    assert guess_mime("/a/b.mp4") == "video/mp4"
    assert guess_mime("/a/b.unknown") == "application/octet-stream"
