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
- Native Windows screen-share picker/session transport (WORK IN PROGRESS)
- Managed Equicord runtime download, cache, version check, and update flow.

## Platform Scope

- Primary target: **Windows 10/11**
- Runtime requirement: **Microsoft Edge WebView2** (It very likely came [with your ~~Xbox~~](https://www.youtube.com/watch?v=04X5x4LDEDc&t=71s) Windows installation.)
- Future macOS/Linux support is undetermined at this time.

## Install

Download from [GitHub Releases](https://github.com/thehollowedone/equirust/releases):

- `.msi` / setup `.exe` for standard install
- `.zip` for portable usage

## Logs and Crash Logs

Log directory (Windows):

- `%LOCALAPPDATA%\equirust\logs`

Files:

- `Equirust.log`: normal runtime log output
- `Equirust-crash.log`: panic/crash entries (always recorded)
- `Equirust-debug.log`: extra verbose bridge/runtime diagnostics (debug builds and `--profiling-diagnostics` runs)

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

Profiling build:

```sh
cargo build -p equirust --profile profiling
```

Use this when recording traces in Windows Performance Recorder, Visual Studio,
or another native profiler. The `profiling` profile inherits `release`, keeps
optimizations enabled, preserves symbols, and does not turn `debug_assertions`
back on. It is not meant for shipping.

Artifact:

- Binary: `target/profiling/equirust.exe`

Optional profiling diagnostics:

```sh
target/profiling/equirust.exe --profiling-diagnostics
```

That flag enables extra runtime/media diagnostics and more verbose logging for
investigation, but it does add overhead.

Release build:

```sh
cargo tauri build
```

Artifacts:

- Binary: `target/release/equirust.exe`
- Bundles/installers: `target/release/bundle/`


## Documentation

- File ownership/reference doc: [docs/file-reference.md](docs/file-reference.md)

