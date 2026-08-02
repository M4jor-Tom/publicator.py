# Publication Workflow Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace multi-step publish workflow with a single `publish_next.py` command that shows unpublished images in a browser gallery with checkboxes and auto-schedules selected images to Tuesday 8pm slots.

**Architecture:** A single Python script (`publish_next.py`) that does candidate discovery → thumbnail generation → local HTTP server with HTML gallery → receive selection → schedule slots → write to `publications.json`. No new dependencies (stdlib + jsonschema + ImageMagick which is already in flake).

**Tech Stack:** Python 3 stdlib (`http.server`, `json`, `hashlib`, `uuid`, `os`, `subprocess`, `random`, `tempfile`, `argparse`, `datetime`), `jsonschema`, ImageMagick `convert`, `firefox`

---
### Task 1: Remove history tracking from open_unpublished.py

**Files:**
- Modify: `open_unpublished.py`
- Delete: `.opened_history.json`

- [ ] **Remove HISTORY_FILE, load_history, save_history and history usage**

```python
# Before (open_unpublished.py):
HISTORY_FILE: str = ".opened_history.json"
...
def load_history() -> set[str]:
    ...
def save_history(history_set: str) -> None:
    ...
# In main():
    history_hashes: list[str] = load_history()
    ...
    if file_hash in history_hashes and not open_published:
        continue
    ...
    updated_history.add(file_hash)
    ...
    save_history(updated_history)
```

Replace with:

```python
# Remove the three blocks above entirely from the file.
# The function signature and variable declarations referencing history should be deleted.
# After removal, main() should look like:

def main(directory: str, json_path: str, limit: int, open_published: bool):
    publicated_hashes: list[str] = load_publicated_hashes(json_path)

    images: list[any] = collect_images(directory)
    if not open_published:
        random.shuffle(images)

    opened: int = 0

    for full_path in images:
        if opened >= limit:
            break

        try:
            file_hash: str = compute_sha512(full_path)
        except Exception as e:
            print(f"Error hashing {full_path}: {e}")
            continue

        if (file_hash in publicated_hashes) != open_published:
            continue

        print(f"Opening: {full_path}")
        open_file(full_path)
        opened += 1

    print(f"\nOpened {opened} image(s).")
```

No more `updated_history`, `history_hashes`, and the `save_history` call at the end.

- [ ] **Delete `.opened_history.json`**

```bash
rm .opened_history.json
git rm .opened_history.json
```

- [ ] **Commit**

```bash
git add open_unpublished.py .opened_history.json
git commit -m "refactor: remove opened history tracking from open_unpublished.py"
```

---

### Task 2: Write candidate discovery + thumbnail generation

**Files:**
- Create: `publish_next.py`

- [ ] **Add candidate discovery function**

```python
#!/usr/bin/env python3

import json
import hashlib
import os
import subprocess
import sys
import random
import tempfile
import uuid
import time
import argparse
from datetime import datetime, timedelta
from http.server import HTTPServer, ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse

IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp", ".tiff", ".mp4"}

def compute_sha512(filepath: str, chunk_size: int = 1024 * 1024) -> str:
    h = hashlib.sha512()
    with open(filepath, "rb") as f:
        while chunk := f.read(chunk_size):
            h.update(chunk)
    return h.hexdigest()


def load_publicated_hashes(json_path: str) -> set[str]:
    publicated = set()
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)
        for entry in data:
            files = entry.get("files", [])
            preview_files = entry.get("previewFilesIfNotFree", [])
            for file_obj in files + preview_files:
                sha = file_obj.get("sha512sum") or file_obj.get("fileSha512sum")
                if sha:
                    publicated.add(sha.lower())
    return publicated


def collect_images(directory: str) -> list[str]:
    images = []
    for root, _, files in os.walk(directory):
        for filename in files:
            ext = os.path.splitext(filename)[1].lower()
            if ext in IMAGE_EXTENSIONS:
                images.append(os.path.join(root, filename))
    return images


def find_candidates(directory: str, json_path: str, limit: int) -> list[str]:
    publicated = load_publicated_hashes(json_path)
    images = collect_images(directory)
    random.shuffle(images)

    candidates = []
    for path in images:
        if len(candidates) >= limit:
            break
        try:
            file_hash = compute_sha512(path)
        except Exception:
            continue
        if file_hash not in publicated:
            candidates.append(path)
    return candidates
```

