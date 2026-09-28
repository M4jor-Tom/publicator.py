# Publications store (publications.json + schema)

- **Does:** Owns `publications.json`, the source of truth for what gets published: append/patch/state-flip writers, lookup helpers, JSON-Schema + config allow-list validation, and the validate/echo-first CLIs.
- **Run:** `nix run .#validate -- [--data-dir <dir>]` reads `publications.json` + `publicator.toml`, prints `Valid` or raises.
- **Run:** `nix run .#echo-first -- [--data-dir <dir>]` prints path/title/schedule of the first state=unpublished entry.
- **Code:** `src/publicator/store.py` (`atomic_write_json`, `write_publications`, `mark_state`, `set_or_pop`, `apply_update`), `src/publicator/entries.py` (`STATE_UNPUBLISHED`, `STATE_PUBLISHED`, `set_data_dir`, `first_unpublished`, `deviantart_apparition`, `find_art_path`), `src/publicator/publicationsSchema.json` (`$defs.Publication`, `$defs.File`, `$defs.Apparition`), `src/publicator/config.py` (`SCHEMA`, `load_config`, `validate_publications`), `src/publicator/scheduling.py` (`zone`, `resolve_ts`, `existing_ts`), `src/publicator/apps/validate.py` (`main`), `src/publicator/apps/echo_first.py` (`main`)
- **Tests:** `tests/test_store.py` (append, schedule required, apply_update set/clear/unknown uuid, custom schedule timezone), `tests/test_config.py` (allow-list accept/reject; `_pubs` builds schema-length uuid/sha), `tests/test_scheduling.py` (resolve_ts naive string in schedule tz; existing_ts future-only, missing file), `tests/conftest.py` (autouse fixture restores `entries.DATA_DIR` after each test)
- **Config:** `publicator.toml` `[deviantart]` `tiers`, `galleries` (list[str], default []): allow-lists a DA apparition's `tier`/gallery must belong to, else `validate_publications` fails. `[schedule] timezone` (str, default `"Europe/Paris"` = `scheduling.DEFAULT_TZ`): zone the naive `schedule` string is resolved in.
- **Data:** `publications.json` (appended by `write_publications`, patched by `apply_update` + `webui.server`, flipped by `mark_state`; always via `atomic_write_json`), `publicator.toml` (read by `load_config`; validate passes `--data-dir`, `write_publications(config=None)` uses `Path.cwd()`), art files (`write_publications` sha512s the given `path`, stores only the basename; `find_art_path` rglobs `*<stem>*`, skips `.webp`), `src/publicator/publicationsSchema.json` (code asset resolved via `__file__` as `config.SCHEMA`, parsed once at import into `_SCHEMA`)
- **Decisions:** none
- **Verify:** `nix develop -c pytest tests/test_store.py tests/test_config.py tests/test_scheduling.py -q`
- **Verify:** `nix run .#validate -- --data-dir <data-dir>` (prints `Valid`, exit 0)

## How it works

1. `store.write_publications(entries, json_path, config)` (gallery `/publish` or `/stage`) loads the existing array (FileNotFoundError -> `[]`), computes `tz = zone((config or {}).get('schedule', {}))` and one `now = int(time.time())` for the batch.
2. Per entry `write_publications` runs `images.compute_sha512(path)` and `scheduling.resolve_ts(entry, tz)`, then wraps the result in `{uuid: str(uuid4()), submissionTimestamp: now, description, files: [{basename, sha512sum}], apparitions: [...]}`.
3. The single apparition is `{platformName: 'deviantart', state: STATE_UNPUBLISHED, urlElsePublicationName: title, apparitionTimestampIfDifferentThanSubmission: ts}`; `set_or_pop` adds `priceIfNotFree` (float), `tier`, `galleries` only when set.
4. `write_publications` then calls `config.validate_publications(data, config)` (`load_config(Path.cwd())` if `config is None`; jsonschema on `_SCHEMA`, then DA allow-lists), then `atomic_write_json(json_path, data)`; returns the new uuids in order.
5. `/stage` relays the uuids `write_publications` returned; the gallery page JS (`src/publicator/webui/page.py`, `e.uuid = d.uuids[i]`) stamps them on its queue entries, so a later `/publish` treats them as existing and does not re-append.
6. Patch (gallery `/update`): the server calls `store.apply_update(pubs, uuid, fields, zone)`, which finds the publication (for/else -> KeyError), sets `description` (keeps the old one if absent), then patches the entry's `deviantart_apparition(p)`.
7. There `apply_update` sets `urlElsePublicationName = fields['title']`, `apparitionTimestampIfDifferentThanSubmission = resolve_ts(fields, zone)`, and `set_or_pop`s price/tier/galleries; server then `validate_publications` + `atomic_write_json`.
8. `scheduling.resolve_ts` prefers a naive `schedule` ISO string via `datetime.fromisoformat(s).replace(tzinfo=tz)`, falls back to a raw `scheduleTs` epoch, and raises `ValueError('schedule required')` if neither: stored apparitions always have one.
9. Flip: `deviantart.publish_batch(entries, uuids, json_path)` calls `store.mark_state(json_path, u, STATE_PUBLISHED)` after each successful `submit_entry` (reload, set `deviantart_apparition(p)['state']`, atomic write); loop breaks on first failure.
10. Lookup (echo-first, `deviantart.configure`): `entries.set_data_dir(--data-dir or CWD)` retargets global `DATA_DIR`; `first_unpublished()` returns the first publication with ANY unpublished apparition; `deviantart_apparition` picks the DA one.
11. `apps.echo_first.main` then runs `find_art_path(basename)` and `format_schedule(ts)` (`date --date @TS`, the format `deviantart.parse_schedule` parses); exit 1 via SystemExit if no pending entry, DA apparition, timestamp, or non-webp file.
12. `apps.validate.main` reads `<data_dir>/publications.json`, runs `validate_publications(data, load_config(data_dir))`, prints `Valid`. `scheduling.existing_ts` reads the same file for future non-unpublished timestamps (scheduler occupancy).

