(() => {
  if (window.__EQUIRUST_BOOTSTRAPPED__) return;

  const ensureWebStorage = () => {
    const makeMemoryStorage = () => {
      const store = new Map();
      return {
        getItem: key => store.has(String(key)) ? store.get(String(key)) : null,
        setItem: (key, value) => { store.set(String(key), String(value ?? "")); },
        removeItem: key => { store.delete(String(key)); },
        clear: () => { store.clear(); },
        key: n => Array.from(store.keys())[n] ?? null,
        get length() { return store.size; },
      };
    };

    const wrapStorage = real => {
      const mem = makeMemoryStorage();
      return {
        getItem: key => { try { return real.getItem(key); } catch { return mem.getItem(key); } },
        setItem: (key, value) => { try { real.setItem(key, value); } catch {} mem.setItem(key, value); },
        removeItem: key => { try { real.removeItem(key); } catch {} mem.removeItem(key); },
        clear: () => { try { real.clear(); } catch {} mem.clear(); },
        key: n => { try { return real.key(n); } catch { return mem.key(n); } },
        get length() { try { return real.length; } catch { return mem.size; } },
      };
    };

    const patchStorage = name => {
      let real = null;
      try { real = window[name] ?? null; } catch {}
      const value = real !== null ? wrapStorage(real) : makeMemoryStorage();
      try {
        Object.defineProperty(window, name, { value, configurable: false, writable: false });
        return real !== null ? "wrapped" : "memory";
      } catch { return false; }
    };

    const ls = patchStorage("localStorage");
    const ss = patchStorage("sessionStorage");
    window.__EQUIRUST_STORAGE_POLYFILL__ = { localStorage: ls, sessionStorage: ss };
  };

  ensureWebStorage();

  const installBootBackdrop = () => {
    const bootColor = "#050608";
    const bootBackgroundCss =
      "var(--app-background-frame, var(--background-primary, var(--bg-base-primary, #050608)))";
    try {
      const root = document.documentElement;
      if (root) {
        root.style.backgroundColor = bootColor;
        root.style.colorScheme = "dark";
      }

      const applyBodyBackdrop = () => {
        if (!document.body) return;
        document.body.style.backgroundColor = bootColor;
        document.body.style.colorScheme = "dark";
      };

      applyBodyBackdrop();
      document.addEventListener("DOMContentLoaded", applyBodyBackdrop, { once: true });

      if (!document.getElementById("__equirust-boot-style")) {
        const style = document.createElement("style");
        style.id = "__equirust-boot-style";
        style.textContent = `
          :root {
            color-scheme: dark;
            background: ${bootBackgroundCss} !important;
          }
          html, body, #app-mount, [class*="appMount"] {
            background: ${bootBackgroundCss} !important;
            color-scheme: dark;
          }
        `;
        (document.head || root || document.body)?.appendChild(style);
      }
    } catch {}
  };

  installBootBackdrop();

  const seed = __EQUIRUST_SEED_JSON__;
  const vencordRenderer = __EQUIRUST_VENCORD_RENDERER_JSON__;
  const controlRuntime = __EQUIRUST_CONTROL_RUNTIME_JSON__;
  const installHostRuntime = __EQUIRUST_INSTALL_HOST_RUNTIME_JSON__;
  const installModRuntime = __EQUIRUST_INSTALL_MOD_RUNTIME_JSON__;
  const spoofEdgeClientHints = __EQUIRUST_SPOOF_EDGE_CLIENT_HINTS_JSON__;
  const startBootstrap = () => {
    if (window.__EQUIRUST_BOOTSTRAPPED__) {
      return true;
    }

    const internals = window.__TAURI_INTERNALS__;
    if (!internals || typeof internals.invoke !== "function") {
      return false;
    }

    window.__EQUIRUST_BOOTSTRAPPED__ = true;

    const invoke = (cmd, args = {}) => internals.invoke(cmd, args);
    const state = {
    settings: seed.settings || {},
    hostSettings: seed.hostSettings || {},
    hostUpdateStatus: null,
    runtimeUpdateStatus: null,
    hostUpdateDownloadState: null,
    fileManagerState: null,
    debugBuild: seed.debugBuild === true,
    profilingDiagnostics: seed.profilingDiagnostics === true,
    nativeAutoStartEnabled: seed.nativeAutoStartEnabled === true,
    quickCss: typeof seed.quickCss === "string" ? seed.quickCss : "",
    versions: seed.versions || {},
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
    screenShareThumbnailCache: new Map(),
    screenSharePreviewCache: new Map(),
    screenShareEncoderPreviewCache: new Map(),
    nativeGeneratedTrackSupport: null,
    nativeGeneratedTrackSupportPromise: null,
    nativeGeneratedTrackMode: null,
    nativeGeneratedTrackProbeAt: 0,
    nativeGeneratedTrackProbeReason: null,
    desktopStreamVideoWorkerUrl: null,
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
    cloudCspDiagnosticsInstalled: false,
    cloudBackendInfoObserver: null,
    cloudBackendInfoRenderTimer: null,
    typingObserver: null,
    typingPollScheduled: false,
    nativeSdpPatchReady: false,
    nativeAbrReady: false,
    goLiveQualityPatchReady: false,
    pendingScreenShareQuality: null,
  };
  const debugCategoryDefinitions = {
    standard: {
      key: "debugStandardDiagnostics",
      defaultValue: true,
    },
    media: {
      key: "debugMediaDiagnostics",
      defaultValue: true,
    },
  };
  const diagnosticsCategoryForMessage = message => {
    const text = typeof message === "string" ? message : String(message);
    if (
      text.startsWith("desktop_stream") ||
      text.startsWith("display_media") ||
      text.startsWith("media_compat") ||
      text.startsWith("voice_diag") ||
      text.startsWith("sdp_snapshot") ||
      text.startsWith("negotiated_sdp") ||
      text.startsWith("negotiated_send") ||
      text.startsWith("telemetry_fps_cap") ||
      text.startsWith("telemetry_size_cap")
    ) {
      return "media";
    }

    return "standard";
  };
  const debugCategoryEnabled = category => {
    if (state.debugBuild !== true && state.profilingDiagnostics !== true) {
      return false;
    }

    const definition =
      debugCategoryDefinitions[
        category === "media" ? "media" : "standard"
      ] || debugCategoryDefinitions.standard;
    const value = state.hostSettings?.[definition.key];
    return typeof value === "boolean" ? value : Boolean(definition.defaultValue);
  };
  const shouldEmitMediaDiagnostics = () => debugCategoryEnabled("media");
  const runtimeLogQueue = [];
  let runtimeLogFlushTimer = null;

  const flushRuntimeLogQueue = force => {
    if (runtimeLogQueue.length === 0) {
      return Promise.resolve();
    }

    const messages = runtimeLogQueue.splice(0, runtimeLogQueue.length);
    const promise = invoke("log_client_runtime_batch", { messages }).catch(() => {});
    return force ? promise : promise.then(() => undefined);
  };

  const scheduleRuntimeLogFlush = () => {
    if (runtimeLogFlushTimer !== null) {
      return;
    }

    runtimeLogFlushTimer = window.setTimeout(() => {
      runtimeLogFlushTimer = null;
      void flushRuntimeLogQueue(false);
    }, 150);
  };

  const report = (message, options = {}) => {
    const category = (() => {
      if (options && typeof options.category === "string") {
        return options.category === "media" ? "media" : "standard";
      }

      return diagnosticsCategoryForMessage(message);
    })();

    if (!debugCategoryEnabled(category)) {
      return Promise.resolve();
    }

    runtimeLogQueue.push(
      typeof message === "string" ? message : String(message)
    );

    const force = options && options.force === true;
    if (force || runtimeLogQueue.length >= 64) {
      if (runtimeLogFlushTimer !== null) {
        window.clearTimeout(runtimeLogFlushTimer);
        runtimeLogFlushTimer = null;
      }
      return flushRuntimeLogQueue(true);
    }

    scheduleRuntimeLogFlush();
    return Promise.resolve();
  };
  const reportMedia = (message, options = {}) =>
    report(message, { ...options, category: "media" });

  const logStoragePolyfillState = () => {
    const polyfill = window.__EQUIRUST_STORAGE_POLYFILL__;
    if (!polyfill) return;
    void report(
      "storage_polyfill=" + JSON.stringify(polyfill),
      { force: true }
    );
  };
  logStoragePolyfillState();

  let settingsSyncScheduled = false;
  const notifySettingsSync = () => {
    if (settingsSyncScheduled) {
      return;
    }

    settingsSyncScheduled = true;
    const flush = () => {
      settingsSyncScheduled = false;
      try {
        window.__EQUIRUST_SETTINGS_SYNC__?.();
      } catch (error) {
        console.error("[Equirust]", error);
      }
    };

    if (typeof window.requestAnimationFrame === "function") {
      window.requestAnimationFrame(flush);
    } else {
      window.setTimeout(flush, 0);
    }
  };

  const scheduleDeferredTask = (task, options = {}) => {
    if (typeof task !== "function") {
      return;
    }

    const delay = Math.max(0, Number(options.delay || 0) || 0);
    const timeout = Math.max(250, Number(options.timeout || 1500) || 1500);
    const run = () => {
      try {
        const result = task();
        if (result && typeof result.catch === "function") {
          result.catch(error => console.error("[Equirust]", error));
        }
      } catch (error) {
        console.error("[Equirust]", error);
      }
    };

    if (typeof window.requestIdleCallback === "function") {
      window.requestIdleCallback(
        () => {
          if (delay > 0) {
            window.setTimeout(run, delay);
          } else {
            run();
          }
        },
        { timeout }
      );
      return;
    }

    window.setTimeout(run, delay > 0 ? delay : 32);
  };

  const scheduleAfterFirstPaint = (task, options = {}) => {
    if (typeof task !== "function") {
      return;
    }

    const run = () => scheduleDeferredTask(task, options);
    if (typeof window.requestAnimationFrame === "function") {
      window.requestAnimationFrame(() => {
        window.requestAnimationFrame(run);
      });
      return;
    }

    run();
  };

  const whenDocumentBodyReady = task => {
    if (typeof task !== "function") {
      return;
    }

    if (document.body) {
      task();
      return;
    }

    if (typeof MutationObserver !== "function" || !document.documentElement) {
      document.addEventListener("DOMContentLoaded", task, { once: true });
      return;
    }

    let done = false;
    let observer = null;
    const run = () => {
      if (done || !document.body) {
        return;
      }
      done = true;
      observer?.disconnect();
      observer = null;
      task();
    };

    observer = new MutationObserver(run);
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
    });
    document.addEventListener("DOMContentLoaded", run, { once: true });
    run();
  };

  window.addEventListener("beforeunload", () => {
    void report(
      "page_lifecycle=" +
        JSON.stringify({
          event: "beforeunload",
          href: window.location.href,
          readyState: document.readyState,
          visibilityState: document.visibilityState,
        }),
      { category: "standard", force: true }
    );
    if (runtimeLogFlushTimer !== null) {
      window.clearTimeout(runtimeLogFlushTimer);
      runtimeLogFlushTimer = null;
    }
    void flushRuntimeLogQueue(true);
  });

  const installPageLifecycleDiagnostics = () => {
    if (window.__EQUIRUST_PAGE_LIFECYCLE_DIAGNOSTICS__) {
      return;
    }
    window.__EQUIRUST_PAGE_LIFECYCLE_DIAGNOSTICS__ = true;
    let lastResizeObserverErrorAt = 0;
    const describeLifecycleReason = value => {
      if (value == null) return null;
      if (typeof value === "string") return value;
      if (value instanceof Error) {
        const stack =
          typeof value.stack === "string"
            ? value.stack.split("\n").slice(0, 6).join("\n")
            : null;
        try {
          return JSON.stringify({
            name: value.name || "Error",
            message: value.message || value.name || "Error",
            stack,
          });
        } catch {}
        return value.message || value.name || "Error";
      }
      try {
        return JSON.stringify(value);
      } catch {}
      try {
        return String(value);
      } catch {}
      return "<unknown>";
    };
    const logLifecycle = (event, extra = {}) =>
      void report(
        "page_lifecycle=" +
          JSON.stringify({
            event,
            href: window.location.href,
            readyState: document.readyState,
            visibilityState: document.visibilityState,
            ...extra,
          }),
        { category: "standard" }
      );

    window.addEventListener("pageshow", event => {
      logLifecycle("pageshow", { persisted: event?.persisted === true });
    });
    window.addEventListener("pagehide", event => {
      logLifecycle("pagehide", { persisted: event?.persisted === true });
    });
    document.addEventListener("visibilitychange", () => {
      logLifecycle("visibilitychange");
    });
    window.addEventListener("freeze", () => {
      logLifecycle("freeze");
    });
    window.addEventListener("resume", () => {
      logLifecycle("resume");
    });
    window.addEventListener("error", event => {
      const message = event?.message ? String(event.message) : null;
      if (message === "ResizeObserver loop completed with undelivered notifications.") {
        const now =
          typeof performance !== "undefined" && typeof performance.now === "function"
            ? performance.now()
            : Date.now();
        if (now - lastResizeObserverErrorAt < 2000) {
          return;
        }
        lastResizeObserverErrorAt = now;
      }
      const target = event?.target;
      const isResourceError =
        target &&
        target !== window &&
        typeof target?.tagName === "string";
      logLifecycle("error", {
        message,
        filename: event?.filename ? String(event.filename) : null,
        lineno: Number(event?.lineno || 0) || null,
        colno: Number(event?.colno || 0) || null,
        resourceTag: isResourceError ? String(target.tagName || "").toLowerCase() : null,
      });
    }, true);
    window.addEventListener("unhandledrejection", event => {
      const reason = describeLifecycleReason(event?.reason);
      logLifecycle("unhandledrejection", {
        reason,
      });
    });
  };
  installPageLifecycleDiagnostics();

  window.__EQUIRUST_BRIDGE__ = { invoke, state };

  const isDiscordHost = () =>
    /(^|\.)discord\.com$/i.test(window.location.hostname) ||
    /(^|\.)discordapp\.com$/i.test(window.location.hostname);

  const getConfiguredCloudOrigin = () => {
    const cloudUrl =
      window.Vencord?.Settings?.cloud?.url ??
      state.settings?.cloud?.url ??
      state.hostSettings?.cloud?.url;
    if (typeof cloudUrl !== "string" || !cloudUrl.trim()) {
      return null;
    }

    try {
      const parsed = new URL(cloudUrl, window.location.href);
      const scheme = String(parsed.protocol || "").toLowerCase();
      if (scheme === "https:" || scheme === "http:") {
        return parsed.origin;
      }
      return null;
    } catch {
      return null;
    }
  };

  const shouldProxyCloudRequest = targetUrl => {
    if (!(targetUrl instanceof URL)) {
      return false;
    }

    if (!/^https?:$/i.test(targetUrl.protocol)) {
      return false;
    }

    if (targetUrl.origin === window.location.origin) {
      return false;
    }

    const host = String(targetUrl.hostname || "").toLowerCase();

    const configuredOrigin = getConfiguredCloudOrigin();

    // The renderer shim only intercepts the official hosted cloud APIs plus the exact backend
    // origin the user configured in the Cloud settings tab. This bridge exists because Discord's
    // renderer can hit CSP/CORS when cloud OAuth/settings requests leave discord.com; the host
    // replays those requests natively and returns a synthetic Response.
    return (
      (configuredOrigin !== null && targetUrl.origin === configuredOrigin) ||
      host === "cloud.equicord.org" ||
      host === "api.vencord.dev"
    );
  };

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

  const normalizeLegacyPlatform = platform => {
    const raw = String(platform || "").toLowerCase();
    if (raw.includes("windows")) return "win32";
    if (raw.includes("mac")) return "darwin";
    if (raw.includes("linux")) return "linux";
    return raw || "unknown";
  };

  const safeCall = async fn => {
    try {
      return await fn();
    } catch (error) {
      console.error("[Equirust]", error);
      throw error;
    }
  };

  const serializeIpcError = error => {
    if (error instanceof Error) {
      return {
        name: error.name,
        message: error.message,
        stack: error.stack,
      };
    }

    return {
      name: "Error",
      message: typeof error === "string" ? error : String(error),
    };
  };

  const wrapIpcResult = async fn => {
    try {
      return {
        ok: true,
        value: await fn(),
      };
    } catch (error) {
      console.error("[Equirust]", error);
      return {
        ok: false,
        error: serializeIpcError(error),
      };
    }
  };

  const encodeBytesToBase64 = bytes => {
    let binary = "";
    const chunkSize = 0x8000;
    for (let index = 0; index < bytes.length; index += chunkSize) {
      const slice = bytes.subarray(index, index + chunkSize);
      binary += String.fromCharCode(...slice);
    }
    return window.btoa(binary);
  };

  const decodeBase64ToBytes = value => {
    if (typeof value !== "string" || !value.length) {
      return new Uint8Array();
    }

    const binary = window.atob(value);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  };

  const currentWebviewWindowTarget = () => {
    const label = String(internals.metadata?.currentWindow?.label || "main");
    return {
      kind: "WebviewWindow",
      label,
    };
  };

  const listenTauriEvent = async (event, handler, target = currentWebviewWindowTarget()) => {
    if (typeof internals.transformCallback !== "function") {
      throw new Error("Tauri event callbacks are unavailable.");
    }

    const callback = internals.transformCallback(payload => {
      try {
        handler(payload);
      } catch (error) {
        console.error("[Equirust]", error);
      }
    });
    const eventId = await invoke("plugin:event|listen", {
      event,
      target,
      handler: callback,
    });

    return async () => {
      try {
        window.__TAURI_EVENT_PLUGIN_INTERNALS__?.unregisterListener?.(event, eventId);
      } catch (error) {
        console.warn("[Equirust] Failed to unregister event listener", error);
      }

      await invoke("plugin:event|unlisten", {
        event,
        eventId,
      }).catch(() => {});
    };
  };

  const applyEdgeClientHintsSpoof = () => {
    if (!spoofEdgeClientHints) return;

    const version = String(state.versions.webview || "").trim();
    const majorVersion = version.split(".")[0] || "145";
    const fullVersion = version || `${majorVersion}.0.0.0`;
    const platform = (() => {
      const raw = String(state.versions.platform || "windows").toLowerCase();
      if (raw.includes("mac")) return "macOS";
      if (raw.includes("linux")) return "Linux";
      return "Windows";
    })();
    const fullVersionList = Object.freeze([
      Object.freeze({ brand: "Not=A?Brand", version: "99" }),
      Object.freeze({ brand: "Chromium", version: fullVersion }),
      Object.freeze({ brand: "Microsoft Edge", version: fullVersion }),
    ]);
    const brands = Object.freeze(
      fullVersionList.map(item =>
        Object.freeze({
          brand: item.brand,
          version: item.brand === "Not=A?Brand" ? "99" : majorVersion,
        })
      )
    );
    const uaData = Object.freeze({
      brands,
      mobile: false,
      platform,
      toJSON() {
        return { brands, mobile: false, platform };
      },
      async getHighEntropyValues(hints) {
        const values = {
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
        };

        if (!Array.isArray(hints)) {
          return values;
        }

        return hints.reduce((result, hint) => {
          if (Object.prototype.hasOwnProperty.call(values, hint)) {
            result[hint] = values[hint];
          }
          return result;
        }, {});
      },
    });

    try {
      Object.defineProperty(window.Navigator.prototype, "userAgentData", {
        configurable: true,
        enumerable: true,
        get() {
          return uaData;
        },
      });
      report("ua_data_spoofed=true profile=edge-chromium");
    } catch (error) {
      report(`ua_data_spoof_failed=${error && error.message ? error.message : String(error)}`, { force: true });
    }
  };
  applyEdgeClientHintsSpoof();

  const collectVoiceDiagnostics = () => {
    const senderAudioCaps = (() => {
      try {
        return window.RTCRtpSender?.getCapabilities?.("audio")?.codecs?.length ?? 0;
      } catch {
        return -1;
      }
    })();
    const senderVideoCaps = (() => {
      try {
        return window.RTCRtpSender?.getCapabilities?.("video")?.codecs?.length ?? 0;
      } catch {
        return -1;
      }
    })();
    const userAgentBrands = (() => {
      try {
        return navigator.userAgentData?.brands?.map(brand => `${brand.brand}/${brand.version}`).join(",") || "";
      } catch {
        return "";
      }
    })();

    return {
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
    };
  };

  const reportVoiceDiagnostics = reason => {
    const diagnostics = collectVoiceDiagnostics();
    reportMedia(`voice_diag reason=${reason} data=${JSON.stringify(diagnostics)}`);
  };

  const installVoiceDiagnostics = () => {
    if (!isDiscordHost() || !shouldEmitMediaDiagnostics()) return;
    reportVoiceDiagnostics("bootstrap");
    window.setTimeout(() => reportVoiceDiagnostics("settled"), 2500);
  };

  const getAutomaticGainControlPreference = () => {
    try {
      return window.Vencord?.Webpack?.Common?.MediaEngineStore?.getAutomaticGainControl?.();
    } catch {
      return undefined;
    }
  };

