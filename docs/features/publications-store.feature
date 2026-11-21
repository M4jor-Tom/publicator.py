Feature: Persist publication state in publications.json

  Rule: Every mutation validates a fresh, complete document under the persistent store lock

    Scenario: Concurrent writers change different records
      When each writer acquires the thread and advisory file locks for its mutation
      Then it rereads current bytes and validates before atomically replacing the file
      And unchanged keys retain their order and JSON uses four-space indentation
      And provider and browser work take place outside the mutation lock

    Scenario Outline: Reject an invalid document rather than treating it as an empty queue
      Given the document contains <fault>
      When it is read for mutation
      Then validation fails before the callback and the source bytes are unchanged

      Examples:
        | fault                           |
        | malformed JSON                  |
        | two publications with one UUID  |
        | a non-array apparitions field   |

    Scenario: A publication has no targets
      Given the apparitions field is absent
      When the publication is loaded
      Then it has no targets, just as if the field were an empty array

    Scenario: Recover an uncertain delivery
      Given phase, title, description and date belong to the publication and delivery state belongs to each apparition
      When a send may have reached its platform without a local success record
      Then its durable receipt survives restart and blocks automatic resend until reconciled under the publisher lock
      And retry considers only fresh registered pending targets and skips saved successes

  Rule: Deleting a publication does not delete its artwork or its receipts

    Scenario: Delete one undelivered entry
      Given a publication with no successful delivery or unresolved attempt
      When a user confirms deletion in the UI or runs delete-publication for its UUID
      Then only that entry is removed and automatic date suggestions are repacked
      And the artwork is available to Pick again and any resolved receipt remains on disk

    Scenario Outline: Refuse deletion
      Given <protected state>
      When deletion is requested
      Then the entry and artwork are unchanged

      Examples:
        | protected state                                          |
        | another publisher holds the publisher lock               |
        | generation is running for this UUID                      |
        | an unresolved attempt names this UUID                    |
        | any apparition is published_or_scheduled                 |
        | the publication is APPEARED                              |

  Rule: Rolling back moves an undelivered publication back exactly one state

    Scenario: Roll back one step at a time
      Given a DESCRIBED publication with no successful delivery or unresolved attempt
      When a user runs roll-back for its UUID
      Then it becomes SCHEDULED, and a second roll-back makes it PICKED
      And its title, description and dates are kept

    Scenario Outline: Refuse rollback
      Given <protected state>
      When roll-back is requested
      Then the entry is unchanged

      Examples:
        | protected state                                                       |
        | the publication is PICKED, and the error points at delete-publication |
        | the publication is APPEARED                                           |
        | any apparition is published_or_scheduled                              |
        | an unresolved attempt names this UUID                                 |
        | another publisher holds the publisher lock                            |

  Rule: Legacy migration is a validated transformation, not title guessing

    Scenario: Recover a partially delivered publication
      Given complete metadata and a legacy date or successful delivery
      When inferring the publication phase during migration
      Then partially delivered history is DESCRIBED rather than PICKED
      And no title is inferred from a URL slug or fetched by modifying a browser profile

    Scenario: Read a recognized legacy urlElsePublicationName field
      When migrating its value
      Then a string is accepted, null is treated as absent, and any other type produces a contextual error