- [ ] **Add thumbnail generation function**

```python
def generate_thumbnails(image_paths: list[str], thumb_dir: str) -> dict[str, str]:
    path_to_thumb = {}
    for i, path in enumerate(image_paths):
        ext = os.path.splitext(path)[1].lower()
        thumb_name = f"thumb_{i:04d}{ext}"
        thumb_path = os.path.join(thumb_dir, thumb_name)
        try:
            subprocess.run(
                ["convert", path, "-resize", "300x300>", thumb_path],
                capture_output=True, check=True
            )
        except subprocess.CalledProcessError:
            # fallback: copy original
            subprocess.run(["cp", path, thumb_path], check=True)
        path_to_thumb[path] = thumb_path
    return path_to_thumb
```

- [ ] **Commit**

```bash
git add publish_next.py
git commit -m "feat: add candidate discovery and thumbnail generation"
```

---

### Task 3: Gallery server (HTTP + HTML)

**Files:**
- Modify: `publish_next.py`

- [ ] **Add GalleryRequestHandler class and server function**

```python
class GalleryHandler(BaseHTTPRequestHandler):
    selected_results = None
    thumb_dir = ""
    thumb_map = {}

    def _build_page(self):
        cards = []
        for orig_path, thumb_path in self.thumb_map.items():
            rel = os.path.relpath(thumb_path, self.thumb_dir)
            filename = os.path.basename(orig_path)
            cards.append(
                '<div class="card">'
                f'<input type="checkbox" value="{orig_path}" id="cb_{abs(hash(orig_path))}">'
                f'<img src="/thumbs/{rel}" alt="{filename}">'
                f'<label for="cb_{abs(hash(orig_path))}">{filename}</label>'
                '</div>'
            )
        cards_html = "\n".join(cards)
        return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Select images to publish</title>
<style>
  body {{ font-family: sans-serif; margin: 20px; background: #1a1a1a; color: #eee; }}
  .grid {{ display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 16px; }}
  .card {{ background: #2a2a2a; border-radius: 8px; padding: 8px; text-align: center; }}
  .card img {{ max-width: 100%; max-height: 200px; border-radius: 4px; }}
  .card label {{ display: block; margin-top: 6px; cursor: pointer; font-size: 14px; word-break: break-all; }}
  .card input {{ transform: scale(1.5); margin-bottom: 6px; }}
  #publish-btn {{ padding: 12px 32px; font-size: 18px; background: #4caf50; color: white; border: none; border-radius: 6px; cursor: pointer; margin-bottom: 20px; }}
  #publish-btn:hover {{ background: #45a049; }}
  #count {{ margin-left: 12px; font-size: 16px; }}
</style>
</head>
<body>
<h1>Select images to publish</h1>
<button id="publish-btn" onclick="publishSelected()">Publish Selected</button><span id="count">0 selected</span>
<div class="grid" id="gallery">
{cards_html}
</div>
<script>
const count = document.getElementById('count');
document.querySelectorAll('input[type=checkbox]').forEach(c => c.addEventListener('change', function() {{
  const n = document.querySelectorAll('input[type=checkbox]:checked').length;
  count.textContent = n + ' selected';
}}));
function publishSelected() {{
  const checked = Array.from(document.querySelectorAll('input[type=checkbox]:checked')).map(c => c.value);
  if (checked.length === 0) {{ alert('Select at least one image.'); return; }}
  fetch('/publish', {{
    method: 'POST',
    headers: {{ 'Content-Type': 'application/json' }},
    body: JSON.stringify({{ files: checked }})
  }}).then(r => r.json()).then(d => {{
    if (d.status === 'ok') {{
      document.body.innerHTML = '<h1>Published!</h1><pre>' + d.summary + '</pre><p>You can close this tab.</p>';
    }}
  }});
}}
</script>
</body>
</html>"""

    def do_GET(self):
        if self.path == "/" or self.path == "/index.html":
            html = self._build_page()
            self.send_response(200)
            self.send_header("Content-type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(html.encode("utf-8"))
        elif self.path.startswith("/thumbs/"):
            rel_path = self.path[len("/thumbs/"):]
            full_path = os.path.normpath(os.path.join(self.thumb_dir, rel_path))
            if full_path.startswith(os.path.normpath(self.thumb_dir)) and os.path.isfile(full_path):
                self.send_response(200)
                self.send_header("Content-type", self._guess_mime(full_path))
                self.end_headers()
                with open(full_path, "rb") as f:
                    self.wfile.write(f.read())
            else:
                self.send_response(404)
                self.end_headers()
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.path == "/publish":
            length = int(self.headers.get("Content-length", 0))
            body = self.rfile.read(length)
            data = json.loads(body)
            self.__class__.selected_results = data.get("files", [])
            self.send_response(200)
            self.send_header("Content-type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"status": "ok", "summary": "Processing..."}).encode("utf-8"))

    def _guess_mime(self, path):
        ext = os.path.splitext(path)[1].lower()
        return {
            ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
            ".png": "image/png", ".webp": "image/webp",
            ".gif": "image/gif", ".mp4": "video/mp4",
        }.get(ext, "application/octet-stream")

    def log_message(self, format, *args):
        pass  # quiet


def serve_gallery(thumb_dir: str, thumb_map: dict[str, str]) -> list[str]:
    GalleryHandler.thumb_dir = thumb_dir
    GalleryHandler.thumb_map = thumb_map
    GalleryHandler.selected_results = None

    server = ThreadingHTTPServer(("127.0.0.1", 8765), GalleryHandler)
    print(f"Gallery: http://127.0.0.1:8765")
    subprocess.Popen(
        ["firefox", "--private-window", "--no-remote", "http://127.0.0.1:8765"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )

    try:
        while GalleryHandler.selected_results is None:
            server.handle_request()
    except KeyboardInterrupt:
        pass

    server.server_close()
    return GalleryHandler.selected_results
```