## Invariants

- **Every write goes through `atomic_write_json` (mkstemp `.pub-` in the same dir + `os.replace`, indent=4).** A torn write would corrupt the git-tracked queue; it is the only `json.dump` in `src/`, callers `write_publications`, `mark_state`, `webui.server` `/update`.
- **Appends and patches validate (schema + allow-lists) BEFORE the atomic write; `mark_state` does not.** Bad tier/gallery names are caught at the trust boundary, not by the DA form; `store.write_publications`, `webui.server` `/update`, `tests/test_config.py::test_validate_rejects_{tier,gallery}_absent_from_config`.
- **Apparition state is the only "unpublished" signal; the sole transition is unpublished -> published_or_scheduled after a successful submit.** A failed run must leave the entry pending; `publish_batch` marks after `submit_entry`; `entries.STATE_*`, schema `Apparition.state` enum.
- Relies on stored `apparitionTimestampIfDifferentThanSubmission` being LOCAL wall clock in `[schedule] timezone` (naive `schedule` beats client `scheduleTs`), owned by `docs/features/scheduling.md`.
- Relies on `resolve_ts` raising `ValueError('schedule required')`, so a schedule is mandatory on write although the schema leaves the field optional, owned by `docs/features/scheduling.md`.
- **Cleared optional fields (price/tier/galleries) are removed, never stored as null/''/[].** The schema types `priceIfNotFree` number and `tier` string, and `deviantart._step_premium`/`_step_tier` skip absent keys; `store.set_or_pop`, `tests/test_store.py::test_apply_update_sets_then_clears_optional_fields`.
- **`tier` and `galleries` are DA-only.** Other platforms have no such fields; the schema's `$defs.Apparition` if/else forbids them on any other `platformName`, and `config.validate_publications` only checks allow-lists on deviantart apparitions.
- **The schema is a code asset resolved via `__file__`, never CWD/`--data-dir`, parsed once at import.** Apps run from the data dir; a CWD-relative schema would break the code/data split. `config.SCHEMA` / `_SCHEMA`; `src/publicator/apps/validate.py` has no `--schema` flag.
- **`uuid` and `freeVersionPublicationUuid` are exactly 36 chars; `sha512sum` exactly 128.** Schema minLength/maxLength; `write_publications` uses `str(uuid4())` and `images.compute_sha512`; `tests/test_config.py::_pubs` builds both for that reason.

## Gotchas

- `store.mark_state` silently no-ops on an unknown uuid and still rewrites the file; `apply_update`, by contrast, raises KeyError.
- `write_publications(config=None)` loads `publicator.toml` from `Path.cwd()`, NOT `json_path`'s dir, and only for the allow-lists: the timezone is `zone({}) = DEFAULT_TZ`. Run outside the data dir, every tier/gallery write fails validation.
- `apply_update` raises KeyError both for an unknown uuid and for a missing `title` key; `webui.server` maps any KeyError to 404 `uuid not found`, so a payload without title is reported as a missing entry.
- `entries.DATA_DIR` is a process-global mutated by `set_data_dir` (also from `deviantart.configure`); tests need conftest's autouse restore fixture or a `tmp_path` leaks into the next test.
- `entries.first_unpublished` matches state=unpublished on ANY platform, then callers take `deviantart_apparition`: a pixiv-only pending entry is returned and then SystemExit on the missing DA apparition.
- `entries.find_art_path` rglobs `*<stem>*` over the data dir, first non-`.webp` hit wins: a stem contained in another filename can resolve wrong; the check is case-sensitive (`.WEBP` passes) and lacks `is_file()`, unlike `deviantart._pending_art`.
- `entries.format_schedule` shells out to GNU `date --date @TS`; its output follows the system TZ/locale, not `[schedule] timezone`, and exists only to feed `deviantart.parse_schedule`.
- `submissionTimestamp` is the wall-clock moment of the write (one `int(time.time())` per batch), not the schedule; `apparitionTimestampIfDifferentThanSubmission` is always written despite its name.
- The schema requires neither `apparitions` nor `basename`/`sha512sum` in `files[]`: valid != publishable. echo-first SystemExits on a missing DA apparition/timestamp but tracebacks on a missing basename; da-publish `load_pending_entries` skips both.
- Real data carries the legacy file key `fileSha512sum` on most file rows instead of `sha512sum`; readers (`src/publicator/webui/calendar_view.py`, `images.load_publicated_hashes`) must accept both, and `nix run .#validate` still prints `Valid`.
- `description` is top-level (shared across platforms); title/URL, schedule, price, tier, galleries live on the apparition. After publishing, `urlElsePublicationName` is NOT rewritten to the DA URL; `mark_state` only flips state.
- Running `validate` from the code repo without `--data-dir` raises FileNotFoundError on `publications.json`.

## Open items

- Whether `urlElsePublicationName` should become the DA URL after publishing: the name and `calendar_view.entry_title`'s docstring say so, but nothing in `src/` writes it; most published DA apparitions have no http URL, so the calendar shows no link.
- `freeVersionPublicationUuid` and `platformName` `pixiv` exist in the schema but no code writes or reads them (pixiv appears only as the row `calendar_view.timeline` skips; `freeVersionPublicationUuid` only in the schema).
