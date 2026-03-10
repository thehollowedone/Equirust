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
| `window.rs` | Main webview/window lifecycle | Creates/configures main window shell, state persistence, and window placement | Launch Discord webview with host controls |
| `desktop/mod.rs` | Host-to-page bridge entry | Serializes bootstrap inputs and renders the injected page-runtime template | Runtime behavior inside hosted Discord page |
| `desktop/template.rs` | Bootstrap template renderer | Concatenates ordered bootstrap sections, substitutes seed/runtime tokens, and validates output | Keep injected runtime script maintainable without a giant Rust format string |
| `desktop_host.rs` | Desktop page command surface | Window controls, runtime logging bridge, and relaunch helpers exposed to the hosted page | Lightweight host commands consumed by injected desktop runtime |
| `discord.rs` | Discord URL/routing policy | Resolves branch URL, route parsing, navigation allowlist, user-agent/args | Safe navigation and external-link routing |
| `browser_runtime.rs` | Browser runtime policy/config | Runtime kind, browser args, user-agent policy, and runtime-mode flags | Keep runtime-specific decisions out of window lifecycle code |
| `arrpc.rs` | Rich Presence subsystem | Native process detection + Discord IPC bridge + activity/status commands | Show game/activity state in Discord profile |
| `updater.rs` | Host + runtime update manager | Checks versions, exposes status, download/install/snooze/ignore actions | Inline update flow in settings |
| `mod_runtime.rs` + `mod_runtime/` | Equicord mod-runtime host | Thin module root plus purpose-split runtime resolver, protocol, legacy import, settings protection, and theme/file commands | Load/update managed runtime and mod assets |
| `capturer.rs` | Source enumeration API | Returns capturer sources and thumbnails | Feed native screen-share picker UI |
| `tray.rs` | System tray integration | Tray lifecycle, show/hide, voice state variants, unread badge sync | Background behavior and quick restore |
| `notifications.rs` | Attention/badge signals | Badge count + taskbar frame flash commands | Mention notifications and user attention |
| `autostart.rs` | Login-item/autostart control | Read/set autostart and sync with settings | Start Equirust with Windows login |
| `app_menu.rs` | Native application menu | Builds/syncs menu and handles click events | Native menu behavior tied to settings |
| `protocol.rs` | Deep-link protocol handling | Registers and dispatches protocol callbacks | Open Discord routes from protocol links |
| `ipc_bridge.rs` | Renderer command bridge | Event-based request/response channel with timeouts | Rust-hosted replacement for preload IPC |
| `csp.rs` | CSP override manager | Add/remove/check CSP overrides and apply response rewrites | Vencord/Equicord CSP compatibility |
| `http_proxy.rs` | Host-side cloud HTTP bridge | Proxied HTTP command used for cloud integration fallback/testing | Diagnosing cloud OAuth/settings fetch failures |
| `file_manager.rs` | User assets/runtime path manager | Pick/open assets, manage custom runtime directory, resolve user asset paths | Theme/splash/tray asset management |
| `store.rs` | Persistent store wrapper | Load/snapshot/update settings and state via commands | Durable host settings and runtime state |
| `settings.rs` | Data model schema | Defines `Settings`, `PersistedState`, defaults, normalization/fallbacks | Version-safe settings migrations |
| `paths.rs` | App path resolver | Resolves logs/cache/assets/runtime directories | Shared path lookups across modules |
| `privacy.rs` | Log/data redaction | Sanitizes text/URLs/path-like values for logs | Prevent sensitive data leakage in diagnostics |
| `processes.rs` | Managed child-process ownership | Assigns long-lived helper processes to a Windows kill-on-close Job Object | Keep helper processes tied to the Equirust process lifecycle |
| `spellcheck.rs` | Spellcheck backend | Spellcheck command surface and suggestion responses | Context menu spelling fixes in page bridge |
| `utilities.rs` | Misc host utilities | Clipboard image copy, system theme values, debug-page opener | Small cross-cutting utility commands |
| `doctor.rs` | Diagnostics report | Generates health/diagnostic snapshot via command | Debug support and troubleshooting |
| `voice.rs` | Second-instance voice toggles | Parses launch args and emits mute/deafen events | Global/secondary launch voice controls |
| `virtmic.rs` | Virtual microphone API boundary | `virtmic_*` commands with platform-specific support behavior | Future Linux/advanced audio routing path |
| `win32_window_snapshot.rs` | Win32 window geometry + snapshot helper | Reads physical window bounds and captures immediate GDI/PrintWindow startup snapshots | Picker sizing correctness and instant first-frame window-share startup |

