#!/usr/bin/env python3
"""Unattended native acceptance replay of the Iced UI.

Runs the `ui_acceptance` example's fixture scenarios on a private Xvfb display, drives the real
window with xdotool and asserts what a user could observe: the persisted publications.json, the
fixtures' fake-call logs (loopback HTTP, publish worker, URL opener), files on disk and pixel
probes of window captures. Only disposable fixture roots and injected loopback fakes are used:
no Art data, credentials, login profile or live publisher. Every launch, input, wait, exit and
cleanup is bounded; a failing story keeps its capture, JSON, logs and PID list under the
printed temporary directory (outside Git).

usage: scripts/native-ui-replay.py [--binary PATH] [--keep] [STORY ...]
Without --binary it builds `cargo build --release --example ui_acceptance --features desktop`
first: an unoptimized build spends seconds rasterizing each new glyph on the UI thread, which
leaves the window unresponsive to input for tens of seconds.
"""
import argparse
import ctypes
import datetime
import fcntl
import hashlib
import json
import os
import re
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import zoneinfo
from pathlib import Path

WAIT = 30.0  # default bound, in seconds, for any wait on the app
STORY_BUDGET = 900  # hard bound for one story, including its launches and exits
CONTROL = (0xE9, 0xE9, 0xED)  # browser-grey buttons, pick lists and checked boxes
CONTROL_PRESSED = (0xD0, 0xD0, 0xD7)  # active tab or choice; hovered pick list
HOVER = (0xC7, 0xC7, 0xD9)  # hovered button, checkbox or scrollbar thumb
PAGE = (0x1A, 0x1A, 0x1A)  # page background, as in the gutters between cards
CARD = (0x2A, 0x2A, 0x2A)  # workflow cards and calendar chips
HEADER = (0x11, 0x11, 0x11)  # header and footer bars
WHITE = (0xFF, 0xFF, 0xFF)  # text inputs and editors
NOTICE = (0xFF, 0xF3, 0xCD)  # amber confirmation banners and nearest prompts
WARNING = (0xE0, 0xA8, 0x00)
ACCENT = (0x88, 0xAA, 0xCC)
MUTED = (0x88, 0x88, 0x88)
DAY = (0x22, 0x22, 0x22)
TODAY = (0x4C, 0xAF, 0x50)  # the current month's heading; today's outline
CHIP_HOVER = (0x33, 0x33, 0x33)
PROMPT_EXACT = (0xEE, 0xF4, 0xEE)
PROMPT_TEXT = (0xF7, 0xF7, 0xF7)  # a prompt panel's text lines
RAIL = (0x32, 0x32, 0x32)  # scrollbar rail; also an unchecked checkbox
THUMB = (0x4C, 0x4C, 0x4C)
TABLE = (0x55, 0x55, 0x55)  # the Publish table's border
PARIS = zoneinfo.ZoneInfo("Europe/Paris")
KIRITIMATI = zoneinfo.ZoneInfo("Pacific/Kiritimati")


class Failure(Exception):
    pass


def check(condition, message):
    if not condition:
        raise Failure(message)


def wait_until(predicate, what, timeout=WAIT):
    """Poll until predicate() is truthy; a Failure it raises counts as "not yet"."""
    deadline = time.monotonic() + timeout
    last = None
    while True:
        try:
            value = predicate()
            if value:
                return value
        except Failure as failure:
            last = failure
        if time.monotonic() > deadline:
            detail = f" (last: {last})" if last else ""
            raise Failure(f"timed out after {timeout:.0f}s waiting for {what}{detail}")
        time.sleep(0.1)


