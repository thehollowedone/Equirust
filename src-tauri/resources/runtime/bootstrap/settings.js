  const desktopSettingDefinitions = [
    {
      key: "customTitleBar",
      title: "Discord Titlebar",
      description: "Keep the Discord-style custom titlebar provided by the Rust host.",
      defaultValue: String(state.versions.platform || "").toLowerCase() === "windows",
      restartRequired: true,
    },
    {
      key: "autoStartMinimized",
      title: "Auto Start Minimized",
      description: "Start minimized when Equirust is launched automatically with Windows.",
      defaultValue: false,
    },
    {
      key: "tray",
      title: "Tray Icon",
      description: "Show a tray icon for quick access and background behavior.",
      defaultValue: true,
    },
    {
      key: "minimizeToTray",
      title: "Minimize To Tray",
      description: "Clicking close hides the app to the tray instead of exiting.",
      defaultValue: true,
    },
    {
      key: "clickTrayToShowHide",
      title: "Toggle On Tray Click",
      description: "Left clicking the tray icon toggles the main window.",
      defaultValue: false,
    },
    {
      key: "disableMinSize",
      title: "Disable Minimum Size",
      description: "Allow shrinking the window below the default Discord minimum.",
      defaultValue: false,
    },
    {
      key: "staticTitle",
      title: "Static Title",
      description: "Keep the window title fixed instead of following the active page.",
      defaultValue: false,
    },
    {
      key: "enableMenu",
      title: "Enable Menu Bar",
      description: "Expose a native menu bar when the custom titlebar is disabled.",
      defaultValue: false,
      restartRequired: true,
    },
    {
      key: "openLinksWithElectron",
      title: "Open Links In App",
      description: "Open external web links in separate Equirust windows instead of your default browser.",
      defaultValue: false,
    },
    {
      key: "middleClickAutoscroll",
      title: "Middle Click Autoscroll",
      description: "Enable browser autoscroll for the hosted Discord runtime.",
      defaultValue: false,
      restartRequired: true,
    },
    {
      key: "mouseSideButtonsNavigation",
      title: "Mouse Back And Forward Buttons",
      description: "Use mouse side buttons to navigate Discord history without leaving it up to default webview behavior.",
      defaultValue: true,
    },
    {
      key: "hardwareAcceleration",
      title: "Browser Hardware Acceleration",
      description: "Use GPU compositing for the hosted Discord browser runtime. Restart Equirust after changing this.",
      defaultValue: false,
      restartRequired: true,
    },
    {
      key: "appBadge",
      title: "Unread Badge",
      description: "Show unread and mention counts on the Windows taskbar icon and tray state.",
      defaultValue: true,
    },
    {
      key: "badgeOnlyForMentions",
      title: "Badge Only For Mentions",
      description: "Limit badge counts to mentions and requests instead of all unread channels.",
      defaultValue: true,
    },
    {
      key: "enableTaskbarFlashing",
      title: "Taskbar Flashing",
      description: "Flash the taskbar button for new attention-worthy activity while Equirust is unfocused.",
      defaultValue: false,
    },
  ];

  const getHostSettingValue = definition => {
    const value = state.hostSettings?.[definition.key];
    return typeof value === "boolean" ? value : Boolean(definition.defaultValue);
  };

  const getHostSettingChoice = (key, defaultValue) => {
    const value = state.hostSettings?.[key];
    return typeof value === "string" && value.length ? value : String(defaultValue);
  };

  const getHostSettingText = (key, defaultValue = "") => {
    const value = state.hostSettings?.[key];
    return typeof value === "string" ? value : String(defaultValue);
  };

  const getHostSettingList = key => {
    const value = state.hostSettings?.[key];
    if (!Array.isArray(value)) {
      return [];
    }

    return value
      .filter(entry => typeof entry === "string")
      .map(entry => entry.trim())
      .filter(Boolean);
  };

  const getHostSettingNumberText = key => {
    const value = state.hostSettings?.[key];
    return typeof value === "number" && Number.isFinite(value) ? String(value) : "";
  };

  const syncSpellcheckDictionaryFromHostSettings = () => {
    state.spellcheckLearnedWords = new Set(
      getHostSettingList("spellCheckDictionary").map(word => word.toLocaleLowerCase())
    );
  };

  const focusEquirustSettingsSection = async sectionKey => {
    const clickEquirustEntry = () => {
      const candidates = Array.from(
        document.querySelectorAll(
          '[role="tab"], nav button, nav [role="button"], [class*="sidebar"] [role="button"], [class*="side"] [role="button"]'
        )
      );
      const entry = candidates.find(node => {
        if (!(node instanceof HTMLElement)) return false;
        if (node.closest("#equirust-titlebar")) return false;
        return (node.textContent || "").trim() === "Equirust";
      });

      if (!entry) return false;
      entry.click();
      return true;
    };

    clickEquirustEntry();

    for (let attempt = 0; attempt < 20; attempt += 1) {
      const target = document.querySelector(`[data-equirust-section="${sectionKey}"]`);
      if (target instanceof HTMLElement) {
        target.scrollIntoView({ behavior: "smooth", block: "start" });
        return true;
      }

      await new Promise(resolve => window.setTimeout(resolve, 120));
    }

    return false;
  };

  const hostSettingDisabled = definition => {
    switch (definition.key) {
      case "minimizeToTray":
      case "clickTrayToShowHide":
        return !getHostSettingValue({ key: "tray", defaultValue: true });
      case "autoStartMinimized":
        return !state.nativeAutoStartEnabled;
      case "hardwareVideoAcceleration":
        return !getHostSettingValue({ key: "hardwareAcceleration", defaultValue: true });
      case "badgeOnlyForMentions":
        return !getHostSettingValue({ key: "appBadge", defaultValue: true });
      case "enableMenu":
        return getHostSettingValue({
          key: "customTitleBar",
          defaultValue: String(state.versions.platform || "").toLowerCase() === "windows",
        });
      default:
        return false;
    }
  };

  const persistHostSetting = async (key, value) => {
    const next = { ...state.hostSettings, [key]: value };
    state.hostSettings = next;
    const snapshot = await invoke("set_settings", { settings: next });
    state.hostSettings = snapshot?.settings || next;
    if (key === "spellCheckDictionary") {
      syncSpellcheckDictionaryFromHostSettings();
    }
    if (String(key).startsWith("arRpc")) {
      refreshArRPCStatus().catch(error => {
        console.error("[Equirust]", error);
      });
    }
    scheduleHostBadgeSync();
    notifySettingsSync();
    return state.hostSettings;
  };

  const refreshNativeAutoStart = async () => {
    if (!supportsNativeAutoStart()) {
      state.nativeAutoStartEnabled = false;
      notifySettingsSync();
      return false;
    }

    const enabled = await invoke("get_auto_start_status");
    state.nativeAutoStartEnabled = enabled === true;
    notifySettingsSync();
    return state.nativeAutoStartEnabled;
  };

  const persistNativeAutoStart = async enabled => {
    state.nativeAutoStartEnabled = (await invoke("set_auto_start_enabled", { enabled })) === true;
    notifySettingsSync();
    return state.nativeAutoStartEnabled;
  };

  const refreshHostUpdateStatus = async () => {
    state.hostUpdateStatus = await invoke("get_host_update_status");
    notifySettingsSync();
    return state.hostUpdateStatus;
  };

  const refreshRuntimeUpdateStatus = async () => {
    state.runtimeUpdateStatus = await invoke("get_runtime_update_status");
    notifySettingsSync();
    return state.runtimeUpdateStatus;
  };

  const refreshHostUpdateDownloadState = async () => {
    state.hostUpdateDownloadState = await invoke("get_host_update_download_state");
    notifySettingsSync();
    return state.hostUpdateDownloadState;
  };

  const refreshArRPCStatus = async () => {
    state.arrpcStatus = await invoke("get_arrpc_status");
    notifySettingsSync();
    return state.arrpcStatus;
  };

  const restartArRPC = async () => {
    state.arrpcStatus = await invoke("restart_arrpc");
    notifySettingsSync();
    return state.arrpcStatus;
  };

  const refreshFileManagerState = async () => {
    state.fileManagerState = await invoke("get_file_manager_state");
    notifySettingsSync();
    return state.fileManagerState;
  };

  const openHostUpdate = async () => {
    await invoke("open_host_update");
    return true;
  };

  const openRuntimeUpdate = async () => {
    await invoke("open_runtime_update");
    return true;
  };

  const installRuntimeUpdate = async () => {
    await invoke("install_runtime_update");
    return true;
  };

  const installHostUpdate = async () => {
    await invoke("install_host_update");
    return refreshHostUpdateDownloadState();
  };

  const getRuntimeUpdateStatusCached = async forceRefresh => {
    if (forceRefresh === true || !state.runtimeUpdateStatus) {
      await refreshRuntimeUpdateStatus();
    }

    return state.runtimeUpdateStatus;
  };

  const getRuntimeUpdateRepo = () =>
    state.versions.vencordRepo || "https://github.com/Equicord/Equicord";

  const formatRuntimeVersionLabel = raw => {
    const text = String(raw || "").trim();
    if (!text) {
      return "Unknown";
    }

    const cleaned = text.replace(/^(equicord|vencord)\s+/i, "").trim();
    const semverMatch = cleaned.match(/v?\d+\.\d+\.\d+(?:[-+._][A-Za-z0-9.-]+)*/);
    if (semverMatch?.[0]) {
      return semverMatch[0].replace(/^v/i, "");
    }

    if (/^[0-9a-f]{10,}$/i.test(cleaned)) {
      return cleaned.slice(0, 12);
    }

    return cleaned.length > 24 ? `${cleaned.slice(0, 21)}...` : cleaned;
  };

  const summarizeRuntimeUpdateEntries = status => {
    if (!status?.updateAvailable) {
      return [];
    }

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
          ? `Runtime ${formatRuntimeVersionLabel(status.latestVersion)} is available`
          : "A linked runtime update is available";
    const author = (() => {
      try {
        const repo = new URL(getRuntimeUpdateRepo());
        const parts = repo.pathname.split("/").filter(Boolean);
        return parts.at(-1) || "Equicord";
      } catch {
        return "Equicord";
      }
    })();
    const hashBase =
      String(status.latestVersion || status.releaseName || "latest")
        .trim()
        .replace(/[^a-z0-9]+/gi, "-")
        .replace(/^-+|-+$/g, "")
        .toLowerCase() || "latest";

    return (messages.length ? messages : [fallbackMessage]).slice(0, 24).map((message, index) => ({
      hash: `${hashBase}-${String(index + 1).padStart(2, "0")}`,
      author,
      message,
    }));
  };

  const snoozeHostUpdate = async () => {
    state.hostUpdateStatus = await invoke("snooze_host_update");
    notifySettingsSync();
    return state.hostUpdateStatus;
  };

  const snoozeRuntimeUpdate = async () => {
    state.runtimeUpdateStatus = await invoke("snooze_runtime_update");
    notifySettingsSync();
    return state.runtimeUpdateStatus;
  };

  const ignoreHostUpdate = async version => {
    state.hostUpdateStatus = await invoke("ignore_host_update", { version });
    notifySettingsSync();
    return state.hostUpdateStatus;
  };

  const ignoreRuntimeUpdate = async version => {
    state.runtimeUpdateStatus = await invoke("ignore_runtime_update", { version });
    notifySettingsSync();
    return state.runtimeUpdateStatus;
  };

  const openUserAssetsFolder = async () => {
    await invoke("open_user_assets_folder");
    return true;
  };

  const chooseUserAsset = async (asset, reset) => {
    return invoke("choose_user_asset", { asset, reset: reset === true });
  };

  const showCustomVencordDir = async () => {
    await invoke("show_custom_vencord_dir");
    return true;
  };

  const selectCustomVencordDir = async reset => {
    const result = await invoke("select_vencord_dir", { reset: reset === true });
    await refreshFileManagerState();
    return result;
  };

  const openDebugPage = async target => {
    await invoke("open_debug_page", { target });
    return true;
  };

  const getHostUpdateStatusCached = async forceRefresh => {
    if (forceRefresh === true || !state.hostUpdateStatus) {
      await refreshHostUpdateStatus();
    }

    return state.hostUpdateStatus;
  };

  const getSystemThemeValues = async () => {
    try {
      const values = await invoke("get_system_theme_values");
      if (values && typeof values === "object") {
        return values;
      }
    } catch (error) {
      console.warn("[Equirust] Failed to read system theme values", error);
    }

    return { "os-accent-color": "#5865f2" };
  };

  const ensureRendererCommandBridge = () => {
    if (state.commandBridgeReady || state.commandBridgeCleanup) return;

    listenTauriEvent("equirust:ipc-command", payload => {
      const command =
        payload?.payload && typeof payload.payload === "object" ? payload.payload : payload;
      const nonce = String(command?.nonce || "");
      const message = String(command?.message || "");

      if (!nonce || !message) {
        invoke("respond_renderer_command", {
          nonce,
          ok: false,
          data: "Malformed renderer command payload.",
        }).catch(error => console.error("[Equirust]", error));
        return;
      }

      if (!state.commandListeners.size) {
        invoke("respond_renderer_command", {
          nonce,
          ok: false,
          data: "No renderer command handler is registered.",
        }).catch(error => console.error("[Equirust]", error));
        return;
      }

      state.commandListeners.forEach(listener => {
        try {
          listener({
            nonce,
            message,
            data: command?.data,
          });
        } catch (error) {
          console.error("[Equirust]", error);
        }
      });
    })
      .then(cleanup => {
        state.commandBridgeCleanup = cleanup;
        state.commandBridgeReady = true;
      })
      .catch(error => {
        console.error("[Equirust]", error);
      });
  };

  const releaseRendererCommandBridge = () => {
    if (state.commandListeners.size || !state.commandBridgeCleanup) return;

    const cleanup = state.commandBridgeCleanup;
    state.commandBridgeCleanup = null;
    state.commandBridgeReady = false;
    cleanup().catch(error => console.error("[Equirust]", error));
  };

  const notifyQuickCssListeners = () => {
    state.quickCssListeners.forEach(listener => {
      try {
        listener(state.quickCss);
      } catch (error) {
        console.error("[Equirust]", error);
      }
    });
  };

  const notifyThemeListeners = () => {
    state.themeListeners.forEach(listener => {
      try {
        listener();
      } catch (error) {
        console.error("[Equirust]", error);
      }
    });
  };

  const notifyRendererCssListeners = css => {
    state.rendererCssListeners.forEach(listener => {
      try {
        listener(css);
      } catch (error) {
        console.error("[Equirust]", error);
      }
    });
  };

  const refreshRendererCss = async () => {
    const css = await invoke("get_vencord_renderer_css");
    if (typeof css === "string" && css !== state.rendererCssValue) {
      state.rendererCssValue = css;
      notifyRendererCssListeners(css);
    }

    return typeof state.rendererCssValue === "string" ? state.rendererCssValue : "";
  };

  const ensureRendererCssWatch = () => {
    if (state.rendererCssPollTimer || !state.rendererCssListeners.size) return;

    if (!state.rendererCssVisibilityBound) {
      state.rendererCssVisibilityBound = true;
      document.addEventListener("visibilitychange", () => {
        if (document.hidden || !state.rendererCssListeners.size) return;
        if (state.rendererCssPollTimer) {
          window.clearTimeout(state.rendererCssPollTimer);
          state.rendererCssPollTimer = null;
        }
        ensureRendererCssWatch();
      });
      window.addEventListener("focus", () => {
        if (!state.rendererCssListeners.size) return;
        if (state.rendererCssPollTimer) {
          window.clearTimeout(state.rendererCssPollTimer);
          state.rendererCssPollTimer = null;
        }
        ensureRendererCssWatch();
      });
    }

    const delay =
      state.rendererCssValue == null
        ? 0
        : getAdaptivePollDelay(state.debugBuild ? 1000 : 3000, 12000);
    state.rendererCssPollTimer = window.setTimeout(() => {
      state.rendererCssPollTimer = null;
      if (!state.rendererCssListeners.size) {
        return;
      }

      const refresh = shouldUseActivePolling()
        ? refreshRendererCss()
        : Promise.resolve(typeof state.rendererCssValue === "string" ? state.rendererCssValue : "");

      refresh
        .catch(error => console.error("[Equirust]", error))
        .finally(() => {
          ensureRendererCssWatch();
        });
    }, delay);
  };

  const addRendererCssListener = listener => {
    if (typeof listener !== "function") return;
    state.rendererCssListeners.add(listener);
    ensureRendererCssWatch();
  };

  const normalizeSpellcheckWord = value =>
    String(value || "")
      .trim()
      .replace(/^[^A-Za-z0-9]+|[^A-Za-z0-9]+$/g, "")
      .replace(/\s+/g, " ");

  const getSpellcheckLanguages = () => {
    const configured = getHostSettingList("spellCheckLanguages");
    if (configured.length) {
      return configured.slice(0, 5);
    }

    const browserLanguages = Array.isArray(navigator.languages)
      ? navigator.languages
      : [navigator.language];
    const unique = [];
    browserLanguages.forEach(language => {
      if (typeof language !== "string") return;
      const normalized = language.trim();
      if (!normalized || unique.includes(normalized)) return;
      unique.push(normalized);
    });

    return unique.length ? unique.slice(0, 5) : ["en-US"];
  };

  const resolveSpellcheckSelection = () => {
    const active = document.activeElement;
    if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) {
      if (active.disabled || active.readOnly) {
        return null;
      }

      const value = String(active.value || "");
      let start = Number(active.selectionStart ?? 0);
      let end = Number(active.selectionEnd ?? start);

      if (start === end) {
        while (start > 0 && /[A-Za-z0-9'_’-]/.test(value[start - 1])) start -= 1;
        while (end < value.length && /[A-Za-z0-9'_’-]/.test(value[end])) end += 1;
      }

      const word = normalizeSpellcheckWord(value.slice(start, end));
      if (!word) {
        return null;
      }

      return {
        kind: "input",
        element: active,
        start,
        end,
        word,
      };
    }

    const selection = window.getSelection();
    const selectedText = normalizeSpellcheckWord(selection?.toString?.() || "");
    if (!selectedText) {
      return null;
    }

    return {
      kind: "selection",
      word: selectedText,
    };
  };

  const buildSpellcheckSuggestions = word => {
    const normalized = normalizeSpellcheckWord(word);
    if (!normalized) {
      return [];
    }

    const lower = normalized.toLocaleLowerCase();
    const titleCase = lower
      ? `${lower.slice(0, 1).toLocaleUpperCase()}${lower.slice(1)}`
      : normalized;
    const deDoubled = lower.replace(/(.)\1{2,}/g, "$1$1");
    const suggestions = [lower, titleCase, deDoubled].filter(
      candidate => candidate && candidate !== normalized
    );

    if (!suggestions.length) {
      suggestions.push(normalized);
    }

    return Array.from(new Set(suggestions)).slice(0, 5);
  };

  const getSpellcheckSuggestions = async word => {
    try {
      const result = await invoke("check_spelling", {
        word,
        languages: getSpellcheckLanguages(),
      });
      const suggestions = Array.isArray(result?.suggestions)
        ? result.suggestions.filter(candidate => typeof candidate === "string" && candidate)
        : [];
      if (suggestions.length) {
        return suggestions.slice(0, 5);
      }
    } catch (error) {
      console.warn("[Equirust] Native spellcheck lookup failed", error);
    }

    return buildSpellcheckSuggestions(word);
  };

  const notifySpellcheckResult = (word, suggestions) => {
    state.spellcheckResultListeners.forEach(listener => {
      try {
        listener(word, suggestions);
      } catch (error) {
        console.error("[Equirust]", error);
      }
    });
  };

  const replaceSpellcheckSelection = replacement => {
    const nextValue = String(replacement || "");
    const target = state.spellcheckSelection;
    state.spellcheckSelection = null;
    if (!target || !nextValue) {
      return;
    }

    if (
      target.kind === "input" &&
      (target.element instanceof HTMLInputElement || target.element instanceof HTMLTextAreaElement)
    ) {
      target.element.focus();
      target.element.setRangeText(nextValue, target.start, target.end, "end");
      target.element.dispatchEvent(new InputEvent("input", { bubbles: true, data: nextValue }));
      target.element.dispatchEvent(new Event("change", { bubbles: true }));
      return;
    }

    try {
      document.execCommand("insertText", false, nextValue);
    } catch (error) {
      console.warn("[Equirust] Failed to replace misspelling", error);
    }
  };

  const addSpellcheckWordToDictionary = async word => {
    const normalized = normalizeSpellcheckWord(word).toLocaleLowerCase();
    if (!normalized) {
      return;
    }

    const nextWords = Array.from(new Set([...state.spellcheckLearnedWords, normalized])).sort(
      (left, right) => left.localeCompare(right)
    );
    await persistHostSetting("spellCheckDictionary", nextWords);
  };

  const installSpellcheckBridge = () => {
    if (state.spellcheckContextMenuInstalled) return;

    document.addEventListener(
      "contextmenu",
      () => {
        if (!state.spellcheckResultListeners.size) {
          return;
        }

        const target = resolveSpellcheckSelection();
        state.spellcheckSelection = target;
        if (!target) {
          return;
        }

        const normalizedWord = normalizeSpellcheckWord(target.word);
        if (
          normalizedWord.length < 3 ||
          !/[A-Za-z]/.test(normalizedWord) ||
          state.spellcheckLearnedWords.has(normalizedWord.toLocaleLowerCase())
        ) {
          return;
        }

        void getSpellcheckSuggestions(normalizedWord).then(suggestions => {
          if (!suggestions.length) {
            return;
          }

          notifySpellcheckResult(normalizedWord, suggestions);
        });
      },
      true
    );

    state.spellcheckContextMenuInstalled = true;
  };

  const refreshQuickCssFromDisk = async () => {
    const css = await invoke("get_vencord_quick_css");
    if (typeof css === "string" && css !== state.quickCss) {
      state.quickCss = css;
      notifyQuickCssListeners();
    }

    return state.quickCss;
  };

  const pollVencordFileState = async () => {
    const next = await invoke("get_vencord_file_state");
    const nextQuickCssRevision = Number(next?.quickCssRevision ?? -1);
    const nextThemesRevision = Number(next?.themesRevision ?? -1);

    if (state.quickCssRevision == null) {
      state.quickCssRevision = nextQuickCssRevision;
    } else if (nextQuickCssRevision !== state.quickCssRevision) {
      state.quickCssRevision = nextQuickCssRevision;
      await refreshQuickCssFromDisk();
    }

    if (state.themesRevision == null) {
      state.themesRevision = nextThemesRevision;
    } else if (nextThemesRevision !== state.themesRevision) {
      state.themesRevision = nextThemesRevision;
      notifyThemeListeners();
    }

    return next;
  };

  const ensureVencordFileWatch = () => {
    if (state.vencordFileWatchTimer) return;
    if (!state.quickCssListeners.size && !state.themeListeners.size) return;

    if (!state.vencordFileWatchVisibilityBound) {
      state.vencordFileWatchVisibilityBound = true;
      document.addEventListener("visibilitychange", () => {
        if (document.hidden || (!state.quickCssListeners.size && !state.themeListeners.size)) return;
        if (state.vencordFileWatchTimer) {
          window.clearTimeout(state.vencordFileWatchTimer);
          state.vencordFileWatchTimer = null;
        }
        ensureVencordFileWatch();
      });
      window.addEventListener("focus", () => {
        if (!state.quickCssListeners.size && !state.themeListeners.size) return;
        if (state.vencordFileWatchTimer) {
          window.clearTimeout(state.vencordFileWatchTimer);
          state.vencordFileWatchTimer = null;
        }
        ensureVencordFileWatch();
      });
    }

    const delay =
      state.quickCssRevision == null && state.themesRevision == null
        ? 0
        : getAdaptivePollDelay(2000, 12000);
    state.vencordFileWatchTimer = window.setTimeout(() => {
      state.vencordFileWatchTimer = null;
      if (!state.quickCssListeners.size && !state.themeListeners.size) {
        state.quickCssRevision = null;
        state.themesRevision = null;
        return;
      }

      const refresh = shouldUseActivePolling()
        ? pollVencordFileState()
        : Promise.resolve(null);

      refresh
        .catch(error => console.error("[Equirust]", error))
        .finally(() => {
          ensureVencordFileWatch();
        });
    }, delay);
  };


  const installDesktopSettingsStyles = () => {
    if (document.getElementById("equirust-inline-settings-style")) return;

    const style = document.createElement("style");
    style.id = "equirust-inline-settings-style";
    style.textContent = `
      .vc-equirust-settings-page {
        display: grid;
        gap: 20px;
        max-width: 860px;
      }
      .vc-equirust-settings-page .equirust-settings__hero {
        padding: 22px 24px;
        border-radius: 18px;
        background:
          radial-gradient(circle at top right, rgba(88, 101, 242, 0.22), transparent 42%),
          linear-gradient(180deg, rgba(255,255,255,0.03), rgba(255,255,255,0)),
          #12161d;
        border: 1px solid rgba(255, 255, 255, 0.08);
        box-shadow: 0 20px 48px rgba(0, 0, 0, 0.28);
      }
      .vc-equirust-settings-page .equirust-settings__eyebrow {
        display: inline-flex;
        padding: 6px 10px;
        border-radius: 999px;
        background: rgba(88, 101, 242, 0.16);
        color: #c9d0ff;
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      .vc-equirust-settings-page .equirust-settings__heading {
        margin: 12px 0 6px;
        font-size: 30px;
        line-height: 1.05;
      }
      .vc-equirust-settings-page .equirust-settings__lead {
        margin: 0;
        color: #b5bac1;
        line-height: 1.5;
      }
      .vc-equirust-settings-page .equirust-settings__section {
        padding: 18px;
        border-radius: 16px;
        background: rgba(255, 255, 255, 0.035);
        border: 1px solid rgba(255, 255, 255, 0.06);
      }
      .vc-equirust-settings-page .equirust-settings__section-title {
        margin: 0 0 6px;
        font-size: 16px;
      }
      .vc-equirust-settings-page .equirust-settings__section-copy {
        margin: 0 0 14px;
        color: #aab1bb;
        line-height: 1.5;
      }
      .vc-equirust-settings-page .equirust-settings__actions {
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
      }
      .vc-equirust-settings-page .equirust-settings__action {
        border: 0;
        border-radius: 10px;
        padding: 10px 14px;
        background: rgba(88, 101, 242, 0.18);
        color: #eef1ff;
        font-weight: 600;
        cursor: pointer;
      }
      .vc-equirust-settings-page .equirust-settings__action--secondary {
        background: rgba(255, 255, 255, 0.06);
        color: #d7dce3;
      }
      .vc-equirust-settings-page .equirust-settings__action:disabled {
        opacity: 0.55;
        cursor: default;
      }
      .vc-equirust-settings-page .equirust-settings__grid {
        display: grid;
        gap: 12px;
      }
      .vc-equirust-settings-page .equirust-settings__update-notes {
        display: grid;
        gap: 6px;
        margin: 0 0 14px;
      }
      .vc-equirust-settings-page .equirust-settings__card {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 14px;
        align-items: center;
        padding: 14px 16px;
        border-radius: 14px;
        background: rgba(255, 255, 255, 0.03);
        border: 1px solid rgba(255, 255, 255, 0.05);
      }
      .vc-equirust-settings-page .equirust-settings__title-row {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
      }
      .vc-equirust-settings-page .equirust-settings__title {
        font-weight: 700;
      }
      .vc-equirust-settings-page .equirust-settings__badge {
        display: inline-flex;
        padding: 3px 8px;
        border-radius: 999px;
        background: rgba(255, 184, 108, 0.16);
        color: #ffd39a;
        font-size: 11px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.06em;
      }
      .vc-equirust-settings-page .equirust-settings__description {
        margin: 6px 0 0;
        color: #aab1bb;
        line-height: 1.45;
      }
      .vc-equirust-settings-page .equirust-settings__jump-nav {
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
        margin: 18px 0 0;
      }
      .vc-equirust-settings-page .equirust-settings__jump-button {
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.04);
        color: #d7dce3;
        padding: 8px 12px;
        font: inherit;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
      }
      .vc-equirust-settings-page .equirust-settings__hero-grid {
        display: grid;
        gap: 12px;
        grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
        margin-top: 18px;
      }
      .vc-equirust-settings-page .equirust-settings__metric {
        display: grid;
        gap: 6px;
        padding: 14px 16px;
        border-radius: 14px;
        background: rgba(255, 255, 255, 0.03);
        border: 1px solid rgba(255, 255, 255, 0.05);
      }
      .vc-equirust-settings-page .equirust-settings__metric-label {
        color: #8f98a3;
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }
      .vc-equirust-settings-page .equirust-settings__metric-value {
        color: #f5f7fb;
        font-size: 18px;
        font-weight: 800;
        line-height: 1.2;
      }
      .vc-equirust-settings-page .equirust-settings__metric-copy {
        margin: 0;
        color: #aab1bb;
        font-size: 13px;
        line-height: 1.45;
      }
      .vc-equirust-settings-page .equirust-settings__select {
        min-width: 220px;
        border-radius: 10px;
        border: 1px solid rgba(255, 255, 255, 0.08);
        background: rgba(255, 255, 255, 0.06);
        color: #eef1ff;
        padding: 10px 12px;
        font: inherit;
      }
      .vc-equirust-settings-page .equirust-settings__field {
        display: grid;
        gap: 8px;
      }
      .vc-equirust-settings-page .equirust-settings__input {
        min-width: 220px;
        border-radius: 10px;
        border: 1px solid rgba(255, 255, 255, 0.08);
        background: rgba(255, 255, 255, 0.06);
        color: #eef1ff;
        padding: 10px 12px;
        font: inherit;
      }
      .vc-equirust-settings-page .equirust-settings__hint {
        margin: 0;
        color: #8f98a3;
        font-size: 13px;
        line-height: 1.5;
      }
      .vc-equirust-settings-page .equirust-settings__subtle {
        color: #8f98a3;
      }
      .vc-equirust-settings-page .equirust-settings__card-side {
        display: flex;
        flex-direction: column;
        align-items: flex-end;
        gap: 8px;
      }
      .vc-equirust-settings-page .equirust-settings__switch {
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
      }
      .vc-equirust-settings-page .equirust-settings__switch:disabled {
        opacity: 0.55;
        cursor: default;
      }
      .vc-equirust-settings-page .equirust-settings__switch-track {
        position: relative;
        width: 44px;
        height: 26px;
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.14);
        transition: background 140ms ease, box-shadow 140ms ease;
        box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08);
      }
      .vc-equirust-settings-page .equirust-settings__switch-track::after {
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
      }
      .vc-equirust-settings-page .equirust-settings__switch[data-checked="true"] .equirust-settings__switch-track {
        background: rgba(88, 101, 242, 0.82);
      }
      .vc-equirust-settings-page .equirust-settings__switch[data-checked="true"] .equirust-settings__switch-track::after {
        transform: translateX(18px);
      }
      .vc-equirust-settings-page .equirust-settings__switch-label {
        min-width: 24px;
        text-align: right;
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.03em;
        text-transform: uppercase;
      }
      .vc-equirust-settings-page .equirust-settings__saving {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: 16px;
        color: #8f98a3;
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }
      .vc-equirust-settings-page .equirust-settings__details {
        margin-top: 18px;
        border-radius: 16px;
        border: 1px solid rgba(255, 255, 255, 0.06);
        background: rgba(255, 255, 255, 0.02);
      }
      .vc-equirust-settings-page .equirust-settings__details > summary {
        list-style: none;
        cursor: pointer;
        padding: 16px 18px;
        font-weight: 700;
        color: #eef1ff;
      }
      .vc-equirust-settings-page .equirust-settings__details > summary::-webkit-details-marker {
        display: none;
      }
      .vc-equirust-settings-page .equirust-settings__details-copy {
        margin: 0;
        padding: 0 18px 16px;
        color: #8f98a3;
        line-height: 1.5;
      }
      .vc-equirust-settings-page .equirust-settings__details-content {
        display: grid;
        gap: 18px;
        padding: 0 18px 18px;
      }
      @media (max-width: 720px) {
        .vc-equirust-settings-page .equirust-settings__card {
          grid-template-columns: 1fr;
        }
        .vc-equirust-settings-page .equirust-settings__card-side {
          align-items: flex-start;
        }
        .vc-equirust-settings-page .equirust-settings__switch {
          justify-content: flex-start;
        }
      }
    `;
    document.documentElement.appendChild(style);
  };

  const createDesktopSettingsComponent = React => {
    const h = React.createElement;

    return function EquirustSettingsPage() {
      const [, forceRender] = React.useState(0);
      const [viewHostSettings, setViewHostSettings] = React.useState(
        () => (state.hostSettings ? { ...state.hostSettings } : {})
      );
      const [viewNativeAutoStart, setViewNativeAutoStart] = React.useState(
        state.nativeAutoStartEnabled === true
      );
      const [savingHostSettings, setSavingHostSettings] = React.useState({});
      const [savingNativeAutoStart, setSavingNativeAutoStart] = React.useState(false);

      React.useEffect(() => {
        installDesktopSettingsStyles();

        const sync = () => {
          setViewHostSettings(state.hostSettings ? { ...state.hostSettings } : {});
          setViewNativeAutoStart(state.nativeAutoStartEnabled === true);
          forceRender(version => version + 1);
        };
        window.__EQUIRUST_SETTINGS_SYNC__ = sync;

        return () => {
          if (window.__EQUIRUST_SETTINGS_SYNC__ === sync) {
            delete window.__EQUIRUST_SETTINGS_SYNC__;
          }
        };
      }, []);

      React.useEffect(() => {
        Promise.allSettled([
          refreshNativeAutoStart(),
          refreshHostUpdateStatus(),
          refreshRuntimeUpdateStatus(),
        ]).catch(error => {
          console.error("[Equirust]", error);
        });

        scheduleDeferredTask(() => refreshHostUpdateDownloadState(), { delay: 180 });
        scheduleDeferredTask(() => refreshArRPCStatus(), { delay: 320 });
        scheduleDeferredTask(() => refreshFileManagerState(), { delay: 520 });
      }, []);

      React.useEffect(() => {
        const phase = String(state.hostUpdateDownloadState?.phase || "idle");
        if (phase !== "downloading" && phase !== "launching") {
          return undefined;
        }

        const timer = window.setInterval(() => {
          refreshHostUpdateDownloadState().catch(error => {
            console.error("[Equirust]", error);
          });
        }, 1000);

        return () => {
          window.clearInterval(timer);
        };
      }, [String(state.hostUpdateDownloadState?.phase || "idle")]);

      const getHostSettingValue = definition => {
        const value = viewHostSettings?.[definition.key];
        return typeof value === "boolean" ? value : Boolean(definition.defaultValue);
      };

      const getHostSettingChoice = (key, defaultValue) => {
        const value = viewHostSettings?.[key];
        return typeof value === "string" && value.length ? value : String(defaultValue);
      };

      const getHostSettingText = (key, defaultValue = "") => {
        const value = viewHostSettings?.[key];
        return typeof value === "string" ? value : String(defaultValue);
      };

      const getHostSettingNumberText = key => {
        const value = viewHostSettings?.[key];
        if (typeof value === "string") {
          return value;
        }
        return typeof value === "number" && Number.isFinite(value) ? String(value) : "";
      };

      const hostSettingDisabled = definition => {
        switch (definition.key) {
          case "minimizeToTray":
          case "clickTrayToShowHide":
            return !getHostSettingValue({ key: "tray", defaultValue: true });
          case "autoStartMinimized":
            return !viewNativeAutoStart;
          case "hardwareVideoAcceleration":
            return !getHostSettingValue({ key: "hardwareAcceleration", defaultValue: true });
          case "badgeOnlyForMentions":
            return !getHostSettingValue({ key: "appBadge", defaultValue: true });
          case "enableMenu":
            return getHostSettingValue({
              key: "customTitleBar",
              defaultValue: String(state.versions.platform || "").toLowerCase() === "windows",
            });
          default:
            return false;
        }
      };

      const isHostSettingSaving = key => savingHostSettings?.[key] === true;

      const commitHostSetting = async (key, value) => {
        setViewHostSettings(current => ({ ...(current || {}), [key]: value }));
        setSavingHostSettings(current => ({ ...(current || {}), [key]: true }));

        try {
          const next = await persistHostSetting(key, value);
          setViewHostSettings(next ? { ...next } : (state.hostSettings ? { ...state.hostSettings } : {}));
          return next;
        } catch (error) {
          setViewHostSettings(state.hostSettings ? { ...state.hostSettings } : {});
          throw error;
        } finally {
          setSavingHostSettings(current => {
            const next = { ...(current || {}) };
            delete next[key];
            return next;
          });
        }
      };

      const commitNativeAutoStart = async enabled => {
        setViewNativeAutoStart(enabled);
        setSavingNativeAutoStart(true);

        try {
          const next = await persistNativeAutoStart(enabled);
          setViewNativeAutoStart(next === true);
          return next;
        } catch (error) {
          setViewNativeAutoStart(state.nativeAutoStartEnabled === true);
          throw error;
        } finally {
          setSavingNativeAutoStart(false);
        }
      };

      const updateLocalHostSetting = (key, value) => {
        setViewHostSettings(current => ({ ...(current || {}), [key]: value }));
      };

      const scrollToSettingsSection = sectionKey => {
        const target = document.querySelector(`[data-equirust-section="${sectionKey}"]`);
        if (target instanceof HTMLElement) {
          target.scrollIntoView({ behavior: "smooth", block: "start" });
        }
      };

      const renderSwitchControl = (checked, disabled, saving, onToggle) =>
        h(
          "div",
          {
            className: "equirust-settings__card-side",
          },
          h(
            "button",
            {
              type: "button",
              className: "equirust-settings__switch",
              role: "switch",
              "aria-checked": checked,
              "data-checked": checked ? "true" : "false",
              disabled,
              onClick: () => {
                if (disabled) return;
                onToggle(!checked);
              },
            },
            h("span", {
              className: "equirust-settings__switch-track",
              "aria-hidden": "true",
            }),
            h(
              "span",
              {
                className: "equirust-settings__switch-label",
              },
              checked ? "On" : "Off"
            )
          ),
          h(
            "span",
            {
              className: "equirust-settings__saving",
            },
            saving ? "Saving" : "\u00A0"
          )
        );

      const renderSettingCard = definition => {
        const checked = getHostSettingValue(definition);
        const disabled = hostSettingDisabled(definition);
        const saving = isHostSettingSaving(definition.key);

        return h(
          "div",
          {
            key: definition.key,
            className: "equirust-settings__card",
          },
          h(
            "div",
            {
              className: "equirust-settings__copy",
            },
            h(
              "div",
              {
                className: "equirust-settings__title-row",
              },
              h(
                "span",
                {
                  className: "equirust-settings__title",
                },
                definition.title
              ),
              definition.restartRequired
                ? h(
                    "span",
                    {
                      className: "equirust-settings__badge",
                    },
                    "Restart"
                  )
                : null
            ),
            h(
              "p",
              {
                className: "equirust-settings__description",
              },
              definition.description
            )
          ),
          renderSwitchControl(checked, disabled, saving, nextChecked => {
            commitHostSetting(definition.key, nextChecked).catch(error => {
              console.error("[Equirust]", error);
              forceRender(version => version + 1);
            });
          })
        );
      };

      const renderStartupCard = () => {
        if (!supportsNativeAutoStart()) return null;

        return h(
          "div",
          {
            className: "equirust-settings__card",
          },
          h(
            "div",
            {
              className: "equirust-settings__copy",
            },
            h(
              "div",
              {
                className: "equirust-settings__title-row",
              },
              h(
                "span",
                {
                  className: "equirust-settings__title",
                },
                "Start With System"
              )
            ),
            h(
              "p",
              {
                className: "equirust-settings__description",
              },
              "Start Equirust automatically when you sign in."
            )
          ),
          renderSwitchControl(
            viewNativeAutoStart === true,
            false,
            savingNativeAutoStart,
            checked => {
              commitNativeAutoStart(checked).catch(error => {
                console.error("[Equirust]", error);
                forceRender(version => version + 1);
              });
            }
          )
        );
      };

      const renderTransparencyCard = () => {
        if (!supportsWindowsTransparency()) return null;

        const value = getHostSettingChoice("transparencyOption", "none");
        const options = [
          {
            value: "none",
            label: "None",
          },
          {
            value: "mica",
            label: "Mica",
          },
          {
            value: "tabbed",
            label: "Tabbed",
          },
          {
            value: "acrylic",
            label: "Acrylic",
          },
        ];

        return h(
          "label",
          {
            className: "equirust-settings__card",
            key: "transparencyOption",
          },
          h(
            "div",
            {
              className: "equirust-settings__copy",
            },
            h(
              "div",
              {
                className: "equirust-settings__title-row",
              },
              h(
                "span",
                {
                  className: "equirust-settings__title",
                },
                "Window Transparency"
              ),
              h(
                "span",
                {
                  className: "equirust-settings__badge",
                },
                "Restart"
              )
            ),
            h(
              "p",
              {
                className: "equirust-settings__description",
              },
              "Use a Windows backdrop effect behind the Discord window."
            )
          ),
          h(
            "select",
            {
              className: "equirust-settings__select",
              value,
              onChange: event => {
                commitHostSetting("transparencyOption", event.currentTarget.value).catch(error => {
                  console.error("[Equirust]", error);
                  forceRender(version => version + 1);
                });
              },
            },
            options.map(option =>
              h(
                "option",
                {
                  key: option.value,
                  value: option.value,
                },
                option.label
              )
            )
          )
        );
      };

      const renderRichPresenceSection = () => {
        return h(
          "section",
          {
            className: "equirust-settings__section",
            "data-equirust-section": "rich-presence",
          },
          h(
            "h3",
            {
              className: "equirust-settings__section-title",
            },
            "Presence and Privacy"
          ),
          h(
            "p",
            {
              className: "equirust-settings__section-copy",
            },
            "Control game detection and Discord activity. Advanced diagnostics stay below."
          ),
          h(
            "div",
            {
              className: "equirust-settings__grid",
            },
            renderSettingCard({
              key: "arRpc",
              title: "Enable Rich Presence",
              description: "Show supported games and apps in Discord.",
              defaultValue: false,
            }),
            renderSettingCard({
              key: "arRpcProcessScanning",
              title: "Process Scanning",
              description: "Detect running apps and games automatically.",
              defaultValue: true,
            })
          )
        );
      };

      const renderUpdaterSection = () => {
        const hostUpdate = state.hostUpdateStatus;
        const runtimeUpdate = state.runtimeUpdateStatus;
        const download = state.hostUpdateDownloadState;
        const downloadPhase = String(download?.phase || "idle");
        const downloadBusy = downloadPhase === "downloading" || downloadPhase === "launching";

        const renderUpdateCard = (title, update, options = {}) => {
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
          const releaseNotes = (() => {
            if (typeof update?.releaseNotes !== "string") {
              return [];
            }

            const trimmed = update.releaseNotes.trim();
            if (!trimmed) {
              return [];
            }

            return trimmed
              .split(/\r?\n/)
              .map(line => String(line || "").trim())
              .filter(Boolean)
              .slice(0, 3);
          })();
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
            {
              className: "equirust-settings__section",
              key: title,
            },
            h(
              "h4",
              {
                className: "equirust-settings__section-title",
                style: { marginBottom: "8px" },
              },
              title
            ),
            h(
              "p",
              {
                className: "equirust-settings__section-copy",
              },
              summary
            ),
            releaseNotes.length
              ? h(
                  "div",
                  {
                    className: "equirust-settings__update-notes",
                  },
                  releaseNotes.map((line, index) =>
                    h(
                      "p",
                      {
                        key: title + "-update-note-" + index,
                        className: "equirust-settings__hint",
                      },
                      line
                    )
                  )
                )
              : null,
            error
              ? h(
                  "p",
                  {
                    className: "equirust-settings__hint",
                  },
                  error
                )
              : null,
            update?.ignored
              ? h(
                  "p",
                  {
                    className: "equirust-settings__hint",
                  },
                  "This release is currently ignored."
                )
              : null,
            update?.snoozed
              ? h(
                  "p",
                  {
                    className: "equirust-settings__hint",
                  },
                  "Update prompts are currently snoozed for one day."
                )
              : null,
            options.statusHint
              ? h(
                  "p",
                  {
                    className: "equirust-settings__hint",
                  },
                  options.statusHint
                )
              : null,
            h(
              "div",
              {
                className: "equirust-settings__actions",
              },
              h(
                "button",
                {
                  type: "button",
                  className: "equirust-settings__action",
                  onClick: () => {
                    options.primaryAction?.().catch(error => console.error("[Equirust]", error));
                  },
                  disabled: options.primaryDisabled === true,
                },
                options.primaryLabel
              ),
              h(
                "button",
                {
                  type: "button",
                  className: "equirust-settings__action equirust-settings__action--secondary",
                  onClick: () => {
                    options.refreshAction?.().catch(error => console.error("[Equirust]", error));
                  },
                  disabled: options.refreshDisabled === true,
                },
                "Check Again"
              ),
              updateReady
                ? h(
                    "button",
                    {
                      type: "button",
                      className: "equirust-settings__action equirust-settings__action--secondary",
                      onClick: () => {
                        options.snoozeAction?.().catch(error => console.error("[Equirust]", error));
                      },
                      disabled: options.secondaryDisabled === true,
                    },
                    "Snooze 1 Day"
                  )
                : null,
              updateReady && update?.latestVersion
                ? h(
                    "button",
                    {
                      type: "button",
                      className: "equirust-settings__action equirust-settings__action--secondary",
                      onClick: () => {
                        options.ignoreAction?.(update.latestVersion).catch(error => console.error("[Equirust]", error));
                      },
                      disabled: options.secondaryDisabled === true,
                    },
                    "Ignore This Release"
                  )
                : null
            )
          );
        };

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
          {
            className: "equirust-settings__section",
          },
          h(
            "h3",
            {
              className: "equirust-settings__section-title",
            },
            "Update Center"
          ),
          h(
            "p",
            {
              className: "equirust-settings__section-copy",
            },
            "Host updates and linked Equicord runtime updates are checked separately, but managed in one place."
          ),
          h(
            "div",
            {
              className: "equirust-settings__grid",
            },
            renderUpdateCard("Equirust Host", hostUpdate, {
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
            }),
            renderUpdateCard("Equicord Runtime", runtimeUpdate, {
              primaryAction: runtimePrimaryAction,
              primaryDisabled: false,
              primaryLabel: runtimePrimaryLabel,
              refreshAction: refreshRuntimeUpdateStatus,
              snoozeAction: snoozeRuntimeUpdate,
              ignoreAction: ignoreRuntimeUpdate,
              statusHint: runtimeStatusHint,
            })
          )
        );
      };

      const renderFileManagerSection = () => {
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
        const handleAssetAction = (asset, reset) => {
          chooseUserAsset(asset, reset).catch(error => console.error("[Equirust]", error));
        };

        return h(
          "section",
          {
            className: "equirust-settings__section",
          },
          h(
            "h3",
            {
              className: "equirust-settings__section-title",
            },
            "Files and Runtime"
          ),
          h(
            "p",
            {
              className: "equirust-settings__section-copy",
            },
            runtimeMessage
          ),
          activeRuntimeDir
            ? h(
                "p",
                {
                  className: "equirust-settings__hint",
                },
                `${runtimeSourceLabel}: ${activeRuntimeDir}`
              )
            : null,
          h(
            "div",
            {
              className: "equirust-settings__actions",
            },
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action",
                onClick: () => {
                  openUserAssetsFolder().catch(error => console.error("[Equirust]", error));
                },
              },
              "Open User Assets"
            ),
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action",
                onClick: () => {
                  selectCustomVencordDir(false).catch(error => console.error("[Equirust]", error));
                },
              },
              "Change Custom Dir"
            ),
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {
                  showCustomVencordDir().catch(error => console.error("[Equirust]", error));
                },
                disabled: !usingCustomDir,
              },
              "Open Custom Dir"
            ),
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {
                  selectCustomVencordDir(true).catch(error => console.error("[Equirust]", error));
                },
                disabled: !usingCustomDir,
              },
              "Reset Custom Dir"
            )
          ),
          h(
            "div",
            {
              className: "equirust-settings__actions",
              style: { marginTop: "10px" },
            },
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {
                  handleAssetAction("tray", false);
                },
              },
              "Set Tray Icon"
            ),
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {
                  handleAssetAction("trayUnread", false);
                },
              },
              "Set Unread Tray Icon"
            ),
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {
                  handleAssetAction("tray", true);
                  handleAssetAction("trayUnread", true);
                  handleAssetAction("traySpeaking", true);
                  handleAssetAction("trayIdle", true);
                  handleAssetAction("trayMuted", true);
                  handleAssetAction("trayDeafened", true);
                },
              },
              "Reset Asset Overrides"
            )
          )
        );
      };

      const renderDebugSection = () => {
        if (state.debugBuild !== true) {
          return null;
        }

        const debugDefinitions = [
          {
            key: "debugStandardDiagnostics",
            title: "Standard Diagnostics",
            description:
              "Lower overhead. Runtime bridge, updater, cloud, WebView memory snapshots, and Rich Presence diagnostics.",
            defaultValue: true,
          },
          {
            key: "debugMediaDiagnostics",
            title: "Media Diagnostics",
            description:
              "Higher overhead. Voice, screen share, desktop stream, WebRTC, SDP, ABR, and encoder diagnostics.",
            defaultValue: true,
          },
        ];

        return h(
          "section",
          {
            className: "equirust-settings__section",
          },
          h(
            "h3",
            {
              className: "equirust-settings__section-title",
            },
            "Debug"
          ),
          h(
            "p",
            {
              className: "equirust-settings__section-copy",
            },
            "Debug builds start with both diagnostic groups enabled. Disable the heavier media group first if you need a lighter repro."
          ),
          h(
            "div",
            {
              className: "equirust-settings__grid",
            },
            debugDefinitions.map(renderSettingCard)
          ),
          h(
            "div",
            {
              className: "equirust-settings__actions",
              style: { marginTop: "10px" },
            },
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {
                  openDebugPage("gpu").catch(error => console.error("[Equirust]", error));
                },
              },
              "Open GPU Debug"
            ),
            h(
              "button",
              {
                type: "button",
                className: "equirust-settings__action equirust-settings__action--secondary",
                onClick: () => {
                  openDebugPage("webrtc-internals").catch(error => console.error("[Equirust]", error));
                },
              },
              "Open WebRTC Internals"
            )
          )
        );
      };

      const getSettingDefinition = key =>
        desktopSettingDefinitions.find(definition => definition.key === key);

      const renderOverviewMetric = (label, value, copy) =>
        h(
          "div",
          {
            className: "equirust-settings__metric",
            key: label,
          },
          h(
            "span",
            {
              className: "equirust-settings__metric-label",
            },
            label
          ),
          h(
            "span",
            {
              className: "equirust-settings__metric-value",
            },
            value
          ),
          h(
            "p",
            {
              className: "equirust-settings__metric-copy",
            },
            copy
          )
        );

      const quickControlDefinitions = [
        {
          key: "arRpc",
          title: "Rich Presence",
          description: "Show game and app activity in Discord.",
          defaultValue: false,
        },
        getSettingDefinition("customTitleBar"),
        getSettingDefinition("hardwareAcceleration"),
        getSettingDefinition("tray"),
      ].filter(Boolean);
      const performanceDefinitions = [
        "hardwareAcceleration",
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
        {
          className: "vc-equirust-settings-page",
        },
        h(
          "section",
          {
            className: "equirust-settings__hero",
          },
          h(
            "span",
            {
              className: "equirust-settings__eyebrow",
            },
            "Equirust Settings"
          ),
          h(
            "h2",
            {
              className: "equirust-settings__heading",
            },
            "Equirust"
          ),
          h(
            "p",
            {
              className: "equirust-settings__lead",
            },
            "Common controls are first. Restart-tagged options apply after relaunch."
          ),
          h(
            "div",
            {
              className: "equirust-settings__hero-grid",
            },
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
            {
              className: "equirust-settings__jump-nav",
            },
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
                {
                  key,
                  type: "button",
                  className: "equirust-settings__jump-button",
                  onClick: () => scrollToSettingsSection(key),
                },
                label
              )
            )
          )
        ),
        h(
          "section",
          {
            className: "equirust-settings__section",
            "data-equirust-section": "quick-controls",
          },
          h(
            "h3",
            {
              className: "equirust-settings__section-title",
            },
            "Most Used"
          ),
          h(
            "p",
            {
              className: "equirust-settings__section-copy",
            },
            "Most people only need these."
          ),
          h(
            "div",
            {
              className: "equirust-settings__grid",
            },
            supportsNativeAutoStart() ? renderStartupCard() : null,
            quickControlDefinitions.map(renderSettingCard)
          )
        ),
        h(
          "div",
          {
            "data-equirust-section": "updates",
          },
          renderUpdaterSection()
        ),
        supportsNativeAutoStart()
          ? h(
              "section",
              {
                className: "equirust-settings__section",
                "data-equirust-section": "startup",
              },
              h(
                "h3",
                {
                  className: "equirust-settings__section-title",
                },
                "Startup"
              ),
              h(
                "p",
              {
                className: "equirust-settings__section-copy",
              },
                "Choose what happens when Windows starts Equirust."
              ),
              h(
                "div",
                {
                  className: "equirust-settings__grid",
                },
                renderSettingCard(getSettingDefinition("autoStartMinimized"))
              )
            )
          : null,
        h(
          "div",
          {
            "data-equirust-section": "privacy",
          },
          renderRichPresenceSection()
        ),
        h(
          "section",
          {
            className: "equirust-settings__section",
            "data-equirust-section": "performance",
          },
          h(
            "h3",
            {
              className: "equirust-settings__section-title",
            },
            "Performance"
          ),
          h(
            "p",
            {
              className: "equirust-settings__section-copy",
            },
            "Tune browser rendering and taskbar attention behavior. Browser video acceleration stays managed by the host."
          ),
          h(
            "div",
            {
              className: "equirust-settings__grid",
            },
            performanceDefinitions.map(renderSettingCard)
          )
        ),
        h(
          "section",
          {
            className: "equirust-settings__section",
            "data-equirust-section": "preferences",
          },
          h(
            "h3",
            {
              className: "equirust-settings__section-title",
            },
            "Preferences"
          ),
          h(
            "p",
            {
              className: "equirust-settings__section-copy",
            },
            "Adjust everyday window and navigation behavior."
          ),
          h(
            "div",
            {
              className: "equirust-settings__grid",
            },
            renderTransparencyCard(),
            preferenceDefinitions.map(renderSettingCard)
          )
        ),
        h(
          "details",
          {
            className: "equirust-settings__details",
            "data-equirust-section": "advanced",
          },
          h("summary", null, "Advanced"),
          h(
            "p",
            {
              className: "equirust-settings__details-copy",
            },
            state.debugBuild
              ? "Extra folders, runtime controls, and debug tools live here."
              : "Extra folders and runtime controls live here."
          ),
          h(
            "div",
            {
              className: "equirust-settings__details-content",
            },
            h(
              "section",
              {
                className: "equirust-settings__section",
              },
              h(
                "h3",
                {
                  className: "equirust-settings__section-title",
                },
                "Themes, QuickCSS, and Plugins"
              ),
              h(
                "p",
                {
                  className: "equirust-settings__section-copy",
                },
                  "Use these shortcuts for themes, QuickCSS, and Equicord files."
              ),
              h(
                "div",
                {
                  className: "equirust-settings__actions",
                },
                h(
                  "button",
                  {
                    type: "button",
                    className: "equirust-settings__action",
                    onClick: () => invoke("open_vencord_quick_css"),
                  },
                  "Open QuickCSS"
                ),
                h(
                  "button",
                  {
                    type: "button",
                    className: "equirust-settings__action",
                    onClick: () => invoke("open_vencord_themes_folder"),
                  },
                  "Open Themes Folder"
                ),
                h(
                  "button",
                  {
                    type: "button",
                    className: "equirust-settings__action",
                    onClick: () => invoke("open_vencord_settings_folder"),
                  },
                  "Open Equicord Settings Folder"
                )
              )
            ),
            renderDebugSection(),
            renderFileManagerSection()
          )
        ),
        h(
          "p",
          {
            className: "equirust-settings__hint",
            style: { marginTop: "18px" },
          },
          "Settings marked Restart are saved now and apply after the next relaunch."
        )
      );
    };
  };

  const installVencordSettingsEntry = () => {
    if (state.vencordSettingsReady || !isDiscordHost()) return;

    const settingsPlugin = window.Vencord?.Plugins?.plugins?.Settings;
    const React = window.Vencord?.Webpack?.Common?.React;
    if (!settingsPlugin || !React) return;

    installDesktopSettingsStyles();

    const customEntries = settingsPlugin.customEntries;
    if (!Array.isArray(customEntries)) return;

    const entryKey = "equirust_settings";
    const SettingsPage = createDesktopSettingsComponent(React);

    if (!customEntries.some(entry => entry?.key === entryKey)) {
      customEntries.push({
        key: entryKey,
        title: "Equirust",
        Component: SettingsPage,
        Icon: () => React.createElement(
          "svg",
          {
            width: 18,
            height: 18,
            viewBox: "0 0 24 24",
            fill: "none",
            "aria-hidden": "true",
          },
          React.createElement("path", {
            d: "M12 2.75 20.25 7v10L12 21.25 3.75 17V7L12 2.75Z",
            stroke: "currentColor",
            strokeWidth: "1.75",
            strokeLinejoin: "round",
          }),
          React.createElement("path", {
            d: "M8.5 9.5h7v1.75h-7zm0 3.25h7v1.75h-7z",
            fill: "currentColor",
          })
        ),
      });
    }

    state.vencordSettingsReady = true;
    report(`vencord_settings_registered=true entries=${customEntries.length} sections=0`);
  };

  const stopVencordSettingsWatcher = () => {
    if (state.vencordSettingsObserver) {
      state.vencordSettingsObserver.disconnect();
      state.vencordSettingsObserver = null;
    }
  };

  const installVencordSettingsWatcher = () => {
    if (state.vencordSettingsObserver || !isDiscordHost()) return;
    installVencordSettingsEntry();
    if (state.vencordSettingsReady) {
      return;
    }

    const observerTarget = document.body || document.documentElement;
    if (!observerTarget) {
      return;
    }

    state.vencordSettingsObserver = new MutationObserver(() => {
      installVencordSettingsEntry();
      if (state.vencordSettingsReady) {
        stopVencordSettingsWatcher();
      }
    });
    state.vencordSettingsObserver.observe(observerTarget, {
      subtree: true,
      childList: true,
    });
  };

  const ready = () => {
    if (installHostRuntime && shouldUseCustomTitleBar()) {
      installTitlebar();
      scheduleAfterFirstPaint(() => window.__EQUIRUST_TITLEBAR_SYNC__?.(), {
        delay: 0,
        timeout: 900,
      });
    }

    scheduleAfterFirstPaint(() => installVoiceDiagnostics(), { delay: 120, timeout: 1200 });
    if (installModRuntime) {
      ensureVoiceToggleBridge();
      scheduleAfterFirstPaint(() => {
        installMediaCompatibilityPatches();
        installNativeScreenShareSdpPatch();
        installNativeScreenShareAbr();
        installNitroStreamQualityBypassPatch();
        installGoLiveQualityPatch();
        installGoLiveDispatchPatch();
        installDisplayMediaCompatibilityPatches();
      }, { delay: 80, timeout: 1400 });
      scheduleAfterFirstPaint(() => warmDesktopStreamStartupCaches(), { delay: 900, timeout: 2000 });
      scheduleAfterFirstPaint(() => installCloudBackendInfoWatcher(), { delay: 420, timeout: 1800 });
      scheduleAfterFirstPaint(() => installVencordSettingsWatcher(), { delay: 560, timeout: 2000 });
    }
    if (installHostRuntime) {
      scheduleAfterFirstPaint(() => installNotificationSync(), { delay: 180, timeout: 1400 });
      scheduleAfterFirstPaint(() => installTypingIndicator(), { delay: 420, timeout: 1800 });
    }
    if (installHostRuntime && installModRuntime) {
      scheduleAfterFirstPaint(() => installVoiceTrayWatcher(), { delay: 680, timeout: 2200 });
    }
    scheduleAfterFirstPaint(() => {
      const webpack = window.Vencord?.Webpack;
      if (!webpack) return;
      try {
        const m = webpack.find(exports => typeof exports?.GetWindowFullscreenTypeByPid === "function");
        if (m) {
          m.GetWindowFullscreenTypeByPid = () => 0;
          report("discord_utils_patch=success");
        } else {
          report("discord_utils_patch=not_found");
        }
      } catch (error) {
        report("discord_utils_patch=error:" + String(error?.message || error));
      }
    }, { delay: 800, timeout: 2200 });
  };

  if (installModRuntime) {
    createVencordNative();
    installCloudFetchProxy();
    installVencordRuntime();
    scheduleAfterFirstPaint(() => ensureArrpcBridge(), { delay: 240, timeout: 1600 });
    scheduleAfterFirstPaint(() => {
      try {
        const wf = window.Vencord?.Webpack?.waitFor;
        if (typeof wf === "function") {
          wf(
            exports => typeof exports?.GetWindowFullscreenTypeByPid === "function",
            m => {
              m.GetWindowFullscreenTypeByPid = () => 0;
              report("discord_utils_waitfor=applied");
            }
          );
          report("discord_utils_waitfor=registered");
        } else {
          report("discord_utils_waitfor=unavailable");
        }
      } catch (error) {
        report("discord_utils_waitfor=error:" + String(error?.message || error));
      }
    }, { delay: 320, timeout: 1800 });
  }

  whenDocumentBodyReady(ready);

    return true;
  };

  if (startBootstrap()) {
    return;
  }

  let bootstrapAttempts = 0;
  const bootstrapRetryTimer = window.setInterval(() => {
    bootstrapAttempts += 1;
    if (startBootstrap()) {
      window.clearInterval(bootstrapRetryTimer);
      return;
    }

    if (bootstrapAttempts >= 120) {
      window.clearInterval(bootstrapRetryTimer);
      console.warn("[Equirust] Tauri invoke bridge is unavailable after retry window.");
    }
  }, 25);
})();
