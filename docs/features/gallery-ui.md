# Gallery UI

- **Does:** Durable human review in Pick / Schedule / Describe / Publish phases, plus a read-only calendar; phase labels indicate the next action, not stored state.
- **Run:** `nix run .#ui -- [--data-dir DIR] [--json FILE] [--ai-model ID] [--openrouter-model ID] [--ai-timeout 300] [--port 8765] [-v]` (Search defaults to 25 candidates and all configured `publicable` paths selected)
- **Code:** `src/publicator/webui/page.py` (pure renderer and small client), `src/publicator/webui/server.py` (`GalleryHandler`, `create_handler`, `serve`), `src/publicator/apps/ui.py` (startup); persistence owned by `src/publicator/store.py`, dispatch by `src/publicator/platforms.py`.
- **Tests:** `tests/test_page.py` (escaping, phase forms, accessible selection grid), `tests/test_server.py` (real HTTP + real JSON persistence, reopen, AI races, retry, batch guards), `tests/test_scheduling.py` (server allocation).
- **Config:** `publicator.toml`: `publicable`, `[deviantart]` allow-lists, read-only `[schedule]` profiles, optional `[prompts]`; local model availability from llmfit/Ollama.
- **Data:** `publications.json` is authoritative on every request/refresh; `.thumbs/` is a lazy content-addressed cache. Disk deletion is restricted to unpicked candidate paths; no persisted publication is removed by the UI.
- **Decisions:** Native inputs/popovers and server-rendered HTML; no database/frontend framework, RAM-only publish queue, or browser-clock allocation. Provider/browser calls run without store mutation locks.
- **Verify:** `nix develop -c pytest tests/test_page.py tests/test_server.py tests/test_scheduling.py tests/test_docs.py -q`

## How it works

Search chooses a runtime subset of the configured `publicable` paths; paths are
never arbitrary filesystem inputs and TOML is read-only. Its candidate limit is
Empty/0, 25, 50, 75, or 100; launch defaults to 25 with every configured path
selected. The four phases expose different actions, so picking does not imply a
platform, metadata, or approval. UI mutations delegate to application operations,
which validate and persist; refresh and reopening reconstruct phases and calendar
from disk, including history-only data. Publication state meanings belong to
[publications-store](publications-store.md).

## Invariants

- `/pick` takes an allow-listed candidate path. Other publication actions take a UUID and resolve fresh stored records; `/publish` accepts no client metadata or queue. `/stage` and `/update` no longer exist.
- `/schedule`, `/schedule/approve`, `/metadata`, `/metadata/approve`, `/ai`, `/ai/batch`, `/selection`, `/selection/all`, and `/options` persist before reporting saved. Explicit backwards actions are `/schedule/return` and `/metadata/return`; successful deliveries forbid shared-content rewrites.
- Manual drafts debounce, phase changes flush pending saves, failed saves remain dirty, and leaving unsaved work triggers a native warning. AI always requests both title and description; only a valid pair advances SCHEDULED to DESCRIBED. Single generation preserves the per-card action. “Describe all marked” selects only SCHEDULED records with an exact top-level TODO marker, merges only fields still bearing the exact marker, saves independently, and continues after failures. It tries the last eligible record first; OpenRouter prefers the first successful model on remaining records. Local install-on-use asks consent.
- All eligible registered-platform cells are selected/unselected in one store mutation by select-all. Native `indeterminate` reflects partial selection; successful cells are locked. Selection never submits; Publish selected / Retry is separate.
- A batch guard rejects simultaneous publish attempts and HTTP mutations while publishing. Only registered pending cells participate in executable preflight, dispatch and remaining/unattempted counts; historical pending cells are preserved and do not block registered targets. Historical-only pending data publishes nothing, and undelivered historical apparitions still prevent APPEARED completion. Accepted AI requests block duplicate generation and phase advancement for that UUID until results/markers are saved; unrelated drafts remain editable.
- Native controls disable autocomplete so Firefox reload restoration cannot override saved drafts/selections.
- Errors and results use `textContent`, not HTML interpolation. Dynamic model JSON lives in escaped attributes, never script text. Prompt blocks/search/delete remain on candidate cards; calendar prompt popovers retain the shared renderer.
- Originals and thumbnails retain exact allow-lists of rendered paths/names, including search results. Arbitrary paths, obsolete request fields and unregistered platforms are rejected.
- Every mutation requires `application/json` (charset parameters allowed). Browser `Origin` must match the bound HTTP address or `localhost` at that port, never an origin inferred from the request's `Host`; foreign/opaque origins are rejected before dispatch. Non-browser JSON clients without `Origin` remain supported.

## Gotchas

- Publication results stay in this tab's session storage across reload; durable success/pending states live in JSON. Other tabs see successes on refresh, not an automatic live update. The server stays open after attempts; stop it with Ctrl-C.
- Search reloads candidates from the selected configured paths; prompt search walks those paths with a 200-result cap before hashing. The UI may select only a runtime subset of TOML `publicable` paths. Missing stored artwork remains visible rather than silently disappearing.
- The localhost UI is intended for one trusted human, not deployment on a public interface. Publishing is synchronous per request but other requests can read the workflow during it.
- Browser/UI checks must use disposable data and stub publishers/providers. Never demonstrate a workflow by submitting real artwork or changing the Art directory.