- [ ] **Commit**

```bash
git add publish_next.py
git commit -m "feat: add gallery HTTP server with HTML selection page"
```

---

### Task 4: Scheduling + publication writing functions

**Files:**
- Modify: `publish_next.py`

- [ ] **Add scheduling and write functions**

```python
def get_next_tuesday_8pm_after(timestamp: int) -> datetime:
    dt = datetime.utcfromtimestamp(timestamp)
    days_ahead = (1 - dt.weekday()) % 7
    if days_ahead == 0:
        days_ahead = 7
    next_tue = dt + timedelta(days=days_ahead)
    return next_tue.replace(hour=20, minute=0, second=0, microsecond=0)


def compute_slots(selected_count: int, json_path: str) -> list[int]:
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    max_ts = 0
    for entry in data:
        for apparition in entry.get("apparitions", []):
            ts = apparition.get("apparitionTimestampIfDifferentThanSubmission")
            if ts and ts > max_ts:
                max_ts = ts

    if max_ts == 0:
        max_ts = int(time.time())

    current = get_next_tuesday_8pm_after(max_ts)
    slots = []
    for i in range((selected_count + 1) // 2):
        slots.append(int(current.timestamp()))
        current += timedelta(days=7)
    return slots


def publish_images(selected_paths: list[str], slots: list[int], json_path: str, schema_path: str):
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    new_entries = []
    for i, path in enumerate(selected_paths):
        sha512 = compute_sha512(path)
        slot_idx = i // 2
        entry = {
            "uuid": str(uuid.uuid4()),
            "submissionTimestamp": int(time.time()),
            "description": "",
            "files": [
                {
                    "basename": os.path.basename(path),
                    "fileSha512sum": sha512
                }
            ],
            "apparitions": [
                {
                    "platformName": "deviantart",
                    "urlElsePublicationName": "",
                    "apparitionTimestampIfDifferentThanSubmission": slots[slot_idx]
                }
            ]
        }
        new_entries.append(entry)

    data.extend(new_entries)

    from jsonschema import validate
    with open(schema_path, "r", encoding="utf-8") as s:
        schema = json.load(s)
    validate(instance=data, schema=schema)

    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=4)

    return new_entries
```

