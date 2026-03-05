# Equirust File Reference

## Top-Level Project Files

| File | Responsibility | Functionality | Typical Use Case |
| --- | --- | --- | --- |
| `README.md` | User-facing project docs | Install/build/use/update guidance | Onboarding users and release notes context |
| `.env.example` | Optional build-time env overrides | Documents supported environment variables | Local/dev override reference |
| `runtime-package.json` | Linked runtime metadata | Stores expected Equicord runtime version metadata | Runtime version display/check fallback |
| `src-tauri/Cargo.toml` | Rust package manifest | Defines app version, dependencies, features | Build and dependency management |
| `src-tauri/tauri.conf.json` | Tauri app/bundle config | Product/version/identifier, bundle targets/icons | Packaging/install behavior |

## Rust Host (`src-tauri/src`)

| File | Responsibility | Functionality | Typical Use Case |
| --- | --- | --- | --- |
| `main.rs` | Native process entrypoint | Uses windowed subsystem on Windows and calls `equirust_lib::run()` | Start app without console window |
| `lib.rs` | App composition root | Registers modules, plugins, panic/log hooks, and all Tauri commands | Boot and wire entire desktop host |
| `window.rs` | Main webview/window lifecycle | Creates/configures main window and navigation hooks | Launch Discord webview with host controls |
| `desktop.rs` | Host-to-page bridge surface | Injects bootstrap runtime script, titlebar logic, screenshare bridge, and desktop commands | Runtime behavior inside hosted Discord page |
| `discord.rs` | Discord URL/routing policy | Resolves branch URL, route parsing, navigation allowlist, user-agent/args | Safe navigation and external-link routing |
| `arrpc.rs` | Rich Presence subsystem | Native process detection + Discord IPC bridge + activity/status commands | Show game/activity state in Discord profile |
| `updater.rs` | Host + runtime update manager | Checks versions, exposes status, download/install/snooze/ignore actions | Inline update flow in settings |
| `vencord.rs` | Equicord runtime + assets host | Runtime resolver/injection + QuickCSS/themes/settings/file-state commands with sensitive plugin-setting values protected at rest | Load/update managed runtime and mod assets |
| `capturer.rs` | Source enumeration API | Returns capturer sources and thumbnails | Feed native screen-share picker UI |
| `tray.rs` | System tray integration | Tray lifecycle, show/hide, voice state variants, unread badge sync | Background behavior and quick restore |
| `notifications.rs` | Attention/badge signals | Badge count + taskbar frame flash commands | Mention notifications and user attention |
| `autostart.rs` | Login-item/autostart control | Read/set autostart and sync with settings | Start Equirust with Windows login |
| `app_menu.rs` | Native application menu | Builds/syncs menu and handles click events | Native menu behavior tied to settings |
| `protocol.rs` | Deep-link protocol handling | Registers and dispatches protocol callbacks | Open Discord routes from protocol links |
| `ipc_bridge.rs` | Renderer command bridge | Event-based request/response channel with timeouts | Rust-hosted replacement for preload IPC |
| `csp.rs` | CSP override manager | Add/remove/check CSP overrides and apply response rewrites | Vencord/Equicord CSP compatibility |
| `http_proxy.rs` | Controlled HTTP proxy surface | Validated proxied requests/response payload conversion | Cloud OAuth/settings requests from page runtime |
| `file_manager.rs` | User assets/runtime path manager | Pick/open assets, manage custom runtime directory, resolve user asset paths | Theme/splash/tray asset management |
| `store.rs` | Persistent store wrapper | Load/snapshot/update settings and state via commands | Durable host settings and runtime state |
| `settings.rs` | Data model schema | Defines `Settings`, `PersistedState`, defaults, normalization/fallbacks | Version-safe settings migrations |
| `paths.rs` | App path resolver | Resolves logs/cache/assets/runtime directories | Shared path lookups across modules |
| `privacy.rs` | Log/data redaction | Sanitizes text/URLs/path-like values for logs | Prevent sensitive data leakage in diagnostics |
| `spellcheck.rs` | Spellcheck backend | Spellcheck command surface and suggestion responses | Context menu spelling fixes in page bridge |
| `utilities.rs` | Misc host utilities | Clipboard image copy, system theme values, debug-page opener | Small cross-cutting utility commands |
| `doctor.rs` | Diagnostics report | Generates health/diagnostic snapshot via command | Debug support and troubleshooting |
| `voice.rs` | Second-instance voice toggles | Parses launch args and emits mute/deafen events | Global/secondary launch voice controls |
| `virtmic.rs` | Virtual microphone API boundary | `virtmic_*` commands with platform-specific support behavior | Future Linux/advanced audio routing path |

## Native Screen Share (`src-tauri/src/native_capture`)

| File | Responsibility | Functionality | Typical Use Case |
| --- | --- | --- | --- |
| `mod.rs` | Module export root | Re-exports native-capture submodules | Compile-time module wiring |
| `types.rs` | Contract types | Request/response/session/event structs for capture commands | Stable host-page data contracts |
| `source.rs` | Source capture + frame prep | Resolve window/screen IDs, capture frames, liveness checks, letterboxing | Pull selected capture source frames |
| `audio.rs` | Loopback audio capture | WASAPI loopback capture with source-aware targeting | Include/exclude system audio in streams |
| `encoder.rs` | Video encoding strategy | Chooses hardware/software/JPEG path and encodes RGBA frames | Efficient stream payload generation |
| `session.rs` | Session orchestrator | Start/stop/session-state commands, websocket transport, lifecycle/error handling | End-to-end native stream session runtime |
| `transport.rs` | Transport constants | Packet kinds + queue/backpressure limits | Shared transport tuning definitions |
| `video.rs` | Video config placeholder | Basic video capture config struct | Reserved extension point for video pipeline growth |

## Tauri Frontend Dist Placeholder (`src-tauri/dist`)

| File | Responsibility | Functionality | Typical Use Case |
| --- | --- | --- | --- |
| `index.html` | Build-time frontend dist placeholder | Minimal static entry used to satisfy Tauri packaging requirements | Runtime launches directly to Discord external URL |
