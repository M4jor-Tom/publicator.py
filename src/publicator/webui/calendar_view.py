"""The calendar tab: publications.json rendered as month grids, past + scheduled.

Read-only — the gallery tab still owns editing. Like scheduling.py, every
day/month decision is made here in the schedule timezone: the browser is handed
finished HTML, never an instant to place on a calendar of its own (a private
window spoofs Date to UTC and would file a 00:30 Paris post on the day before).
"""

import calendar
import html
import json
import re
import time
from datetime import date, datetime

# ".../art/Haunted-Tower-1278691961" -> "Haunted-Tower" (trailing id dropped)
SLUG = re.compile(r"/art/(.+?)(?:-\d+)?(?:\?|$)")


def entry_title(name: str, description: str) -> str:
    """A human title for a row. Published apparitions store their DeviantArt URL
    in urlElsePublicationName, so the title is recovered from the slug; a few old
    ones have a bare numeric slug, which falls back to the description."""
    if not name.startswith("http"):
        return name                       # still queued: the plain title
    m = SLUG.search(name)
    slug = m.group(1).replace("-", " ").strip() if m else ""
    if any(c.isalpha() for c in slug):
        return slug
    return description.strip().split("\n")[0]


def timeline(json_path: str) -> list[dict]:
    """Every DeviantArt apparition as a calendar row, ascending by date.
    Rows carry the stored sha512 so the view can name the cached thumbnail
    without hashing (or even touching) the file."""
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return []
    rows = []
    for p in data:
        for app in p.get("apparitions", []):
            if app.get("platformName") != "deviantart":
                continue
            name = app.get("urlElsePublicationName", "")
            f0 = (p.get("files") or [{}])[0]
            rows.append({
                "uuid": p.get("uuid"),
                # Most entries carry their own instant; the oldest fall back to
                # the publication's submission time.
                "ts": int(app.get("apparitionTimestampIfDifferentThanSubmission")
                          or p.get("submissionTimestamp") or 0),
                "title": entry_title(name, p.get("description", "")),
                "url": name if name.startswith("http") else None,
                "state": app.get("state"),
                "basename": f0.get("basename", ""),
                # Most of the back-catalogue stores the legacy key (as
                # images.load_publicated_hashes also has to allow for).
                "sha": f0.get("sha512sum") or f0.get("fileSha512sum") or "",
            })
    return sorted(rows, key=lambda r: r["ts"])


def months_between(first: date, last: date):
    while (first.year, first.month) <= (last.year, last.month):
        yield first.year, first.month
        first = date(first.year + 1, 1, 1) if first.month == 12 \
            else date(first.year, first.month + 1, 1)


def event_html(row: dict, now: int, anchors: dict) -> str:
    """One publication inside a day cell: its thumbnail, linking to DeviantArt
    once published, else to its card in the gallery tab. Rows whose art is no
    longer on disk carry no `thumb` and show their title instead."""
    cls = "ev " + ("upcoming" if row["ts"] >= now else "past")
    title = html.escape(row["title"], quote=True)
    thumb = row.get("thumb")   # stamped server-side; the view never touches disk
    img = (f'<img src="/thumbs/{html.escape(thumb, quote=True)}" alt="{title}"'
           f' title="{title}" loading="lazy" onerror="this.replaceWith(this.alt)">'
           if thumb else title)
    href = row["url"] or (f"#{anchors[row['uuid']]}" if row["uuid"] in anchors else "")
    if not href:
        return f'<span class="{cls}">{img}</span>'
    tab = ' target="_blank" rel="noopener"' if row["url"] else ""
    return f'<a class="{cls}" href="{html.escape(href, quote=True)}"{tab}>{img}</a>'


def month_html(year: int, month: int, by_day: dict, today: date, now: int, anchors: dict) -> str:
    head = "".join(f"<th>{d}</th>" for d in calendar.day_abbr)
    weeks = []
    for week in calendar.Calendar().monthdatescalendar(year, month):
        cells = []
        for d in week:
            if d.month != month:   # spill-over from a neighbour month, which owns it
                cells.append('<td class="day other"></td>')
                continue
            cls = "day today" if d == today else "day"
            evs = "".join(event_html(r, now, anchors) for r in by_day.get(d, []))
            cells.append(f'<td class="{cls}" data-date="{d.isoformat()}">'
                         f'<span class="num">{d.day}</span>{evs}</td>')
        weeks.append("<tr>" + "".join(cells) + "</tr>")
    now_cls = " now" if (today.year, today.month) == (year, month) else ""  # scroll target
    return (f'<section class="month{now_cls}" id="m-{year}-{month:02d}">'
            f"<h3>{calendar.month_name[month]} {year}</h3>"
            f'<table class="cal"><tr>{head}</tr>{"".join(weeks)}</table></section>')


def render_calendar(rows: list[dict], *, tz, now: int | None = None,
                    anchors: dict | None = None) -> str:
    """Month grids from the first publication through the last scheduled one,
    always including the current month. `anchors` maps uuid -> gallery card id,
    which is how a queued entry links back to its editable card."""
    now = int(now if now is not None else time.time())
    anchors = anchors or {}
    today = datetime.fromtimestamp(now, tz).date()
    by_day: dict[date, list] = {}
    for r in rows:
        by_day.setdefault(datetime.fromtimestamp(r["ts"], tz).date(), []).append(r)
    days = list(by_day) + [today]
    return "\n".join(month_html(y, m, by_day, today, now, anchors)
                     for y, m in months_between(min(days), max(days)))