## Desktop Stream (`src-tauri/src/desktop_stream`)

| File | Responsibility | Functionality | Typical Use Case |
| --- | --- | --- | --- |
| `mod.rs` | Module export root | Re-exports desktop-stream submodules | Compile-time module wiring |
| `contracts.rs` | Command/data contracts | Request/response/session/event structs for stream commands | Stable host-page data contracts |
| `sink_contract.rs` | Browser/runtime sink contract | Declares the browser-facing desktop-stream sink, transport, and ingress descriptors | Keep WebView2 today and Tartarust tomorrow on an explicit media boundary |
| `capture_sources.rs` | Source resolution + frame prep | Resolves source IDs, manages screen/window capture sessions, resizes/letterboxes/prepares frames | Pull selected display/window frames into the stream pipeline |
| `system_audio.rs` | Loopback audio capture | WASAPI loopback capture with source-aware targeting | Include app/system audio in streams |
| `video_encoder.rs` | Video encoding pipeline | Chooses hardware/software/JPEG path, drives Media Foundation/OpenH264, and handles Windows GPU color conversion | Efficient stream payload generation |
| `stream_session.rs` | Session orchestrator | Start/stop/session-state commands, websocket transport, lifecycle/error handling, keyframe control | End-to-end desktop stream runtime |
| `transport.rs` | Transport constants | Packet kinds + queue/backpressure limits | Shared transport tuning definitions |
| `video_config.rs` | Video config placeholder | Basic video capture config struct | Reserved extension point for stream-pipeline growth |
| `d3d11_device.rs` | Shared D3D11 device helpers | Creates and resolves shared Windows capture/encode devices and outputs | Keep DXGI/WGC/encoder on compatible adapters |
| `dxgi_duplication.rs` | Screen capture backend | Desktop Duplication capture with BGRA/HDR texture paths | High-performance display capture on Windows |
| `wgc_window_capture.rs` | Window capture backend | Windows Graphics Capture session management with texture/readback paths and LUT tonemap fallback | High-performance window capture on Windows |

## Tauri Frontend Dist Placeholder (`src-tauri/dist`)

| File | Responsibility | Functionality | Typical Use Case |
| --- | --- | --- | --- |
| `index.html` | Build-time frontend dist placeholder | Minimal static entry used to satisfy Tauri packaging requirements | Runtime launches directly to Discord external URL |


## Injected Runtime Bootstrap (`src-tauri/resources/runtime/bootstrap`)

| File | Responsibility | Functionality | Typical Use Case |
| --- | --- | --- | --- |
| `prelude.js` | Bootstrap prelude + shared runtime state | Storage polyfill, diagnostics, host bridge setup, utility helpers, early runtime state | Shared foundation for all later injected runtime behavior |
| `media.js` | Media + desktop stream runtime | WebRTC constraints, GoLive patches, desktop stream transport, ABR logic, and screen-share picker | Discord media compatibility and desktop streaming flow |
| `integrations.js` | Host/runtime integrations | Cloud proxying, notification sync, tray/voice hooks, arRPC bridge, titlebar/runtime glue | Non-settings page integrations layered onto Discord |
| `settings.js` | Desktop settings/runtime entry | Desktop settings UI, host update/file-manager/spellcheck wiring, Vencord settings hooks, bootstrap ready/retry tail | Host settings surface and final bootstrap execution |

