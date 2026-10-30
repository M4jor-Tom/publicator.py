# Native interaction acceptance

Iced is a native application, not an HTTP UI. Logic tests drive real Application/Store operations,
but injected messages, compiled examples and screenshots of an idle window do not establish human
interaction behavior. No native-window story was executed during this audit.

## UI-C02 — P2 · Gap · Existing harness does not replay acceptance interactions

Also reported as **VER-06** (P3 in that lane); use this one P2 item for the combined coverage scope.

**Evidence:** `examples/ui_acceptance.rs:1313–1379` runs real widgets with loopback metadata and a
synthetic worker. `scripts/native-ui-smoke.sh:62–74` launches empty/history/mixed/reopen, checks a
visible window, then waits for human closure. It has no input driver or persisted-state assertions;
no checked-in JS/TS replay was found. Cargo all-targets compiles/tests the example but does not run
its native main as a user story.

**Existing coverage:** `tests/ui_describe.rs:789–873` injects viewport ranges and counts cached
files; `tests/ui_publish_calendar.rs:1662–1743` checks a jump operation exists, not that its target
is visible. Example tests validate fixture schema/fault worker behavior and initial-byte
preservation (`examples/ui_acceptance.rs:853,1011,1108,1168,1205`). Historical manual evidence is
not invalidated; it simply cannot be replayed from the inspected repository.

**Close when:** Reuse the existing Xvfb/window manager/VNC/noVNC bridge with loopback binding and
browser automation of the actual native canvas. Start with bounded launch/close and one persisted
keyboard action, then cover the stories below. Capture failing screenshots, persisted JSON and
fake-call/PID logs outside Git; bound startup, input, waits, exit and cleanup.
Do not run the current human-wait script as an unattended CI gate.

Required replay/checklist cases, each with observable state rather than click-only success:
- Empty, history, mixed and reopen; also explicit publish, calendar, prompt-search and
  prompt-search-missing-archive scenarios currently omitted by the launch script.
- Pick search/subset/limit, selection, real delete confirmation and visible-list removal.
- Schedule custom wall time, preset, refresh and persistence without mutating on render.
- Describe multiline Unicode, newlines and literal markup; F6/Tab/Space focus/selection;
  debounce, explicit saves, dirty field preservation, failure/retry and restart persistence.
- Actual model-install consent controls: refusal makes no pull; stale consent/source/model/target
  cannot act on a newer selection. All services remain injected loopback fakes.
- Publish persisted cells/options, selected-set drift, safe failure, retry and unknown reconciliation;
  gate a worker and verify all close/cancel/discard/refresh routes wait for required cleanup.
- Calendar configured-zone boundaries, exact/nearest/unknown prompt states, long text overflow,
  captured URL opening and phase jumps that put the actual target onscreen.
- Resize and scroll at multiple dimensions; later-visible distinguishable images load correctly.

**Safety:** Use a fresh ui_acceptance fixture root, not the production UI plus arbitrary dummy
artwork. The example already injects endpoints, dummy key, fake fit results, workers and gates
(`examples/ui_acceptance.rs:571–642,1348–1365`). Never expose the user's desktop, real credentials,
Art directory, login profile or live publisher. Fake artwork alone is not service isolation.
Live DeviantArt compatibility remains a separately authorized activity.

## UI-R01 — P2 · Risk · Estimated viewport geometry may miss visible previews

**Evidence:** `src/ui/style.rs:82–95` estimates ranges from fixed row heights. Pick has controls
and separate lineage grids (`src/ui/pick.rs:272–310`); Describe prepends controls
(`src/ui/describe.rs:136–148`); Publish rows grow with option lists (`src/ui/publish.rs:399–445`).
Tests inject geometry rather than measure these rendered widgets. No missing-thumbnail incident
was reproduced by the audit.

**Regression / close when:** Render at least 30 visually distinguishable images, long prompt
panels and several tier/gallery options. At narrow/wide sizes scroll each affected view; every
visible card must load its own correct image without extra scrolling. Inspect pixels/identity,
not only cache counts. Preserve lazy bounded preparation; change geometry only if the probe
reproduces a failure. Use UI-C02's existing native harness, not a second fake renderer.

## Acceptance boundary

Package/runtime ownership is tracked by VER-03 in [test infrastructure](test-infrastructure.md),
including the original UI-R03 observation. Pure action-stream close correctness is UI-C01; actual
window exit is an additional UI-C02 assertion. Neither can substitute for the other.
