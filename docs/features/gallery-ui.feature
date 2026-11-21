Feature: Review artwork in the native gallery

  Rule: Pick searches a runtime subset of configured publicable roots

    Scenario: Open Pick for the first time
      When the UI starts
      Then all configured publicable paths are selected and the candidate limit defaults to 25
      And the available limits are Empty/0, 25, 50, 75, or 100

    Scenario: Inspect and pick a candidate
      When the user opens Show on one candidate and then picks it
      Then Show and Hide affect only that card's archived text or non-authoritative prompt candidates
      And picking keeps Pick open; a digest filename and its delete-confirmation path wrap inside their containers

    Scenario: Switch tabs or jump to a publication
      When the user returns to a tab
      Then the tab retains its scroll position unless a Jump puts the revealed publication at the top
      And native selection controls indicate focus and work from the keyboard

    Scenario: Scroll to unseen previews
      When pending placeholders come within one thumbnail of the viewport
      Then at most four previews load concurrently and more load as places free up
      And compact Calendar events request artwork only when expanded

    Scenario: Inspect a loaded thumbnail
      When the user clicks a loaded thumbnail in any workflow tab or expanded Calendar event
      Then the original fits a full-window black viewer, or the cached preview is used for mp4
      And click or Escape closes only the viewer, restoring the tab and scroll position; Publish shortcuts do nothing while it is open

  Rule: Editing and deletion use the same persistent operations as the CLI

    Scenario: Confirm deletion after scrolling
      When the user opens Delete on a Pick, Schedule or Describe tile
      Then the confirmation stays visible without moving the list
      And confirming in Pick deletes the unpicked file; in Schedule or Describe it removes only the entry and its unsaved draft
      And a DESCRIBED row must use Return to Describe in Publish before tile deletion; the CLI can delete it directly

    Scenario: A draft changes while its save is in flight
      When the earlier save completes
      Then the newer revision saves next and lands last, with at most one save in flight for the draft
      And phase changes flush drafts; a failed save stays visible for retry
      And reload shows persisted text wherever no unsaved or in-flight field takes precedence

    Scenario: Refresh while a custom date is unsaved
      Given date approval is refused while the row has unsaved custom input
      When the user refreshes
      Then workflow and history reload from disk, prompt labels resolve against a refreshed archive, and unsaved custom dates are discarded
      And saving a date also clears that row's custom input

    Scenario: An old response arrives after a reload or publish run
      When a stale reload, calendar, thumbnail or publish reply arrives
      Then it cannot change the current rows or an already finished or different publish run

  Rule: Describe shows per-card progress and preserves finished results until refresh

    Scenario: Generate marked metadata
      When the batch starts
      Then cards move from Queued to Generating… and then Generated or Failed with the error
      And only the active card has an accent-filled label; each finish reloads its persisted text
      And the summary moves from Preparing N… to Generating K of N · D done · F failed to Generated S of N, with failures counted
      And admission, preflight or batch failure shows Generation failed with the error

    Scenario: Act on a busy or finished card
      When the user edits, deletes or approves a queued or generating card
      Then nothing changes and the UI says "Wait for this image's generation to finish"
      And Generate marked during a batch says "A batch generation is already running"
      And a generated DESCRIBED card stays read-only in Describe until Refresh; its actions belong to Publish

    Scenario: Generate one publication
      When the user chooses Generate one
      Then only that card's state is updated for the new generation
      And other finished states and the previous batch summary remain until replaced or successfully refreshed

  Rule: Publish selection is not submission

    Scenario: A delivery has succeeded or its outcome is unresolved
      When the user views Publish
      Then its ordinary rows and count include only pending DeviantArt deliveries; successful sends remain in Calendar
      And a completed row opened from Calendar or a row awaiting reconciliation is locked
      And only registered platform cells can execute; retries skip recorded successes

    Scenario: Return a pending row to Describe
      Given a DESCRIBED row with a pending DeviantArt delivery
      When the user clicks Return to Describe in Publish
      Then the CLI's roll-back makes it SCHEDULED, so it leaves Publish and shows in Describe with its saved title and description
      And the button is disabled while another operation or a publish run is active, or an unresolved attempt locks the row
      And unsaved options refuse it until saved; delivered and APPEARED rows have no such button

    Scenario: Close, refresh or submit again during publishing
      When a close is requested, including after draft discard, save or status refresh
      Then the UI offers Wait or Cancel and does not exit before the durable result and worker cleanup
      And Refresh and a second publish are refused while the run is active

    Scenario: Override the initial OpenRouter vision model
      When starting the UI with --openrouter-model NAME
      Then a supported model with or without the openrouter/ prefix is selected; an unsupported name fails at startup
