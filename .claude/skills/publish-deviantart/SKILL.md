---
name: publish-deviantart
description: Publish or schedule the next unpublished art piece to DeviantArt. Try the automated app first; fall back to driving Chromium manually if it exits non-zero.
---

Every command reads its **publication database** (`publications.json` + images +
the `.deviantart-session/` profile) from `--data-dir <dir>`, defaulting to the
current working directory. Run the commands from the Art data dir and omit the
flag, or run them from anywhere and pass `--data-dir <path-to-art-data-dir>`
(for nix apps, after `--`: `nix run ... -- --data-dir <dir>`).

## First: try the automated app

```sh
nix run .#da-publish -- --data-dir <path-to-art-data-dir>
```

Publishes ONE entry — the first `state=unpublished` in `publications.json` (add
`--uuid <id>` to target a specific one). If it exits 0, the publication is done
— stop here.

If it exits non-zero, it prints `AUTOMATION FAILED: <reason>` to stderr and
closes the browser cleanly. Fall through to the manual steps below to finish the
current publication, then update `deviantart.py` to cover whatever
tripped it (so the next run doesn't fall through).

## Values for the current publication

`<pub.path>` / `<pub.title>` / `<pub.schedule>` come from the first
`state=unpublished` entry in `publications.json` (stdlib-only, no app needed):

```sh
nix run <publicator.py>#echo-first -- --data-dir .
```

## Manual fallback: drive Chromium via the daemon

```sh
nix run .#pw-daemon
```

Chromium persistent context in `.deviantart-session/` (the same logged-in
session the publish-next app uses — no Firefox profile prep needed). First run is
headed; log into DeviantArt once and the session persists.

Daemon FIFO `/tmp/pw.cmd`, output `/tmp/pw.log`, screenshot `/tmp/pw.png`; scope
has `page`, `context`, `p`. Send a command chunk, e.g.:

```sh
printf '%s' 'page.goto("https://www.deviantart.com")' > /tmp/pw.cmd
```

### Steps

1. Connect to www.deviantart.com
2. Click on submit
3. Click on "upload your art" and pick file `<pub.path>`
4. Set as title `<pub.title>`
5. Set as description `<pub.description>`
6. Tick boxes "Mature" and "Created using AI tools"
7. Drop all the pre-filled tags in the "Tags" field
8. Copy the content of the tags file (`<data-dir>/<"tags" path>`, per publicator.toml [tags]) into the "Tags" field
9. If the piece has a price, tick "Sell as Premium Download" and set the price
10. Set the subscription tier `<pub.tier>`
11. Add to galleries `<pub.galleries>`
12. Schedule publication for the `<pub.schedule>`
