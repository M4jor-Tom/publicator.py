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
    entry = {"uuid": "u1", "submissionTimestamp": ts(2026, 1, 6), "description": "d",
             "files": [{"basename": "art.webp", "sha512sum": "ab" * 64}],
             "apparitions": [{"platformName": "deviantart", "state": "published_or_scheduled",
                              "urlElsePublicationName":
                                  "https://www.deviantart.com/snoiot/art/Haunted-Tower-1278691961",
                              "apparitionTimestampIfDifferentThanSubmission": ts(2026, 1, 6)}]}
    entry.update(over)
    return entry


# --- timeline -------------------------------------------------------------

def test_timeline_titles_a_published_entry_from_its_url_slug(tmp_path):
    row, = timeline(write_pubs(tmp_path, [pub()]))
    assert row["title"] == "Haunted Tower"
    assert row["url"] == "https://www.deviantart.com/snoiot/art/Haunted-Tower-1278691961"
    assert row["state"] == "published_or_scheduled"
    assert row["sha"] == "ab" * 64 and row["basename"] == "art.webp"


def test_timeline_falls_back_to_submission_timestamp(tmp_path):
    e = pub(submissionTimestamp=ts(2026, 3, 4))
    e["apparitions"][0].pop("apparitionTimestampIfDifferentThanSubmission")
    row, = timeline(write_pubs(tmp_path, [e]))
    assert row["ts"] == ts(2026, 3, 4)


def test_timeline_falls_back_to_the_description_when_the_slug_is_only_an_id(tmp_path):
    e = pub(description="A lady and a bewildered monster")
    e["apparitions"][0]["urlElsePublicationName"] = \
        "https://www.deviantart.com/snoiot/art/1281297380?action=published"
    row, = timeline(write_pubs(tmp_path, [e]))
    assert row["title"] == "A lady and a bewildered monster"


def test_timeline_keeps_a_queued_entry_as_plain_title_without_a_link(tmp_path):
    e = pub()
    e["apparitions"][0].update(state="unpublished", urlElsePublicationName="Tickled Pink")
    row, = timeline(write_pubs(tmp_path, [e]))
    assert row["title"] == "Tickled Pink"
    assert row["url"] is None
    assert row["state"] == "unpublished"


def test_timeline_ignores_other_platforms_and_sorts_by_date(tmp_path):
    late = pub(uuid="late")
    late["apparitions"][0]["apparitionTimestampIfDifferentThanSubmission"] = ts(2026, 5, 1)
    early = pub(uuid="early")
    early["apparitions"] = [
        {"platformName": "pixiv", "state": "published_or_scheduled",
         "urlElsePublicationName": "https://www.pixiv.net/en/artworks/1"},
        early["apparitions"][0]]
    rows = timeline(write_pubs(tmp_path, [late, early]))
    assert [r["uuid"] for r in rows] == ["early", "late"]   # pixiv row dropped, sorted


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

def _render(rows, now=None, anchors=None):
    return render_calendar(rows, tz=PARIS, now=now or ts(2026, 1, 6, 12),
                           anchors=anchors or {})


def row(**over):
    # `thumb` is stamped server-side (server.thumb_maps): the view never hashes.
    r = {"uuid": "u1", "ts": ts(2026, 1, 6), "title": "Haunted Tower",
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
