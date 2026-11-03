## ADDED Requirements

### Requirement: Typed event occurrence conditions
Conditions MAY select an event subject ID and either a system actor or a specific user ID. Matching SHALL use the captured envelope and SHALL NOT substitute current document metadata for an absent snapshot. Document and event predicates SHALL match within the same all/any/none branch. Unknown event filter fields and blank user IDs SHALL be rejected. Pre-import workflows SHALL reject committed-event filters.

#### Scenario: Explicit system actor
- **WHEN** an event condition selects the system actor
- **THEN** a captured envelope with a null actor matches and absent event context does not

#### Scenario: Mixed alternative branches
- **WHEN** one any branch matches document metadata but fails its actor filter and another fails metadata but matches its actor filter
- **THEN** the condition does not match

#### Scenario: Entry without a document snapshot
- **WHEN** an entry occurrence has no historical document snapshot
- **THEN** event predicates can match but document predicates cannot
