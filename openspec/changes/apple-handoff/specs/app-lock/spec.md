# Spec Delta

## ADDED Requirements

### Requirement: Continuations wait for unlock
While an enabled App lock is locked, the app SHALL hold the most recent incoming Handoff, universal-link or Spotlight continuation. It SHALL deliver that continuation only after successful unlock. A locked app SHALL NOT advertise Handoff activities. Signing out discards a held continuation.

#### Scenario: Handoff into a locked app
- **WHEN** a document activity arrives while the lock screen is showing
- **THEN** the lock screen stays in front without revealing the document, and after the user unlocks, the document opens

#### Scenario: Unlock cancelled
- **WHEN** a continuation is held and authentication fails or is cancelled
- **THEN** the app stays locked and the continuation stays held until a later unlock

#### Scenario: Several continuations while locked
- **WHEN** two continuations arrive before unlock
- **THEN** only the later one is delivered after unlock

#### Scenario: Spotlight result while locked
- **WHEN** the user picks a Folio Spotlight result while the app is locked
- **THEN** the document opens after unlock instead of the result being dropped

#### Scenario: Locking stops advertising
- **WHEN** the app locks while a document activity is advertised
- **THEN** no Folio activity is advertised until the app is unlocked and the screen is shown again