def stays(predicate, what, seconds=2.0):
    """Bounded absence check: predicate stays true for `seconds`."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        check(predicate(), f"{what} changed")
        time.sleep(0.1)


def tool(*args, timeout=15, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, timeout=timeout,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs).stdout


# Captures -----------------------------------------------------------------------------------
class Image:
    """A raw RGB window capture with pixel probes and colour-region search."""

    def __init__(self, width, height, data):
        self.w, self.h, self.data = width, height, data
        self.cards = {}  # region -> card boxes: see cards()

    def px(self, x, y):
        offset = (y * self.w + x) * 3
        return tuple(self.data[offset:offset + 3])

    def save(self, path):
        tool("magick", "-size", f"{self.w}x{self.h}", "-depth", "8", "rgb:-", path, input=self.data)

    def runs(self, color, y, x0=0, x1=None):
        """Horizontal [start, end) runs of exactly `color` in row y."""
        x1 = self.w if x1 is None else x1
        row = self.data[(y * self.w + x0) * 3:(y * self.w + x1) * 3]
        found = []
        for match in re.finditer(b"(?:" + re.escape(bytes(color)) + b")+", row):
            start, end = -(-match.start() // 3), match.end() // 3  # snap to whole pixels
            if end > start:
                found.append((x0 + start, x0 + end))
        return found

    def boxes(self, color, region=None, min_w=1, min_h=1):
        """Bounding boxes (x, y, w, h) of 4-connected regions of `color`, in reading order."""
        x0, y0, x1, y1 = clip(region or (0, 0, self.w, self.h), self)
        parent, bounds, previous = {}, {}, []

        def find(node):
            while parent[node] != node:
                parent[node] = parent[parent[node]]
                node = parent[node]
            return node

        for y in range(y0, y1):
            current = []
            for start, end in self.runs(color, y, x0, x1):
                node = (y, start)
                parent[node], bounds[node] = node, (start, y, end, y + 1)
                for other_start, other_end, other in previous:
                    if other_start < end and start < other_end:
                        parent[find(node)] = find(other)
                current.append((start, end, node))
            previous = current
        merged = {}
        for node, (bx0, by0, bx1, by1) in bounds.items():
            box = merged.setdefault(find(node), [bx0, by0, bx1, by1])
            box[:] = min(box[0], bx0), min(box[1], by0), max(box[2], bx1), max(box[3], by1)
        found = [(a, b, c - a, d - b) for a, b, c, d in merged.values()
                 if c - a >= min_w and d - b >= min_h]
        return sorted(found, key=lambda box: (box[1] // 8, box[0]))

    def stems(self, color, region):
        """Glyph stems (≥ 6 px tall, narrow) of text drawn in `color`; anti-aliased edges of
        other text only hit an exact colour in isolated pixels."""
        return [box for box in self.boxes(color, region, 1, 6) if box[2] < 40]


def clip(region, img):
    x0, y0, x1, y1 = region
    return max(0, x0), max(0, y0), min(img.w, x1), min(img.h, y1)


def center(box):
    x, y, w, h = box
    return x + w // 2, y + h // 2


def area(box):
    return (box[0], box[1], box[0] + box[2], box[1] + box[3])


def within(box, region):
    x, y, w, h = box
    x0, y0, x1, y1 = region
    return x >= x0 and y >= y0 and x + w <= x1 and y + h <= y1


def file_color(path):
    """The top-left pixel of a fixture image."""
    return tuple(tool("magick", path, "-crop", "1x1+0+0", "-depth", "8", "rgb:-"))


# Processes ----------------------------------------------------------------------------------
class Display:
    """A private Xvfb display with a window manager, owned and torn down by this process."""

    def __init__(self, work):
        self.work, self.processes, self.report = work, [], None

    def start(self):
        work = self.work
        read, write = os.pipe()
        self.spawn(["Xvfb", "-displayfd", str(write), "-screen", "0", "1600x1600x24",
                    "-nolisten", "tcp"], work / "xvfb", pass_fds=(write,))
        os.close(write)
        # Keep the read end open while Xvfb lives: it may write the number in pieces.
        self.report, number = read, b""
        while not number.endswith(b"\n"):
            ready, _, _ = select.select([read], [], [], WAIT)
            check(ready, "Xvfb did not report a display")
            chunk = os.read(read, 64)
            check(chunk, "Xvfb exited before reporting a display")
            number += chunk
        number = number.decode().strip()
        check(number.isdigit(), f"Xvfb reported {number!r}")
        self.name = f":{number}"
        wait_until(lambda: self.ok("xdpyinfo"), "X server")
        self.spawn(["openbox", "--sm-disable"], work / "openbox", env=self.env())
        wait_until(lambda: self.ok("wmctrl", "-m"), "window manager", 60)
        self.map_typed_characters()
        self.wait_for_mapping()

    def wait_for_mapping(self):
        """Wait until openbox maps a probe window. `wmctrl -m` answers once openbox claims the
        screen, which on a slow machine comes before it handles map requests: the first app's
        window then stays unmapped for good. A probe whose request was dropped is replaced."""
        x11 = ctypes.CDLL("libX11.so.6")
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XDefaultRootWindow.restype = ctypes.c_ulong
        x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
        x11.XCreateSimpleWindow.restype = ctypes.c_ulong
        x11.XCreateSimpleWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong] + [ctypes.c_int] * 2 \
            + [ctypes.c_uint] * 3 + [ctypes.c_ulong] * 2
        for name in ("XMapWindow", "XDestroyWindow"):
            getattr(x11, name).argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        x11.XFlush.argtypes = x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
        display = x11.XOpenDisplay(self.name.encode())
        check(display, "cannot open the display for the mapping probe")
        try:
            def mapped():
                window = x11.XCreateSimpleWindow(display, x11.XDefaultRootWindow(display),
                                                 0, 0, 10, 10, 0, 0, 0)
                x11.XMapWindow(display, window)
                x11.XFlush(display)
                try:
                    return wait_until(lambda: b"IsViewable" in tool(
                        "xwininfo", "-id", hex(window), env=self.env()), "a mapped probe", 5)
                except Failure:
                    return False
                finally:
                    x11.XDestroyWindow(display, window)
                    x11.XFlush(display)
            wait_until(mapped, "the window manager to map windows", 60)
        finally:
            x11.XCloseDisplay(display)

    def map_typed_characters(self):
        """Bind every non-ASCII character this replay types to its own keycode. Otherwise
        xdotool remaps a scratch key per character and reverts it before the app refetches the
        keymap, so the app drops the character."""
        # ponytail: overwrites the top keycodes (unused media keys on Xvfb); widen if a story
        # ever types more distinct characters than the 50 spared here.
        typed = sorted({c for c in Path(__file__).read_text(encoding="utf-8") if ord(c) > 127})
        x11 = ctypes.CDLL("libX11.so.6")
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        handle = x11.XOpenDisplay(self.name.encode())
        check(handle and len(typed) <= 50, f"cannot map {len(typed)} typed characters")
        display = ctypes.c_void_p(handle)
        low, high = ctypes.c_int(), ctypes.c_int()
        x11.XDisplayKeycodes(display, ctypes.byref(low), ctypes.byref(high))
        for offset, char in enumerate(typed):
            code = ord(char) if ord(char) < 0x100 else 0x1000000 + ord(char)
            symbols = (ctypes.c_ulong * 2)(code, code)
            x11.XChangeKeyboardMapping(display, high.value - offset, 2, symbols, 1)
        x11.XCloseDisplay(display)

    def env(self, **extra):
        keep = ("PATH", "LD_LIBRARY_PATH", "LANG", "LOCALE_ARCHIVE", "FONTCONFIG_FILE",
                "FONTCONFIG_PATH", "XDG_DATA_DIRS", "TZDIR")
        env = {key: os.environ[key] for key in keep if key in os.environ}
        home = self.work / "home"
        (home / "runtime").mkdir(parents=True, exist_ok=True)
        os.chmod(home / "runtime", 0o700)
        env.update(DISPLAY=self.name, HOME=str(home), XDG_RUNTIME_DIR=str(home / "runtime"),
                   XDG_SESSION_TYPE="x11", LANG=env.get("LANG", "C.UTF-8"))
        env.update(extra)
        return env

    def ok(self, *command):
        return subprocess.run(list(command), env=self.env(), stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=15).returncode == 0

    def spawn(self, command, log, **kwargs):
        """Start `command` in its own process group, logging to `log`.log and pids.log. It is
        SIGKILLed if this replay dies first, so even a killed replay leaves no display or app."""
        with open(f"{log}.log", "ab") as out:
            process = subprocess.Popen([str(part) for part in command], stdout=out, stderr=out,
                                       start_new_session=True, preexec_fn=die_with_parent, **kwargs)
        self.processes.append(process)
        with open(self.work / "pids.log", "a") as pids:
            pids.write(f"{process.pid} {' '.join(map(str, command))}\n")
        return process

    def close(self):
        for process in reversed(self.processes):
            stop(process)
        if self.report is not None:
            os.close(self.report)


LIBC = ctypes.CDLL(None, use_errno=True)


def die_with_parent():
    LIBC.prctl(1, signal.SIGKILL)  # PR_SET_PDEATHSIG


def stop(process, grace=3.0):
    """SIGTERM, then SIGKILL, the process group; reap. Bounded."""
    for sig in (signal.SIGTERM, signal.SIGKILL):
        if process.poll() is not None:
            break
        try:
            os.killpg(process.pid, sig)
            process.wait(timeout=grace)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            continue
    try:
        process.wait(timeout=grace)
    except subprocess.TimeoutExpired:
        pass


def leaked(root):
    """Live processes whose command line or working directory is under `root`."""
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            cmdline = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
            cwd = os.readlink(entry / "cwd")
        except OSError:
            continue
        if str(root) in cmdline or cwd.startswith(str(root)):
            found.append((int(entry.name), cmdline.strip()))
    return found


class App:
    """One ui_acceptance process and its native window."""

    def __init__(self, display, binary, fixture, scenario, settings=None):
        self.display, self.fixture, self.scenario = display, Path(fixture), scenario
        self.fixture.mkdir(parents=True, exist_ok=True)
        settings_path = self.fixture / "native-fixture.json"
        if settings is not None and not settings_path.exists():
            settings_path.write_text(json.dumps(settings))
        # The calendar's URL opener resolves `xdg-open` on the app's own PATH only.
        fakebin = self.fixture.parent / f"{self.fixture.name}-bin"
        fakebin.mkdir(exist_ok=True)
        opener = fakebin / "xdg-open"
        opener.write_text(f'#!/bin/sh\nprintf "%s\\n" "$1" >> "{self.fixture}/opened-urls.log"\n')
        opener.chmod(0o755)
        self.process = display.spawn(
            [binary, "--fixture-dir", self.fixture, "--scenario", scenario],
            self.fixture.parent / f"{self.fixture.name}-app",
            env=display.env(PATH=f"{fakebin}:{os.environ['PATH']}"), cwd=self.fixture)
        self.wid = None

    def ready(self, size=None):
        try:
            self.wid = wait_until(self.find_window, f"{self.scenario} window", 90)
        except Failure:
            self.report_no_window()
            raise
        self.x("windowactivate", "--sync", self.wid)
        if size:
            self.resize(*size)
        self.park()
        wait_until(lambda: len(tabs(self.shot())) == 5, "loaded header tabs", 90)

    def x(self, *args, timeout=15):
        return tool("xdotool", *args, env=self.display.env(), timeout=timeout).decode()

    def find_window(self):
        check(self.process.poll() is None, f"app exited early with {self.process.returncode}")
        found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(self.process.pid)],
                               env=self.display.env(), stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, timeout=15).stdout.split()
        return found[0].decode() if found else None

    def report_no_window(self):
        """Record what X and the process show when no window turned visible: whether the app
        created an unmapped window, what the window manager manages, and where its threads wait."""
        env, pid = self.display.env(), self.process.pid
        sections = [("own windows", ["xdotool", "search", "--pid", str(pid)]),
                    ("managed windows", ["wmctrl", "-l", "-p"]),
                    ("window tree", ["xwininfo", "-root", "-tree"])]
        with open(self.fixture.parent / f"{self.fixture.name}-no-window.txt", "w") as out:
            for title, command in sections:
                result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
                out.write(f"== {title}\n{result.stdout}{result.stderr}\n")
                if title == "own windows":
                    for wid in result.stdout.split():
                        info = subprocess.run(["xwininfo", "-id", wid], env=env,
                                              capture_output=True, text=True, timeout=15)
                        out.write(info.stdout + info.stderr)
            out.write("== threads (name state wchan)\n")
            for task in sorted(Path(f"/proc/{pid}/task").glob("*")):
                try:
                    status = dict(line.split(":\t", 1) for line in
                                  (task / "status").read_text().splitlines() if ":\t" in line)
                    wchan = (task / "wchan").read_text()
                except OSError:
                    continue
                out.write(f"{task.name} {status.get('Name')} {status.get('State')} {wchan}\n")

    def geometry(self):
        values = dict(line.split("=", 1) for line in self.x("getwindowgeometry", "--shell", self.wid).split())
        return int(values["WIDTH"]), int(values["HEIGHT"])

    def resize(self, width, height):
        self.x("windowsize", self.wid, width, height)
        wait_until(lambda: self.geometry() == (width, height), f"window size {width}x{height}")

    def shot(self):
        data = tool("import", "-silent", "-window", self.wid, "-depth", "8", "ppm:-",
                    env=self.display.env(), timeout=30)
        header = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", data)
        check(header, "window capture is not a binary PPM")
        width, height, pixels = int(header[1]), int(header[2]), data[header.end():]
        check(len(pixels) == width * height * 3, "window capture size mismatch")
        self.last = Image(width, height, pixels)
        return self.last

    def park(self):
        """Move the pointer onto the header so no control renders hovered. Only park once the
        previous input's effect is observed: Iced applies a batch of input events at the latest
        pointer position, so moving away early can cancel a pending click."""
        self.x("mousemove", "--window", self.wid, 2, 2)

    def look(self, predicate, what, timeout=WAIT):
        """Wait for a capture where predicate(img) is truthy; return that value. Captures keep
        the pointer where the last input left it, so probes accept hovered colours."""
        return wait_until(lambda: predicate(self.shot()), what, timeout)

    def click(self, point):
        self.x("mousemove", "--window", self.wid, point[0], point[1], "click", 1)

    def type(self, text):
        self.x("type", "--delay", 30, "--", text, timeout=120)

    def key(self, *keys):
        self.x("key", "--delay", 60, *keys)

    def close_window(self):
        """A window-manager close request, as the title-bar close button sends."""
        tool("wmctrl", "-i", "-c", self.wid, env=self.display.env())

    def wait_exit(self, timeout=WAIT):
        try:
            return self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            raise Failure(f"app did not exit within {timeout:.0f}s") from None

    def alive(self):
        return self.process.poll() is None

    # Persisted and fake-service state ------------------------------------------------------
    @property
    def json_path(self):
        return self.fixture / "publications.json"

    def rows(self):
        return json.loads(self.json_path.read_text())

    def row(self, uuid):
        return next(row for row in self.rows() if row["uuid"] == uuid)

    def digest(self):
        return hashlib.sha256(self.json_path.read_bytes()).hexdigest()

    def lines(self, name):
        path = self.fixture / name
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def edit_rows(self, change):
        """An external, lock-respecting atomic edit of publications.json (another tool)."""
        with open(self.fixture / ".publications.json.lock", "a+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            rows = self.rows()
            change(rows)
            temp = self.json_path.with_suffix(".external")
            temp.write_text(json.dumps(rows, indent=2, ensure_ascii=False))
            os.replace(temp, self.json_path)


def schedule_of(row):
    return row.get("apparitionTimestampIfDifferentThanSubmission", row["submissionTimestamp"])


# Views --------------------------------------------------------------------------------------
TABS = ("Pick", "Schedule", "Describe", "Publish", "Calendar")


def tabs(img):
    """Header tab buttons in order (the header wraps when narrow), each with whether it is
    the active one."""
    bar = (0, 0, img.w, content(img)[1])
    found = [(box, False) for color in (CONTROL, HOVER) for box in img.boxes(color, bar, 20, 15)]
    found += [(box, True) for box in img.boxes(CONTROL_PRESSED, bar, 20, 15)]
    return sorted(found, key=lambda item: (item[0][1] // 20, item[0][0]))


def navigate(app, name):
    index = TABS.index(name)
    header = app.look(lambda img: (found := tabs(img)) and len(found) == 5 and found, "header tabs")
    if not header[index][1]:
        app.click(center(header[index][0]))
    app.look(lambda img: (found := tabs(img)) and len(found) == 5 and found[index][1],
             f"{name} tab to become active")


def content(img):
    """The area between the header and footer bars."""
    top = next(y for y in range(img.h) if img.px(2, y) != HEADER)
    bottom = next(y for y in range(img.h - 1, 0, -1) if img.px(2, y) != HEADER) + 1
    return (0, top, img.w, bottom)


def footer(img):
    """Footer buttons: Refresh, Retry save, Keep editing, Discard drafts and close."""
    found = buttons(img, (0, content(img)[3], img.w, img.h))
    check(len(found) == 4, f"footer shows {len(found)} buttons")
    return found


def press_footer(app, index):
    app.click(center(app.look(lambda img: footer(img), "footer")[index]))


def list_view(img):
    """The scrolled list's region (from its vertical scrollbar) and the scrollbar thumb; when
    everything fits, no thumb and the content area, from the Publish table's top if shown."""
    strip = (img.w - 60, 60, img.w, content(img)[3])
    thumb = img.boxes(THUMB, strip, 6, 10) + [b for b in img.boxes(HOVER, strip, 6, 10) if b[2] <= 12]
    parts = img.boxes(RAIL, strip, 6, 20) + thumb
    if not parts:
        region = content(img)
        table = table_edges(img, region)
        return ((0, table[0][1] - 1, region[2], region[3]) if table else region), None
    top = min(y for _, y, _, _ in parts)
    bottom = max(y + h for _, y, _, h in parts)
    right = max(x + w for x, _, w, _ in parts)
    return (0, top, right, bottom), (thumb[0] if thumb else None)


