# Prompt archive and rendering

Contract: `docs/features/prompt-mapping.md`. Existing tests cover historical/deleted/duplicate paths,
unreachable blobs present before construction, content-digest identity and concurrent readers.
All proposed repositories are disposable; never mutate the real prompt archive.

## ASSET-06 — P2 · Decision · Refresh invalidation does not cover the indexed universe

**Evidence:** `src/prompts.rs:197–207` skips rebuilding when HEAD is unchanged, but the index reads
all objects and refs (`222–241`). After warm-up, new unreachable blobs and non-HEAD branch changes
stay stale beyond the two-second probe TTL. Confidence: high in source behavior.

**Qualification:** HEAD-based refresh is itself documented historical behavior. This needs an
explicit freshness contract, not an invented guarantee that every new object is already promised
immediate visibility.

**Existing coverage:** `tests/prompts.rs:231–254` changes HEAD; `289–324` writes the unreachable blob
before constructing the archive. Neither covers post-warm-up object/ref changes.

**Regression / close when:** Warm a committed archive, add a blob with unchanged HEAD, age the probe
without a sleep where private test access permits, then resolve it. Repeat with another-ref update
and object removal. Decide whether automatic invalidation covers that universe or a documented
explicit refresh is required; implement/test the chosen guarantee consistently with the owner docs.
Do not remove all-object/unreachable lookup merely to make the mismatch disappear accidentally.

## ASSET-08 — P2 · Defect · Git quoting corrupts lineage filenames

**Evidence:** `src/prompts.rs:231–241,365–400` parses line-based `git log --raw` without `-z` and
keeps the post-tab text literally. `core.quotePath=false` still escapes quotes, tabs, newlines and
backslashes. Confidence: high in source/command semantics, not a runtime witness from this audit.

**Trigger/impact:** A prompt named `dir/a"b.txt` resolves by content but its stored path is Git's
quoted representation. Reported paths, lineage lookup and explicit lineage search are wrong.

**Existing coverage:** `tests/prompts.rs:181–228,328–359` uses ordinary historical/duplicate names.

**Regression / close when:** Commit quoted, tab, newline and backslash filenames in an isolated
repo; Exact paths round-trip byte-for-byte, and a missing digest with that lineage yields Nearest.
Consume unambiguous NUL-delimited path output instead of inventing a partial quoting decoder.
Retain handling of removed paths and duplicate basenames.

## ASSET-09 — P2 · Decision · Nearest text contradicts the maintained no-render contract

**Evidence:** `docs/features/prompt-mapping.md:9,21` says Nearest has no renderable prompt text.
`src/prompts.rs:342–354` supplies full versions and `src/ui/prompt.rs:42–49` renders their bodies.
The UI has a non-authoritative warning; it does **not** falsely label these Exact.

**Existing coverage:** `tests/prompts.rs:181–208` asserts Nearest candidate contents;
`src/ui/prompt.rs:59–87` checks warning/paths but not suppression of the bodies.

**Regression / close when:** Decide the intended product contract. If no-render remains the rule,
use a sentinel body and assert Nearest shows only warning/lineage while Exact still shows text.
If candidate bodies are intentionally supported, update maintained docs and assert unmistakable
non-authority plus separation from Exact. Use pure in-memory matches; do not silently turn candidate
text into authoritative provenance. No code change is authorized until the policy is resolved.

## PROMPT-C01 — P2 · Risk / gap · Search limits may starve later eligible results

**Evidence:** `tests/prompt_search.rs:111–140` establishes truncation for 205 matches. The report
identified a separate untested ordering: the first 200 archive matches may already be referenced,
leaving later available artwork beyond the bound. No user-visible failure was reproduced.

**Regression / close when:** Create a small synthetic archive/name corpus with referenced early
matches and eligible later matches, plus malformed/unreadable files. Assert the intended meaning
of the limit, truncation notice and available-result selection. Preserve reference parity from
ASSET-07; do not enlarge an arbitrary cap without deciding which operation is being bounded.

## PROMPT-C02 — P3 · Risk / gap · Archive buffering and framing lack resource probes

**Evidence:** `src/prompts.rs:128–150,222` buffers the complete all-object cat-file result, then
copies blob bodies into derived indexes. No measured memory exhaustion was observed. Existing
atomic derived-map and concurrent-reader tests remain valuable (`tests/prompts.rs:258,289`).

**Regression / close when:** With a bounded moderate repository or subprocess-stream seam, measure
peak behavior and exercise malformed/truncated framing, missing/unavailable repo and failed refresh.
Readers must not observe a partial replacement index. State an acceptable archive-size ceiling
before considering streaming; do not allocate giant blobs or redesign the index speculatively.
