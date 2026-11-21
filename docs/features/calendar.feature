Feature: Read-only publication calendar

  Scenario: One publication appears on two platforms
    Given a publication with a saved date and two apparitions, one on an unregistered platform
    When Calendar loads the store
    Then it shows two events with the publication's shared title and date
    And each retains its own platform, delivery state and actual URL

  Scenario: First opening starts on today's week
    Given months of history before the current month
    When Calendar is first shown in a session
    Then the week containing today in the configured schedule timezone is at the top of the list, with earlier months above it out of view
    And if today cannot be placed, the list starts at the top

  Scenario: Bucket an event near midnight
    Given no date override and a submission timestamp on different dates in UTC and the schedule timezone
    When Calendar places the event
    Then the day is the submission date in the configured schedule timezone

  Scenario: Expand an event on a crowded day
    When a day's events and expanded artwork need more than the 78 pixel minimum
    Then the day shows every event without clipping
    And all cells in that week, including blank cells, grow to the tallest day's height

  Scenario: Two records share a basename
    Given two publications record different hashes for identically named artwork
    When their events are expanded
    Then each shows only its own artwork or recorded preview
    And an unresolvable row remains a text event with an Artwork unavailable reason

  Scenario: Navigate without changing a publication
    Given an event with a nearest prompt match but no valid stored URL
    When the user inspects the event and jumps to its publication
    Then the candidate prompt is not presented as exact and no URL is guessed
    And the relevant workflow phase opens without reserving, approving or publishing anything