def jump(app, fraction):
    """Click the scrollbar rail so the list jumps there, as dragging the scrollbar does."""
    try:
        (_, top, right, bottom), thumb = app.look(lambda img: (v := list_view(img))[1] and v,
                                                  "list scrollbar", 5)
    except Failure:
        return  # everything fits: nothing to scroll
    y = max(top + 8, min(bottom - 8, int(top + fraction * (bottom - top))))
    if thumb[1] - 3 <= y <= thumb[1] + thumb[3] + 3:
        return
    app.click((right - 5, y))
    # Loading previews grow their cards, so the thumb drifts after the jump: wait for the
    # jump itself, not for the thumb to stay put.
    app.look(lambda img: list_view(img)[1] != thumb, f"list to jump to {fraction:.0%}")


def scroll_by(app, delta):
    """Scroll the list by about `delta` px. A rail click centres the thumb on it and a thumb
    pixel scrolls viewport/thumb pixels; a target under the thumb is reached from the far end."""
    (_, top, _, bottom), thumb = list_view(app.shot())
    check(thumb, "nothing to scroll")
    y = thumb[1] + thumb[3] / 2 + delta * thumb[3] / (bottom - top)
    check(top <= y <= bottom, f"cannot scroll {delta} px")
    if abs(delta) < 8:
        return
    if thumb[1] - 3 <= y <= thumb[1] + thumb[3] + 3:
        jump(app, 0.0 if y > (top + bottom) / 2 else 1.0)
    jump(app, (y - top) / (bottom - top))


