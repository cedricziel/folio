## Why

Folio opens personal documents without a local authentication step. Users need an optional privacy lock when lending or leaving an unlocked device.

## What changes

- Add a device-local App lock setting, disabled by default, to the Apple app.
- Authenticate with the system's device-owner policy, including biometrics and passcode or password fallback.
- Lock on launch, device lock, explicit Lock now, and return after the selected absence interval: immediately, one minute by default, or five minutes.
- Cover all app windows and presentations while inactive.
- Keep Files, Finder, Shortcuts, and sharing independent, and explain that boundary in settings.

## Impact

Only the Apple app changes. No folio-core, folio-infra, or folio-api changes, HTTP endpoints, schema migrations, or generated client changes. Settings live in app-local UserDefaults, not config.toml, the settings table, or the shared App Group. This is a UI privacy lock, not archive encryption.
