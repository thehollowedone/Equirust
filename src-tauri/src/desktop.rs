use crate::{privacy, store::PersistedStore};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, State as TauriState, Window};
use tauri_runtime::ResizeDirection;

#[tauri::command]
pub fn log_client_runtime(
    message: String,
    app: tauri::AppHandle,
    store: TauriState<'_, PersistedStore>,
) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Ok(());
    }

    let snapshot = store.snapshot();
    if snapshot.settings.runtime_diagnostics != Some(true)
        && snapshot.settings.ar_rpc_debug != Some(true)
    {
        return Ok(());
    }

    let sanitized = privacy::sanitize_text_for_log(&message);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    if let Ok(log_dir) = app.path().app_log_dir() {
        if fs::create_dir_all(&log_dir).is_ok() {
            let debug_log_path = log_dir.join("Equirust-debug.log");
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(debug_log_path)
            {
                let _ = writeln!(file, "[{timestamp}] {}", sanitized);
            }
        }
    }

    if cfg!(debug_assertions) {
        log::info!("Client runtime: {}", sanitized);
    }
    Ok(())
}

#[tauri::command]
pub fn app_relaunch(app: tauri::AppHandle) -> Result<(), String> {
    app.restart();
}

#[tauri::command]
pub fn window_focus(window: Window) -> Result<(), String> {
    window.set_focus().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_close(window: Window) -> Result<(), String> {
    window.close().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_minimize(window: Window) -> Result<(), String> {
    window.minimize().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_toggle_maximize(window: Window) -> Result<(), String> {
    if window.is_maximized().map_err(|err| err.to_string())? {
        window.unmaximize().map_err(|err| err.to_string())
    } else {
        window.maximize().map_err(|err| err.to_string())
    }
}

#[tauri::command]
pub fn window_is_maximized(window: Window) -> Result<bool, String> {
    window.is_maximized().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_start_dragging(window: Window) -> Result<(), String> {
    window.start_dragging().map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_set_title(title: String, window: Window) -> Result<(), String> {
    window.set_title(&title).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn window_start_resize_dragging(direction: String, window: Window) -> Result<(), String> {
    let direction = parse_resize_direction(&direction)
        .ok_or_else(|| format!("unsupported resize direction: {direction}"))?;

    window
        .start_resize_dragging(direction)
        .map_err(|err| err.to_string())
}

fn parse_resize_direction(direction: &str) -> Option<ResizeDirection> {
    match direction {
        "East" => Some(ResizeDirection::East),
        "North" => Some(ResizeDirection::North),
        "NorthEast" => Some(ResizeDirection::NorthEast),
        "NorthWest" => Some(ResizeDirection::NorthWest),
        "South" => Some(ResizeDirection::South),
        "SouthEast" => Some(ResizeDirection::SouthEast),
        "SouthWest" => Some(ResizeDirection::SouthWest),
        "West" => Some(ResizeDirection::West),
        _ => None,
    }
}

pub fn bootstrap_script(
    seed: &Value,
    vencord_renderer: Option<&str>,
    control_runtime: bool,
    install_host_runtime: bool,
    install_mod_runtime: bool,
    spoof_edge_client_hints: bool,
) -> Result<String, String> {
    let seed_json = serde_json::to_string(seed).map_err(|err| err.to_string())?;
    let vencord_renderer_json =
        serde_json::to_string(&vencord_renderer.unwrap_or("")).map_err(|err| err.to_string())?;
    let control_runtime_json =
        serde_json::to_string(&control_runtime).map_err(|err| err.to_string())?;
    let install_host_runtime_json =
        serde_json::to_string(&install_host_runtime).map_err(|err| err.to_string())?;
    let install_mod_runtime_json =
        serde_json::to_string(&install_mod_runtime).map_err(|err| err.to_string())?;
    let spoof_edge_client_hints_json =
        serde_json::to_string(&spoof_edge_client_hints).map_err(|err| err.to_string())?;

    Ok(format!(
        r###"
(() => {{
  if (window.__EQUIRUST_BOOTSTRAPPED__) return;
  window.__EQUIRUST_BOOTSTRAPPED__ = true;

  const seed = {seed_json};
  const vencordRenderer = {vencord_renderer_json};
  const controlRuntime = {control_runtime_json};
  const installHostRuntime = {install_host_runtime_json};
  const installModRuntime = {install_mod_runtime_json};
  const spoofEdgeClientHints = {spoof_edge_client_hints_json};
  const internals = window.__TAURI_INTERNALS__;
  if (!internals || typeof internals.invoke !== "function") {{
    console.warn("[Equirust] Tauri invoke bridge is unavailable.");
    return;
  }}

  const invoke = (cmd, args = {{}}) => internals.invoke(cmd, args);
  const state = {{
    settings: seed.settings || {{}},
    hostSettings: seed.hostSettings || {{}},
    hostUpdateStatus: null,
    runtimeUpdateStatus: null,
    hostUpdateDownloadState: null,
    fileManagerState: null,
    debugBuild: seed.debugBuild === true,
    nativeAutoStartEnabled: seed.nativeAutoStartEnabled === true,
    quickCss: typeof seed.quickCss === "string" ? seed.quickCss : "",
    versions: seed.versions || {{}},
    vencordRenderer,
    quickCssListeners: new Set(),
    themeListeners: new Set(),
    rendererCssListeners: new Set(),
    vencordFileWatchTimer: null,
    vencordFileWatchVisibilityBound: false,
    rendererCssPollTimer: null,
    rendererCssVisibilityBound: false,
    quickCssRevision: null,
    themesRevision: null,
    rendererCssValue: null,
    nativeWindowTitle: null,
    titlebarReady: false,
    vencordSettingsObserver: null,
    vencordSettingsReady: false,
    mediaCompatReady: false,
    displayMediaCompatReady: false,
    screenSharePickerBusy: false,
    screenShareThumbnailCache: Object.create(null),
    screenSharePreviewCache: Object.create(null),
    notificationSyncReady: false,
    notificationObserver: null,
    notificationSyncQueued: false,
    lastBadgeCount: null,
    flashActive: false,
    voiceTrayReady: false,
    voiceTrayTimer: null,
    voiceTrayInCall: false,
    voiceTrayVariant: null,
    voiceToggleMuteListeners: new Set(),
    voiceToggleDeafListeners: new Set(),
    voiceToggleQueue: [],
    voiceToggleRetryTimer: null,
    voiceBridgeCleanup: null,
    voiceBridgeReady: false,
    arrpcActivityListeners: new Set(),
    arrpcBridgeCleanup: null,
    arrpcBridgeReady: false,
    arrpcPendingPayloads: [],
    arrpcStatus: null,
    arrpcLastSocketId: null,
    arrpcLastRunningGame: null,
    arrpcNullClearTimer: null,
    arrpcLastNonNullAtMs: 0,
    spellcheckResultListeners: new Set(),
    spellcheckContextMenuInstalled: false,
    spellcheckSelection: null,
    spellcheckLearnedWords: new Set(
      Array.isArray(seed.hostSettings?.spellCheckDictionary)
        ? seed.hostSettings.spellCheckDictionary
            .filter(value => typeof value === "string")
            .map(value => value.trim().toLocaleLowerCase())
            .filter(Boolean)
        : []
    ),
    commandListeners: new Set(),
    commandBridgeCleanup: null,
    commandBridgeReady: false,
    cloudFetchProxyInstalled: false,
    typingObserver: null,
    typingPollScheduled: false,
  }};
  const shouldEmitRuntimeDiagnostics = () =>
    state.debugBuild === true && state.hostSettings?.runtimeDiagnostics === true;
  const report = (message, options = {{}}) => {{
    if (state.debugBuild !== true) {{
      return Promise.resolve();
    }}

    if (options.force !== true && !shouldEmitRuntimeDiagnostics()) {{
      return Promise.resolve();
    }}

    return invoke("log_client_runtime", {{ message }}).catch(() => {{}});
  }};

  window.__EQUIRUST_BRIDGE__ = {{ invoke, state }};

  const isDiscordHost = () =>
    /(^|\.)discord\.com$/i.test(window.location.hostname) ||
    /(^|\.)discordapp\.com$/i.test(window.location.hostname);

  const getConfiguredCloudOrigin = () => {{
    const cloudUrl = state.settings?.cloud?.url;
    if (typeof cloudUrl !== "string" || !cloudUrl.trim()) {{
      return null;
    }}

    try {{
      return new URL(cloudUrl, window.location.href).origin;
    }} catch {{
      return null;
    }}
  }};

  const shouldUseCustomTitleBar = () =>
    installHostRuntime &&
    !controlRuntime &&
    String(state.versions.platform || "").toLowerCase() === "windows" &&
    state.hostSettings?.customTitleBar !== false;

  const shouldUseActivePolling = () =>
    !document.hidden &&
    document.visibilityState !== "hidden" &&
    (typeof document.hasFocus !== "function" || document.hasFocus());

  const getAdaptivePollDelay = (foregroundMs, backgroundMs) =>
    shouldUseActivePolling() ? foregroundMs : backgroundMs;

  const supportsNativeAutoStart = () =>
    String(state.versions.platform || "").toLowerCase() === "windows";

  const supportsWindowsTransparency = () =>
    String(state.versions.platform || "").toLowerCase() === "windows";

  const supportsNativeWindowsScreenShare = () =>
    String(state.versions.platform || "").toLowerCase() === "windows";

  const normalizeLegacyPlatform = platform => {{
    const raw = String(platform || "").toLowerCase();
    if (raw.includes("windows")) return "win32";
    if (raw.includes("mac")) return "darwin";
    if (raw.includes("linux")) return "linux";
    return raw || "unknown";
  }};

  const safeCall = async fn => {{
    try {{
      return await fn();
    }} catch (error) {{
      console.error("[Equirust]", error);
      throw error;
    }}
  }};

  const serializeIpcError = error => {{
    if (error instanceof Error) {{
      return {{
        name: error.name,
        message: error.message,
        stack: error.stack,
      }};
    }}

    return {{
      name: "Error",
      message: typeof error === "string" ? error : String(error),
    }};
  }};

  const wrapIpcResult = async fn => {{
    try {{
      return {{
        ok: true,
        value: await fn(),
      }};
    }} catch (error) {{
      console.error("[Equirust]", error);
      return {{
        ok: false,
        error: serializeIpcError(error),
      }};
    }}
  }};

  const encodeBytesToBase64 = bytes => {{
    let binary = "";
    const chunkSize = 0x8000;
    for (let index = 0; index < bytes.length; index += chunkSize) {{
      const slice = bytes.subarray(index, index + chunkSize);
      binary += String.fromCharCode(...slice);
    }}
    return window.btoa(binary);
  }};

  const decodeBase64ToBytes = value => {{
    if (typeof value !== "string" || !value.length) {{
      return new Uint8Array();
    }}

    const binary = window.atob(value);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {{
      bytes[index] = binary.charCodeAt(index);
    }}
    return bytes;
  }};

  const currentWebviewWindowTarget = () => {{
    const label = String(internals.metadata?.currentWindow?.label || "main");
    return {{
      kind: "WebviewWindow",
      label,
    }};
  }};

  const listenTauriEvent = async (event, handler, target = currentWebviewWindowTarget()) => {{
    if (typeof internals.transformCallback !== "function") {{
      throw new Error("Tauri event callbacks are unavailable.");
    }}

    const callback = internals.transformCallback(payload => {{
      try {{
        handler(payload);
      }} catch (error) {{
        console.error("[Equirust]", error);
      }}
    }});
    const eventId = await invoke("plugin:event|listen", {{
      event,
      target,
      handler: callback,
    }});

    return async () => {{
      try {{
        window.__TAURI_EVENT_PLUGIN_INTERNALS__?.unregisterListener?.(event, eventId);
      }} catch (error) {{
        console.warn("[Equirust] Failed to unregister event listener", error);
      }}

      await invoke("plugin:event|unlisten", {{
        event,
        eventId,
      }}).catch(() => {{}});
    }};
  }};

  const applyEdgeClientHintsSpoof = () => {{
    if (!spoofEdgeClientHints) return;

    const version = String(state.versions.webview || "").trim();
    const majorVersion = version.split(".")[0] || "145";
    const platform = (() => {{
      const raw = String(state.versions.platform || "windows").toLowerCase();
      if (raw.includes("mac")) return "macOS";
      if (raw.includes("linux")) return "Linux";
      return "Windows";
    }})();
    const fullVersionList = Object.freeze([
      Object.freeze({{ brand: "Not:A-Brand", version: "99" }}),
      Object.freeze({{ brand: "Chromium", version: majorVersion }}),
      Object.freeze({{ brand: "Microsoft Edge", version: majorVersion }}),
    ]);
    const brands = Object.freeze(
      fullVersionList.map(item =>
        Object.freeze({{
          brand: item.brand,
          version: item.version,
        }})
      )
    );
    const uaData = Object.freeze({{
      brands,
      mobile: false,
      platform,
      toJSON() {{
        return {{ brands, mobile: false, platform }};
      }},
      async getHighEntropyValues(hints) {{
        const values = {{
          architecture: String(state.versions.arch || "x86"),
          bitness: String(state.versions.arch || "").includes("64") ? "64" : "32",
          brands,
          fullVersionList,
          mobile: false,
          model: "",
          platform,
          platformVersion: platform === "Windows" ? "10.0.0" : "",
          uaFullVersion: version || majorVersion,
          wow64: false,
        }};

        if (!Array.isArray(hints)) {{
          return values;
        }}

        return hints.reduce((result, hint) => {{
          if (Object.prototype.hasOwnProperty.call(values, hint)) {{
            result[hint] = values[hint];
          }}
          return result;
        }}, {{}});
      }},
    }});

    try {{
      Object.defineProperty(window.Navigator.prototype, "userAgentData", {{
        configurable: true,
        enumerable: true,
        get() {{
          return uaData;
        }},
      }});
      report(
        `ua_data_spoofed=true brands=${{brands.map(item => `${{item.brand}}/${{item.version}}`).join(",")}}`
      );
    }} catch (error) {{
      report(`ua_data_spoof_failed=${{error && error.message ? error.message : String(error)}}`, {{ force: true }});
    }}
  }};
  applyEdgeClientHintsSpoof();

  const collectVoiceDiagnostics = () => {{
    const senderAudioCaps = (() => {{
      try {{
        return window.RTCRtpSender?.getCapabilities?.("audio")?.codecs?.length ?? 0;
      }} catch {{
        return -1;
      }}
    }})();
    const senderVideoCaps = (() => {{
      try {{
        return window.RTCRtpSender?.getCapabilities?.("video")?.codecs?.length ?? 0;
      }} catch {{
        return -1;
      }}
    }})();
    const userAgentBrands = (() => {{
      try {{
        return navigator.userAgentData?.brands?.map(brand => `${{brand.brand}}/${{brand.version}}`).join(",") || "";
      }} catch {{
        return "";
      }}
    }})();

    return {{
      controlRuntime,
      installHostRuntime,
      installModRuntime,
      secureContext: window.isSecureContext,
      crossOriginIsolated: Boolean(window.crossOriginIsolated),
      mediaDevices: typeof navigator.mediaDevices === "object",
      getUserMedia: typeof navigator.mediaDevices?.getUserMedia === "function",
      enumerateDevices: typeof navigator.mediaDevices?.enumerateDevices === "function",
      permissionsApi: typeof navigator.permissions?.query === "function",
      peerConnection: typeof window.RTCPeerConnection === "function",
      sessionDescription: typeof window.RTCSessionDescription === "function",
      iceCandidate: typeof window.RTCIceCandidate === "function",
      encodedStreams:
        typeof window.RTCRtpScriptTransform === "function" ||
        typeof window.RTCRtpSender?.prototype?.createEncodedStreams === "function" ||
        typeof window.RTCRtpReceiver?.prototype?.createEncodedStreams === "function",
      insertableStreams:
        typeof window.RTCRtpSender?.prototype?.createEncodedAudioStreams === "function" ||
        typeof window.RTCRtpReceiver?.prototype?.createEncodedAudioStreams === "function",
      audioCapabilities: senderAudioCaps,
      videoCapabilities: senderVideoCaps,
      cryptoSubtle: typeof window.crypto?.subtle === "object",
      sharedArrayBuffer: typeof window.SharedArrayBuffer === "function",
      worker: typeof window.Worker === "function",
      mediaSource: typeof window.MediaSource === "function",
      userAgentBrands,
      protocol: window.location.protocol,
      host: window.location.hostname,
      ua: navigator.userAgent,
    }};
  }};

  const reportVoiceDiagnostics = reason => {{
    const diagnostics = collectVoiceDiagnostics();
    report(`voice_diag reason=${{reason}} data=${{JSON.stringify(diagnostics)}}`);
  }};

  const installVoiceDiagnostics = () => {{
    if (!isDiscordHost() || !shouldEmitRuntimeDiagnostics()) return;
    reportVoiceDiagnostics("bootstrap");
    window.setTimeout(() => reportVoiceDiagnostics("settled"), 2500);
  }};

  const getAutomaticGainControlPreference = () => {{
    try {{
      return window.Vencord?.Webpack?.Common?.MediaEngineStore?.getAutomaticGainControl?.();
    }} catch {{
      return undefined;
    }}
  }};

  const installMediaCompatibilityPatches = () => {{
    if (state.mediaCompatReady || !isDiscordHost()) return;
    if (typeof navigator.mediaDevices?.getUserMedia !== "function") return;

    state.mediaCompatReady = true;

    const fixAudioTrackConstraints = constraint => {{
      if (!constraint || typeof constraint !== "object") return;

      const target =
        Array.isArray(constraint.advanced)
          ? constraint.advanced.find(option => option && Object.prototype.hasOwnProperty.call(option, "autoGainControl")) || constraint
          : constraint;
      const automaticGainControl = getAutomaticGainControlPreference();
      if (typeof automaticGainControl === "boolean") {{
        target.autoGainControl = automaticGainControl;
      }}
    }};

    const fixVideoTrackConstraints = constraint => {{
      if (!constraint || typeof constraint !== "object") return;
      if (typeof constraint.deviceId === "string" && constraint.deviceId !== "default") {{
        constraint.deviceId = {{ exact: constraint.deviceId }};
      }}
    }};

    const fixStreamConstraints = constraints => {{
      if (!constraints || typeof constraints !== "object") return;

      if (constraints.audio) {{
        if (typeof constraints.audio !== "object") {{
          constraints.audio = {{}};
        }}
        fixAudioTrackConstraints(constraints.audio);
      }}

      if (constraints.video) {{
        if (typeof constraints.video !== "object") {{
          constraints.video = {{}};
        }}
        fixVideoTrackConstraints(constraints.video);
      }}
    }};

    const originalGetUserMedia = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia = function(constraints) {{
      try {{
        fixStreamConstraints(constraints);
      }} catch (error) {{
        console.warn("[Equirust] Failed to normalize getUserMedia constraints", error);
      }}

      return originalGetUserMedia(constraints);
    }};

    if (typeof window.MediaStreamTrack?.prototype?.applyConstraints === "function") {{
      const originalApplyConstraints = window.MediaStreamTrack.prototype.applyConstraints;
      window.MediaStreamTrack.prototype.applyConstraints = function(constraints) {{
        if (constraints) {{
          try {{
            if (this.kind === "audio") {{
              fixAudioTrackConstraints(constraints);
            }} else if (this.kind === "video") {{
              fixVideoTrackConstraints(constraints);
            }}
          }} catch (error) {{
            console.warn("[Equirust] Failed to normalize track constraints", error);
          }}
        }}

        return originalApplyConstraints.call(this, constraints);
      }};
    }}

    report("media_compat_installed=true");
  }};

  const readScreenShareQuality = () => {{
    try {{
      const raw = window.localStorage?.getItem("EquibopState");
      if (!raw) {{
        return {{ frameRate: 30, height: 720, width: 1280 }};
      }}

      const parsed = JSON.parse(raw);
      const frameRate = Number(parsed?.screenshareQuality?.frameRate ?? 30);
      const height = Number(parsed?.screenshareQuality?.resolution ?? 720);
      const safeFrameRate = Number.isFinite(frameRate) && frameRate > 0 ? frameRate : 30;
      const safeHeight = Number.isFinite(height) && height >= 480 ? height : 720;

      return {{
        frameRate: safeFrameRate,
        height: safeHeight,
        width: Math.round(safeHeight * (16 / 9)),
      }};
    }} catch (error) {{
      console.warn("[Equirust] Failed to read stored screen share quality", error);
      return {{ frameRate: 30, height: 720, width: 1280 }};
    }}
  }};

  const persistScreenShareQuality = quality => {{
    try {{
      const raw = window.localStorage?.getItem("EquibopState");
      const parsed = raw ? JSON.parse(raw) : {{}};
      parsed.screenshareQuality = {{
        resolution: String(quality?.height || 720),
        frameRate: String(quality?.frameRate || 30),
      }};
      window.localStorage?.setItem("EquibopState", JSON.stringify(parsed));
    }} catch (error) {{
      console.warn("[Equirust] Failed to persist screen share quality", error);
    }}
  }};

  const applyScreenShareTrackConstraints = async (videoTrack, quality, contentHint) => {{
    if (!videoTrack) return false;

    videoTrack.contentHint = contentHint === "detail" ? "detail" : "motion";
    const constraints = {{
      ...videoTrack.getConstraints(),
      frameRate: {{ min: quality.frameRate, ideal: quality.frameRate }},
      width: {{ min: 640, ideal: quality.width, max: quality.width }},
      height: {{ min: 480, ideal: quality.height, max: quality.height }},
      advanced: [{{ width: quality.width, height: quality.height }}],
      resizeMode: "none",
    }};

    try {{
      await videoTrack.applyConstraints(constraints);
      return true;
    }} catch (error) {{
      console.warn("[Equirust] Failed to apply display-media constraints", error);
      return false;
    }}
  }};

  const applyScreenShareConnectionQuality = (stream, quality) => {{
    try {{
      const common = window.Vencord?.Webpack?.Common;
      const mediaEngine = common?.MediaEngineStore?.getMediaEngine?.();
      const currentUserId = common?.UserStore?.getCurrentUser?.()?.id;
      const connections = Array.isArray(mediaEngine?.connections)
        ? mediaEngine.connections
        : mediaEngine?.connections
          ? Array.from(mediaEngine.connections)
          : [];

      const connection = connections.find(entry => {{
        if (!entry || entry.streamUserId !== currentUserId) return false;
        const inputStream = entry?.input?.stream;
        return inputStream === stream || inputStream?.id === stream?.id;
      }});

      if (!connection?.videoStreamParameters?.[0]) {{
        return false;
      }}

      connection.videoStreamParameters[0].maxFrameRate = quality.frameRate;
      if (connection.videoStreamParameters[0].maxResolution) {{
        connection.videoStreamParameters[0].maxResolution.width = quality.width;
        connection.videoStreamParameters[0].maxResolution.height = quality.height;
      }}

      if (connection.goLiveSource) {{
        connection.goLiveSource.quality = {{
          ...(connection.goLiveSource.quality || {{}}),
          frameRate: quality.frameRate,
          resolution: quality.height,
        }};
      }}

      return true;
    }} catch (error) {{
      console.warn("[Equirust] Failed to patch stream connection quality", error);
      return false;
    }}
  }};

  const reinforceScreenShareQuality = (stream, quality, contentHint) => {{
    let attempts = 0;
    const videoTrack = stream?.getVideoTracks?.()?.[0];
    const timer = window.setInterval(() => {{
      attempts += 1;
      const connectionReady = applyScreenShareConnectionQuality(stream, quality);
      if (videoTrack) {{
        void applyScreenShareTrackConstraints(videoTrack, quality, contentHint);
      }}
      if (connectionReady || attempts >= 24) {{
        window.clearInterval(timer);
      }}
    }}, 180);
  }};

  const ensureNativeSurfaceStyles = () => {{
    if (document.getElementById("equirust-surface-style")) {{
      return;
    }}

    const style = document.createElement("style");
    style.id = "equirust-surface-style";
    style.textContent = `
      :root {{
        --equirust-surface-backdrop:
          radial-gradient(circle at top, rgba(88, 101, 242, 0.16), transparent 38%),
          rgba(10, 12, 18, 0.72);
        --equirust-surface-shell:
          linear-gradient(180deg, rgba(255,255,255,0.03), rgba(255,255,255,0)),
          var(--modal-background, var(--background-primary, #11141b));
        --equirust-surface-shell-alt: var(--background-secondary, #151924);
        --equirust-surface-border: var(--background-modifier-accent, rgba(255,255,255,0.08));
        --equirust-surface-shadow:
          0 24px 70px rgba(0, 0, 0, 0.44),
          0 0 0 1px rgba(255, 255, 255, 0.02);
        --equirust-surface-radius: 18px;
        --equirust-surface-radius-sm: 12px;
        --equirust-surface-fg: var(--header-primary, #fff);
        --equirust-surface-muted: var(--text-muted, rgba(255,255,255,0.66));
        --equirust-surface-accent: var(--brand-experiment, #5865f2);
        --equirust-surface-card: var(--background-secondary, #171b27);
        --equirust-surface-card-alt: var(--background-tertiary, #0f121a);
      }}
      .equirust-surface-backdrop {{
        position: fixed;
        inset: 0;
        z-index: 2147483646;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 28px;
        background: var(--equirust-surface-backdrop);
        backdrop-filter: blur(18px);
      }}
      .equirust-surface-dialog {{
        color: var(--equirust-surface-fg);
        background: var(--equirust-surface-shell);
        border: 1px solid var(--equirust-surface-border);
        border-radius: var(--equirust-surface-radius);
        box-shadow: var(--equirust-surface-shadow);
      }}
      .equirust-surface-panel {{
        background: var(--equirust-surface-shell-alt);
        border-left: 1px solid var(--equirust-surface-border);
      }}
      .equirust-surface-header {{
        padding: 18px 20px 16px;
        border-bottom: 1px solid var(--equirust-surface-border);
      }}
      .equirust-surface-eyebrow {{
        margin: 0 0 6px;
        color: var(--equirust-surface-muted);
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }}
      .equirust-surface-title {{
        margin: 0;
        font-size: 22px;
        line-height: 1.1;
        font-weight: 800;
      }}
      .equirust-surface-copy {{
        margin: 8px 0 0;
        color: var(--equirust-surface-muted);
        font-size: 14px;
        line-height: 1.45;
      }}
      .equirust-surface-footer {{
        margin-top: auto;
        padding: 16px 18px 18px;
        border-top: 1px solid var(--equirust-surface-border);
        display: flex;
        justify-content: flex-end;
        gap: 10px;
      }}
      .equirust-surface-button {{
        min-width: 104px;
        min-height: 38px;
        border-radius: 10px;
        border: 0;
        padding: 0 14px;
        font: inherit;
        font-size: 14px;
        font-weight: 700;
        cursor: pointer;
        transition: filter 120ms ease, transform 120ms ease, opacity 120ms ease;
      }}
      .equirust-surface-button:hover {{
        filter: brightness(1.05);
        transform: translateY(-1px);
      }}
      .equirust-surface-button:disabled {{
        opacity: 0.45;
        cursor: default;
        transform: none;
        filter: none;
      }}
      .equirust-surface-button--secondary {{
        background: var(--background-modifier-hover, rgba(255,255,255,0.08));
        color: var(--equirust-surface-fg);
      }}
      .equirust-surface-button--primary {{
        background: var(--equirust-surface-accent);
        color: white;
      }}
    `;
    document.documentElement.appendChild(style);
  }};

  const ensureScreenSharePickerStyles = () => {{
    ensureNativeSurfaceStyles();
    if (document.getElementById("equirust-screenshare-style")) {{
      return;
    }}

    const style = document.createElement("style");
    style.id = "equirust-screenshare-style";
    style.textContent = `
      .equirust-screenshare.equirust-surface-backdrop {{
        backdrop-filter: none;
        background: transparent;
      }}
      .equirust-screenshare__dialog {{
        width: min(1080px, calc(100vw - 48px));
        max-height: min(760px, calc(100vh - 48px));
        display: grid;
        grid-template-columns: minmax(0, 1.55fr) minmax(280px, 0.9fr);
        overflow: hidden;
        color: var(--header-primary, #fff);
      }}
      .equirust-screenshare__main {{
        min-width: 0;
        display: flex;
        flex-direction: column;
        background:
          linear-gradient(180deg, color-mix(in srgb, var(--background-primary, #11141b) 88%, white 3%), var(--background-primary, #11141b));
      }}
      .equirust-screenshare__sidebar {{
        min-width: 0;
        display: flex;
        flex-direction: column;
      }}
      .equirust-screenshare__tabs {{
        display: inline-flex;
        gap: 8px;
        margin-top: 14px;
      }}
      .equirust-screenshare__tab {{
        border: 0;
        border-radius: 999px;
        padding: 8px 13px;
        background: var(--background-secondary-alt, rgba(255,255,255,0.06));
        color: var(--header-secondary, rgba(255,255,255,0.7));
        font-size: 13px;
        font-weight: 700;
        cursor: pointer;
        transition: background-color 140ms ease, color 140ms ease, transform 140ms ease;
      }}
      .equirust-screenshare__tab[data-active="true"] {{
        background: var(--brand-experiment, #5865f2);
        color: white;
        transform: translateY(-1px);
      }}
      .equirust-screenshare__grid {{
        padding: 18px 20px 20px;
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
        gap: 14px;
        overflow: auto;
        align-content: start;
      }}
      .equirust-screenshare__card {{
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        border-radius: 14px;
        overflow: hidden;
        background: var(--background-secondary, #171b27);
        cursor: pointer;
        transition: transform 140ms ease, border-color 140ms ease, background-color 140ms ease, box-shadow 140ms ease;
      }}
      .equirust-screenshare__card:hover {{
        transform: translateY(-1px);
        border-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 48%, rgba(255,255,255,0.12));
      }}
      .equirust-screenshare__card[data-selected="true"] {{
        border-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 78%, white 8%);
        background: color-mix(in srgb, var(--background-secondary, #171b27) 90%, var(--brand-experiment, #5865f2) 10%);
        box-shadow: 0 0 0 1px color-mix(in srgb, var(--brand-experiment, #5865f2) 55%, transparent);
      }}
      .equirust-screenshare__cardPreview {{
        aspect-ratio: 16 / 9;
        width: 100%;
        object-fit: cover;
        display: block;
        background:
          linear-gradient(135deg, rgba(88, 101, 242, 0.16), transparent),
          var(--background-tertiary, #0f121a);
      }}
      .equirust-screenshare__cardMeta {{
        padding: 10px 12px 12px;
      }}
      .equirust-screenshare__cardName {{
        font-size: 13px;
        font-weight: 700;
        line-height: 1.3;
        color: var(--header-primary, #fff);
        display: -webkit-box;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
      }}
      .equirust-screenshare__cardKind {{
        margin-top: 4px;
        color: var(--text-muted, rgba(255,255,255,0.62));
        font-size: 12px;
      }}
      .equirust-screenshare__empty {{
        padding: 30px 20px 24px;
        color: var(--text-muted, rgba(255,255,255,0.62));
        font-size: 14px;
      }}
      .equirust-screenshare__previewWrap {{
        padding: 18px 18px 14px;
      }}
      .equirust-screenshare__preview {{
        width: 100%;
        aspect-ratio: 16 / 9;
        border-radius: 14px;
        overflow: hidden;
        background:
          linear-gradient(135deg, rgba(88, 101, 242, 0.16), transparent),
          var(--background-tertiary, #0f121a);
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
      }}
      .equirust-screenshare__preview img {{
        width: 100%;
        height: 100%;
        object-fit: cover;
        display: block;
      }}
      .equirust-screenshare__previewLabel {{
        margin-top: 10px;
        font-size: 15px;
        font-weight: 700;
        line-height: 1.3;
      }}
      .equirust-screenshare__controls {{
        padding: 0 18px 18px;
        display: grid;
        gap: 12px;
      }}
      .equirust-screenshare__field {{
        display: grid;
        gap: 6px;
      }}
      .equirust-screenshare__fieldLabel {{
        color: var(--header-secondary, rgba(255,255,255,0.7));
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }}
      .equirust-screenshare__select,
      .equirust-screenshare__toggle {{
        width: 100%;
        min-height: 40px;
        border-radius: 11px;
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        background: var(--background-tertiary, #0f121a);
        color: var(--header-primary, #fff);
        padding: 0 12px;
        font: inherit;
      }}
      .equirust-screenshare__toggle {{
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        cursor: pointer;
      }}
      .equirust-screenshare__toggle input {{
        accent-color: var(--brand-experiment, #5865f2);
      }}
      .equirust-screenshare__audioNote {{
        margin: -4px 2px 0;
        color: var(--text-muted, rgba(255,255,255,0.48));
        font-size: 12px;
        line-height: 1.35;
      }}
      .equirust-screenshare__hintOptions {{
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 8px;
      }}
      .equirust-screenshare__hint {{
        min-height: 40px;
        border-radius: 11px;
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        background: var(--background-tertiary, #0f121a);
        color: var(--header-secondary, rgba(255,255,255,0.72));
        font-size: 13px;
        font-weight: 700;
        cursor: pointer;
      }}
      .equirust-screenshare__hint[data-active="true"] {{
        border-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 65%, white 6%);
        background: color-mix(in srgb, var(--background-tertiary, #0f121a) 76%, var(--brand-experiment, #5865f2) 24%);
        color: white;
      }}
      @media (max-width: 980px) {{
        .equirust-screenshare__dialog {{
          grid-template-columns: 1fr;
          max-height: calc(100vh - 32px);
        }}
        .equirust-screenshare__sidebar {{
          border-top: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        }}
      }}
    `;
    document.documentElement.appendChild(style);
  }};

  const createAbortError = message => {{
    try {{
      return new DOMException(message, "AbortError");
    }} catch {{
      const error = new Error(message);
      error.name = "AbortError";
      return error;
    }}
  }};

  const escapeHtml = value =>
    String(value ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");

  const loadScreenShareSources = async () => {{
    const sources = await invoke("get_capturer_sources");
    return Array.isArray(sources)
      ? sources.filter(
          source =>
            source &&
            typeof source.id === "string" &&
            typeof source.name === "string" &&
            typeof source.url === "string"
        )
      : [];
  }};

  const loadScreenShareThumbnail = async sourceId => {{
    if (!sourceId) return null;
    if (typeof state.screenShareThumbnailCache[sourceId] === "string") {{
      return state.screenShareThumbnailCache[sourceId];
    }}

    try {{
      const url = await invoke("get_capturer_thumbnail", {{ id: sourceId }});
      if (typeof url === "string" && url.length) {{
        state.screenShareThumbnailCache[sourceId] = url;
        return url;
      }}
    }} catch (error) {{
      console.warn("[Equirust] Failed to load screen share thumbnail", error);
    }}

    return null;
  }};

  const loadLargeScreenSharePreview = async sourceId => {{
    if (!sourceId) return null;
    if (typeof state.screenSharePreviewCache[sourceId] === "string") {{
      return state.screenSharePreviewCache[sourceId];
    }}

    try {{
      const url = await invoke("get_capturer_large_thumbnail", {{ id: sourceId }});
      if (typeof url === "string" && url.length) {{
        state.screenSharePreviewCache[sourceId] = url;
        return url;
      }}
    }} catch (error) {{
      console.warn("[Equirust] Failed to load large screen share preview", error);
    }}

    return null;
  }};

  const openScreenSharePicker = (sources, defaults = {{}}) => {{
    ensureScreenSharePickerStyles();

    if (state.screenSharePickerBusy) {{
      return Promise.reject(createAbortError("Screen share picker is already open."));
    }}

    state.screenSharePickerBusy = true;

    const normalizePickerSources = value =>
      Array.isArray(value)
        ? value.filter(
            source =>
              source &&
              typeof source.id === "string" &&
              typeof source.name === "string" &&
              typeof source.url === "string"
          )
        : [];
    const sourcesSignature = value =>
      normalizePickerSources(value)
        .map(
          source =>
            `${{source.id}}|${{source.name}}|${{source.kind}}|${{source.processName || ""}}`
        )
        .sort()
        .join("||");
    let pickerSources = normalizePickerSources(sources);
    const quality = readScreenShareQuality();
    const getSourceGroups = () => ({{
      window: pickerSources.filter(
        source => String(source.kind || "").toLowerCase() === "window"
      ),
      screen: pickerSources.filter(
        source => String(source.kind || "").toLowerCase() === "screen"
      ),
    }});
    let groups = getSourceGroups();
    const resolutionOptions = [480, 720, 1080, 1440, 2160];
    const frameRateOptions = [15, 30, 60];
    const toPositiveNumber = value => {{
      const parsed = Number(value);
      return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
    }};
    const chooseSupportedQuality = (source, fallbackQuality) => {{
      const nativeWidth = toPositiveNumber(source?.nativeWidth);
      const nativeHeight = toPositiveNumber(source?.nativeHeight);
      const nativeAspectRatio =
        nativeWidth > 0 && nativeHeight > 0 ? nativeWidth / nativeHeight : 16 / 9;
      const fallbackHeight = toPositiveNumber(fallbackQuality?.height) || 720;
      const fallbackFrameRate = toPositiveNumber(fallbackQuality?.frameRate) || 30;
      const availableHeight = nativeHeight || fallbackHeight;
      const availableFrameRate =
        Math.min(60, toPositiveNumber(source?.maxFrameRate) || fallbackFrameRate || 30);

      const height =
        resolutionOptions
          .filter(option => option <= availableHeight)
          .pop() ||
        resolutionOptions[0];
      const frameRate =
        frameRateOptions
          .filter(option => option <= availableFrameRate)
          .pop() ||
        frameRateOptions[0];
      const width =
        nativeWidth > 0 && nativeHeight > 0 && height === nativeHeight
          ? Math.round(nativeWidth)
          : Math.max(1, Math.round(height * nativeAspectRatio));

      return {{ width, height, frameRate }};
    }};
    const hasExplicitDefaultQuality =
      toPositiveNumber(defaults.width) > 0 ||
      toPositiveNumber(defaults.height) > 0 ||
      toPositiveNumber(defaults.frameRate) > 0;
    const initialTab = groups.window.length ? "window" : "screen";
    let activeTab = initialTab;
    let selectedId =
      (groups[initialTab][0] || groups.screen[0] || groups.window[0] || {{}}).id || "";
    const findSourceById = sourceId =>
      pickerSources.find(source => source.id === sourceId) || null;
    const currentSource = () =>
      findSourceById(selectedId) ||
      groups[activeTab].find(source => source.id === selectedId) ||
      groups.window[0] ||
      groups.screen[0] ||
      null;
    const initialSource = currentSource();
    let currentQuality = hasExplicitDefaultQuality
      ? {{
          width:
            toPositiveNumber(defaults.width) ||
            Math.round(
              (toPositiveNumber(defaults.height) || quality.height || 720) * (16 / 9)
            ),
          height: toPositiveNumber(defaults.height) || quality.height || 720,
          frameRate: toPositiveNumber(defaults.frameRate) || quality.frameRate || 30,
        }}
      : chooseSupportedQuality(initialSource, quality);
    let qualityDirty = hasExplicitDefaultQuality;
    let includeAudio = defaults.audio !== false;
    let contentHint = defaults.contentHint === "detail" ? "detail" : "motion";

    return new Promise((resolve, reject) => {{
      const overlay = document.createElement("div");
      overlay.className = "equirust-screenshare equirust-surface-backdrop";
      let closed = false;
      let refreshTimer = null;
      let refreshInFlight = false;
      let lastSourceSignature = sourcesSignature(pickerSources);

      const refreshGroupsAndSelection = () => {{
        groups = getSourceGroups();
        if (!groups[activeTab]?.length) {{
          activeTab = groups.window.length ? "window" : "screen";
        }}
        if (!groups[activeTab]?.some(source => source.id === selectedId)) {{
          selectedId =
            (groups[activeTab][0] || groups.window[0] || groups.screen[0] || {{}}).id || "";
        }}
        if (!qualityDirty) {{
          currentQuality = chooseSupportedQuality(currentSource(), quality);
        }}
      }};

      const cleanup = result => {{
        if (closed) {{
          return;
        }}
        closed = true;
        if (refreshTimer) {{
          window.clearInterval(refreshTimer);
          refreshTimer = null;
        }}
        document.removeEventListener("keydown", onKeyDown, true);
        overlay.remove();
        state.screenSharePickerBusy = false;
        if (result?.ok) {{
          resolve(result.value);
        }} else {{
          reject(result?.error || createAbortError("Screen share was cancelled."));
        }}
      }};

      const selectSource = nextId => {{
        selectedId = String(nextId || "");
        if (!qualityDirty) {{
          currentQuality = chooseSupportedQuality(currentSource(), quality);
        }}
        render();
      }};

      const onKeyDown = event => {{
        if (event.key === "Escape") {{
          event.preventDefault();
          cleanup({{ ok: false }});
        }}
      }};

      const render = () => {{
        if (closed) {{
          return;
        }}
        const visibleSources = groups[activeTab];
        const chosen = currentSource();
        const sourceKind = String(chosen?.kind || activeTab || "screen");
        const previewUrl =
          (chosen?.id && state.screenSharePreviewCache[chosen.id]) ||
          chosen?.url ||
          "";

        overlay.innerHTML = `
          <div class="equirust-screenshare__dialog equirust-surface-dialog" role="dialog" aria-modal="true" aria-label="Screen Share Picker">
            <section class="equirust-screenshare__main">
              <div class="equirust-screenshare__header equirust-surface-header">
                <p class="equirust-screenshare__eyebrow equirust-surface-eyebrow">Screen Share</p>
                <h2 class="equirust-screenshare__title equirust-surface-title">Choose what to stream</h2>
                <p class="equirust-screenshare__description equirust-surface-copy">Use a desktop-style source picker inside Discord, then continue into the capture handoff with your quality preferences already applied.</p>
                <div class="equirust-screenshare__tabs">
                  <button class="equirust-screenshare__tab" type="button" data-tab="window" data-active="${{activeTab === "window"}}">Applications</button>
                  <button class="equirust-screenshare__tab" type="button" data-tab="screen" data-active="${{activeTab === "screen"}}">Screens</button>
                </div>
              </div>
              <div class="equirust-screenshare__grid">
                ${{
                  visibleSources.length
                    ? visibleSources
                        .map(
                          source => `
                            <button class="equirust-screenshare__card" type="button" data-source-id="${{source.id}}" data-selected="${{source.id === selectedId}}">
                              <img class="equirust-screenshare__cardPreview" data-preview-id="${{source.id}}" src="${{state.screenShareThumbnailCache[source.id] || source.url}}" alt="" />
                              <div class="equirust-screenshare__cardMeta">
                                <div class="equirust-screenshare__cardName">${{source.name}}</div>
                                <div class="equirust-screenshare__cardKind">${{source.kind === "window" ? escapeHtml(source.processName || "Unknown App") : "Display"}}</div>
                              </div>
                            </button>
                          `
                        )
                        .join("")
                    : `<div class="equirust-screenshare__empty">No ${{
                        activeTab === "window" ? "shareable application windows" : "screens"
                      }} are available right now.</div>`
                }}
              </div>
            </section>
            <aside class="equirust-screenshare__sidebar equirust-surface-panel">
              <div class="equirust-screenshare__sidebarHeader equirust-surface-header">
                <p class="equirust-screenshare__eyebrow equirust-surface-eyebrow">Stream Settings</p>
                <h3 class="equirust-screenshare__title equirust-surface-title" style="font-size:18px;">${{chosen?.name || "Nothing selected"}}</h3>
              </div>
              <div class="equirust-screenshare__previewWrap">
                <div class="equirust-screenshare__preview">
                  ${{
                    previewUrl
                      ? `<img src="${{previewUrl}}" alt="" />`
                      : ""
                  }}
                </div>
                <div class="equirust-screenshare__previewLabel">${{
                  sourceKind === "window"
                    ? escapeHtml(chosen?.processName || "Unknown App")
                    : "Display"
                }}</div>
              </div>
              <div class="equirust-screenshare__controls">
                <label class="equirust-screenshare__field">
                  <span class="equirust-screenshare__fieldLabel">Resolution</span>
                  <select class="equirust-screenshare__select" data-control="resolution">
                    ${{[480, 720, 1080, 1440, 2160]
                      .map(
                        value =>
                          `<option value="${{value}}" ${{
                            currentQuality.height === value ? "selected" : ""
                          }}>${{value}}p</option>`
                      )
                      .join("")}}
                  </select>
                </label>
                <label class="equirust-screenshare__field">
                  <span class="equirust-screenshare__fieldLabel">Frame Rate</span>
                  <select class="equirust-screenshare__select" data-control="framerate">
                    ${{[15, 30, 60]
                      .map(
                        value =>
                          `<option value="${{value}}" ${{
                            currentQuality.frameRate === value ? "selected" : ""
                          }}>${{value}} FPS</option>`
                      )
                      .join("")}}
                  </select>
                </label>
                <div class="equirust-screenshare__field">
                  <span class="equirust-screenshare__fieldLabel">Content Type</span>
                  <div class="equirust-screenshare__hintOptions">
                    <button class="equirust-screenshare__hint" type="button" data-hint="motion" data-active="${{contentHint === "motion"}}">Prefer Smoothness</button>
                    <button class="equirust-screenshare__hint" type="button" data-hint="detail" data-active="${{contentHint === "detail"}}">Prefer Clarity</button>
                  </div>
                </div>
                <label class="equirust-screenshare__toggle">
                  <span>${{sourceKind === "window" ? "Include App Audio" : "Include System Audio"}}</span>
                  <input type="checkbox" data-control="audio" ${{
                    includeAudio ? "checked" : ""
                  }} />
                </label>
                <p class="equirust-screenshare__audioNote">${{
                  sourceKind === "window"
                    ? "Only the selected app's audio will be shared."
                    : "System audio excludes Equirust and Discord output to reduce feedback."
                }}</p>
              </div>
              <div class="equirust-screenshare__footer equirust-surface-footer">
                <button class="equirust-screenshare__button equirust-surface-button equirust-surface-button--secondary" type="button" data-action="cancel">Cancel</button>
                <button class="equirust-screenshare__button equirust-surface-button equirust-surface-button--primary" type="button" data-action="continue" ${{
                  chosen ? "" : "disabled"
                }}>Continue</button>
              </div>
            </aside>
          </div>
        `;

        overlay.querySelectorAll("[data-tab]").forEach(button => {{
          button.addEventListener("click", () => {{
            activeTab = button.getAttribute("data-tab") === "screen" ? "screen" : "window";
            if (!groups[activeTab].some(source => source.id === selectedId)) {{
              selectedId = groups[activeTab][0]?.id || "";
            }}
            if (!qualityDirty) {{
              currentQuality = chooseSupportedQuality(currentSource(), quality);
            }}
            render();
          }});
        }});

        overlay.querySelectorAll("[data-source-id]").forEach(button => {{
          button.addEventListener("click", () => {{
            selectSource(button.getAttribute("data-source-id"));
          }});
        }});

        overlay.querySelector('[data-control="resolution"]')?.addEventListener("change", event => {{
          qualityDirty = true;
          currentQuality.height = Number(event.target.value || 720) || 720;
          const chosen = currentSource();
          const nativeWidth = toPositiveNumber(chosen?.nativeWidth);
          const nativeHeight = toPositiveNumber(chosen?.nativeHeight);
          const aspectRatio =
            nativeWidth > 0 && nativeHeight > 0 ? nativeWidth / nativeHeight : 16 / 9;
          currentQuality.width =
            nativeWidth > 0 && nativeHeight > 0 && currentQuality.height === nativeHeight
              ? Math.round(nativeWidth)
              : Math.max(1, Math.round(currentQuality.height * aspectRatio));
        }});

        overlay.querySelector('[data-control="framerate"]')?.addEventListener("change", event => {{
          qualityDirty = true;
          currentQuality.frameRate = Number(event.target.value || 30) || 30;
        }});

        overlay.querySelector('[data-control="audio"]')?.addEventListener("change", event => {{
          includeAudio = event.target.checked === true;
        }});

        overlay.querySelectorAll("[data-hint]").forEach(button => {{
          button.addEventListener("click", () => {{
            contentHint = button.getAttribute("data-hint") === "detail" ? "detail" : "motion";
            render();
          }});
        }});

        overlay.querySelector('[data-action="cancel"]')?.addEventListener("click", () => {{
          cleanup({{ ok: false }});
        }});

        overlay.querySelector('[data-action="continue"]')?.addEventListener("click", () => {{
          const picked = currentSource();
          if (!picked) return;
          persistScreenShareQuality(currentQuality);
          cleanup({{
            ok: true,
            value: {{
              id: picked.id,
              kind: picked.kind === "window" ? "window" : "screen",
              processId:
                typeof picked.processId === "number" && Number.isFinite(picked.processId)
                  ? picked.processId
                  : null,
              audio: includeAudio,
              contentHint,
              frameRate: currentQuality.frameRate,
              height: currentQuality.height,
              width: currentQuality.width,
            }},
          }});
        }});

        const chosenSource = currentSource();
        visibleSources.forEach(source => {{
          if (!source?.id || state.screenShareThumbnailCache[source.id]) return;
          loadScreenShareThumbnail(source.id).then(url => {{
            if (!url) return;
            const previewImage = overlay.querySelector(`[data-preview-id="${{CSS.escape(source.id)}}"]`);
            if (previewImage) {{
              previewImage.src = url;
            }}
          }});
        }});
        if (chosenSource?.id && !state.screenSharePreviewCache[chosenSource.id]) {{
          loadLargeScreenSharePreview(chosenSource.id).then(url => {{
            if (!url || currentSource()?.id !== chosenSource.id) return;
            const previewImage = overlay.querySelector(".equirust-screenshare__preview img");
            if (previewImage) {{
              previewImage.src = url;
            }}
          }});
        }}
      }};

      const refreshSources = async () => {{
        if (closed || refreshInFlight) {{
          return;
        }}
        refreshInFlight = true;
        try {{
          const refreshed = normalizePickerSources(await loadScreenShareSources());
          const refreshedSignature = sourcesSignature(refreshed);
          if (refreshedSignature !== lastSourceSignature) {{
            pickerSources = refreshed;
            lastSourceSignature = refreshedSignature;
            refreshGroupsAndSelection();
            render();
          }}
        }} catch (error) {{
          console.warn("[Equirust] Failed to refresh screen share sources", error);
        }} finally {{
          refreshInFlight = false;
        }}
      }};

      overlay.addEventListener("click", event => {{
        if (event.target === overlay) {{
          cleanup({{ ok: false }});
        }}
      }});

      document.addEventListener("keydown", onKeyDown, true);
      document.body.appendChild(overlay);
      refreshGroupsAndSelection();
      render();
      refreshTimer = window.setInterval(() => {{
        void refreshSources();
      }}, 1200);
    }});
  }};

  const startNativeDisplayMediaStream = async picked => {{
    const session = await invoke("start_native_capture_session", {{
      request: {{
        sourceId: picked.id,
        sourceKind: picked.kind,
        sourceProcessId:
          typeof picked.processId === "number" && Number.isFinite(picked.processId)
            ? picked.processId
            : null,
        width: Number(picked.width || 1280) || 1280,
        height: Number(picked.height || 720) || 720,
        frameRate: Number(picked.frameRate || 30) || 30,
        contentHint: picked.contentHint === "detail" ? "detail" : "motion",
        includeSystemAudio: picked.audio === true,
      }},
    }});

    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Number(session.width || picked.width || 1280) || 1280);
    canvas.height = Math.max(1, Number(session.height || picked.height || 720) || 720);
    canvas.setAttribute("aria-hidden", "true");
    canvas.style.position = "fixed";
    canvas.style.left = "-99999px";
    canvas.style.top = "-99999px";
    canvas.style.width = `${{canvas.width}}px`;
    canvas.style.height = `${{canvas.height}}px`;
    canvas.style.pointerEvents = "none";
    canvas.style.opacity = "0";
    document.body.appendChild(canvas);

    const context = canvas.getContext("2d", {{
      alpha: false,
      desynchronized: true,
    }});
    if (!context) {{
      canvas.remove();
      await invoke("stop_native_capture_session", {{ sessionId: session.sessionId }}).catch(() => {{}});
      throw new Error("Native screen share could not start: canvas initialization failed.");
    }}

    let socket = null;
    let closed = false;
    let stopIssued = false;
    let decodeBusy = false;
    let pendingFrame = null;
    const videoCodec = typeof session.videoCodec === "string" ? session.videoCodec : "jpeg";
    const frameDurationMicros = Math.max(
      1,
      Math.round(1_000_000 / Math.max(1, Number(session.frameRate || picked.frameRate || 30) || 30))
    );
    const encoderMode = String(session.encoderMode || "Software H.264");
    const encoderDetail =
      typeof session.encoderDetail === "string" && session.encoderDetail.trim()
        ? session.encoderDetail.trim()
        : "";
    const colorMode = String(session.colorMode || "SDR-safe");
    const nativeReport = (message, options = {{}}) =>
      report(`native_capture session=${{session.sessionId}} ${{message}}`, options);
    const normalizeErrorMessage = error =>
      error && typeof error.message === "string" ? error.message : String(error);
    report(
      "display_media_native_session=" +
        JSON.stringify({{
          sessionId: session.sessionId,
          requestedWidth: Number(picked.width || 1280) || 1280,
          requestedHeight: Number(picked.height || 720) || 720,
          requestedFrameRate: Number(picked.frameRate || 30) || 30,
          actualWidth: canvas.width,
          actualHeight: canvas.height,
          actualFrameRate: Number(session.frameRate || picked.frameRate || 30) || 30,
          codec: videoCodec,
          encoderMode,
          encoderDetail: encoderDetail || null,
          colorMode,
          audioEnabled: session.audioEnabled === true,
        }})
    );
    let videoDecoder = null;
    let audioChannels = Math.max(1, Number(session.audioChannels || 0) || 0);
    let audioSampleRate = Math.max(1, Number(session.audioSampleRate || 0) || 0);
    let audioQueue = [];
    let audioNodeState = {{ chunkOffset: 0 }};
    let audioContext =
      session.audioEnabled && audioChannels > 0 && audioSampleRate > 0
        ? new AudioContext({{ sampleRate: audioSampleRate, latencyHint: "interactive" }})
        : null;
    const audioDestination = audioContext ? audioContext.createMediaStreamDestination() : null;
    const audioProcessor = audioContext
      ? audioContext.createScriptProcessor(2048, 0, audioChannels)
      : null;

    if (audioProcessor && audioDestination) {{
      audioProcessor.onaudioprocess = event => {{
        const outputBuffer = event.outputBuffer;
        const frameCount = outputBuffer.length;
        for (let channel = 0; channel < audioChannels; channel += 1) {{
          outputBuffer.getChannelData(channel).fill(0);
        }}

        let frameIndex = 0;
        while (frameIndex < frameCount && audioQueue.length) {{
          const head = audioQueue[0];
          const availableSamples = head.length - audioNodeState.chunkOffset;
          const availableFrames = Math.floor(availableSamples / audioChannels);
          if (availableFrames <= 0) {{
            audioQueue.shift();
            audioNodeState.chunkOffset = 0;
            continue;
          }}

          const framesToCopy = Math.min(frameCount - frameIndex, availableFrames);
          for (let index = 0; index < framesToCopy; index += 1) {{
            const sourceOffset = audioNodeState.chunkOffset + index * audioChannels;
            for (let channel = 0; channel < audioChannels; channel += 1) {{
              outputBuffer.getChannelData(channel)[frameIndex + index] =
                head[sourceOffset + channel] ?? 0;
            }}
          }}

          audioNodeState.chunkOffset += framesToCopy * audioChannels;
          frameIndex += framesToCopy;
          if (audioNodeState.chunkOffset >= head.length) {{
            audioQueue.shift();
            audioNodeState.chunkOffset = 0;
          }}
        }}
      }};
      audioProcessor.connect(audioDestination);
      void audioContext.resume().catch(() => {{}});
    }}

    if (videoCodec !== "jpeg") {{
      if (typeof VideoDecoder !== "function") {{
        canvas.remove();
        if (audioProcessor) {{
          try {{
            audioProcessor.disconnect();
          }} catch {{}}
        }}
        if (audioContext && audioContext.state !== "closed") {{
          await audioContext.close().catch(() => {{}});
        }}
        await invoke("stop_native_capture_session", {{ sessionId: session.sessionId }}).catch(() => {{}});
        throw new Error("Native screen share could not start: H.264 decode is unavailable in this runtime.");
      }}

      const decoderConfig = {{
        codec: videoCodec,
        optimizeForLatency: true,
        hardwareAcceleration: "prefer-hardware",
        avc: {{ format: "annexb" }},
      }};
      const support = typeof VideoDecoder.isConfigSupported === "function"
        ? await VideoDecoder.isConfigSupported(decoderConfig).catch(() => null)
        : null;
      if (support && support.supported === false) {{
        canvas.remove();
        if (audioProcessor) {{
          try {{
            audioProcessor.disconnect();
          }} catch {{}}
        }}
        if (audioContext && audioContext.state !== "closed") {{
          await audioContext.close().catch(() => {{}});
        }}
        await invoke("stop_native_capture_session", {{ sessionId: session.sessionId }}).catch(() => {{}});
        throw new Error("Native screen share could not start: H.264 decode is unsupported in this runtime.");
      }}

      videoDecoder = new VideoDecoder({{
        output: frame => {{
          try {{
            context.clearRect(0, 0, canvas.width, canvas.height);
            context.drawImage(frame, 0, 0, canvas.width, canvas.height);
          }} finally {{
            frame.close();
          }}
        }},
        error: error => {{
          console.warn("[Equirust] Native screen-share decode error:", error);
          nativeReport(`decoder_error=${{normalizeErrorMessage(error)}}`, {{ force: true }});
          void forceStopTracks(false, "decoder_error");
        }},
      }});
      videoDecoder.configure(support?.config || decoderConfig);
    }}

    const stream = canvas.captureStream(
      Math.max(1, Number(session.frameRate || picked.frameRate || 30) || 30)
    );
    if (audioDestination?.stream?.getAudioTracks?.().length) {{
      const audioTrack = audioDestination.stream.getAudioTracks()[0];
      if (audioTrack) {{
        stream.addTrack(audioTrack);
      }}
    }}
    const tracks = stream.getTracks();

    const stopSession = async (stopNativeSession, reason = "unknown") => {{
      if (closed) return;
      closed = true;
      nativeReport(
        `stop_session reason=${{reason}} stopNativeSession=${{stopNativeSession}} stopIssued=${{stopIssued}}`
      );

      try {{
        if (socket && socket.readyState === WebSocket.OPEN) {{
          socket.close();
        }}
      }} catch {{}}

      socket = null;
      pendingFrame = null;
      canvas.remove();
      if (videoDecoder && videoDecoder.state !== "closed") {{
        try {{
          await videoDecoder.flush();
        }} catch {{}}
        try {{
          videoDecoder.close();
        }} catch {{}}
      }}
      audioQueue.length = 0;
      if (audioProcessor) {{
        try {{
          audioProcessor.disconnect();
        }} catch {{}}
      }}
      if (audioDestination) {{
        try {{
          audioDestination.disconnect?.();
        }} catch {{}}
      }}
      if (audioContext && audioContext.state !== "closed") {{
        await audioContext.close().catch(() => {{}});
      }}

      if (stopNativeSession && !stopIssued) {{
        stopIssued = true;
        await invoke("stop_native_capture_session", {{ sessionId: session.sessionId }}).catch(error => {{
          nativeReport(`stop_native_failed=${{normalizeErrorMessage(error)}}`, {{ force: true }});
        }});
      }}
    }};

    const forceStopTracks = async (stopNativeSession, reason = "unknown") => {{
      nativeReport(`force_stop_tracks reason=${{reason}} stopNativeSession=${{stopNativeSession}}`);
      await stopSession(stopNativeSession, `force_stop_tracks:${{reason}}`);
      tracks.forEach(track => {{
        try {{
          track.enabled = false;
        }} catch {{}}
        try {{
          MediaStreamTrack.prototype.stop.call(track);
        }} catch {{}}
        try {{
          track.dispatchEvent(new Event("ended"));
        }} catch {{}}
      }});
    }};

    tracks.forEach(track => {{
      track.contentHint = picked.contentHint === "detail" ? "detail" : "motion";
      try {{
        const originalStop = track.stop.bind(track);
        track.stop = () => {{
          void stopSession(true, `track_stop:${{track.kind}}`);
          originalStop();
        }};
      }} catch {{}}
      track.addEventListener("ended", () => {{
        void stopSession(true, `track_ended:${{track.kind}}`);
      }});
    }});

    const drawFrame = async bytes => {{
      decodeBusy = true;
      try {{
        const blob = new Blob([bytes], {{ type: "image/jpeg" }});
        const bitmap = await createImageBitmap(blob);
        context.clearRect(0, 0, canvas.width, canvas.height);
        context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
        bitmap.close();
      }} catch (error) {{
        nativeReport(`jpeg_decode_failed=${{normalizeErrorMessage(error)}}`, {{ force: true }});
        void forceStopTracks(false, "jpeg_decode_failed");
      }} finally {{
        decodeBusy = false;
        if (pendingFrame && !closed) {{
          const nextFrame = pendingFrame;
          pendingFrame = null;
          void drawFrame(nextFrame);
        }}
      }}
    }};

    const handleFrame = payload => {{
      const bytes = new Uint8Array(payload);
      if (bytes.byteLength <= 10 || bytes[0] !== 0x01) return;
      const flags = bytes[1];
      const timestamp = Number(
        new DataView(bytes.buffer, bytes.byteOffset + 2, 8).getBigUint64(0, true)
      );
      const frameBytes = bytes.slice(10);

      if (videoCodec === "jpeg") {{
        if (decodeBusy) {{
          pendingFrame = frameBytes;
          return;
        }}
        void drawFrame(frameBytes);
        return;
      }}

      if (!videoDecoder || videoDecoder.state === "closed") return;
      const chunkType = (flags & 0x01) === 0x01 ? "key" : "delta";
      if (videoDecoder.decodeQueueSize > 3 && chunkType !== "key") {{
        return;
      }}

      try {{
        videoDecoder.decode(
          new EncodedVideoChunk({{
            type: chunkType,
            timestamp,
            duration: frameDurationMicros,
            data: frameBytes,
          }})
        );
      }} catch (error) {{
        console.warn("[Equirust] Native screen-share packet decode failed:", error);
        nativeReport(`packet_decode_failed=${{normalizeErrorMessage(error)}}`, {{ force: true }});
        void forceStopTracks(false, "packet_decode_failed");
      }}
    }};

    const handleAudio = payload => {{
      if (!audioContext || !audioProcessor || audioChannels <= 0) return;
      const bytes = new Uint8Array(payload);
      if (!bytes.length || bytes[0] !== 0x02) return;
      const pcmBytes = bytes.slice(1);
      const pcmBuffer = pcmBytes.buffer.slice(
        pcmBytes.byteOffset,
        pcmBytes.byteOffset + pcmBytes.byteLength
      );
      const samples = new Float32Array(pcmBuffer);
      audioQueue.push(samples);
      if (audioQueue.length > 32) {{
        audioQueue.splice(0, audioQueue.length - 32);
      }}
      if (audioContext.state === "suspended") {{
        void audioContext.resume().catch(() => {{}});
      }}
    }};

    await new Promise((resolve, reject) => {{
      let helloReceived = false;
      const timeout = window.setTimeout(() => {{
        if (helloReceived || closed) return;
        nativeReport("handshake_timeout", {{ force: true }});
        void stopSession(true, "handshake_timeout");
        reject(new Error("Native screen share could not start: stream handshake timed out."));
      }}, 5000);

      socket = new WebSocket(session.websocketUrl);
      socket.binaryType = "arraybuffer";
      socket.onopen = () => {{
        nativeReport("ws_open");
      }};

      socket.onmessage = event => {{
        if (typeof event.data === "string") {{
          let payload = null;
          try {{
            payload = JSON.parse(event.data);
          }} catch {{
            return;
          }}

          if (payload?.type === "hello") {{
            helloReceived = true;
            window.clearTimeout(timeout);
             nativeReport(
              `ws_hello codec=${{String(payload?.video?.codec || videoCodec)}} audio=${{payload?.audio?.enabled === true}}`
            );
            resolve(payload);
            return;
          }}

          if (
            payload?.type === "source_closed" ||
            payload?.type === "fatal" ||
            payload?.type === "ended" ||
            payload?.type === "audio_device_lost"
          ) {{
            const payloadType = String(payload?.type || "unknown");
            const payloadMessage =
              typeof payload?.message === "string" && payload.message.trim()
                ? payload.message.trim()
                : "<none>";
            nativeReport(
              `ws_control type=${{payloadType}} helloReceived=${{helloReceived}} message=${{payloadMessage}}`,
              {{ force: payloadType !== "ended" }}
            );
            window.clearTimeout(timeout);
            if (!helloReceived) {{
              void stopSession(true, `ws_control_before_hello:${{payloadType}}`);
              reject(new Error(payload?.message || "Native screen share could not start."));
              return;
            }}
            void forceStopTracks(false, `ws_control:${{payloadType}}`);
          }}
          return;
        }}

        if (event.data instanceof ArrayBuffer) {{
          const bytes = new Uint8Array(event.data);
          if (!bytes.length) return;
          if (bytes[0] === 0x01) {{
            handleFrame(event.data);
          }} else if (bytes[0] === 0x02) {{
            handleAudio(event.data);
          }}
        }} else if (event.data?.arrayBuffer) {{
          void event.data
            .arrayBuffer()
            .then(buffer => {{
              const bytes = new Uint8Array(buffer);
              if (!bytes.length) return;
              if (bytes[0] === 0x01) {{
                handleFrame(buffer);
              }} else if (bytes[0] === 0x02) {{
                handleAudio(buffer);
              }}
            }})
            .catch(() => {{}});
        }}
      }};

      socket.onerror = event => {{
        nativeReport(
          `ws_error helloReceived=${{helloReceived}} hasEvent=${{event ? "true" : "false"}}`,
          {{ force: true }}
        );
        window.clearTimeout(timeout);
        if (!helloReceived) {{
          void stopSession(true, "ws_error_before_hello");
          reject(new Error("Native screen share could not start: transport connection failed."));
        }} else {{
          void forceStopTracks(false, "ws_error");
        }}
      }};

      socket.onclose = event => {{
        nativeReport(
          `ws_close code=${{Number(event?.code || 0)}} clean=${{event?.wasClean === true}} helloReceived=${{helloReceived}} reason=${{String(event?.reason || "")}} closed=${{closed}}`,
          {{ force: true }}
        );
        window.clearTimeout(timeout);
        if (!helloReceived && !closed) {{
          void stopSession(true, "ws_close_before_hello");
          reject(new Error("Native screen share could not start: transport closed before startup."));
          return;
        }}
        if (!closed) {{
          void forceStopTracks(false, "ws_close");
        }}
      }};
    }});

    return stream;
  }};

  const installDisplayMediaCompatibilityPatches = () => {{
    if (state.displayMediaCompatReady || !isDiscordHost()) return;
    if (typeof navigator.mediaDevices?.getDisplayMedia !== "function") return;

    state.displayMediaCompatReady = true;
    const originalGetDisplayMedia = navigator.mediaDevices.getDisplayMedia.bind(navigator.mediaDevices);

    const normalizeDisplayMediaRequest = options => {{
      const quality = readScreenShareQuality();
      const next = options && typeof options === "object" ? {{ ...options }} : {{}};

      next.systemAudio ??= "include";
      next.surfaceSwitching ??= "include";
      next.monitorTypeSurfaces ??= "include";

      if (next.audio) {{
        if (next.audio === true) {{
          next.audio = {{}};
        }} else if (typeof next.audio !== "object") {{
          next.audio = {{}};
        }} else {{
          next.audio = {{ ...next.audio }};
        }}

        next.audio.suppressLocalAudioPlayback ??= false;
        next.audio.echoCancellation ??= false;
        next.audio.noiseSuppression ??= false;
        next.audio.autoGainControl ??= false;
        next.audio.channelCount ??= 2;
        next.audio.sampleRate ??= 48000;
      }}

      if (next.video !== false) {{
        if (next.video === true || typeof next.video !== "object") {{
          next.video = {{}};
        }} else {{
          next.video = {{ ...next.video }};
        }}

        next.video.frameRate ??= {{ ideal: quality.frameRate, max: quality.frameRate }};
        next.video.width ??= {{ ideal: quality.width, max: quality.width }};
        next.video.height ??= {{ ideal: quality.height, max: quality.height }};
        next.video.resizeMode ??= "none";
      }}

      return next;
    }};

    navigator.mediaDevices.getDisplayMedia = async function(options) {{
      const requestedAudio = options?.audio !== false;
      const sources = await loadScreenShareSources().catch(error => {{
        console.warn("[Equirust] Failed to load capturer sources", error);
        return [];
      }});
      const picked = sources.length
        ? await openScreenSharePicker(sources, {{
            audio: requestedAudio,
          }})
        : null;
      if (!picked) {{
        throw createAbortError("Screen share was cancelled.");
      }}

      const normalized = normalizeDisplayMediaRequest(options);
      const quality = {{
        frameRate: Number(picked.frameRate || 30),
        width: Number(picked.width || 1280),
        height: Number(picked.height || 720),
      }};
      if (!picked.audio) {{
        normalized.audio = false;
        normalized.systemAudio = "exclude";
      }} else {{
        normalized.audio = normalized.audio || {{}};
        normalized.systemAudio = "include";
      }}
      normalized.surfaceSwitching = "exclude";
      normalized.selfBrowserSurface ??= "exclude";
      normalized.monitorTypeSurfaces = picked.kind === "screen" ? "include" : "exclude";
      if (normalized.video && typeof normalized.video === "object") {{
        normalized.video.preferCurrentTab = false;
        normalized.video.logicalSurface ??= picked.kind === "window";
        normalized.video.displaySurface ??= picked.kind === "window" ? "window" : "monitor";
      }}
      report(
        "display_media_request=" +
          JSON.stringify({{
            pickedId: picked.id,
            pickedKind: picked.kind,
            audio: Boolean(normalized.audio),
            frameRate: quality.frameRate,
            width: quality.width,
            height: quality.height,
            systemAudio: normalized.systemAudio ?? null,
            native: supportsNativeWindowsScreenShare(),
          }})
      );

      const stream = supportsNativeWindowsScreenShare()
        ? await startNativeDisplayMediaStream(picked)
        : await originalGetDisplayMedia(normalized);
      const videoTrack = stream.getVideoTracks()[0];

      if (videoTrack) {{
        await applyScreenShareTrackConstraints(videoTrack, quality, picked.contentHint);
      }}
      reinforceScreenShareQuality(stream, quality, picked.contentHint);

      const settings = videoTrack?.getSettings?.() || {{}};
      report(
        "display_media_result=" +
          JSON.stringify({{
            videoTracks: stream.getVideoTracks().length,
            audioTracks: stream.getAudioTracks().length,
            width: settings.width ?? null,
            height: settings.height ?? null,
            frameRate: settings.frameRate ?? null,
            displaySurface: settings.displaySurface ?? null,
          }})
      );

      return stream;
    }};

    report("display_media_compat_installed=true");
  }};

  const installCloudFetchProxy = () => {{
    if (state.cloudFetchProxyInstalled || !isDiscordHost()) return;
    if (typeof window.fetch !== "function" || typeof window.Response !== "function") return;

    const originalFetch = window.fetch.bind(window);
    const proxiedFetch = async function(input, init) {{
      const request = new Request(input, init);
      const cloudOrigin = getConfiguredCloudOrigin();
      const targetUrl = (() => {{
        try {{
          return new URL(request.url, window.location.href);
        }} catch {{
          return null;
        }}
      }})();

      if (
        !cloudOrigin ||
        !targetUrl ||
        targetUrl.origin !== cloudOrigin ||
        targetUrl.origin === window.location.origin
      ) {{
        return originalFetch(input, init);
      }}

      report(`cloud_fetch_proxy_request=${{request.method}} ${{targetUrl.toString()}}`);
      const forwardedHeaderNames = new Set([
        "accept",
        "authorization",
        "content-type",
        "if-none-match",
        "origin",
      ]);
      const headers = Object.fromEntries(
        Array.from(request.headers.entries()).filter(([name]) =>
          forwardedHeaderNames.has(String(name || "").toLowerCase())
        )
      );
      const bodyBuffer =
        request.method === "GET" || request.method === "HEAD"
          ? null
          : await request.clone().arrayBuffer();
      try {{
        const proxied = await invoke("proxy_http_request", {{
          request: {{
            url: targetUrl.toString(),
            method: request.method,
            headers,
            bodyBase64:
              bodyBuffer && bodyBuffer.byteLength
                ? encodeBytesToBase64(new Uint8Array(bodyBuffer))
                : null,
          }},
        }});

        const status = Number(proxied.status || 0);
        const hasNullBodyStatus = new Set([101, 103, 204, 205, 304]).has(status);
        const responseBody =
          request.method === "HEAD" || hasNullBodyStatus
            ? null
            : decodeBase64ToBytes(proxied.bodyBase64);

        report(`cloud_fetch_proxy_response=${{status}} ${{targetUrl.toString()}}`);
        return new Response(responseBody, {{
          status,
          statusText: String(proxied.statusText || ""),
          headers: Array.isArray(proxied.headers) ? proxied.headers : [],
        }});
      }} catch (error) {{
        report(
          `cloud_fetch_proxy_failed=${{error && error.message ? error.message : String(error)}}`,
          {{ force: true }}
        );
        throw error;
      }}
    }};

    window.fetch = proxiedFetch;
    if (typeof globalThis === "object") {{
      globalThis.fetch = proxiedFetch;
    }}

    state.cloudFetchProxyInstalled = true;
    report("cloud_fetch_proxy_installed=true");
  }};

  const isDiscordWindowActive = () => {{
    try {{
      return document.visibilityState === "visible" && !document.hidden && document.hasFocus();
    }} catch {{
      return false;
    }}
  }};

  const appBadgeEnabled = () =>
    getHostSettingValue({{
      key: "appBadge",
      defaultValue: true,
    }});

  const badgeOnlyForMentionsEnabled = () =>
    getHostSettingValue({{
      key: "badgeOnlyForMentions",
      defaultValue: true,
    }});

  const taskbarFlashingEnabled = () =>
    getHostSettingValue({{
      key: "enableTaskbarFlashing",
      defaultValue: false,
    }});

  const parseTitleBadgeCount = () => {{
    const title = typeof document.title === "string" ? document.title.trim() : "";
    const mentionMatch = title.match(/^\((\d+)\)\s+/);
    if (mentionMatch) {{
      return Number(mentionMatch[1]) || 0;
    }}

    if (!badgeOnlyForMentionsEnabled() && /^[•●]\s*/.test(title)) {{
      return -1;
    }}

    return 0;
  }};

  const normalizeAttentionCount = count => {{
    const numeric = Number(count || 0);
    if (!Number.isFinite(numeric) || numeric <= 0) {{
      return numeric < 0 ? 1 : 0;
    }}

    return numeric;
  }};

  const titleMutationTouched = records =>
    records.some(record => {{
      const target = record.target;
      if (target?.nodeName === "TITLE" || target?.parentNode?.nodeName === "TITLE") {{
        return true;
      }}

      return Array.from(record.addedNodes || []).some(node => node?.nodeName === "TITLE") ||
        Array.from(record.removedNodes || []).some(node => node?.nodeName === "TITLE");
    }});

  const syncHostBadgeState = () => {{
    state.notificationSyncQueued = false;
    window.__EQUIRUST_TITLEBAR_SYNC__?.();

    const previousCount = state.lastBadgeCount;
    const nextCount = appBadgeEnabled() ? parseTitleBadgeCount() : 0;
    const previousAttention = normalizeAttentionCount(previousCount);
    const nextAttention = normalizeAttentionCount(nextCount);

    if (previousCount !== nextCount) {{
      state.lastBadgeCount = nextCount;
      invoke("set_badge_count", {{ count: nextCount }}).catch(error => {{
        console.warn("[Equirust]", error);
      }});
    }}

    const focused = isDiscordWindowActive();
    const shouldFlash = taskbarFlashingEnabled() && !focused && nextAttention > 0;
    const shouldStartFlash = shouldFlash && nextAttention > previousAttention;

    if (shouldStartFlash && !state.flashActive) {{
      state.flashActive = true;
      invoke("flash_frame", {{ flag: true }}).catch(error => {{
        console.warn("[Equirust]", error);
      }});
      return;
    }}

    if ((!shouldFlash || focused || nextAttention === 0) && state.flashActive) {{
      state.flashActive = false;
      invoke("flash_frame", {{ flag: false }}).catch(error => {{
        console.warn("[Equirust]", error);
      }});
    }}
  }};

  const scheduleHostBadgeSync = () => {{
    if (!isDiscordHost()) return;
    if (state.notificationSyncQueued) return;
    state.notificationSyncQueued = true;
    window.requestAnimationFrame(syncHostBadgeState);
  }};

  const installNotificationSync = () => {{
    if (state.notificationSyncReady || !isDiscordHost()) return;
    if (!document.head && !document.documentElement) return;

    state.notificationSyncReady = true;
    state.notificationObserver = new MutationObserver(records => {{
      if (!titleMutationTouched(records)) return;
      scheduleHostBadgeSync();
    }});
    state.notificationObserver.observe(document.head || document.documentElement, {{
      subtree: true,
      childList: true,
      characterData: true,
    }});

    window.addEventListener("focus", scheduleHostBadgeSync);
    window.addEventListener("blur", scheduleHostBadgeSync);
    window.addEventListener("pageshow", scheduleHostBadgeSync);
    document.addEventListener("visibilitychange", scheduleHostBadgeSync);
    scheduleHostBadgeSync();
    report("notification_sync_installed=true");
  }};

  const setVoiceTrayCallState = inCall => {{
    if (state.voiceTrayInCall === inCall) return;
    state.voiceTrayInCall = inCall;
    if (!inCall) {{
      state.voiceTrayVariant = null;
    }}

    invoke("set_tray_voice_call_state", {{ inCall }}).catch(error => {{
      console.warn("[Equirust]", error);
    }});
  }};

  const setVoiceTrayVariant = variant => {{
    if (!state.voiceTrayInCall || state.voiceTrayVariant === variant) return;
    state.voiceTrayVariant = variant;
    invoke("set_tray_voice_state", {{ variant }}).catch(error => {{
      console.warn("[Equirust]", error);
    }});
  }};

  const installVoiceTrayWatcher = () => {{
    if (state.voiceTrayReady || !isDiscordHost()) return;

    const FluxDispatcher = window.Vencord?.Webpack?.Common?.FluxDispatcher;
    const MediaEngineStore = window.Vencord?.Webpack?.Common?.MediaEngineStore;
    const UserStore = window.Vencord?.Webpack?.Common?.UserStore;
    const currentUserId = UserStore?.getCurrentUser?.()?.id;

    if (!FluxDispatcher || !MediaEngineStore || !currentUserId) {{
      if (!state.voiceTrayTimer) {{
        state.voiceTrayTimer = window.setInterval(() => {{
          installVoiceTrayWatcher();
          if (state.voiceTrayReady && state.voiceTrayTimer) {{
            window.clearInterval(state.voiceTrayTimer);
            state.voiceTrayTimer = null;
          }}
        }}, 1200);
      }}
      return;
    }}

    const updateIdleVoiceTray = () => {{
      if (!state.voiceTrayInCall) return;

      if (MediaEngineStore.isSelfDeaf?.()) {{
        setVoiceTrayVariant("trayDeafened");
      }} else if (MediaEngineStore.isSelfMute?.()) {{
        setVoiceTrayVariant("trayMuted");
      }} else {{
        setVoiceTrayVariant("trayIdle");
      }}
    }};

    const speakingCallback = params => {{
      if (params?.userId !== currentUserId || params?.context !== "default") return;

      if (params.speakingFlags) {{
        setVoiceTrayVariant("traySpeaking");
      }} else {{
        updateIdleVoiceTray();
      }}
    }};

    const muteCallback = () => {{
      if (state.voiceTrayInCall) {{
        updateIdleVoiceTray();
      }}
    }};

    const rtcCallback = params => {{
      if (params?.context !== "default") return;

      if (params.state === "RTC_CONNECTED") {{
        setVoiceTrayCallState(true);
        updateIdleVoiceTray();
      }} else if (params.state === "RTC_DISCONNECTED") {{
        setVoiceTrayCallState(false);
        scheduleHostBadgeSync();
      }}
    }};

    FluxDispatcher.subscribe("SPEAKING", speakingCallback);
    FluxDispatcher.subscribe("AUDIO_TOGGLE_SELF_DEAF", muteCallback);
    FluxDispatcher.subscribe("AUDIO_TOGGLE_SELF_MUTE", muteCallback);
    FluxDispatcher.subscribe("RTC_CONNECTION_STATE", rtcCallback);

    state.voiceTrayReady = true;
    if (state.voiceTrayTimer) {{
      window.clearInterval(state.voiceTrayTimer);
      state.voiceTrayTimer = null;
    }}

    report("voice_tray_installed=true");
  }};

  const callListeners = listeners => {{
    listeners.forEach(listener => {{
      try {{
        listener();
      }} catch (error) {{
        console.error("[Equirust]", error);
      }}
    }});
  }};

  const resolveVoiceActions = () => window.Vencord?.Webpack?.Common?.VoiceActions;

  const dispatchVoiceToggle = kind => {{
    const listeners =
      kind === "mute" ? state.voiceToggleMuteListeners : state.voiceToggleDeafListeners;

    if (listeners.size > 0) {{
      callListeners(listeners);
      report(`voice_toggle_dispatched=${{kind}} mode=listener`);
      return true;
    }}

    const VoiceActions = resolveVoiceActions();
    if (!VoiceActions) {{
      return false;
    }}

    if (kind === "mute") {{
      VoiceActions.toggleSelfMute?.();
    }} else {{
      VoiceActions.toggleSelfDeaf?.();
    }}

    report(`voice_toggle_dispatched=${{kind}} mode=direct`);
    return true;
  }};

  const flushQueuedVoiceToggles = () => {{
    if (!state.voiceToggleQueue.length) {{
      if (state.voiceToggleRetryTimer) {{
        window.clearInterval(state.voiceToggleRetryTimer);
        state.voiceToggleRetryTimer = null;
      }}
      return;
    }}

    if (!resolveVoiceActions() &&
        state.voiceToggleMuteListeners.size === 0 &&
        state.voiceToggleDeafListeners.size === 0) {{
      return;
    }}

    const pending = [...state.voiceToggleQueue];
    state.voiceToggleQueue = [];

    pending.forEach(kind => {{
      if (!dispatchVoiceToggle(kind)) {{
        state.voiceToggleQueue.push(kind);
      }}
    }});

    if (!state.voiceToggleQueue.length && state.voiceToggleRetryTimer) {{
      window.clearInterval(state.voiceToggleRetryTimer);
      state.voiceToggleRetryTimer = null;
    }}
  }};

  const queueVoiceToggle = kind => {{
    state.voiceToggleQueue.push(kind);
    if (!state.voiceToggleRetryTimer) {{
      state.voiceToggleRetryTimer = window.setInterval(flushQueuedVoiceToggles, 700);
    }}
  }};

  const handleVoiceToggleEvent = kind => {{
    if (!dispatchVoiceToggle(kind)) {{
      queueVoiceToggle(kind);
      report(`voice_toggle_queued=${{kind}}`);
    }}
  }};

  const ensureVoiceToggleBridge = () => {{
    if (state.voiceBridgeReady || !isDiscordHost()) return;

    Promise.all([
      listenTauriEvent("equirust:voice-toggle-mute", () => handleVoiceToggleEvent("mute")),
      listenTauriEvent("equirust:voice-toggle-deafen", () => handleVoiceToggleEvent("deafen")),
    ])
      .then(cleanups => {{
        state.voiceBridgeCleanup = async () => {{
          await Promise.all(cleanups.map(cleanup => cleanup()));
          state.voiceBridgeCleanup = null;
          state.voiceBridgeReady = false;
        }};
        state.voiceBridgeReady = true;
        flushQueuedVoiceToggles();
        report("voice_toggle_bridge_installed=true");
      }})
      .catch(error => {{
        const message = error && error.message ? error.message : String(error);
        report(`voice_toggle_bridge_failed=${{message}}`, {{ force: true }});
      }});
  }};

  const arrpcAppCache = new Map();
  const ARRPC_NULL_CLEAR_GRACE_MS = 900;

  const lookupArrpcAsset = async (applicationId, key) => {{
    try {{
      const assetUtils = window.Vencord?.Webpack?.Common?.ApplicationAssetUtils;
      if (!assetUtils?.fetchAssetIds) return undefined;
      const assets = await assetUtils.fetchAssetIds(applicationId, [key]);
      return assets?.[0];
    }} catch (error) {{
      console.warn("[Equirust] Failed to resolve arRPC asset", error);
      return undefined;
    }}
  }};

  const lookupArrpcApplication = async applicationId => {{
    if (!applicationId) return undefined;
    if (arrpcAppCache.has(applicationId)) {{
      const cached = arrpcAppCache.get(applicationId);
      arrpcAppCache.delete(applicationId);
      arrpcAppCache.set(applicationId, cached);
      return cached;
    }}

    try {{
      const fetchApplicationsRPC = window.Vencord?.Webpack?.Common?.fetchApplicationsRPC;
      if (typeof fetchApplicationsRPC !== "function") return undefined;
      const socket = {{}};
      await fetchApplicationsRPC(socket, applicationId);
      if (socket.application) {{
        if (arrpcAppCache.size >= 50) {{
          const firstKey = arrpcAppCache.keys().next().value;
          if (firstKey) arrpcAppCache.delete(firstKey);
        }}
        arrpcAppCache.set(applicationId, socket.application);
        return socket.application;
      }}
    }} catch (error) {{
      console.warn("[Equirust] Failed to resolve arRPC application", error);
    }}

    return undefined;
  }};

  const sameArrpcRunningGame = (left, right) => {{
    if (!left && !right) return true;
    if (!left || !right) return false;
    return (
      left.socketId === right.socketId &&
      left.applicationId === right.applicationId &&
      left.pid === right.pid &&
      left.name === right.name &&
      left.startTime === right.startTime
    );
  }};

  const buildArrpcRunningGame = payload => {{
    const activity = payload?.activity;
    if (!activity || typeof activity !== "object") {{
      return null;
    }}

    const socketId =
      typeof payload?.socketId === "string" && payload.socketId.trim()
        ? payload.socketId.trim()
        : null;
    const applicationId =
      typeof activity?.application_id === "string" && activity.application_id.trim()
        ? activity.application_id.trim()
        : socketId;
    const name =
      typeof payload?.name === "string" && payload.name.trim()
        ? payload.name.trim()
        : typeof activity?.name === "string" && activity.name.trim()
          ? activity.name.trim()
          : applicationId;

    if (!socketId && !applicationId) {{
      return null;
    }}

    const pid =
      typeof payload?.pid === "number" && Number.isFinite(payload.pid) ? payload.pid : 0;
    const startTime =
      typeof activity?.timestamps?.start === "number" && Number.isFinite(activity.timestamps.start)
        ? activity.timestamps.start
        : null;

    return {{
      id: socketId || applicationId,
      pid,
      socketId,
      name,
      applicationId,
      application_id: applicationId,
      start: startTime,
      startTime,
      hidden: false,
      isLauncher: false,
      type: 0,
    }};
  }};

  const syncArrpcRunningGames = (dispatcher, nextRunningGame) => {{
    const previousRunningGame = state.arrpcLastRunningGame;
    if (sameArrpcRunningGame(previousRunningGame, nextRunningGame)) {{
      return;
    }}

    dispatcher.dispatch({{
      type: "RUNNING_GAMES_CHANGE",
      removed: previousRunningGame ? [previousRunningGame] : [],
      added: nextRunningGame ? [nextRunningGame] : [],
      games: nextRunningGame ? [nextRunningGame] : [],
    }});

    state.arrpcLastRunningGame = nextRunningGame;
  }};

  const dispatchArrpcPayload = async payload => {{
    const common = window.Vencord?.Webpack?.Common;
    const dispatcher = common?.FluxDispatcher;
    if (!dispatcher?.dispatch) {{
      return false;
    }}

    let normalizedPayload =
      payload && typeof payload === "object" ? {{ ...payload }} : {{ activity: null }};
    const bypassNullGrace = normalizedPayload?.__equirustBypassClearGrace === true;
    if (normalizedPayload && typeof normalizedPayload === "object") {{
      delete normalizedPayload.__equirustBypassClearGrace;
    }}
    let activity =
      normalizedPayload && typeof normalizedPayload === "object"
        ? normalizedPayload.activity
        : null;
    const hasActivity = activity && typeof activity === "object";

    if (hasActivity) {{
      state.arrpcLastNonNullAtMs = Date.now();
      if (state.arrpcNullClearTimer) {{
        window.clearTimeout(state.arrpcNullClearTimer);
        state.arrpcNullClearTimer = null;
      }}
    }} else {{
      const synthesized = synthesizeArrpcPayloadFromStatus(state.arrpcStatus);
      if (synthesized?.activity && typeof synthesized.activity === "object") {{
        normalizedPayload = synthesized;
        activity = normalizedPayload.activity;
      }} else if (!bypassNullGrace && state.arrpcLastSocketId) {{
        if (!state.arrpcNullClearTimer) {{
          state.arrpcNullClearTimer = window.setTimeout(() => {{
            state.arrpcNullClearTimer = null;
            dispatchArrpcPayload({{
              socketId: state.arrpcLastSocketId,
              activity: null,
              __equirustBypassClearGrace: true,
            }}).catch(error => {{
              console.error("[Equirust]", error);
            }});
          }}, ARRPC_NULL_CLEAR_GRACE_MS);
        }}
        return true;
      }}
    }}

    if (
      normalizedPayload?.socketId === "STREAMERMODE" ||
      activity?.application_id === "STREAMERMODE"
    ) {{
      const streamerModeStore = common?.StreamerModeStore;
      if (streamerModeStore?.autoToggle) {{
        dispatcher.dispatch({{
          type: "STREAMER_MODE_UPDATE",
          key: "enabled",
          value: activity != null,
        }});
      }}
      return true;
    }}

    if (activity && typeof activity === "object") {{
      if (
        typeof normalizedPayload.socketId === "string" &&
        normalizedPayload.socketId.trim()
      ) {{
        state.arrpcLastSocketId = normalizedPayload.socketId.trim();
      }}

      if (activity.assets?.large_image) {{
        activity.assets.large_image = await lookupArrpcAsset(
          activity.application_id,
          activity.assets.large_image
        );
      }}
      if (activity.assets?.small_image) {{
        activity.assets.small_image = await lookupArrpcAsset(
          activity.application_id,
          activity.assets.small_image
        );
      }}

      const application = await lookupArrpcApplication(activity.application_id);
      if (application?.name && !activity.name) {{
        activity.name = application.name;
      }}
    }} else {{
      const rememberedSocketId =
        typeof state.arrpcLastSocketId === "string" && state.arrpcLastSocketId.trim()
          ? state.arrpcLastSocketId.trim()
          : null;
      if (
        rememberedSocketId &&
        !(typeof normalizedPayload.socketId === "string" && normalizedPayload.socketId.trim())
      ) {{
        normalizedPayload.socketId = rememberedSocketId;
      }}
    }}

    syncArrpcRunningGames(dispatcher, buildArrpcRunningGame(normalizedPayload));

    dispatcher.dispatch({{
      type: "LOCAL_ACTIVITY_UPDATE",
      ...normalizedPayload,
    }});

    if (!(activity && typeof activity === "object")) {{
      state.arrpcLastSocketId = null;
    }}

    return true;
  }};

  const synthesizeArrpcPayloadFromStatus = status => {{
    const summary = Array.isArray(status?.activities) ? status.activities[0] : null;
    if (!summary?.applicationId && !summary?.socketId) {{
      return {{ activity: null }};
    }}

    const activity = {{
      application_id: summary.applicationId || summary.socketId,
      type: 0,
    }};

    if (summary.name) {{
      activity.name = summary.name;
    }}

    if (summary.startTime) {{
      activity.timestamps = {{ start: summary.startTime }};
    }}

    const payload = {{ activity }};
    if (summary.socketId) {{
      payload.socketId = summary.socketId;
    }}
    if (summary.pid) {{
      payload.pid = summary.pid;
    }}
    if (summary.name) {{
      payload.name = summary.name;
    }}

    return payload;
  }};

  const flushPendingArrpcPayloads = async () => {{
    if (!state.arrpcPendingPayloads.length) return;
    const pending = [...state.arrpcPendingPayloads];
    state.arrpcPendingPayloads.length = 0;
    for (const payload of pending) {{
      const handled = await dispatchArrpcPayload(payload);
      if (!handled) {{
        state.arrpcPendingPayloads.unshift(payload);
        break;
      }}
    }}
  }};

  const handleArrpcPayload = payload => {{
    state.arrpcActivityListeners.forEach(listener => {{
      try {{
        listener(payload);
      }} catch (error) {{
        console.error("[Equirust]", error);
      }}
    }});

    Promise.resolve(dispatchArrpcPayload(payload))
      .then(handled => {{
        if (!handled) {{
          state.arrpcPendingPayloads.push(payload);
          window.setTimeout(() => {{
            flushPendingArrpcPayloads().catch(error => {{
              console.error("[Equirust]", error);
            }});
          }}, 1200);
        }}
      }})
      .catch(error => {{
        console.error("[Equirust]", error);
      }});
  }};

  const ensureArrpcBridge = () => {{
    if (state.arrpcBridgeReady || state.arrpcBridgeCleanup || !isDiscordHost()) return;

    Promise.all([
      listenTauriEvent("equirust:arrpc-activity", payload => {{
        const nextPayload =
          payload?.payload && typeof payload.payload === "object" ? payload.payload : payload;
        handleArrpcPayload(nextPayload);
      }}),
      listenTauriEvent("equirust:arrpc-status", payload => {{
        state.arrpcStatus =
          payload?.payload && typeof payload.payload === "object" ? payload.payload : payload;
        window.__EQUIRUST_SETTINGS_SYNC__?.();
      }}),
    ])
      .then(cleanups => {{
        state.arrpcBridgeCleanup = async () => {{
          await Promise.all(cleanups.map(cleanup => cleanup()));
          state.arrpcBridgeCleanup = null;
          state.arrpcBridgeReady = false;
        }};
        state.arrpcBridgeReady = true;
        Promise.allSettled([
          refreshArRPCStatus(),
          invoke("get_arrpc_current_activity"),
        ])
          .then(results => {{
            const statusResult = results[0];
            const payloadResult = results[1];
            const status =
              statusResult?.status === "fulfilled" ? statusResult.value : state.arrpcStatus;
            const currentPayload =
              payloadResult?.status === "fulfilled" ? payloadResult.value : null;
            const replayPayload =
              currentPayload && typeof currentPayload === "object"
                ? currentPayload
                : synthesizeArrpcPayloadFromStatus(status);

            handleArrpcPayload(replayPayload);
            return flushPendingArrpcPayloads();
          }})
          .catch(error => {{
            console.error("[Equirust]", error);
          }});
        report("arrpc_bridge_installed=true");
      }})
      .catch(error => {{
        const message = error && error.message ? error.message : String(error);
        report(`arrpc_bridge_failed=${{message}}`, {{ force: true }});
      }});
  }};

  const createVencordNative = () => {{
    if (window.VencordNative) return;

    window.VencordNative = {{
      app: {{
        relaunch: () => invoke("app_relaunch"),
        getVersion: () => String(state.versions.equirust || "1.0.0"),
        getGitHash: () => String(state.versions.gitHash || "unknown"),
        isDevBuild: () => state.debugBuild === true,
        setBadgeCount: count => invoke("set_badge_count", {{ count }}),
        supportsWindowsTransparency: () => supportsWindowsTransparency(),
        getEnableHardwareAcceleration: () =>
          state.hostSettings?.hardwareAcceleration !== false,
        isOutdated: async () => {{
          const status = await getHostUpdateStatusCached(false);
          return status?.updateAvailable === true;
        }},
        openUpdater: () => openHostUpdate(),
        getPlatformSpoofInfo: () => ({{
          spoofed: false,
          originalPlatform: normalizeLegacyPlatform(state.versions.platform),
          spoofedPlatform: null,
        }}),
        getRendererCss: () => refreshRendererCss(),
        onRendererCssUpdate: listener => addRendererCssListener(listener),
      }},
      autostart: {{
        isEnabled: () => refreshNativeAutoStart(),
        enable: () => persistNativeAutoStart(true),
        disable: () => persistNativeAutoStart(false),
      }},
      arrpc: {{
        onActivity(listener) {{
          if (typeof listener !== "function") return;
          state.arrpcActivityListeners.add(listener);
          ensureArrpcBridge();
        }},
        offActivity(listener) {{
          state.arrpcActivityListeners.delete(listener);
        }},
        getStatus: () => refreshArRPCStatus(),
        restart: () => restartArRPC(),
        openSettings: () => focusEquirustSettingsSection("rich-presence"),
      }},
      themes: {{
        uploadTheme: async () => {{
          const result = await invoke("upload_vencord_theme");
          if (result === "ok") {{
            notifyThemeListeners();
          }}
          return result;
        }},
        deleteTheme: async fileName => {{
          const result = await invoke("delete_vencord_theme", {{ fileName }});
          if (result === "ok" || result === "missing") {{
            notifyThemeListeners();
          }}
          return result;
        }},
        getThemesList: () => invoke("get_vencord_themes_list"),
        getThemeData: fileName => invoke("get_vencord_theme_data", {{ fileName }}),
        getSystemValues: () => getSystemThemeValues(),
        openFolder: () => invoke("open_vencord_themes_folder"),
      }},
      updater: {{
        getUpdates: async () =>
          wrapIpcResult(async () => {{
            const status = await getRuntimeUpdateStatusCached(true);
            return summarizeRuntimeUpdateEntries(status);
          }}),
        update: async () =>
          wrapIpcResult(async () => {{
            const status = await getRuntimeUpdateStatusCached(true);
            if (status?.updateAvailable !== true) {{
              return false;
            }}

            await openRuntimeUpdate();
            return true;
          }}),
        rebuild: async () =>
          wrapIpcResult(async () => {{
            await openRuntimeUpdate();
            return false;
          }}),
        build: async () =>
          wrapIpcResult(async () => {{
            await openRuntimeUpdate();
            return false;
          }}),
        getRepo: async () => wrapIpcResult(async () => getRuntimeUpdateRepo()),
      }},
      settings: {{
        get: () => state.settings,
        set: (settings, path) => safeCall(async () => {{
          state.settings = settings || {{}};
          await invoke("set_vencord_settings", {{ settings, path }});
          state.themeListeners.forEach(listener => {{
            try {{ listener(); }} catch (error) {{ console.error("[Equirust]", error); }}
          }});
        }}),
        openFolder: () => invoke("open_vencord_settings_folder"),
      }},
      spellcheck: {{
        getAvailableLanguages: () => getSpellcheckLanguages(),
        onSpellcheckResult(listener) {{
          if (typeof listener !== "function") return;
          state.spellcheckResultListeners.add(listener);
          installSpellcheckBridge();
        }},
        offSpellcheckResult(listener) {{
          state.spellcheckResultListeners.delete(listener);
        }},
        replaceMisspelling: word => replaceSpellcheckSelection(word),
        addToDictionary: word => addSpellcheckWordToDictionary(word),
      }},
      capturer: {{
        getSources: () => invoke("get_capturer_sources"),
        getLargeThumbnail: id => invoke("get_capturer_large_thumbnail", {{ id }}),
      }},
      virtmic: {{
        list: () => invoke("virtmic_list"),
        start: include => invoke("virtmic_start", {{ include }}),
        startSystem: exclude => invoke("virtmic_start_system", {{ exclude }}),
        stop: () => invoke("virtmic_stop"),
      }},
      quickCss: {{
        get: async () => state.quickCss,
        set: css => safeCall(async () => {{
          state.quickCss = typeof css === "string" ? css : "";
          await invoke("set_vencord_quick_css", {{ css: state.quickCss }});
          notifyQuickCssListeners();
        }}),
        addChangeListener(listener) {{
          state.quickCssListeners.add(listener);
          ensureVencordFileWatch();
        }},
        addThemeChangeListener(listener) {{
          state.themeListeners.add(listener);
          ensureVencordFileWatch();
        }},
        openFile: () => invoke("open_vencord_quick_css"),
        openEditor: () => invoke("open_vencord_quick_css"),
        getEditorTheme: () =>
          window.matchMedia("(prefers-color-scheme: dark)").matches ? "vs-dark" : "vs-light",
      }},
      fileManager: {{
        getState: () => invoke("get_file_manager_state"),
        isUsingCustomVencordDir: async () =>
          Boolean((await invoke("get_file_manager_state"))?.usingCustomVencordDir),
        showCustomVencordDir: () => invoke("show_custom_vencord_dir"),
        selectEquicordDir: reset => invoke("select_vencord_dir", {{ reset: reset === null || reset === true }}),
        chooseUserAsset: (asset, reset) =>
          invoke("choose_user_asset", {{ asset, reset: reset === null || reset === true }}),
        openUserAssetsFolder: () => invoke("open_user_assets_folder"),
      }},
      clipboard: {{
        copyImage: (imageBuffer, _imageSrc) =>
          invoke("copy_image_to_clipboard", {{
            bytes: Array.isArray(imageBuffer) ? imageBuffer : Array.from(imageBuffer || []),
          }}),
      }},
      win: {{
        focus: () => invoke("window_focus"),
        close: () => invoke("window_close"),
        minimize: () => invoke("window_minimize"),
        maximize: () => invoke("window_toggle_maximize"),
        flashFrame: flag => invoke("flash_frame", {{ flag }}),
        setDevtoolsCallbacks: () => {{}},
      }},
      tray: {{
        setVoiceState: variant => invoke("set_tray_voice_state", {{ variant }}),
        setVoiceCallState: inCall => invoke("set_tray_voice_call_state", {{ inCall }}),
      }},
      voice: {{
        onToggleSelfMute(listener) {{
          if (typeof listener !== "function") return;
          state.voiceToggleMuteListeners.add(listener);
          ensureVoiceToggleBridge();
        }},
        offToggleSelfMute(listener) {{
          state.voiceToggleMuteListeners.delete(listener);
        }},
        onToggleSelfDeaf(listener) {{
          if (typeof listener !== "function") return;
          state.voiceToggleDeafListeners.add(listener);
          ensureVoiceToggleBridge();
        }},
        offToggleSelfDeaf(listener) {{
          state.voiceToggleDeafListeners.delete(listener);
        }},
      }},
      debug: {{
        launchGpu: () => invoke("open_debug_page", {{ target: "gpu" }}),
        launchWebrtcInternals: () => invoke("open_debug_page", {{ target: "webrtc-internals" }}),
      }},
      commands: {{
        onCommand(callback) {{
          if (typeof callback !== "function") return;
          state.commandListeners.add(callback);
          ensureRendererCommandBridge();
        }},
        offCommand(callback) {{
          state.commandListeners.delete(callback);
          releaseRendererCommandBridge();
        }},
        respond: response =>
          invoke("respond_renderer_command", {{
            nonce: String(response?.nonce || ""),
            ok: response?.ok !== false,
            data: Object.prototype.hasOwnProperty.call(response || {{}}, "data")
              ? response.data
              : null,
          }}),
      }},
      native: {{
        getVersions: () => state.versions,
        openExternal: url => invoke("open_external_link", {{ url }}),
        getRendererCss: () => refreshRendererCss(),
        onRendererCssUpdate: listener => addRendererCssListener(listener),
      }},
      csp: {{
        isDomainAllowed: (url, directives) =>
          invoke("csp_is_domain_allowed", {{ url, directives }}),
        removeOverride: url =>
          invoke("csp_remove_override", {{ url }}),
        requestAddOverride: (url, directives, reason) =>
          invoke("csp_request_add_override", {{ url, directives, reason }}),
      }},
      pluginHelpers: {{}},
    }};
  }};

  const installVencordRuntime = () => {{
    if (!isDiscordHost() || !state.vencordRenderer || window.__EQUIRUST_VENCORD_LOADED__) return;
    window.__EQUIRUST_VENCORD_LOADED__ = true;

    try {{
      window.eval(`${{state.vencordRenderer}}\n;window.Vencord = typeof Vencord !== "undefined" ? Vencord : window.Vencord;`);
      window.setTimeout(() => {{
        const styleCount = window.VencordStyles instanceof Map ? window.VencordStyles.size : 0;
        const hasRoot = Boolean(document.querySelector("vencord-root"));
        const hasNative = typeof window.VencordNative?.settings?.get === "function";
        const hasVesktop = typeof window.VesktopNative !== "undefined";
        const hasSettingsPlugin = Boolean(window.Vencord?.Plugins?.plugins?.Settings);
        const hasSettingsApi = Boolean(window.Vencord?.Api?.Settings);
        const vencordKeys = Object.keys(window.Vencord || {{}}).join(",");
        const pluginKeys = Object.keys(window.Vencord?.Plugins || {{}}).join(",");
        report(`vencord_root=${{hasRoot}} style_count=${{styleCount}} native_bridge=${{hasNative}} vesktop_bridge=${{hasVesktop}} settings_plugin=${{hasSettingsPlugin}} settings_api=${{hasSettingsApi}} vencord_keys=${{vencordKeys}} plugin_keys=${{pluginKeys}} host=${{window.location.hostname}} ua=${{navigator.userAgent}}`);
      }}, 1600);
    }} catch (error) {{
      const message = error && error.message ? error.message : String(error);
      report(
        `vencord_load_failed=${{message}} host=${{window.location.hostname}}`,
        {{ force: true }}
      );
      throw error;
    }}
  }};

  const installTitlebar = () => {{
    if (state.titlebarReady || !document.body) return;
    state.titlebarReady = true;

    const style = document.createElement("style");
    style.id = "equirust-titlebar-style";
    style.textContent = `
      :root {{
        --equirust-titlebar-height: 28px;
        --equirust-titlebar-bg:
          var(
            --background-secondary-alt,
            var(--background-secondary, var(--background-primary, #0b0d12))
          );
        --equirust-titlebar-border:
          var(--border-subtle, var(--background-modifier-accent, rgba(255, 255, 255, 0.08)));
        --equirust-titlebar-fg:
          var(--header-primary, var(--text-normal, #dbdee1));
        --equirust-titlebar-fg-muted:
          var(--header-secondary, var(--text-muted, #b5bac1));
        --equirust-titlebar-hover:
          var(--background-modifier-hover, rgba(255, 255, 255, 0.08));
        --equirust-titlebar-active:
          var(--background-modifier-active, rgba(255, 255, 255, 0.12));
        --equirust-titlebar-danger:
          var(--button-danger-background, var(--status-danger, #da373c));
        --equirust-titlebar-danger-hover:
          var(--button-danger-background-hover, var(--status-danger, #c53030));
      }}
      html.equirust-chrome body {{
        padding-top: var(--equirust-titlebar-height) !important;
        box-sizing: border-box;
      }}
      html.equirust-chrome #app-mount,
      html.equirust-chrome [class*="appMount"] {{
        min-height: calc(100vh - var(--equirust-titlebar-height)) !important;
      }}
      #equirust-titlebar {{
        position: fixed;
        inset: 0 0 auto 0;
        height: var(--equirust-titlebar-height);
        display: grid;
        grid-template-columns: auto minmax(0, 1fr) auto;
        align-items: stretch;
        background: var(--equirust-titlebar-bg);
        border-bottom: 1px solid var(--equirust-titlebar-border);
        color: var(--equirust-titlebar-fg);
        z-index: 2147483646;
        user-select: none;
        overflow: hidden;
        isolation: isolate;
        transition: background-color 150ms ease, border-color 150ms ease, color 150ms ease;
        box-shadow: none;
        pointer-events: none;
      }}
      .equirust-titlebar__cluster {{
        display: inline-flex;
        align-items: center;
        min-width: 0;
        pointer-events: none;
        position: relative;
        z-index: 2;
      }}
      .equirust-titlebar__left {{
        gap: 0;
        padding-left: 5px;
      }}
      .equirust-titlebar__drag {{
        display: flex;
        align-items: center;
        justify-content: center;
        min-width: 0;
        padding: 0 10px;
        overflow: hidden;
        pointer-events: auto;
        position: relative;
        z-index: 1;
      }}
      .equirust-titlebar__identity {{
        display: inline-flex;
        align-items: center;
        gap: 7px;
        min-width: 0;
        max-width: 100%;
        pointer-events: none;
      }}
      .equirust-titlebar__icon {{
        width: 15px;
        height: 15px;
        border-radius: 999px;
        overflow: hidden;
        flex: 0 0 auto;
        align-items: center;
        justify-content: center;
        display: none;
        color: var(--equirust-titlebar-fg-muted);
        background: var(--equirust-titlebar-hover);
        transition: background-color 150ms ease, color 150ms ease;
      }}
      .equirust-titlebar__icon[data-visible="true"] {{
        display: inline-flex;
      }}
      .equirust-titlebar__icon[data-variant="dm"] {{
        background: transparent;
        color: var(--equirust-titlebar-fg);
      }}
      .equirust-titlebar__icon img,
      .equirust-titlebar__icon svg {{
        width: 100%;
        height: 100%;
        display: block;
      }}
      .equirust-titlebar__icon[data-variant="guild-text"] {{
        font-family: "Segoe UI Variable Text", "Segoe UI", sans-serif;
        font-size: 9px;
        font-weight: 700;
        letter-spacing: 0.01em;
        text-transform: uppercase;
        color: var(--equirust-titlebar-fg);
      }}
      .equirust-titlebar__label {{
        font-family: "Segoe UI Variable Text", "Segoe UI", sans-serif;
        font-size: 12px;
        font-weight: 600;
        line-height: 1.15;
        color: var(--equirust-titlebar-fg);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        pointer-events: none;
        text-rendering: optimizeLegibility;
        transition: color 150ms ease;
      }}
      .equirust-titlebar__controls {{
        justify-content: flex-end;
        gap: 0;
      }}
      .equirust-titlebar__button {{
        width: 30px;
        height: 100%;
        border: 0;
        background: transparent;
        color: inherit;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        cursor: pointer;
        touch-action: manipulation;
        transition: background-color 120ms ease, color 120ms ease;
        pointer-events: auto;
      }}
      .equirust-titlebar__button * {{
        pointer-events: none;
      }}
      .equirust-titlebar__button:hover {{
        background: var(--equirust-titlebar-hover);
      }}
      .equirust-titlebar__button:active {{
        background: var(--equirust-titlebar-active);
      }}
      .equirust-titlebar__button--close:hover {{
        background: var(--equirust-titlebar-danger-hover);
        color: white;
      }}
      .equirust-titlebar__button--close:active {{
        background: var(--equirust-titlebar-danger);
      }}
      .equirust-titlebar__button svg {{
        width: 11px;
        height: 11px;
        opacity: 0.96;
      }}
      .equirust-titlebar__button--nav {{
        width: 22px;
        height: 100%;
        margin: 0 1px;
        border-radius: 5px;
        color: var(--equirust-titlebar-fg-muted);
      }}
      .equirust-titlebar__button--nav:hover {{
        color: var(--equirust-titlebar-fg);
      }}
      .equirust-titlebar__button--utility {{
        width: 28px;
        color: var(--equirust-titlebar-fg-muted);
      }}
      .equirust-titlebar__button--utility svg {{
        width: 13px;
        height: 13px;
        opacity: 0.98;
      }}
      .equirust-titlebar__button--utility:hover {{
        color: var(--equirust-titlebar-fg);
      }}
      .equirust-titlebar__button--utility:disabled {{
        opacity: 0.42;
        cursor: default;
      }}
      .equirust-titlebar__button--utility:disabled:hover {{
        background: transparent;
        color: var(--equirust-titlebar-fg-muted);
      }}
      .equirust-titlebar__divider {{
        width: 1px;
        height: 12px;
        margin: 0 3px 0 1px;
        background: var(--equirust-titlebar-border);
        flex: 0 0 auto;
      }}
      .equirust-resize {{
        position: fixed;
        z-index: 2147483647;
        background: transparent;
      }}
      .equirust-resize--n,
      .equirust-resize--s {{
        left: 10px;
        right: 10px;
        height: 4px;
      }}
      .equirust-resize--n {{
        top: 0;
        cursor: n-resize;
      }}
      .equirust-resize--s {{
        bottom: 0;
        cursor: s-resize;
      }}
      .equirust-resize--e,
      .equirust-resize--w {{
        top: 10px;
        bottom: 10px;
        width: 4px;
      }}
      .equirust-resize--e {{
        right: 0;
        cursor: e-resize;
      }}
      .equirust-resize--w {{
        left: 0;
        cursor: w-resize;
      }}
      .equirust-resize--ne,
      .equirust-resize--nw,
      .equirust-resize--se,
      .equirust-resize--sw {{
        width: 10px;
        height: 10px;
      }}
      .equirust-resize--ne {{
        top: 0;
        right: 0;
        cursor: ne-resize;
      }}
      .equirust-resize--nw {{
        top: 0;
        left: 0;
        cursor: nw-resize;
      }}
      .equirust-resize--se {{
        right: 0;
        bottom: 0;
        cursor: se-resize;
      }}
      .equirust-resize--sw {{
        left: 0;
        bottom: 0;
        cursor: sw-resize;
      }}
      .equirust-typing-host {{
        display: inline-flex !important;
        align-items: center;
      }}
      .equirust-typing-host [class*="dot"],
      .equirust-typing-host [class*="dots"] {{
        animation: none !important;
        opacity: 0 !important;
        width: 0 !important;
        margin: 0 !important;
        overflow: hidden !important;
      }}
      .equirust-typing-bloom {{
        width: 30px;
        height: 8px;
        margin-left: 8px;
        border-radius: 999px;
        position: relative;
        overflow: hidden;
        background: rgba(88, 101, 242, 0.16);
        box-shadow:
          inset 0 0 0 1px rgba(255, 255, 255, 0.08),
          0 0 10px rgba(88, 101, 242, 0.18);
        flex: 0 0 auto;
      }}
      .equirust-typing-bloom::before {{
        content: "";
        position: absolute;
        inset: 1px;
        border-radius: inherit;
        background:
          linear-gradient(90deg, rgba(88,101,242,0.04), rgba(88,101,242,0.92) 36%, rgba(88,214,255,0.95) 68%, rgba(114,240,164,0.9));
        transform-origin: left center;
        animation: equirust-typing-bloom 1.45s cubic-bezier(0.4, 0, 0.2, 1) infinite;
      }}
      @keyframes equirust-typing-bloom {{
        0% {{
          transform: translateX(-76%) scaleX(0.38);
          opacity: 0.16;
        }}
        42% {{
          transform: translateX(-4%) scaleX(0.96);
          opacity: 1;
        }}
        100% {{
          transform: translateX(88%) scaleX(0.42);
          opacity: 0.14;
        }}
      }}
    `;
    document.documentElement.appendChild(style);
    document.documentElement.classList.add("equirust-chrome");

    const titlebar = document.createElement("div");
    titlebar.id = "equirust-titlebar";
    titlebar.innerHTML = `
      <div class="equirust-titlebar__cluster equirust-titlebar__left">
        <button class="equirust-titlebar__button equirust-titlebar__button--nav" type="button" data-action="back" aria-label="Back">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <path d="M15 18l-6-6 6-6"></path>
          </svg>
        </button>
        <button class="equirust-titlebar__button equirust-titlebar__button--nav" type="button" data-action="forward" aria-label="Forward">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <path d="M9 18l6-6-6-6"></path>
          </svg>
        </button>
      </div>
      <div class="equirust-titlebar__drag">
        <span class="equirust-titlebar__identity">
          <span class="equirust-titlebar__icon" data-visible="false" aria-hidden="true"></span>
          <span class="equirust-titlebar__label">Equirust</span>
        </span>
      </div>
      <div class="equirust-titlebar__cluster equirust-titlebar__controls">
        <button class="equirust-titlebar__button equirust-titlebar__button--utility" type="button" data-action="inbox" aria-label="Inbox">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4.75 6.75h14.5v10.5H4.75z"></path>
            <path d="M8 11.75h2.3l1.2 2h1l1.2-2H16"></path>
          </svg>
        </button>
        <button class="equirust-titlebar__button equirust-titlebar__button--utility" type="button" data-action="help" aria-label="Help">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="8.25"></circle>
            <path d="M9.75 9.35a2.42 2.42 0 0 1 4.61 1.03c0 1.5-1.37 2.07-2.1 2.61-.53.39-.76.7-.76 1.26"></path>
            <circle cx="12" cy="16.9" r="0.55" fill="currentColor" stroke="none"></circle>
          </svg>
        </button>
        <span class="equirust-titlebar__divider" aria-hidden="true"></span>
        <button class="equirust-titlebar__button" type="button" data-action="minimize" aria-label="Minimize">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <path d="M5 12h14"></path>
          </svg>
        </button>
        <button class="equirust-titlebar__button" type="button" data-action="maximize" aria-label="Maximize">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <rect x="6" y="6" width="12" height="12"></rect>
          </svg>
        </button>
        <button class="equirust-titlebar__button equirust-titlebar__button--close" type="button" data-action="close" aria-label="Close">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <path d="M6 6l12 12M18 6L6 18"></path>
          </svg>
        </button>
      </div>
    `;
    document.body.prepend(titlebar);

    const label = titlebar.querySelector(".equirust-titlebar__label");
    const labelIcon = titlebar.querySelector(".equirust-titlebar__icon");
    const inboxButton = titlebar.querySelector('[data-action="inbox"]');
    const helpButton = titlebar.querySelector('[data-action="help"]');
    const maximizeButton = titlebar.querySelector('[data-action="maximize"]');
    const dragRegion = titlebar.querySelector(".equirust-titlebar__drag");
    let pendingDragPointer = null;

    const sanitizeWindowTitle = value => {{
      const currentTitle = value && String(value).trim().length
        ? String(value).trim()
        : (isDiscordHost() ? "Discord" : "Equirust");
      return currentTitle.replace(/\s+\|\s+Discord$/i, "").replace(/^Discord\s+\|\s+/i, "");
    }};

    const getRouteParts = () =>
      window.location.pathname
        .split("/")
        .map(part => part.trim())
        .filter(Boolean);

    const getCurrentGuildId = () => {{
      const parts = getRouteParts();
      if (parts[0] !== "channels") return null;
      if (!parts[1] || parts[1] === "@me") return null;
      return parts[1];
    }};

    const isDirectMessagesRoute = () => {{
      const parts = getRouteParts();
      return parts[0] === "channels" && parts[1] === "@me";
    }};

    const getInitials = value =>
      String(value || "")
        .split(/\s+/)
        .map(part => part.trim()[0] || "")
        .join("")
        .slice(0, 2)
        .toUpperCase();

    const normalizeGuildLabel = value =>
      sanitizeWindowTitle(value)
        .replace(/\s*,\s*(server|guild)\s*(menu|actions|dropdown).*$/i, "")
        .replace(/\s*\((server|guild)\s*(menu|actions|dropdown)\)\s*$/i, "")
        .trim();

    const findVisibleElement = selectors =>
      selectors
        .flatMap(selector => Array.from(document.querySelectorAll(selector)))
        .find(target => {{
          if (!(target instanceof HTMLElement)) return false;
          if (target.closest("#equirust-titlebar")) return false;

          const rect = target.getBoundingClientRect();
          return rect.width > 0 && rect.height > 0;
        }});

    const resolveGuildIdentity = () => {{
      const guildId = getCurrentGuildId();
      if (!guildId) return null;

      const guildStore = window.Vencord?.Webpack?.Common?.GuildStore;
      const guild = guildStore?.getGuild?.(guildId) || null;

      const guildNavTarget = findVisibleElement([
        `[data-list-item-id="guildsnav___${{guildId}}"]`,
        `[data-list-item-id^="guildsnav___${{guildId}}"]`,
        `nav [href="/channels/${{guildId}}"]`,
        `nav [href="/channels/${{guildId}}/"]`,
      ]);
      const guildMenuTarget = findVisibleElement([
        'button[aria-label*=", server menu"]',
        'button[aria-label*=", server actions"]',
        'button[aria-label*=", server dropdown"]',
      ]);

      const target = guildNavTarget || guildMenuTarget;
      if (!(target instanceof HTMLElement)) return null;

      const scopedTarget =
        target.closest('[data-list-item-id^="guildsnav___"]') ||
        target.closest("a[href]") ||
        target;
      const image =
        scopedTarget?.querySelector?.("img") ||
        target.querySelector("img");
      const labelText = normalizeGuildLabel(
        guild?.name ||
          image?.getAttribute("alt") ||
          scopedTarget?.getAttribute?.("aria-label") ||
          target.getAttribute("aria-label") ||
          ""
      );
      const iconUrl =
        image?.currentSrc ||
        image?.getAttribute("src") ||
        "";

      if (!labelText) return null;

      return {{
        label: labelText,
        iconUrl,
        iconText: getInitials(labelText),
      }};
    }};

    const setTitlebarIcon = context => {{
      if (!labelIcon) return;

      if (!context) {{
        labelIcon.dataset.visible = "false";
        labelIcon.dataset.variant = "";
        labelIcon.innerHTML = "";
        return;
      }}

      labelIcon.dataset.visible = "true";
      labelIcon.dataset.variant = context.variant || "";

      if (context.iconUrl) {{
        labelIcon.innerHTML = `<img src="${{context.iconUrl}}" alt="" referrerpolicy="no-referrer" />`;
        return;
      }}

      if (context.variant === "dm") {{
        labelIcon.innerHTML = `
          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <path d="M19.73 5.03A16.8 16.8 0 0 0 15.65 4l-.2.4a15.4 15.4 0 0 1 3.74 1.12 12.9 12.9 0 0 0-4.13-1.24 14.7 14.7 0 0 0-6.12 0A12.85 12.85 0 0 0 4.8 5.52 15.4 15.4 0 0 1 8.55 4.4L8.35 4a16.8 16.8 0 0 0-4.08 1.03C1.69 8.86.99 12.59 1.34 16.27a16.95 16.95 0 0 0 5.01 2.53l1.08-1.76c-.59-.2-1.16-.45-1.7-.73.14.1.29.2.44.28a11.55 11.55 0 0 0 10.66 0c.15-.09.3-.18.44-.28-.54.28-1.11.53-1.7.73l1.08 1.76a16.91 16.91 0 0 0 5.01-2.53c.41-4.26-.7-7.96-2.93-11.24ZM8.68 13.95c-.98 0-1.78-.9-1.78-2s.79-2 1.78-2 1.79.9 1.78 2c0 1.1-.8 2-1.78 2Zm6.64 0c-.98 0-1.78-.9-1.78-2s.79-2 1.78-2 1.79.9 1.78 2c0 1.1-.79 2-1.78 2Z"></path>
          </svg>
        `;
        return;
      }}

      labelIcon.textContent = context.iconText || "";
    }};

    const syncNativeWindowTitle = nextTitle => {{
      if (!nextTitle || state.nativeWindowTitle === nextTitle) return;
      state.nativeWindowTitle = nextTitle;
      invoke("window_set_title", {{ title: nextTitle }}).catch(error => console.warn("[Equirust]", error));
    }};

    const syncLabel = () => {{
      if (getHostSettingValue({{
        key: "staticTitle",
        defaultValue: false,
      }})) {{
        const resolvedTitle = "Equirust";
        label.textContent = resolvedTitle;
        setTitlebarIcon(null);
        syncNativeWindowTitle(resolvedTitle);
        return;
      }}

      if (isDirectMessagesRoute()) {{
        const resolvedTitle = "Direct Messages";
        label.textContent = resolvedTitle;
        setTitlebarIcon({{ variant: "dm" }});
        syncNativeWindowTitle(resolvedTitle);
        return;
      }}

      const guild = resolveGuildIdentity();
      if (guild) {{
        label.textContent = guild.label;
        setTitlebarIcon({{
          variant: guild.iconUrl ? "guild-image" : "guild-text",
          iconUrl: guild.iconUrl,
          iconText: guild.iconText,
        }});
        syncNativeWindowTitle(guild.label);
        return;
      }}

      const resolvedTitle = sanitizeWindowTitle(document.title);
      label.textContent = resolvedTitle;
      setTitlebarIcon(null);
      syncNativeWindowTitle(resolvedTitle);
    }};

    const syncMaximizeState = () => {{
      invoke("window_is_maximized")
        .then(maximized => {{
          maximizeButton.innerHTML = maximized
            ? '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M8 8h10v10H8z"></path><path d="M6 16V6h10"></path></svg>'
            : '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="6" y="6" width="12" height="12"></rect></svg>';
        }})
        .catch(error => console.warn("[Equirust]", error));
    }};

    const findDiscordHeaderButton = selectors =>
      selectors
        .flatMap(selector => Array.from(document.querySelectorAll(selector)))
        .find(target => {{
          if (!(target instanceof HTMLElement)) return false;
          if (target.closest("#equirust-titlebar")) return false;

          const rect = target.getBoundingClientRect();
          return rect.width > 0 && rect.height > 0;
        }});

    const clickDiscordHeaderButton = selectors => {{
      const target = findDiscordHeaderButton(selectors);
      if (!target) return false;
      target.focus?.();
      target.dispatchEvent(new PointerEvent("pointerdown", {{ bubbles: true, cancelable: true, pointerId: 1, view: window }}));
      target.dispatchEvent(new MouseEvent("mousedown", {{ bubbles: true, cancelable: true, view: window }}));
      target.dispatchEvent(new PointerEvent("pointerup", {{ bubbles: true, cancelable: true, pointerId: 1, view: window }}));
      target.dispatchEvent(new MouseEvent("mouseup", {{ bubbles: true, cancelable: true, view: window }}));
      target.click();
      return true;
    }};

    const discordUtilitySelectors = {{
      inbox: [
        'button[aria-label="Inbox"]',
        'button[aria-label*="Inbox"]',
        'button[aria-label*="Mentions"]',
        'div[role="button"][aria-label="Inbox"]',
        '[role="button"][aria-label*="Inbox"]',
        '[role="button"][aria-label*="Mentions"]',
      ],
      help: [
        'button[aria-label="Help"]',
        'button[aria-label*="Help"]',
        'button[aria-label*="Support"]',
        'div[role="button"][aria-label="Help"]',
        '[role="button"][aria-label*="Help"]',
        '[role="button"][aria-label*="Support"]',
      ],
    }};

    const openDiscordUtility = action => {{
      switch (action) {{
        case "inbox":
          return clickDiscordHeaderButton(discordUtilitySelectors.inbox);
        case "help":
          return clickDiscordHeaderButton(discordUtilitySelectors.help);
        default:
          return false;
      }}
    }};

    const syncUtilityButtonState = () => {{
      if (inboxButton) {{
        inboxButton.disabled = !findDiscordHeaderButton(discordUtilitySelectors.inbox);
      }}
      if (helpButton) {{
        helpButton.disabled = !findDiscordHeaderButton(discordUtilitySelectors.help);
      }}
    }};

    const navigateHistory = direction => {{
      if (direction === "back") {{
        history.back();
      }} else if (direction === "forward") {{
        history.forward();
      }}
    }};

    const mouseSideButtonsNavigationEnabled = () =>
      getHostSettingValue({{
        key: "mouseSideButtonsNavigation",
        defaultValue: true,
      }});

    const getMouseSideButtonDirection = button => {{
      if (button === 3) return "back";
      if (button === 4) return "forward";
      return null;
    }};

    const stopInteractiveTitlebarEvent = event => {{
      event.stopPropagation();
      if (typeof event.stopImmediatePropagation === "function") {{
        event.stopImmediatePropagation();
      }}
    }};

    const cancelInteractiveTitlebarEvent = event => {{
      event.preventDefault();
      stopInteractiveTitlebarEvent(event);
    }};

    const swallowMouseSideButton = event => {{
      if (!getMouseSideButtonDirection(event.button)) return;
      event.preventDefault();
      event.stopPropagation();
    }};

    const handleMouseSideButtonNavigation = event => {{
      const direction = getMouseSideButtonDirection(event.button);
      if (!direction) return;

      event.preventDefault();
      event.stopPropagation();

      if (!isDiscordWindowActive()) return;
      if (!mouseSideButtonsNavigationEnabled()) return;
      navigateHistory(direction);
    }};

    const runTitlebarAction = action => {{
      switch (action) {{
        case "back":
          navigateHistory("back");
          break;
        case "forward":
          navigateHistory("forward");
          break;
        case "inbox":
          openDiscordUtility("inbox");
          window.setTimeout(syncUtilityButtonState, 0);
          break;
        case "help":
          openDiscordUtility("help");
          window.setTimeout(syncUtilityButtonState, 0);
          break;
        case "minimize":
          invoke("window_minimize");
          break;
        case "maximize":
          invoke("window_toggle_maximize").then(syncMaximizeState);
          break;
        case "close":
          invoke("window_close");
          break;
      }}
    }};

    const isInteractiveTitlebarTarget = target => {{
      if (!(target instanceof Element)) return false;
      return Boolean(
        target.closest(
          '#equirust-titlebar [data-action], #equirust-titlebar .equirust-titlebar__left, #equirust-titlebar .equirust-titlebar__controls'
        )
      );
    }};

    titlebar.querySelectorAll("[data-action]").forEach(button => {{
      button.setAttribute("draggable", "false");
      [
        "pointerdown",
        "mousedown",
        "pointerup",
        "mouseup",
        "auxclick",
        "mousemove",
        "pointermove",
      ].forEach(eventName => {{
        button.addEventListener(eventName, cancelInteractiveTitlebarEvent, true);
      }});
      ["dragstart", "dblclick", "selectstart"].forEach(eventName => {{
        button.addEventListener(eventName, cancelInteractiveTitlebarEvent, true);
      }});
      button.addEventListener(
        "click",
        event => {{
          event.preventDefault();
          stopInteractiveTitlebarEvent(event);
          runTitlebarAction(button.getAttribute("data-action"));
        }},
        true
      );
    }});

    window.addEventListener(
      "dblclick",
      event => {{
        if (!isInteractiveTitlebarTarget(event.target)) return;
        cancelInteractiveTitlebarEvent(event);
      }},
      true
    );

    const clearPendingDrag = () => {{
      pendingDragPointer = null;
    }};

    dragRegion.addEventListener(
      "mousedown",
      event => {{
        if (event.button !== 0) return;
        if (event.target !== dragRegion) return;
        pendingDragPointer = {{
          x: event.clientX,
          y: event.clientY,
        }};
      }},
      true
    );

    window.addEventListener(
      "mousemove",
      event => {{
        if (!pendingDragPointer) return;
        if ((event.buttons & 1) !== 1) {{
          clearPendingDrag();
          return;
        }}

        const movedX = Math.abs(event.clientX - pendingDragPointer.x);
        const movedY = Math.abs(event.clientY - pendingDragPointer.y);
        if (Math.max(movedX, movedY) < 4) return;

        clearPendingDrag();
        event.preventDefault();
        event.stopPropagation();
        invoke("window_start_dragging").catch(error => console.warn("[Equirust]", error));
      }},
      true
    );

    window.addEventListener("mouseup", clearPendingDrag, true);
    window.addEventListener("blur", clearPendingDrag, true);

    dragRegion.addEventListener("dblclick", event => {{
      if (event.target !== dragRegion) {{
        cancelInteractiveTitlebarEvent(event);
        return;
      }}
      if (isInteractiveTitlebarTarget(event.target)) {{
        cancelInteractiveTitlebarEvent(event);
        return;
      }}
      clearPendingDrag();
      invoke("window_toggle_maximize").then(syncMaximizeState);
    }});

    const resizeHandles = [
      ["n", "North"],
      ["s", "South"],
      ["e", "East"],
      ["w", "West"],
      ["ne", "NorthEast"],
      ["nw", "NorthWest"],
      ["se", "SouthEast"],
      ["sw", "SouthWest"],
    ];

    resizeHandles.forEach(([suffix, direction]) => {{
      const handle = document.createElement("div");
      handle.className = `equirust-resize equirust-resize--${{suffix}}`;
      handle.addEventListener("mousedown", event => {{
        if (event.button !== 0) return;
        event.preventDefault();
        invoke("window_start_resize_dragging", {{ direction }}).catch(error => console.warn("[Equirust]", error));
      }});
      document.body.appendChild(handle);
    }});

    window.__EQUIRUST_TITLEBAR_SYNC__ = () => {{
      syncLabel();
      syncMaximizeState();
      syncUtilityButtonState();
    }};

    syncLabel();
    syncMaximizeState();
    syncUtilityButtonState();
    window.addEventListener("resize", syncMaximizeState);
    window.addEventListener("popstate", syncLabel);
    ["pushState", "replaceState"].forEach(methodName => {{
      const original = history[methodName];
      if (typeof original !== "function") return;

      history[methodName] = function(...args) {{
        const result = original.apply(this, args);
        window.setTimeout(syncLabel, 0);
        return result;
      }};
    }});
    document.addEventListener("visibilitychange", () => {{
      if (document.hidden) return;
      syncLabel();
      syncUtilityButtonState();
    }});
    window.addEventListener("mousedown", swallowMouseSideButton, true);
    window.addEventListener("mouseup", handleMouseSideButtonNavigation, true);
  }};

  const syncTypingIndicators = () => {{
    state.typingPollScheduled = false;
    if (!isDiscordHost() || document.hidden) return;

    document.querySelectorAll("[aria-live='polite']").forEach(node => {{
      const text = (node.textContent || "").trim().toLowerCase();
      const isTyping = text.includes("typing");
      const existing = Array.from(node.children || []).find(child =>
        child.classList && child.classList.contains("equirust-typing-bloom")
      );

      if (isTyping) {{
        node.classList.add("equirust-typing-host");
        if (!existing) {{
          const bloom = document.createElement("span");
          bloom.className = "equirust-typing-bloom";
          bloom.setAttribute("aria-hidden", "true");
          node.appendChild(bloom);
        }}
      }} else {{
        node.classList.remove("equirust-typing-host");
        if (existing) existing.remove();
      }}
    }});
  }};

  const scheduleTypingSync = () => {{
    if (state.typingPollScheduled) return;
    state.typingPollScheduled = true;
    window.requestAnimationFrame(syncTypingIndicators);
  }};

  const typingNodeTouched = node => {{
    if (!node) return false;

    if (node.nodeType === Node.TEXT_NODE) {{
      return Boolean(node.parentElement?.closest?.("[aria-live='polite']"));
    }}

    if (node.nodeType !== Node.ELEMENT_NODE) {{
      return false;
    }}

    return (
      node.matches?.("[aria-live='polite']") ||
      Boolean(node.closest?.("[aria-live='polite']")) ||
      Boolean(node.querySelector?.("[aria-live='polite']"))
    );
  }};

  const typingMutationsTouched = records =>
    records.some(record =>
      typingNodeTouched(record.target) ||
      Array.from(record.addedNodes || []).some(typingNodeTouched) ||
      Array.from(record.removedNodes || []).some(typingNodeTouched)
    );

  const installTypingIndicator = () => {{
    if (state.typingObserver || !document.body || !isDiscordHost()) return;

    scheduleTypingSync();
    state.typingObserver = new MutationObserver(records => {{
      if (document.hidden) return;
      if (!typingMutationsTouched(records)) return;
      scheduleTypingSync();
    }});
    state.typingObserver.observe(document.body, {{
      subtree: true,
      childList: true,
      characterData: true,
    }});

    document.addEventListener("visibilitychange", () => {{
      if (document.hidden) return;
      scheduleTypingSync();
    }});
  }};

  const desktopSettingDefinitions = [
    {{
      key: "customTitleBar",
      title: "Discord Titlebar",
      description: "Keep the Discord-style custom titlebar provided by the Rust host.",
      defaultValue: String(state.versions.platform || "").toLowerCase() === "windows",
      restartRequired: true,
    }},
    {{
      key: "autoStartMinimized",
      title: "Auto Start Minimized",
      description: "Start minimized when Equirust is launched automatically with Windows.",
      defaultValue: false,
    }},
    {{
      key: "tray",
      title: "Tray Icon",
      description: "Show a tray icon for quick access and background behavior.",
      defaultValue: true,
    }},
    {{
      key: "minimizeToTray",
      title: "Minimize To Tray",
      description: "Clicking close hides the app to the tray instead of exiting.",
      defaultValue: true,
    }},
    {{
      key: "clickTrayToShowHide",
      title: "Toggle On Tray Click",
      description: "Left clicking the tray icon toggles the main window.",
      defaultValue: false,
    }},
    {{
      key: "disableMinSize",
      title: "Disable Minimum Size",
      description: "Allow shrinking the window below the default Discord minimum.",
      defaultValue: false,
    }},
    {{
      key: "staticTitle",
      title: "Static Title",
      description: "Keep the window title fixed instead of following the active page.",
      defaultValue: false,
    }},
    {{
      key: "enableMenu",
      title: "Enable Menu Bar",
      description: "Expose a native menu bar when the custom titlebar is disabled.",
      defaultValue: false,
      restartRequired: true,
    }},
    {{
      key: "openLinksWithElectron",
      title: "Open Links In App",
      description: "Open external web links in separate Equirust windows instead of your default browser.",
      defaultValue: false,
    }},
    {{
      key: "middleClickAutoscroll",
      title: "Middle Click Autoscroll",
      description: "Enable browser autoscroll for the hosted Discord runtime.",
      defaultValue: false,
      restartRequired: true,
    }},
    {{
      key: "mouseSideButtonsNavigation",
      title: "Mouse Back And Forward Buttons",
      description: "Use mouse side buttons to navigate Discord history without leaving it up to default webview behavior.",
      defaultValue: true,
    }},
    {{
      key: "hardwareAcceleration",
      title: "Hardware Acceleration",
      description: "Allow the webview to use GPU acceleration where available.",
      defaultValue: true,
      restartRequired: true,
    }},
    {{
      key: "hardwareVideoAcceleration",
      title: "Video Hardware Acceleration",
      description: "Prefer hardware decode for video playback and streaming paths.",
      defaultValue: true,
      restartRequired: true,
    }},
    {{
      key: "appBadge",
      title: "Unread Badge",
      description: "Show unread and mention counts on the Windows taskbar icon and tray state.",
      defaultValue: true,
    }},
    {{
      key: "badgeOnlyForMentions",
      title: "Badge Only For Mentions",
      description: "Limit badge counts to mentions and requests instead of all unread channels.",
      defaultValue: true,
    }},
    {{
      key: "enableTaskbarFlashing",
      title: "Taskbar Flashing",
      description: "Flash the taskbar button for new attention-worthy activity while Equirust is unfocused.",
      defaultValue: false,
    }},
    {{
      key: "runtimeDiagnostics",
      title: "Verbose Runtime Diagnostics",
      description: "Enable extra Rust and Discord-page bridge diagnostics for troubleshooting. Leave this off for the lightest normal runtime.",
      defaultValue: false,
    }},
  ];

  const getHostSettingValue = definition => {{
    const value = state.hostSettings?.[definition.key];
    return typeof value === "boolean" ? value : Boolean(definition.defaultValue);
  }};

  const getHostSettingChoice = (key, defaultValue) => {{
    const value = state.hostSettings?.[key];
    return typeof value === "string" && value.length ? value : String(defaultValue);
  }};

  const getHostSettingText = (key, defaultValue = "") => {{
    const value = state.hostSettings?.[key];
    return typeof value === "string" ? value : String(defaultValue);
  }};

  const getHostSettingList = key => {{
    const value = state.hostSettings?.[key];
    if (!Array.isArray(value)) {{
      return [];
    }}

    return value
      .filter(entry => typeof entry === "string")
      .map(entry => entry.trim())
      .filter(Boolean);
  }};

  const getHostSettingNumberText = key => {{
    const value = state.hostSettings?.[key];
    return typeof value === "number" && Number.isFinite(value) ? String(value) : "";
  }};

  const syncSpellcheckDictionaryFromHostSettings = () => {{
    state.spellcheckLearnedWords = new Set(
      getHostSettingList("spellCheckDictionary").map(word => word.toLocaleLowerCase())
    );
  }};

  const focusEquirustSettingsSection = async sectionKey => {{
    const clickEquirustEntry = () => {{
      const candidates = Array.from(
        document.querySelectorAll(
          '[role="tab"], nav button, nav [role="button"], [class*="sidebar"] [role="button"], [class*="side"] [role="button"]'
        )
      );
      const entry = candidates.find(node => {{
        if (!(node instanceof HTMLElement)) return false;
        if (node.closest("#equirust-titlebar")) return false;
        return (node.textContent || "").trim() === "Equirust";
      }});

      if (!entry) return false;
      entry.click();
      return true;
    }};

    clickEquirustEntry();

    for (let attempt = 0; attempt < 20; attempt += 1) {{
      const target = document.querySelector(`[data-equirust-section="${{sectionKey}}"]`);
      if (target instanceof HTMLElement) {{
        target.scrollIntoView({{ behavior: "smooth", block: "start" }});
        return true;
      }}

      await new Promise(resolve => window.setTimeout(resolve, 120));
    }}

    return false;
  }};

  const hostSettingDisabled = definition => {{
    switch (definition.key) {{
      case "minimizeToTray":
      case "clickTrayToShowHide":
        return !getHostSettingValue({{ key: "tray", defaultValue: true }});
      case "autoStartMinimized":
        return !state.nativeAutoStartEnabled;
      case "hardwareVideoAcceleration":
        return !getHostSettingValue({{ key: "hardwareAcceleration", defaultValue: true }});
      case "badgeOnlyForMentions":
        return !getHostSettingValue({{ key: "appBadge", defaultValue: true }});
      case "enableMenu":
        return getHostSettingValue({{
          key: "customTitleBar",
          defaultValue: String(state.versions.platform || "").toLowerCase() === "windows",
        }});
      default:
        return false;
    }}
  }};

  const persistHostSetting = async (key, value) => {{
    const next = {{ ...state.hostSettings, [key]: value }};
    state.hostSettings = next;
    const snapshot = await invoke("set_settings", {{ settings: next }});
    state.hostSettings = snapshot?.settings || next;
    if (key === "spellCheckDictionary") {{
      syncSpellcheckDictionaryFromHostSettings();
    }}
    if (String(key).startsWith("arRpc")) {{
      refreshArRPCStatus().catch(error => {{
        console.error("[Equirust]", error);
      }});
    }}
    scheduleHostBadgeSync();
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.hostSettings;
  }};

  const refreshNativeAutoStart = async () => {{
    if (!supportsNativeAutoStart()) {{
      state.nativeAutoStartEnabled = false;
      window.__EQUIRUST_SETTINGS_SYNC__?.();
      return false;
    }}

    const enabled = await invoke("get_auto_start_status");
    state.nativeAutoStartEnabled = enabled === true;
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.nativeAutoStartEnabled;
  }};

  const persistNativeAutoStart = async enabled => {{
    state.nativeAutoStartEnabled = (await invoke("set_auto_start_enabled", {{ enabled }})) === true;
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.nativeAutoStartEnabled;
  }};

  const refreshHostUpdateStatus = async () => {{
    state.hostUpdateStatus = await invoke("get_host_update_status");
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.hostUpdateStatus;
  }};

  const refreshRuntimeUpdateStatus = async () => {{
    state.runtimeUpdateStatus = await invoke("get_runtime_update_status");
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.runtimeUpdateStatus;
  }};

  const refreshHostUpdateDownloadState = async () => {{
    state.hostUpdateDownloadState = await invoke("get_host_update_download_state");
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.hostUpdateDownloadState;
  }};

  const refreshArRPCStatus = async () => {{
    state.arrpcStatus = await invoke("get_arrpc_status");
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.arrpcStatus;
  }};

  const restartArRPC = async () => {{
    state.arrpcStatus = await invoke("restart_arrpc");
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.arrpcStatus;
  }};

  const refreshFileManagerState = async () => {{
    state.fileManagerState = await invoke("get_file_manager_state");
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.fileManagerState;
  }};

  const openHostUpdate = async () => {{
    await invoke("open_host_update");
    return true;
  }};

  const openRuntimeUpdate = async () => {{
    await invoke("open_runtime_update");
    return true;
  }};

  const installRuntimeUpdate = async () => {{
    await invoke("install_runtime_update");
    return true;
  }};

  const installHostUpdate = async () => {{
    await invoke("install_host_update");
    return refreshHostUpdateDownloadState();
  }};

  const getRuntimeUpdateStatusCached = async forceRefresh => {{
    if (forceRefresh === true || !state.runtimeUpdateStatus) {{
      await refreshRuntimeUpdateStatus();
    }}

    return state.runtimeUpdateStatus;
  }};

  const getRuntimeUpdateRepo = () =>
    state.versions.vencordRepo || "https://github.com/Equicord/Equicord";

  const formatRuntimeVersionLabel = raw => {{
    const text = String(raw || "").trim();
    if (!text) {{
      return "Unknown";
    }}

    const cleaned = text.replace(/^(equicord|vencord)\s+/i, "").trim();
    const semverMatch = cleaned.match(/v?\d+\.\d+\.\d+(?:[-+._][A-Za-z0-9.-]+)*/);
    if (semverMatch?.[0]) {{
      return semverMatch[0].replace(/^v/i, "");
    }}

    if (/^[0-9a-f]{{10,}}$/i.test(cleaned)) {{
      return cleaned.slice(0, 12);
    }}

    return cleaned.length > 24 ? `${{cleaned.slice(0, 21)}}...` : cleaned;
  }};

  const summarizeRuntimeUpdateEntries = status => {{
    if (!status?.updateAvailable) {{
      return [];
    }}

    const rawNotes =
      typeof status.releaseNotes === "string" ? status.releaseNotes.trim() : "";
    const messages = rawNotes
      .split(/\r?\n+/)
      .map(line => line.replace(/^[\s*-]+/, "").trim())
      .filter(Boolean);
    const fallbackMessage =
      typeof status.releaseName === "string" && status.releaseName.trim()
        ? status.releaseName.trim()
        : typeof status.latestVersion === "string" && status.latestVersion.trim()
          ? `Runtime ${{formatRuntimeVersionLabel(status.latestVersion)}} is available`
          : "A linked runtime update is available";
    const author = (() => {{
      try {{
        const repo = new URL(getRuntimeUpdateRepo());
        const parts = repo.pathname.split("/").filter(Boolean);
        return parts.at(-1) || "Equicord";
      }} catch {{
        return "Equicord";
      }}
    }})();
    const hashBase =
      String(status.latestVersion || status.releaseName || "latest")
        .trim()
        .replace(/[^a-z0-9]+/gi, "-")
        .replace(/^-+|-+$/g, "")
        .toLowerCase() || "latest";

    return (messages.length ? messages : [fallbackMessage]).slice(0, 24).map((message, index) => ({{
      hash: `${{hashBase}}-${{String(index + 1).padStart(2, "0")}}`,
      author,
      message,
    }}));
  }};

  const snoozeHostUpdate = async () => {{
    state.hostUpdateStatus = await invoke("snooze_host_update");
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.hostUpdateStatus;
  }};

  const snoozeRuntimeUpdate = async () => {{
    state.runtimeUpdateStatus = await invoke("snooze_runtime_update");
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.runtimeUpdateStatus;
  }};

  const ignoreHostUpdate = async version => {{
    state.hostUpdateStatus = await invoke("ignore_host_update", {{ version }});
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.hostUpdateStatus;
  }};

  const ignoreRuntimeUpdate = async version => {{
    state.runtimeUpdateStatus = await invoke("ignore_runtime_update", {{ version }});
    window.__EQUIRUST_SETTINGS_SYNC__?.();
    return state.runtimeUpdateStatus;
  }};

  const openUserAssetsFolder = async () => {{
    await invoke("open_user_assets_folder");
    return true;
  }};

  const chooseUserAsset = async (asset, reset) => {{
    return invoke("choose_user_asset", {{ asset, reset: reset === true }});
  }};

  const showCustomVencordDir = async () => {{
    await invoke("show_custom_vencord_dir");
    return true;
  }};

  const selectCustomVencordDir = async reset => {{
    const result = await invoke("select_vencord_dir", {{ reset: reset === true }});
    await refreshFileManagerState();
    return result;
  }};

  const openDebugPage = async target => {{
    await invoke("open_debug_page", {{ target }});
    return true;
  }};

  const getHostUpdateStatusCached = async forceRefresh => {{
    if (forceRefresh === true || !state.hostUpdateStatus) {{
      await refreshHostUpdateStatus();
    }}

    return state.hostUpdateStatus;
  }};

  const getSystemThemeValues = async () => {{
    try {{
      const values = await invoke("get_system_theme_values");
      if (values && typeof values === "object") {{
        return values;
      }}
    }} catch (error) {{
      console.warn("[Equirust] Failed to read system theme values", error);
    }}

    return {{ "os-accent-color": "#5865f2" }};
  }};

  const ensureRendererCommandBridge = () => {{
    if (state.commandBridgeReady || state.commandBridgeCleanup) return;

    listenTauriEvent("equirust:ipc-command", payload => {{
      const command =
        payload?.payload && typeof payload.payload === "object" ? payload.payload : payload;
      const nonce = String(command?.nonce || "");
      const message = String(command?.message || "");

      if (!nonce || !message) {{
        invoke("respond_renderer_command", {{
          nonce,
          ok: false,
          data: "Malformed renderer command payload.",
        }}).catch(error => console.error("[Equirust]", error));
        return;
      }}

      if (!state.commandListeners.size) {{
        invoke("respond_renderer_command", {{
          nonce,
          ok: false,
          data: "No renderer command handler is registered.",
        }}).catch(error => console.error("[Equirust]", error));
        return;
      }}

      state.commandListeners.forEach(listener => {{
        try {{
          listener({{
            nonce,
            message,
            data: command?.data,
          }});
        }} catch (error) {{
          console.error("[Equirust]", error);
        }}
      }});
    }})
      .then(cleanup => {{
        state.commandBridgeCleanup = cleanup;
        state.commandBridgeReady = true;
      }})
      .catch(error => {{
        console.error("[Equirust]", error);
      }});
  }};

  const releaseRendererCommandBridge = () => {{
    if (state.commandListeners.size || !state.commandBridgeCleanup) return;

    const cleanup = state.commandBridgeCleanup;
    state.commandBridgeCleanup = null;
    state.commandBridgeReady = false;
    cleanup().catch(error => console.error("[Equirust]", error));
  }};

  const notifyQuickCssListeners = () => {{
    state.quickCssListeners.forEach(listener => {{
      try {{
        listener(state.quickCss);
      }} catch (error) {{
        console.error("[Equirust]", error);
      }}
    }});
  }};

  const notifyThemeListeners = () => {{
    state.themeListeners.forEach(listener => {{
      try {{
        listener();
      }} catch (error) {{
        console.error("[Equirust]", error);
      }}
    }});
  }};

  const notifyRendererCssListeners = css => {{
    state.rendererCssListeners.forEach(listener => {{
      try {{
        listener(css);
      }} catch (error) {{
        console.error("[Equirust]", error);
      }}
    }});
  }};

  const refreshRendererCss = async () => {{
    const css = await invoke("get_vencord_renderer_css");
    if (typeof css === "string" && css !== state.rendererCssValue) {{
      state.rendererCssValue = css;
      notifyRendererCssListeners(css);
    }}

    return typeof state.rendererCssValue === "string" ? state.rendererCssValue : "";
  }};

  const ensureRendererCssWatch = () => {{
    if (state.rendererCssPollTimer || !state.rendererCssListeners.size) return;

    if (!state.rendererCssVisibilityBound) {{
      state.rendererCssVisibilityBound = true;
      document.addEventListener("visibilitychange", () => {{
        if (document.hidden || !state.rendererCssListeners.size) return;
        if (state.rendererCssPollTimer) {{
          window.clearTimeout(state.rendererCssPollTimer);
          state.rendererCssPollTimer = null;
        }}
        ensureRendererCssWatch();
      }});
      window.addEventListener("focus", () => {{
        if (!state.rendererCssListeners.size) return;
        if (state.rendererCssPollTimer) {{
          window.clearTimeout(state.rendererCssPollTimer);
          state.rendererCssPollTimer = null;
        }}
        ensureRendererCssWatch();
      }});
    }}

    const delay =
      state.rendererCssValue == null
        ? 0
        : getAdaptivePollDelay(state.debugBuild ? 1000 : 3000, 12000);
    state.rendererCssPollTimer = window.setTimeout(() => {{
      state.rendererCssPollTimer = null;
      if (!state.rendererCssListeners.size) {{
        return;
      }}

      const refresh = shouldUseActivePolling()
        ? refreshRendererCss()
        : Promise.resolve(typeof state.rendererCssValue === "string" ? state.rendererCssValue : "");

      refresh
        .catch(error => console.error("[Equirust]", error))
        .finally(() => {{
          ensureRendererCssWatch();
        }});
    }}, delay);
  }};

  const addRendererCssListener = listener => {{
    if (typeof listener !== "function") return;
    state.rendererCssListeners.add(listener);
    ensureRendererCssWatch();
  }};

  const normalizeSpellcheckWord = value =>
    String(value || "")
      .trim()
      .replace(/^[^A-Za-z0-9]+|[^A-Za-z0-9]+$/g, "")
      .replace(/\s+/g, " ");

  const getSpellcheckLanguages = () => {{
    const configured = getHostSettingList("spellCheckLanguages");
    if (configured.length) {{
      return configured.slice(0, 5);
    }}

    const browserLanguages = Array.isArray(navigator.languages)
      ? navigator.languages
      : [navigator.language];
    const unique = [];
    browserLanguages.forEach(language => {{
      if (typeof language !== "string") return;
      const normalized = language.trim();
      if (!normalized || unique.includes(normalized)) return;
      unique.push(normalized);
    }});

    return unique.length ? unique.slice(0, 5) : ["en-US"];
  }};

  const resolveSpellcheckSelection = () => {{
    const active = document.activeElement;
    if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) {{
      if (active.disabled || active.readOnly) {{
        return null;
      }}

      const value = String(active.value || "");
      let start = Number(active.selectionStart ?? 0);
      let end = Number(active.selectionEnd ?? start);

      if (start === end) {{
        while (start > 0 && /[A-Za-z0-9'_’-]/.test(value[start - 1])) start -= 1;
        while (end < value.length && /[A-Za-z0-9'_’-]/.test(value[end])) end += 1;
      }}

      const word = normalizeSpellcheckWord(value.slice(start, end));
      if (!word) {{
        return null;
      }}

      return {{
        kind: "input",
        element: active,
        start,
        end,
        word,
      }};
    }}

    const selection = window.getSelection();
    const selectedText = normalizeSpellcheckWord(selection?.toString?.() || "");
    if (!selectedText) {{
      return null;
    }}

    return {{
      kind: "selection",
      word: selectedText,
    }};
  }};

  const buildSpellcheckSuggestions = word => {{
    const normalized = normalizeSpellcheckWord(word);
    if (!normalized) {{
      return [];
    }}

    const lower = normalized.toLocaleLowerCase();
    const titleCase = lower
      ? `${{lower.slice(0, 1).toLocaleUpperCase()}}${{lower.slice(1)}}`
      : normalized;
    const deDoubled = lower.replace(/(.)\1{{2,}}/g, "$1$1");
    const suggestions = [lower, titleCase, deDoubled].filter(
      candidate => candidate && candidate !== normalized
    );

    if (!suggestions.length) {{
      suggestions.push(normalized);
    }}

    return Array.from(new Set(suggestions)).slice(0, 5);
  }};

  const getSpellcheckSuggestions = async word => {{
    try {{
      const result = await invoke("check_spelling", {{
        word,
        languages: getSpellcheckLanguages(),
      }});
      const suggestions = Array.isArray(result?.suggestions)
        ? result.suggestions.filter(candidate => typeof candidate === "string" && candidate)
        : [];
      if (suggestions.length) {{
        return suggestions.slice(0, 5);
      }}
    }} catch (error) {{
      console.warn("[Equirust] Native spellcheck lookup failed", error);
    }}

    return buildSpellcheckSuggestions(word);
  }};

  const notifySpellcheckResult = (word, suggestions) => {{
    state.spellcheckResultListeners.forEach(listener => {{
      try {{
        listener(word, suggestions);
      }} catch (error) {{
        console.error("[Equirust]", error);
      }}
    }});
  }};

  const replaceSpellcheckSelection = replacement => {{
    const nextValue = String(replacement || "");
    const target = state.spellcheckSelection;
    state.spellcheckSelection = null;
    if (!target || !nextValue) {{
      return;
    }}

    if (
      target.kind === "input" &&
      (target.element instanceof HTMLInputElement || target.element instanceof HTMLTextAreaElement)
    ) {{
      target.element.focus();
      target.element.setRangeText(nextValue, target.start, target.end, "end");
      target.element.dispatchEvent(new InputEvent("input", {{ bubbles: true, data: nextValue }}));
      target.element.dispatchEvent(new Event("change", {{ bubbles: true }}));
      return;
    }}

    try {{
      document.execCommand("insertText", false, nextValue);
    }} catch (error) {{
      console.warn("[Equirust] Failed to replace misspelling", error);
    }}
  }};

  const addSpellcheckWordToDictionary = async word => {{
    const normalized = normalizeSpellcheckWord(word).toLocaleLowerCase();
    if (!normalized) {{
      return;
    }}

    const nextWords = Array.from(new Set([...state.spellcheckLearnedWords, normalized])).sort(
      (left, right) => left.localeCompare(right)
    );
    await persistHostSetting("spellCheckDictionary", nextWords);
  }};

  const installSpellcheckBridge = () => {{
    if (state.spellcheckContextMenuInstalled) return;

    document.addEventListener(
      "contextmenu",
      () => {{
        if (!state.spellcheckResultListeners.size) {{
          return;
        }}

        const target = resolveSpellcheckSelection();
        state.spellcheckSelection = target;
        if (!target) {{
          return;
        }}

        const normalizedWord = normalizeSpellcheckWord(target.word);
        if (
          normalizedWord.length < 3 ||
          !/[A-Za-z]/.test(normalizedWord) ||
          state.spellcheckLearnedWords.has(normalizedWord.toLocaleLowerCase())
        ) {{
          return;
        }}

        void getSpellcheckSuggestions(normalizedWord).then(suggestions => {{
          if (!suggestions.length) {{
            return;
          }}

          notifySpellcheckResult(normalizedWord, suggestions);
        }});
      }},
      true
    );

    state.spellcheckContextMenuInstalled = true;
  }};

  const refreshQuickCssFromDisk = async () => {{
    const css = await invoke("get_vencord_quick_css");
    if (typeof css === "string" && css !== state.quickCss) {{
      state.quickCss = css;
      notifyQuickCssListeners();
    }}

    return state.quickCss;
  }};

  const pollVencordFileState = async () => {{
    const next = await invoke("get_vencord_file_state");
    const nextQuickCssRevision = Number(next?.quickCssRevision ?? -1);
    const nextThemesRevision = Number(next?.themesRevision ?? -1);

    if (state.quickCssRevision == null) {{
      state.quickCssRevision = nextQuickCssRevision;
    }} else if (nextQuickCssRevision !== state.quickCssRevision) {{
      state.quickCssRevision = nextQuickCssRevision;
      await refreshQuickCssFromDisk();
    }}

    if (state.themesRevision == null) {{
      state.themesRevision = nextThemesRevision;
    }} else if (nextThemesRevision !== state.themesRevision) {{
      state.themesRevision = nextThemesRevision;
      notifyThemeListeners();
    }}

    return next;
  }};

  const ensureVencordFileWatch = () => {{
    if (state.vencordFileWatchTimer) return;
    if (!state.quickCssListeners.size && !state.themeListeners.size) return;

    if (!state.vencordFileWatchVisibilityBound) {{
      state.vencordFileWatchVisibilityBound = true;
      document.addEventListener("visibilitychange", () => {{
        if (document.hidden || (!state.quickCssListeners.size && !state.themeListeners.size)) return;
        if (state.vencordFileWatchTimer) {{
          window.clearTimeout(state.vencordFileWatchTimer);
          state.vencordFileWatchTimer = null;
        }}
        ensureVencordFileWatch();
      }});
      window.addEventListener("focus", () => {{
        if (!state.quickCssListeners.size && !state.themeListeners.size) return;
        if (state.vencordFileWatchTimer) {{
          window.clearTimeout(state.vencordFileWatchTimer);
          state.vencordFileWatchTimer = null;
        }}
        ensureVencordFileWatch();
      }});
    }}

    const delay =
      state.quickCssRevision == null && state.themesRevision == null
        ? 0
        : getAdaptivePollDelay(2000, 12000);
    state.vencordFileWatchTimer = window.setTimeout(() => {{
      state.vencordFileWatchTimer = null;
      if (!state.quickCssListeners.size && !state.themeListeners.size) {{
        state.quickCssRevision = null;
        state.themesRevision = null;
        return;
      }}

      const refresh = shouldUseActivePolling()
        ? pollVencordFileState()
        : Promise.resolve(null);

      refresh
        .catch(error => console.error("[Equirust]", error))
        .finally(() => {{
          ensureVencordFileWatch();
        }});
    }}, delay);
  }};

  const installDesktopSettingsPanel = () => {{
    if (state.settingsPanelReady || !document.body) return;
    state.settingsPanelReady = true;
    ensureNativeSurfaceStyles();

    const style = document.createElement("style");
    style.id = "equirust-settings-style";
    style.textContent = `
      body.equirust-settings-open #equirust-settings-modal {{
        opacity: 1;
        pointer-events: auto;
      }}
      #equirust-settings-modal {{
        position: fixed;
        inset: 0;
        z-index: 2147483645;
        display: grid;
        place-items: center;
        background: rgba(6, 8, 11, 0.68);
        backdrop-filter: blur(16px);
        opacity: 0;
        pointer-events: none;
        transition: opacity 140ms ease;
      }}
      .equirust-settings__dialog {{
        width: min(920px, calc(100vw - 32px));
        max-height: min(760px, calc(100vh - 48px));
        overflow: auto;
        color: #f2f3f5;
      }}
      .equirust-settings__header {{
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 16px;
        padding: 24px 24px 18px;
        border-bottom: 1px solid rgba(255, 255, 255, 0.06);
      }}
      .equirust-settings__eyebrow {{
        display: inline-flex;
        padding: 6px 10px;
        border-radius: 999px;
        background: rgba(88, 101, 242, 0.16);
        color: #c9d0ff;
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }}
      .equirust-settings__heading {{
        margin: 12px 0 6px;
        font-size: 30px;
        line-height: 1.05;
      }}
      .equirust-settings__lead {{
        margin: 0;
        color: #b5bac1;
        line-height: 1.5;
      }}
      .equirust-settings__close {{
        width: 38px;
        height: 38px;
        border: 0;
        border-radius: 10px;
        background: rgba(255, 255, 255, 0.06);
        color: #f2f3f5;
      }}
      .equirust-settings__body {{
        padding: 20px 24px 24px;
        display: grid;
        gap: 18px;
      }}
      .equirust-settings__section {{
        padding: 18px;
        border-radius: 16px;
        background: rgba(255, 255, 255, 0.035);
        border: 1px solid rgba(255, 255, 255, 0.06);
      }}
      .equirust-settings__section-title {{
        margin: 0 0 6px;
        font-size: 16px;
      }}
      .equirust-settings__section-copy {{
        margin: 0 0 14px;
        color: #aab1bb;
        line-height: 1.5;
      }}
      .equirust-settings__actions {{
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
      }}
      .equirust-settings__action {{
        border: 0;
        border-radius: 10px;
        padding: 10px 14px;
        background: rgba(88, 101, 242, 0.18);
        color: #eef1ff;
        font-weight: 600;
      }}
      .equirust-settings__grid {{
        display: grid;
        gap: 12px;
      }}
      .equirust-settings__card {{
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 14px;
        align-items: center;
        padding: 14px 16px;
        border-radius: 14px;
        background: rgba(255, 255, 255, 0.03);
        border: 1px solid rgba(255, 255, 255, 0.05);
      }}
      .equirust-settings__title-row {{
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
      }}
      .equirust-settings__title {{
        font-weight: 700;
      }}
      .equirust-settings__badge {{
        display: inline-flex;
        padding: 3px 8px;
        border-radius: 999px;
        background: rgba(255, 184, 108, 0.16);
        color: #ffd39a;
        font-size: 11px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.06em;
      }}
      .equirust-settings__description {{
        margin: 6px 0 0;
        color: #aab1bb;
        line-height: 1.45;
      }}
      .equirust-settings__toggle {{
        width: 22px;
        height: 22px;
      }}
      .equirust-settings__hint {{
        margin: 0;
        color: #8f98a3;
        font-size: 13px;
        line-height: 1.5;
      }}
      .equirust-settings__nav {{
        width: 100%;
        border: 0;
        background: transparent;
        color: var(--channels-default, #b5bac1);
        text-align: left;
        padding: 8px 10px;
        border-radius: 6px;
        font: inherit;
        font-weight: 600;
      }}
      .equirust-settings__nav:hover {{
        background: rgba(255, 255, 255, 0.06);
        color: var(--interactive-active, #f2f3f5);
      }}
      @media (max-width: 720px) {{
        .equirust-settings__dialog {{
          width: calc(100vw - 18px);
          max-height: calc(100vh - 18px);
        }}
        .equirust-settings__header,
        .equirust-settings__body {{
          padding-left: 16px;
          padding-right: 16px;
        }}
        .equirust-settings__card {{
          grid-template-columns: 1fr;
        }}
      }}
    `;
    document.documentElement.appendChild(style);

    const modal = document.createElement("div");
    modal.id = "equirust-settings-modal";
    modal.innerHTML = `
      <div class="equirust-settings__dialog equirust-surface-dialog" role="dialog" aria-modal="true" aria-labelledby="equirust-settings-heading">
        <div class="equirust-settings__header equirust-surface-header">
          <div>
            <span class="equirust-settings__eyebrow equirust-surface-eyebrow">Equirust Settings</span>
            <h2 class="equirust-settings__heading equirust-surface-title" id="equirust-settings-heading">Equirust</h2>
            <p class="equirust-settings__lead equirust-surface-copy">
              Settings for the title bar, startup, tray behavior, and local Equicord files.
            </p>
          </div>
          <button type="button" class="equirust-settings__close" aria-label="Close">✕</button>
        </div>
        <div class="equirust-settings__body">
          <section class="equirust-settings__section">
            <h3 class="equirust-settings__section-title">Themes, QuickCSS, and Plugins</h3>
            <p class="equirust-settings__section-copy">
              Use these shortcuts for local Equicord files. Use the Equicord sections in Discord settings for plugin and theme controls.
            </p>
            <div class="equirust-settings__actions">
              <button type="button" class="equirust-settings__action" data-eq-action="quickcss">Open QuickCSS</button>
              <button type="button" class="equirust-settings__action" data-eq-action="themes">Open Themes Folder</button>
              <button type="button" class="equirust-settings__action" data-eq-action="settings">Open Equicord Settings Folder</button>
            </div>
          </section>
          <section class="equirust-settings__section">
            <h3 class="equirust-settings__section-title">Desktop Host</h3>
            <p class="equirust-settings__section-copy">
              These controls are owned by Equirust rather than Discord itself, so changes here are persisted through the Rust store.
            </p>
            <div class="equirust-settings__grid" data-equirust-settings-grid></div>
          </section>
          <p class="equirust-settings__hint">
            Settings marked Restart are stored immediately but still depend on future full-host parity work before every legacy behavior matches Equibop exactly.
          </p>
        </div>
      </div>
    `;
    document.body.appendChild(modal);

    modal.addEventListener("click", event => {{
      if (event.target === modal) {{
        closeDesktopSettings();
      }}
    }});

    modal.querySelector(".equirust-settings__close")?.addEventListener("click", closeDesktopSettings);
    modal.querySelector('[data-eq-action="quickcss"]')?.addEventListener("click", () => invoke("open_vencord_quick_css"));
    modal.querySelector('[data-eq-action="themes"]')?.addEventListener("click", () => invoke("open_vencord_themes_folder"));
    modal.querySelector('[data-eq-action="settings"]')?.addEventListener("click", () => invoke("open_vencord_settings_folder"));

    document.addEventListener("keydown", event => {{
      if (event.key === "Escape") {{
        closeDesktopSettings();
      }}
    }});

    window.__EQUIRUST_SETTINGS_SYNC__ = renderDesktopSettingsPanel;
  }};

  const installDesktopSettingsStyles = () => {{
    if (document.getElementById("equirust-inline-settings-style")) return;

    const style = document.createElement("style");
    style.id = "equirust-inline-settings-style";
    style.textContent = `
      .vc-equirust-settings-page {{
        display: grid;
        gap: 20px;
        max-width: 860px;
      }}
      .vc-equirust-settings-page .equirust-settings__hero {{
        padding: 22px 24px;
        border-radius: 18px;
        background:
          radial-gradient(circle at top right, rgba(88, 101, 242, 0.22), transparent 42%),
          linear-gradient(180deg, rgba(255,255,255,0.03), rgba(255,255,255,0)),
          #12161d;
        border: 1px solid rgba(255, 255, 255, 0.08);
        box-shadow: 0 20px 48px rgba(0, 0, 0, 0.28);
      }}
      .vc-equirust-settings-page .equirust-settings__eyebrow {{
        display: inline-flex;
        padding: 6px 10px;
        border-radius: 999px;
        background: rgba(88, 101, 242, 0.16);
        color: #c9d0ff;
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }}
      .vc-equirust-settings-page .equirust-settings__heading {{
        margin: 12px 0 6px;
        font-size: 30px;
        line-height: 1.05;
      }}
      .vc-equirust-settings-page .equirust-settings__lead {{
        margin: 0;
        color: #b5bac1;
        line-height: 1.5;
      }}
      .vc-equirust-settings-page .equirust-settings__section {{
        padding: 18px;
        border-radius: 16px;
        background: rgba(255, 255, 255, 0.035);
        border: 1px solid rgba(255, 255, 255, 0.06);
      }}
      .vc-equirust-settings-page .equirust-settings__section-title {{
        margin: 0 0 6px;
        font-size: 16px;
      }}
      .vc-equirust-settings-page .equirust-settings__section-copy {{
        margin: 0 0 14px;
        color: #aab1bb;
        line-height: 1.5;
      }}
      .vc-equirust-settings-page .equirust-settings__actions {{
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
      }}
      .vc-equirust-settings-page .equirust-settings__action {{
        border: 0;
        border-radius: 10px;
        padding: 10px 14px;
        background: rgba(88, 101, 242, 0.18);
        color: #eef1ff;
        font-weight: 600;
        cursor: pointer;
      }}
      .vc-equirust-settings-page .equirust-settings__action--secondary {{
        background: rgba(255, 255, 255, 0.06);
        color: #d7dce3;
      }}
      .vc-equirust-settings-page .equirust-settings__action:disabled {{
        opacity: 0.55;
        cursor: default;
      }}
      .vc-equirust-settings-page .equirust-settings__grid {{
        display: grid;
        gap: 12px;
      }}
      .vc-equirust-settings-page .equirust-settings__update-notes {{
        display: grid;
        gap: 6px;
        margin: 0 0 14px;
      }}
      .vc-equirust-settings-page .equirust-settings__card {{
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 14px;
        align-items: center;
        padding: 14px 16px;
        border-radius: 14px;
        background: rgba(255, 255, 255, 0.03);
        border: 1px solid rgba(255, 255, 255, 0.05);
      }}
      .vc-equirust-settings-page .equirust-settings__title-row {{
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
      }}
      .vc-equirust-settings-page .equirust-settings__title {{
        font-weight: 700;
      }}
      .vc-equirust-settings-page .equirust-settings__badge {{
        display: inline-flex;
        padding: 3px 8px;
        border-radius: 999px;
        background: rgba(255, 184, 108, 0.16);
        color: #ffd39a;
        font-size: 11px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.06em;
      }}
      .vc-equirust-settings-page .equirust-settings__description {{
        margin: 6px 0 0;
        color: #aab1bb;
        line-height: 1.45;
      }}
      .vc-equirust-settings-page .equirust-settings__jump-nav {{
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
        margin: 18px 0 0;
      }}
      .vc-equirust-settings-page .equirust-settings__jump-button {{
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.04);
        color: #d7dce3;
        padding: 8px 12px;
        font: inherit;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
      }}
      .vc-equirust-settings-page .equirust-settings__hero-grid {{
        display: grid;
        gap: 12px;
        grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
        margin-top: 18px;
      }}
      .vc-equirust-settings-page .equirust-settings__metric {{
        display: grid;
        gap: 6px;
        padding: 14px 16px;
        border-radius: 14px;
        background: rgba(255, 255, 255, 0.03);
        border: 1px solid rgba(255, 255, 255, 0.05);
      }}
      .vc-equirust-settings-page .equirust-settings__metric-label {{
        color: #8f98a3;
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }}
      .vc-equirust-settings-page .equirust-settings__metric-value {{
        color: #f5f7fb;
        font-size: 18px;
        font-weight: 800;
        line-height: 1.2;
      }}
      .vc-equirust-settings-page .equirust-settings__metric-copy {{
        margin: 0;
        color: #aab1bb;
        font-size: 13px;
        line-height: 1.45;
      }}
      .vc-equirust-settings-page .equirust-settings__select {{
        min-width: 220px;
        border-radius: 10px;
        border: 1px solid rgba(255, 255, 255, 0.08);
        background: rgba(255, 255, 255, 0.06);
        color: #eef1ff;
        padding: 10px 12px;
        font: inherit;
      }}
      .vc-equirust-settings-page .equirust-settings__field {{
        display: grid;
        gap: 8px;
      }}
      .vc-equirust-settings-page .equirust-settings__input {{
        min-width: 220px;
        border-radius: 10px;
        border: 1px solid rgba(255, 255, 255, 0.08);
        background: rgba(255, 255, 255, 0.06);
        color: #eef1ff;
        padding: 10px 12px;
        font: inherit;
      }}
      .vc-equirust-settings-page .equirust-settings__hint {{
        margin: 0;
        color: #8f98a3;
        font-size: 13px;
        line-height: 1.5;
      }}
      .vc-equirust-settings-page .equirust-settings__subtle {{
        color: #8f98a3;
      }}
      .vc-equirust-settings-page .equirust-settings__card-side {{
        display: flex;
        flex-direction: column;
        align-items: flex-end;
        gap: 8px;
      }}
      .vc-equirust-settings-page .equirust-settings__switch {{
        display: inline-flex;
        align-items: center;
        gap: 10px;
        min-width: 94px;
        justify-content: flex-end;
        border: 0;
        background: transparent;
        color: #eef1ff;
        padding: 0;
        cursor: pointer;
        font: inherit;
      }}
      .vc-equirust-settings-page .equirust-settings__switch:disabled {{
        opacity: 0.55;
        cursor: default;
      }}
      .vc-equirust-settings-page .equirust-settings__switch-track {{
        position: relative;
        width: 44px;
        height: 26px;
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.14);
        transition: background 140ms ease, box-shadow 140ms ease;
        box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08);
      }}
      .vc-equirust-settings-page .equirust-settings__switch-track::after {{
        content: "";
        position: absolute;
        top: 3px;
        left: 3px;
        width: 20px;
        height: 20px;
        border-radius: 50%;
        background: #ffffff;
        box-shadow: 0 2px 8px rgba(0, 0, 0, 0.32);
        transition: transform 140ms ease;
      }}
      .vc-equirust-settings-page .equirust-settings__switch[data-checked="true"] .equirust-settings__switch-track {{
        background: rgba(88, 101, 242, 0.82);
      }}
      .vc-equirust-settings-page .equirust-settings__switch[data-checked="true"] .equirust-settings__switch-track::after {{
        transform: translateX(18px);
      }}
      .vc-equirust-settings-page .equirust-settings__switch-label {{
        min-width: 24px;
        text-align: right;
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.03em;
        text-transform: uppercase;
      }}
      .vc-equirust-settings-page .equirust-settings__saving {{
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: 16px;
        color: #8f98a3;
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }}
      .vc-equirust-settings-page .equirust-settings__details {{
        margin-top: 18px;
        border-radius: 16px;
        border: 1px solid rgba(255, 255, 255, 0.06);
        background: rgba(255, 255, 255, 0.02);
      }}
      .vc-equirust-settings-page .equirust-settings__details > summary {{
        list-style: none;
        cursor: pointer;
        padding: 16px 18px;
        font-weight: 700;
        color: #eef1ff;
      }}
      .vc-equirust-settings-page .equirust-settings__details > summary::-webkit-details-marker {{
        display: none;
      }}
      .vc-equirust-settings-page .equirust-settings__details-copy {{
        margin: 0;
        padding: 0 18px 16px;
        color: #8f98a3;
        line-height: 1.5;
      }}
      .vc-equirust-settings-page .equirust-settings__details-content {{
        display: grid;
        gap: 18px;
        padding: 0 18px 18px;
      }}
      @media (max-width: 720px) {{
        .vc-equirust-settings-page .equirust-settings__card {{
          grid-template-columns: 1fr;
        }}
        .vc-equirust-settings-page .equirust-settings__card-side {{
          align-items: flex-start;
        }}
        .vc-equirust-settings-page .equirust-settings__switch {{
          justify-content: flex-start;
        }}
      }}
    `;
    document.documentElement.appendChild(style);
  }};

  const createDesktopSettingsComponent = React => {{
    const h = React.createElement;

    return function EquirustSettingsPage() {{
      const [, forceRender] = React.useState(0);
      const [viewHostSettings, setViewHostSettings] = React.useState(
        () => (state.hostSettings ? {{ ...state.hostSettings }} : {{}})
      );
      const [viewNativeAutoStart, setViewNativeAutoStart] = React.useState(
        state.nativeAutoStartEnabled === true
      );
      const [savingHostSettings, setSavingHostSettings] = React.useState({{}});
      const [savingNativeAutoStart, setSavingNativeAutoStart] = React.useState(false);

      React.useEffect(() => {{
        installDesktopSettingsStyles();

        const sync = () => {{
          setViewHostSettings(state.hostSettings ? {{ ...state.hostSettings }} : {{}});
          setViewNativeAutoStart(state.nativeAutoStartEnabled === true);
          forceRender(version => version + 1);
        }};
        window.__EQUIRUST_SETTINGS_SYNC__ = sync;

        return () => {{
          if (window.__EQUIRUST_SETTINGS_SYNC__ === sync) {{
            delete window.__EQUIRUST_SETTINGS_SYNC__;
          }}
        }};
      }}, []);

      React.useEffect(() => {{
        refreshNativeAutoStart().catch(error => {{
          console.error("[Equirust]", error);
        }});
        refreshFileManagerState().catch(error => {{
          console.error("[Equirust]", error);
        }});
        refreshHostUpdateStatus().catch(error => {{
          console.error("[Equirust]", error);
        }});
        refreshRuntimeUpdateStatus().catch(error => {{
          console.error("[Equirust]", error);
        }});
        refreshHostUpdateDownloadState().catch(error => {{
          console.error("[Equirust]", error);
        }});
        refreshArRPCStatus().catch(error => {{
          console.error("[Equirust]", error);
        }});
      }}, []);

      React.useEffect(() => {{
        const phase = String(state.hostUpdateDownloadState?.phase || "idle");
        if (phase !== "downloading" && phase !== "launching") {{
          return undefined;
        }}

        const timer = window.setInterval(() => {{
          refreshHostUpdateDownloadState().catch(error => {{
            console.error("[Equirust]", error);
          }});
        }}, 1000);

        return () => {{
          window.clearInterval(timer);
        }};
      }}, [String(state.hostUpdateDownloadState?.phase || "idle")]);

      const getHostSettingValue = definition => {{
        const value = viewHostSettings?.[definition.key];
        return typeof value === "boolean" ? value : Boolean(definition.defaultValue);
      }};

      const getHostSettingChoice = (key, defaultValue) => {{
        const value = viewHostSettings?.[key];
        return typeof value === "string" && value.length ? value : String(defaultValue);
      }};

      const getHostSettingText = (key, defaultValue = "") => {{
        const value = viewHostSettings?.[key];
        return typeof value === "string" ? value : String(defaultValue);
      }};

      const getHostSettingNumberText = key => {{
        const value = viewHostSettings?.[key];
        if (typeof value === "string") {{
          return value;
        }}
        return typeof value === "number" && Number.isFinite(value) ? String(value) : "";
      }};

      const hostSettingDisabled = definition => {{
        switch (definition.key) {{
          case "minimizeToTray":
          case "clickTrayToShowHide":
            return !getHostSettingValue({{ key: "tray", defaultValue: true }});
          case "autoStartMinimized":
            return !viewNativeAutoStart;
          case "hardwareVideoAcceleration":
            return !getHostSettingValue({{ key: "hardwareAcceleration", defaultValue: true }});
          case "badgeOnlyForMentions":
            return !getHostSettingValue({{ key: "appBadge", defaultValue: true }});
          case "enableMenu":
            return getHostSettingValue({{
              key: "customTitleBar",
              defaultValue: String(state.versions.platform || "").toLowerCase() === "windows",
            }});
          default:
            return false;
        }}
      }};

      const isHostSettingSaving = key => savingHostSettings?.[key] === true;

      const commitHostSetting = async (key, value) => {{
        setViewHostSettings(current => ({{ ...(current || {{}}), [key]: value }}));
        setSavingHostSettings(current => ({{ ...(current || {{}}), [key]: true }}));

        try {{
          const next = await persistHostSetting(key, value);
          setViewHostSettings(next ? {{ ...next }} : (state.hostSettings ? {{ ...state.hostSettings }} : {{}}));
          return next;
        }} catch (error) {{
          setViewHostSettings(state.hostSettings ? {{ ...state.hostSettings }} : {{}});
          throw error;
        }} finally {{
          setSavingHostSettings(current => {{
            const next = {{ ...(current || {{}}) }};
            delete next[key];
            return next;
          }});
        }}
      }};

      const commitNativeAutoStart = async enabled => {{
        setViewNativeAutoStart(enabled);
        setSavingNativeAutoStart(true);

        try {{
          const next = await persistNativeAutoStart(enabled);
          setViewNativeAutoStart(next === true);
          return next;
        }} catch (error) {{
          setViewNativeAutoStart(state.nativeAutoStartEnabled === true);
          throw error;
        }} finally {{
          setSavingNativeAutoStart(false);
        }}
      }};

      const updateLocalHostSetting = (key, value) => {{
        setViewHostSettings(current => ({{ ...(current || {{}}), [key]: value }}));
      }};

      const scrollToSettingsSection = sectionKey => {{
        const target = document.querySelector(`[data-equirust-section="${{sectionKey}}"]`);
        if (target instanceof HTMLElement) {{
          target.scrollIntoView({{ behavior: "smooth", block: "start" }});
        }}
      }};

      const renderSwitchControl = (checked, disabled, saving, onToggle) =>
        h(
          "div",
          {{
            className: "equirust-settings__card-side",
          }},
          h(
            "button",
            {{
              type: "button",
              className: "equirust-settings__switch",
              role: "switch",
              "aria-checked": checked,
              "data-checked": checked ? "true" : "false",
              disabled,
              onClick: () => {{
                if (disabled) return;
                onToggle(!checked);
              }},
            }},
            h("span", {{
              className: "equirust-settings__switch-track",
              "aria-hidden": "true",
            }}),
            h(
              "span",
              {{
                className: "equirust-settings__switch-label",
              }},
              checked ? "On" : "Off"
            )
          ),
          h(
            "span",
            {{
              className: "equirust-settings__saving",
            }},
            saving ? "Saving" : "\u00A0"
          )
        );

      const renderSettingCard = definition => {{
        const checked = getHostSettingValue(definition);
        const disabled = hostSettingDisabled(definition);
        const saving = isHostSettingSaving(definition.key);

        return h(
          "div",
          {{
            key: definition.key,
            className: "equirust-settings__card",
          }},
          h(
            "div",
            {{
              className: "equirust-settings__copy",
            }},
            h(
              "div",
              {{
                className: "equirust-settings__title-row",
              }},
              h(
                "span",
                {{
                  className: "equirust-settings__title",
                }},
                definition.title
              ),
              definition.restartRequired
                ? h(
                    "span",
                    {{
                      className: "equirust-settings__badge",
                    }},
                    "Restart"
                  )
                : null
            ),
            h(
              "p",
              {{
                className: "equirust-settings__description",
              }},
              definition.description
            )
          ),
          renderSwitchControl(checked, disabled, saving, nextChecked => {{
            commitHostSetting(definition.key, nextChecked).catch(error => {{
              console.error("[Equirust]", error);
              forceRender(version => version + 1);
            }});
          }})
        );
      }};

      const renderStartupCard = () => {{
        if (!supportsNativeAutoStart()) return null;

        return h(
          "div",
          {{
            className: "equirust-settings__card",
          }},
          h(
            "div",
            {{
              className: "equirust-settings__copy",
            }},
            h(
              "div",
              {{
                className: "equirust-settings__title-row",
              }},
              h(
                "span",
                {{
                  className: "equirust-settings__title",
                }},
                "Start With System"
              )
            ),
            h(
              "p",
              {{
                className: "equirust-settings__description",
              }},
              "Start Equirust automatically when you sign in."
            )
          ),
          renderSwitchControl(
            viewNativeAutoStart === true,
            false,
            savingNativeAutoStart,
            checked => {{
              commitNativeAutoStart(checked).catch(error => {{
                console.error("[Equirust]", error);
                forceRender(version => version + 1);
              }});
            }}
          )
        );
      }};

      const renderTransparencyCard = () => {{
        if (!supportsWindowsTransparency()) return null;

        const value = getHostSettingChoice("transparencyOption", "none");
        const options = [
          {{
            value: "none",
            label: "None",
          }},
          {{
            value: "mica",
            label: "Mica",
          }},
          {{
            value: "tabbed",
            label: "Tabbed",
          }},
          {{
            value: "acrylic",
            label: "Acrylic",
          }},
        ];

        return h(
          "label",
          {{
            className: "equirust-settings__card",
            key: "transparencyOption",
          }},
          h(
            "div",
            {{
              className: "equirust-settings__copy",
            }},
            h(
              "div",
              {{
                className: "equirust-settings__title-row",
              }},
              h(
                "span",
                {{
                  className: "equirust-settings__title",
                }},
                "Window Transparency"
              ),
              h(
                "span",
                {{
                  className: "equirust-settings__badge",
                }},
                "Restart"
              )
            ),
            h(
              "p",
              {{
                className: "equirust-settings__description",
              }},
              "Use a Windows backdrop effect behind the Discord window."
            )
          ),
          h(
            "select",
            {{
              className: "equirust-settings__select",
              value,
              onChange: event => {{
                commitHostSetting("transparencyOption", event.currentTarget.value).catch(error => {{
                  console.error("[Equirust]", error);
                  forceRender(version => version + 1);
                }});
              }},
            }},
            options.map(option =>
              h(
                "option",
                {{
                  key: option.value,
                  value: option.value,
                }},
                option.label
              )
            )
          )
        );
      }};

      const renderRichPresenceSection = () => {{
        return h(
          "section",
          {{
            className: "equirust-settings__section",
            "data-equirust-section": "rich-presence",
          }},
          h(
            "h3",
            {{
              className: "equirust-settings__section-title",
            }},
            "Rich Presence"
          ),
          h(
            "p",
            {{
              className: "equirust-settings__section-copy",
            }},
            "Control how games and apps show up in Discord. Use Advanced diagnostics only when troubleshooting."
          ),
          h(
            "div",
            {{
              className: "equirust-settings__grid",
            }},
            renderSettingCard({{
              key: "arRpc",
              title: "Enable Rich Presence",
              description: "Show supported games and apps in Discord.",
              defaultValue: false,
            }}),
            renderSettingCard({{
              key: "arRpcProcessScanning",
              title: "Process Scanning",
              description: "Detect running apps and games automatically.",
              defaultValue: true,
            }})
          )
        );
      }};

      const renderUpdaterSection = () => {{
        const hostUpdate = state.hostUpdateStatus;
        const runtimeUpdate = state.runtimeUpdateStatus;
        const download = state.hostUpdateDownloadState;
        const downloadPhase = String(download?.phase || "idle");
        const downloadBusy = downloadPhase === "downloading" || downloadPhase === "launching";

        const renderUpdateCard = (title, update, options = {{}}) => {{
          const latestLabel =
            title === "Equicord Runtime"
              ? formatRuntimeVersionLabel(update?.latestVersion || update?.releaseName || "Unknown")
              : update?.latestVersion || update?.releaseName || "Unknown";
          const currentLabel =
            title === "Equicord Runtime"
              ? formatRuntimeVersionLabel(
                  update?.currentVersion ||
                    (title === "Equirust Host" ? state.versions?.equirust : "Unknown") ||
                    "Unknown"
                )
              : update?.currentVersion ||
                (title === "Equirust Host" ? state.versions?.equirust : "Unknown") ||
                "Unknown";
          const error = update?.error;
          const configured = update?.configured !== false;
        const updateReady = update?.updateAvailable === true;
          const releaseNotes = (() => {{
            if (typeof update?.releaseNotes !== "string") {{
              return [];
            }}

            const trimmed = update.releaseNotes.trim();
            if (!trimmed) {{
              return [];
            }}

            return trimmed
              .split(/\r?\n/)
              .map(line => String(line || "").trim())
              .filter(Boolean)
              .slice(0, 3);
          }})();
          const summary = !update
            ? "Checking for updates."
            : !configured
              ? "Current version: " +
                currentLabel +
                ". This update track is not configured yet."
              : updateReady
                ? "A newer release is available. Current: " +
                  currentLabel +
                  ". Latest: " +
                  latestLabel +
                  "."
                : "Current version: " +
                  currentLabel +
                  ". " +
                  (error ? "Update check failed." : "You are on the latest checked release.");

          return h(
            "div",
            {{
              className: "equirust-settings__section",
              key: title,
            }},
            h(
              "h4",
              {{
                className: "equirust-settings__section-title",
                style: {{ marginBottom: "8px" }},
              }},
              title
            ),
            h(
              "p",
              {{
                className: "equirust-settings__section-copy",
              }},
              summary
            ),
            releaseNotes.length
              ? h(
                  "div",
                  {{
                    className: "equirust-settings__update-notes",
                  }},
                  releaseNotes.map((line, index) =>
                    h(
                      "p",
                      {{
                        key: title + "-update-note-" + index,
                        className: "equirust-settings__hint",
                      }},
                      line
                    )
                  )
                )
              : null,
            error
              ? h(
                  "p",
                  {{
                    className: "equirust-settings__hint",
                  }},
                  error
                )
              : null,
            update?.ignored
              ? h(
                  "p",
                  {{
                    className: "equirust-settings__hint",
                  }},
                  "This release is currently ignored."
                )
              : null,
            update?.snoozed
              ? h(
                  "p",
                  {{
                    className: "equirust-settings__hint",
                  }},
                  "Update prompts are currently snoozed for one day."
                )
              : null,
            options.statusHint
              ? h(
                  "p",
                  {{
                    className: "equirust-settings__hint",
                  }},
                  options.statusHint
                )
              : null,
            h(
              "div",
              {{
                className: "equirust-settings__actions",
              }},
              h(
                "button",
                {{
                  type: "button",
                  className: "equirust-settings__action",
                  onClick: () => {{
                    options.primaryAction?.().catch(error => console.error("[Equirust]", error));
                  }},
                  disabled: options.primaryDisabled === true,
                }},
                options.primaryLabel
              ),
              h(
                "button",
                {{
                  type: "button",
                  className: "equirust-settings__action equirust-settings__action--secondary",
                  onClick: () => {{
                    options.refreshAction?.().catch(error => console.error("[Equirust]", error));
                  }},
                  disabled: options.refreshDisabled === true,
                }},
                "Check Again"
              ),
              updateReady
                ? h(
                    "button",
                    {{
                      type: "button",
                      className: "equirust-settings__action equirust-settings__action--secondary",
                      onClick: () => {{
                        options.snoozeAction?.().catch(error => console.error("[Equirust]", error));
                      }},
                      disabled: options.secondaryDisabled === true,
                    }},
                    "Snooze 1 Day"
                  )
                : null,
              updateReady && update?.latestVersion
                ? h(
                    "button",
                    {{
                      type: "button",
                      className: "equirust-settings__action equirust-settings__action--secondary",
                      onClick: () => {{
                        options.ignoreAction?.(update.latestVersion).catch(error => console.error("[Equirust]", error));
                      }},
                      disabled: options.secondaryDisabled === true,
                    }},
                    "Ignore This Release"
                  )
                : null
            )
          );
        }};

        const hostStatusHint = downloadBusy
          ? downloadPhase === "launching"
            ? "Launching the downloaded installer."
            : "Downloading update: " + Math.round(Number(download?.percent || 0)) + "%"
          : downloadPhase === "launched"
            ? "The installer has been launched."
            : download?.error || null;
        const hostConfigured = hostUpdate?.configured !== false;
        const hostUpdateReady = hostUpdate?.updateAvailable === true;
        const hostInstallReady = hostUpdateReady && Boolean(hostUpdate?.downloadUrl);
        const runtimeSource = String(state.fileManagerState?.runtimeSource || "");
        const runtimeUpdateAffectsActive =
          runtimeSource === "" ||
          runtimeSource === "managed-equicord-cache";
        const runtimeStatusHint =
          runtimeSource === "custom-dir"
            ? "Runtime updates do not replace your custom runtime folder."
            : runtimeSource === "env-override"
              ? "Runtime updates do not replace the folder set by EQUIRUST_VENCORD_DIST_DIR."
              : null;
        const runtimePrimaryAction =
          runtimeUpdate?.updateAvailable === true && runtimeUpdateAffectsActive
            ? installRuntimeUpdate
            : openRuntimeUpdate;
        const runtimePrimaryLabel =
          runtimeUpdate?.updateAvailable === true
            ? (runtimeUpdateAffectsActive ? "Install Runtime Update" : "View Runtime Release")
            : "View Runtime Release";

        return h(
          "section",
          {{
            className: "equirust-settings__section",
          }},
          h(
            "h3",
            {{
              className: "equirust-settings__section-title",
            }},
            "Updates"
          ),
          h(
            "p",
            {{
              className: "equirust-settings__section-copy",
            }},
            "Equirust host updates and linked Equicord runtime updates are tracked separately, but they live in the same client."
          ),
          h(
            "div",
            {{
              className: "equirust-settings__grid",
            }},
            renderUpdateCard("Equirust Host", hostUpdate, {{
              primaryAction: hostInstallReady ? installHostUpdate : openHostUpdate,
              primaryDisabled: downloadBusy || hostConfigured === false,
              primaryLabel: hostConfigured === false
                ? "Host Feed Pending"
                : hostInstallReady
                  ? (downloadBusy ? "Downloading Update" : "Install Update")
                  : (hostUpdateReady ? "Open Release" : "View Releases"),
              refreshAction: refreshHostUpdateStatus,
              refreshDisabled: downloadBusy,
              snoozeAction: snoozeHostUpdate,
              ignoreAction: ignoreHostUpdate,
              secondaryDisabled: downloadBusy,
              statusHint: hostStatusHint,
            }}),
            renderUpdateCard("Equicord Runtime", runtimeUpdate, {{
              primaryAction: runtimePrimaryAction,
              primaryDisabled: false,
              primaryLabel: runtimePrimaryLabel,
              refreshAction: refreshRuntimeUpdateStatus,
              snoozeAction: snoozeRuntimeUpdate,
              ignoreAction: ignoreRuntimeUpdate,
              statusHint: runtimeStatusHint,
            }})
          )
        );
      }};

      const renderFileManagerSection = () => {{
        const fileManager = state.fileManagerState;
        const usingCustomDir = fileManager?.usingCustomVencordDir === true;
        const customDir = fileManager?.customVencordDir || null;
        const activeRuntimeDir = fileManager?.activeRuntimeDir || null;
        const runtimeSource = String(fileManager?.runtimeSource || "");
        const runtimeSourceLabel =
          runtimeSource === "env-override"
            ? "environment override"
            : runtimeSource === "custom-dir"
              ? "custom runtime folder"
              : runtimeSource === "managed-equicord-cache"
                ? "managed Equicord runtime cache"
                : "runtime folder";
        const runtimeMessage = usingCustomDir && customDir
          ? "Equirust is using your custom runtime folder. Restart after changing it."
          : runtimeSource === "managed-equicord-cache"
            ? "Equirust is using the managed Equicord runtime cache."
            : runtimeSource === "env-override"
              ? "Equirust is using the runtime folder from EQUIRUST_VENCORD_DIST_DIR."
              : "Equirust is using the current runtime folder. You can override it here or open the user assets folder.";
        const handleAssetAction = (asset, reset) => {{
          chooseUserAsset(asset, reset).catch(error => console.error("[Equirust]", error));
        }};

        return h(
          "section",
          {{
            className: "equirust-settings__section",
          }},
          h(
            "h3",
            {{
              className: "equirust-settings__section-title",
            }},
            "Developer and Assets"
          ),
          h(
            "p",
            {{
              className: "equirust-settings__section-copy",
            }},
            runtimeMessage
          ),
          activeRuntimeDir
            ? h(
                "p",
                {{
                  className: "equirust-settings__hint",
                }},
                `${{runtimeSourceLabel}}: ${{activeRuntimeDir}}`
              )
            : null,
          h(
            "div",
            {{
              className: "equirust-settings__actions",
            }},
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action",
                onClick: () => {{
                  openUserAssetsFolder().catch(error => console.error("[Equirust]", error));
                }},
              }},
              "Open User Assets"
            ),
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action",
                onClick: () => {{
                  selectCustomVencordDir(false).catch(error => console.error("[Equirust]", error));
                }},
              }},
              "Change Custom Dir"
            ),
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {{
                  showCustomVencordDir().catch(error => console.error("[Equirust]", error));
                }},
                disabled: !usingCustomDir,
              }},
              "Open Custom Dir"
            ),
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {{
                  selectCustomVencordDir(true).catch(error => console.error("[Equirust]", error));
                }},
                disabled: !usingCustomDir,
              }},
              "Reset Custom Dir"
            )
          ),
          h(
            "div",
            {{
              className: "equirust-settings__actions",
              style: {{ marginTop: "10px" }},
            }},
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {{
                  handleAssetAction("tray", false);
                }},
              }},
              "Set Tray Icon"
            ),
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {{
                  handleAssetAction("trayUnread", false);
                }},
              }},
              "Set Unread Tray Icon"
            ),
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {{
                  handleAssetAction("tray", true);
                  handleAssetAction("trayUnread", true);
                  handleAssetAction("traySpeaking", true);
                  handleAssetAction("trayIdle", true);
                  handleAssetAction("trayMuted", true);
                  handleAssetAction("trayDeafened", true);
                }},
              }},
              "Reset Asset Overrides"
            )
          ),
          h(
            "div",
            {{
              className: "equirust-settings__actions",
              style: {{ marginTop: "10px" }},
            }},
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {{
                  openDebugPage("gpu").catch(error => console.error("[Equirust]", error));
                }},
              }},
              "Open GPU Debug"
            ),
            h(
              "button",
              {{
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {{
                  openDebugPage("webrtc-internals").catch(error => console.error("[Equirust]", error));
                }},
              }},
              "Open WebRTC Internals"
            )
          )
        );
      }};

      const getSettingDefinition = key =>
        desktopSettingDefinitions.find(definition => definition.key === key);

      const renderOverviewMetric = (label, value, copy) =>
        h(
          "div",
          {{
            className: "equirust-settings__metric",
            key: label,
          }},
          h(
            "span",
            {{
              className: "equirust-settings__metric-label",
            }},
            label
          ),
          h(
            "span",
            {{
              className: "equirust-settings__metric-value",
            }},
            value
          ),
          h(
            "p",
            {{
              className: "equirust-settings__metric-copy",
            }},
            copy
          )
        );

      const quickControlDefinitions = [
        {{
          key: "arRpc",
          title: "Rich Presence",
          description: "Show game and app activity in Discord.",
          defaultValue: false,
        }},
        getSettingDefinition("customTitleBar"),
        getSettingDefinition("hardwareAcceleration"),
        getSettingDefinition("tray"),
      ].filter(Boolean);
      const performanceDefinitions = [
        "hardwareAcceleration",
        "hardwareVideoAcceleration",
        "appBadge",
        "enableTaskbarFlashing",
      ]
        .map(getSettingDefinition)
        .filter(Boolean);
      const preferenceDefinitions = [
        "customTitleBar",
        "tray",
        "minimizeToTray",
        "clickTrayToShowHide",
        "staticTitle",
        "mouseSideButtonsNavigation",
        "openLinksWithElectron",
        "middleClickAutoscroll",
        "disableMinSize",
        "badgeOnlyForMentions",
        "enableMenu",
      ]
        .map(getSettingDefinition)
        .filter(Boolean);

      return h(
        "div",
        {{
          className: "vc-equirust-settings-page",
        }},
        h(
          "section",
          {{
            className: "equirust-settings__hero",
          }},
          h(
            "span",
            {{
              className: "equirust-settings__eyebrow",
            }},
            "Equirust Settings"
          ),
          h(
            "h2",
            {{
              className: "equirust-settings__heading",
            }},
            "Equirust"
          ),
          h(
            "p",
            {{
              className: "equirust-settings__lead",
            }},
            "The most important settings are first. Advanced options are below when you need them."
          ),
          h(
            "div",
            {{
              className: "equirust-settings__hero-grid",
            }},
            renderOverviewMetric(
              "Equirust Host",
              String(state.versions?.equirust || "1.0.0"),
              state.hostUpdateStatus?.updateAvailable === true
                ? "A host update is available."
                : "Host build is currently loaded."
            ),
            renderOverviewMetric(
              "Equicord Runtime",
              formatRuntimeVersionLabel(state.runtimeUpdateStatus?.currentVersion || "Unknown"),
              state.runtimeUpdateStatus?.updateAvailable === true
                ? "A linked runtime update is available."
                : "Runtime link is present."
            ),
            renderOverviewMetric(
              "Startup",
              viewNativeAutoStart ? "Enabled" : "Disabled",
              viewNativeAutoStart
                ? "Equirust launches with Windows for this user."
                : "Equirust only starts when you open it."
            )
          ),
          h(
            "div",
            {{
              className: "equirust-settings__jump-nav",
            }},
            [
              ["updates", "Updates"],
              ["startup", "Startup"],
              ["privacy", "Privacy"],
              ["performance", "Performance"],
              ["preferences", "Preferences"],
              ["advanced", "Advanced"],
            ].map(([key, label]) =>
              h(
                "button",
                {{
                  key,
                  type: "button",
                  className: "equirust-settings__jump-button",
                  onClick: () => scrollToSettingsSection(key),
                }},
                label
              )
            )
          )
        ),
        h(
          "section",
          {{
            className: "equirust-settings__section",
            "data-equirust-section": "quick-controls",
          }},
          h(
            "h3",
            {{
              className: "equirust-settings__section-title",
            }},
            "Quick Controls"
          ),
          h(
            "p",
            {{
              className: "equirust-settings__section-copy",
            }},
            "These are the settings most people change."
          ),
          h(
            "div",
            {{
              className: "equirust-settings__grid",
            }},
            supportsNativeAutoStart() ? renderStartupCard() : null,
            quickControlDefinitions.map(renderSettingCard)
          )
        ),
        h(
          "div",
          {{
            "data-equirust-section": "updates",
          }},
          renderUpdaterSection()
        ),
        supportsNativeAutoStart()
          ? h(
              "section",
              {{
                className: "equirust-settings__section",
                "data-equirust-section": "startup",
              }},
              h(
                "h3",
                {{
                  className: "equirust-settings__section-title",
                }},
                "Startup"
              ),
              h(
                "p",
                {{
                  className: "equirust-settings__section-copy",
                }},
                "Choose how Equirust starts with Windows."
              ),
              h(
                "div",
                {{
                  className: "equirust-settings__grid",
                }},
                renderStartupCard(),
                renderSettingCard(getSettingDefinition("autoStartMinimized"))
              )
            )
          : null,
        h(
          "div",
          {{
            "data-equirust-section": "privacy",
          }},
          renderRichPresenceSection()
        ),
        h(
          "section",
          {{
            className: "equirust-settings__section",
            "data-equirust-section": "performance",
          }},
          h(
            "h3",
            {{
              className: "equirust-settings__section-title",
            }},
            "Performance"
          ),
          h(
            "p",
            {{
              className: "equirust-settings__section-copy",
            }},
            "Choose how much work is handled by your GPU and Windows."
          ),
          h(
            "div",
            {{
              className: "equirust-settings__grid",
            }},
            performanceDefinitions.map(renderSettingCard)
          )
        ),
        h(
          "section",
          {{
            className: "equirust-settings__section",
            "data-equirust-section": "preferences",
          }},
          h(
            "h3",
            {{
              className: "equirust-settings__section-title",
            }},
            "Preferences and Visuals"
          ),
          h(
            "p",
            {{
              className: "equirust-settings__section-copy",
            }},
            "Adjust how the app looks and behaves."
          ),
          h(
            "div",
            {{
              className: "equirust-settings__grid",
            }},
            renderTransparencyCard(),
            preferenceDefinitions.map(renderSettingCard)
          )
        ),
        h(
          "details",
          {{
            className: "equirust-settings__details",
            "data-equirust-section": "advanced",
          }},
          h("summary", null, "Advanced"),
          h(
            "p",
            {{
              className: "equirust-settings__details-copy",
            }},
            "Extra tools, folders, and debugging live here."
          ),
          h(
            "div",
            {{
              className: "equirust-settings__details-content",
            }},
            h(
              "section",
              {{
                className: "equirust-settings__section",
              }},
              h(
                "h3",
                {{
                  className: "equirust-settings__section-title",
                }},
                "Themes, QuickCSS, and Plugins"
              ),
              h(
                "p",
                {{
                  className: "equirust-settings__section-copy",
                }},
                  "Use these shortcuts for themes, QuickCSS, and Equicord files."
              ),
              h(
                "div",
                {{
                  className: "equirust-settings__actions",
                }},
                h(
                  "button",
                  {{
                    type: "button",
                    className: "equirust-settings__action",
                    onClick: () => invoke("open_vencord_quick_css"),
                  }},
                  "Open QuickCSS"
                ),
                h(
                  "button",
                  {{
                    type: "button",
                    className: "equirust-settings__action",
                    onClick: () => invoke("open_vencord_themes_folder"),
                  }},
                  "Open Themes Folder"
                ),
                h(
                  "button",
                  {{
                    type: "button",
                    className: "equirust-settings__action",
                    onClick: () => invoke("open_vencord_settings_folder"),
                  }},
                  "Open Equicord Settings Folder"
                )
              )
            ),
            h(
              "section",
              {{
                className: "equirust-settings__section",
              }},
              h(
                "h3",
                {{
                  className: "equirust-settings__section-title",
                }},
                "Diagnostics"
              ),
              h(
                "p",
                {{
                  className: "equirust-settings__section-copy",
                }},
                state.debugBuild
                  ? "Keep this off during normal use. Turn it on only when you need deeper bridge and runtime troubleshooting."
                  : "Verbose diagnostics are available in debug builds only."
              ),
              state.debugBuild
                ? h(
                    "div",
                    {{
                      className: "equirust-settings__grid",
                    }},
                    renderSettingCard(getSettingDefinition("runtimeDiagnostics")),
                    renderSettingCard({{
                      key: "arRpcDebug",
                      title: "Verbose Rich Presence Logs",
                      description: "Write extra Rich Presence details to local logs for troubleshooting.",
                      defaultValue: false,
                    }})
                  )
                : null
            ),
            renderFileManagerSection()
          )
        ),
        h(
          "p",
          {{
            className: "equirust-settings__hint",
            style: {{ marginTop: "18px" }},
          }},
          "Settings marked Restart are stored immediately but still depend on the current Windows-first parity scope."
        )
      );
    }};
  }};

  const installVencordSettingsEntry = () => {{
    if (state.vencordSettingsReady || !isDiscordHost()) return;

    const settingsPlugin = window.Vencord?.Plugins?.plugins?.Settings;
    const React = window.Vencord?.Webpack?.Common?.React;
    if (!settingsPlugin || !React) return;

    installDesktopSettingsStyles();

    const customEntries = settingsPlugin.customEntries;
    if (!Array.isArray(customEntries)) return;

    const entryKey = "equirust_settings";
    const SettingsPage = createDesktopSettingsComponent(React);

    if (!customEntries.some(entry => entry?.key === entryKey)) {{
      customEntries.push({{
        key: entryKey,
        title: "Equirust",
        Component: SettingsPage,
        Icon: () => React.createElement(
          "svg",
          {{
            width: 18,
            height: 18,
            viewBox: "0 0 24 24",
            fill: "none",
            "aria-hidden": "true",
          }},
          React.createElement("path", {{
            d: "M12 2.75 20.25 7v10L12 21.25 3.75 17V7L12 2.75Z",
            stroke: "currentColor",
            strokeWidth: "1.75",
            strokeLinejoin: "round",
          }}),
          React.createElement("path", {{
            d: "M8.5 9.5h7v1.75h-7zm0 3.25h7v1.75h-7z",
            fill: "currentColor",
          }})
        ),
      }});
    }}

    state.vencordSettingsReady = true;
    report(`vencord_settings_registered=true entries=${{customEntries.length}} sections=0`);
  }};

  const stopVencordSettingsWatcher = () => {{
    if (state.vencordSettingsObserver) {{
      state.vencordSettingsObserver.disconnect();
      state.vencordSettingsObserver = null;
    }}
  }};

  const installVencordSettingsWatcher = () => {{
    if (state.vencordSettingsObserver || !isDiscordHost()) return;
    installVencordSettingsEntry();
    if (state.vencordSettingsReady) {{
      return;
    }}

    const observerTarget = document.body || document.documentElement;
    if (!observerTarget) {{
      return;
    }}

    state.vencordSettingsObserver = new MutationObserver(() => {{
      installVencordSettingsEntry();
      if (state.vencordSettingsReady) {{
        stopVencordSettingsWatcher();
      }}
    }});
    state.vencordSettingsObserver.observe(observerTarget, {{
      subtree: true,
      childList: true,
    }});
  }};

  const ready = () => {{
    installVoiceDiagnostics();
    if (installModRuntime) {{
      ensureVoiceToggleBridge();
      installMediaCompatibilityPatches();
      installDisplayMediaCompatibilityPatches();
      installCloudFetchProxy();
      installVencordSettingsWatcher();
    }}
    if (installHostRuntime) {{
      installNotificationSync();
      installTypingIndicator();
      if (shouldUseCustomTitleBar()) {{
        installTitlebar();
        window.__EQUIRUST_TITLEBAR_SYNC__?.();
      }}
    }}
    if (installHostRuntime && installModRuntime) {{
      installVoiceTrayWatcher();
    }}
  }};

  if (installModRuntime) {{
    createVencordNative();
    ensureArrpcBridge();
    installCloudFetchProxy();
    installVencordRuntime();
  }}

  if (document.readyState === "loading") {{
    document.addEventListener("DOMContentLoaded", ready, {{ once: true }});
  }} else {{
    ready();
  }}
}})();
        "###
    ))
}
