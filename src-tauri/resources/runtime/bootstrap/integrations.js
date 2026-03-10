  const installCloudFetchProxy = () => {
    if (state.cloudFetchProxyInstalled || !isDiscordHost()) return;
    if (typeof window.fetch !== "function" || typeof window.Response !== "function") return;

    if (!state.cloudCspDiagnosticsInstalled) {
      state.cloudCspDiagnosticsInstalled = true;
      document.addEventListener(
        "securitypolicyviolation",
        event => {
          const configuredOrigin = getConfiguredCloudOrigin();
          const blockedUri = String(event?.blockedURI || "");
          const effectiveDirective = String(event?.effectiveDirective || "");
          const violatedDirective = String(event?.violatedDirective || "");
          const interested =
            blockedUri.includes("cloud.equicord.org") ||
            blockedUri.includes("api.vencord.dev") ||
            (configuredOrigin !== null && blockedUri.includes(configuredOrigin)) ||
            effectiveDirective.includes("connect-src") ||
            violatedDirective.includes("connect-src");
          if (!interested) return;
          report(
            "cloud_csp_violation=" +
              JSON.stringify({
                blockedUri,
                effectiveDirective,
                violatedDirective,
                originalPolicy: String(event?.originalPolicy || ""),
                sourceFile: String(event?.sourceFile || ""),
                lineNumber: Number(event?.lineNumber || 0) || null,
                columnNumber: Number(event?.columnNumber || 0) || null,
                disposition: String(event?.disposition || ""),
              }),
            { force: true }
          );
        },
        true
      );
    }

    const originalFetch = window.fetch.bind(window);
    // Cloud OAuth/settings requests are normal page fetches from Discord's renderer, but Discord's
    // CSP/CORS policy can block direct access to the cloud API. This shim only intercepts the
    // official cloud APIs plus the exact configured backend origin, forwards them to the Rust
    // host, and rebuilds a Response so the in-page client can keep using fetch semantics.
    const proxiedFetch = async function(input, init) {
      const request = new Request(input, init);
      const targetUrl = (() => {
        try {
          return new URL(request.url, window.location.href);
        } catch {
          return null;
        }
      })();

      if (!targetUrl || !shouldProxyCloudRequest(targetUrl)) {
        return originalFetch(input, init);
      }

      const method = String(request.method || "GET").toUpperCase();
      const headers = {};
      request.headers.forEach((value, name) => {
        headers[name] = value;
      });

      let bodyBase64 = null;
      if (method !== "GET" && method !== "HEAD") {
        try {
          const bodyBuffer = await request.clone().arrayBuffer();
          const bodyBytes = new Uint8Array(bodyBuffer);
          if (bodyBytes.byteLength > 0) {
            bodyBase64 = encodeBytesToBase64(bodyBytes);
          }
        } catch (bodyError) {
          report(
            "cloud_fetch_proxy_body_read_failed=" +
              JSON.stringify({
                url: targetUrl.toString(),
                method,
                name:
                  bodyError && typeof bodyError.name === "string"
                    ? bodyError.name
                    : "Error",
                message:
                  bodyError && typeof bodyError.message === "string"
                    ? bodyError.message
                    : String(bodyError),
              }),
            { force: true }
          );
        }
      }

      report(
        "cloud_fetch_proxy_request=" +
          JSON.stringify({
            method,
            url: targetUrl.toString(),
            mode: request.mode || null,
            credentials: request.credentials || null,
            cache: request.cache || null,
            redirect: request.redirect || null,
            headerCount: Object.keys(headers).length,
            hasBody: !!bodyBase64,
          })
      );

      let proxiedResponse;
      try {
        proxiedResponse = await invoke("proxy_http_request", {
          request: {
            url: targetUrl.toString(),
            method,
            headers,
            bodyBase64,
          },
        });
      } catch (proxyError) {
        report(
          "cloud_fetch_proxy_failed=" +
            JSON.stringify({
              url: targetUrl.toString(),
              method,
              name:
                proxyError && typeof proxyError.name === "string"
                  ? proxyError.name
                  : "Error",
              message:
                proxyError && typeof proxyError.message === "string"
                  ? proxyError.message
                  : String(proxyError),
            }),
          { force: true }
        );
        throw proxyError;
      }

      const responseHeaders = new Headers();
      if (Array.isArray(proxiedResponse?.headers)) {
        for (const entry of proxiedResponse.headers) {
          if (!Array.isArray(entry) || entry.length < 2) continue;
          const [name, value] = entry;
          if (typeof name !== "string" || typeof value !== "string") continue;
          try {
            responseHeaders.append(name, value);
          } catch {}
        }
      }

      const status = Number(proxiedResponse?.status || 500);
      const statusText =
        typeof proxiedResponse?.statusText === "string"
          ? proxiedResponse.statusText
          : "";
      const isNullBodyStatus =
        status === 101 ||
        status === 103 ||
        status === 204 ||
        status === 205 ||
        status === 304;
      const shouldReturnBody = method !== "HEAD" && !isNullBodyStatus;
      if (!shouldReturnBody) {
        responseHeaders.delete("content-length");
        responseHeaders.delete("transfer-encoding");
      }
      const bodyBytes = shouldReturnBody
        ? decodeBase64ToBytes(
            typeof proxiedResponse?.bodyBase64 === "string"
              ? proxiedResponse.bodyBase64
              : ""
          )
        : new Uint8Array();
      const responseBody =
        shouldReturnBody && bodyBytes.byteLength > 0 ? bodyBytes : undefined;

      report(
        "cloud_fetch_proxy_response=" +
          JSON.stringify({
            method,
            url: targetUrl.toString(),
            status,
            statusText: statusText || null,
            bodyBytes: bodyBytes.byteLength,
            bodyAllowed: shouldReturnBody,
            contentType: responseHeaders.get("content-type"),
          })
      );

      return new Response(responseBody, {
        status,
        statusText,
        headers: responseHeaders,
      });
    };

    window.fetch = proxiedFetch;
    if (typeof globalThis === "object") {
      globalThis.fetch = proxiedFetch;
    }

    state.cloudFetchProxyInstalled = true;
    report("cloud_fetch_diagnostics_installed=true");
  };

  const cloudBackendReference = [
    {
      id: "equicord",
      label: "Equicord Cloud",
      host: "cloud.equicord.org",
      offers:
        "Equicord's hosted Equicloud backend for settings sync. The backend is BSD-3 licensed and can be self-hosted upstream.",
      policy:
        "Equicloud's published policy says it stores your Discord user ID, settings blob, and sync timestamps. It says it does not collect IP addresses, user agents, cookies, or analytics by default, and uses size limits and optional allowlists for abuse control.",
      policyUrl: "https://equicord.org/cloud/policy",
      sourceUrl: "https://github.com/Equicord/Equicloud",
    },
    {
      id: "vencord",
      label: "Vencord Cloud",
      host: "api.vencord.dev",
      offers:
        "Vencord's hosted cloud sync backend for Vencord and compatible runtimes. Upstream exposes both a hosted service and a self-hostable backend.",
      policy:
        "Vencord upstream links a dedicated cloud privacy policy and AGPL backend source. Equirust links those upstream documents directly instead of paraphrasing backend retention details beyond what Vencord publishes there.",
      policyUrl: "https://vencord.dev/cloud/privacy",
      sourceUrl: "https://github.com/Vencord/Backend",
    },
  ];

  const installCloudBackendInfoStyles = () => {
    if (document.getElementById("equirust-cloud-backend-info-style")) return;

    const style = document.createElement("style");
    style.id = "equirust-cloud-backend-info-style";
    style.textContent = `
      .equirust-cloud-backend-info {
        margin: 12px 0 0;
        display: grid;
        gap: 10px;
      }
      .equirust-cloud-backend-info__intro,
      .equirust-cloud-backend-info__compat {
        margin: 0;
        color: var(--text-muted);
        line-height: 1.5;
      }
      .equirust-cloud-backend-info__compat {
        padding: 10px 12px;
        border-radius: 12px;
        background: color-mix(in srgb, var(--background-secondary) 78%, transparent);
        border: 1px solid color-mix(in srgb, var(--border-faint, rgba(255,255,255,0.08)) 100%, transparent);
      }
      .equirust-cloud-backend-info__grid {
        display: grid;
        gap: 10px;
        grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
      }
      .equirust-cloud-backend-card {
        padding: 12px 14px;
        border-radius: 14px;
        background: color-mix(in srgb, var(--background-secondary) 88%, transparent);
        border: 1px solid color-mix(in srgb, var(--border-faint, rgba(255,255,255,0.08)) 100%, transparent);
        display: grid;
        gap: 8px;
      }
      .equirust-cloud-backend-card[data-active="true"] {
        border-color: color-mix(in srgb, var(--brand-500, #5865f2) 55%, transparent);
        box-shadow: 0 0 0 1px color-mix(in srgb, var(--brand-500, #5865f2) 24%, transparent);
      }
      .equirust-cloud-backend-card__header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
      }
      .equirust-cloud-backend-card__title {
        color: var(--header-primary);
        font-size: 14px;
        font-weight: 700;
      }
      .equirust-cloud-backend-card__active {
        display: inline-flex;
        align-items: center;
        padding: 3px 8px;
        border-radius: 999px;
        background: color-mix(in srgb, var(--brand-500, #5865f2) 18%, transparent);
        color: var(--text-brand, #c9d0ff);
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }
      .equirust-cloud-backend-card__host {
        color: var(--interactive-active);
        font-family: var(--font-code);
        font-size: 12px;
      }
      .equirust-cloud-backend-card__copy {
        margin: 0;
        color: var(--text-normal);
        line-height: 1.45;
      }
      .equirust-cloud-backend-card__links {
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
      }
      .equirust-cloud-backend-card__link {
        color: var(--text-link);
        font-weight: 600;
        text-decoration: none;
      }
      .equirust-cloud-backend-info__custom {
        color: var(--text-danger, #f23f43);
        font-weight: 600;
      }
      @media (max-width: 680px) {
        .equirust-cloud-backend-info__grid {
          grid-template-columns: 1fr;
        }
      }
    `;
    (document.head || document.documentElement || document.body)?.appendChild(style);
  };

  const readCurrentCloudBackendUrl = () => {
    const raw =
      window.Vencord?.Settings?.cloud?.url ??
      state.settings?.cloud?.url ??
      state.hostSettings?.cloud?.url;
    return typeof raw === "string" ? raw.trim() : "";
  };

  const getCloudBackendHost = url => {
    if (typeof url !== "string" || !url.trim()) return "";
    try {
      return String(new URL(url, window.location.href).hostname || "").toLowerCase();
    } catch {
      return "";
    }
  };

  const findCloudBackendInfoAnchor = () => {
    const paragraphs = Array.from(document.querySelectorAll("p"));
    return (
      paragraphs.find(node =>
        /choose which cloud backend to use for storing your settings/i.test(
          String(node.textContent || "")
        )
      ) ||
      paragraphs.find(node =>
        /which backend to use when using cloud integrations/i.test(
          String(node.textContent || "")
        )
      ) ||
      Array.from(document.querySelectorAll("h1,h2,h3,h4,h5,[role='heading']")).find(node =>
        /^cloud backend$/i.test(String(node.textContent || "").trim())
      ) ||
      Array.from(document.querySelectorAll("h1,h2,h3,h4,h5,[role='heading']")).find(node =>
        /^backend url$/i.test(String(node.textContent || "").trim())
      )
    );
  };

  const renderCloudBackendInfo = () => {
    if (!isDiscordHost()) return;

    const anchor = findCloudBackendInfoAnchor();
    const existingPanels = Array.from(
      document.querySelectorAll(".equirust-cloud-backend-info")
    );
    if (!anchor) {
      existingPanels.forEach(node => node.remove());
      return;
    }

    installCloudBackendInfoStyles();

    const activeHost = getCloudBackendHost(readCurrentCloudBackendUrl());
    let panel =
      anchor.nextElementSibling &&
      anchor.nextElementSibling.classList?.contains("equirust-cloud-backend-info")
        ? anchor.nextElementSibling
        : null;

    existingPanels.forEach(node => {
      if (node !== panel) {
        node.remove();
      }
    });

    if (!panel) {
      panel = document.createElement("section");
      panel.className = "equirust-cloud-backend-info";
      anchor.insertAdjacentElement("afterend", panel);
    }

    panel.textContent = "";

    const intro = document.createElement("p");
    intro.className = "equirust-cloud-backend-info__intro";
    intro.textContent =
      "These are the official hosted backends, plus how Equirust treats a custom backend URL you set yourself.";
    panel.appendChild(intro);

    const grid = document.createElement("div");
    grid.className = "equirust-cloud-backend-info__grid";
    panel.appendChild(grid);

    for (const backend of cloudBackendReference) {
      const card = document.createElement("article");
      card.className = "equirust-cloud-backend-card";
      card.dataset.active = activeHost === backend.host ? "true" : "false";

      const header = document.createElement("div");
      header.className = "equirust-cloud-backend-card__header";

      const title = document.createElement("div");
      title.className = "equirust-cloud-backend-card__title";
      title.textContent = backend.label;
      header.appendChild(title);

      if (activeHost === backend.host) {
        const badge = document.createElement("span");
        badge.className = "equirust-cloud-backend-card__active";
        badge.textContent = "Active";
        header.appendChild(badge);
      }

      card.appendChild(header);

      const host = document.createElement("div");
      host.className = "equirust-cloud-backend-card__host";
      host.textContent = backend.host;
      card.appendChild(host);

      const offers = document.createElement("p");
      offers.className = "equirust-cloud-backend-card__copy";
      offers.textContent = backend.offers;
      card.appendChild(offers);

      const policy = document.createElement("p");
      policy.className = "equirust-cloud-backend-card__copy";
      policy.textContent = backend.policy;
      card.appendChild(policy);

      const links = document.createElement("div");
      links.className = "equirust-cloud-backend-card__links";

      const policyLink = document.createElement("a");
      policyLink.className = "equirust-cloud-backend-card__link";
      policyLink.href = backend.policyUrl;
      policyLink.target = "_blank";
      policyLink.rel = "noreferrer noopener";
      policyLink.textContent = "Policy";
      links.appendChild(policyLink);

      const sourceLink = document.createElement("a");
      sourceLink.className = "equirust-cloud-backend-card__link";
      sourceLink.href = backend.sourceUrl;
      sourceLink.target = "_blank";
      sourceLink.rel = "noreferrer noopener";
      sourceLink.textContent = "Source";
      links.appendChild(sourceLink);

      card.appendChild(links);
      grid.appendChild(card);
    }

    const compat = document.createElement("p");
    compat.className = "equirust-cloud-backend-info__compat";
    if (
      activeHost &&
      !cloudBackendReference.some(backend => backend.host === activeHost)
    ) {
      compat.appendChild(
        document.createTextNode(
          "Equirust's cloud bridge is exact-host only for CSP/CORS compatibility. "
        )
      );
      const custom = document.createElement("span");
      custom.className = "equirust-cloud-backend-info__custom";
      const customUrl = readCurrentCloudBackendUrl();
      const insecureHttp = /^http:\/\//i.test(customUrl);
      custom.textContent = insecureHttp
        ? `The current backend (${activeHost}) is custom and uses plain HTTP. Equirust will proxy only this exact configured origin, but your cloud auth and sync traffic is not transport-encrypted.`
        : `The current backend (${activeHost}) is custom. Equirust will only proxy this exact configured origin; review that backend's own policy before using it.`;
      compat.appendChild(custom);
    } else {
      compat.textContent =
        "Equirust forwards cloud requests through its Rust host because Discord's renderer can block them with CSP or CORS. Official hosted backends are built in, and a custom backend works only for the exact origin you configure below.";
    }
    panel.appendChild(compat);
  };

  const scheduleCloudBackendInfoRender = () => {
    if (state.cloudBackendInfoRenderTimer !== null) return;
    state.cloudBackendInfoRenderTimer = window.setTimeout(() => {
      state.cloudBackendInfoRenderTimer = null;
      renderCloudBackendInfo();
    }, 120);
  };

  const installCloudBackendInfoWatcher = () => {
    if (state.cloudBackendInfoObserver || !isDiscordHost()) return;

    scheduleCloudBackendInfoRender();

    const observerTarget = document.body || document.documentElement;
    if (!observerTarget) return;

    state.cloudBackendInfoObserver = new MutationObserver(() => {
      scheduleCloudBackendInfoRender();
    });
    state.cloudBackendInfoObserver.observe(observerTarget, {
      subtree: true,
      childList: true,
      characterData: true,
    });
  };

  const isDiscordWindowActive = () => {
    try {
      return document.visibilityState === "visible" && !document.hidden && document.hasFocus();
    } catch {
      return false;
    }
  };

  const appBadgeEnabled = () =>
    getHostSettingValue({
      key: "appBadge",
      defaultValue: true,
    });

  const badgeOnlyForMentionsEnabled = () =>
    getHostSettingValue({
      key: "badgeOnlyForMentions",
      defaultValue: true,
    });

  const taskbarFlashingEnabled = () =>
    getHostSettingValue({
      key: "enableTaskbarFlashing",
      defaultValue: false,
    });

  const parseTitleBadgeCount = () => {
    const title = typeof document.title === "string" ? document.title.trim() : "";
    const mentionMatch = title.match(/^\((\d+)\)\s+/);
    if (mentionMatch) {
      return Number(mentionMatch[1]) || 0;
    }

    if (!badgeOnlyForMentionsEnabled() && /^[•●]\s*/.test(title)) {
      return -1;
    }

    return 0;
  };

  const normalizeAttentionCount = count => {
    const numeric = Number(count || 0);
    if (!Number.isFinite(numeric) || numeric <= 0) {
      return numeric < 0 ? 1 : 0;
    }

    return numeric;
  };

  const titleMutationTouched = records =>
    records.some(record => {
      const target = record.target;
      if (target?.nodeName === "TITLE" || target?.parentNode?.nodeName === "TITLE") {
        return true;
      }

      return Array.from(record.addedNodes || []).some(node => node?.nodeName === "TITLE") ||
        Array.from(record.removedNodes || []).some(node => node?.nodeName === "TITLE");
    });

  const syncHostBadgeState = () => {
    state.notificationSyncQueued = false;
    window.__EQUIRUST_TITLEBAR_SYNC__?.();

    const previousCount = state.lastBadgeCount;
    const nextCount = appBadgeEnabled() ? parseTitleBadgeCount() : 0;
    const previousAttention = normalizeAttentionCount(previousCount);
    const nextAttention = normalizeAttentionCount(nextCount);

    if (previousCount !== nextCount) {
      state.lastBadgeCount = nextCount;
      invoke("set_badge_count", { count: nextCount }).catch(error => {
        console.warn("[Equirust]", error);
      });
    }

    const focused = isDiscordWindowActive();
    const shouldFlash = taskbarFlashingEnabled() && !focused && nextAttention > 0;
    const shouldStartFlash = shouldFlash && nextAttention > previousAttention;

    if (shouldStartFlash && !state.flashActive) {
      state.flashActive = true;
      invoke("flash_frame", { flag: true }).catch(error => {
        console.warn("[Equirust]", error);
      });
      return;
    }

    if ((!shouldFlash || focused || nextAttention === 0) && state.flashActive) {
      state.flashActive = false;
      invoke("flash_frame", { flag: false }).catch(error => {
        console.warn("[Equirust]", error);
      });
    }
  };

  const scheduleHostBadgeSync = () => {
    if (!isDiscordHost()) return;
    if (state.notificationSyncQueued) return;
    state.notificationSyncQueued = true;
    window.requestAnimationFrame(syncHostBadgeState);
  };

  const installNotificationSync = () => {
    if (state.notificationSyncReady || !isDiscordHost()) return;
    if (!document.head && !document.documentElement) return;

    state.notificationSyncReady = true;
    state.notificationObserver = new MutationObserver(records => {
      if (!titleMutationTouched(records)) return;
      scheduleHostBadgeSync();
    });
    state.notificationObserver.observe(document.head || document.documentElement, {
      subtree: true,
      childList: true,
      characterData: true,
    });

    window.addEventListener("focus", scheduleHostBadgeSync);
    window.addEventListener("blur", scheduleHostBadgeSync);
    window.addEventListener("pageshow", scheduleHostBadgeSync);
    document.addEventListener("visibilitychange", scheduleHostBadgeSync);
    scheduleHostBadgeSync();
    report("notification_sync_installed=true");
  };

  const setVoiceTrayCallState = inCall => {
    if (state.voiceTrayInCall === inCall) return;
    state.voiceTrayInCall = inCall;
    if (!inCall) {
      state.voiceTrayVariant = null;
    }

    invoke("set_tray_voice_call_state", { inCall }).catch(error => {
      console.warn("[Equirust]", error);
    });
  };

  const setVoiceTrayVariant = variant => {
    if (!state.voiceTrayInCall || state.voiceTrayVariant === variant) return;
    state.voiceTrayVariant = variant;
    invoke("set_tray_voice_state", { variant }).catch(error => {
      console.warn("[Equirust]", error);
    });
  };

  const installVoiceTrayWatcher = () => {
    if (state.voiceTrayReady || !isDiscordHost()) return;

    const FluxDispatcher = window.Vencord?.Webpack?.Common?.FluxDispatcher;
    const MediaEngineStore = window.Vencord?.Webpack?.Common?.MediaEngineStore;
    const UserStore = window.Vencord?.Webpack?.Common?.UserStore;
    const currentUserId = UserStore?.getCurrentUser?.()?.id;

    if (!FluxDispatcher || !MediaEngineStore || !currentUserId) {
      if (!state.voiceTrayTimer) {
        state.voiceTrayTimer = window.setInterval(() => {
          installVoiceTrayWatcher();
          if (state.voiceTrayReady && state.voiceTrayTimer) {
            window.clearInterval(state.voiceTrayTimer);
            state.voiceTrayTimer = null;
          }
        }, 1200);
      }
      return;
    }

    const updateIdleVoiceTray = () => {
      if (!state.voiceTrayInCall) return;

      if (MediaEngineStore.isSelfDeaf?.()) {
        setVoiceTrayVariant("trayDeafened");
      } else if (MediaEngineStore.isSelfMute?.()) {
        setVoiceTrayVariant("trayMuted");
      } else {
        setVoiceTrayVariant("trayIdle");
      }
    };

    const speakingCallback = params => {
      if (params?.userId !== currentUserId || params?.context !== "default") return;

      if (params.speakingFlags) {
        setVoiceTrayVariant("traySpeaking");
      } else {
        updateIdleVoiceTray();
      }
    };

    const muteCallback = () => {
      if (state.voiceTrayInCall) {
        updateIdleVoiceTray();
      }
    };

    const rtcCallback = params => {
      if (params?.context !== "default") return;

      if (params.state === "RTC_CONNECTED") {
        setVoiceTrayCallState(true);
        updateIdleVoiceTray();
      } else if (params.state === "RTC_DISCONNECTED") {
        setVoiceTrayCallState(false);
        scheduleHostBadgeSync();
      }
    };

    FluxDispatcher.subscribe("SPEAKING", speakingCallback);
    FluxDispatcher.subscribe("AUDIO_TOGGLE_SELF_DEAF", muteCallback);
    FluxDispatcher.subscribe("AUDIO_TOGGLE_SELF_MUTE", muteCallback);
    FluxDispatcher.subscribe("RTC_CONNECTION_STATE", rtcCallback);

    state.voiceTrayReady = true;
    if (state.voiceTrayTimer) {
      window.clearInterval(state.voiceTrayTimer);
      state.voiceTrayTimer = null;
    }

    report("voice_tray_installed=true");
  };

  const callListeners = listeners => {
    listeners.forEach(listener => {
      try {
        listener();
      } catch (error) {
        console.error("[Equirust]", error);
      }
    });
  };

  const resolveVoiceActions = () => window.Vencord?.Webpack?.Common?.VoiceActions;

  const dispatchVoiceToggle = kind => {
    const listeners =
      kind === "mute" ? state.voiceToggleMuteListeners : state.voiceToggleDeafListeners;

    if (listeners.size > 0) {
      callListeners(listeners);
      report(`voice_toggle_dispatched=${kind} mode=listener`);
      return true;
    }

    const VoiceActions = resolveVoiceActions();
    if (!VoiceActions) {
      return false;
    }

    if (kind === "mute") {
      VoiceActions.toggleSelfMute?.();
    } else {
      VoiceActions.toggleSelfDeaf?.();
    }

    report(`voice_toggle_dispatched=${kind} mode=direct`);
    return true;
  };

  const flushQueuedVoiceToggles = () => {
    if (!state.voiceToggleQueue.length) {
      if (state.voiceToggleRetryTimer) {
        window.clearInterval(state.voiceToggleRetryTimer);
        state.voiceToggleRetryTimer = null;
      }
      return;
    }

    if (!resolveVoiceActions() &&
        state.voiceToggleMuteListeners.size === 0 &&
        state.voiceToggleDeafListeners.size === 0) {
      return;
    }

    const pending = [...state.voiceToggleQueue];
    state.voiceToggleQueue = [];

    pending.forEach(kind => {
      if (!dispatchVoiceToggle(kind)) {
        state.voiceToggleQueue.push(kind);
      }
    });

    if (!state.voiceToggleQueue.length && state.voiceToggleRetryTimer) {
      window.clearInterval(state.voiceToggleRetryTimer);
      state.voiceToggleRetryTimer = null;
    }
  };

  const queueVoiceToggle = kind => {
    state.voiceToggleQueue.push(kind);
    if (!state.voiceToggleRetryTimer) {
      state.voiceToggleRetryTimer = window.setInterval(flushQueuedVoiceToggles, 700);
    }
  };

  const handleVoiceToggleEvent = kind => {
    if (!dispatchVoiceToggle(kind)) {
      queueVoiceToggle(kind);
      report(`voice_toggle_queued=${kind}`);
    }
  };

  const ensureVoiceToggleBridge = () => {
    if (state.voiceBridgeReady || !isDiscordHost()) return;

    Promise.all([
      listenTauriEvent("equirust:voice-toggle-mute", () => handleVoiceToggleEvent("mute")),
      listenTauriEvent("equirust:voice-toggle-deafen", () => handleVoiceToggleEvent("deafen")),
    ])
      .then(cleanups => {
        state.voiceBridgeCleanup = async () => {
          await Promise.all(cleanups.map(cleanup => cleanup()));
          state.voiceBridgeCleanup = null;
          state.voiceBridgeReady = false;
        };
        state.voiceBridgeReady = true;
        flushQueuedVoiceToggles();
        report("voice_toggle_bridge_installed=true");
      })
      .catch(error => {
        const message = error && error.message ? error.message : String(error);
        report(`voice_toggle_bridge_failed=${message}`, { force: true });
      });
  };

  const arrpcAppCache = new Map();
  const ARRPC_NULL_CLEAR_GRACE_MS = 500;

  const lookupArrpcAsset = async (applicationId, key) => {
    try {
      const assetUtils = window.Vencord?.Webpack?.Common?.ApplicationAssetUtils;
      if (!assetUtils?.fetchAssetIds) return undefined;
      const assets = await assetUtils.fetchAssetIds(applicationId, [key]);
      return assets?.[0];
    } catch (error) {
      console.warn("[Equirust] Failed to resolve arRPC asset", error);
      return undefined;
    }
  };

  const lookupArrpcApplication = async applicationId => {
    if (!applicationId) return undefined;
    if (arrpcAppCache.has(applicationId)) {
      const cached = arrpcAppCache.get(applicationId);
      arrpcAppCache.delete(applicationId);
      arrpcAppCache.set(applicationId, cached);
      return cached;
    }

    try {
      const fetchApplicationsRPC = window.Vencord?.Webpack?.Common?.fetchApplicationsRPC;
      if (typeof fetchApplicationsRPC !== "function") return undefined;
      const socket = {};
      await fetchApplicationsRPC(socket, applicationId);
      if (socket.application) {
        if (arrpcAppCache.size >= 50) {
          const firstKey = arrpcAppCache.keys().next().value;
          if (firstKey) arrpcAppCache.delete(firstKey);
        }
        arrpcAppCache.set(applicationId, socket.application);
        return socket.application;
      }
    } catch (error) {
      console.warn("[Equirust] Failed to resolve arRPC application", error);
    }

    return undefined;
  };

  const sameArrpcRunningGame = (left, right) => {
    if (!left && !right) return true;
    if (!left || !right) return false;
    return (
      left.socketId === right.socketId &&
      left.applicationId === right.applicationId &&
      left.pid === right.pid &&
      left.name === right.name &&
      left.startTime === right.startTime
    );
  };

  const buildArrpcRunningGame = payload => {
    const activity = payload?.activity;
    if (!activity || typeof activity !== "object") {
      return null;
    }

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

    if (!socketId && !applicationId) {
      return null;
    }

    const pid =
      typeof payload?.pid === "number" && Number.isFinite(payload.pid) ? payload.pid : 0;
    const startTime =
      typeof activity?.timestamps?.start === "number" && Number.isFinite(activity.timestamps.start)
        ? activity.timestamps.start
        : null;

    return {
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
    };
  };

  const syncArrpcRunningGames = (dispatcher, nextRunningGame) => {
    const previousRunningGame = state.arrpcLastRunningGame;
    if (sameArrpcRunningGame(previousRunningGame, nextRunningGame)) {
      return;
    }

    dispatcher.dispatch({
      type: "RUNNING_GAMES_CHANGE",
      removed: previousRunningGame ? [previousRunningGame] : [],
      added: nextRunningGame ? [nextRunningGame] : [],
      games: nextRunningGame ? [nextRunningGame] : [],
    });

    state.arrpcLastRunningGame = nextRunningGame;
  };

  const dispatchArrpcPayload = async payload => {
    const common = window.Vencord?.Webpack?.Common;
    const dispatcher = common?.FluxDispatcher;
    if (!dispatcher?.dispatch) {
      return false;
    }

    let normalizedPayload =
      payload && typeof payload === "object" ? { ...payload } : { activity: null };
    const bypassNullGrace = normalizedPayload?.__equirustBypassClearGrace === true;
    if (normalizedPayload && typeof normalizedPayload === "object") {
      delete normalizedPayload.__equirustBypassClearGrace;
    }
    let activity =
      normalizedPayload && typeof normalizedPayload === "object"
        ? normalizedPayload.activity
        : null;
    const hasActivity = activity && typeof activity === "object";

    if (hasActivity) {
      state.arrpcLastNonNullAtMs = Date.now();
      if (state.arrpcNullClearTimer) {
        window.clearTimeout(state.arrpcNullClearTimer);
        state.arrpcNullClearTimer = null;
      }
    } else {
      if (!bypassNullGrace && state.arrpcLastSocketId) {
        if (!state.arrpcNullClearTimer) {
          state.arrpcNullClearTimer = window.setTimeout(() => {
            state.arrpcNullClearTimer = null;
            dispatchArrpcPayload({
              socketId: state.arrpcLastSocketId,
              activity: null,
              __equirustBypassClearGrace: true,
            }).catch(error => {
              console.error("[Equirust]", error);
            });
          }, ARRPC_NULL_CLEAR_GRACE_MS);
        }
        return true;
      }
    }

    if (
      normalizedPayload?.socketId === "STREAMERMODE" ||
      activity?.application_id === "STREAMERMODE"
    ) {
      const streamerModeStore = common?.StreamerModeStore;
      if (streamerModeStore?.autoToggle) {
        dispatcher.dispatch({
          type: "STREAMER_MODE_UPDATE",
          key: "enabled",
          value: activity != null,
        });
      }
      return true;
    }

    if (activity && typeof activity === "object") {
      if (
        typeof normalizedPayload.socketId === "string" &&
        normalizedPayload.socketId.trim()
      ) {
        state.arrpcLastSocketId = normalizedPayload.socketId.trim();
      }

      if (activity.assets?.large_image) {
        activity.assets.large_image = await lookupArrpcAsset(
          activity.application_id,
          activity.assets.large_image
        );
      }
      if (activity.assets?.small_image) {
        activity.assets.small_image = await lookupArrpcAsset(
          activity.application_id,
          activity.assets.small_image
        );
      }

      const application = await lookupArrpcApplication(activity.application_id);
      if (application?.name && !activity.name) {
        activity.name = application.name;
      }
    } else {
      const rememberedSocketId =
        typeof state.arrpcLastSocketId === "string" && state.arrpcLastSocketId.trim()
          ? state.arrpcLastSocketId.trim()
          : null;
      if (
        rememberedSocketId &&
        !(typeof normalizedPayload.socketId === "string" && normalizedPayload.socketId.trim())
      ) {
        normalizedPayload.socketId = rememberedSocketId;
      }
    }

    const runningGame = buildArrpcRunningGame(normalizedPayload);
    syncArrpcRunningGames(dispatcher, runningGame);

    dispatcher.dispatch({
      type: "LOCAL_ACTIVITY_UPDATE",
      ...normalizedPayload,
    });

    report(`arrpc_dispatch=LOCAL_ACTIVITY_UPDATE has_activity=${!!(activity && typeof activity === "object")} app_id=${activity?.application_id ?? "null"} socket_id=${normalizedPayload?.socketId ?? "null"} running_game=${runningGame ? runningGame.name : "null"}`);

    if (!(activity && typeof activity === "object")) {
      state.arrpcLastSocketId = null;
    }

    return true;
  };

  const synthesizeArrpcPayloadFromStatus = status => {
    const summary = Array.isArray(status?.activities) ? status.activities[0] : null;
    if (!summary?.applicationId && !summary?.socketId) {
      return { activity: null };
    }

    const activity = {
      application_id: summary.applicationId || summary.socketId,
      type: 0,
    };

    if (summary.name) {
      activity.name = summary.name;
    }

    if (summary.startTime) {
      activity.timestamps = { start: summary.startTime };
    }

    const payload = { activity };
    if (summary.socketId) {
      payload.socketId = summary.socketId;
    }
    if (summary.pid) {
      payload.pid = summary.pid;
    }
    if (summary.name) {
      payload.name = summary.name;
    }

    return payload;
  };

  const flushPendingArrpcPayloads = async () => {
    if (!state.arrpcPendingPayloads.length) return;
    const pending = [...state.arrpcPendingPayloads];
    state.arrpcPendingPayloads.length = 0;
    for (const payload of pending) {
      const handled = await dispatchArrpcPayload(payload);
      if (!handled) {
        state.arrpcPendingPayloads.unshift(payload);
        break;
      }
    }
  };

  const handleArrpcPayload = payload => {
    state.arrpcActivityListeners.forEach(listener => {
      try {
        listener(payload);
      } catch (error) {
        console.error("[Equirust]", error);
      }
    });

    Promise.resolve(dispatchArrpcPayload(payload))
      .then(handled => {
        if (!handled) {
          state.arrpcPendingPayloads.push(payload);
          window.setTimeout(() => {
            flushPendingArrpcPayloads().catch(error => {
              console.error("[Equirust]", error);
            });
          }, 1200);
        }
      })
      .catch(error => {
        console.error("[Equirust]", error);
      });
  };

  const ensureArrpcBridge = () => {
    if (state.arrpcBridgeReady || state.arrpcBridgeCleanup || !isDiscordHost()) return;

    Promise.all([
      listenTauriEvent("equirust:arrpc-activity", payload => {
        const nextPayload =
          payload?.payload && typeof payload.payload === "object" ? payload.payload : payload;
        handleArrpcPayload(nextPayload);
      }),
      listenTauriEvent("equirust:arrpc-status", payload => {
        state.arrpcStatus =
          payload?.payload && typeof payload.payload === "object" ? payload.payload : payload;
        notifySettingsSync();
      }),
    ])
      .then(cleanups => {
        state.arrpcBridgeCleanup = async () => {
          await Promise.all(cleanups.map(cleanup => cleanup()));
          state.arrpcBridgeCleanup = null;
          state.arrpcBridgeReady = false;
        };
        state.arrpcBridgeReady = true;
        Promise.allSettled([
          refreshArRPCStatus(),
          invoke("get_arrpc_current_activity"),
        ])
          .then(results => {
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
          })
          .catch(error => {
            console.error("[Equirust]", error);
          });
        report("arrpc_bridge_installed=true");
      })
      .catch(error => {
        const message = error && error.message ? error.message : String(error);
        report(`arrpc_bridge_failed=${message}`, { force: true });
      });
  };

  const createVencordNative = () => {
    if (window.VencordNative) return;

    window.VencordNative = {
      app: {
        relaunch: () => invoke("app_relaunch"),
        getVersion: () => String(state.versions.equirust || "1.0.0"),
        getGitHash: () => String(state.versions.gitHash || "unknown"),
        isDevBuild: () => state.debugBuild === true,
        setBadgeCount: count => invoke("set_badge_count", { count }),
        supportsWindowsTransparency: () => supportsWindowsTransparency(),
        getEnableHardwareAcceleration: () =>
          state.hostSettings?.hardwareAcceleration !== false,
        isOutdated: async () => {
          const status = await getHostUpdateStatusCached(false);
          return status?.updateAvailable === true;
        },
        openUpdater: () => openHostUpdate(),
        getPlatformSpoofInfo: () => ({
          spoofed: false,
          originalPlatform: normalizeLegacyPlatform(state.versions.platform),
          spoofedPlatform: null,
        }),
        getRendererCss: () => refreshRendererCss(),
        onRendererCssUpdate: listener => addRendererCssListener(listener),
      },
      autostart: {
        isEnabled: () => refreshNativeAutoStart(),
        enable: () => persistNativeAutoStart(true),
        disable: () => persistNativeAutoStart(false),
      },
      arrpc: {
        onActivity(listener) {
          if (typeof listener !== "function") return;
          state.arrpcActivityListeners.add(listener);
          ensureArrpcBridge();
        },
        offActivity(listener) {
          state.arrpcActivityListeners.delete(listener);
        },
        getStatus: () => refreshArRPCStatus(),
        restart: () => restartArRPC(),
        openSettings: () => focusEquirustSettingsSection("rich-presence"),
      },
      themes: {
        uploadTheme: async (...args) => {
          const [fileName, content] = args;
          if (typeof fileName === "string" && typeof content === "string") {
            await invoke("set_vencord_theme_data", { fileName, content });
            notifyThemeListeners();
            return "ok";
          }

          const result = await invoke("upload_vencord_theme");
          if (result === "ok") {
            notifyThemeListeners();
          }
          return result;
        },
        deleteTheme: async fileName => {
          const result = await invoke("delete_vencord_theme", { fileName });
          if (result === "ok" || result === "missing") {
            notifyThemeListeners();
          }
          return result;
        },
        getThemesList: () => invoke("get_vencord_theme_entries"),
        getThemeData: fileName => invoke("get_vencord_theme_data", { fileName }),
        getThemesDir: () => invoke("get_vencord_themes_dir"),
        getSystemValues: () => getSystemThemeValues(),
        openFolder: () => invoke("open_vencord_themes_folder"),
      },
      updater: {
        getUpdates: async () =>
          wrapIpcResult(async () => {
            report("runtime_update_bridge_get_updates_blocked=manual_only");
            return [];
          }),
        update: async () =>
          wrapIpcResult(async () => {
            report("runtime_update_bridge_update_blocked=manual_only", { force: true });
            focusEquirustSettingsSection("updates");
            return false;
          }),
        rebuild: async () =>
          wrapIpcResult(async () => {
            report("runtime_update_bridge_rebuild_blocked=manual_only", { force: true });
            focusEquirustSettingsSection("updates");
            return false;
          }),
        build: async () =>
          wrapIpcResult(async () => {
            report("runtime_update_bridge_build_blocked=manual_only", { force: true });
            focusEquirustSettingsSection("updates");
            return false;
          }),
        getRepo: async () => wrapIpcResult(async () => getRuntimeUpdateRepo()),
      },
      settings: {
        get: () => state.settings,
        set: (settings, path) => safeCall(async () => {
          state.settings = settings || {};
          await invoke("set_vencord_settings", { settings, path });
          state.themeListeners.forEach(listener => {
            try { listener(); } catch (error) { console.error("[Equirust]", error); }
          });
        }),
        getSettingsDir: () => invoke("get_vencord_settings_dir"),
        openFolder: () => invoke("open_vencord_settings_folder"),
      },
      spellcheck: {
        getAvailableLanguages: () => getSpellcheckLanguages(),
        onSpellcheckResult(listener) {
          if (typeof listener !== "function") return;
          state.spellcheckResultListeners.add(listener);
          installSpellcheckBridge();
        },
        offSpellcheckResult(listener) {
          state.spellcheckResultListeners.delete(listener);
        },
        replaceMisspelling: word => replaceSpellcheckSelection(word),
        addToDictionary: word => addSpellcheckWordToDictionary(word),
      },
      capturer: {
        getSources: () => invoke("get_capturer_sources"),
        getLargeThumbnail: id => invoke("get_capturer_large_thumbnail", { id }),
      },
      virtmic: {
        list: () => invoke("virtmic_list"),
        start: include => invoke("virtmic_start", { include }),
        startSystem: exclude => invoke("virtmic_start_system", { exclude }),
        stop: () => invoke("virtmic_stop"),
      },
      quickCss: {
        get: async () => state.quickCss,
        set: css => safeCall(async () => {
          state.quickCss = typeof css === "string" ? css : "";
          await invoke("set_vencord_quick_css", { css: state.quickCss });
          notifyQuickCssListeners();
        }),
        addChangeListener(listener) {
          state.quickCssListeners.add(listener);
          ensureVencordFileWatch();
        },
        addThemeChangeListener(listener) {
          state.themeListeners.add(listener);
          ensureVencordFileWatch();
        },
        openFile: () => invoke("open_vencord_quick_css"),
        openEditor: () => invoke("open_vencord_quick_css"),
        getEditorTheme: () =>
          window.matchMedia("(prefers-color-scheme: dark)").matches ? "vs-dark" : "vs-light",
      },
      fileManager: {
        getState: () => invoke("get_file_manager_state"),
        isUsingCustomVencordDir: async () =>
          Boolean((await invoke("get_file_manager_state"))?.usingCustomVencordDir),
        showCustomVencordDir: () => invoke("show_custom_vencord_dir"),
        selectEquicordDir: reset => invoke("select_vencord_dir", { reset: reset === null || reset === true }),
        chooseUserAsset: (asset, reset) =>
          invoke("choose_user_asset", { asset, reset: reset === null || reset === true }),
        openUserAssetsFolder: () => invoke("open_user_assets_folder"),
      },
      clipboard: {
        copyImage: (imageBuffer, _imageSrc) =>
          invoke("copy_image_to_clipboard", {
            bytes: Array.isArray(imageBuffer) ? imageBuffer : Array.from(imageBuffer || []),
          }),
      },
      win: {
        focus: () => invoke("window_focus"),
        close: () => invoke("window_close"),
        minimize: () => invoke("window_minimize"),
        maximize: () => invoke("window_toggle_maximize"),
        flashFrame: flag => invoke("flash_frame", { flag }),
        setDevtoolsCallbacks: () => {},
      },
      tray: {
        setVoiceState: variant => invoke("set_tray_voice_state", { variant }),
        setVoiceCallState: inCall => invoke("set_tray_voice_call_state", { inCall }),
        onCheckUpdates: () => () => {},
        onRepair: () => () => {},
        setUpdateState: () => {},
      },
      voice: {
        onToggleSelfMute(listener) {
          if (typeof listener !== "function") return;
          state.voiceToggleMuteListeners.add(listener);
          ensureVoiceToggleBridge();
        },
        offToggleSelfMute(listener) {
          state.voiceToggleMuteListeners.delete(listener);
        },
        onToggleSelfDeaf(listener) {
          if (typeof listener !== "function") return;
          state.voiceToggleDeafListeners.add(listener);
          ensureVoiceToggleBridge();
        },
        offToggleSelfDeaf(listener) {
          state.voiceToggleDeafListeners.delete(listener);
        },
      },
      debug: {
        launchGpu: () => invoke("open_debug_page", { target: "gpu" }),
        launchWebrtcInternals: () => invoke("open_debug_page", { target: "webrtc-internals" }),
      },
      commands: {
        onCommand(callback) {
          if (typeof callback !== "function") return;
          state.commandListeners.add(callback);
          ensureRendererCommandBridge();
        },
        offCommand(callback) {
          state.commandListeners.delete(callback);
          releaseRendererCommandBridge();
        },
        respond: response =>
          invoke("respond_renderer_command", {
            nonce: String(response?.nonce || ""),
            ok: response?.ok !== false,
            data: Object.prototype.hasOwnProperty.call(response || {}, "data")
              ? response.data
              : null,
          }),
      },
      native: {
        getVersions: () => state.versions,
        openExternal: url => invoke("open_external_link", { url }),
        getRendererCss: () => refreshRendererCss(),
        onRendererCssUpdate: listener => addRendererCssListener(listener),
      },
      csp: {
        isDomainAllowed: (url, directives) =>
          invoke("csp_is_domain_allowed", { url, directives }),
        removeOverride: url =>
          invoke("csp_remove_override", { url }),
        requestAddOverride: (url, directives, reason) =>
          invoke("csp_request_add_override", { url, directives, reason }),
      },
      pluginHelpers: {},
    };
  };

  const installVencordRuntime = () => {
    if (!isDiscordHost() || !state.vencordRenderer || window.__EQUIRUST_VENCORD_LOADED__) return;
    window.__EQUIRUST_VENCORD_LOADED__ = true;

    try {
      // Upstream desktop runtimes run preload/bootstrap before the renderer
      // bundle. Storage bindings are already established in the early bootstrap,
      // so load the renderer bundle without reshaping its execution context.
      window.eval(`${state.vencordRenderer}\n;window.Vencord = typeof Vencord !== "undefined" ? Vencord : window.Vencord;`);
      window.setTimeout(() => {
        const styleCount = window.VencordStyles instanceof Map ? window.VencordStyles.size : 0;
        const hasRoot = Boolean(document.querySelector("vencord-root"));
        const hasNative = typeof window.VencordNative?.settings?.get === "function";
        const hasVesktop = typeof window.VesktopNative !== "undefined";
        const hasSettingsPlugin = Boolean(window.Vencord?.Plugins?.plugins?.Settings);
        const hasSettingsApi = Boolean(window.Vencord?.Api?.Settings);
        const vencordKeys = Object.keys(window.Vencord || {}).join(",");
        const pluginKeys = Object.keys(window.Vencord?.Plugins || {}).join(",");
        report(`vencord_root=${hasRoot} style_count=${styleCount} native_bridge=${hasNative} vesktop_bridge=${hasVesktop} settings_plugin=${hasSettingsPlugin} settings_api=${hasSettingsApi} vencord_keys=${vencordKeys} plugin_keys=${pluginKeys} host=${window.location.hostname} ua=${navigator.userAgent}`);
      }, 1600);
    } catch (error) {
      const message = error && error.message ? error.message : String(error);
      report(
        `vencord_load_failed=${message} host=${window.location.hostname}`,
        { force: true }
      );
      throw error;
    }
  };

  const installTitlebar = () => {
    if (state.titlebarReady || !document.body) return;
    state.titlebarReady = true;

    const style = document.createElement("style");
    style.id = "equirust-titlebar-style";
    style.textContent = `
      :root {
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
      }
      html.equirust-chrome body {
        padding-top: var(--equirust-titlebar-height) !important;
        box-sizing: border-box;
      }
      html.equirust-chrome #app-mount,
      html.equirust-chrome [class*="appMount"] {
        min-height: calc(100vh - var(--equirust-titlebar-height)) !important;
      }
      #equirust-titlebar {
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
      }
      .equirust-titlebar__cluster {
        display: inline-flex;
        align-items: center;
        min-width: 0;
        pointer-events: none;
        position: relative;
        z-index: 2;
      }
      .equirust-titlebar__left {
        gap: 0;
        padding-left: 5px;
      }
      .equirust-titlebar__drag {
        display: flex;
        align-items: center;
        justify-content: center;
        min-width: 0;
        padding: 0 10px;
        overflow: hidden;
        pointer-events: auto;
        position: relative;
        z-index: 1;
      }
      .equirust-titlebar__identity {
        display: inline-flex;
        align-items: center;
        gap: 7px;
        min-width: 0;
        max-width: 100%;
        pointer-events: none;
      }
      .equirust-titlebar__icon {
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
      }
      .equirust-titlebar__icon[data-visible="true"] {
        display: inline-flex;
      }
      .equirust-titlebar__icon[data-variant="dm"] {
        background: transparent;
        color: var(--equirust-titlebar-fg);
      }
      .equirust-titlebar__icon img,
      .equirust-titlebar__icon svg {
        width: 100%;
        height: 100%;
        display: block;
      }
      .equirust-titlebar__icon[data-variant="guild-text"] {
        font-family: "Segoe UI Variable Text", "Segoe UI", sans-serif;
        font-size: 9px;
        font-weight: 700;
        letter-spacing: 0.01em;
        text-transform: uppercase;
        color: var(--equirust-titlebar-fg);
      }
      .equirust-titlebar__label {
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
      }
      .equirust-titlebar__controls {
        justify-content: flex-end;
        gap: 0;
      }
      .equirust-titlebar__button {
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
      }
      .equirust-titlebar__button * {
        pointer-events: none;
      }
      .equirust-titlebar__button:hover {
        background: var(--equirust-titlebar-hover);
      }
      .equirust-titlebar__button:active {
        background: var(--equirust-titlebar-active);
      }
      .equirust-titlebar__button--close:hover {
        background: var(--equirust-titlebar-danger-hover);
        color: white;
      }
      .equirust-titlebar__button--close:active {
        background: var(--equirust-titlebar-danger);
      }
      .equirust-titlebar__button svg {
        width: 11px;
        height: 11px;
        opacity: 0.96;
      }
      .equirust-titlebar__button--nav {
        width: 22px;
        height: 100%;
        margin: 0 1px;
        border-radius: 5px;
        color: var(--equirust-titlebar-fg-muted);
      }
      .equirust-titlebar__button--nav:hover {
        color: var(--equirust-titlebar-fg);
      }
      .equirust-titlebar__button--utility {
        width: 28px;
        color: var(--equirust-titlebar-fg-muted);
      }
      .equirust-titlebar__button--utility svg {
        width: 13px;
        height: 13px;
        opacity: 0.98;
      }
      .equirust-titlebar__button--utility:hover {
        color: var(--equirust-titlebar-fg);
      }
      .equirust-titlebar__button--utility:disabled {
        opacity: 0.42;
        cursor: default;
      }
      .equirust-titlebar__button--utility:disabled:hover {
        background: transparent;
        color: var(--equirust-titlebar-fg-muted);
      }
      .equirust-titlebar__divider {
        width: 1px;
        height: 12px;
        margin: 0 3px 0 1px;
        background: var(--equirust-titlebar-border);
        flex: 0 0 auto;
      }
      .equirust-resize {
        position: fixed;
        z-index: 2147483647;
        background: transparent;
      }
      .equirust-resize--n,
      .equirust-resize--s {
        left: 10px;
        right: 10px;
        height: 4px;
      }
      .equirust-resize--n {
        top: 0;
        cursor: n-resize;
      }
      .equirust-resize--s {
        bottom: 0;
        cursor: s-resize;
      }
      .equirust-resize--e,
      .equirust-resize--w {
        top: 10px;
        bottom: 10px;
        width: 4px;
      }
      .equirust-resize--e {
        right: 0;
        cursor: e-resize;
      }
      .equirust-resize--w {
        left: 0;
        cursor: w-resize;
      }
      .equirust-resize--ne,
      .equirust-resize--nw,
      .equirust-resize--se,
      .equirust-resize--sw {
        width: 10px;
        height: 10px;
      }
      .equirust-resize--ne {
        top: 0;
        right: 0;
        cursor: ne-resize;
      }
      .equirust-resize--nw {
        top: 0;
        left: 0;
        cursor: nw-resize;
      }
      .equirust-resize--se {
        right: 0;
        bottom: 0;
        cursor: se-resize;
      }
      .equirust-resize--sw {
        left: 0;
        bottom: 0;
        cursor: sw-resize;
      }
      .equirust-typing-host {
        display: inline-flex !important;
        align-items: center;
      }
      .equirust-typing-host [class*="dot"],
      .equirust-typing-host [class*="dots"] {
        animation: none !important;
        opacity: 0 !important;
        width: 0 !important;
        margin: 0 !important;
        overflow: hidden !important;
      }
      .equirust-typing-bloom {
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
      }
      .equirust-typing-bloom::before {
        content: "";
        position: absolute;
        inset: 1px;
        border-radius: inherit;
        background:
          linear-gradient(90deg, rgba(88,101,242,0.04), rgba(88,101,242,0.92) 36%, rgba(88,214,255,0.95) 68%, rgba(114,240,164,0.9));
        transform-origin: left center;
        animation: equirust-typing-bloom 1.45s cubic-bezier(0.4, 0, 0.2, 1) infinite;
      }
      @keyframes equirust-typing-bloom {
        0% {
          transform: translateX(-76%) scaleX(0.38);
          opacity: 0.16;
        }
        42% {
          transform: translateX(-4%) scaleX(0.96);
          opacity: 1;
        }
        100% {
          transform: translateX(88%) scaleX(0.42);
          opacity: 0.14;
        }
      }
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

    const sanitizeWindowTitle = value => {
      const currentTitle = value && String(value).trim().length
        ? String(value).trim()
        : (isDiscordHost() ? "Discord" : "Equirust");
      return currentTitle.replace(/\s+\|\s+Discord$/i, "").replace(/^Discord\s+\|\s+/i, "");
    };

    const getRouteParts = () =>
      window.location.pathname
        .split("/")
        .map(part => part.trim())
        .filter(Boolean);

    const getCurrentGuildId = () => {
      const parts = getRouteParts();
      if (parts[0] !== "channels") return null;
      if (!parts[1] || parts[1] === "@me") return null;
      return parts[1];
    };

    const isDirectMessagesRoute = () => {
      const parts = getRouteParts();
      return parts[0] === "channels" && parts[1] === "@me";
    };

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
        .find(target => {
          if (!(target instanceof HTMLElement)) return false;
          if (target.closest("#equirust-titlebar")) return false;

          const rect = target.getBoundingClientRect();
          return rect.width > 0 && rect.height > 0;
        });

    const resolveGuildIdentity = () => {
      const guildId = getCurrentGuildId();
      if (!guildId) return null;

      const guildStore = window.Vencord?.Webpack?.Common?.GuildStore;
      const guild = guildStore?.getGuild?.(guildId) || null;

      const guildNavTarget = findVisibleElement([
        `[data-list-item-id="guildsnav___${guildId}"]`,
        `[data-list-item-id^="guildsnav___${guildId}"]`,
        `nav [href="/channels/${guildId}"]`,
        `nav [href="/channels/${guildId}/"]`,
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

      return {
        label: labelText,
        iconUrl,
        iconText: getInitials(labelText),
      };
    };

    const setTitlebarIcon = context => {
      if (!labelIcon) return;

      if (!context) {
        labelIcon.dataset.visible = "false";
        labelIcon.dataset.variant = "";
        labelIcon.innerHTML = "";
        return;
      }

      labelIcon.dataset.visible = "true";
      labelIcon.dataset.variant = context.variant || "";

      if (context.iconUrl) {
        labelIcon.innerHTML = `<img src="${context.iconUrl}" alt="" referrerpolicy="no-referrer" />`;
        return;
      }

      if (context.variant === "dm") {
        labelIcon.innerHTML = `
          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <path d="M19.73 5.03A16.8 16.8 0 0 0 15.65 4l-.2.4a15.4 15.4 0 0 1 3.74 1.12 12.9 12.9 0 0 0-4.13-1.24 14.7 14.7 0 0 0-6.12 0A12.85 12.85 0 0 0 4.8 5.52 15.4 15.4 0 0 1 8.55 4.4L8.35 4a16.8 16.8 0 0 0-4.08 1.03C1.69 8.86.99 12.59 1.34 16.27a16.95 16.95 0 0 0 5.01 2.53l1.08-1.76c-.59-.2-1.16-.45-1.7-.73.14.1.29.2.44.28a11.55 11.55 0 0 0 10.66 0c.15-.09.3-.18.44-.28-.54.28-1.11.53-1.7.73l1.08 1.76a16.91 16.91 0 0 0 5.01-2.53c.41-4.26-.7-7.96-2.93-11.24ZM8.68 13.95c-.98 0-1.78-.9-1.78-2s.79-2 1.78-2 1.79.9 1.78 2c0 1.1-.8 2-1.78 2Zm6.64 0c-.98 0-1.78-.9-1.78-2s.79-2 1.78-2 1.79.9 1.78 2c0 1.1-.79 2-1.78 2Z"></path>
          </svg>
        `;
        return;
      }

      labelIcon.textContent = context.iconText || "";
    };

    const syncNativeWindowTitle = nextTitle => {
      if (!nextTitle || state.nativeWindowTitle === nextTitle) return;
      state.nativeWindowTitle = nextTitle;
      invoke("window_set_title", { title: nextTitle }).catch(error => console.warn("[Equirust]", error));
    };

    const syncLabel = () => {
      if (getHostSettingValue({
        key: "staticTitle",
        defaultValue: false,
      })) {
        const resolvedTitle = "Equirust";
        label.textContent = resolvedTitle;
        setTitlebarIcon(null);
        syncNativeWindowTitle(resolvedTitle);
        return;
      }

      if (isDirectMessagesRoute()) {
        const resolvedTitle = "Direct Messages";
        label.textContent = resolvedTitle;
        setTitlebarIcon({ variant: "dm" });
        syncNativeWindowTitle(resolvedTitle);
        return;
      }

      const guild = resolveGuildIdentity();
      if (guild) {
        label.textContent = guild.label;
        setTitlebarIcon({
          variant: guild.iconUrl ? "guild-image" : "guild-text",
          iconUrl: guild.iconUrl,
          iconText: guild.iconText,
        });
        syncNativeWindowTitle(guild.label);
        return;
      }

      const resolvedTitle = sanitizeWindowTitle(document.title);
      label.textContent = resolvedTitle;
      setTitlebarIcon(null);
      syncNativeWindowTitle(resolvedTitle);
    };

    const syncMaximizeState = () => {
      invoke("window_is_maximized")
        .then(maximized => {
          maximizeButton.innerHTML = maximized
            ? '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M8 8h10v10H8z"></path><path d="M6 16V6h10"></path></svg>'
            : '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="6" y="6" width="12" height="12"></rect></svg>';
        })
        .catch(error => console.warn("[Equirust]", error));
    };

    const findDiscordHeaderButton = selectors =>
      selectors
        .flatMap(selector => Array.from(document.querySelectorAll(selector)))
        .find(target => {
          if (!(target instanceof HTMLElement)) return false;
          if (target.closest("#equirust-titlebar")) return false;

          const rect = target.getBoundingClientRect();
          return rect.width > 0 && rect.height > 0;
        });

    const clickDiscordHeaderButton = selectors => {
      const target = findDiscordHeaderButton(selectors);
      if (!target) return false;
      target.focus?.();
      target.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1, view: window }));
      target.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, view: window }));
      target.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, cancelable: true, pointerId: 1, view: window }));
      target.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, view: window }));
      target.click();
      return true;
    };

    const discordUtilitySelectors = {
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
    };

    const openDiscordUtility = action => {
      switch (action) {
        case "inbox":
          return clickDiscordHeaderButton(discordUtilitySelectors.inbox);
        case "help":
          return clickDiscordHeaderButton(discordUtilitySelectors.help);
        default:
          return false;
      }
    };

    const syncUtilityButtonState = () => {
      if (inboxButton) {
        inboxButton.disabled = !findDiscordHeaderButton(discordUtilitySelectors.inbox);
      }
      if (helpButton) {
        helpButton.disabled = !findDiscordHeaderButton(discordUtilitySelectors.help);
      }
    };

    const navigateHistory = direction => {
      if (direction === "back") {
        history.back();
      } else if (direction === "forward") {
        history.forward();
      }
    };

    const mouseSideButtonsNavigationEnabled = () =>
      getHostSettingValue({
        key: "mouseSideButtonsNavigation",
        defaultValue: true,
      });

    const getMouseSideButtonDirection = button => {
      if (button === 3) return "back";
      if (button === 4) return "forward";
      return null;
    };

    const stopInteractiveTitlebarEvent = event => {
      event.stopPropagation();
      if (typeof event.stopImmediatePropagation === "function") {
        event.stopImmediatePropagation();
      }
    };

    const cancelInteractiveTitlebarEvent = event => {
      event.preventDefault();
      stopInteractiveTitlebarEvent(event);
    };

    const swallowMouseSideButton = event => {
      if (!getMouseSideButtonDirection(event.button)) return;
      event.preventDefault();
      event.stopPropagation();
    };

    const handleMouseSideButtonNavigation = event => {
      const direction = getMouseSideButtonDirection(event.button);
      if (!direction) return;

      event.preventDefault();
      event.stopPropagation();

      if (!isDiscordWindowActive()) return;
      if (!mouseSideButtonsNavigationEnabled()) return;
      navigateHistory(direction);
    };

    const runTitlebarAction = action => {
      switch (action) {
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
      }
    };

    const isInteractiveTitlebarTarget = target => {
      if (!(target instanceof Element)) return false;
      return Boolean(
        target.closest(
          '#equirust-titlebar [data-action], #equirust-titlebar .equirust-titlebar__left, #equirust-titlebar .equirust-titlebar__controls'
        )
      );
    };

    titlebar.querySelectorAll("[data-action]").forEach(button => {
      button.setAttribute("draggable", "false");
      [
        "pointerdown",
        "mousedown",
        "pointerup",
        "mouseup",
        "auxclick",
        "mousemove",
        "pointermove",
      ].forEach(eventName => {
        button.addEventListener(eventName, cancelInteractiveTitlebarEvent, true);
      });
      ["dragstart", "dblclick", "selectstart"].forEach(eventName => {
        button.addEventListener(eventName, cancelInteractiveTitlebarEvent, true);
      });
      button.addEventListener(
        "click",
        event => {
          event.preventDefault();
          stopInteractiveTitlebarEvent(event);
          runTitlebarAction(button.getAttribute("data-action"));
        },
        true
      );
    });

    window.addEventListener(
      "dblclick",
      event => {
        if (!isInteractiveTitlebarTarget(event.target)) return;
        cancelInteractiveTitlebarEvent(event);
      },
      true
    );

    const clearPendingDrag = () => {
      pendingDragPointer = null;
    };

    dragRegion.addEventListener(
      "mousedown",
      event => {
        if (event.button !== 0) return;
        if (event.target !== dragRegion) return;
        pendingDragPointer = {
          x: event.clientX,
          y: event.clientY,
        };
      },
      true
    );

    window.addEventListener(
      "mousemove",
      event => {
        if (!pendingDragPointer) return;
        if ((event.buttons & 1) !== 1) {
          clearPendingDrag();
          return;
        }

        const movedX = Math.abs(event.clientX - pendingDragPointer.x);
        const movedY = Math.abs(event.clientY - pendingDragPointer.y);
        if (Math.max(movedX, movedY) < 4) return;

        clearPendingDrag();
        event.preventDefault();
        event.stopPropagation();
        invoke("window_start_dragging").catch(error => console.warn("[Equirust]", error));
      },
      true
    );

    window.addEventListener("mouseup", clearPendingDrag, true);
    window.addEventListener("blur", clearPendingDrag, true);

    dragRegion.addEventListener("dblclick", event => {
      if (event.target !== dragRegion) {
        cancelInteractiveTitlebarEvent(event);
        return;
      }
      if (isInteractiveTitlebarTarget(event.target)) {
        cancelInteractiveTitlebarEvent(event);
        return;
      }
      clearPendingDrag();
      invoke("window_toggle_maximize").then(syncMaximizeState);
    });

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

    resizeHandles.forEach(([suffix, direction]) => {
      const handle = document.createElement("div");
      handle.className = `equirust-resize equirust-resize--${suffix}`;
      handle.addEventListener("mousedown", event => {
        if (event.button !== 0) return;
        event.preventDefault();
        invoke("window_start_resize_dragging", { direction }).catch(error => console.warn("[Equirust]", error));
      });
      document.body.appendChild(handle);
    });

    window.__EQUIRUST_TITLEBAR_SYNC__ = () => {
      syncLabel();
      syncMaximizeState();
      syncUtilityButtonState();
    };

    syncLabel();
    syncMaximizeState();
    syncUtilityButtonState();
    window.addEventListener("resize", syncMaximizeState);
    window.addEventListener("popstate", syncLabel);
    ["pushState", "replaceState"].forEach(methodName => {
      const original = history[methodName];
      if (typeof original !== "function") return;

      history[methodName] = function(...args) {
        const result = original.apply(this, args);
        window.setTimeout(syncLabel, 0);
        return result;
      };
    });
    document.addEventListener("visibilitychange", () => {
      if (document.hidden) return;
      syncLabel();
      syncUtilityButtonState();
    });
    window.addEventListener("mousedown", swallowMouseSideButton, true);
    window.addEventListener("mouseup", handleMouseSideButtonNavigation, true);
  };

  const syncTypingIndicators = () => {
    state.typingPollScheduled = false;
    if (!isDiscordHost() || document.hidden) return;

    document.querySelectorAll("[aria-live='polite']").forEach(node => {
      const text = (node.textContent || "").trim().toLowerCase();
      const isTyping = text.includes("typing");
      const host = node.parentElement || node;
      const existing = Array.from(node.children || []).find(child =>
        child.classList && child.classList.contains("equirust-typing-bloom")
      );

      if (isTyping) {
        host.classList.add("equirust-typing-host");
        if (!existing) {
          const bloom = document.createElement("span");
          bloom.className = "equirust-typing-bloom";
          bloom.setAttribute("aria-hidden", "true");
          node.appendChild(bloom);
        }
      } else {
        host.classList.remove("equirust-typing-host");
        if (existing) existing.remove();
      }
    });
  };

  const scheduleTypingSync = () => {
    if (state.typingPollScheduled) return;
    state.typingPollScheduled = true;
    window.requestAnimationFrame(syncTypingIndicators);
  };

  const typingNodeTouched = node => {
    if (!node) return false;

    if (node.nodeType === Node.TEXT_NODE) {
      return Boolean(node.parentElement?.closest?.("[aria-live='polite']"));
    }

    if (node.nodeType !== Node.ELEMENT_NODE) {
      return false;
    }

    return (
      node.matches?.("[aria-live='polite']") ||
      Boolean(node.closest?.("[aria-live='polite']")) ||
      Boolean(node.querySelector?.("[aria-live='polite']"))
    );
  };

  const typingMutationsTouched = records =>
    records.some(record =>
      typingNodeTouched(record.target) ||
      Array.from(record.addedNodes || []).some(typingNodeTouched) ||
      Array.from(record.removedNodes || []).some(typingNodeTouched)
    );

  const installTypingIndicator = () => {
    if (state.typingObserver || !document.body || !isDiscordHost()) return;

    scheduleTypingSync();
    state.typingObserver = new MutationObserver(records => {
      if (document.hidden) return;
      if (!typingMutationsTouched(records)) return;
      scheduleTypingSync();
    });
    state.typingObserver.observe(document.body, {
      subtree: true,
      childList: true,
      characterData: true,
    });

    document.addEventListener("visibilitychange", () => {
      if (document.hidden) return;
      scheduleTypingSync();
    });
  };

