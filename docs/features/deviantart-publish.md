# DeviantArt publishing (Playwright submit flow)

- **Does:** Drives Playwright's Firefox, on a fresh copy of the `#login` profile, through DeviantArt's submit form for each metadata-approved pending DeviantArt target and flips its DA apparition `unpublished` to `published_or_scheduled`. Blank or marker-bearing metadata is rejected before opening a browser.
- **Run:** `nix run .#da-publish -- --data-dir <dir> [--json <path>] [--uuid <id> | --all] [--headless] [--check-steps] [-v]` (first pending; `--uuid` one; `--all` all pending, one browser)
- **Run:** `nix run .#check-steps` (STEPS vs SKILL.md drift guard, no browser/data deps)
- **Run:** `nix run .#login` (run FROM the data dir; real Firefox, one-time human sign-in into `.deviantart-login/`)
- **Run:** `nix run .#pw-daemon -- [--data-dir <dir>]` (headed Chromium on `.deviantart-session/`, exec's Python chunks from `/tmp/pw.cmd`, log `/tmp/pw.log`, screenshot `/tmp/pw.png`)
- **Run:** `nix run .#echo-first -- --data-dir <dir>` (path/title/schedule of the first pending entry, for the skill's manual fallback)
- **Code:** `src/publicator/deviantart.py` (`configure`, `load_pending_entries`, `publish_batch`, `submit_entry`, `STEPS`, `check_steps`, `_prepare_session_from_login`, `_da_login_cookies`, `_session_authed`, `pick_schedule`, `PREMIUM_TOGGLE_LABEL`, `DESCRIPTION_SELECTOR`), `src/publicator/apps/da_publish.py` (`main`), `src/publicator/apps/pw_daemon.py` (`FIFO`, `LOG`, `SHOT`, `main`), `src/publicator/apps/echo_first.py` (`main`), `src/publicator/entries.py` (`deviantart_apparition`, `find_art_path`, `format_schedule`), `src/publicator/store.py` (`mark_state`), `src/publicator/platforms.py` (registry/dispatcher), `flake.nix` (`login`, `da-publish`, `pw-daemon`, `check-steps`, `echo-first`, `app`), `.claude/skills/publish-deviantart/SKILL.md` (`### Steps`)
- **Tests:** `tests/test_deviantart.py` (parse_schedule, STEPS/SKILL.md sync, optional-step no-ops, TAGS_FILE resolution, AI-marker publish guard, cookie extraction), `tests/test_da_publish.py` (CLI entry selection, flag exclusivity, --headless reaching the Firefox launch), `tests/conftest.py` (restores data-dir globals between tests)
- **Config:** `publicator.toml` top-level `tags` (str, default None; data-dir-relative, one tag per line, typed at step 8 for EVERY entry; missing raises RuntimeError "no 'tags' path configured"). `[deviantart]` `tiers`/`galleries` (str[], default []) are NOT read here (entries carry their own `tiers`/`galleries`; legacy `tier` is accepted); they are the allow-lists `config.validate_publications` enforces on web-UI appends.
- **Data:** `publications.json` (reads `files[0].basename`, publication `title`, `description`, publication `apparitionTimestampIfDifferentThanSubmission`, `priceIfNotFree`, `tiers` or legacy `tier`, `galleries`; rewritten by `store.mark_state` via `atomic_write_json`); `publicator.toml` (read by `configure()` via `load_config` unless a pre-loaded dict is passed); `.deviantart-login/` (written by `#login`; read-only here: copied, `cookies.sqlite` opened `?immutable=1`); `.deviantart-session/` (wiped and rebuilt from `.deviantart-login/` on EVERY `publish_batch`; also pw-daemon's Chromium profile; in `.gitignore`); `<tags>` file (one tag per line, typed at step 8); `**/*<stem>*` (art found by `DATA_DIR.rglob` on the stored basename's stem, non-.webp preferred); `tempfile.gettempdir()/<stem>.png` (imagemagick `convert` output when only a .webp exists); `/tmp/pw.cmd`, `/tmp/pw.log`, `/tmp/pw.png` (pw-daemon, hardcoded, not under the data dir)
- **Decisions:** none
- **Verify:** `nix develop -c pytest tests/test_deviantart.py tests/test_da_publish.py -q`
- **Verify:** `nix run .#check-steps` (prints `check-steps OK: <n> steps in sync with the skill`)

## How it works

The explicit registry in `src/publicator/platforms.py` binds an identifier/label
to its executable module and accompanying skill. Registration requires
configure(data_dir, config), load_pending_entries(json_path), and
publish_batch(entries, uuids, json_path, headless=False) returning
(published, failed, error). Successful targets must be marked immediately through
the validated store; loading pending entries must exclude successful deliveries.
Each publisher owns its ordered script/skill step guard. DeviantArt is initially
the only registration; historical pixiv data is not a selectable publisher.

The dispatcher and direct DA batch validate fresh stored approved records and
options before browser startup; client/caller metadata is never authoritative.
DA stops at its first submission failure, leaving failed/unattempted targets
pending. Retry skips successes. The web UI remains open after an attempt; direct
CLI flags --uuid/--all and --headless retain their meanings.

## Invariants

- **`STEPS` titles equal, in order and count, the numbered lines under `### Steps` in the skill's `SKILL.md`.** The skill is the manual fallback, so an unimplemented step must fail loudly; `check_steps` + `parse_skill_steps`, `tests/test_deviantart.py::test_steps_stay_in_sync_with_the_skill`, `check-steps` app.
- **A batch stops at the first failing entry; `mark_state` runs only after `submit_entry` returned.** A half-submitted flow must not be recorded as published, and the next run picks up the rest; `publish_batch` (try: submit_entry; mark_state / except: failed += 1; break).
- **Unresolved `TODO_GENERATE` metadata never reaches DeviantArt.** `publish_batch` rejects the marker before browser startup; UI `/publish` rejects it at the HTTP boundary too.
- **Playwright never opens `.deviantart-login/`; it runs on a `.deviantart-session/` copy minus `lock`/`.parentlock`/`parent.lock`/`compatibility.ini`.** Another Firefox build refuses a locked or newer profile; `_prepare_session_from_login`, `test_da_publish.py::test_publish_batch_launches_firefox_headless_when_asked`.
- **Login is never automated; a session is VERIFIED site-side (`studio?new=1` must not bounce to `/users/login`), not from cookies.** PerimeterX blocks Playwright at login; a stale `userinfo` cookie outlives the `auth_secure` token; `_session_authed`, sole auth logic; `publish_batch` returns `_LOGIN_HINT` on False.
- **DA cookies from `.deviantart-login/cookies.sqlite` are re-injected via `ctx.add_cookies` before the auth check.** Firefox drops httpOnly `auth`/`auth_secure` from a copied profile, leaving the session logged-out; `_da_login_cookies`, `test_deviantart.py::test_login_cookies_empty_without_profile_db`.
- **Firefox runs headed by default; headless only when the caller passes it (`da-publish --headless`); the web UI never does.** Headless is itself a bot-detection signal; `publish_batch(headless=False)`, `src/publicator/webui/server.py` omits the kwarg, `test_da_publish.py::test_headless_flag_reaches_publish_batch`.
- **A .webp is never uploaded: `_pending_art` prefers a non-webp match and `_resolve_art` otherwise converts to a temp PNG with imagemagick `convert`.** DeviantArt rejects .webp; `_resolve_art`, `_pending_art`, `flake.nix` da-publish `runtimeInputs` includes `pkgs.imagemagick`.
- **Optional steps (premium, tiers, galleries) never touch the page when the entry has no price/tiers/galleries.** Clicking the toggle or a combobox would flip form state; early returns in `_step_premium`/`_step_tier`/`_step_galleries`, `tests/test_deviantart.py::test_optional_steps_are_no_ops_when_unset`.
- **Only DESCRIBED publications with DA apparitions in `state == 'unpublished'` are candidates; the flip to `published_or_scheduled` lands on that same apparition; git history is never consulted.** The state field is the single source of truth; `deviantart_apparition`, `load_pending_entries`, `store.mark_state`.
- **`--uuid` and `--all` are mutually exclusive; without either exactly the first pending entry is published.** Skill contract: "Publishes ONE entry" unless told otherwise; `da_publish.main`, `tests/test_da_publish.py::test_all_and_uuid_are_mutually_exclusive` + `::test_default_publishes_only_the_first_pending_entry`.
- **`pick_schedule` receives `date --date @TS` output (`Tue Sep 8 08:00:00 PM CEST 2026`); `parse_schedule`'s regex + `_MONTHS` expect English AM/PM and month abbreviations.** `format_schedule`/`parse_schedule` are a matched pair; `date` renders in the host's local timezone; `test_deviantart.py::test_parse_schedule`.
- **`TAGS_FILE` resolves as `<data-dir>/<top-level tags key>`, None when unset.** The tags file is data, not a code asset; `configure`, `tests/test_deviantart.py::test_configure_resolves_tags_file_under_data_dir` + `::test_configure_leaves_tags_file_unset_without_config`.

## Gotchas

- Label drift: `AUTOMATION FAILED: ... Locator ... Timeout` means DA renamed a label; selectors are literal text. cb88bbe: `PREMIUM_TOGGLE_LABEL` "Submit as Premium Download" became "Sell as Premium Download", tier heading lowercased (now non-exact; i18n has both). Recipe (`nix develop -c python`): `configure()`, `_prepare_session_from_login()`, launch on `SESSION_DIR`, add `_da_login_cookies`, `_session_authed`, `for _, fn in STEPS[:n]: fn(page, entry)`, dump `page.content()` + screenshot, grep `<label>` after stripping `<script>`. Fix constant AND SKILL.md step, `#check-steps`, re-run.
- A failed run past step 3 leaves an untitled draft in DA's studio (the upload already happened); no cleanup in code, delete it by hand. `_step_clear_tags` exists because a restored draft can arrive pre-tagged.
- Late failures can leave a draft: for example, no top-level tags key fails at step 8. Approved publications use the publication-level date, with historical submissionTimestamp fallback.
- Close `#login`'s Firefox first: it holds `cookies.sqlite` (read `?immutable=1` here). Unhandled, `compatibility.ini` shows an invisible downgrade modal (`page.goto` times out); a stale lock makes `launch_persistent_context` refuse the profile.
- opensnitch silently denies the Playwright Firefox binary on first launch: `NS_ERROR_NET_TIMEOUT` on every request while `page.evaluate` works; allow `/nix/store/...-playwright-browsers/firefox-*/firefox/firefox` once (`#login` prints the hint).
- pw-daemon runs Chromium on `.deviantart-session/`, which `publish_batch` refills with a Firefox profile; PerimeterX blocks its login. It is a DOM prober only; the skill's manual fallback is the signed-in Firefox from `nix run .#login`.
- `check_steps` returns True with "not found (dev-only check; skipping)" when `SKILL_MD` (REPO_ROOT = `deviantart.py` parents[2]) is absent: an installed copy passes the guard silently and `test_steps_stay_in_sync_with_the_skill` passes vacuously.
- `set_checkbox` JS-clicks DA's hidden `<input>` and is racy: 3 retries with 300 ms waits, RuntimeError "checkbox ... would not stay" if it never sticks; matureContent is re-asserted after the schedule dialog because it can drop.
- `pick_schedule` clicks "Go to the Next Month" forward only, 24x max, until `button[aria-label*='<Month> <day>']`: a past or >24-months-out month never shows, the day click times out. Hour: `select_option('#schedule-draft-time-picker', str(hour))`.
- `_step_galleries` reads the combobox's `inner_text` and skips a gallery already listed; blindly clicking would UNtick DA's default "Featured" folder. `_pick_in_combo` closes the combobox after picking so it can't overlay later steps.
- `_step_clear_tags` caps at 60 chip clicks (DA caps a deviation at 30 tags; one leftover overflows once add_tags types its 30). `span[role=button][data-tag]` tells chips from the "Copy Tags" button and is the selector to update if DA reworks tags.
- `format_schedule` shells out to `date` and `parse_schedule` requires `(AM|PM)` plus English month abbreviations: any locale without a 12-hour clock (`LC_ALL=C` included) breaks step 12 with "unparseable schedule".
- `_pending_art` / `entries.find_art_path` rglob the whole data dir for `*<stem>*` and take the first hit: a stem that is a substring of another file's name can resolve to the wrong image.
- `src/publicator/webui/server.py`'s POST /publish runs `publish_batch` in-process and headed, through the registered dispatcher: CLI and UI share exactly the same flow, so a DA breakage shows in both.
- No live test exists: the suite stubs `playwright.sync_api` entirely; the only proof a selector works is a real `#da-publish` run followed by reading back the DA studio list.

## Open items

- This repo's `.gitignore` lists `.deviantart-session/` but not `.deviantart-login/`; whether the data dir's `.gitignore` covers both auth-secret profile dirs cannot be verified from this repo.
- `format_schedule`'s dependence on `date` (host locale and host timezone) is untested; a `datetime.fromtimestamp(ts)` pair would remove the subprocess and the English-only regex.
