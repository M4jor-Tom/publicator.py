# Publication Workflow Optimization

## Overview

Replace the current multi-step publication workflow (`open_unpublished.py` → manual selection → `publish.py` → `appear.py`) with a single command that discovers unpublished images, presents them in a visual gallery with checkboxes, and auto-schedules selected images to Tuesday 8pm slots.

## Motivation

The current workflow requires:
1. Running `open_unpublished.py -u N` to open N random tabs in Firefox
2. Manually tracking which images to publish
3. Running `publish.py <path>` for each image
4. Running `appear.py` to schedule each one
5. Entering dates and platform names interactively

The new workflow eliminates steps 2-5 with a single gallery + batch scheduling step.

## Design

### Single Command

```bash
python publish_next.py -n 50
```

Finds 50 random unpublished images from `picked/`, shows them in a browser gallery, and batch publishes + schedules selected images.

### Phase 1 — Candidate Discovery

- Scans `picked/` directory recursively for image files (`.jpg`, `.jpeg`, `.png`, `.webp`, `.gif`, `.bmp`, `.tiff`, `.mp4`)
- Computes SHA512 hash for each file
- Filters out files whose hashes appear in `publications.json` (checks both `sha512sum` and `fileSha512sum` keys)
- Randomly shuffles remaining candidates
- Takes the first N (`-n` flag, default 10)
- Generates thumbnails to `/tmp/publish-next-thumbs/` using ImageMagick (`convert`)

### Phase 2 — Gallery Page

A minimal Python HTTP server on `http://localhost:8765` serves:
- An HTML page with all candidate thumbnails in a grid layout
- Each image has a checkbox next to it
- A "Publish Selected" button at the top
- The page uses inline CSS/JS (no external dependencies)
- On submit, POSTs the selected filenames to the server as JSON

The server opens Firefox to the page after starting.

### Phase 3 — Smart Scheduling

After receiving the selection, the server shuts down. Selected images are processed as follows:

- Read `publications.json` to find the latest `apparitionTimestampIfDifferentThanSubmission` across all entries
- If no apparition timestamps exist, use the current time as reference
- Calculate the *following* Tuesday at 8pm UTC from that timestamp. If the reference timestamp is already a Tuesday at 8pm, advance to the *next* Tuesday (7 days later).
- Group selected images into pairs (ordered by selection order)
- Each pair gets the next available Tuesday 8pm slot:
  - Slot 1: the Tuesday after the latest apparition
  - Slot 2: 7 days after slot 1
  - Slot 3: 7 days after slot 2
  - etc.
- If an odd number of images is selected, the last image gets its own slot
- Images in the same pair share the exact same timestamp

### Phase 4 — Write Publications

For each selected image, create a publication entry:

```json
{
  "uuid": "<new-uuid>",
  "submissionTimestamp": <current-time>,
  "description": "",
  "files": [
    {
      "basename": "<filename>",
      "fileSha512sum": "<sha512>"
    }
  ],
  "apparitions": [
    {
      "platformName": "deviantart",
      "urlElsePublicationName": "",
      "apparitionTimestampIfDifferentThanSubmission": <tuesday-8pm-timestamp>
    }
  ]
}
```

- Validate the file against `publicationsSchema.json` before writing
- Validate the full `publications.json` after writing

### Output

Print a summary:
```
Selected 3 images for publication:

  Slot 1 — Tue Jul  7 20:00:
    mommy_tentacles_....webp
    greek_mommy_....webp

  Slot 2 — Tue Jul 14 20:00:
    topless_mommy_....webp
```

## What stays the same

- `open_unpublished.py`, `publish.py`, `appear.py` — untouched
- `publications.json` schema — unchanged
- `publicationsSchema.json` — unchanged
- Manual posting on deviantart.com — still required
- Flake devShell (`python3.withPackages jsonschema`) — already has all Python dependencies needed

## Files to create

- `publish_next.py` — the new script

## Files to modify

- `.opened_history.json` — delete this file and its usage from `open_unpublished.py`
- `open_unpublished.py` — remove `HISTORY_FILE` constant and all `load_history`/`save_history` references
