# UI state, persistence and lifecycle

Contract: `docs/features/gallery-ui.md`. Run message/task regressions with temporary Stores and
injected services. Actual input/rendering evidence is tracked separately in [native UI](native-ui.md).

## UI-01 — P1 · Defect · Alternate close paths bypass active publishing

**Evidence:** Ordinary close waits (`src/ui/mod.rs:1798–1807`), but footer ConfirmDiscard is always
available (`2563–2568`) and exits unconditionally (`778–786`). Refresh admits reload during publish
(`758–764`); receipt loading fails on the held publisher lock (`591–607`,
`src/publish_journal.rs:123–140`). Both status success/error continuations may close on pending_close
and clean drafts without checking publishing (`src/ui/mod.rs:1228–1265`). Confidence: high.

**Trigger/impact:** Hold a delivery and choose Discard-and-close; or request close then Refresh
and consume its lock-failure result. Exit precedes PublishFinished/cleanup. Durable intents reduce
duplicate risk but a normal UI action still unnecessarily interrupts delivery and requires recovery.

**Existing coverage:** `tests/ui_publish_calendar.rs:422–477,613–681` tests normal waiting/cancel,
not either bypass; its Output-only task collector also misses Exit (UI-C01).

**Regression / close when:** Reuse held_publisher_worker (`tests/ui_publish_calendar.rs:370–397`)
and exercise both routes with clean/dirty drafts. Observe zero Exit actions before release and
durable outcome/process cleanup before eventual close. Gate every exit continuation, including
discard and status errors; discarding a draft must not discard publisher lifecycle ownership.

## UI-02 — P1 · Defect · Repeated explicit saves can persist an older revision last

**Evidence:** Enter/Save remains wired during saving (`src/ui/describe.rs:217–226,257–262`).
`src/ui/mod.rs:2290–2304` checks dirty/generation state but not is_saving; each call snapshots a patch
and starts an independent Store mutation (`2266–2287`). Store writes overwrite supplied fields
(`src/workflow.rs:123–140`). Completion revision checks clear dirty state but cannot undo stale
write ordering (`src/ui/drafts.rs:106–110,161–169`). Confidence: high.

**Trigger/impact:** Save A, edit/save B before A finishes, execute B first then A. Disk ends at A
while the UI displays B as clean/saved. Store locking prevents simultaneous corruption, not this.

**Existing coverage:** `tests/ui_drafts.rs:4–32` tests bookkeeping;
`tests/ui_describe.rs:876–962` tests one held save and a chained newer save, not two explicit saves.

**Regression / close when:** Retain A's task unpolled, request B, execute B/completion then A.
Newest disk value and correct dirty state survive. Reuse the per-draft saving flag and completion
continuation to serialize flushes; leave unrelated-record editing available. No provider needed.

## UI-03 — P2 · Defect · Clean drafts override refreshed authoritative content

**Evidence:** Successful saves retain drafts (`src/ui/mod.rs:1507–1535`); reload updates rows/editors
without rebasing clean drafts (`1185–1225`). Describe always prefers draft values
(`src/ui/describe.rs:196–204`), as do new edit initialization paths (`src/ui/mod.rs:1457–1475`).
Provider failure writes markers (`src/workflow.rs:171–176`) then reloads (`src/ui/mod.rs:1419–1434`).
Confidence: high.

**Trigger/impact:** Save manual text, update Store through another writer or fail generation, then
Refresh. Old clean title/draft state remains visible despite different disk values. Description
editor state and draft-derived status can also diverge.

**Existing coverage:** Generation/save tests such as `tests/ui_describe.rs:965–991` assert disk,
not retained draft presentation.

**Regression / close when:** Drive real save tasks, update Store, Refresh and complete tasks.
Displayed/clean metadata matches disk while dirty/in-flight fields remain intact; include generation
failure markers. Inspect private App state without adding public testing getters. Reconcile/prune
stale presentation copies, not a second cache.

## UI-04 — P2 · Defect · GUI model override is accepted but unused

**Evidence:** `src/bin/publicator-ui.rs:19–20,48–52` parses/forwards openrouter_model, but App always
selects VISION_MODELS[0] (`src/ui/mod.rs:396`) and generation reads self.model (`2452–2457`). No later
read of the option was found. Confidence: high.

**Impact/coverage:** Another or invalid model name silently uses the default. UI fixtures always
pass None; `tests/cli.rs:266–275` only checks rejection of the removed limit flag.

**Regression / close when:** Construct App with a second supported model and inspect the loopback
wire request. Unsupported values fail before provider work. Honor/validate the existing option or
explicitly reject/remove it; do not keep silently dead configuration.

## UI-C01 — P2 · Gap · Close tests discard the very Exit action they need to observe

**Evidence:** `tests/ui_publish_calendar.rs:184–195` filters everything except Action::Output;
`650–654` treats no collected messages as proof that close waited. An immediate Exit also produces
no messages under that collector. Success-close assertions likewise fail to establish an actual exit.
Confidence: high; this is a test-oracle defect, not native-window proof.

**Regression / close when:** Extend the existing driver to retain/classify lifecycle and widget
actions. A deliberate immediate Exit cannot satisfy wait assertions. Verify zero exits while save
or publish is pending, and one after successful final flush and cleanup. Include failed-save close
and refresh/discard paths. Reuse/consolidate existing draining loops rather than add a new framework.

## UI-R02 — P2 · Risk / decision · Reload/preparation replies can arrive out of order

**Evidence:** Search/local-fit results carry identities, but LoadFinished and CalendarPrepared do
not (`src/ui/mod.rs:108–109,244`). Reload resets preparation (`1185–1215`); any preparation completion
replaces it (`1064–1075`). No reversed-completion runtime defect was reproduced.

**Regression / close when:** Hold two snapshot/preparation results around a Store change, deliver
new then old and compare rows, artwork and Calendar state. Reject stale effects if demonstrated.
Also examine Save Custom → Preset → Refresh: `src/ui/schedule.rs:101–105` prefers custom_dates,
and reload never clears the map. Decide intended retained-input semantics before calling that
behavior a defect. Use deterministic held Tasks, not race sleeps or speculative request plumbing.

## UI-C03 — P2 · Gap · Duplicate completion needs to pass through the actual reducer

**Evidence:** Publish/recovery tests in `tests/ui_publish_calendar.rs:946–1392` cover persisted
selection, stale sets, failures and reconciliation. The inspected duplicate test does not apply
the rejected duplicate completion through App::update, so reducer idempotence remains unproven.

**Regression / close when:** Deliver the same completed attempt through App::update twice, with a
newer UI operation between where relevant. Assert no duplicate history/write, wrong selection reset,
extra send, stale error overwrite or lease/close effect. Observe task actions as well as disk bytes;
keep unknown receipts and retry protection intact.

## CLI-C01 — P2 · Gap · Successful reconciliation lacks a complete CLI witness

**Evidence:** `tests/cli.rs:219–834` covers external CWD, argument conflicts, check-only, PTY consent,
worker failures and unknown exits; successful CLI reconciliation end-to-end remains a gap.

**Regression / close when:** Create a synthetic unresolved receipt with a fake worker, reconcile
through the public CLI using each supported human decision, restart/read status and assert the
intended persisted result and retry eligibility. Unknown stays blocked until that explicit action.
Use unrelated CWD and temporary data; no real remote inspection or browser access.
