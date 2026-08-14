"""Schedule cadence — ALL weekday/hour/timezone math lives here, server-side.

The browser receives absolute slot instants and counts occupancy by exact
timestamp equality; it never reads its own clock, because
`firefox --private-window` with resist-fingerprinting spoofs Date to UTC.
"""

import json
import time
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

from publicator.entries import STATE_UNPUBLISHED

# ---------------------------------------------------------------------------
# Schedule cadence — all weekday/hour/timezone math is done here server-side and
# shipped to the browser as absolute slot instants (see webui.page.render_page).
# ---------------------------------------------------------------------------

DEFAULT_TZ = "Europe/Paris"       # the wall clock "20:00" is anchored to; overridable via schedule.timezone
SLOT_HORIZON_WEEKS = 52           # ponytail: 1y of weekly slots embedded; bump if you ever queue further out
WEEKDAY = {"monday": 0, "tuesday": 1, "wednesday": 2, "thursday": 3,
            "friday": 4, "saturday": 5, "sunday": 6}   # Python date.weekday()


def zone(schedule: dict) -> ZoneInfo:
    return ZoneInfo(schedule.get("timezone", DEFAULT_TZ))


def schedule_profiles(schedule: dict) -> list[dict]:
    """Validated cadence: [{name, day: <Python weekday, 0=Mon>, hour, per_slot}, ...].
    Reads schedule['profiles']; a flat day/hour/per_slot config becomes one 'default'
    profile, an empty dict the built-in default. Only frequency='weekly' is
    implemented; profiles need distinct (day, hour). Internal to schedule_data —
    the browser never sees day/hour, only the generated slot instants."""
    raw = schedule.get("profiles")
    if raw is None:               # flat day/hour/per_slot config, or {} -> one 'default' profile
        raw = [schedule]
    out, seen = [], set()
    for p in raw:
        freq = p.get("frequency", "weekly")
        if freq != "weekly":
            raise NotImplementedError(f"schedule.frequency {freq!r} not implemented (only 'weekly')")
        day = str(p.get("day", "tuesday")).lower()
        if day not in WEEKDAY:
            raise ValueError(f"schedule.day {day!r} invalid")
        hour = int(p.get("hour", 20))
        per_slot = int(p.get("per_slot", 2))
        if per_slot < 1:
            raise ValueError(f"schedule.per_slot must be >= 1, got {per_slot}")
        key = (WEEKDAY[day], hour)
        if key in seen:
            raise ValueError(f"schedule profiles collide on (day={day}, hour={hour})")
        seen.add(key)
        out.append({"name": str(p.get("name", "default")),
                    "day": WEEKDAY[day], "hour": hour, "per_slot": per_slot})
    if not out:
        raise ValueError("schedule.profiles is empty")
    return out


def profile_slots(profile: dict, zone: ZoneInfo, start: int) -> list[int]:
    """Ascending epochs of the next SLOT_HORIZON_WEEKS weekly slots for `profile`,
    each at its (weekday, hour) wall-clock in `zone`. Built date-by-date in the zone
    so a slot stays 20:00 local across DST (never a fixed UTC offset). First slot is
    the earliest matching instant strictly after `start`."""
    hour = profile["hour"]
    day = datetime.fromtimestamp(start, zone).date()
    day += timedelta(days=(profile["day"] - day.weekday()) % 7)   # this week's (or today's) weekday
    if datetime(day.year, day.month, day.day, hour, tzinfo=zone).timestamp() <= start:
        day += timedelta(days=7)                                  # today's slot already passed
    slots = []
    for _ in range(SLOT_HORIZON_WEEKS):
        slots.append(int(datetime(day.year, day.month, day.day, hour, tzinfo=zone).timestamp()))
        day += timedelta(days=7)
    return slots


def schedule_data(schedule: dict, now: int | None = None) -> list[dict]:
    """Client scheduling payload: per profile, its upcoming canonical slot
    instants. ALL weekday/hour/timezone math lives here (Python zoneinfo) so the
    browser never reads a clock — it only counts occupancy by exact-timestamp
    equality, immune to the private-window UTC spoof."""
    tz = zone(schedule)
    start = int(now if now is not None else time.time())
    return [{"name": p["name"], "per_slot": p["per_slot"],
             "slots": profile_slots(p, tz, start)}
            for p in schedule_profiles(schedule)]


def ts_labels(zone: ZoneInfo, tss) -> dict[int, str]:
    """{ts: 'YYYY-MM-DDTHH:MM'} in the schedule timezone, so datetime-local inputs
    show the intended wall clock regardless of the browser's timezone."""
    return {ts: datetime.fromtimestamp(ts, zone).strftime("%Y-%m-%dT%H:%M")
            for ts in set(tss)}


def existing_ts(json_path: str) -> list[int]:
    """Future timestamps of already-scheduled apparitions (state != unpublished) —
    the occupancy background the client packs new slots around. Unpublished
    (pending) entries are excluded; they ride in the client queue instead.
    Past timestamps are dropped: nextSlotForProfile only compares against slots
    >= now, so they can never occupy a candidate slot, and this keeps the
    embedded __EXISTING_TS__ payload from growing without bound over the years."""
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return []
    out, now = [], time.time()
    for entry in data:
        for app in entry.get("apparitions", []):
            ts = app.get("apparitionTimestampIfDifferentThanSubmission")
            if ts and ts >= now and app.get("state") != STATE_UNPUBLISHED:
                out.append(int(ts))
    return out
