# Equirust

[![forthebadge](https://forthebadge.com/images/badges/made-with-rust.svg)](https://forthebadge.com)
[![forthebadge](https://forthebadge.com/images/badges/open-source.svg)](https://forthebadge.com)
[![forthebadge](https://forthebadge.com/images/badges/fuck-it-ship-it.svg)](https://forthebadge.com)

Rust-first desktop host for Discord + Equicord runtime.

## Project Goal

Equirust replaces the old Electron/Bun desktop host path with Rust/Tauri while preserving Discord usability and Equicord plugin/runtime behavior.

Current Rust-owned path includes:

- Windowing, tray, startup, protocol handling, updater, and persisted state.
- Native Rich Presence bridge / Discord IPC behavior.
- Native Windows screen-share picker/session transport.
- Managed Equicord runtime download, cache, version check, and update flow.

## Platform Scope

- Primary target: **Windows 10/11**
- Runtime requirement: **Microsoft Edge WebView2** (It very likely came [with your ~~Xbox](https://www.youtube.com/watch?v=04X5x4LDEDc&t=71s)~~ Windows installation.)
- Future macOS/Linux support is undetermined at this time.

## Install

Download from [GitHub Releases](https://github.com/thehollowedone/equirust/releases):

- `.msi` / setup `.exe` for standard install
- `.zip` for portable usage

## Runtime Path (Windows)

- Managed Equicord runtime cache:
`%LOCALAPPDATA%\equirust\equicord-runtime\current`

Installers do **not** pre-bundle the runtime payload. Equirust downloads and caches it on first run, then reuses/updates it when a newer runtime release is available.

## Logs and Crash Logs

Log directory (Windows):

- `%LOCALAPPDATA%\equirust\logs`

Files:

- `Equirust.log`: normal runtime log output
- `Equirust-crash.log`: panic/crash entries (always recorded)
- `Equirust-debug.log`: extra verbose bridge/runtime diagnostics (debug builds)

## Build

Prerequisites:

- Rust toolchain
- Tauri CLI: `cargo install tauri-cli --version "^2"`
- WebView2 runtime on Windows

Debug build, portable:

```sh
cargo tauri build --debug --no-bundle
```

Debug build:

```sh
cargo tauri dev
```

`cargo check --workspace` performs compile/type checks only. It does not produce runnable binaries.

Release build:

```sh
cargo tauri build
```

Artifacts:

- Binary: `target/release/equirust.exe`
- Bundles/installers: `target/release/bundle/`

## Documentation

- File ownership/reference doc: [docs/file-reference.md](docs/file-reference.md)

