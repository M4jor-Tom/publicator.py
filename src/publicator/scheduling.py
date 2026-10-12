"""Schedule cadence — ALL weekday/hour/timezone math lives here, server-side.

The browser receives absolute slot instants and counts occupancy by exact
timestamp equality; it never reads its own clock, because
`firefox --private-window` with resist-fingerprinting spoofs Date to UTC.
"""

import time
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

from publicator.entries import PUBLICATION_STATE_PICKED, effective_schedule_ts

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
    """Validated cadence and DA defaults: name/day/hour/per_slot/default_tiers/default_price.
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
        default_tiers = p.get("default_tiers", [])
        if not isinstance(default_tiers, list) or not all(isinstance(t, str) for t in default_tiers):
            raise ValueError("schedule.default_tiers must be a list of tier names")
        default_price = p.get("default_price")
        if default_price is not None and (type(default_price) not in (int, float)
                                         or not 0 <= default_price < float("inf")):
            raise ValueError("schedule.default_price must be a finite nonnegative number")
        key = (WEEKDAY[day], hour)
        if key in seen:
            raise ValueError(f"schedule profiles collide on (day={day}, hour={hour})")
        seen.add(key)
        out.append({"name": str(p.get("name", "default")),
                    "day": WEEKDAY[day], "hour": hour, "per_slot": per_slot,
                    "default_tiers": default_tiers, "default_price": default_price})
    if not out:
        raise ValueError("schedule.profiles is empty")
    return out


def profile_slots(profile: dict, tz: ZoneInfo, start: int) -> list[int]:
    """Ascending epochs of the next SLOT_HORIZON_WEEKS weekly slots for `profile`,
    each at its (weekday, hour) wall-clock in `tz`. Built date-by-date in the zone
    so a slot stays 20:00 local across DST (never a fixed UTC offset). First slot is
    the earliest matching instant strictly after `start`."""
    hour = profile["hour"]
    day = datetime.fromtimestamp(start, tz).date()
    day += timedelta(days=(profile["day"] - day.weekday()) % 7)   # this week's (or today's) weekday
    if datetime(day.year, day.month, day.day, hour, tzinfo=tz).timestamp() <= start:
        day += timedelta(days=7)                                  # today's slot already passed
    slots = []
    for _ in range(SLOT_HORIZON_WEEKS):
        slots.append(int(datetime(day.year, day.month, day.day, hour, tzinfo=tz).timestamp()))
        day += timedelta(days=7)
    return slots


def schedule_data(schedule: dict, now: int | None = None,
                  allowed_tiers: list[str] | None = None) -> list[dict]:
    """Client scheduling payload: each profile's upcoming slots and DA defaults.
    ALL weekday/hour/timezone math lives here (Python zoneinfo) so the browser
    never reads a clock. If supplied, `allowed_tiers` validates profile defaults."""
    tz = zone(schedule)
    start = int(now if now is not None else time.time())
    profiles = schedule_profiles(schedule)
    if allowed_tiers is not None:
        allowed = set(allowed_tiers)
        for p in profiles:
            unknown = set(p["default_tiers"]) - allowed
            if unknown:
                raise ValueError(f"schedule profile {p['name']!r} has unconfigured default tiers: "
                                 f"{sorted(unknown)}")
    return [{"name": p["name"], "per_slot": p["per_slot"],
             "default_tiers": p["default_tiers"], "default_price": p["default_price"],
             "slots": profile_slots(p, tz, start)}
            for p in profiles]


def resolve_ts(fields: dict, tz: ZoneInfo) -> int:
    """Epoch for a save. Prefer the naive 'schedule' wall-clock string resolved in
    the schedule timezone (browser-TZ-proof — a private window spoofs Date to UTC,
    so the client can't be trusted to convert it); fall back to a raw 'scheduleTs'
    epoch (non-UI callers). One of the two must be present."""
    s = fields.get("schedule")
    if s:
        return int(datetime.fromisoformat(s).replace(tzinfo=tz).timestamp())
    if fields.get("scheduleTs"):
        return int(fields["scheduleTs"])
    raise ValueError("schedule required")


def _next_profile_slot(schedule_payload: list[dict], profile_name: str, occupancy: dict[int, int]) -> int | None:
    profile = next((p for p in schedule_payload if p["name"] == profile_name), None)
    if profile is None:
        profile = schedule_payload[0]
    for ts in profile["slots"]:
        if occupancy.get(ts, 0) < profile["per_slot"]:
            return ts
    return None


def repack_publication_dates(publications: list[dict], schedule: dict, now: int | None = None) -> None:
    """Assign automatic PICKED dates in stored order without moving pinned/approved dates."""
    payload = schedule_data(schedule, now=now)
    if not payload:
        return
    default_profile = payload[0]["name"]
    occupancy: dict[int, int] = {}
    for pub in publications:
        if pub.get("state") != PUBLICATION_STATE_PICKED or pub.get("scheduleIsManual"):
            ts = effective_schedule_ts(pub)
            if ts is not None:
                occupancy[int(ts)] = occupancy.get(int(ts), 0) + 1

    for pub in publications:
        if pub.get("state") != PUBLICATION_STATE_PICKED or pub.get("scheduleIsManual"):
            continue
        profile = str(pub.get("scheduleProfile") or default_profile)
        pub["scheduleProfile"] = profile
        pub["scheduleIsManual"] = False
        pub.pop("apparitionTimestampIfDifferentThanSubmission", None)
        ts = _next_profile_slot(payload, profile, occupancy)
        if ts is None:
            continue
        pub["apparitionTimestampIfDifferentThanSubmission"] = ts
        occupancy[ts] = occupancy.get(ts, 0) + 1