- [ ] **Commit**

```bash
git add publish_next.py
git commit -m "feat: add scheduling and publication writing logic"
```

---

### Task 5: Main function and final wiring

**Files:**
- Modify: `publish_next.py`

- [ ] **Add main() function**

```python
def main():
    parser = argparse.ArgumentParser(description="Find unpublished images, select in gallery, and schedule publication.")
    parser.add_argument("-n", type=int, default=10, help="Number of candidate images to show (default: 10)")
    parser.add_argument("--picked-dir", default="picked", help="Directory to scan for images (default: picked)")
    parser.add_argument("--json", default="publications.json", help="Path to publications.json")
    parser.add_argument("--schema", default="publicationsSchema.json", help="Path to publicationsSchema.json")
    args = parser.parse_args()

    print("Finding unpublished images...")
    candidates = find_candidates(args.picked_dir, args.json, args.n)
    if not candidates:
        print("No unpublished images found.")
        sys.exit(0)

    print(f"Found {len(candidates)} candidates. Generating thumbnails...")
    with tempfile.TemporaryDirectory(prefix="publish-next-") as thumb_dir:
        thumb_map = generate_thumbnails(candidates, thumb_dir)

        print(f"Starting gallery server...")
        selected = serve_gallery(thumb_dir, thumb_map)

    if not selected:
        print("No images selected.")
        sys.exit(0)

    print(f"Selected {len(selected)} images. Computing schedule...")
    slots = compute_slots(len(selected), args.json)

    print(f"Publishing...")
    new_entries = publish_images(selected, slots, args.json, args.schema)

    print(f"\nDone! Published {len(new_entries)} image(s):\n")
    from collections import defaultdict
    by_slot = defaultdict(list)
    for entry in new_entries:
        ts = entry["apparitions"][0]["apparitionTimestampIfDifferentThanSubmission"]
        by_slot[ts].append(entry["files"][0]["basename"])

    for ts, names in by_slot.items():
        dt = datetime.utcfromtimestamp(ts)
        print(f"  Slot — {dt.strftime('%a %b %d %Y %H:%M')} UTC:")
        for name in names:
            print(f"    {name}")
        print()


if __name__ == "__main__":
    main()
```

- [ ] **Verify the script works end-to-end**

```bash
# Check that imports resolve and argparse works
python publish_next.py --help
```

Expected output shows help text with `-n`, `--picked-dir`, `--json`, `--schema` options.

- [ ] **Validate on a small sample**

```bash
python publish_next.py -n 3
```

Expected: Opens gallery in Firefox with 3 thumbnails. After selecting and submitting, creates entries in `publications.json` with correct schema.

- [ ] **Commit**

```bash
git add publish_next.py
git commit -m "feat: complete publish_next.py with main flow"
```

---

### Task 6: Validate publications.json schema compliance

**Files:**
- No changes

- [ ] **Run schema validation on publications.json**

```bash
python -c "
import json
from jsonschema import validate
with open('publicationsSchema.json') as s, open('publications.json') as o:
    validate(instance=json.load(o), schema=json.load(s))
print('Schema validation: OK')
"
```

Expected: Prints "Schema validation: OK"

- [ ] **If validation fails, fix in publish_next.py and verify**

```bash
# Fix any issues found in the publish_images function
# then re-run schema validation
```
