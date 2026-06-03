<div align="center">
 <img src="public/logo.png" width="144" height="144" alt="Terax" />
 <h1>Terax for Android</h1>

 <p><strong>Bringing Terax, the terminal-first AI-native dev workspace, to Android.</strong></p>

 <p>
 <img src="https://img.shields.io/badge/license-Apache--2.0-green" alt="license" />
 <img src="https://img.shields.io/badge/platform-Android-lightgrey" alt="platform" />
 <img src="https://img.shields.io/badge/status-active%20development-yellow" alt="status" />
 </p>

 <p>
 <a href="https://terax.app">Terax (desktop)</a>
 ·
 <a href="https://github.com/crynta/terax-ai">Desktop source code</a>
 </p>
</div>

---

Terax is a lightweight open-source terminal (ADE) built on Tauri 2 + Rust and React 19. A native PTY backend with a WebGL renderer, an agentic AI side-panel that runs against your own keys or fully local models, plus a code editor, file explorer, source control with a git graph, and a web preview pane built in. About 7-8 MB on disk. No telemetry. No account.

This repository ports Terax to **Android** using Tauri 2's Android target. The frontend (React 19 + xterm.js + CodeMirror 6) and Rust backend (PTY layer, git, filesystem, AI tool surface) are shared with the desktop builds. The Android-specific glue -- system WebView integration, JNI bridge, APK packaging -- lives in `android/`.

## Features

The Android port aims to bring the same core Terax experience to mobile:

- **Multi-tab terminal** with xterm.js WebGL rendering and native PTY backend
- **Code editor** powered by CodeMirror 6 (TypeScript, Rust, Python, Go, HTML/CSS, JSON, Markdown and more)
- **AI side-panel** with multiple cloud providers (OpenAI, Anthropic, Google, Groq, xAI, Cerebras, OpenRouter-compatible) and local model support (LM Studio, MLX, Ollama)
- **File explorer** with fuzzy search, keyboard navigation, inline rename
- **Source control** panel with stage/commit and git history graph
- **Web preview** for local dev servers
- **Custom themes**: UI presets, image backgrounds, independent editor themes
- **No telemetry**, no account required. API keys stored in the OS keychain.
- **Lightweight**: shared Rust + JS codebase keeps the binary footprint small

## What works now

This is early-stage active development, focused on the Android runtime plumbing:

- [x] Tauri 2 Android target setup (Android WebView + JNI)
- [x] APK build configuration (minSdk 24, targetSdk 36, compileSdk 36)
- [x] Shared Rust backend builds for Android (aarch64, armeabi-v7a, x86, x86_64)
- [x] Frontend builds and is packaged as an asset bundle
- [x] Terminal input works via soft keyboard (keyCode 0 and 229 fallback paths)
- [x] ProGuard / R8 release shrinker configured
- [x] Versioning synced with desktop (`android/app/tauri.properties`)
- [ ] Functional tests on real Android devices(still adding features to make it work)

## Screenshots

Screenshots from the desktop version (the Android UI follows the same design language):

<table>
 <tr>
 <td align="center"><img src="docs/terminal.png" alt="Terminal" /><br/><sub>Multi-tab terminal</sub></td>
 <td align="center"><img src="docs/themes.png" alt="Themes and background image" /><br/><sub>Custom themes</sub></td>
 </tr>
 <tr>
 <td align="center"><img src="docs/web-preview.png" alt="Web preview" /><br/><sub>Web preview of local dev servers</sub></td>
 <td align="center"><img src="docs/source-control.png" alt="Source control and git graph" /><br/><sub>Source control panel</sub></td>
 </tr>
 <tr>
 <td colspan="2" align="center"><img src="docs/ai-workflow.png" alt="AI window" /><br/><sub>Agentic AI workflow</sub></td>
 </tr>
</table>

## Building

**Prerequisites**
- Rust (stable), https://rustup.rs
- Node 22+ and [pnpm](https://pnpm.io)
- Android SDK (compileSdk 36, targetSdk 36, minSdk 24)
- Tauri prerequisites for Android: https://tauri.app/start/prerequisites/#android

**Run on device / emulator**
```bash
pnpm install
pnpm tauri android dev
```

**Build release APK**
```bash
pnpm tauri android build
```

**Frontend checks**
```bash
pnpm exec tsc --noEmit  # type-check
pnpm test               # tests
```

**Rust checks**
```bash
cd src-tauri && cargo clippy --all-targets --locked -D warnings
cd src-tauri && cargo test --locked
```

## Tech stack

Tauri 2, Rust, `portable-pty`, React 19, TypeScript, Vite, xterm.js, CodeMirror 6, Vercel AI SDK v6, Tailwind v4, Android WebView.

## Repository layout

```
android/            -- Android project (app module, Gradle, ProGuard)
src/                -- React frontend (shared with desktop)
src-tauri/          -- Rust backend (shared with desktop)
docs/               -- Screenshots from the desktop build
```
