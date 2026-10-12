import json
import re
from datetime import datetime
from zoneinfo import ZoneInfo

from publicator.webui.calendar_view import render_calendar, timeline

PARIS = ZoneInfo("Europe/Paris")


def ts(y, m, d, h=20, minute=0):
    return int(datetime(y, m, d, h, minute, tzinfo=PARIS).timestamp())


def cell(page, iso_date):
    """The <td> for a given day, so tests assert on placement, not on layout."""
    m = re.search(rf'<td[^>]*data-date="{iso_date}"[^>]*>(.*?)</td>', page, re.S)
    assert m, f"no cell for {iso_date}"
    return m.group(0)


def write_pubs(tmp_path, pubs):
    p = tmp_path / "publications.json"
    p.write_text(json.dumps(pubs))
    return str(p)


def pub(**over):
    entry = {"uuid": "u1", "state": "APPEARED", "title": "Haunted Tower",
             "submissionTimestamp": ts(2026, 1, 6), "description": "d",
             "apparitionTimestampIfDifferentThanSubmission": ts(2026, 1, 6),
             "files": [{"basename": "art.webp", "sha512sum": "ab" * 64}],
             "apparitions": [{"platformName": "deviantart", "state": "published_or_scheduled",
                              "url": "https://www.deviantart.com/snoiot/art/actual-title-1278691961"}]}
    entry.update(over)
    return entry


# --- timeline -------------------------------------------------------------

def test_timeline_uses_publication_title_and_apparition_url(tmp_path):
    row, = timeline(write_pubs(tmp_path, [pub(title="Shared Title")]))
    assert row["title"] == "Shared Title"
    assert row["url"] == "https://www.deviantart.com/snoiot/art/actual-title-1278691961"
    assert row["state"] == "published_or_scheduled"
    assert row["sha"] == "ab" * 64 and row["basename"] == "art.webp"


def test_timeline_labels_entries_waiting_for_ai_metadata(tmp_path):
    e = pub(title="[[PUBLICATOR:TODO_GENERATE]]", description="[[PUBLICATOR:TODO_GENERATE]]")
    e["apparitions"][0] = {"platformName": "deviantart", "state": "unpublished"}
    row, = timeline(write_pubs(tmp_path, [e]))
    assert row["title"] == "AI metadata pending"
    assert row["url"] is None


def test_timeline_falls_back_to_submission_timestamp(tmp_path):
    e = pub(submissionTimestamp=ts(2026, 3, 4))
    e.pop("apparitionTimestampIfDifferentThanSubmission")
    row, = timeline(write_pubs(tmp_path, [e]))
    assert row["ts"] == ts(2026, 3, 4)


def test_timeline_keeps_a_queued_entry_as_plain_title_without_a_link(tmp_path):
    e = pub(state="DESCRIBED", title="Tickled Pink")
    e["apparitions"][0] = {"platformName": "deviantart", "state": "unpublished"}
    row, = timeline(write_pubs(tmp_path, [e]))
    assert row["title"] == "Tickled Pink"
    assert row["url"] is None
    assert row["state"] == "unpublished"


def test_timeline_keeps_all_platforms_and_sorts_by_date(tmp_path):
    late = pub(uuid="late", apparitionTimestampIfDifferentThanSubmission=ts(2026, 5, 1))
    early = pub(uuid="early")
    early["apparitions"] = [
        {"platformName": "pixiv", "state": "published_or_scheduled",
         "url": "https://www.pixiv.net/en/artworks/1"},
        early["apparitions"][0]]
    early['apparitions'][1]['state'] = 'unpublished'
    rows = timeline(write_pubs(tmp_path, [late, early]))
    assert [(r['uuid'], r['platformName']) for r in rows] == [
        ('early', 'pixiv'), ('early', 'deviantart'), ('late', 'deviantart')]
    assert rows[0]['url'] == 'https://www.pixiv.net/en/artworks/1'
    assert rows[0]['title'] == rows[1]['title'] == early['title']
    assert rows[0]['ts'] == rows[1]['ts'] == early['apparitionTimestampIfDifferentThanSubmission']
    assert rows[0]['state'] == 'published_or_scheduled'
    assert rows[1]['state'] == 'unpublished'


def test_timeline_reads_the_legacy_sha_key(tmp_path):
    # Most of the back-catalogue stores fileSha512sum, not sha512sum.
    e = pub()
    e["files"] = [{"basename": "art.webp", "fileSha512sum": "cd" * 64}]
    row, = timeline(write_pubs(tmp_path, [e]))
    assert row["sha"] == "cd" * 64


def test_timeline_survives_an_entry_with_no_files(tmp_path):
    e = pub()
    e.pop("files")
    row, = timeline(write_pubs(tmp_path, [e]))
    assert row["sha"] == "" and row["basename"] == ""


def test_timeline_tolerates_a_missing_file(tmp_path):
    assert timeline(str(tmp_path / "nope.json")) == []


# --- render ---------------------------------------------------------------

def _render(rows, now=None, anchors=None, prompts=None):
    return render_calendar(rows, tz=PARIS, now=now or ts(2026, 1, 6, 12),
                           anchors=anchors or {}, prompts=prompts or {})


def row(**over):
    # `thumb` is stamped server-side (server.thumb_maps): the view never hashes.
    r = {"uuid": "u1", "platformName": "deviantart", "ts": ts(2026, 1, 6), "title": "Haunted Tower",
         "url": "https://da/art/Haunted-Tower", "state": "published_or_scheduled",
         "basename": "art.webp", "sha": "ab" * 64, "thumb": "ab" * 16 + ".webp"}
    r.update(over)
    return r


