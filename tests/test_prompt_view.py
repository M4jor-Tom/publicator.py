from zoneinfo import ZoneInfo

from publicator.prompts import Exact, Lineage, Nearest, PromptVersion, Unknown
from publicator.webui.prompt_view import render_prompt


def version(text="a prompt", paths=("p/a",), committed=1_700_000_000, digest="ab" * 20):
    return PromptVersion(version=digest, text=text, paths=paths, committed=committed)


def test_exact_renders_the_prompt_text():
    out = render_prompt(Exact(version=version(text="the real prompt")))
    assert "the real prompt" in out
    assert "NOT ARCHIVED" not in out


def test_exact_shows_every_path_not_just_the_first():
    out = render_prompt(Exact(version=version(paths=("p/a", "q/a"))))
    assert "p/a" in out and "q/a" in out


def test_exact_escapes_html_in_the_prompt():
    out = render_prompt(Exact(version=version(text="<script>alert(1)</script>")))
    assert "<script>" not in out
    assert "&lt;script&gt;" in out


def test_nearest_is_labelled_as_not_archived():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="p/a", versions=(version(text="an older one"),)),)))
    assert "NOT ARCHIVED" in out
    assert "an older one" in out, "near-misses are still shown, just labelled"
    # the text only ever appears inside the warning block
    assert out.index("NOT ARCHIVED") < out.index("an older one")


def test_nearest_lists_every_candidate_lineage():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="dir_one/a", versions=(version(),)),
        Lineage(path="dir_two/a", versions=(version(), version(digest="cd" * 20))))))
    assert "dir_one/a" in out and "dir_two/a" in out
    assert "1 known version" in out and "2 known versions" in out


def test_unknown_renders_nothing():
    assert render_prompt(Unknown(reason="whatever")) == ""


def test_undated_version_does_not_crash():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="p/a", versions=(version(committed=0),)),)))
    assert "unknown date" in out


def test_stamp_renders_in_the_given_timezone():
    """1700000000 is 2023-11-14T22:13:20Z, so a UTC+13 zone lands on the NEXT
    day — a date that silently ignored tz would fail this."""
    v = version(committed=1_700_000_000)
    assert "2023-11-14" in render_prompt(Nearest(candidates=(
        Lineage(path="p/a", versions=(v,)),)), tz=ZoneInfo("UTC"))
    assert "2023-11-15" in render_prompt(Nearest(candidates=(
        Lineage(path="p/a", versions=(v,)),)), tz=ZoneInfo("Pacific/Auckland"))


def test_nearest_escapes_html_in_candidate_path_and_text():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="p/<script>", versions=(version(text="<script>alert(1)</script>"),)),)))
    assert "<script>" not in out
    assert "&lt;script&gt;" in out


def test_nearest_lineages_link_to_their_group():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="anima_v1/hot.json", versions=(version(),)),)))
    assert 'href="/?lineage=anima_v1%2Fhot.json"' in out


def test_exact_paths_link_to_their_group():
    out = render_prompt(Exact(version=version(paths=("anima_v1/hot.json",))))
    assert 'href="/?lineage=anima_v1%2Fhot.json"' in out