def buttons(img, region, min_w=30):
    """Buttons and pick lists, hovered or not, in reading order."""
    found = [box for color in (CONTROL, HOVER, CONTROL_PRESSED) for box in img.boxes(color, region, min_w, 18)]
    return sorted(found, key=lambda box: (box[1] // 8, box[0]))


def table_edges(img, region):
    """The Publish table's left border at the content padding (x = 20), one segment per row
    from its header down: row separators split it, and its top border anti-aliases."""
    return img.boxes(TABLE, (0, region[1], 40, region[3]), 1, 20)


def cards(img, region):
    """Card boxes in `region`, searched once per capture: previews() and gutters() share them."""
    if region not in img.cards:
        img.cards[region] = img.boxes(CARD, region, 150, 100)
    return img.cards[region]


def clear(img, box, color):
    """Every pixel of box is `color`: no text runs across this gap."""
    return crop(img, box) == bytes(color) * (box[2] * box[3])


def gutters(img, region):
    """The gaps right of cards with a neighbour in their row, inset from anti-aliased edges."""
    listed = cards(img, region)
    return [(a[0] + a[2] + 2, a[1], b[0] - a[0] - a[2] - 4, a[3])
            for a, b in zip(listed, listed[1:]) if abs(a[1] - b[1]) <= 2]


def notice(img, region=None):
    """The amber confirmation banner and its buttons."""
    banners = img.boxes(NOTICE, region or content(img), 200, 30)
    if not banners:
        return None
    banner = banners[0]
    return banner, buttons(img, area(banner), 40)


class Palette:
    """Fixture artwork colours: colour <-> basename."""

    def __init__(self, entries):
        self.name = {tuple(rgb): name for name, rgb in entries}

    @classmethod
    def manifest(cls, path):
        return cls((entry["basename"], entry["rgb"]) for entry in json.loads(Path(path).read_text()))

    @classmethod
    def files(cls, paths):
        return cls((path.name, file_color(path)) for path in paths)

    def blocks(self, img, region):
        """Visible fixture images as (basename, box), in reading order."""
        x0, y0, x1, y1 = clip(region, img)
        colors = {img.px(x, y) for y in range(y0, y1, 5) for x in range(x0, x1, 5)}
        found = []
        for color in colors & self.name.keys():
            boxes = img.boxes(color, region, 6, 6)
            if boxes:
                found.append((self.name[color], max(boxes, key=lambda b: b[2] * b[3])))
        return sorted(found, key=lambda item: (item[1][1] // 12, item[1][0]))


def previews(app, palette, what, timeout=90):
    """Wait until artwork is listed and every card whose top is visible shows a fixture image
    (not "Loading preview…"); return (capture, list region, visible images). Other muted text,
    such as an unknown-prompt label or a placeholder, is legitimate."""

    def loaded(img):
        region, thumb = list_view(img)
        listed = cards(img, region)
        blocks = palette.blocks(img, region)
        if not listed:  # a table (Publish) keeps its previews in the left column
            left = (region[0], region[1], region[0] + (region[2] - region[0]) // 4, region[3])
            return (img, region, blocks) if blocks and not img.stems(MUTED, left) else None
        # A card scrolled partly out starts on the region's top edge, and so does the first row
        # of a list scrolled to its top; only the thumb tells them apart.
        top = region[1] - 1 if thumb and thumb[1] <= region[1] + 1 else region[1] + 3
        for card in listed:
            if card[1] > top and card[1] + 40 < region[3]:  # top not clipped
                if not any(within(box, area(card)) for _, box in blocks):
                    return None
        return img, region, blocks

    return app.look(loaded, f"every visible {what} card to show its own preview", timeout)


def assert_order(blocks, expected, what):
    """Visible images, in reading order, are one contiguous run of the list's own order."""
    check(blocks, f"no {what} artwork visible")
    names = [name for name, _ in blocks]
    check(len(set(names)) == len(names), f"{what} shows an image twice: {names}")
    check(all(name in expected for name in names), f"{what} shows foreign artwork {names}")
    positions = [expected.index(name) for name in names]
    run = list(range(positions[0], positions[0] + len(positions)))
    check(positions == run, f"{what} cards show artwork out of order: {positions}")
    return positions


def card_of(img, region, name, palette):
    """The card (box) that shows fixture image `name`."""
    for card in cards(img, region):
        if any(n == name and within(box, area(card)) for n, box in palette.blocks(img, area(card))):
            return card
    return None


def wide_buttons(img, card):
    return [box for box in buttons(img, area(card)) if box[2] > card[2] * 0.8]


def inputs(img, region):
    return img.boxes(WHITE, region, 40, 20)


def checkboxes(img, region):
    """Checked (grey) and unchecked (dark) boxes, in reading order, with their state."""
    found = [(box, True) for color in (CONTROL, HOVER) for box in img.boxes(color, region, 10, 10)
             if box[2] <= 20 and box[3] <= 20]
    found += [(box, False) for box in img.boxes(RAIL, region, 10, 10) if box[2] <= 20 and box[3] <= 20]
    # Not hovered, an unchecked box is only its 1 px outline, which anti-aliases to a dark grey
    # that depends on the box's sub-pixel position: pair left and right edges of any such grey.
    edges = [box for grey in outline_greys(img, region)
             for box in img.boxes(grey, region, 1, 10) if box[2] <= 2 and box[3] <= 20]
    pairs = [(x, y, x2 + w2 - x, h) for x, y, _, h in edges
             for x2, y2, w2, _ in edges if 12 <= x2 - x <= 18 and abs(y2 - y) <= 1]
    found += [(box, False) for box in pairs if hollow(img, box)]
    unique = []
    for box, state in sorted(found, key=lambda item: (item[0][1] // 8, item[0][0])):
        cx, cy = center(box)
        if all(abs(cx - center(b)[0]) > 4 or abs(cy - center(b)[1]) > 4 for b, _ in unique):
            unique.append((box, state))
    return unique


def hollow(img, box):
    """An outline's inside is one flat fill, unlike the space between two glyph strokes."""
    x, y, w, h = box
    cx, cy = center(box)
    fill = img.px(cx, cy)
    return all(img.px(i, cy) == fill for i in range(x + 3, x + w - 3)) and \
        all(img.px(cx, j) == fill for j in range(y + 1, y + h - 1))


def outline_greys(img, region):
    """Greys between the page and a checkbox outline's full colour present in `region`."""
    x0, y0, x1, y1 = clip(region, img)
    found = set()
    for y in range(y0, y1):
        row = img.data[(y * img.w + x0) * 3:(y * img.w + x1) * 3]
        found.update(row[i:i + 3] for i in range(0, len(row), 3))
    return [tuple(c) for c in found if c[0] == c[1] == c[2] and 0x22 <= c[0] <= 0x48]


def choose(app, pick_list, index, count):
    """Open a pick list and click its `index`th of `count` options. The open menu is a #323232
    panel under the list with the current option highlighted."""
    x, y, w, h = pick_list
    app.click(center(pick_list))
    below = (x, y + h, x + w, y + h + 40 * count)

    def menu(img):
        parts = [b for color in (RAIL, HOVER) for b in img.boxes(color, below, w - 10, 10)]
        if not parts:
            return None
        top, bottom = min(b[1] for b in parts), max(b[1] + b[3] for b in parts)
        return (top, bottom) if bottom - top > 20 * count else None

    top, bottom = app.look(menu, "pick list menu")
    app.click((x + 12, int(top + (bottom - top) * (index + 0.5) / count)))
    app.look(lambda img: not menu(img), "pick list menu to close")


def type_into(app, box, text, lines=None):
    """Focus an input, select all, then type (lines joined by Return in an editor)."""
    app.click((box[0] + 8, box[1] + box[3] // 2))
    app.key("ctrl+a")
    for index, line in enumerate(lines or [text]):
        if index:
            app.key("Return")
        app.type(line)


# Stories ------------------------------------------------------------------------------------
STORIES = []


def story(function):
    STORIES.append(function)
    return function


class Context:
    """One story's directory, apps and failure evidence."""

    def __init__(self, display, binary, root):
        self.display, self.binary, self.root = display, binary, root
        self.root.mkdir(parents=True)
        self.apps = []

    def launch(self, scenario, fixture="fixture", settings=None, size=None):
        app = App(self.display, self.binary, self.root / fixture, scenario, settings)
        self.apps.append(app)
        app.ready(size)
        return app

    def evidence(self):
        for index, app in enumerate(self.apps):
            if app.alive() and app.wid:
                try:
                    if getattr(app, "last", None):
                        app.last.save(self.root / f"failure-{index}-last-probed.png")
                    app.shot().save(self.root / f"failure-{index}.png")
                except Exception as error:  # evidence is best effort; the failure is reported
                    print(f"  capture failed: {error}")

    def close(self):
        for app in self.apps:
            stop(app.process)
        remaining = leaked(self.root)
        for pid, _ in remaining:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        check(not remaining, f"processes outlived the story: {remaining}")


def close_and_wait(app, timeout=WAIT):
    app.close_window()
    code = app.wait_exit(timeout)
    check(code == 0, f"app exited with {code}")


def phase_names(app, states, keep=lambda row: True):
    return [row["files"][0]["basename"] for row in app.rows() if row["state"] in states and keep(row)]


TAB_STATES = {"Schedule": ("PICKED", "SCHEDULED"), "Describe": ("SCHEDULED",),
              "Publish": ("DESCRIBED", "APPEARED")}


def listed_names(app, tab):
    """The artwork a workflow tab lists, in store order; Publish skips sent DeviantArt cells."""
    return phase_names(app, TAB_STATES[tab], lambda row: tab != "Publish"
                       or (da(row) or {}).get("state", "unpublished") == "unpublished")


SMOKE = {"empty": "Schedule", "history": "Publish", "mixed": "Describe", "publish": "Publish"}


@story
def launch_and_close_every_scenario(ctx):
    """Each fixture scenario opens, renders its data without writing, and exits on a window
    close within the bound, leaving no process behind."""
    for scenario in ("empty", "history", "mixed", "publish", "calendar", "prompt-search",
                     "prompt-search-missing-archive", "gallery"):
        app = ctx.launch(scenario, fixture=scenario)
        before = app.digest()
        if scenario in SMOKE:
            tab = SMOKE[scenario]
            navigate(app, tab)
            expected = listed_names(app, tab)
            palette = Palette.files(app.fixture.glob("art/*.png"))
            if expected:
                app.look(lambda img: (names := [n for n, _ in palette.blocks(img, content(img))])
                         and names == expected[:len(names)], f"{scenario} {tab} artwork {expected}")
            elif tab == "Publish":  # a rendered table whose borders span only its header row
                app.look(lambda img: (edges := table_edges(img, content(img)))
                         and edges[-1][1] + edges[-1][3] - edges[0][1] < 80,
                         f"an empty {scenario} Publish table")
            else:
                app.look(lambda img: not cards(img, content(img)), f"an empty {scenario} {tab}")
        elif scenario == "calendar":
            navigate(app, "Calendar")
            app.look(lambda img: len(img.boxes(DAY, content(img), 40, 40)) > 20, "calendar month grid")
        check(app.digest() == before, f"{scenario}: rendering changed publications.json")
        close_and_wait(app)


def pick_controls(img):
    """The Pick controls (limit list, then Search): the first grid cell, left of and above
    every candidate card."""
    region = content(img)
    below = [card[1] for card in cards(img, region) if card[0] < 100]
    first_card_x = min([card[0] for card in cards(img, region) if card[0] > 100] or [img.w])
    area_ = (0, region[1], first_card_x, min(below or [region[3]]))
    found = buttons(img, area_, 40)
    return found if len(found) >= 2 else None


def search(app, limit_index=None):
    """Press Search (optionally choosing a candidate limit first) at the top of Pick, and wait
    for its result line: until then the screen still shows the previous list."""
    navigate(app, "Pick")
    jump(app, 0.0)
    if limit_index is not None:
        choose(app, app.look(pick_controls, "Pick controls")[0], limit_index, 5)
    status = header_status(app.shot())
    app.click(center(app.look(pick_controls, "Pick controls")[-1]))
    app.look(lambda img: header_status(img) != status, "the search's result line")


def list_names(app, palette, what):
    """Page down the list; return every artwork seen, asserting at each stop that every visible
    card shows its own preview. A rail click just past the thumb centres the thumb there: half a
    viewport per stop. Cards growing as they load drift the view further, which can skip a row,
    so a stop sharing no card with the previous one backs up: each view is a contiguous run of
    the list, so overlapping views leave no gap. (xdotool wheel clicks do not reach Iced on Xvfb.)"""
    jump(app, 0.0)
    seen, previous, backups = [], [], 0
    while True:
        names = [name for name, _ in previews(app, palette, what)[2]]
        app.park()  # a hovered list restyles its scrollbar
        (_, top, right, bottom), thumb = list_view(app.shot())
        if previous and not set(names) & set(previous):
            backups += 1
            check(backups <= 10 and thumb, f"{what} keeps skipping rows")
            jump(app, (thumb[1] - 5 - top) / (bottom - top))
            continue
        seen += [name for name in names if name not in seen]
        previous = names
        if thumb is None or thumb[1] + thumb[3] + 5 > bottom - 5:  # rail padding: the end
            return seen
        jump(app, (thumb[1] + thumb[3] + 5 - top) / (bottom - top))


def search_paths(app):
    """The Pick search-path checkboxes (scrolled into view) with their states."""
    jump(app, 0.0)
    return app.look(lambda img: checkboxes(img, (0, 0, 400, img.h)), "search path checkboxes")


@story
def pick_search_limit_subset_selection_and_delete(ctx):
    app = ctx.launch("gallery", size=(1580, 1550))  # five columns, tall: fewer stops
    palette = Palette.manifest(app.fixture / "gallery-fixture-manifest.json")
    art = sorted(path.name for path in (app.fixture / "art").glob("gallery-*.png"))
    more = sorted(path.name for path in (app.fixture / "more").glob("*.png"))
    every = set(art + more)
    search(app)  # startup limit 25, every configured root selected
    first = list_names(app, palette, "Pick")
    check(len(first) == 25 and set(first) <= every, f"limit 25 listed {len(first)}")
    search(app, limit_index=2)  # 50
    second = list_names(app, palette, "Pick")
    check(set(second) == every, f"limit 50 listed {len(second)} of {len(every)}; missing {sorted(every - set(second))}")
    search(app, limit_index=0)  # 0: an empty search
    app.look(lambda img: not cards(img, content(img)), "an empty candidate list")
    # Subset: untick the first configured root, keep `more`.
    boxes = search_paths(app)
    check(len(boxes) == 2 and all(state for _, state in boxes), f"search paths {boxes}")
    app.click(center(boxes[0][0]))
    app.look(lambda img: not checkboxes(img, (0, 0, 400, img.h))[0][1], "art unticked")
    search(app, limit_index=4)  # 100
    subset = list_names(app, palette, "Pick subset")
    check(sorted(subset) == more, f"subset listed {subset}")
    app.click(center(search_paths(app)[0][0]))
    app.look(lambda img: checkboxes(img, (0, 0, 400, img.h))[0][1], "art ticked")
    search(app)
    jump(app, 0.0)
    img, region, blocks = previews(app, palette, "Pick")
    # Selection: the card showing image A is the candidate A that becomes PICKED.
    name, box = blocks[0]
    card = card_of(img, region, name, palette)
    pick_button, delete_button = wide_buttons(img, card)[:2]
    app.click(center(pick_button))
    wait_until(lambda: name in phase_names(app, ("PICKED",)), f"{name} to be picked")
    app.look(lambda img: tabs(img)[0][1], "Pick to stay open after a pick")
    jump(app, 0.0)
    img, region, blocks = previews(app, palette, "Pick")
    check(name not in [n for n, _ in blocks], f"picked {name} is still offered in Pick")
    # Delete: cancel keeps the file; confirm removes it and its card. Narrower, the banner's long
    # path must wrap rather than push its buttons out of the window or run under them.
    # From the list's end, the confirmation must still show: it stays above the scrolled list.
    app.resize(1100, 1000)
    jump(app, 1.0)
    img, region, blocks = previews(app, palette, "Pick")
    name = next(n for n, _ in reversed(blocks) if card_of(img, region, n, palette))
    target = app.fixture / ("art" if name in art else "more") / name
    before = app.digest()
    for confirm in (False, True):
        card = card_of(app.shot(), region, name, palette)
        app.click(center(wide_buttons(app.shot(), card)[1]))
        banner, found = app.look(notice, "delete confirmation")
        # Whole buttons: "Cancel", the narrower, is about 72 px wide.
        check(len(found) == 2 and min(b[2] for b in found) >= 60,
              f"the delete confirmation clips its buttons {found} at {banner}")
        if confirm:  # narrower than the path's digest segment: it wraps beside the buttons
            app.resize(560, 1000)
            banner, found = app.look(lambda img: (n := notice(img)) and len(n[1]) == 2 and n,
                                     "delete confirmation at 560 px")
            gap = (found[0][0] - 6, banner[1] + 2, 4, banner[3] - 4)
            check(clear(app.last, gap, NOTICE), f"the delete path runs under its buttons {found}")
        confirm_button, cancel_button = found
        app.click(center(confirm_button if confirm else cancel_button))
        app.look(lambda img: not notice(img), "the confirmation to close")
        check(target.exists() != confirm, f"{target} exists={target.exists()} after confirm={confirm}")
    app.look(lambda img: name not in [n for n, _ in palette.blocks(img, content(img))],
             f"deleted {name} to leave the list")
    check(app.digest() == before, "deleting an unpicked file changed publications.json")
    # Each tab keeps its own scroll: Pick resumes where it was left after visiting Schedule.
    jump(app, 0.5)
    previews(app, palette, "Pick")
    app.park()
    thumb = list_view(app.shot())[1]
    navigate(app, "Schedule")
    navigate(app, "Pick")
    app.park()
    app.look(lambda img: list_view(img)[1] == thumb, "Pick to resume its scroll")
    close_and_wait(app)


@story
def schedule_custom_preset_refresh_and_persistence(ctx):
    app = ctx.launch("gallery")
    palette = Palette.manifest(app.fixture / "gallery-fixture-manifest.json")
    before = app.digest()
    navigate(app, "Schedule")
    expected = listed_names(app, "Schedule")
    img, region, blocks = previews(app, palette, "Schedule")
    assert_order(blocks, expected, "Schedule")
    jump(app, 0.5)
    previews(app, palette, "Schedule")
    check(app.digest() == before, "rendering and scrolling Schedule wrote publications.json")
    jump(app, 0.0)
    img, region, blocks = previews(app, palette, "Schedule")
    rows = [row for row in app.rows() if row["state"] in ("PICKED", "SCHEDULED")]
    # Custom wall time in the configured zone (Europe/Paris), on the first picked card.
    card = card_of(img, region, expected[0], palette)
    date, time_ = inputs(img, area(card))[:2]
    type_into(app, date, "2031-02-03")
    type_into(app, time_, "04:05")
    app.click(center(wide_buttons(app.shot(), card)[0]))
    wanted = int(datetime.datetime(2031, 2, 3, 4, 5, tzinfo=PARIS).timestamp())
    wait_until(lambda: schedule_of(app.row(rows[0]["uuid"])) == wanted, "custom wall time saved")
    # Preset on the second card: the named profile's next Saturday 09:00 in Paris.
    img, region, _ = previews(app, palette, "Schedule")
    card = card_of(img, region, expected[1], palette)
    preset = [box for box in buttons(img, area(card)) if box[2] < card[2] * 0.8][0]
    choose(app, preset, 1, 2)
    wait_until(lambda: app.row(rows[1]["uuid"]).get("scheduleProfile") == "Weekend morning",
               "preset saved")
    local = datetime.datetime.fromtimestamp(schedule_of(app.row(rows[1]["uuid"])), PARIS)
    check((local.weekday(), local.hour, local.minute) == (5, 9, 0), f"preset scheduled {local}")
    # Refresh shows an external edit and discards unsaved custom input.
    img, region, _ = previews(app, palette, "Schedule")
    card = card_of(img, region, expected[2], palette)
    date = inputs(img, area(card))[0]
    pristine = crop(img, date)
    type_into(app, date, "2032-12-31")
    app.look(lambda img: crop(img, date) != pristine, "typed custom date")
    shifted = schedule_of(rows[3]) + 7 * 86400
    app.edit_rows(lambda saved: next(r for r in saved if r["uuid"] == rows[3]["uuid"]).update(
        apparitionTimestampIfDifferentThanSubmission=shifted))
    external = app.digest()
    press_footer(app, 0)
    app.look(lambda img: crop(img, date) == pristine, "Refresh to discard the unsaved date")
    app.click(center(wide_buttons(app.shot(), card)[0]))  # Save Custom with nothing typed
    stays(lambda: app.digest() == external, "publications.json after Save Custom without input")
    close_and_wait(app)
    # Restart: saved dates persist and the external edit is not reverted.
    app = ctx.launch("gallery")
    check(schedule_of(app.row(rows[0]["uuid"])) == wanted, "custom date lost on restart")
    check(schedule_of(app.row(rows[3]["uuid"])) == shifted, "external edit lost")
    close_and_wait(app)


@story
def schedule_and_describe_tiles_delete_their_publication(ctx):
    app = ctx.launch("gallery", size=(1580, 1550))  # tall: whole cards, Delete… included
    palette = Palette.manifest(app.fixture / "gallery-fixture-manifest.json")
    for tab in ("Schedule", "Describe"):
        navigate(app, tab)
        jump(app, 0.0)
        img, region, _ = previews(app, palette, tab)
        name = listed_names(app, tab)[0]
        row = next(r for r in app.rows() if r["files"][0]["basename"] == name)
        artwork = next(app.fixture.rglob(name))
        before = app.digest()
        for confirm in (False, True):
            card = card_of(app.shot(), region, name, palette)
            check(card, f"{tab} shows no card for {name}")
            app.click(center(wide_buttons(app.shot(), card)[-1]))  # Delete…, the last button
            banner, found = app.look(lambda img: (n := notice(img)) and len(n[1]) == 2 and n,
                                     f"{tab} delete confirmation")
            confirm_button, cancel_button = found
            app.click(center(confirm_button if confirm else cancel_button))
            app.look(lambda img: not notice(img), f"the {tab} confirmation to close")
            if not confirm:
                stays(lambda: app.digest() == before, f"{tab} cancel left publications.json")
        wait_until(lambda: all(r["uuid"] != row["uuid"] for r in app.rows()),
                   f"{tab} deletion of {name} to persist")
        check(artwork.exists(), f"deleting the {tab} entry removed its artwork {artwork}")
        app.look(lambda img: name not in [n for n, _ in palette.blocks(img, content(img))],
                 f"deleted {name} to leave {tab}")
    close_and_wait(app)


def crop(img, box):
    x, y, w, h = box
    return b"".join(img.data[((y + r) * img.w + x) * 3:((y + r) * img.w + x + w) * 3] for r in range(h))


class InodeWatch:
    """Counts atomic replacements of a file (a new inode each) while active."""

    def __init__(self, path):
        self.path, self.inodes, self.running = path, set(), True
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while self.running:
            try:
                self.inodes.add(os.stat(self.path).st_ino)
            except FileNotFoundError:
                pass
            time.sleep(0.01)

    def stop(self):
        self.running = False
        self.thread.join(timeout=5)
        return len(self.inodes) - 1


@story
def describe_unicode_debounce_save_failure_retry_and_restart(ctx):
    app = ctx.launch("gallery", size=(1100, 1000))  # the first card's Save draft on screen
    palette = Palette.manifest(app.fixture / "gallery-fixture-manifest.json")
    navigate(app, "Describe")
    expected = listed_names(app, "Describe")
    target = next(row for row in app.rows() if row["files"][0]["basename"] == expected[0])
    img, region, blocks = previews(app, palette, "Describe")
    assert_order(blocks, expected, "Describe")

    def fields():
        """The first card and its title and description inputs, located afresh: a header
        status line appearing after a save shifts the whole list down."""
        img, region, _ = previews(app, palette, "Describe")
        card = card_of(img, region, expected[0], palette)
        return (card, *inputs(img, area(card))[:2])

    card, title, editor = fields()
    # Debounced title: one burst of keystrokes is one save, with Unicode and literal markup.
    typed = "Édité ✓ 雪 <i>raw</i> & more"
    watch = InodeWatch(app.json_path)
    type_into(app, title, typed)
    wait_until(lambda: app.row(target["uuid"])["title"] == typed, "debounced title save")
    saves = watch.stop()
    check(1 <= saves <= 2, f"{len(typed)} keystrokes caused {saves} saves")
    # Multiline description: newlines and literal markup persist verbatim.
    lines = ["Line A <b>not bold</b> 雪", "", "Line C — ünïcödé [x](y)"]
    card, title, editor = fields()
    type_into(app, editor, None, lines)
    wait_until(lambda: app.row(target["uuid"])["description"] == "\n".join(lines), "description save")
    # Explicit save: Save draft persists before the debounce would.
    card, title, editor = fields()
    type_into(app, title, "Explicit save")
    app.click(center(wide_buttons(app.shot(), card)[3]))
    wait_until(lambda: app.row(target["uuid"])["title"] == "Explicit save", "explicit save", 5)
    # Failure keeps the dirty field; Retry save lands it once the store is writable again.
    stored = app.digest()
    status = header_status(app.shot())
    app.fixture.chmod(0o555)
    try:
        type_into(app, title, "Kept through failure ✓")
        app.look(lambda img: header_status(img) != status, "a save failure message", 20)
        stays(lambda: app.digest() == stored, "publications.json after a failed save", 2)
    finally:
        app.fixture.chmod(0o755)
    press_footer(app, 1)  # Retry save
    wait_until(lambda: app.row(target["uuid"])["title"] == "Kept through failure ✓", "retried save")
    # The file is written before the app shows it: locate the inputs only once the earlier
    # one-line success status replaces the (wrapped, per-attempt) failures, or the header's
    # shrink moves them away from the click.
    app.look(lambda img: header_status(img) == status, "the retried save's status")
    # Closing with an unsaved edit saves it before exiting.
    card, title, editor = fields()
    type_into(app, title, "Saved by close")
    close_and_wait(app)
    check(app.row(target["uuid"])["title"] == "Saved by close", "close lost the dirty title")
    app = ctx.launch("gallery")
    check(app.row(target["uuid"])["description"] == "\n".join(lines), "restart lost the description")
    navigate(app, "Describe")
    previews(app, palette, "Describe")
    close_and_wait(app)


def header_status(img):
    """Pixels of the header's status message, right of the tabs."""
    right = max(x + w for (x, _, w, _), _ in tabs(img))
    return crop(img, (right + 4, 0, img.w - right - 4, content(img)[1]))


def describe_controls(img, region):
    """Provider/model choices and Generate marked, top to bottom, in the first grid cell."""
    first_card = min(cards(img, region), key=lambda c: (c[1], c[0]))
    found = [box for box in buttons(img, region, 100) if box[0] + box[2] < first_card[0]]
    return sorted(found, key=lambda b: b[1])


def pressed(img, box):
    return img.px(box[0] + 4, box[1] + box[3] // 2) == CONTROL_PRESSED


@story
def describe_model_install_consent(ctx):
    app = ctx.launch("gallery", settings={"installed_models": ["gemma4:31b"]})
    palette = Palette.manifest(app.fixture / "gallery-fixture-manifest.json")
    navigate(app, "Describe")
    marked = next(row for row in app.rows() if row["state"] == "SCHEDULED" and "TODO_GENERATE" in row["title"])
    img, region, _ = previews(app, palette, "Describe")
    openrouter, local, small, large, _ = describe_controls(img, region)
    for choice in (local, small):
        app.click(center(choice))
        app.look(lambda img: pressed(img, choice), "provider/model choice")

    def generate():
        img, region, _ = previews(app, palette, "Describe")
        card = card_of(img, region, marked["files"][0]["basename"], palette)
        app.click(center(wide_buttons(img, card)[0]))  # Generate one
        return app.look(notice, "install consent")

    def pulls():
        return [call for call in app.lines("http-calls.jsonl") if call["path"] in ("/api/pull", "/api/chat")]

    _, (confirm, cancel) = generate()
    app.click(center(cancel))
    app.look(lambda img: not notice(img), "consent to close")
    stays(lambda: not pulls(), "no pull after refusal")
    # Stale consent: the app refuses a newer model choice while consent is pending, so the
    # consent can only act on the selection it names.
    _, (confirm, cancel) = generate()
    app.click(center(large))
    app.park()
    stays(lambda: pressed(app.shot(), small) and not pressed(app.shot(), large),
          "the model choice while consent is pending", 2)
    stays(lambda: not pulls(), "no pull or generation while consent is pending", 1)
    app.click(center(cancel))
    app.look(lambda img: not notice(img), "consent to close")
    # Fresh consent for the current choice installs that model, then generates.
    app.click(center(small))
    app.look(lambda img: pressed(img, small), "model choice")
    _, (confirm, cancel) = generate()
    app.click(center(confirm))
    wait_until(lambda: [c["path"] for c in pulls()] == ["/api/pull", "/api/chat"], "pull then chat")
    check(all(c["model"] == "gemma4:26b-a4b" for c in pulls()), f"wrong model {pulls()}")
    wait_until(lambda: app.row(marked["uuid"])["title"].startswith("Fixture generated"), "generated title")
    close_and_wait(app)


def generating(img, region):
    """Cards carrying the accent-filled `Generating…` label (an active choice's accent border
    is hollow: its inside is the button grey)."""
    labels = [box for box in img.boxes(ACCENT, region, 60, 14) if img.px(box[0] + 3, box[1] + box[3] // 2) == ACCENT]
    return [card for card in cards(img, region) if any(within(box, area(card)) for box in labels)]


@story
def describe_batch_progress_per_card_and_summary(ctx):
    # The first chat reply fails and the third waits for the release below, so the running
    # state is checked while it cannot move; the long deadline keeps the held record alive.
    settings = {"installed_models": ["gemma4:31b"], "local_failures": 1, "local_hold": 3,
                "ai_timeout_ms": 60000}
    app = ctx.launch("gallery", settings=settings, size=(1500, 1400))  # two card rows
    palette = Palette.manifest(app.fixture / "gallery-fixture-manifest.json")
    marked = [row for row in app.rows() if row["state"] == "SCHEDULED" and "TODO_GENERATE" in row["title"]]

    def keep_three(rows):
        for row in rows:
            if row["uuid"] in {r["uuid"] for r in marked[3:]}:
                row["title"], row["description"] = "Manual title", "Manual description"

    app.edit_rows(keep_three)
    navigate(app, "Describe")
    press_footer(app, 0)  # Refresh shows the three-record batch
    names = [row["files"][0]["basename"] for row in marked[:3]]
    img, region, _ = previews(app, palette, "Describe")
    check(all(card_of(img, region, name, palette) for name in names), "a batch card is off screen")
    openrouter, local, small, large, generate = describe_controls(img, region)[:5]  # not row 2
    for choice in (local, large):  # the installed model
        app.click(center(choice))
        app.look(lambda img: pressed(img, choice), "provider/model choice")
    summary = (generate[0], generate[1] + generate[3] + 2, generate[2], 24)
    idle = crop(app.shot(), summary)
    app.click(center(generate))
    app.park()
    wait_until(lambda: (app.fixture / "local-chat-held").exists(), "the third chat reply held")
    # The batch runs in its Admitted order, which its chat requests follow: the record held
    # is the marked one not yet requested.
    uuid_of = {row["files"][0]["basename"]: row["uuid"] for row in marked[:3]}
    asked = [uuid_of[palette.name[tuple(call["image_rgb"])]]
             for call in app.lines("http-calls.jsonl") if call["path"] == "/api/chat"]
    check(len(asked) == 2, f"chat requests before the hold: {asked}")
    batch = asked + [uuid for uuid in uuid_of.values() if uuid not in asked]
    held = app.row(batch[2])["files"][0]["basename"]

    def one_active(img):
        region, _ = list_view(img)
        active = generating(img, region)
        return len(active) == 1 and card_of(img, region, held, palette) == active[0] and img

    running = app.look(one_active, "exactly the held card generating")
    running.save(ctx.root / "running.png")
    check(crop(running, summary) != idle, "no batch summary while running")

    second, third = (app.row(uuid) for uuid in batch[1:])
    check(second["state"] == "DESCRIBED" and "TODO_GENERATE" in third["title"],
          "an earlier record not persisted while the batch runs")
    (app.fixture / "local-chat-release").write_text("release")
    wait_until(lambda: app.row(batch[2])["state"] == "DESCRIBED", "the last record")

    def settled(img):
        """No card generating, and both described records still listed, labelled Generated,
        as read-only cards (no inputs) once the last reload lands."""
        region, _ = list_view(img)
        done = [card_of(img, region, name, palette) for name in names[1:]]
        return (not generating(img, region) and all(done)
                and all(img.stems(TODAY, area(card)) and not inputs(img, area(card)) for card in done)
                and img)

    finished = app.look(settled, "no card generating and Generated cards kept until Refresh")
    finished.save(ctx.root / "finished.png")
    check(crop(finished, summary) != crop(running, summary), "the summary did not change")
    failed = app.row(batch[0])
    check(failed["state"] == "SCHEDULED" and "TODO_GENERATE" in failed["title"], f"failed record {failed}")
    close_and_wait(app)


def publish_row(img, palette, name):
    """The Publish table row (box) showing artwork `name`, from its thumbnail to the next one.
    Located per capture: editing options grows the row by its Save options button."""
    region, _ = list_view(img)
    blocks = palette.blocks(img, region)
    for index, (found, box) in enumerate(blocks):
        if found == name:
            bottom = blocks[index + 1][1][1] - 12 if index + 1 < len(blocks) else region[3]
            return (region[0], box[1] - 12, region[2] - region[0], bottom - box[1] + 12)
    raise Failure(f"no Publish row shows {name}")


def publish_controls(img):
    """Publish selected (and Cancel after current) above the table."""
    region, _ = list_view(img)
    top = content(img)[1]
    return buttons(img, (0, top, img.w, region[1]), 40)


def row_cells(img, row):
    """The DeviantArt column's checkboxes for a table row: Select, then tiers and galleries."""
    x, y, w, h = row
    return checkboxes(img, (x + w // 2, y, x + w - 20, y + h))  # not the scrollbar


@story
def publish_options_keyboard_drift_failure_retry_and_reconcile(ctx):
    settings = {"worker_outcomes": ["failed", "scheduled", "unknown", "scheduled"]}
    app = ctx.launch("publish", settings=settings)
    name_of = {row["uuid"]: row["files"][0]["basename"] for row in app.rows()}
    palette = Palette.files(app.fixture.glob("art/*.png"))
    first, second = (f"00000000-0000-4000-8000-{n:012}" for n in (20, 21))

    def row(img, uuid_):
        return area(publish_row(img, palette, name_of[uuid_]))

    def selected(img, uuid_):
        """The row's Select box is ticked: the app's reloaded rows, never an optimistic state."""
        return row_cells(img, publish_row(img, palette, name_of[uuid_]))[0][1]

    navigate(app, "Publish")
    img, region, _ = previews(app, palette, "Publish")
    # Options: tick Fixture Premium on the first row, then save.
    app.click(center(row_cells(img, publish_row(img, palette, name_of[first]))[2][0]))
    save = app.look(lambda img: buttons(img, row(img, first), 40), "Save options")
    app.click(center(save[0]))
    wait_until(lambda: app.row(first)["apparitions"][0].get("tiers") == ["Fixture Premium"], "options saved")
    # Keyboard: F6 focuses the first pending cell, Tab the next, Space toggles its selection.
    app.key("F6")
    app.look(lambda img: img.stems(ACCENT, row(img, first)), "keyboard focus on the first row")
    app.key("Tab")
    app.look(lambda img: img.stems(ACCENT, row(img, second)), "keyboard focus on the second row")
    app.key("space")
    wait_until(lambda: not da(app.row(second)), "keyboard deselection persisted")
    app.look(lambda img: not selected(img, second), "the keyboard deselection shown")
    app.key("space")
    wait_until(lambda: da(app.row(second)), "keyboard reselection persisted")
    app.look(lambda img: selected(img, second), "the keyboard reselection shown")
    app.key("Escape")
    app.look(lambda img: not img.stems(ACCENT, row(img, second)), "keyboard mode to end")
    # Selected-set drift: another tool deselects the second row. The run on screen no longer
    # matches disk, so the app refuses it whole and reloads to show the drift; the next run
    # sends only what is still selected.
    app.edit_rows(lambda saved: deselect(saved, second))
    press_publish(app)
    app.look(lambda img: not selected(img, second), "the refused run to show the drifted selection")
    check(not app.lines("worker-calls.jsonl"), "a drifted run reached the worker")
    publish = press_publish(app)
    wait_until(lambda: len(app.lines("worker-calls.jsonl")) >= 1 and not publishing(app), "first run")
    stays(lambda: len(app.lines("worker-calls.jsonl")) == 1, "worker calls after a drifted selection", 2)
    check([c["uuid"] for c in app.lines("worker-calls.jsonl")] == [first], "drifted cell was sent")
    check(da(app.row(first))["state"] == "unpublished", "a safe failure must stay retryable")
    # Retry with both selected: the failure is retried, the second is sent once (unknown).
    press_footer(app, 0)
    # Cells stay locked until the finished run's receipts reload, as Publish selected shows.
    app.look(lambda img: publish_controls(img)[:1], "receipt state reloaded")
    img, region, _ = previews(app, palette, "Publish")
    app.click(center(row_cells(img, publish_row(img, palette, name_of[second]))[0][0]))
    app.look(lambda img: selected(img, second), "the second row reselected")
    wait_until(lambda: da(app.row(second)), "second row reselected")
    press_publish(app)
    wait_until(lambda: len(app.lines("worker-calls.jsonl")) == 3 and not publishing(app), "retry run")
    check(da(app.row(first))["state"] == "published_or_scheduled", "retried failure not recorded")
    check(da(app.row(second))["state"] == "unpublished", "unknown outcome must not look published")
    # Unknown blocks every resend until reconciled, across a restart.
    app.look(lambda img: not publish_controls(img), "Publish disabled by the unknown outcome")
    app.click(center(publish))
    stays(lambda: len(app.lines("worker-calls.jsonl")) == 3, "worker calls while unknown", 3)
    close_and_wait(app)
    app = ctx.launch("publish")
    navigate(app, "Publish")
    jump(app, 1.0)
    _, (verified, not_submitted) = app.look(lambda img: notice(img, list_view(img)[0]), "unresolved attempt")
    app.click(center(not_submitted))
    app.look(lambda img: not notice(img, list_view(img)[0]), "the attempt to be reconciled")
    press_publish(app)
    wait_until(lambda: len(app.lines("worker-calls.jsonl")) == 4 and not publishing(app), "post-reconcile run")
    check(da(app.row(second))["state"] == "published_or_scheduled", "reconciled retry not recorded")
    close_and_wait(app)


def da(row):
    return next((a for a in row.get("apparitions", []) if a["platformName"] == "deviantart"), None)


def deselect(rows, target):
    row = next(r for r in rows if r["uuid"] == target)
    row["apparitions"] = [a for a in row["apparitions"] if a["platformName"] != "deviantart"]


def publishing(app):
    """A run is active while Cancel after current shows right of (disabled) Publish selected."""
    return any(box[0] > 100 for box in publish_controls(app.shot()))


def press_publish(app):
    """Click Publish selected once it is enabled; return where it is."""
    button = app.look(lambda img: publish_controls(img)[:1], "Publish selected")[0]
    app.click(center(button))
    return button


def close_choices(app):
    """The Wait / Cancel buttons a close request offers above the footer while publishing."""
    return app.look(lambda img: buttons(img, (0, content(img)[3] - 60, img.w, content(img)[3]), 60),
                    "Wait / Cancel close choices")


@story
def gated_publish_survives_every_close_route(ctx):
    settings = {"worker_gate": True, "worker_outcomes": ["scheduled", "scheduled"]}
    app = ctx.launch("publish", settings=settings)
    navigate(app, "Publish")
    press_publish(app)
    started = app.fixture / "worker-1-started"
    wait_until(started.exists, "the gated worker to start")
    worker = int((app.fixture / "worker-1-pgid").read_text())
    app.close_window()
    wait_buttons = close_choices(app)
    check(len(wait_buttons) == 2, f"close choices {wait_buttons}")
    for index in (3, 0):  # Discard drafts and close, then Refresh
        press_footer(app, index)
        stays(app.alive, "app while publishing", 2)
    app.click(center(wait_buttons[0]))  # Wait for publishing
    stays(app.alive, "app while waiting", 2)
    check(not (app.fixture / "worker-cleaned").exists(), "worker cleaned before its outcome")
    for count in (1, 2):  # both selected cells run to their durable outcome
        (app.fixture / f"worker-{count}-release").touch()
    code = app.wait_exit(60)
    check(code == 0, f"exit {code}")
    check((app.fixture / "worker-cleaned").exists(), "worker cleanup not recorded before exit")
    check(not Path(f"/proc/{worker}").exists(), "worker outlived the app")
    sent = [c["uuid"] for c in app.lines("worker-calls.jsonl")]
    check(len(sent) >= 1 and da(app.row(sent[0]))["state"] == "published_or_scheduled",
          "durable outcome missing after close")
    # Cancel after current: the gated delivery finishes, later cells are not sent.
    app = ctx.launch("publish", fixture="cancel", settings=settings)
    navigate(app, "Publish")
    press_publish(app)
    wait_until((app.fixture / "worker-1-started").exists, "the gated worker to start")
    app.close_window()
    wait_buttons = close_choices(app)
    app.click(center(wait_buttons[1]))
    stays(app.alive, "app while cancelling", 1)
    (app.fixture / "worker-1-release").touch()
    check(app.wait_exit(60) == 0, "cancelled close did not exit")
    check(len(app.lines("worker-calls.jsonl")) == 1, "cancel after current still sent later cells")


def month_heading(img):
    """The top of the current month's (green) heading, or None while it is out of view."""
    region, _ = list_view(img)
    heading = [stem for stem in img.stems(TODAY, region) if stem[3] <= 30]
    return min(stem[1] for stem in heading) if heading else None


def on_today(img):
    """The list is scrolled down and shows today's (green) outlined cell. Its week is at the
    top unless the list ends first: late in the month the scroll clamps at the bottom."""
    region, thumb = list_view(img)
    return thumb and thumb[1] > region[1] + 4 and img.boxes(TODAY, region, 60, 60)


def calendar_days(img):
    """The current month's day cells in view, in day order, as boxes from a day's label strip
    (its #222 fill around the day number, above any event) down to the next week's labels.
    Days grow with their events and every border anti-aliases, so labels, not borders, locate
    them. Empty while the current month's heading is out of view."""
    region, _ = list_view(img)
    heading = month_heading(img)
    if heading is None:
        return []
    weeks = []
    strips = [b for b in img.boxes(DAY, (region[0], heading, region[2], region[3]), 100, 8) if labelled(img, b)]
    for strip in sorted(strips, key=lambda b: b[1]):
        if weeks and strip[1] - weeks[-1][0][1] <= 4:
            weeks[-1].append(strip)
        elif not weeks or strip[1] - weeks[-1][0][1] >= 76:  # a week is at least 78 px tall
            weeks.append([strip])
    days = []
    for index, week in enumerate(weeks):
        bottom = weeks[index + 1][0][1] - 4 if index + 1 < len(weeks) else region[3]
        days += [(x, y, w, bottom - y) for x, y, w, _ in sorted(week)]
    return days


def labelled(img, box):
    """A label strip shows its day number (dim grey #777 text) at its top-left; the space under
    an event, or beside an opened event's (white) text and (coloured) artwork, does not."""
    x, y, _, _ = box
    corner = [img.px(i, j) for j in range(y, y + 12) for i in range(x, x + 20)]
    return any(p[0] > 0x50 for p in corner) and all(p[0] == p[1] == p[2] <= 0x80 for p in corner)


def chips(img, cell):
    """A day's event chips (#2a2a2a; #333 when hovered), top to bottom."""
    found = [box for color in (CARD, CHIP_HOVER) for box in img.boxes(color, area(cell), 30, 10) if box[3] < 80]
    return sorted(found, key=lambda box: box[1])


def day_events(app, day, now):
    """Titles of the events the calendar shows on `day` of the current month, in its order: one
    per apparition at its row's schedule, in a stable timestamp sort."""
    found = []
    for row in app.rows():
        local = datetime.datetime.fromtimestamp(schedule_of(row), KIRITIMATI)
        if (local.year, local.month, local.day) == (now.year, now.month, day):
            title = "AI metadata pending" if "TODO_GENERATE" in row["title"] + row["description"] else row["title"]
            found += [(schedule_of(row), title)] * len(row.get("apparitions", []))
    return [title for _, title in sorted(found, key=lambda event: event[0])]


def whole_month(app, count):
    return app.look(lambda img: len(days := calendar_days(img)) == count and days, "the whole current month")


def artwork(img, rgb):
    """An opened event's 120 px artwork, letterboxed to its aspect ratio."""
    return img.boxes(tuple(rgb), content(img), 100, 50)


def expand(app, count, day, index, rgb):
    """Open the `index`th event of `day`; once its artwork shows (opening grows the day), scroll
    the current month's heading to the top, so the whole opened day is in view, and return the
    capture and the month's cells."""
    jump(app, 1.0)
    app.click(center(chips(app.last, whole_month(app, count)[day - 1])[index]))
    img = app.look(lambda img: artwork(img, rgb) and img, "the opened event's artwork")
    heading = month_heading(img)
    check(heading is not None, "opening an event scrolled the current month away")
    scroll_by(app, heading - list_view(img)[0][1] - 4)
    days = app.look(lambda img: len(days := calendar_days(img)) >= day and days, f"day {day} in view")
    return app.last, days


@story
def calendar_zone_prompts_overflow_url_and_jumps(ctx):
    """Configured-zone day buckets, exact/nearest/unknown prompts, a long prompt kept inside its
    growing day, the actual URL opened, and a phase jump that shows its target."""
    app = ctx.launch("calendar", size=(1100, 1500))  # a day with its long prompt open fits
    manifest = {e["title"]: e for e in json.loads((app.fixture / "calendar-fixture-manifest.json").read_text())}
    now = datetime.datetime.now(KIRITIMATI)
    count = (datetime.date(now.year + now.month // 12, now.month % 12 + 1, 1) - datetime.timedelta(days=1)).day
    navigate(app, "Calendar")
    # First opening: scrolled past the history to today's week.
    app.look(on_today, "Calendar to open on today's week")
    jump(app, 1.0)
    # Zone boundary: local day 1 holds the 00:xx events that are the previous month in UTC, and
    # shows every one of them (a chip per apparition), however many.
    first = day_events(app, 1, now)
    shown = len(chips(app.last, whole_month(app, count)[0]))
    check(shown == len(first) >= 3, f"day 1 shows {shown} of its {len(first)} events")
    check(all(datetime.datetime.fromtimestamp(manifest[t]["timestamp"], datetime.timezone.utc).month != now.month
              for t in first), "fixture boundary events are not across the UTC month")
    kinds = [manifest[t]["prompt"] for t in first]
    # Exact prompt: the long panel stays inside its (grown) day; the actual URL opens.
    exact = manifest[first[kinds.index("exact")]]
    img, days = expand(app, count, 1, kinds.index("exact"), exact["rgb"])
    panel = img.boxes(PROMPT_EXACT, content(img), 40, 40)[0]
    check(panel[3] > 150, f"12-line exact prompt panel only {panel[3]} px tall")
    check(within(panel, area(days[0])), f"exact prompt panel {panel} overflows day 1 {days[0]}")
    url, _ = buttons(img, area(days[0]), 40)[:2]
    status = header_status(img)
    app.click(center(url))
    wait_until(lambda: (app.fixture / "opened-urls.log").exists(), "the URL opener")
    check((app.fixture / "opened-urls.log").read_text() == "https://example.test/calendar-actual\n",
          "wrong URL opened")
    img = app.look(lambda img: header_status(img) != status and img, "the opened URL's status line")
    app.click(center(chips(img, calendar_days(img)[0])[kinds.index("exact")]))
    app.look(lambda img: not img.boxes(PROMPT_EXACT, content(img), 40, 40), "the exact prompt to close")
    # Nearest (amber, every lineage) and unknown (muted) prompt states, each inside day 1.
    for kind, probe in (("nearest", lambda img, day: img.boxes(NOTICE, area(day), 20, 20)),
                        ("unknown", lambda img, day: img.stems(MUTED, area(day)))):
        img, days = expand(app, count, 1, kinds.index(kind), manifest[first[kinds.index(kind)]]["rgb"])
        check(probe(img, days[0]), f"no {kind} prompt state inside day 1")
        app.click(center(chips(img, days[0])[kinds.index(kind)]))
        app.look(lambda img: not artwork(img, manifest[first[kinds.index(kind)]]["rgb"]), f"the {kind} event to close")
    # Phase jump: open the DESCRIBED target; Publish shows it first, on screen and marked.
    target = manifest["Calendar jump target DESCRIBED"]
    day = datetime.datetime.fromtimestamp(target["timestamp"], KIRITIMATI).day
    img, days = expand(app, count, day, day_events(app, day, now).index(target["title"]), target["rgb"])
    app.click(center(buttons(img, area(days[day - 1]), 40)[-1]))  # Open publication
    app.look(lambda img: tabs(img)[3][1], "Publish to open from Calendar")
    color = tuple(target["rgb"])
    app.look(lambda img: any(img.px(x, y) == color for y in range(list_view(img)[0][1], list_view(img)[0][1] + 140, 3)
                             for x in range(20, 200, 3)) and img.stems(WARNING, list_view(img)[0]),
             "the jump target first and marked in Publish")
    close_and_wait(app)


@story
def prompt_search_exact_and_missing_archive(ctx):
    app = ctx.launch("prompt-search", size=(1100, 1150))  # all three cards on screen
    palette = Palette.files(app.fixture.glob("pick/*.png"))
    search(app)
    img, region, blocks = previews(app, palette, "Pick")
    check(len(blocks) == 3, f"prompt-search listed {len(blocks)}")
    app.look(lambda img: img.boxes(PROMPT_EXACT, content(img), 40, 20), "an exact prompt label")
    # Pick prompt text starts collapsed under its heading; Show opens it, Hide closes it again.
    text_lines = lambda img: img.boxes(PROMPT_TEXT, content(img), 40, 20)
    check(not text_lines(app.last), "Pick prompt text open by default")
    for state in ("open", "close"):
        toggle = app.look(lambda img: buttons(img, area(img.boxes(PROMPT_EXACT, content(img), 40, 20)[0]))[:1],
                          "the prompt Show/Hide button")[0]
        app.click(center(toggle))
        app.look(lambda img: bool(text_lines(img)) == (state == "open"), f"the Pick prompt text to {state}")
    query = app.look(lambda img: inputs(img, (0, 0, 400, img.h))[:1], "prompt query input")[0]
    type_into(app, query, "MOONLIT EXACT PROMPT — <literal> 雪")
    search(app)
    exact = [n for n in palette.name.values() if n.startswith("prompt.txt_") and "ffff" not in n]
    app.look(lambda img: [n for n, _ in palette.blocks(img, content(img))] == exact, "only the exact match")
    close_and_wait(app)
    app = ctx.launch("prompt-search-missing-archive", fixture="missing")
    palette = Palette.files(app.fixture.glob("pick/*.png"))
    search(app)
    img, _, blocks = previews(app, palette, "Pick")
    # A missing archive disables prompt lookup (docs/features/config.feature): no prompt inputs.
    check([n for n, _ in blocks] == ["plain.png"], f"missing archive listed {blocks}")
    check(not inputs(img, (0, 0, 400, img.h)), "prompt search offered without an archive")
    close_and_wait(app)


@story
def reopen_keeps_saved_edits(ctx):
    app = ctx.launch("mixed")
    palette = Palette.files(app.fixture.glob("art/*.png"))
    navigate(app, "Describe")
    target = listed_names(app, "Describe")[0]
    uuid_ = next(r["uuid"] for r in app.rows() if r["files"][0]["basename"] == target)
    img, region, _ = previews(app, palette, "Describe")
    title = inputs(img, area(card_of(img, region, target, palette)))[0]
    type_into(app, title, "Reopened ✓")
    wait_until(lambda: app.row(uuid_)["title"] == "Reopened ✓", "saved edit")
    close_and_wait(app)
    app = ctx.launch("reopen")
    check(app.row(uuid_)["title"] == "Reopened ✓", "reopen reinitialised the fixture")
    close_and_wait(app)


SIZES = ((700, 900), (1100, 760), (1500, 1000))


@story
def thumbnail_opens_the_viewer_and_closing_keeps_the_view(ctx):
    """A loaded thumbnail shows its original file over the whole window; a click (Pick) or
    Escape (Schedule) closes it on the same tab at the same scroll."""
    app = ctx.launch("gallery")
    palette = Palette.manifest(app.fixture / "gallery-fixture-manifest.json")
    search(app)
    for tab, close in (("Pick", "click"), ("Schedule", "Escape")):
        navigate(app, tab)
        jump(app, 0.45)
        img, region, blocks = previews(app, palette, tab)
        if not blocks:  # a Pick stop can fall between two rows of thumbnails
            scroll_by(app, -40)
            img, region, blocks = previews(app, palette, f"{tab} - 40 px")
        app.park()
        before = app.shot()
        view = (tabs(before), list_view(before), palette.blocks(before, region))
        name, box = view[2][0]
        app.click(center(box))

        def covered(img):
            bars = [img.px(x, y) for x in (2, img.w // 2, img.w - 3) for y in (2, img.h - 3)]
            shown = [b for n, b in palette.blocks(img, (0, 0, img.w, img.h)) if n == name]
            return (all(px == (0, 0, 0) for px in bars) and not img.boxes(HEADER, None, 20, 20)
                    and not img.boxes(CARD, None, 20, 20) and shown and shown[0])

        # The 480x300 original, not its thumbnail, centred and never upscaled.
        x, y, w, h = app.look(covered, f"{tab} viewer to cover the window with {name}")
        check((w, h) == (480, 300) and abs(x + w // 2 - app.last.w // 2) <= 2,
              f"{tab} viewer shows {name} at {(x, y, w, h)}")
        if close == "click":
            app.click((10, 10))
        else:
            app.key("Escape")
        app.look(lambda img: not covered(img), f"{tab} viewer to close")
        app.park()
        app.look(lambda img: (tabs(img), list_view(img), palette.blocks(img, region)) == view,
                 f"{tab} to keep its tab and scroll after the viewer")
    close_and_wait(app)


@story
def every_visible_card_loads_its_own_image(ctx):
    """At narrow/default/wide sizes, jumps and resizes leave no visible card without its
    own image (pixel identity against the gallery manifest), and no Pick card's text crosses
    into its neighbour."""
    app = ctx.launch("gallery", size=SIZES[0])
    palette = Palette.manifest(app.fixture / "gallery-fixture-manifest.json")
    search(app, limit_index=2)
    for size in SIZES:
        app.resize(*size)
        for tab in ("Pick", "Schedule", "Describe", "Publish"):
            navigate(app, tab)
            for fraction in (1.0, 0.45, 0.0, 0.8):
                what = f"{tab} at {size} {fraction:.0%}"
                jump(app, fraction)
                img, region, blocks = previews(app, palette, what)
                if tab in TAB_STATES:
                    assert_order(blocks, listed_names(app, tab), tab)
                    continue
                if not blocks:
                    # Pick cards are about as tall as the view, so a stop can fall between two
                    # rows of thumbnails; 40 px up, the upper row's always shows.
                    scroll_by(app, -40)
                    img, region, blocks = previews(app, palette, f"{what} - 40 px")
                check(blocks, f"{what} shows no artwork")
                # Long digest names wrap inside their card instead of running across.
                spilled = [box for box in gutters(img, region) if not clear(img, box, PAGE)]
                check(not spilled, f"Pick card text crosses the gutters {spilled} at {size}")
    close_and_wait(app)


# Runner -------------------------------------------------------------------------------------
def build(repo):
    tool("cargo", "build", "--locked", "--release", "--example", "ui_acceptance", "--features",
         "desktop", cwd=repo, timeout=3600)
    target = Path(os.environ.get("CARGO_TARGET_DIR", repo / "target"))
    return target / "release" / "examples" / "ui_acceptance"


def alarm(signum, frame):
    raise Failure(f"story exceeded its {STORY_BUDGET}s budget")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--keep", action="store_true", help="keep artifacts after success")
    parser.add_argument("stories", nargs="*", help="story names (default: all)")
    args = parser.parse_args()
    selected = [s for s in STORIES if not args.stories or s.__name__ in args.stories]
    check(selected, f"unknown stories {args.stories}; known: {[s.__name__ for s in STORIES]}")
    repo = Path(__file__).resolve().parent.parent
    binary = (args.binary or build(repo)).resolve()
    root = Path(tempfile.mkdtemp(prefix="publicator-native-replay.", dir="/tmp"))
    print(f"artifacts: {root}", flush=True)
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    signal.signal(signal.SIGALRM, alarm)
    failures = []
    display = Display(root)
    try:
        display.start()
        for function in selected:
            ctx = Context(display, binary, root / function.__name__)
            started = time.monotonic()
            error = None
            signal.alarm(STORY_BUDGET)
            try:
                function(ctx)
            except Exception as caught:
                error = caught
                ctx.evidence()
            finally:
                signal.alarm(0)
                try:
                    ctx.close()
                except Failure as cleanup:
                    error = error or cleanup
            status = "PASS" if error is None else "FAIL"
            print(f"{status} {function.__name__} ({time.monotonic() - started:.0f}s)", flush=True)
            if error is not None:
                failures.append(function.__name__)
                traceback.print_exception(error, limit=-3)
    finally:
        display.close()
        remaining = leaked(root)
        if remaining:
            failures.append("cleanup")
            print(f"FAIL cleanup: {remaining}")
    print(f"{len(selected) - len([f for f in failures if f != 'cleanup'])} passed, "
          f"{len(failures)} failed")
    if failures or args.keep:
        print(f"artifacts kept: {root}")
    else:
        shutil.rmtree(root, ignore_errors=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
