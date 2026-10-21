import json
import logging
import re
import sys
from pathlib import Path

from .browser import BrowserSession

MAX_LINE = 1024 * 1024
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")


def _read_line(stream):
    line = stream.readline(MAX_LINE + 1)
    if len(line.encode("utf-8")) > MAX_LINE:
        raise ValueError("protocol line exceeds 1 MiB")
    return line


def _validate(command):
    if not isinstance(command, dict) or type(command.get("protocol")) is not int or command.get("protocol") != 1:
        raise ValueError("unsupported protocol")
    kind = command.get("type")
    fields = {
        "open": {"protocol", "type", "login_profile", "work_profile", "headless", "timezone"},
        "submit": {"protocol", "type", "attempt_id", "uuid", "image_path", "title", "description", "tags", "schedule", "tiers", "galleries"},
        "close": {"protocol", "type"},
    }
    if kind not in fields or not fields[kind].issubset(command) or set(command) - fields[kind] - ({"price"} if kind == "submit" else set()):
        raise ValueError("invalid protocol fields")
    if kind == "open":
        if not all(isinstance(command[k], str) and command[k] for k in ("login_profile", "work_profile", "timezone")) or not isinstance(command["headless"], bool):
            raise ValueError("invalid open command types")
    if kind == "submit":
        if not all(isinstance(command[k], str) for k in ("attempt_id", "uuid", "title", "description", "image_path")):
            raise ValueError("invalid submit command types")
        if not UUID.match(command["attempt_id"]) or not UUID.match(command["uuid"]):
            raise ValueError("invalid submit identifiers")
        if not command["image_path"] or Path(command["image_path"]).suffix.lower() == ".webp":
            raise ValueError("invalid or unsupported image path")
        for key in ("tags", "tiers", "galleries"):
            if not isinstance(command[key], list) or any(not isinstance(x, str) for x in command[key]):
                raise ValueError(f"invalid {key}")
        s = command["schedule"]
        if not isinstance(s, dict) or set(s) != {"epoch", "timezone", "year", "month", "day", "hour", "minute"}:
            raise ValueError("invalid schedule fields")
        if any(type(s[k]) is not int for k in ("epoch", "year", "month", "day", "hour", "minute")) or not isinstance(s["timezone"], str):
            raise ValueError("invalid schedule types")
        if not (1 <= s["month"] <= 12 and 1 <= s["day"] <= 31 and 0 <= s["hour"] <= 23 and s["minute"] == 0):
            raise ValueError("invalid schedule parts")
        from datetime import datetime, timezone
        from zoneinfo import ZoneInfo
        expected = datetime(s["year"], s["month"], s["day"], s["hour"], s["minute"], tzinfo=ZoneInfo(s["timezone"])).timestamp()
        if int(expected) != s["epoch"]:
            raise ValueError("schedule epoch does not match wall time")
        if "price" in command and (type(command["price"]) not in (int, float) or command["price"] < 0):
            raise ValueError("invalid price")
    return kind


def _emit(stream, event):
    stream.write(json.dumps(event, separators=(",", ":"), ensure_ascii=False) + "\n")
    stream.flush()


def serve(stdin, stdout, session_factory=BrowserSession):
    session = None
    try:
        line = _read_line(stdin)
        if not line:
            return 1
        command = json.loads(line)
        if _validate(command) != "open":
            raise ValueError("first command must open session")
        session = session_factory(command)
        _emit(stdout, {"protocol": 1, "type": "ready"})
        while True:
            line = _read_line(stdin)
            if not line:
                raise ValueError("unexpected end of protocol stream")
            command = json.loads(line)
            kind = _validate(command)
            if kind == "close":
                return 0
            if kind != "submit":
                raise ValueError("unexpected command")
            event = session.submit(command)
            if event.get("type") not in ("scheduled", "failed", "unknown") or event.get("attempt_id") != command["attempt_id"] or event.get("uuid") != command["uuid"]:
                raise ValueError("session returned mismatched event")
            if event["type"] in ("failed", "unknown") and not isinstance(event.get("error"), str):
                raise ValueError("terminal error event is missing its error")
            event["protocol"] = 1
            _emit(stdout, event)
            if event["type"] in ("failed", "unknown"):
                return 1
    except Exception as exc:
        logging.getLogger(__name__).exception("worker protocol failed")
        try:
            _emit(stdout, {"protocol": 1, "type": "fatal", "error": str(exc)})
        except Exception:
            pass
        return 1
    finally:
        if session is not None:
            session.close()


def main():
    if sys.argv[1:] and sys.argv[1] == "--check-steps":
        import argparse
        parser = argparse.ArgumentParser()
        parser.add_argument("--check-steps", action="store_true")
        parser.add_argument("--skill", required=True)
        args = parser.parse_args()
        skill = Path(args.skill)
        if not skill.is_file():
            print(f"check-steps: skill asset not found: {skill}", file=sys.stderr)
            return 1
        from publicator_da.browser import STEPS, check_steps
        return 0 if check_steps(skill, STEPS) else 1
    return serve(sys.stdin, sys.stdout)


if __name__ == "__main__":
    raise SystemExit(main())
