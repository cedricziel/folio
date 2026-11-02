## State and authentication

One main-actor observable lock model serves every app window. Persist only enabled state and timeout. An enabled lock starts locked on every process launch. Inject authentication and a monotonic clock for deterministic tests. Authenticate setting changes as well as unlocking. Serialize requests and reject stale completions after device lock or backgrounding.

Use LocalAuthentication's deviceOwnerAuthentication policy with a fresh LAContext for each request. The OS selects biometrics or credential fallback. Failure and cancellation never unlock or change preferences. Devices without usable device-owner authentication cannot enable the setting.

## Presentation and lifecycle

Remove document UI while locked, including its navigation and presented sheets. Place native privacy covers above window content while inactive so sheets and previews cannot appear in app-switcher snapshots. All windows share the lock. Use application lifecycle notifications rather than individual scene focus for the absence interval. Authentication-induced inactivity still covers content but does not start another lock cycle.

Observe iOS protected-data loss and macOS sleep, session resignation, and screen-lock notifications. Lock immediately on these events and invalidate pending authentication. On foreground return, evaluate elapsed absence before removing the privacy cover. Keep integrations' shared credentials unchanged.

## Interface

Expose App lock settings and Lock now from the workspace toolbar, plus a Lock now keyboard command. Use existing design tokens. Settings explain the Files, Finder, and Shortcuts boundary and the authentication fallback. The lock screen offers an explicit Unlock action, allowing cancellation without repeatedly prompting.
