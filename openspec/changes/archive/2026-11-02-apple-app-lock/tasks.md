## 1. Lock behavior

- [x] 1.1 Write failing AppLockTests for persisted launch locking, authentication failure, timeout boundaries, privacy state, prompt inactivity, and stale completion rejection; implement the lock model.

## 2. Apple integration

- [x] 2.1 Add system authentication, lifecycle observers, native privacy covers, settings, and Lock now. Build macOS and iOS targets and run the authentication tests.
- [x] 2.2 Document the lock boundary and update the project verify skill with isolated lock tests and device checks.

## 3. Validation

- [x] 3.1 Run Swift formatting, package and app tests, and OpenSpec validation.
- [x] 3.2 Run make format, make lint, and make test; record environmental blockers.

## Verification results

- All 37 macOS authentication and File Provider tests pass, including 17 App lock tests.
- All 148 FolioKit tests pass.
- Unsigned macOS and iOS Simulator builds pass. The built iOS app includes the Face ID usage description.
- Swift formatting, strict OpenSpec validation, make format, make lint, and make test pass.
- Initial parallel builds exhausted disk space. Sequential Apple builds and Rust builds with debug artifacts disabled completed successfully.
- Real-device biometric prompts and visual snapshot checks remain manual, as documented in the project verify skill.
