# app-lock Specification

## Purpose

Protect the Apple app's document interface with optional device-owner authentication while keeping integrations independent.

## Requirements

### Requirement: Optional device-local app lock
The Apple app SHALL offer an App lock setting, disabled by default, persisted locally per installation. Enabling or disabling it SHALL require successful device-owner authentication. It SHALL offer immediate, one-minute, and five-minute absence intervals, defaulting to one minute.

#### Scenario: Enable with authentication
- **WHEN** the user enables App lock and completes device-owner authentication
- **THEN** the setting persists and the current session remains unlocked

#### Scenario: Cancel a setting change
- **WHEN** authentication fails, is unavailable, or is cancelled
- **THEN** the previous setting remains unchanged

### Requirement: Lock lifecycle
An enabled App lock SHALL start locked on launch, lock on a device-lock event or Lock now, and require authentication when returning after the selected absence interval. Failure, cancellation, or stale authentication completion SHALL NOT unlock the app.

#### Scenario: Return during grace period
- **WHEN** the app returns before the configured absence interval expires
- **THEN** its existing document UI becomes visible without another authentication request

#### Scenario: Return after grace period
- **WHEN** the absence reaches the configured interval
- **THEN** returning to the app shows the lock screen

#### Scenario: Device locks during authentication
- **WHEN** a device-lock event occurs before authentication completes
- **THEN** the app remains locked even if that old request later succeeds

#### Scenario: Authentication prompt changes application focus
- **WHEN** a system authentication prompt temporarily makes the app inactive
- **THEN** it does not cause a second lock cycle after successful authentication

### Requirement: Privacy across windows and presentations
An enabled App lock SHALL hide document content while inactive, including presented sheets and previews. Locked app windows SHALL NOT expose document actions or content.

#### Scenario: App switcher during grace period
- **WHEN** an unlocked app becomes inactive with a document preview open
- **THEN** an opaque privacy cover hides the preview immediately

#### Scenario: Multiple windows
- **WHEN** the user locks one Folio window
- **THEN** all Folio windows require unlock

### Requirement: Integration boundary
App lock SHALL leave Files, Finder, Shortcuts, and share-extension access independent. Settings SHALL explain this boundary.

#### Scenario: App is locked
- **WHEN** a configured integration accesses the archive
- **THEN** it continues using its existing shared sign-in independently of App lock
