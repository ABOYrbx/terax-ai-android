## v0.7.5 — Android keyboard input fix

This release fixes a critical bug where terminal input did not appear on Android. When typing in the terminal, nothing showed up because soft keyboard events were being silently dropped.

### Fixed

- **Terminal input not showing on Android**: The Android keyboard fallback in the xterm.js custom key handler was placed after the IME composition guard, which blocked events with `keyCode === 229` (used by many Android WebViews for soft keyboard input). The `androidPrintableKeySequence()` function only handled `keyCode === 0`, so all printable input was silently discarded.
  - The Android keyboard handler now runs before the IME guard, and accepts both `keyCode === 0` and `keyCode === 229`.
  - On Android, `isComposing` is reliably set during actual IME composition, so the guard correctly distinguishes IME input from regular typing.

### Infrastructure

- Restored Android CI workflow that builds APK and creates GitHub releases on version tags.
- Fixed hardcoded macOS developer paths in `tauri.conf.json`, `gradle.properties`, and `BuildTask.kt` that prevented CI builds from completing.
- CI now runs only on tag pushes (no duplicate runs on branch pushes).
