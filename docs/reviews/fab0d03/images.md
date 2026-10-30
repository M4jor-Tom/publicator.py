# Artwork identity, destructive paths and codecs

Contract: `docs/features/images-thumbnails.md`. These findings concern current source behavior,
not observed real-artwork loss. Media and filesystem tests must use disposable roots only.

## ASSET-01 — P1 · Defect · Final unlink can remove an unauthorized replacement

**Evidence:** `src/images.rs:392,406–419` authorizes the root, checks identity, invokes
`before_unlink`, then removes by pathname. Store locking does not lock filesystem renames.
Confidence: high; requires concurrent filesystem mutation.

**Trigger/impact:** Replace the file after its final check: removal deletes different content.
Replace its parent with a symlink in that interval: removal can target an external directory.
Another immediately-before-unlink check alone would leave the same class of race.

**Existing coverage:** `tests/ui_workflow.rs:257–276` rejects earlier changes;
`src/workflow.rs:587–681` covers both Store-coordinated Pick/Delete orderings, not this window.

**Regression / close when:** Use `delete_candidate_with_hooks` and its existing `before_unlink`
hook to rename A aside and install B. Require B to survive. Add a Unix parent-substitution case
using another temporary directory. Bind destructive removal to authorized object/root identity;
retain both existing Pick/Delete orderings. No timing sleeps or real filesystem targets.

## ASSET-02 — P1 · Defect · Misnamed WebP bypasses conversion

**Evidence:** `src/images.rs:343–351` chooses conversion by suffix; detection recognizes WebP but
validation accepts it (`318–340`). Publisher calls them sequentially (`src/publishing.rs:437–440`);
protocol and worker reject only the suffix (`src/publish_protocol.rs:112–118`,
`python/publicator_da/worker.py:41`). Confidence: high.

**Trigger/impact:** Valid WebP bytes named `art.png` reach upload unchanged despite the explicit
DeviantArt no-WebP contract. Successful mixed-case `.WEBP` tests do not establish content policy.

**Existing coverage:** `tests/images.rs:306–321` and `tests/publishing.rs:290–337` test conversion
under a truthful extension.

**Regression / close when:** Generate an 8×8 WebP, copy it to a PNG filename and inspect the actual
prepared/dispatched bytes. They must be acceptable non-WebP content; test the inverse mismatch too.
Keep per-record staging, decoded dimensions and alpha checks. Base conversion/rejection on detected
content, not filename alone. Use a fake worker, no browser.

## ASSET-03 — P1 · Defect · AI input ignores stored artwork identity

Also reported as **META-R01**; one canonical item owns both reports.

**Evidence:** `src/images.rs:446–458` keeps the first basename match in traversal order.
`src/application.rs:308–326,263–268` resolves and prepares it without comparing the publication's
hash; both generation paths use it (`94,186–190`). Publishing separately verifies the stored hash
(`src/publish_journal.rs:207–237`). Confidence: high.

**Trigger/impact:** Replace recorded A with valid B at the same path, or introduce distinct visible
files sharing a basename. The application can generate from and externally disclose the wrong
artwork while retaining A's stored identity.

**Existing coverage:** `tests/images.rs:362–373` excludes hidden directories;
`tests/application_ai.rs:64–120` uses one correct image. Many successful AI fixtures retain the
placeholder digest from `tests/common/mod.rs:14–16`, masking identity failures.

**Regression / close when:** Store A's real hash, replace it with B, then attempt generation against
a request recorder: zero provider calls and no generated metadata on mismatch. Test duplicate
basenames without depending on enumeration order. Resolve by recorded identity or reject ambiguity;
any identity-refresh policy must be explicit, never silently accept changed bytes.

## ASSET-04 — P2 · Defect · Changed content can poison an old thumbnail identity

**Evidence:** `src/ui/preview.rs:17–22` trusts its supplied digest; cache fill converts the current
path without matching that digest (`src/images.rs:242–257`). Hits are trusted unconditionally.
Historical previews also supply recorded hashes (`src/ui/describe.rs:59–82`). Confidence: high.

**Trigger/impact:** Discover red A, replace with blue B before first lazy preparation, and cache B
under A's digest. Later, an unchanged copy of A reuses B's preview and misleads review/selection.

**Existing coverage:** `tests/images.rs:233–264` checks resize, names and reuse, including a sentinel
showing hits are trusted. It does not change the source between discovery and first fill.

**Regression / close when:** Preserve A elsewhere, replace the source with B, request a preview
under A's digest, then preview the preserved A. Assert pixels/content never show B under A's key.
Verify identity at cache creation while preserving valid cache reuse and lazy loading.
Use real tiny colored PNGs, real hashes and an empty cache.

## ASSET-05 — P2 · Risk / decision · Multi-frame detection concatenates format tokens

**Evidence:** `src/images.rs:309–335` runs `identify -format %m` over the whole sequence and compares
the entire output to one token. Expected two-frame GIF output is `GIFGIF`; audit execution did not
confirm it. Single-frame format tests (`tests/images.rs:324–351`) cannot resolve the issue.

**Trigger/impact to probe:** Valid animated GIF/WebP or multipage TIFF may fail preview, AI or
upload admission accidentally rather than follow an explicit sequence policy.

**Regression / close when:** Create two colored 8×8 frames/pages and run detection, preparation and
upload validation. Record actual output; define accepted formats and first-frame/animation/rejection
behavior per consumer, then assert it. Do not claim a runtime codec failure until this probe runs.
Keep allocations tiny; an arbitrary invalid `.mp4` fixture is not a valid-media rejection test.

## ASSET-07 — P2 · Defect · Prompt search omits basename reference protection

**Evidence:** Normal discovery excludes referenced basenames (`src/images.rs:142–153`), whereas
prompt search only excludes hashes (`src/prompts.rs:543–552,602–611`). UI accepts its candidates
(`src/ui/pick.rs:46–75`); Pick deduplicates hashes only (`src/workflow.rs:90–109`). Confidence: high.

**Trigger/impact:** A referenced prompt-matching filename changes bytes. Normal search hides it,
but prompt search offers it for another Pick, producing the same basename with another identity.

**Existing coverage:** `tests/images.rs:162–188` tests changed referenced names in normal search;
`tests/prompt_search.rs` lacks the matching publication-reference case.

**Regression / close when:** Use a real prompt-matching PNG, a stored reference with its original
hash, and a replacement PNG. Compare both search paths: neither offers the referenced filename.
Use existing Git/row helpers and share one reference-protection policy rather than duplicate guards.