def test_event_lands_on_its_local_day_not_the_utc_one():
    # 00:30 Paris is 23:30 UTC the day before: a browser/UTC reading would
    # file this under the 5th. It belongs to the 6th.
    page = _render([row(ts=ts(2026, 1, 6, 0, 30))])
    assert "Haunted Tower" in cell(page, "2026-01-06")
    assert "Haunted Tower" not in cell(page, "2026-01-05")


def test_published_event_links_to_deviantart_and_shows_its_thumbnail():
    page = _render([row()])
    c = cell(page, "2026-01-06")
    assert 'href="https://da/art/Haunted-Tower"' in c
    assert f'src="/thumbs/{"ab" * 16}.webp"' in c        # content-addressed cache name
    assert 'target="_blank"' in c


def test_queued_event_links_to_its_gallery_card():
    page = _render([row(state="unpublished", url=None, ts=ts(2026, 1, 20))],
                   anchors={"u1": "pending_3"})
    c = cell(page, "2026-01-20")
    assert 'href="#pending_3"' in c
    assert "upcoming" in c


def test_an_event_whose_art_left_the_disk_shows_its_title():
    page = _render([row(thumb="")])
    c = cell(page, "2026-01-06")
    assert "<img" not in c
    assert "Haunted Tower" in c


def test_a_queued_event_without_a_card_is_still_shown_unlinked():
    page = _render([row(state="unpublished", url=None, ts=ts(2026, 1, 20))])
    c = cell(page, "2026-01-20")
    assert "Haunted Tower" in c
    assert "href=" not in c


BLOCK = '<details class="prompt exact"><summary>prompt</summary><pre>a cat</pre></details>'


def test_a_published_event_shows_the_same_prompt_block_the_gallery_renders():
    # The whole point of the unification: prompt_view renders the block once,
    # for a card and a calendar thumbnail alike — spliced in, never re-derived.
    page = _render([row(path="/d/art.webp")], prompts={"/d/art.webp": BLOCK})
    c = cell(page, "2026-01-06")
    assert BLOCK in c
    assert 'popovertarget="evp2026-01-06-0"' in c
    assert 'id="evp2026-01-06-0" popover' in c


def test_a_prompt_bearing_event_keeps_its_deviantart_link_inside_the_panel():
    page = _render([row(path="/d/art.webp")], prompts={"/d/art.webp": BLOCK})
    c = cell(page, "2026-01-06")
    assert 'href="https://da/art/Haunted-Tower"' in c and 'target="_blank"' in c
    assert "open on DeviantArt" in c


def test_prompt_links_use_the_apparition_platform_and_escape_unknown_names():
    for name, label in [('pixiv', 'Pixiv'), ('other<platform>', 'other&lt;platform&gt;')]:
        page = _render([row(platformName=name, path='/d/art.webp')],
                       prompts={'/d/art.webp': BLOCK})
        assert f'open on {label} ↗' in page
        assert 'open on DeviantArt' not in page
        assert 'other<platform>' not in page


def test_a_prompt_bearing_queued_event_still_reaches_its_gallery_card():
    page = _render([row(state="unpublished", url=None, ts=ts(2026, 1, 20),
                        path="/d/art.webp")],
                   anchors={"u1": "pending_3"}, prompts={"/d/art.webp": BLOCK})
    c = cell(page, "2026-01-20")
    assert '<a href="#pending_3">go to its gallery card</a>' in c
    assert 'target="_blank"' not in c   # an in-page anchor, not a new tab


def test_an_event_without_a_known_prompt_stays_a_one_click_link():
    # Most of the back-catalogue predates the archive; a panel holding only a
    # link would cost those an extra click for nothing.
    c = cell(_render([row(path="/d/art.webp")]), "2026-01-06")
    assert "popover" not in c
    assert '<a class="ev upcoming" href="https://da/art/Haunted-Tower"' in c


def test_every_event_panel_gets_its_own_id_even_within_one_publication():
    # Two apparitions of one entry share a uuid AND, here, one art file, so the
    # id has to come off the cell — otherwise both panels answer to one target.
    page = _render([row(path="/d/a.webp"), row(path="/d/a.webp"),
                    row(path="/d/a.webp", ts=ts(2026, 1, 8))],
                   prompts={"/d/a.webp": BLOCK})
    for eid in ("evp2026-01-06-0", "evp2026-01-06-1", "evp2026-01-08-0"):
        assert f'id="{eid}" popover' in page and f'popovertarget="{eid}"' in page


def test_today_is_marked_and_past_events_are_dimmed():
    page = _render([row(ts=ts(2025, 12, 30))], now=ts(2026, 1, 6, 12))
    assert 'data-date="2026-01-06" class="day today"' in page or \
           'class="day today" data-date="2026-01-06"' in page
    assert "past" in cell(page, "2025-12-30")


def test_months_span_the_first_event_through_today():
    page = _render([row(ts=ts(2025, 11, 4))], now=ts(2026, 1, 6, 12))
    for month in ("November 2025", "December 2025", "January 2026"):
        assert month in page


def test_months_span_today_through_the_last_scheduled_event():
    page = _render([row(ts=ts(2026, 3, 10), state="unpublished", url=None)],
                   now=ts(2026, 1, 6, 12))
    for month in ("January 2026", "February 2026", "March 2026"):
        assert month in page


def test_empty_timeline_still_renders_the_current_month():
    assert "January 2026" in _render([])


def test_the_current_month_is_marked_so_the_tab_can_scroll_to_it():
    page = _render([row(ts=ts(2025, 11, 4))], now=ts(2026, 1, 6, 12))
    assert 'class="month now" id="m-2026-01"' in page
    assert 'class="month" id="m-2025-11"' in page
