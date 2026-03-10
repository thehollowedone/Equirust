  const installMediaCompatibilityPatches = () => {
    if (state.mediaCompatReady || !isDiscordHost()) return;
    if (typeof navigator.mediaDevices?.getUserMedia !== "function") return;
    try {
      const fixAudioTrackConstraints = constraint => {
        if (!constraint || typeof constraint !== "object") return;

        const target =
          Array.isArray(constraint.advanced)
            ? constraint.advanced.find(option => option && Object.prototype.hasOwnProperty.call(option, "autoGainControl")) || constraint
            : constraint;
        const automaticGainControl = getAutomaticGainControlPreference();
        if (typeof automaticGainControl === "boolean") {
          target.autoGainControl = automaticGainControl;
        }
      };

      const fixVideoTrackConstraints = constraint => {
        if (!constraint || typeof constraint !== "object") return;
        if (typeof constraint.deviceId === "string" && constraint.deviceId !== "default") {
          constraint.deviceId = { exact: constraint.deviceId };
        }
      };

      const fixStreamConstraints = constraints => {
        if (!constraints || typeof constraints !== "object") return;

        if (constraints.audio) {
          if (typeof constraints.audio !== "object") {
            constraints.audio = {};
          }
          fixAudioTrackConstraints(constraints.audio);
        }

        if (constraints.video) {
          if (typeof constraints.video !== "object") {
            constraints.video = {};
          }
          fixVideoTrackConstraints(constraints.video);
        }
      };

      const originalGetUserMedia = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
      navigator.mediaDevices.getUserMedia = function(constraints) {
        try {
          fixStreamConstraints(constraints);
        } catch (error) {
          console.warn("[Equirust] Failed to normalize getUserMedia constraints", error);
        }

        return originalGetUserMedia(constraints);
      };

      if (typeof window.MediaStreamTrack?.prototype?.applyConstraints === "function") {
        const originalApplyConstraints = window.MediaStreamTrack.prototype.applyConstraints;
        window.MediaStreamTrack.prototype.applyConstraints = function(constraints) {
          if (constraints) {
            try {
              if (this.kind === "audio") {
                fixAudioTrackConstraints(constraints);
              } else if (this.kind === "video") {
                fixVideoTrackConstraints(constraints);
              }
            } catch (error) {
              console.warn("[Equirust] Failed to normalize track constraints", error);
            }
          }

          const result = originalApplyConstraints.call(this, constraints);
          if (!result || typeof result.catch !== "function") {
            return result;
          }

          return result.catch(error => {
            const message =
              error && typeof error.message === "string"
                ? error.message
                : String(error);
            const name = error && typeof error.name === "string" ? error.name : "";
            const ended = this?.readyState === "ended";
            const overconstrained =
              name === "OverconstrainedError" ||
              /constraint/i.test(name) ||
              /constraint/i.test(message) ||
              /cannot satisfy constraints/i.test(message);
            if (ended || overconstrained) {
              report(
                `media_compat_apply_constraints_ignored kind=${String(this?.kind || "<unknown>")} ended=${ended} message=${message}`,
                { category: "media" }
              );
              return undefined;
            }
            throw error;
          });
        };
      }

      state.mediaCompatReady = true;
      report("media_compat_installed=true");
    } catch (error) {
      state.mediaCompatReady = false;
      report(
        `media_compat_install_failed=${
          error && error.message ? error.message : String(error)
        }`,
        { force: true }
      );
    }
  };

  const readScreenShareQuality = () => {
    try {
      const raw = window.localStorage?.getItem("EquibopState");
      if (!raw) {
        return { frameRate: 60, height: 1080, width: 1920, resolutionMode: "1080" };
      }

      const parsed = JSON.parse(raw);
      const frameRate = Number(parsed?.screenshareQuality?.frameRate ?? 60);
      const storedResolution = String(parsed?.screenshareQuality?.resolution ?? 1080);
      const height = Number(
        storedResolution.toLowerCase() === "source"
          ? parsed?.screenshareQuality?.height ?? 1080
          : storedResolution
      );
      const width = Number(parsed?.screenshareQuality?.width ?? 0);
      const safeFrameRate = Number.isFinite(frameRate) && frameRate > 0 ? frameRate : 60;
      const safeHeight = Number.isFinite(height) && height >= 480 ? height : 1080;
      const safeWidth =
        Number.isFinite(width) && width >= 2 ? Math.round(width) : Math.round(safeHeight * (16 / 9));
      const resolutionMode = storedResolution.toLowerCase() === "source" ? "source" : String(safeHeight);

      return {
        frameRate: safeFrameRate,
        height: safeHeight,
        width: safeWidth,
        resolutionMode,
      };
    } catch (error) {
      console.warn("[Equirust] Failed to read stored screen share quality", error);
      return { frameRate: 60, height: 1080, width: 1920, resolutionMode: "1080" };
    }
  };

  const persistScreenShareQuality = quality => {
    try {
      const raw = window.localStorage?.getItem("EquibopState");
      const parsed = raw ? JSON.parse(raw) : {};
      const resolutionMode =
        String(quality?.resolutionMode || "").toLowerCase() === "source"
          ? "source"
          : String(quality?.height || 1080);
      parsed.screenshareQuality = {
        resolution: resolutionMode,
        height: String(quality?.height || 1080),
        width: String(quality?.width || 1920),
        frameRate: String(quality?.frameRate || 60),
      };
      window.localStorage?.setItem("EquibopState", JSON.stringify(parsed));
    } catch (error) {
      console.warn("[Equirust] Failed to persist screen share quality", error);
    }
  };
  const readScreenShareQualityRememberPreference = () => {
    try {
      const raw = window.localStorage?.getItem("EquibopState");
      if (!raw) {
        return true;
      }
      const parsed = JSON.parse(raw);
      return typeof parsed?.screenShareRememberQuality === "boolean"
        ? parsed.screenShareRememberQuality
        : true;
    } catch {
      return true;
    }
  };
  const persistScreenShareRememberPreference = remember => {
    try {
      const raw = window.localStorage?.getItem("EquibopState");
      const parsed = raw ? JSON.parse(raw) : {};
      parsed.screenShareRememberQuality = remember === true;
      window.localStorage?.setItem("EquibopState", JSON.stringify(parsed));
    } catch (error) {
      console.warn("[Equirust] Failed to persist screen share remember preference", error);
    }
  };
  const screenShareConstraintState = new WeakMap();

  const applyScreenShareTrackConstraints = async (videoTrack, quality, contentHint) => {
    if (!videoTrack) return false;
    const targetFrameRate = Math.max(1, Number(quality?.frameRate || 60) || 60);
    const targetWidth = Math.max(2, Number(quality?.width || 1920) || 1920);
    const targetHeight = Math.max(2, Number(quality?.height || 1080) || 1080);
    const targetHint = contentHint === "detail" ? "detail" : "motion";
    const constraintKey = `${targetWidth}x${targetHeight}@${targetFrameRate}:${targetHint}`;
    const now = Date.now();
    const previousState = screenShareConstraintState.get(videoTrack) || null;
    if (
      previousState &&
      previousState.key === constraintKey &&
      now - Number(previousState.at || 0) < 1200
    ) {
      return true;
    }

    videoTrack.contentHint = targetHint;
    const constraints = {
      ...videoTrack.getConstraints(),
      frameRate: {
        min: Math.max(1, Math.min(30, targetFrameRate)),
        ideal: targetFrameRate,
        max: targetFrameRate,
      },
      width: { min: 640, ideal: targetWidth, max: targetWidth },
      height: { min: 480, ideal: targetHeight, max: targetHeight },
      advanced: [
        {
          width: targetWidth,
          height: targetHeight,
          frameRate: targetFrameRate,
        },
      ],
      resizeMode: "none",
    };

    try {
      await videoTrack.applyConstraints(constraints);
      screenShareConstraintState.set(videoTrack, { key: constraintKey, at: now });
      return true;
    } catch (error) {
      console.warn("[Equirust] Failed to apply display-media constraints", error);
      screenShareConstraintState.set(videoTrack, { key: `error:${constraintKey}`, at: now });
      return false;
    }
  };

  const normalizeScreenShareQuality = quality => {
    const frameRate = Math.max(1, Math.round(Number(quality?.frameRate || 60) || 60));
    const width = Math.max(2, Math.round(Number(quality?.width || 1920) || 1920));
    const height = Math.max(2, Math.round(Number(quality?.height || 1080) || 1080));
    return { frameRate, width, height };
  };

  const normalizeDesktopStreamSinkDescriptor = descriptor => {
    const raw = descriptor && typeof descriptor === "object" ? descriptor : {};
    const fallbackSpecified = Object.prototype.hasOwnProperty.call(
      raw,
      "fallbackVideoIngress"
    );
    return {
      sinkKind:
        typeof raw.sinkKind === "string" && raw.sinkKind.trim()
          ? raw.sinkKind.trim()
          : "browserMediaStream",
      transportKind:
        typeof raw.transportKind === "string" && raw.transportKind.trim()
          ? raw.transportKind.trim()
          : "websocketBinary",
      preferredVideoIngress:
        typeof raw.preferredVideoIngress === "string" && raw.preferredVideoIngress.trim()
          ? raw.preferredVideoIngress.trim()
          : "generatedTrack",
      fallbackVideoIngress:
        typeof raw.fallbackVideoIngress === "string" && raw.fallbackVideoIngress.trim()
          ? raw.fallbackVideoIngress.trim()
          : fallbackSpecified
          ? null
          : "canvasCapture",
      preferredAudioIngress:
        typeof raw.preferredAudioIngress === "string" && raw.preferredAudioIngress.trim()
          ? raw.preferredAudioIngress.trim()
          : "mediaStreamDestination",
      browserOwnedPeerConnection: raw.browserOwnedPeerConnection !== false,
      browserOwnedEncoder: raw.browserOwnedEncoder !== false,
    };
  };

  const desktopStreamSinkUsesGeneratedTrack = descriptor =>
    descriptor?.sinkKind === "browserMediaStream" &&
    descriptor?.preferredVideoIngress === "generatedTrack";

  const desktopStreamSinkAllowsCanvasFallback = descriptor =>
    descriptor?.sinkKind === "browserMediaStream" &&
    descriptor?.fallbackVideoIngress === "canvasCapture";

  const computeGoLiveBitrateProfile = quality => {
    const normalized = normalizeScreenShareQuality(quality);
    const pixelRate = normalized.width * normalized.height * normalized.frameRate;
    let bitrateMin = 1_000_000;
    let bitrateTarget = 6_000_000;
    let bitrateMax = 12_000_000;
    if (pixelRate >= 1920 * 1080 * 60) {
      bitrateMin = 2_500_000;
      bitrateTarget = 8_000_000;
      bitrateMax = 16_000_000;
    }
    if (pixelRate >= 2560 * 1440 * 60) {
      bitrateMin = 4_500_000;
      bitrateTarget = 12_000_000;
      bitrateMax = 22_000_000;
    }
    if (pixelRate >= 3840 * 2160 * 60) {
      bitrateMin = 8_000_000;
      bitrateTarget = 18_000_000;
      bitrateMax = 32_000_000;
    }
    return {
      ...normalized,
      bitrateMin,
      bitrateTarget,
      bitrateMax,
    };
  };

  const STREAM_QUALITY_GATE_KEYS = [
    "guildPremiumTier",
    "requiredGuildPremiumTier",
    "premiumTier",
    "requiredPremiumTier",
    "minPremiumTier",
    "maxPremiumTier",
    "minimumGuildPremiumTier",
    "minimumPremiumTier",
  ];

  const looksLikeQualityKey = key =>
    /(quality|stream|video|go.?live|broadcast|target|meta|state|settings|resolution|fps|frame|size|encoding|preset|option)/i.test(
      String(key || "")
    );

  const looksLikeQualityNode = entry => {
    if (!entry || typeof entry !== "object") return false;
    const keys = Object.keys(entry);
    if (!keys.length) return false;
    return keys.some(key =>
      /(fps|frame|resolution|width|height|maxfr|maxframe|maxresolution|degradation|bitrate|encoding|quality|stream|broadcast|option|premium)/i.test(
        key
      )
    );
  };

  const looksLikeQualityOptionNode = entry => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) return false;
    const keys = Object.keys(entry);
    if (!keys.length) return false;
    if (keys.some(key => STREAM_QUALITY_GATE_KEYS.includes(String(key)))) return true;
    if (
      ("value" in entry || "label" in entry || "name" in entry) &&
      keys.some(key =>
        /(fps|frame|resolution|quality|bitrate|premium|tier|locked|disabled|stream|broadcast)/i.test(
          key
        )
      )
    ) {
      return true;
    }
    return false;
  };

  const applyGoLiveQualityObjectPatch = (entry, quality) => {
    if (!entry || typeof entry !== "object") return 0;
    const profile = computeGoLiveBitrateProfile(quality);
    let touched = 0;
    const setValue = (key, value) => {
      if (!(key in entry)) return;
      try {
        if (entry[key] !== value) {
          entry[key] = value;
          touched += 1;
        }
      } catch {}
    };

    setValue("frameRate", profile.frameRate);
    setValue("framerate", profile.frameRate);
    setValue("fps", profile.frameRate);
    setValue("maxFrameRate", profile.frameRate);
    setValue("maxFramerate", profile.frameRate);
    setValue("width", profile.width);
    setValue("height", profile.height);
    setValue("pixelCount", profile.width * profile.height);
    setValue("bitrateMin", profile.bitrateMin);
    setValue("bitrateTarget", profile.bitrateTarget);
    setValue("bitrateMax", profile.bitrateMax);
    setValue("minBitrate", profile.bitrateMin);
    setValue("targetBitrate", profile.bitrateTarget);
    setValue("startBitrate", profile.bitrateTarget);
    setValue("maxBitrate", profile.bitrateMax);

    if ("resolution" in entry) {
      try {
        if (entry.resolution && typeof entry.resolution === "object") {
          if (entry.resolution.width !== profile.width) {
            entry.resolution.width = profile.width;
            touched += 1;
          }
          if (entry.resolution.height !== profile.height) {
            entry.resolution.height = profile.height;
            touched += 1;
          }
        } else if (entry.resolution !== profile.height) {
          entry.resolution = profile.height;
          touched += 1;
        }
      } catch {}
    }

    if ("maxResolution" in entry) {
      try {
        if (!entry.maxResolution || typeof entry.maxResolution !== "object") {
          entry.maxResolution = { width: profile.width, height: profile.height };
          touched += 1;
        } else {
          if (entry.maxResolution.width !== profile.width) {
            entry.maxResolution.width = profile.width;
            touched += 1;
          }
          if (entry.maxResolution.height !== profile.height) {
            entry.maxResolution.height = profile.height;
            touched += 1;
          }
        }
      } catch {}
    }

    return touched;
  };

  const patchGoLiveQualityStateTree = (root, quality, options = {}) => {
    if (!root || typeof root !== "object") return 0;
    const seen = new WeakSet();
    const maxDepth = Math.max(1, Math.round(Number(options.maxDepth || 5) || 5));
    let budget = Math.max(24, Math.round(Number(options.maxNodes || 220) || 220));
    let hits = 0;
    const walk = (value, depth, parentKey = "") => {
      if (!value || typeof value !== "object") return;
      if (seen.has(value) || budget <= 0 || depth > maxDepth) return;
      seen.add(value);
      budget -= 1;

      const shouldPatch =
        looksLikeQualityKey(parentKey) ||
        looksLikeQualityNode(value);
      if (shouldPatch) {
        hits += applyGoLiveQualityObjectPatch(value, quality);
      }

      if (Array.isArray(value)) {
        for (const item of value) {
          walk(item, depth + 1, parentKey);
        }
        return;
      }

      for (const [key, nested] of Object.entries(value)) {
        if (!nested || typeof nested !== "object") continue;
        if (depth >= 2 && !looksLikeQualityKey(key) && !looksLikeQualityNode(nested)) continue;
        walk(nested, depth + 1, key);
      }
    };
    walk(root, 0, "");
    return hits;
  };

  const patchGoLiveOptionAvailabilityTree = (root, options = {}) => {
    if (!root || typeof root !== "object") return 0;
    const seen = new WeakSet();
    const maxDepth = Math.max(1, Math.round(Number(options.maxDepth || 5) || 5));
    let budget = Math.max(24, Math.round(Number(options.maxNodes || 220) || 220));
    let hits = 0;
    const walk = (value, depth, parentKey = "") => {
      if (!value || typeof value !== "object") return;
      if (seen.has(value) || budget <= 0 || depth > maxDepth) return;
      seen.add(value);
      budget -= 1;

      const optionNode =
        looksLikeQualityOptionNode(value) ||
        (looksLikeQualityKey(parentKey) && looksLikeQualityNode(value));
      if (optionNode && !Array.isArray(value)) {
        for (const gateKey of STREAM_QUALITY_GATE_KEYS) {
          if (!(gateKey in value)) continue;
          try {
            delete value[gateKey];
            hits += 1;
          } catch {
            try {
              value[gateKey] = undefined;
              hits += 1;
            } catch {}
          }
        }
        const boolOverrides = [
          ["disabled", false],
          ["isDisabled", false],
          ["hidden", false],
          ["isHidden", false],
          ["locked", false],
          ["isLocked", false],
          ["available", true],
          ["isAvailable", true],
          ["canUse", true],
          ["canAccess", true],
          ["enabled", true],
        ];
        for (const [key, nextValue] of boolOverrides) {
          if (!(key in value)) continue;
          try {
            if (value[key] !== nextValue) {
              value[key] = nextValue;
              hits += 1;
            }
          } catch {}
        }
      }

      if (Array.isArray(value)) {
        for (const item of value) {
          walk(item, depth + 1, parentKey);
        }
        return;
      }

      for (const [key, nested] of Object.entries(value)) {
        if (!nested || typeof nested !== "object") continue;
        if (
          depth >= 2 &&
          !looksLikeQualityKey(key) &&
          !looksLikeQualityOptionNode(nested) &&
          !looksLikeQualityNode(nested)
        ) {
          continue;
        }
        walk(nested, depth + 1, key);
      }
    };
    walk(root, 0, "");
    return hits;
  };

  const patchGoLiveQualityPayload = (payload, quality) => {
    const profile = computeGoLiveBitrateProfile(quality);
    const next =
      payload && typeof payload === "object"
        ? Array.isArray(payload)
          ? [...payload]
          : { ...payload }
        : {};
    const patchBranch = branch => {
      const branchNext =
        branch && typeof branch === "object" ? { ...branch } : {};
      branchNext.framerate = profile.frameRate;
      branchNext.frameRate = profile.frameRate;
      branchNext.width = profile.width;
      branchNext.height = profile.height;
      branchNext.pixelCount = profile.width * profile.height;
      return branchNext;
    };
    next.bitrateMin = profile.bitrateMin;
    next.bitrateTarget = profile.bitrateTarget;
    next.bitrateMax = profile.bitrateMax;
    next.capture = patchBranch(next.capture);
    next.encode = patchBranch(next.encode);
    patchGoLiveQualityStateTree(next, profile);
    if (isFakeNitroStreamQualityBypassEnabled()) {
      patchGoLiveOptionAvailabilityTree(next);
    }
    return next;
  };

  window.__EQUIRUST_PATCH_STREAM_QUALITY__ = payload => {
    const pendingQuality =
      (state.pendingScreenShareQuality && typeof state.pendingScreenShareQuality === "object"
        ? state.pendingScreenShareQuality
        : window.__EQUIRUST_PENDING_NATIVE_SHARE_QUALITY__) || readScreenShareQuality();
    return patchGoLiveQualityPayload(payload, pendingQuality);
  };

  const isFakeNitroStreamQualityBypassEnabled = () => {
    try {
      const pluginSettings = window.Vencord?.Settings?.plugins?.FakeNitro;
      if (!pluginSettings || typeof pluginSettings !== "object") return false;
      if (pluginSettings.enabled === false) return false;
      return pluginSettings.enableStreamQualityBypass !== false;
    } catch {
      return false;
    }
  };

  let fakeNitroStreamBypassInstalled = false;
  let fakeNitroStreamBypassInstalling = false;
  const installNitroStreamQualityBypassPatch = () => {
    if (fakeNitroStreamBypassInstalled || !isDiscordHost()) return;
    if (fakeNitroStreamBypassInstalling) return;
    fakeNitroStreamBypassInstalling = true;

    const tryInstall = () => {
      const addPatch = window.Vencord?.Plugins?.addPatch;
      const webpack = window.Vencord?.Webpack;
      if (typeof addPatch !== "function" || !webpack || typeof webpack !== "object") {
        return false;
      }

      let webpackPatch = false;
      const wrappedMethods = [];
      const optionTreePatches = [];
      const seenOwners = new WeakSet();
      const methodNames = ["canUseHighVideoUploadQuality", "canStreamQuality"];
      const patchOptionTree = (root, label) => {
        if (!root || (typeof root !== "object" && typeof root !== "function")) return 0;
        let hits = 0;
        try {
          hits += patchGoLiveOptionAvailabilityTree(root);
        } catch {}
        try {
          hits += patchGoLiveQualityStateTree(
            root,
            (state.pendingScreenShareQuality && typeof state.pendingScreenShareQuality === "object"
              ? state.pendingScreenShareQuality
              : window.__EQUIRUST_PENDING_NATIVE_SHARE_QUALITY__) || readScreenShareQuality()
          );
        } catch {}
        if (hits > 0) {
          optionTreePatches.push({ label, hits });
        }
        return hits;
      };
      const wrapOwnerMethod = (owner, label) => {
        if (!owner || (typeof owner !== "object" && typeof owner !== "function")) return;
        if (seenOwners.has(owner)) return;
        let wrapped = false;
        for (const methodName of methodNames) {
          const patchMarker = `__EQUIRUST_NITRO_STREAM_PATCHED_${methodName}__`;
          let currentMethod;
          try {
            currentMethod = owner[methodName];
          } catch {
            continue;
          }
          if (typeof currentMethod !== "function") continue;
          if (owner[patchMarker] === true) {
            wrappedMethods.push(`${label}.${methodName}`);
            wrapped = true;
            continue;
          }
          const originalMethod = currentMethod;
          try {
            owner[methodName] = function (...args) {
              if (isFakeNitroStreamQualityBypassEnabled()) {
                return true;
              }
              return originalMethod.apply(this, args);
            };
          } catch {
            continue;
          }
          try {
            Object.defineProperty(owner, patchMarker, {
              value: true,
              configurable: false,
              enumerable: false,
              writable: false,
            });
          } catch {}
          wrappedMethods.push(`${label}.${methodName}`);
          wrapped = true;
        }
        if (wrapped) {
          seenOwners.add(owner);
        }
      };

      try {
        addPatch(
          {
            find: "canUseCustomStickersEverywhere:",
            replacement: [
              {
                match: /(?<=canUseHighVideoUploadQuality:)\i/,
                replace: "() => window.Vencord?.Settings?.plugins?.FakeNitro?.enabled !== false && window.Vencord?.Settings?.plugins?.FakeNitro?.enableStreamQualityBypass !== false ? true : $&()",
              },
              {
                match: /(?<=canStreamQuality:)\i/,
                replace: "() => window.Vencord?.Settings?.plugins?.FakeNitro?.enabled !== false && window.Vencord?.Settings?.plugins?.FakeNitro?.enableStreamQualityBypass !== false ? true : $&()",
              },
            ],
          },
          "EquirustNativeFakeNitroStreamBypass",
          "window"
        );
        addPatch(
          {
            find: "#{intl::STREAM_FPS_OPTION}",
            replacement: {
              match: /guildPremiumTier:\i\.\i\.TIER_\d,?/g,
              replace: "",
            },
          },
          "EquirustNativeFakeNitroStreamOptions",
          "window"
        );
        webpackPatch = true;
      } catch (error) {
        report(
          `desktop_stream_nitro_stream_webpack_patch_failed=${
            error && error.message ? error.message : String(error)
          }`,
          { force: true }
        );
      }

      try {
        const common = window.Vencord?.Webpack?.Common;
        if (common && typeof common === "object") {
          for (const [key, value] of Object.entries(common)) {
            patchOptionTree(value, `Common.${key}`);
            patchOptionTree(value?.default, `Common.${key}.default`);
            wrapOwnerMethod(value, `Common.${key}`);
            wrapOwnerMethod(value?.default, `Common.${key}.default`);
          }
        }

        const findAll = window.Vencord?.Webpack?.findAll;
        if (typeof findAll === "function") {
          const discovered = findAll(moduleExport => {
            if (!moduleExport) return false;
            if (methodNames.some(methodName => {
              try {
                if (typeof moduleExport[methodName] === "function") return true;
              } catch {}
              try {
                return typeof moduleExport?.default?.[methodName] === "function";
              } catch {
                return false;
              }
            })) {
              return true;
            }
            try {
              return (
                patchGoLiveOptionAvailabilityTree(moduleExport, { maxDepth: 4, maxNodes: 120 }) > 0 ||
                patchGoLiveOptionAvailabilityTree(moduleExport?.default, { maxDepth: 4, maxNodes: 120 }) > 0
              );
            } catch {
              return false;
            }
          });
          if (Array.isArray(discovered)) {
            discovered.slice(0, 32).forEach((moduleExport, index) => {
              patchOptionTree(moduleExport, `Webpack.${index}`);
              patchOptionTree(moduleExport?.default, `Webpack.${index}.default`);
              wrapOwnerMethod(moduleExport, `Webpack.${index}`);
              wrapOwnerMethod(moduleExport?.default, `Webpack.${index}.default`);
            });
          }
        }
      } catch (error) {
        report(
          `desktop_stream_nitro_stream_method_patch_failed=${
            error && error.message ? error.message : String(error)
          }`,
          { force: true }
        );
      }

      if (!webpackPatch && wrappedMethods.length === 0 && optionTreePatches.length === 0) {
        return false;
      }

      fakeNitroStreamBypassInstalled = true;
      report(
        "desktop_stream_nitro_stream_patch_installed=" +
          JSON.stringify({
            webpackPatch,
            methodWrapCount: wrappedMethods.length,
            methodWraps: wrappedMethods.slice(0, 16),
            optionTreePatchCount: optionTreePatches.length,
            optionTreePatches: optionTreePatches.slice(0, 16),
          })
      );
      return true;
    };

    if (tryInstall()) {
      fakeNitroStreamBypassInstalling = false;
      return;
    }

    let attempts = 0;
    const retryTimer = window.setInterval(() => {
      attempts += 1;
      if (tryInstall() || attempts >= 120) {
        window.clearInterval(retryTimer);
        if (!fakeNitroStreamBypassInstalled) {
          report("desktop_stream_nitro_stream_patch_timeout=true", { force: true });
        }
        fakeNitroStreamBypassInstalling = false;
      }
    }, 250);
  };

  const primeGoLiveQualityForCurrentConnections = quality => {
    try {
      const common = window.Vencord?.Webpack?.Common;
      const mediaEngine = common?.MediaEngineStore?.getMediaEngine?.();
      const currentUserId = common?.UserStore?.getCurrentUser?.()?.id;
      if (!mediaEngine || !currentUserId) return false;
      const normalized = normalizeScreenShareQuality(quality);
      const connections = Array.isArray(mediaEngine?.connections)
        ? mediaEngine.connections
        : mediaEngine?.connections
          ? Array.from(mediaEngine.connections)
          : [];
      let patched = 0;
      for (const connection of connections) {
        if (!connection || connection.streamUserId !== currentUserId) continue;
        const videoParams = Array.isArray(connection.videoStreamParameters)
          ? connection.videoStreamParameters[0]
          : connection.videoStreamParameters;
        if (videoParams && typeof videoParams === "object") {
          videoParams.maxFrameRate = normalized.frameRate;
          videoParams.maxFramerate = normalized.frameRate;
          videoParams.frameRate = normalized.frameRate;
          videoParams.framerate = normalized.frameRate;
          videoParams.maxResolution = videoParams.maxResolution && typeof videoParams.maxResolution === "object"
            ? videoParams.maxResolution
            : { width: normalized.width, height: normalized.height };
          if (videoParams.maxResolution && typeof videoParams.maxResolution === "object") {
            videoParams.maxResolution.width = normalized.width;
            videoParams.maxResolution.height = normalized.height;
          }
          patched += 1;
        }
      }
      if (patched > 0) {
        report(
          "desktop_stream_golive_prime=" +
            JSON.stringify({
              frameRate: normalized.frameRate,
              width: normalized.width,
              height: normalized.height,
              patchedConnections: patched,
            })
        );
      }
      return patched > 0;
    } catch (error) {
      console.warn("[Equirust] Failed to prime Go Live quality", error);
      return false;
    }
  };

  const applyScreenShareConnectionQuality = (stream, quality) => {
    try {
      const common = window.Vencord?.Webpack?.Common;
      const mediaEngine = common?.MediaEngineStore?.getMediaEngine?.();
      const currentUserId = common?.UserStore?.getCurrentUser?.()?.id;
      const connections = Array.isArray(mediaEngine?.connections)
        ? mediaEngine.connections
        : mediaEngine?.connections
          ? Array.from(mediaEngine.connections)
          : [];

      const connection = connections.find(entry => {
        if (!entry || entry.streamUserId !== currentUserId) return false;
        const inputStream = entry?.input?.stream;
        return inputStream === stream || inputStream?.id === stream?.id;
      });
      const goLiveCandidates = connections.filter(entry => {
        if (!entry || entry.streamUserId !== currentUserId) return false;
        if (entry === connection) return true;
        if (entry?.goLiveSource) return true;
        if (entry?.isGoLive === true) return true;
        const streamType = String(entry?.streamType || entry?.type || "").toLowerCase();
        if (streamType.includes("screen") || streamType.includes("golive")) return true;
        return Boolean(entry?.videoStreamParameters?.length || entry?.videoStreamParameters);
      });
      const selectedConnections = goLiveCandidates.length ? goLiveCandidates : connection ? [connection] : [];
      if (!selectedConnections.length) return false;

      const targetFrameRate = Math.max(1, Math.round(Number(quality?.frameRate || 60) || 60));
      const targetWidth = Math.max(2, Math.round(Number(quality?.width || 1920) || 1920));
      const targetHeight = Math.max(2, Math.round(Number(quality?.height || 1080) || 1080));
      const targetKey = `${targetWidth}x${targetHeight}@${targetFrameRate}`;
      const now = Date.now();
      const bitrateProfile = computeGoLiveBitrateProfile({
        frameRate: targetFrameRate,
        width: targetWidth,
        height: targetHeight,
      });
      const looksLikeQualityKey = key =>
        /(quality|stream|video|go.?live|broadcast|target|meta|state|settings|resolution|fps|frame|size|encoding|preset)/i.test(
          String(key || "")
        );
      const looksLikeQualityNode = entry => {
        if (!entry || typeof entry !== "object") return false;
        const keys = Object.keys(entry);
        if (!keys.length) return false;
        return keys.some(key =>
          /(fps|frame|resolution|width|height|maxfr|maxframe|maxresolution|degradation|bitrate|encoding)/i.test(
            key
          )
        );
      };
      const patchQualityObject = entry => {
        if (!entry || typeof entry !== "object") return;
        entry.frameRate = targetFrameRate;
        entry.framerate = targetFrameRate;
        entry.fps = targetFrameRate;
        entry.maxFrameRate = targetFrameRate;
        entry.maxFramerate = targetFrameRate;
        entry.width = targetWidth;
        entry.height = targetHeight;
        entry.resolution = targetHeight;
        entry.bitrateMin = bitrateProfile.bitrateMin;
        entry.bitrateTarget = bitrateProfile.bitrateTarget;
        entry.bitrateMax = bitrateProfile.bitrateMax;
        entry.minBitrate = bitrateProfile.bitrateMin;
        entry.targetBitrate = bitrateProfile.bitrateTarget;
        entry.startBitrate = bitrateProfile.bitrateTarget;
        entry.maxBitrate = bitrateProfile.bitrateMax;
        entry.maxResolution = entry.maxResolution && typeof entry.maxResolution === "object"
          ? entry.maxResolution
          : { width: targetWidth, height: targetHeight };
        if (entry.maxResolution && typeof entry.maxResolution === "object") {
          entry.maxResolution.width = targetWidth;
          entry.maxResolution.height = targetHeight;
        }
        if (entry.resolution && typeof entry.resolution === "object") {
          entry.resolution.width = targetWidth;
          entry.resolution.height = targetHeight;
        }
      };
      const patchQualityTree = root => {
        const seen = new WeakSet();
        let budget = 180;
        let hits = 0;
        const walk = (value, depth, parentKey = "") => {
          if (!value || typeof value !== "object") return;
          if (seen.has(value) || budget <= 0 || depth > 4) return;
          seen.add(value);
          budget -= 1;

          const isQualityNode =
            looksLikeQualityNode(value) ||
            looksLikeQualityKey(parentKey);
          if (isQualityNode) {
            patchQualityObject(value);
            hits += 1;
          }

          if (Array.isArray(value)) {
            for (const item of value) {
              walk(item, depth + 1, parentKey);
            }
            return;
          }

          for (const [key, nested] of Object.entries(value)) {
            if (!nested || typeof nested !== "object") continue;
            if (depth >= 2 && !looksLikeQualityKey(key) && !looksLikeQualityNode(nested)) continue;
            walk(nested, depth + 1, key);
          }
        };
        walk(root, 0, "");
        return hits;
      };
      const patchParameterShape = entry => {
        if (!entry || typeof entry !== "object") return;
        entry.maxFrameRate = targetFrameRate;
        entry.maxFramerate = targetFrameRate;
        entry.frameRate = targetFrameRate;
        entry.framerate = targetFrameRate;
        entry.bitrateMin = bitrateProfile.bitrateMin;
        entry.bitrateTarget = bitrateProfile.bitrateTarget;
        entry.bitrateMax = bitrateProfile.bitrateMax;
        entry.minBitrate = bitrateProfile.bitrateMin;
        entry.targetBitrate = bitrateProfile.bitrateTarget;
        entry.startBitrate = bitrateProfile.bitrateTarget;
        entry.maxBitrate = bitrateProfile.bitrateMax;
        if (entry.maxResolution && typeof entry.maxResolution === "object") {
          entry.maxResolution.width = targetWidth;
          entry.maxResolution.height = targetHeight;
        }
        if (entry.resolution && typeof entry.resolution === "object") {
          entry.resolution.width = targetWidth;
          entry.resolution.height = targetHeight;
        }
      };
      let patchedCount = 0;
      let goLiveFieldHits = 0;
      let qualityFieldHits = 0;
      let videoQualityFieldHits = 0;
      let recursiveQualityHits = 0;
      for (const selectedConnection of selectedConnections) {
        if (!selectedConnection || typeof selectedConnection !== "object") continue;
        patchQualityObject(selectedConnection);
        const previousPatch = desktopStreamQualityPatchMemo.get(selectedConnection) || null;
        const shouldRefreshRecursivePatch =
          !previousPatch ||
          previousPatch.key !== targetKey ||
          now - Number(previousPatch.at || 0) >= 1400;
        if (shouldRefreshRecursivePatch) {
          recursiveQualityHits += patchQualityTree(selectedConnection);
          desktopStreamQualityPatchMemo.set(selectedConnection, {
            key: targetKey,
            at: now,
          });
        }
        const hasVideoParams = Boolean(
          selectedConnection?.videoStreamParameters?.length ||
            selectedConnection?.videoStreamParameters
        );
        if (hasVideoParams) {
          if (Array.isArray(selectedConnection.videoStreamParameters)) {
            selectedConnection.videoStreamParameters.forEach(patchParameterShape);
          } else {
            patchParameterShape(selectedConnection.videoStreamParameters);
          }
        }
        if (selectedConnection.goLiveSource) {
          goLiveFieldHits += 1;
          if (!selectedConnection.goLiveSource.quality) {
            selectedConnection.goLiveSource.quality = {};
          }
          patchQualityObject(selectedConnection.goLiveSource.quality);
          patchQualityObject(selectedConnection.goLiveSource);
        }
        if (selectedConnection.quality && typeof selectedConnection.quality === "object") {
          qualityFieldHits += 1;
          patchQualityObject(selectedConnection.quality);
        }
        if (selectedConnection.videoQuality && typeof selectedConnection.videoQuality === "object") {
          videoQualityFieldHits += 1;
          patchQualityObject(selectedConnection.videoQuality);
        }
        if (selectedConnection.streamQuality && typeof selectedConnection.streamQuality === "object") {
          patchQualityObject(selectedConnection.streamQuality);
        }
        if (selectedConnection.outputQuality && typeof selectedConnection.outputQuality === "object") {
          patchQualityObject(selectedConnection.outputQuality);
        }
        if (selectedConnection.targetQuality && typeof selectedConnection.targetQuality === "object") {
          patchQualityObject(selectedConnection.targetQuality);
        }
        if (selectedConnection.maxQuality && typeof selectedConnection.maxQuality === "object") {
          patchQualityObject(selectedConnection.maxQuality);
        }
        const genericQualityKeys = [
          "streamOptions",
          "streamSettings",
          "broadcastSettings",
          "qualitySettings",
          "streamState",
          "videoState",
          "target",
        ];
        for (const key of genericQualityKeys) {
          if (selectedConnection[key] && typeof selectedConnection[key] === "object") {
            patchQualityObject(selectedConnection[key]);
          }
        }
        patchedCount += 1;
      }
      const interestingFieldHits = goLiveFieldHits + qualityFieldHits + videoQualityFieldHits;
      if (interestingFieldHits > 0 || recursiveQualityHits > 0) {
        report(
          "desktop_stream_connection_quality_patch=" +
            JSON.stringify({
              streamId: stream?.id || null,
              matched: connection ? "exact" : selectedConnections.length ? "fallback" : "none",
              candidateCount: selectedConnections.length,
              patchedCount,
              goLiveFieldHits,
              qualityFieldHits,
              videoQualityFieldHits,
              recursiveQualityHits,
              targetFrameRate,
              targetWidth,
              targetHeight,
            })
        );
      } else {
        const streamId = String(stream?.id || "");
        if (streamId && !desktopStreamQualityProbeLogged.has(streamId)) {
          desktopStreamQualityProbeLogged.add(streamId);
          const first = selectedConnections[0];
          report(
            "desktop_stream_connection_quality_probe=" +
              JSON.stringify({
                streamId,
                connectionKeys: first && typeof first === "object" ? Object.keys(first).slice(0, 80) : [],
                inputKeys:
                  first?.input && typeof first.input === "object"
                    ? Object.keys(first.input).slice(0, 40)
                    : [],
                videoParamKeys:
                  first?.videoStreamParameters && typeof first.videoStreamParameters === "object"
                    ? Array.isArray(first.videoStreamParameters)
                      ? Object.keys(first.videoStreamParameters[0] || {}).slice(0, 40)
                      : Object.keys(first.videoStreamParameters).slice(0, 40)
                    : [],
              })
          );
        }
      }

      return patchedCount > 0;
    } catch (error) {
      console.warn("[Equirust] Failed to patch stream connection quality", error);
      return false;
    }
  };

  const desktopStreamReinforceControllers = new WeakMap();

  const stopScreenShareReinforcement = stream => {
    if (!stream || (typeof stream !== "object" && typeof stream !== "function")) return;
    const controller = desktopStreamReinforceControllers.get(stream);
    if (!controller || typeof controller.stop !== "function") return;
    try {
      controller.stop("manual_stop");
    } catch {}
    desktopStreamReinforceControllers.delete(stream);
  };

  const reinforceScreenShareQuality = (stream, quality, contentHint) => {
    stopScreenShareReinforcement(stream);
    let attempts = 0;
    let readyHits = 0;
    let sawConnectionReady = false;
    const videoTrack = stream?.getVideoTracks?.()?.[0];
    let stopped = false;
    let timer = null;
    const stop = reason => {
      if (stopped) return;
      stopped = true;
      if (timer) {
        window.clearInterval(timer);
        timer = null;
      }
      try {
        videoTrack?.removeEventListener?.("ended", onTrackEnded);
      } catch {}
      try {
        stream?.removeEventListener?.("inactive", onStreamInactive);
      } catch {}
      desktopStreamReinforceControllers.delete(stream);
      if (reason) {
        report(
          "desktop_stream_reinforce_stop=" +
            JSON.stringify({
              streamId: stream?.id || null,
              reason,
              attempts,
              readyHits,
            })
        );
      }
    };
    const onTrackEnded = () => stop("video_track_ended");
    const onStreamInactive = () => stop("stream_inactive");
    try {
      videoTrack?.addEventListener?.("ended", onTrackEnded);
    } catch {}
    try {
      stream?.addEventListener?.("inactive", onStreamInactive);
    } catch {}
    timer = window.setInterval(() => {
      if (stopped) {
        stop();
        return;
      }
      if (!videoTrack || videoTrack.readyState === "ended") {
        stop("video_track_not_live");
        return;
      }
      const liveTracks = stream?.getTracks?.()?.filter?.(track => track?.readyState === "live") || [];
      if (!liveTracks.length) {
        stop("stream_has_no_live_tracks");
        return;
      }
      attempts += 1;
      const shouldPatchConnection =
        !sawConnectionReady || attempts <= 6 || attempts % 8 === 0;
      let connectionReady = sawConnectionReady;
      if (shouldPatchConnection) {
        connectionReady = applyScreenShareConnectionQuality(stream, quality);
        if (connectionReady) {
          readyHits += 1;
          sawConnectionReady = true;
        }
      }
      if (attempts === 1 || attempts % 10 === 0 || (connectionReady && readyHits === 1)) {
        report(
          "desktop_stream_reinforce_tick=" +
            JSON.stringify({
              streamId: stream?.id || null,
              attempts,
              readyHits,
              connectionReady,
              targetFrameRate: Number(quality?.frameRate || 0) || null,
              targetWidth: Number(quality?.width || 0) || null,
              targetHeight: Number(quality?.height || 0) || null,
            })
        );
      }
      const shouldApplyTrackConstraints = Boolean(
        videoTrack &&
          (!sawConnectionReady || attempts <= 4 || attempts % 12 === 0 || readyHits <= 1)
      );
      if (shouldApplyTrackConstraints) {
        void applyScreenShareTrackConstraints(videoTrack, quality, contentHint);
      }
      if (attempts >= 24 || (attempts >= 12 && readyHits >= 4)) {
        stop("attempt_budget_exhausted");
      }
    }, 320);
    desktopStreamReinforceControllers.set(stream, { stop });
    return { stop };
  };

  const desktopStreamTrackMeta = new WeakMap();
  const desktopStreamStreamMeta = new Map();
  const desktopStreamStreamClonePatched = new WeakSet();
  const desktopStreamSenderPeerConnection = new WeakMap();
  const desktopStreamSenderPendingMeta = new WeakMap();
  const nativeAbrControllers = new WeakMap();
  const nativeAbrControllerRecords = new Set();
  const desktopStreamClosingTracks = new WeakSet();
  const desktopStreamClosingSessionIds = new Set();
  const desktopStreamSenderNegotiationHints = new WeakMap();
  const desktopStreamPeerConnectionNegotiationHints = new WeakMap();
  const desktopStreamQualityProbeLogged = new Set();
  const desktopStreamQualityPatchMemo = new WeakMap();
  const desktopStreamTrackSettingsPatched = new WeakSet();

  const isDesktopStreamDirectIngressMode = mode =>
    /(generator)/i.test(String(mode || ""));

  const rememberDesktopStreamClosingSession = sessionId => {
    const normalized = String(sessionId || "").trim();
    if (!normalized) return;
    desktopStreamClosingSessionIds.add(normalized);
    while (desktopStreamClosingSessionIds.size > 128) {
      const oldest = desktopStreamClosingSessionIds.values().next();
      if (oldest.done) break;
      desktopStreamClosingSessionIds.delete(oldest.value);
    }
  };

  const isDesktopStreamClosing = meta =>
    !!meta?.sessionId && desktopStreamClosingSessionIds.has(String(meta.sessionId));

  const stopNativeAbrControllersForSession = (sessionId, tracks = []) => {
    const normalizedSessionId = String(sessionId || "").trim();
    const trackSet = new Set(Array.isArray(tracks) ? tracks.filter(Boolean) : []);
    nativeAbrControllerRecords.forEach(record => {
      if (!record || typeof record.stop !== "function") {
        return;
      }
      const matchesSession =
        normalizedSessionId && String(record.sessionId || "") === normalizedSessionId;
      const matchesTrack =
        trackSet.size > 0 &&
        (trackSet.has(record.videoTrack) || trackSet.has(record.sender?.track));
      if (!matchesSession && !matchesTrack) {
        return;
      }
      try {
        record.stop("session_teardown");
      } catch {}
    });
  };

  const normalizeDesktopStreamMeta = meta => {
    if (!meta || typeof meta !== "object") return null;
    const videoIngressMode =
      typeof meta.videoIngressMode === "string" && meta.videoIngressMode.trim()
        ? meta.videoIngressMode.trim()
        : "unknown";
    return {
      sessionId: String(meta.sessionId || ""),
      sourceId: String(meta.sourceId || ""),
      sourceKind: String(meta.sourceKind || "").toLowerCase() === "screen" ? "screen" : "window",
      baseWidth: Math.max(2, Number(meta.baseWidth || 0) || 2),
      baseHeight: Math.max(2, Number(meta.baseHeight || 0) || 2),
      baseFrameRate: Math.max(1, Number(meta.baseFrameRate || 0) || 1),
      encoderMode: String(meta.encoderMode || ""),
      encoderDetail:
        typeof meta.encoderDetail === "string" && meta.encoderDetail.trim()
          ? meta.encoderDetail.trim()
          : "",
      contentHint: String(meta.contentHint || "").toLowerCase() === "detail" ? "detail" : "motion",
      videoIngressMode,
      directVideoIngress: isDesktopStreamDirectIngressMode(videoIngressMode),
      workerVideoIngress: /worker/i.test(videoIngressMode),
    };
  };

  const decorateDesktopStreamVideoTrack = (track, meta) => {
    if (!track || track.kind !== "video") return;
    if (desktopStreamTrackSettingsPatched.has(track)) return;
    desktopStreamTrackSettingsPatched.add(track);

    const readDefaults = () => {
      const currentMeta = normalizeDesktopStreamMeta(meta) || desktopStreamTrackMeta.get(track) || null;
      const baseWidth = Math.max(2, Number(currentMeta?.baseWidth || 1920) || 1920);
      const baseHeight = Math.max(2, Number(currentMeta?.baseHeight || 1080) || 1080);
      const baseFrameRate = Math.max(1, Number(currentMeta?.baseFrameRate || 60) || 60);
      const sourceKind = String(currentMeta?.sourceKind || "window").toLowerCase() === "screen"
        ? "screen"
        : "window";
      return {
        width: baseWidth,
        height: baseHeight,
        frameRate: baseFrameRate,
        displaySurface: sourceKind === "screen" ? "monitor" : "window",
        logicalSurface: sourceKind === "window",
      };
    };

    const withVirtualSettings = raw => {
      const defaults = readDefaults();
      const next = raw && typeof raw === "object" ? { ...raw } : {};
      next.width = defaults.width;
      next.height = defaults.height;
      next.frameRate = defaults.frameRate;
      next.displaySurface ??= defaults.displaySurface;
      next.logicalSurface ??= defaults.logicalSurface;
      next.cursor ??= "always";
      return next;
    };

    try {
      if (typeof track.getSettings === "function") {
        const originalGetSettings = track.getSettings.bind(track);
        track.getSettings = () => withVirtualSettings(originalGetSettings());
      }
    } catch {}

    try {
      if (typeof track.getConstraints === "function") {
        const originalGetConstraints = track.getConstraints.bind(track);
        track.getConstraints = () => {
          const defaults = readDefaults();
          const existing = originalGetConstraints() || {};
          return {
            ...existing,
            width: existing.width || { ideal: defaults.width, max: defaults.width },
            height: existing.height || { ideal: defaults.height, max: defaults.height },
            frameRate: existing.frameRate || { ideal: defaults.frameRate, max: defaults.frameRate },
            displaySurface: existing.displaySurface || defaults.displaySurface,
            logicalSurface:
              typeof existing.logicalSurface === "boolean"
                ? existing.logicalSurface
                : defaults.logicalSurface,
          };
        };
      }
    } catch {}

    try {
      if (typeof track.getCapabilities === "function") {
        const originalGetCapabilities = track.getCapabilities.bind(track);
        track.getCapabilities = () => {
          const defaults = readDefaults();
          const existing = originalGetCapabilities() || {};
          const next = { ...existing };
          const capsFrameRate =
            next.frameRate && typeof next.frameRate === "object" ? { ...next.frameRate } : {};
          capsFrameRate.min = Math.max(1, Number(capsFrameRate.min || 1) || 1);
          capsFrameRate.max = Math.max(defaults.frameRate, Number(capsFrameRate.max || 0) || 0);
          next.frameRate = capsFrameRate;

          const capsWidth = next.width && typeof next.width === "object" ? { ...next.width } : {};
          capsWidth.min = Math.max(2, Number(capsWidth.min || 2) || 2);
          capsWidth.max = Math.max(defaults.width, Number(capsWidth.max || 0) || 0);
          next.width = capsWidth;

          const capsHeight =
            next.height && typeof next.height === "object" ? { ...next.height } : {};
          capsHeight.min = Math.max(2, Number(capsHeight.min || 2) || 2);
          capsHeight.max = Math.max(defaults.height, Number(capsHeight.max || 0) || 0);
          next.height = capsHeight;

          const displaySurfaces = Array.isArray(next.displaySurface)
            ? next.displaySurface
            : [];
          if (!displaySurfaces.includes(defaults.displaySurface)) {
            displaySurfaces.push(defaults.displaySurface);
          }
          next.displaySurface = displaySurfaces;
          next.cursor = Array.isArray(next.cursor) ? next.cursor : ["always", "motion", "never"];
          return next;
        };
      }
    } catch {}
  };

  const setDesktopStreamTrackMeta = (track, meta) => {
    if (!track || track.kind !== "video") return false;
    const normalized = normalizeDesktopStreamMeta(meta);
    if (!normalized) return false;
    desktopStreamTrackMeta.set(track, normalized);
    decorateDesktopStreamVideoTrack(track, normalized);
    return true;
  };

  const getDesktopStreamStreamMeta = stream => {
    if (!stream || typeof stream.id !== "string") return null;
    return desktopStreamStreamMeta.get(stream.id) || null;
  };

  const setDesktopStreamStreamMeta = (stream, meta) => {
    if (!stream || typeof stream !== "object") return null;
    const normalized = normalizeDesktopStreamMeta(meta);
    if (!normalized) return null;

    if (typeof stream.id === "string" && stream.id) {
      desktopStreamStreamMeta.set(stream.id, normalized);
    }

    try {
      stream
        .getVideoTracks?.()
        ?.forEach(track => {
          setDesktopStreamTrackMeta(track, normalized);
        });
    } catch {}

    if (
      typeof stream.clone === "function" &&
      !desktopStreamStreamClonePatched.has(stream)
    ) {
      try {
        const originalClone = stream.clone.bind(stream);
        stream.clone = function(...args) {
          const clonedStream = originalClone(...args);
          setDesktopStreamStreamMeta(clonedStream, normalized);
          return clonedStream;
        };
        desktopStreamStreamClonePatched.add(stream);
      } catch {}
    }

    return normalized;
  };

  const getDesktopStreamMetaFromStreams = streams => {
    if (!Array.isArray(streams) || streams.length === 0) return null;
    for (const stream of streams) {
      const meta = getDesktopStreamStreamMeta(stream);
      if (meta) return meta;
    }
    return null;
  };

  const resolveEffectiveNativeShareQuality = (stream, fallbackQuality) => {
    const fallback =
      fallbackQuality && typeof fallbackQuality === "object" ? fallbackQuality : {};
    const meta = getDesktopStreamMetaFromStreams([stream]) || null;
    const videoTrack = stream?.getVideoTracks?.()?.[0] || null;
    let settings = null;
    try {
      settings =
        videoTrack && typeof videoTrack.getSettings === "function"
          ? videoTrack.getSettings()
          : null;
    } catch {}

    const width = Math.max(
      2,
      Math.round(Number(settings?.width || meta?.baseWidth || fallback.width || 1920) || 1920)
    );
    const height = Math.max(
      2,
      Math.round(Number(settings?.height || meta?.baseHeight || fallback.height || 1080) || 1080)
    );
    const frameRate = Math.max(
      1,
      Math.round(
        Number(settings?.frameRate || meta?.baseFrameRate || fallback.frameRate || 60) || 60
      )
    );

    return {
      frameRate,
      width,
      height,
      resolutionMode:
        String(fallback?.resolutionMode || "").toLowerCase() === "source"
          ? "source"
          : String(height),
    };
  };

  const computeScreenShareTargetBitrate = (width, height, frameRate) => {
    const pixelsPerFrame = Math.max(1, Number(width || 0) * Number(height || 0));
    if (pixelsPerFrame <= 921600) {
      return frameRate >= 60 ? 3500000 : 2200000;
    }
    if (pixelsPerFrame <= 2073600) {
      return frameRate >= 60 ? 6500000 : 4000000;
    }
    if (pixelsPerFrame <= 3686400) {
      return frameRate >= 60 ? 10000000 : 6000000;
    }
    return frameRate >= 60 ? 14000000 : 8500000;
  };

    const buildNativeAbrLadder = meta => {
      const baseWidth = Math.max(2, Number(meta?.baseWidth || 1920) || 1920);
      const baseHeight = Math.max(2, Number(meta?.baseHeight || 1080) || 1080);
    const requestedFrameRate = Math.max(1, Number(meta?.requestedFrameRate || 60) || 60);
    const baseFrameRate = Math.max(
      1,
      Number(meta?.baseFrameRate || requestedFrameRate) || requestedFrameRate
    );
    const preferredFrameRate = Math.min(60, Math.max(requestedFrameRate, baseFrameRate));
    const sourceKind = String(meta?.sourceKind || "").toLowerCase();
    const encoderMode = String(meta?.encoderMode || "").toLowerCase();
    const hardwareEncoded = encoderMode.includes("hardware");
    const directVideoIngress = meta?.directVideoIngress === true;
    const resolutionCandidates = [baseHeight, 2160, 1440, 1080, 900, 720, 540, 480]
      .map(value => Math.max(2, Math.round(Number(value) || 0)))
      .filter(value => value <= baseHeight);
    const frameRateCandidates = [preferredFrameRate];
    if (preferredFrameRate > 30) {
      frameRateCandidates.push(30);
    }
    if (preferredFrameRate > 15) {
      frameRateCandidates.push(15);
    }
    const uniqueFrameRateCandidates = Array.from(
      new Set(
        frameRateCandidates
          .map(value => Math.max(1, Math.min(60, Math.round(Number(value) || 0))))
          .filter(Boolean)
      )
    );
    const byKey = new Map();

    const minResolutionBeforeFpsDrop = directVideoIngress
      ? sourceKind === "window"
        ? Math.min(baseHeight, 1080)
        : Math.min(baseHeight, 1440)
      : sourceKind === "window"
        ? 720
        : 1080;
    for (const frameRate of uniqueFrameRateCandidates) {
      for (const height of resolutionCandidates) {
        if (frameRate < preferredFrameRate && height > minResolutionBeforeFpsDrop) {
          continue;
        }
        if (!hardwareEncoded && frameRate >= 60 && height > 1440) {
          continue;
        }

        const key = `${height}x${frameRate}`;
        if (byKey.has(key)) continue;
        const scaleDownBy = Math.max(1, baseHeight / height);
        const width = Math.max(2, Math.round(baseWidth / scaleDownBy));
        const targetBitrate = computeScreenShareTargetBitrate(width, height, frameRate);
        byKey.set(key, {
          width,
          height,
          frameRate,
          scaleDownBy,
          targetBitrate,
        });
      }
    }

    const ladder = Array.from(byKey.values());

    if (!ladder.length) {
      ladder.push({
        width: baseWidth,
        height: baseHeight,
        frameRate: Math.min(60, baseFrameRate),
        scaleDownBy: 1,
        targetBitrate: computeScreenShareTargetBitrate(
          baseWidth,
          baseHeight,
          Math.min(60, baseFrameRate)
        ),
      });
    }

    return ladder;
  };

  const smoothAbrEma = (previous, next, alpha = 0.35) => {
    if (!Number.isFinite(next)) {
      return Number.isFinite(previous) ? previous : null;
    }
    if (!Number.isFinite(previous)) {
      return next;
    }
    const safeAlpha = Math.min(1, Math.max(0.01, Number(alpha || 0.35)));
    return previous + (next - previous) * safeAlpha;
  };

  const normalizeQualityLimitationDurations = durations => {
    if (!durations || typeof durations !== "object") {
      return null;
    }
    const read = key => {
      const value = Number(durations[key] ?? 0);
      return Number.isFinite(value) && value >= 0 ? value : 0;
    };
    return {
      none: read("none"),
      cpu: read("cpu"),
      bandwidth: read("bandwidth"),
      other: read("other"),
    };
  };

  const getNativeAbrProfile = meta => {
    const sourceKind = String(meta?.sourceKind || "").toLowerCase();
    const contentHint = String(meta?.contentHint || "").toLowerCase();
    const detailMode = sourceKind === "window" && contentHint === "detail";
    const directVideoIngress = meta?.directVideoIngress === true;
    const workerVideoIngress = meta?.workerVideoIngress === true;

    if (detailMode) {
      const profile = {
        name: "window_detail",
        tickIntervalMs: 500,
        emaAlphaBitrate: 0.30,
        emaAlphaFps: 0.34,
        emaAlphaLoss: 0.22,
        emaAlphaRtt: 0.22,
        emaAlphaDropRatio: 0.28,
        emaAlphaEncodeMs: 0.22,
        degradeLoss: 0.06,
        severeLoss: 0.12,
        degradeRttMs: 360,
        severeRttMs: 560,
        nearCapacityRatio: 0.99,
        bitrateUndershootRatio: 0.74,
        minHeadroomFactor: 1.45,
        upgradeLossMax: 0.01,
        upgradeRttMaxMs: 155,
        fpsDropThreshold: 0.86,
        encodeMsPerFrameCpuThreshold: 18,
        dropRatioThreshold: 0.15,
        degradeBaseStreak: 3,
        degradeFpsDropStrongStreak: 4,
        degradeFpsDropStreak: 6,
        upgradeStreak: 8,
        telemetryCapDetectStreak: 10,
        telemetryCapRecoverStreak: 8,
      };
      if (directVideoIngress) {
        profile.name = workerVideoIngress
          ? "window_detail_generated_worker"
          : "window_detail_generated";
        profile.degradeLoss = 0.075;
        profile.severeLoss = 0.14;
        profile.degradeRttMs = 420;
        profile.severeRttMs = 620;
        profile.nearCapacityRatio = 1.01;
        profile.bitrateUndershootRatio = 0.68;
        profile.minHeadroomFactor = 1.18;
        profile.upgradeLossMax = 0.02;
        profile.upgradeRttMaxMs = 210;
        profile.fpsDropThreshold = 0.80;
        profile.encodeMsPerFrameCpuThreshold = workerVideoIngress ? 24 : 22;
        profile.dropRatioThreshold = 0.18;
        profile.degradeBaseStreak = 4;
        profile.degradeFpsDropStrongStreak = 5;
        profile.degradeFpsDropStreak = 7;
        profile.upgradeStreak = 5;
        profile.telemetryCapDetectStreak = workerVideoIngress ? 16 : 14;
        profile.telemetryCapRecoverStreak = 5;
      }
      return profile;
    }

    const profile = {
      name: "motion",
      tickIntervalMs: 500,
      emaAlphaBitrate: 0.40,
      emaAlphaFps: 0.44,
      emaAlphaLoss: 0.30,
      emaAlphaRtt: 0.28,
      emaAlphaDropRatio: 0.34,
      emaAlphaEncodeMs: 0.28,
      degradeLoss: 0.04,
      severeLoss: 0.09,
      degradeRttMs: 300,
      severeRttMs: 470,
      nearCapacityRatio: 0.97,
      bitrateUndershootRatio: 0.78,
      minHeadroomFactor: 1.30,
      upgradeLossMax: 0.015,
      upgradeRttMaxMs: 180,
      fpsDropThreshold: 0.82,
      encodeMsPerFrameCpuThreshold: 20,
      dropRatioThreshold: 0.20,
      degradeBaseStreak: 2,
      degradeFpsDropStrongStreak: 3,
      degradeFpsDropStreak: 5,
      upgradeStreak: 6,
      telemetryCapDetectStreak: 10,
      telemetryCapRecoverStreak: 8,
    };
    if (directVideoIngress) {
      profile.name = workerVideoIngress ? "motion_generated_worker" : "motion_generated";
      profile.degradeLoss = 0.055;
      profile.severeLoss = 0.10;
      profile.degradeRttMs = 340;
      profile.severeRttMs = 540;
      profile.nearCapacityRatio = 0.99;
      profile.bitrateUndershootRatio = 0.72;
      profile.minHeadroomFactor = 1.20;
      profile.upgradeLossMax = 0.025;
      profile.upgradeRttMaxMs = 220;
      profile.fpsDropThreshold = 0.76;
      profile.encodeMsPerFrameCpuThreshold = workerVideoIngress ? 24 : 22;
      profile.dropRatioThreshold = 0.24;
      profile.degradeBaseStreak = 3;
      profile.degradeFpsDropStrongStreak = 4;
      profile.degradeFpsDropStreak = 6;
      profile.upgradeStreak = 4;
      profile.telemetryCapDetectStreak = workerVideoIngress ? 16 : 14;
      profile.telemetryCapRecoverStreak = 4;
    }
    return profile;
  };

  const extractNativeAbrStats = async (peerConnection, sender) => {
    const senderStats = await sender.getStats();
    const outboundCandidates = [];
    const remoteInboundCandidates = [];
    for (const stat of senderStats.values()) {
      if (
        stat?.type === "outbound-rtp" &&
        stat?.isRemote !== true &&
        (stat?.kind === "video" || stat?.mediaType === "video")
      ) {
        outboundCandidates.push(stat);
      } else if (
        stat?.type === "remote-inbound-rtp" &&
        (stat?.kind === "video" || stat?.mediaType === "video")
      ) {
        remoteInboundCandidates.push(stat);
      }
    }

    const byBytesDesc = (left, right) => {
      const leftBytes = Number(left?.bytesSent || 0);
      const rightBytes = Number(right?.bytesSent || 0);
      if (rightBytes !== leftBytes) return rightBytes - leftBytes;
      const leftFrames = Number(left?.framesEncoded || 0);
      const rightFrames = Number(right?.framesEncoded || 0);
      return rightFrames - leftFrames;
    };
    let outbound = outboundCandidates.length ? [...outboundCandidates].sort(byBytesDesc)[0] : null;

    const remoteInbound = remoteInboundCandidates.length
      ? [...remoteInboundCandidates].sort((left, right) => {
          const leftPackets = Number(left?.packetsReceived || 0);
          const rightPackets = Number(right?.packetsReceived || 0);
          if (rightPackets !== leftPackets) return rightPackets - leftPackets;
          const leftLost = Number(left?.packetsLost || 0);
          const rightLost = Number(right?.packetsLost || 0);
          return rightLost - leftLost;
        })[0]
      : null;

    let availableOutgoingBitrate = null;
    let candidateRttMs = null;
    const outboundCandidatesFromPc = [];
    try {
      const pcStats = await peerConnection.getStats();
      let selectedCandidatePairId = null;
      for (const stat of pcStats.values()) {
        if (
          !selectedCandidatePairId &&
          stat?.type === "transport" &&
          stat?.selectedCandidatePairId
        ) {
          selectedCandidatePairId = stat.selectedCandidatePairId;
        }
        if (
          stat?.type === "outbound-rtp" &&
          stat?.isRemote !== true &&
          (stat?.kind === "video" || stat?.mediaType === "video")
        ) {
          outboundCandidatesFromPc.push(stat);
        }
      }
      for (const stat of pcStats.values()) {
        if (stat?.type !== "candidate-pair") continue;
        if (
          (selectedCandidatePairId && stat.id === selectedCandidatePairId) ||
          (!selectedCandidatePairId && stat.state === "succeeded" && stat.nominated === true)
        ) {
          if (Number.isFinite(stat.availableOutgoingBitrate)) {
            availableOutgoingBitrate = Number(stat.availableOutgoingBitrate);
          }
          if (Number.isFinite(stat.currentRoundTripTime)) {
            candidateRttMs = Number(stat.currentRoundTripTime) * 1000;
          }
          break;
        }
      }
      if (!outbound && outboundCandidatesFromPc.length) {
        outbound = [...outboundCandidatesFromPc].sort(byBytesDesc)[0];
      }
    } catch {}

    let remoteFractionLost = null;
    let remoteRttMs = null;
    if (remoteInbound) {
      if (Number.isFinite(remoteInbound.fractionLost)) {
        remoteFractionLost = Number(remoteInbound.fractionLost);
        if (remoteFractionLost > 1) {
          remoteFractionLost = remoteFractionLost / 256;
        }
        remoteFractionLost = Math.min(1, Math.max(0, remoteFractionLost));
      }
      if (Number.isFinite(remoteInbound.roundTripTime)) {
        remoteRttMs = Number(remoteInbound.roundTripTime) * 1000;
      }
    }

    return {
      outbound,
      remoteFractionLost,
      remoteRttMs,
      availableOutgoingBitrate,
      candidateRttMs,
    };
  };

  const createNativeAbrController = (peerConnection, sender, videoTrack, meta) => {
    const ladder = buildNativeAbrLadder(meta);
    const profile = getNativeAbrProfile(meta);
    let levelIndex = 0;
    let previousSample = null;
    let degradeStreak = 0;
    let upgradeStreak = 0;
    let stopped = false;
    let intervalId = null;
    let emaSentBitrateBps = null;
    let emaEncodedFps = null;
    let emaLoss = null;
    let emaRttMs = null;
    let emaDropRatio = null;
    let emaEncodeMsPerFrame = null;
    let emaBandwidthDurationShare = null;
    let emaCpuDurationShare = null;
    let abrTickCounter = 0;
    let noOutboundStreak = 0;
    let negotiatedFrameRateCap = null;
    let negotiatedSizeCap = null;
    let fpsCapDetectStreak = 0;
    let fpsCapRecoverStreak = 0;
    let sizeCapDetectStreak = 0;
    let sizeCapRecoverStreak = 0;
    let negotiationSnapshotLogged = false;
    let signalingSnapshotLogged = false;
    let localSdpDiagnostics = collectVideoSdpDiagnostics(peerConnection.localDescription?.sdp || "");
    let remoteSdpDiagnostics = collectVideoSdpDiagnostics(
      peerConnection.remoteDescription?.sdp || ""
    );
    let sdpDiagnosticsFingerprint = JSON.stringify({
      local: localSdpDiagnostics,
      remote: remoteSdpDiagnostics,
    });

    const reportAbr = message =>
      reportMedia(`desktop_stream_abr session=${meta.sessionId} profile=${profile.name} ${message}`);
    const maxDiagnosticValue = list => {
      if (!Array.isArray(list) || !list.length) {
        return null;
      }
      const numbers = list
        .map(value => Number(value))
        .filter(value => Number.isFinite(value) && value > 0);
      return numbers.length ? Math.max(...numbers) : null;
    };
    const frameSizeMacroblocks = (width, height) => {
      const safeWidth = Math.max(1, Math.round(Number(width || 0) || 0));
      const safeHeight = Math.max(1, Math.round(Number(height || 0) || 0));
      return Math.ceil(safeWidth / 16) * Math.ceil(safeHeight / 16);
    };
    const refreshSdpDiagnostics = (reason = "poll") => {
      const nextLocal = collectVideoSdpDiagnostics(peerConnection.localDescription?.sdp || "");
      const nextRemote = collectVideoSdpDiagnostics(peerConnection.remoteDescription?.sdp || "");
      const nextFingerprint = JSON.stringify({
        local: nextLocal,
        remote: nextRemote,
      });
      const changed = nextFingerprint !== sdpDiagnosticsFingerprint;
      if (changed) {
        sdpDiagnosticsFingerprint = nextFingerprint;
        localSdpDiagnostics.maxFr = nextLocal.maxFr;
        localSdpDiagnostics.maxFs = nextLocal.maxFs;
        localSdpDiagnostics.xGoogleStartBitrate = nextLocal.xGoogleStartBitrate;
        localSdpDiagnostics.xGoogleMinBitrate = nextLocal.xGoogleMinBitrate;
        localSdpDiagnostics.xGoogleMaxBitrate = nextLocal.xGoogleMaxBitrate;
        remoteSdpDiagnostics.maxFr = nextRemote.maxFr;
        remoteSdpDiagnostics.maxFs = nextRemote.maxFs;
        remoteSdpDiagnostics.xGoogleStartBitrate = nextRemote.xGoogleStartBitrate;
        remoteSdpDiagnostics.xGoogleMinBitrate = nextRemote.xGoogleMinBitrate;
        remoteSdpDiagnostics.xGoogleMaxBitrate = nextRemote.xGoogleMaxBitrate;
        reportAbr(
          `sdp_snapshot reason=${reason} local=${JSON.stringify(localSdpDiagnostics)} remote=${
            JSON.stringify(remoteSdpDiagnostics)
          }`
        );
      }
      return changed;
    };
    const getSenderEncodingSnapshot = () => {
      try {
        const parameters = sender.getParameters?.() || {};
        const encoding = Array.isArray(parameters.encodings) && parameters.encodings.length
          ? parameters.encodings[0] || {}
          : {};
        return {
          maxBitrate: Number(encoding.maxBitrate || 0) || null,
          maxFramerate: Number(encoding.maxFramerate || 0) || null,
          scaleResolutionDownBy: Number(encoding.scaleResolutionDownBy || 0) || null,
          degradationPreference:
            typeof parameters.degradationPreference === "string"
              ? parameters.degradationPreference
              : null,
        };
      } catch {
        return {
          maxBitrate: null,
          maxFramerate: null,
          scaleResolutionDownBy: null,
          degradationPreference: null,
        };
      }
    };
    const classifyNegotiatedSendLimiter = (sample, level, senderEncoding, trackSettings) => {
      const { requestedWidth, requestedHeight, requestedFrameRate } = getRequestedTarget(level);
      const requestedFs = frameSizeMacroblocks(requestedWidth, requestedHeight);
      const remoteMaxFr = maxDiagnosticValue(remoteSdpDiagnostics.maxFr);
      const remoteMaxFs = maxDiagnosticValue(remoteSdpDiagnostics.maxFs);
      const senderMaxFr = Number(senderEncoding?.maxFramerate || 0) || null;
      const senderScaleDownBy = Number(senderEncoding?.scaleResolutionDownBy || 0) || null;
      const trackFrameRate = Number(trackSettings?.frameRate || 0) || null;
      const trackWidth = Number(trackSettings?.width || 0) || null;
      const trackHeight = Number(trackSettings?.height || 0) || null;
      const outboundFps = Number(sample?.outboundFramesPerSecond || 0) || null;
      const outboundWidth = Number(sample?.frameWidth || 0) || null;
      const outboundHeight = Number(sample?.frameHeight || 0) || null;
      const qualityReason = String(sample?.qualityLimitationReason || "").toLowerCase();

      if (remoteMaxFr && remoteMaxFr < requestedFrameRate * 0.9) {
        return {
          stage: "remote_sdp_fps_cap",
          detail: `answer max-fr=${remoteMaxFr} requested=${requestedFrameRate}`,
        };
      }
      if (remoteMaxFs && remoteMaxFs < requestedFs * 0.9) {
        return {
          stage: "remote_sdp_size_cap",
          detail: `answer max-fs=${remoteMaxFs} requested-fs=${requestedFs}`,
        };
      }
      if (Number.isFinite(negotiatedFrameRateCap) && negotiatedFrameRateCap > 0 && negotiatedFrameRateCap < requestedFrameRate * 0.9) {
        return {
          stage: "telemetry_fps_cap",
          detail: `detected cap=${negotiatedFrameRateCap} requested=${requestedFrameRate}`,
        };
      }
      if (negotiatedSizeCap && negotiatedSizeCap.width > 0 && negotiatedSizeCap.height > 0) {
        const capFs = frameSizeMacroblocks(negotiatedSizeCap.width, negotiatedSizeCap.height);
        if (capFs < requestedFs * 0.9) {
          return {
            stage: "telemetry_size_cap",
            detail: `detected cap=${negotiatedSizeCap.width}x${negotiatedSizeCap.height} requested=${requestedWidth}x${requestedHeight}`,
          };
        }
      }
      if (senderMaxFr && senderMaxFr < requestedFrameRate * 0.9) {
        return {
          stage: "sender_param_fps_cap",
          detail: `sender maxFramerate=${senderMaxFr} requested=${requestedFrameRate}`,
        };
      }
      if (senderScaleDownBy && senderScaleDownBy > 1.12) {
        return {
          stage: "sender_param_scale_down",
          detail: `sender scaleResolutionDownBy=${senderScaleDownBy.toFixed(2)}`,
        };
      }
      if (trackFrameRate && trackFrameRate < requestedFrameRate * 0.9) {
        return {
          stage: "track_settings_fps_cap",
          detail: `track frameRate=${trackFrameRate} requested=${requestedFrameRate}`,
        };
      }
      if (
        trackWidth &&
        trackHeight &&
        (trackWidth < requestedWidth * 0.9 || trackHeight < requestedHeight * 0.9)
      ) {
        return {
          stage: "track_settings_size_cap",
          detail: `track size=${trackWidth}x${trackHeight} requested=${requestedWidth}x${requestedHeight}`,
        };
      }
      if (qualityReason === "bandwidth") {
        return {
          stage: "outbound_bandwidth_limited",
          detail: "qualityLimitationReason=bandwidth",
        };
      }
      if (qualityReason === "cpu") {
        return {
          stage: "outbound_cpu_limited",
          detail: "qualityLimitationReason=cpu",
        };
      }
      if (outboundFps && outboundFps < requestedFrameRate * 0.72) {
        return {
          stage: "outbound_fps_below_target",
          detail: `outbound fps=${outboundFps.toFixed(2)} requested=${requestedFrameRate}`,
        };
      }
      if (
        outboundWidth &&
        outboundHeight &&
        (outboundWidth < requestedWidth * 0.9 || outboundHeight < requestedHeight * 0.9)
      ) {
        return {
          stage: "outbound_size_below_target",
          detail: `outbound size=${outboundWidth}x${outboundHeight} requested=${requestedWidth}x${requestedHeight}`,
        };
      }
      return null;
    };

    const getRequestedTarget = level => {
      const requestedWidth = Math.max(
        2,
        Math.round(Number(meta?.baseWidth || level?.width || 1920) || 1920)
      );
      const requestedHeight = Math.max(
        2,
        Math.round(Number(meta?.baseHeight || level?.height || 1080) || 1080)
      );
      const requestedFrameRate = Math.max(
        1,
        Math.round(Number(meta?.requestedFrameRate || meta?.baseFrameRate || level?.frameRate || 60) || 60)
      );
      return {
        requestedWidth,
        requestedHeight,
        requestedFrameRate,
      };
    };

    const publishNegotiationHints = () => {
      const hints = {};
      if (Number.isFinite(negotiatedFrameRateCap) && negotiatedFrameRateCap > 0) {
        hints.frameRateCap = Math.max(1, Math.min(120, Math.round(negotiatedFrameRateCap)));
      }
      if (negotiatedSizeCap && negotiatedSizeCap.width > 0 && negotiatedSizeCap.height > 0) {
        const { requestedWidth, requestedHeight } = getRequestedTarget();
        const safeWidth = Math.max(2, Math.round(Number(negotiatedSizeCap.width || 0) || 2));
        const safeHeight = Math.max(2, Math.round(Number(negotiatedSizeCap.height || 0) || 2));
        hints.widthCap = safeWidth;
        hints.heightCap = safeHeight;
        hints.scaleResolutionDownBy = Math.max(
          1,
          requestedWidth / safeWidth,
          requestedHeight / safeHeight
        );
      }

      if (Object.keys(hints).length > 0) {
        desktopStreamSenderNegotiationHints.set(sender, hints);
        desktopStreamPeerConnectionNegotiationHints.set(peerConnection, hints);
      } else {
        desktopStreamSenderNegotiationHints.delete(sender);
        desktopStreamPeerConnectionNegotiationHints.delete(peerConnection);
      }
    };

    const emitNegotiatedSendTelemetry = (sample, level, computed, reason = "tick") => {
      const senderEncoding = getSenderEncodingSnapshot();
      const trackSettings = videoTrack.getSettings?.() || {};
      refreshSdpDiagnostics(`telemetry:${reason}`);
      const limiter = classifyNegotiatedSendLimiter(sample, level, senderEncoding, trackSettings);
      const { requestedWidth, requestedHeight, requestedFrameRate } = getRequestedTarget(level);
      const payload = {
        reason,
        sessionId: meta.sessionId || "",
        sourceId: meta.sourceId || "",
        connectionState: String(peerConnection.connectionState || ""),
        iceConnectionState: String(peerConnection.iceConnectionState || ""),
        requested: {
          width: requestedWidth,
          height: requestedHeight,
          frameRate: requestedFrameRate,
        },
        level: level
          ? {
              index: levelIndex + 1,
              count: ladder.length,
              width: Math.round(Number(level.width || 0) || 0),
              height: Math.round(Number(level.height || 0) || 0),
              frameRate: Math.round(Number(level.frameRate || 0) || 0),
              targetBitrate: Math.round(Number(level.targetBitrate || 0) || 0),
            }
          : null,
        computed: computed || null,
        outbound: sample
          ? {
              frameWidth: Number(sample.frameWidth || 0) || null,
              frameHeight: Number(sample.frameHeight || 0) || null,
              fps: Number(sample.outboundFramesPerSecond || 0) || null,
              qualityLimitationReason: String(sample.qualityLimitationReason || ""),
              availableOutgoingBitrate: Number(sample.availableOutgoingBitrate || 0) || null,
              rttMs: Number.isFinite(sample.rttMs) ? Number(sample.rttMs) : null,
              remoteFractionLost: Number.isFinite(sample.remoteFractionLost)
                ? Number(sample.remoteFractionLost)
                : null,
            }
          : null,
        sender: senderEncoding,
        track: {
          width: Number(trackSettings.width || 0) || null,
          height: Number(trackSettings.height || 0) || null,
          frameRate: Number(trackSettings.frameRate || 0) || null,
        },
        telemetryCaps: {
          frameRateCap: Number.isFinite(negotiatedFrameRateCap) ? negotiatedFrameRateCap : null,
          widthCap:
            negotiatedSizeCap && Number.isFinite(negotiatedSizeCap.width)
              ? negotiatedSizeCap.width
              : null,
          heightCap:
            negotiatedSizeCap && Number.isFinite(negotiatedSizeCap.height)
              ? negotiatedSizeCap.height
              : null,
        },
        sdp: {
          local: localSdpDiagnostics,
          remote: remoteSdpDiagnostics,
        },
        limiter,
      };

      reportAbr(`negotiated_send=${JSON.stringify(payload)}`);
    };

    reportAbr(`controller_started tick_ms=${profile.tickIntervalMs} ladder=${ladder.length}`);

    const applyLevel = async (reason = "unknown") => {
      const level = ladder[levelIndex];
      if (!level) return;
      const { requestedWidth, requestedHeight, requestedFrameRate } = getRequestedTarget(level);
      const levelWidth = Math.max(2, Math.round(Number(level.width || 0) || requestedWidth));
      const levelHeight = Math.max(2, Math.round(Number(level.height || 0) || requestedHeight));
      const levelFrameRate = Math.max(
        1,
        Math.round(Number(level.frameRate || 0) || requestedFrameRate)
      );
      let effectiveWidth = levelWidth;
      let effectiveHeight = levelHeight;
      if (
        negotiatedSizeCap &&
        Number.isFinite(negotiatedSizeCap.width) &&
        Number.isFinite(negotiatedSizeCap.height) &&
        negotiatedSizeCap.width > 0 &&
        negotiatedSizeCap.height > 0
      ) {
        const capWidth = Math.max(2, Math.round(Number(negotiatedSizeCap.width || 0) || 2));
        const capHeight = Math.max(2, Math.round(Number(negotiatedSizeCap.height || 0) || 2));
        if (capWidth < effectiveWidth || capHeight < effectiveHeight) {
          const widthScale = effectiveWidth / capWidth;
          const heightScale = effectiveHeight / capHeight;
          const scale = Math.max(1, widthScale, heightScale);
          effectiveWidth = Math.max(2, Math.round(effectiveWidth / scale));
          effectiveHeight = Math.max(2, Math.round(effectiveHeight / scale));
        }
      }
      const fpsCeiling =
        Number.isFinite(negotiatedFrameRateCap) && negotiatedFrameRateCap > 0
          ? Math.max(1, Math.min(120, Math.round(negotiatedFrameRateCap)))
          : null;
      const effectiveFrameRate = fpsCeiling
        ? Math.max(1, Math.min(levelFrameRate, fpsCeiling))
        : levelFrameRate;
      const expectedPixelRate = Math.max(1, levelWidth * levelHeight * levelFrameRate);
      const effectivePixelRate = Math.max(1, effectiveWidth * effectiveHeight * effectiveFrameRate);
      const bitrateScale = Math.min(1, Math.max(0.30, effectivePixelRate / expectedPixelRate));
      const effectiveTargetBitrate = Math.max(
        250000,
        Math.round(Number(level.targetBitrate || 0) * bitrateScale)
      );
      const effectiveScaleDownBy = Math.max(
        1,
        Number(level.scaleDownBy || 1),
        requestedWidth / effectiveWidth,
        requestedHeight / effectiveHeight
      );

      try {
        const parameters = sender.getParameters?.() || {};
        const existingEncodings =
          Array.isArray(parameters.encodings) && parameters.encodings.length
            ? parameters.encodings
            : [{}];
        const primaryEncoding = existingEncodings[0] || {};
        primaryEncoding.active = true;
        primaryEncoding.maxBitrate = effectiveTargetBitrate;
        primaryEncoding.maxFramerate = Math.max(1, Math.min(requestedFrameRate, effectiveFrameRate));
        primaryEncoding.scaleResolutionDownBy = effectiveScaleDownBy;
        primaryEncoding.priority = "high";

        parameters.degradationPreference =
          profile.name === "window_detail" ? "maintain-resolution" : "maintain-framerate";
        parameters.encodings = [primaryEncoding];
        await sender.setParameters?.(parameters);
      } catch (error) {
        reportAbr(
          `set_parameters_failed reason=${reason} message=${
            error && error.message ? error.message : String(error)
          }`
        );
      }

      await applyScreenShareTrackConstraints(
        videoTrack,
        {
          width: effectiveWidth,
          height: effectiveHeight,
          frameRate: effectiveFrameRate,
        },
        meta.contentHint
      );
      publishNegotiationHints();
      const appliedEncoding = getSenderEncodingSnapshot();
      const trackSettings = videoTrack.getSettings?.() || {};

      reportAbr(
        `level=${levelIndex + 1}/${ladder.length} reason=${reason} ` +
          `size=${Math.round(level.width)}x${Math.round(level.height)} ` +
          `fps=${Math.round(level.frameRate)} bitrate=${Math.round(level.targetBitrate)} ` +
          `effective=${effectiveWidth}x${effectiveHeight}@${effectiveFrameRate} ` +
          `effectiveBitrate=${effectiveTargetBitrate} ` +
          `applied=${JSON.stringify(appliedEncoding)} track=${JSON.stringify({
            width: trackSettings.width ?? null,
            height: trackSettings.height ?? null,
            frameRate: trackSettings.frameRate ?? null,
          })}`
      );
      emitNegotiatedSendTelemetry(
        null,
        level,
        {
          width: effectiveWidth,
          height: effectiveHeight,
          frameRate: effectiveFrameRate,
          maxBitrate: effectiveTargetBitrate,
          scaleResolutionDownBy: effectiveScaleDownBy,
          bitrateScale,
        },
        `apply_level:${reason}`
      );
    };

    const controllerApi = {
      sessionId: String(meta?.sessionId || ""),
      sender,
      videoTrack,
      stop: reason => {
        if (stopped) return;
        stopped = true;
        if (intervalId) {
          window.clearInterval(intervalId);
          intervalId = null;
        }
        nativeAbrControllers.delete(sender);
        nativeAbrControllerRecords.delete(controllerApi);
        desktopStreamSenderNegotiationHints.delete(sender);
        desktopStreamPeerConnectionNegotiationHints.delete(peerConnection);
        try {
          peerConnection.removeEventListener("connectionstatechange", onConnectionStateChange);
        } catch {}
        try {
          peerConnection.removeEventListener("iceconnectionstatechange", onIceConnectionStateChange);
        } catch {}
        try {
          peerConnection.removeEventListener("signalingstatechange", onSignalingStateChange);
        } catch {}
        try {
          videoTrack.removeEventListener("ended", onTrackEnded);
        } catch {}
        if (reason) {
          reportAbr(`controller_stopped reason=${reason}`);
        }
      },
    };

    const stop = reason => {
      if (stopped) return;
      controllerApi.stop(reason);
    };

    const onConnectionStateChange = () => {
      refreshSdpDiagnostics("connection_state");
      const state = String(peerConnection.connectionState || "");
      if (state === "closed" || state === "failed" || state === "disconnected") {
        stop("connection_state");
      }
    };
    const onIceConnectionStateChange = () => {
      refreshSdpDiagnostics("ice_state");
    };
    const onSignalingStateChange = () => {
      const changed = refreshSdpDiagnostics("signaling_state");
      if (changed || !signalingSnapshotLogged) {
        signalingSnapshotLogged = true;
        reportAbr(
          `signaling_snapshot signaling=${String(peerConnection.signalingState || "")} ` +
            `connection=${String(peerConnection.connectionState || "")} ` +
            `ice=${String(peerConnection.iceConnectionState || "")} sender=${JSON.stringify(
              getSenderEncodingSnapshot()
            )}`
        );
      }
    };

    const onTrackEnded = () => {
      stop("track_ended");
    };

    const tick = async () => {
      if (stopped) return;
      abrTickCounter += 1;
      if (videoTrack.readyState === "ended") {
        stop();
        return;
      }

      const stats = await extractNativeAbrStats(peerConnection, sender).catch(() => null);
      const outbound = stats?.outbound || null;
      if (!outbound || !Number.isFinite(outbound.bytesSent)) {
        noOutboundStreak += 1;
        if (noOutboundStreak === 1 || noOutboundStreak % 8 === 0) {
          reportAbr(`no_outbound_stats streak=${noOutboundStreak}`);
        }
        return;
      }
      noOutboundStreak = 0;

      const now = performance.now();
      const sample = {
        at: now,
        bytesSent: Number(outbound.bytesSent || 0),
        framesEncoded: Number(outbound.framesEncoded || 0),
        framesDropped: Number(outbound.framesDropped || 0),
        totalEncodeTimeSec: Number(outbound.totalEncodeTime || 0),
        outboundFramesPerSecond: Number(outbound.framesPerSecond || 0),
        frameWidth: Number(outbound.frameWidth || 0),
        frameHeight: Number(outbound.frameHeight || 0),
        encoderImplementation:
          typeof outbound.encoderImplementation === "string"
            ? outbound.encoderImplementation
            : null,
        qualityLimitationReason: String(outbound.qualityLimitationReason || ""),
        qualityLimitationDurations: normalizeQualityLimitationDurations(
          outbound.qualityLimitationDurations
        ),
        availableOutgoingBitrate: Number(stats?.availableOutgoingBitrate || 0),
        remoteFractionLost: Number.isFinite(stats?.remoteFractionLost)
          ? Number(stats.remoteFractionLost)
          : null,
        rttMs: Number.isFinite(stats?.remoteRttMs)
          ? Number(stats.remoteRttMs)
          : Number.isFinite(stats?.candidateRttMs)
            ? Number(stats.candidateRttMs)
            : null,
      };

      if (
        !negotiationSnapshotLogged &&
        (String(peerConnection.connectionState || "") === "connected" ||
          String(peerConnection.iceConnectionState || "") === "connected" ||
          String(peerConnection.iceConnectionState || "") === "completed")
      ) {
        negotiationSnapshotLogged = true;
        refreshSdpDiagnostics("connected");
        reportAbr(
          `negotiated_sdp local=${JSON.stringify(localSdpDiagnostics)} remote=${JSON.stringify(
            remoteSdpDiagnostics
          )} sender=${JSON.stringify(getSenderEncodingSnapshot())}`
        );
      }

      if (!previousSample) {
        previousSample = sample;
        await applyLevel("abr_init");
        return;
      }

      const elapsedSec = Math.max(0.2, (sample.at - previousSample.at) / 1000);
      const sentBitrateRawBps = Math.max(
        0,
        ((sample.bytesSent - previousSample.bytesSent) * 8) / elapsedSec
      );
      const encodedFpsRaw = Math.max(
        0,
        (sample.framesEncoded - previousSample.framesEncoded) / elapsedSec
      );
      const droppedFpsRaw = Math.max(
        0,
        (sample.framesDropped - previousSample.framesDropped) / elapsedSec
      );
      const totalFrameRateRaw = Math.max(0, encodedFpsRaw + droppedFpsRaw);
      const dropRatioRaw = totalFrameRateRaw > 0 ? droppedFpsRaw / totalFrameRateRaw : 0;
      const encodedFramesDelta = Math.max(0, sample.framesEncoded - previousSample.framesEncoded);
      const encodeTimeDeltaSec = Math.max(
        0,
        sample.totalEncodeTimeSec - previousSample.totalEncodeTimeSec
      );
      const encodeMsPerFrameRaw =
        encodedFramesDelta > 0 ? (encodeTimeDeltaSec * 1000) / encodedFramesDelta : 0;

      let bandwidthDurationShareRaw = 0;
      let cpuDurationShareRaw = 0;
      if (sample.qualityLimitationDurations && previousSample.qualityLimitationDurations) {
        const currentDur = sample.qualityLimitationDurations;
        const previousDur = previousSample.qualityLimitationDurations;
        const deltaNone = Math.max(0, currentDur.none - previousDur.none);
        const deltaCpu = Math.max(0, currentDur.cpu - previousDur.cpu);
        const deltaBandwidth = Math.max(0, currentDur.bandwidth - previousDur.bandwidth);
        const deltaOther = Math.max(0, currentDur.other - previousDur.other);
        const deltaTotal = deltaNone + deltaCpu + deltaBandwidth + deltaOther;
        if (deltaTotal > 0) {
          bandwidthDurationShareRaw = deltaBandwidth / deltaTotal;
          cpuDurationShareRaw = deltaCpu / deltaTotal;
        }
      }

      emaSentBitrateBps = smoothAbrEma(emaSentBitrateBps, sentBitrateRawBps, profile.emaAlphaBitrate);
      emaEncodedFps = smoothAbrEma(emaEncodedFps, encodedFpsRaw, profile.emaAlphaFps);
      emaDropRatio = smoothAbrEma(emaDropRatio, dropRatioRaw, profile.emaAlphaDropRatio);
      emaEncodeMsPerFrame = smoothAbrEma(
        emaEncodeMsPerFrame,
        encodeMsPerFrameRaw,
        profile.emaAlphaEncodeMs
      );
      emaLoss = smoothAbrEma(emaLoss, sample.remoteFractionLost, profile.emaAlphaLoss);
      emaRttMs = smoothAbrEma(emaRttMs, sample.rttMs, profile.emaAlphaRtt);
      emaBandwidthDurationShare = smoothAbrEma(
        emaBandwidthDurationShare,
        bandwidthDurationShareRaw,
        0.3
      );
      emaCpuDurationShare = smoothAbrEma(emaCpuDurationShare, cpuDurationShareRaw, 0.3);
      previousSample = sample;

      const level = ladder[levelIndex];
      const expectedBitrate = Number(level?.targetBitrate || 1);
      const expectedFps = Math.max(1, Number(level?.frameRate || 1));
      const bandwidthLimited = sample.qualityLimitationReason === "bandwidth";
      const cpuLimited = sample.qualityLimitationReason === "cpu";
      const bandwidthDurationPressure =
        Number.isFinite(emaBandwidthDurationShare) && emaBandwidthDurationShare > 0.32;
      const cpuDurationPressure = Number.isFinite(emaCpuDurationShare) && emaCpuDurationShare > 0.35;
      const highLoss = Number.isFinite(emaLoss) && emaLoss > profile.degradeLoss;
      const severeLoss = Number.isFinite(emaLoss) && emaLoss > profile.severeLoss;
      const highRtt = Number.isFinite(emaRttMs) && emaRttMs > profile.degradeRttMs;
      const severeRtt = Number.isFinite(emaRttMs) && emaRttMs > profile.severeRttMs;
      const nearCapacity =
        Number.isFinite(sample.availableOutgoingBitrate) &&
        sample.availableOutgoingBitrate > 0 &&
        Number.isFinite(emaSentBitrateBps) &&
        emaSentBitrateBps > sample.availableOutgoingBitrate * profile.nearCapacityRatio;
      const bitrateUndershoot =
        Number.isFinite(emaSentBitrateBps) &&
        emaSentBitrateBps < expectedBitrate * profile.bitrateUndershootRatio;
      const lowEncodeFps =
        expectedFps > 30 &&
        Number.isFinite(emaEncodedFps) &&
        emaEncodedFps > 0 &&
        emaEncodedFps < expectedFps * profile.fpsDropThreshold;
      const highDropRatio =
        Number.isFinite(emaDropRatio) && emaDropRatio > profile.dropRatioThreshold;
      const highEncodeCost =
        Number.isFinite(emaEncodeMsPerFrame) &&
        emaEncodeMsPerFrame > profile.encodeMsPerFrameCpuThreshold &&
        expectedFps >= 45;
      const strongCongestion =
        bandwidthLimited ||
        bandwidthDurationPressure ||
        severeLoss ||
        severeRtt ||
        (nearCapacity && (highLoss || highRtt || bitrateUndershoot));
      const { requestedWidth, requestedHeight, requestedFrameRate } = getRequestedTarget(level);
      const observedFps = Number.isFinite(sample.outboundFramesPerSecond) && sample.outboundFramesPerSecond > 0
        ? sample.outboundFramesPerSecond
        : encodedFpsRaw;
      const observedWidth =
        Number.isFinite(sample.frameWidth) && sample.frameWidth > 0
          ? Math.round(sample.frameWidth)
          : null;
      const observedHeight =
        Number.isFinite(sample.frameHeight) && sample.frameHeight > 0
          ? Math.round(sample.frameHeight)
          : null;
      const uncongestedTelemetryWindow =
        !bandwidthLimited &&
        !cpuLimited &&
        !bandwidthDurationPressure &&
        !cpuDurationPressure &&
        (!Number.isFinite(emaLoss) || emaLoss < 0.01) &&
        (!Number.isFinite(emaRttMs) || emaRttMs < profile.upgradeRttMaxMs) &&
        (!Number.isFinite(emaDropRatio) || emaDropRatio < 0.03);
      const likelyNegotiatedFpsCap =
        requestedFrameRate >= 50 &&
        Number.isFinite(observedFps) &&
        observedFps > 8 &&
        observedFps <= 36 &&
        observedFps < requestedFrameRate * 0.72;
      if (uncongestedTelemetryWindow && likelyNegotiatedFpsCap) {
        fpsCapDetectStreak += 1;
      } else {
        fpsCapDetectStreak = 0;
      }
      if (fpsCapDetectStreak >= profile.telemetryCapDetectStreak) {
        const nextCap = Math.max(24, Math.min(60, Math.round(observedFps / 5) * 5));
        if (!Number.isFinite(negotiatedFrameRateCap) || Math.abs(negotiatedFrameRateCap - nextCap) >= 1) {
          negotiatedFrameRateCap = nextCap;
          publishNegotiationHints();
          reportAbr(
            `telemetry_fps_cap_detected cap=${nextCap} observed=${observedFps.toFixed(2)} ` +
              `requested=${requestedFrameRate}`
          );
          await applyLevel("telemetry_fps_cap");
          return;
        }
        fpsCapDetectStreak = 0;
      }
      if (
        Number.isFinite(negotiatedFrameRateCap) &&
        negotiatedFrameRateCap > 0 &&
        Number.isFinite(observedFps) &&
        observedFps >= Math.min(requestedFrameRate * 0.92, 45)
      ) {
        fpsCapRecoverStreak += 1;
      } else {
        fpsCapRecoverStreak = 0;
      }
      if (
        fpsCapRecoverStreak >= profile.telemetryCapRecoverStreak &&
        Number.isFinite(negotiatedFrameRateCap)
      ) {
        reportAbr(
          `telemetry_fps_cap_cleared previous=${negotiatedFrameRateCap} observed=${
            Number.isFinite(observedFps) ? observedFps.toFixed(2) : "na"
          }`
        );
        negotiatedFrameRateCap = null;
        fpsCapRecoverStreak = 0;
        publishNegotiationHints();
        await applyLevel("telemetry_fps_cap_cleared");
        return;
      }

      const likelyNegotiatedSizeCap =
        levelIndex === 0 &&
        requestedHeight >= 900 &&
        observedWidth &&
        observedHeight &&
        observedWidth > 0 &&
        observedHeight > 0 &&
        observedWidth < requestedWidth * 0.85 &&
        observedHeight < requestedHeight * 0.85;
      if (uncongestedTelemetryWindow && likelyNegotiatedSizeCap) {
        sizeCapDetectStreak += 1;
      } else {
        sizeCapDetectStreak = 0;
      }
      if (sizeCapDetectStreak >= profile.telemetryCapDetectStreak) {
        const nextSizeCap = {
          width: Math.max(2, observedWidth || 2),
          height: Math.max(2, observedHeight || 2),
        };
        const currentArea = negotiatedSizeCap
          ? Math.max(1, Number(negotiatedSizeCap.width || 0) * Number(negotiatedSizeCap.height || 0))
          : 0;
        const nextArea = Math.max(1, nextSizeCap.width * nextSizeCap.height);
        if (!negotiatedSizeCap || Math.abs(currentArea - nextArea) > currentArea * 0.08) {
          negotiatedSizeCap = nextSizeCap;
          publishNegotiationHints();
          reportAbr(
            `telemetry_size_cap_detected cap=${nextSizeCap.width}x${nextSizeCap.height} ` +
              `requested=${requestedWidth}x${requestedHeight}`
          );
          await applyLevel("telemetry_size_cap");
          return;
        }
        sizeCapDetectStreak = 0;
      }
      if (
        negotiatedSizeCap &&
        observedWidth &&
        observedHeight &&
        observedWidth >= requestedWidth * 0.94 &&
        observedHeight >= requestedHeight * 0.94
      ) {
        sizeCapRecoverStreak += 1;
      } else {
        sizeCapRecoverStreak = 0;
      }
      if (sizeCapRecoverStreak >= profile.telemetryCapRecoverStreak && negotiatedSizeCap) {
        reportAbr(
          `telemetry_size_cap_cleared previous=${negotiatedSizeCap.width}x${
            negotiatedSizeCap.height
          }`
        );
        negotiatedSizeCap = null;
        sizeCapRecoverStreak = 0;
        publishNegotiationHints();
        await applyLevel("telemetry_size_cap_cleared");
        return;
      }

      if (abrTickCounter % 8 === 0) {
        reportAbr(
          `tick level=${levelIndex + 1} enc_fps_raw=${encodedFpsRaw.toFixed(2)} ` +
            `enc_fps_ema=${Number.isFinite(emaEncodedFps) ? emaEncodedFps.toFixed(2) : "na"} ` +
            `out_fps=${Number.isFinite(sample.outboundFramesPerSecond) ? sample.outboundFramesPerSecond.toFixed(2) : "na"} ` +
            `bitrate_raw=${Math.round(sentBitrateRawBps)} bitrate_ema=${
              Number.isFinite(emaSentBitrateBps) ? Math.round(emaSentBitrateBps) : "na"
            } ` +
            `avail=${
              Number.isFinite(sample.availableOutgoingBitrate)
                ? Math.round(sample.availableOutgoingBitrate)
                : "na"
            } ` +
            `loss=${Number.isFinite(emaLoss) ? emaLoss.toFixed(3) : "na"} ` +
            `rtt=${Number.isFinite(emaRttMs) ? emaRttMs.toFixed(1) : "na"} ` +
            `drop=${Number.isFinite(emaDropRatio) ? emaDropRatio.toFixed(3) : "na"} ` +
            `reason=${sample.qualityLimitationReason || "none"} sender=${JSON.stringify(
              getSenderEncodingSnapshot()
            )}`
        );
      }
      if (abrTickCounter % 6 === 0) {
        emitNegotiatedSendTelemetry(
          sample,
          level,
          {
            observedFps: Number.isFinite(observedFps) ? Number(observedFps.toFixed(2)) : null,
            observedWidth,
            observedHeight,
            emaEncodedFps: Number.isFinite(emaEncodedFps) ? Number(emaEncodedFps.toFixed(2)) : null,
            emaSentBitrateBps: Number.isFinite(emaSentBitrateBps)
              ? Math.round(emaSentBitrateBps)
              : null,
            emaLoss: Number.isFinite(emaLoss) ? Number(emaLoss.toFixed(4)) : null,
            emaRttMs: Number.isFinite(emaRttMs) ? Number(emaRttMs.toFixed(2)) : null,
            encoderImplementation: sample.encoderImplementation || null,
          },
          "abr_tick"
        );
      }

      const shouldDegrade =
        bandwidthLimited ||
        bandwidthDurationPressure ||
        highLoss ||
        highRtt ||
        (nearCapacity && bitrateUndershoot) ||
        ((cpuLimited || cpuDurationPressure) && (lowEncodeFps || highDropRatio || highEncodeCost));
      const hasHeadroom =
        (!Number.isFinite(sample.availableOutgoingBitrate) ||
          sample.availableOutgoingBitrate <= 0 ||
          sample.availableOutgoingBitrate > expectedBitrate * profile.minHeadroomFactor) &&
        (!Number.isFinite(emaLoss) || emaLoss < profile.upgradeLossMax) &&
        (!Number.isFinite(emaRttMs) || emaRttMs < profile.upgradeRttMaxMs) &&
        !bandwidthLimited &&
        !bandwidthDurationPressure &&
        !cpuDurationPressure &&
        (!Number.isFinite(emaEncodedFps) || emaEncodedFps >= expectedFps * 0.92) &&
        (!Number.isFinite(emaDropRatio) || emaDropRatio < 0.03);

      if (shouldDegrade) {
        degradeStreak += 1;
        upgradeStreak = 0;
      } else if (hasHeadroom) {
        upgradeStreak += 1;
        degradeStreak = 0;
      } else {
        degradeStreak = 0;
        upgradeStreak = 0;
      }

      const nextLevel = levelIndex < ladder.length - 1 ? ladder[levelIndex + 1] : null;
      const fpsDropOnNextLevel = Boolean(
        nextLevel && Number(nextLevel.frameRate || 0) < expectedFps
      );
      const degradeThreshold = fpsDropOnNextLevel
        ? strongCongestion
          ? profile.degradeFpsDropStrongStreak
          : profile.degradeFpsDropStreak
        : profile.degradeBaseStreak;

      if (degradeStreak >= degradeThreshold && levelIndex < ladder.length - 1) {
        levelIndex += 1;
        degradeStreak = 0;
        upgradeStreak = 0;
        await applyLevel(
          `degrade threshold=${degradeThreshold} fpsDrop=${fpsDropOnNextLevel} ` +
            `bw=${bandwidthLimited || bandwidthDurationPressure} loss=${
            Number.isFinite(emaLoss) ? emaLoss.toFixed(3) : "na"
          } rttMs=${Number.isFinite(emaRttMs) ? emaRttMs.toFixed(1) : "na"} ` +
            `bitrate=${Number.isFinite(emaSentBitrateBps) ? Math.round(emaSentBitrateBps) : "na"}`
        );
        return;
      }

      if (upgradeStreak >= profile.upgradeStreak && levelIndex > 0) {
        levelIndex -= 1;
        degradeStreak = 0;
        upgradeStreak = 0;
        await applyLevel("upgrade_headroom");
      }
    };

    peerConnection.addEventListener("connectionstatechange", onConnectionStateChange);
    peerConnection.addEventListener("iceconnectionstatechange", onIceConnectionStateChange);
    peerConnection.addEventListener("signalingstatechange", onSignalingStateChange);
    videoTrack.addEventListener("ended", onTrackEnded);
    void applyLevel("abr_start_force");
    intervalId = window.setInterval(() => {
      void tick().catch(error => {
        reportAbr(
          `tick_failed message=${error && error.message ? error.message : String(error)}`
        );
      });
    }, profile.tickIntervalMs);
    void tick().catch(error => {
      reportAbr(`tick_failed message=${error && error.message ? error.message : String(error)}`);
    });

    nativeAbrControllerRecords.add(controllerApi);
    return controllerApi;
  };

  const getNativeShareNegotiationTarget = peerConnection => {
    if (!peerConnection || typeof peerConnection.getSenders !== "function") {
      return null;
    }

    let target = null;
    for (const sender of peerConnection.getSenders()) {
      const track = sender?.track;
      const kind = String(track?.kind || "").toLowerCase();
      const meta =
        track && kind === "video"
          ? desktopStreamTrackMeta.get(track)
          : desktopStreamSenderPendingMeta.get(sender);
      if (!meta) continue;

      const width = Math.max(2, Math.round(Number(meta.baseWidth || 0) || 0));
      const height = Math.max(2, Math.round(Number(meta.baseHeight || 0) || 0));
      const frameRate = Math.max(1, Math.round(Number(meta.baseFrameRate || 60) || 60));
      if (!width || !height || !frameRate) continue;

      if (
        !target ||
        frameRate > target.frameRate ||
        (frameRate === target.frameRate && height > target.height)
      ) {
        target = { width, height, frameRate };
      }
    }

    const negotiationHints = desktopStreamPeerConnectionNegotiationHints.get(peerConnection);
    if (target && negotiationHints && typeof negotiationHints === "object") {
      const frameRateCap = Number(negotiationHints.frameRateCap || 0);
      if (Number.isFinite(frameRateCap) && frameRateCap > 0) {
        target.frameRate = Math.max(1, Math.min(target.frameRate, Math.round(frameRateCap)));
      }

      const widthCap = Number(negotiationHints.widthCap || 0);
      const heightCap = Number(negotiationHints.heightCap || 0);
      if (Number.isFinite(widthCap) && Number.isFinite(heightCap) && widthCap > 0 && heightCap > 0) {
        const safeWidthCap = Math.max(2, Math.round(widthCap));
        const safeHeightCap = Math.max(2, Math.round(heightCap));
        if (safeWidthCap < target.width || safeHeightCap < target.height) {
          const widthScale = target.width / safeWidthCap;
          const heightScale = target.height / safeHeightCap;
          const scale = Math.max(1, widthScale, heightScale);
          target.width = Math.max(2, Math.round(target.width / scale));
          target.height = Math.max(2, Math.round(target.height / scale));
        }
      }
    }

    return target;
  };

  const patchVideoSectionFmtp = (lines, sectionStart, sectionEnd, target) => {
    const mLine = String(lines[sectionStart] || "");
    if (!mLine.startsWith("m=video")) {
      return 0;
    }

    const mParts = mLine.trim().split(/\s+/);
    const payloadIds = mParts.slice(3).filter(Boolean);
    if (!payloadIds.length) {
      return 0;
    }

    const codecByPayload = new Map();
    for (let lineIndex = sectionStart + 1; lineIndex < sectionEnd; lineIndex += 1) {
      const rtpMatch = String(lines[lineIndex] || "").match(
        /^a=rtpmap:(\d+)\s+([A-Za-z0-9_-]+)/i
      );
      if (!rtpMatch) continue;
      codecByPayload.set(String(rtpMatch[1]), String(rtpMatch[2] || "").toLowerCase());
    }

    const videoPayloadIds = payloadIds.filter(payloadId => {
      const codec = codecByPayload.get(payloadId);
      if (!codec) return true;
      return !["rtx", "red", "ulpfec", "flexfec-03"].includes(codec);
    });
    if (!videoPayloadIds.length) {
      return 0;
    }

    const maxFr = Math.max(1, Math.min(120, Math.round(Number(target.frameRate || 60) || 60)));
    const width = Math.max(2, Math.round(Number(target.width || 1920) || 1920));
    const height = Math.max(2, Math.round(Number(target.height || 1080) || 1080));
    const maxFs = Math.max(1, Math.ceil(width / 16) * Math.ceil(height / 16));
    const targetBitrateKbps = Math.max(
      300,
      Math.round(computeScreenShareTargetBitrate(width, height, maxFr) / 1000)
    );
    const minBitrateKbps = Math.max(150, Math.round(targetBitrateKbps * 0.35));
    const startBitrateKbps = Math.max(minBitrateKbps, Math.round(targetBitrateKbps * 0.75));
    const maxBitrateKbps = Math.max(startBitrateKbps, Math.round(targetBitrateKbps * 1.15));
    const upsertFmtpParams = existingParams => {
      const tokens = String(existingParams || "")
        .split(";")
        .map(token => token.trim())
        .filter(Boolean);
      const nextTokens = [];
      let hasMaxFr = false;
      let hasMaxFs = false;
      let hasStartBitrate = false;
      let hasMinBitrate = false;
      let hasMaxBitrate = false;

      for (const token of tokens) {
        const [rawKey, ...rawValueParts] = token.split("=");
        const key = String(rawKey || "").trim().toLowerCase();
        if (!key) continue;
        if (key === "max-fr") {
          nextTokens.push(`max-fr=${maxFr}`);
          hasMaxFr = true;
          continue;
        }
        if (key === "max-fs") {
          nextTokens.push(`max-fs=${maxFs}`);
          hasMaxFs = true;
          continue;
        }
        if (key === "x-google-start-bitrate") {
          nextTokens.push(`x-google-start-bitrate=${startBitrateKbps}`);
          hasStartBitrate = true;
          continue;
        }
        if (key === "x-google-min-bitrate") {
          nextTokens.push(`x-google-min-bitrate=${minBitrateKbps}`);
          hasMinBitrate = true;
          continue;
        }
        if (key === "x-google-max-bitrate") {
          nextTokens.push(`x-google-max-bitrate=${maxBitrateKbps}`);
          hasMaxBitrate = true;
          continue;
        }
        if (rawValueParts.length > 0) {
          nextTokens.push(`${rawKey.trim()}=${rawValueParts.join("=").trim()}`);
        } else {
          nextTokens.push(rawKey.trim());
        }
      }

      if (!hasMaxFr) nextTokens.push(`max-fr=${maxFr}`);
      if (!hasMaxFs) nextTokens.push(`max-fs=${maxFs}`);
      if (!hasStartBitrate) nextTokens.push(`x-google-start-bitrate=${startBitrateKbps}`);
      if (!hasMinBitrate) nextTokens.push(`x-google-min-bitrate=${minBitrateKbps}`);
      if (!hasMaxBitrate) nextTokens.push(`x-google-max-bitrate=${maxBitrateKbps}`);
      return nextTokens.join(";");
    };

    let insertedLines = 0;
    let currentSectionEnd = sectionEnd;
    for (const payloadId of videoPayloadIds) {
      let fmtpIndex = -1;
      for (
        let lineIndex = sectionStart + 1;
        lineIndex < currentSectionEnd;
        lineIndex += 1
      ) {
        if (String(lines[lineIndex] || "").startsWith(`a=fmtp:${payloadId} `)) {
          fmtpIndex = lineIndex;
          break;
        }
      }

      if (fmtpIndex >= 0) {
        const fmtpMatch = String(lines[fmtpIndex] || "").match(/^a=fmtp:(\d+)\s*(.*)$/);
        const currentParams = fmtpMatch ? String(fmtpMatch[2] || "") : "";
        lines[fmtpIndex] = `a=fmtp:${payloadId} ${upsertFmtpParams(currentParams)}`;
        continue;
      }

      let rtpmapIndex = -1;
      for (
        let lineIndex = sectionStart + 1;
        lineIndex < currentSectionEnd;
        lineIndex += 1
      ) {
        if (String(lines[lineIndex] || "").startsWith(`a=rtpmap:${payloadId} `)) {
          rtpmapIndex = lineIndex;
          break;
        }
      }

      const insertIndex = rtpmapIndex >= 0 ? rtpmapIndex + 1 : currentSectionEnd;
      lines.splice(
        insertIndex,
        0,
        `a=fmtp:${payloadId} max-fr=${maxFr};max-fs=${maxFs};x-google-start-bitrate=${
          startBitrateKbps
        };x-google-min-bitrate=${minBitrateKbps};x-google-max-bitrate=${maxBitrateKbps}`
      );
      insertedLines += 1;
      currentSectionEnd += 1;
    }

    return insertedLines;
  };

  const patchNativeScreenShareOfferSdp = (sdp, target) => {
    if (typeof sdp !== "string" || !sdp.trim() || !target) {
      return sdp;
    }

    const hadTrailingNewline = /(?:\r?\n)$/.test(sdp);
    const newline = sdp.includes("\r\n") ? "\r\n" : "\n";
    const lines = sdp.split(/\r?\n/);
    let index = 0;
    while (index < lines.length) {
      if (!String(lines[index] || "").startsWith("m=")) {
        index += 1;
        continue;
      }

      const sectionStart = index;
      index += 1;
      while (index < lines.length && !String(lines[index] || "").startsWith("m=")) {
        index += 1;
      }
      const sectionEnd = index;
      if (String(lines[sectionStart] || "").startsWith("m=video")) {
        const delta = patchVideoSectionFmtp(lines, sectionStart, sectionEnd, target);
        index += delta;
      }
    }

    const rebuilt = lines.join(newline) + (hadTrailingNewline ? newline : "");
    return rebuilt;
  };

  const collectVideoSdpDiagnostics = sdp => {
    if (typeof sdp !== "string" || !sdp.trim()) {
      return {
        maxFr: [],
        maxFs: [],
        xGoogleStartBitrate: [],
        xGoogleMinBitrate: [],
        xGoogleMaxBitrate: [],
      };
    }

    const diagnostics = {
      maxFr: [],
      maxFs: [],
      xGoogleStartBitrate: [],
      xGoogleMinBitrate: [],
      xGoogleMaxBitrate: [],
    };
    const pushUniqueNumber = (list, value) => {
      const numeric = Number(value);
      if (!Number.isFinite(numeric) || numeric <= 0) return;
      if (!list.includes(numeric)) {
        list.push(numeric);
      }
    };

    const lines = sdp.split(/\r?\n/);
    let inVideoSection = false;
    for (const rawLine of lines) {
      const line = String(rawLine || "").trim();
      if (!line) continue;
      if (line.startsWith("m=")) {
        inVideoSection = line.startsWith("m=video");
        continue;
      }
      if (!inVideoSection || !line.startsWith("a=fmtp:")) continue;
      const fmtp = line.slice("a=fmtp:".length);
      const spaceIndex = fmtp.indexOf(" ");
      if (spaceIndex < 0) continue;
      const parameterBlob = fmtp.slice(spaceIndex + 1);
      for (const token of parameterBlob.split(";")) {
        const [rawKey, rawValue] = token.split("=");
        const key = String(rawKey || "").trim().toLowerCase();
        const value = String(rawValue || "").trim();
        if (!key || !value) continue;
        if (key === "max-fr") pushUniqueNumber(diagnostics.maxFr, value);
        else if (key === "max-fs") pushUniqueNumber(diagnostics.maxFs, value);
        else if (key === "x-google-start-bitrate")
          pushUniqueNumber(diagnostics.xGoogleStartBitrate, value);
        else if (key === "x-google-min-bitrate")
          pushUniqueNumber(diagnostics.xGoogleMinBitrate, value);
        else if (key === "x-google-max-bitrate")
          pushUniqueNumber(diagnostics.xGoogleMaxBitrate, value);
      }
    }

    diagnostics.maxFr.sort((left, right) => left - right);
    diagnostics.maxFs.sort((left, right) => left - right);
    diagnostics.xGoogleStartBitrate.sort((left, right) => left - right);
    diagnostics.xGoogleMinBitrate.sort((left, right) => left - right);
    diagnostics.xGoogleMaxBitrate.sort((left, right) => left - right);
    return diagnostics;
  };

  const installGoLiveQualityPatch = () => {
    if (state.goLiveQualityPatchReady || !isDiscordHost()) return;
    if (window.__EQUIRUST_GOLIVE_PATCH_INSTALLING__) return;
    window.__EQUIRUST_GOLIVE_PATCH_INSTALLING__ = true;

    const tryInstall = () => {
      let hasPatch = window.__EQUIRUST_GOLIVE_WEBPACK_PATCHED__ === true;
      const addPatch = window.Vencord?.Plugins?.addPatch;
      if (!hasPatch && typeof addPatch === "function") {
        try {
          addPatch(
            {
              find: "this.getDefaultGoliveQuality()",
              replacement: {
                match: /this\.getDefaultGoliveQuality\(\)/g,
                replace:
                  "window.__EQUIRUST_PATCH_STREAM_QUALITY__(this.getDefaultGoliveQuality())",
              },
            },
            "EquirustNativeGoLive",
            "window"
          );
          window.__EQUIRUST_GOLIVE_WEBPACK_PATCHED__ = true;
          hasPatch = true;
          report("desktop_stream_golive_webpack_patch_added=true");
        } catch (error) {
          report(
            `desktop_stream_golive_webpack_patch_failed=${
              error && error.message ? error.message : String(error)
            }`,
            { force: true }
          );
        }
      }

      let hasMethodWrap = false;
      try {
        const wrappedMethodLabels = [];
        const seenOwners = new WeakSet();
        const discoveryErrors = [];
        const methodNames = [
          "getDefaultGoliveQuality",
          "getDefaultGoLiveQuality",
          "getGoLiveQuality",
        ];
        const pushDiscoveryError = (stage, error) => {
          if (discoveryErrors.length >= 8) return;
          discoveryErrors.push({
            stage,
            error: error && error.message ? error.message : String(error),
          });
        };
        const safeGet = (target, key, stage) => {
          if (!target || (typeof target !== "object" && typeof target !== "function")) {
            return undefined;
          }
          try {
            return Reflect.get(target, key);
          } catch (error) {
            pushDiscoveryError(stage || `get:${String(key)}`, error);
            return undefined;
          }
        };
        const safeCall = (target, key, args, stage) => {
          const fn = safeGet(target, key, stage || `call:${String(key)}`);
          if (typeof fn !== "function") return undefined;
          try {
            return fn.apply(target, Array.isArray(args) ? args : []);
          } catch (error) {
            pushDiscoveryError(stage || `call:${String(key)}`, error);
            return undefined;
          }
        };
        const safeEntries = (target, stage) => {
          if (!target || typeof target !== "object") return [];
          try {
            return Object.entries(target);
          } catch (error) {
            pushDiscoveryError(stage || "entries", error);
            return [];
          }
        };
        const tryWrapOwnerMethod = (owner, label) => {
          if (!owner || (typeof owner !== "object" && typeof owner !== "function")) return false;
          if (seenOwners.has(owner)) return false;

          let patched = false;
          for (const methodName of methodNames) {
            const currentMethod = safeGet(owner, methodName, `${label}.${methodName}.get`);
            if (typeof currentMethod !== "function") continue;
            const patchMarker = `__EQUIRUST_GOLIVE_METHOD_PATCHED_${methodName}__`;
            if (safeGet(owner, patchMarker, `${label}.${patchMarker}`) === true) {
              patched = true;
              wrappedMethodLabels.push(`${label}.${methodName}`);
              continue;
            }

            const originalMethod = currentMethod;
            try {
              owner[methodName] = function(...args) {
                const result = originalMethod.apply(this, args);
                return window.__EQUIRUST_PATCH_STREAM_QUALITY__(result);
              };
            } catch (error) {
              pushDiscoveryError(`${label}.${methodName}.assign`, error);
              continue;
            }
            try {
              Object.defineProperty(owner, patchMarker, {
                value: true,
                configurable: false,
                enumerable: false,
                writable: false,
              });
            } catch (error) {
              pushDiscoveryError(`${label}.${methodName}.defineMarker`, error);
            }
            patched = true;
            wrappedMethodLabels.push(`${label}.${methodName}`);
          }

          if (patched) {
            seenOwners.add(owner);
          }
          return patched;
        };

        const common = safeGet(window.Vencord?.Webpack, "Common", "Webpack.Common");
        tryWrapOwnerMethod(safeGet(common, "MediaEngineStore", "Common.MediaEngineStore"), "Common.MediaEngineStore");
        tryWrapOwnerMethod(
          safeCall(
            safeGet(common, "MediaEngineStore", "Common.MediaEngineStore"),
            "getMediaEngine",
            [],
            "Common.MediaEngineStore.getMediaEngine()"
          ),
          "Common.MediaEngineStore.getMediaEngine()"
        );

        if (common && typeof common === "object") {
          for (const [key, value] of safeEntries(common, "Common.entries")) {
            if (wrappedMethodLabels.length >= 16) break;
            tryWrapOwnerMethod(value, `Common.${key}`);
            tryWrapOwnerMethod(safeGet(value, "default", `Common.${key}.default`), `Common.${key}.default`);
            if (typeof safeGet(value, "getMediaEngine", `Common.${key}.getMediaEngine`) === "function") {
              tryWrapOwnerMethod(
                safeCall(value, "getMediaEngine", [], `Common.${key}.getMediaEngine()`),
                `Common.${key}.getMediaEngine()`
              );
            }
          }
        }

        const findAll = safeGet(window.Vencord?.Webpack, "findAll", "Webpack.findAll");
        if (typeof findAll === "function" && wrappedMethodLabels.length < 16) {
          let discovered = [];
          try {
            discovered = findAll(moduleExport => {
              if (!moduleExport) return false;
              try {
                return methodNames.some(methodName => {
                  const direct = safeGet(moduleExport, methodName);
                  if (typeof direct === "function") return true;
                  const defaultExport = safeGet(moduleExport, "default");
                  return typeof safeGet(defaultExport, methodName) === "function";
                });
              } catch {
                return false;
              }
            });
          } catch (error) {
            pushDiscoveryError("Webpack.findAll.invoke", error);
          }

          if (Array.isArray(discovered)) {
            discovered.slice(0, 20).forEach((moduleExport, index) => {
              if (wrappedMethodLabels.length >= 16) return;
              tryWrapOwnerMethod(moduleExport, `Webpack.${index}`);
              tryWrapOwnerMethod(
                safeGet(moduleExport, "default", `Webpack.${index}.default`),
                `Webpack.${index}.default`
              );
            });
          }
        }

        hasMethodWrap = wrappedMethodLabels.length > 0;
        if (hasMethodWrap || discoveryErrors.length) {
          report(
            "desktop_stream_golive_method_patch_added=" +
              JSON.stringify({
                count: wrappedMethodLabels.length,
                labels: wrappedMethodLabels.slice(0, 12),
                discoveryErrors,
              })
          );
        }
      } catch (error) {
        report(
          `desktop_stream_golive_method_patch_failed=${
            error && error.message ? error.message : String(error)
          }`,
          { force: true }
        );
      }

      if (!hasPatch && !hasMethodWrap) {
        return false;
      }

      state.goLiveQualityPatchReady = true;
      report(
        "desktop_stream_golive_patch_installed=" +
          JSON.stringify({
            webpackPatch: hasPatch,
            methodWrap: hasMethodWrap,
          })
      );
      return true;
    };

    if (tryInstall()) {
      window.__EQUIRUST_GOLIVE_PATCH_INSTALLING__ = false;
      return;
    }

    let attempts = 0;
    const retryTimer = window.setInterval(() => {
      attempts += 1;
      if (tryInstall() || attempts >= 120) {
        window.clearInterval(retryTimer);
        if (!state.goLiveQualityPatchReady) {
          report("desktop_stream_golive_patch_timeout=true", { force: true });
        }
        window.__EQUIRUST_GOLIVE_PATCH_INSTALLING__ = false;
      }
    }, 250);
  };

  let goLiveDispatchPatchInstalled = false;
  let goLiveDispatchPatchInstalling = false;
  const installGoLiveDispatchPatch = () => {
    if (goLiveDispatchPatchInstalled || !isDiscordHost()) return;
    if (goLiveDispatchPatchInstalling) return;
    goLiveDispatchPatchInstalling = true;

    const tryInstall = () => {
      const dispatcher = window.Vencord?.Webpack?.Common?.FluxDispatcher;
      const dispatch = dispatcher?.dispatch;
      if (!dispatcher || typeof dispatch !== "function") {
        return false;
      }
      if (dispatcher.__EQUIRUST_GOLIVE_DISPATCH_PATCHED__ === true) {
        goLiveDispatchPatchInstalled = true;
        goLiveDispatchPatchInstalling = false;
        return true;
      }

      const loggedTypes = new Set();
      try {
        Object.defineProperty(dispatcher, "__EQUIRUST_GOLIVE_DISPATCH_PATCHED__", {
          value: true,
          configurable: false,
          enumerable: false,
          writable: false,
        });
      } catch {}

      dispatcher.dispatch = function(action, ...args) {
        try {
          const type = String(action?.type || "");
          if (
            action &&
            typeof action === "object" &&
            /STREAM|GO_LIVE|GOLIVE|BROADCAST|RTC_|VIDEO_QUALITY|MEDIA_ENGINE/i.test(type)
          ) {
            const quality =
              (state.pendingScreenShareQuality && typeof state.pendingScreenShareQuality === "object"
                ? state.pendingScreenShareQuality
                : window.__EQUIRUST_PENDING_NATIVE_SHARE_QUALITY__) || readScreenShareQuality();
            const stateHits = patchGoLiveQualityStateTree(action, quality, {
              maxDepth: 6,
              maxNodes: 320,
            });
            const optionHits = isFakeNitroStreamQualityBypassEnabled()
              ? patchGoLiveOptionAvailabilityTree(action, {
                  maxDepth: 6,
                  maxNodes: 320,
                })
              : 0;
            if ((stateHits > 0 || optionHits > 0) && !loggedTypes.has(type)) {
              loggedTypes.add(type);
              report(
                "desktop_stream_golive_dispatch_patch=" +
                  JSON.stringify({
                    type,
                    stateHits,
                    optionHits,
                  })
              );
            }
          }
        } catch (error) {
          report(
            `desktop_stream_golive_dispatch_patch_failed=${
              error && error.message ? error.message : String(error)
            }`,
            { force: true }
          );
        }
        return dispatch.apply(this, [action, ...args]);
      };

      goLiveDispatchPatchInstalled = true;
      goLiveDispatchPatchInstalling = false;
      report("desktop_stream_golive_dispatch_patch_installed=true");
      return true;
    };

    if (tryInstall()) {
      return;
    }

    let attempts = 0;
    const retryTimer = window.setInterval(() => {
      attempts += 1;
      if (tryInstall() || attempts >= 120) {
        window.clearInterval(retryTimer);
        if (!goLiveDispatchPatchInstalled) {
          report("desktop_stream_golive_dispatch_patch_timeout=true", { force: true });
        }
        goLiveDispatchPatchInstalling = false;
      }
    }, 250);
  };

  const installNativeScreenShareSdpPatch = () => {
    if (state.nativeSdpPatchReady) return;
    if (typeof window.RTCPeerConnection !== "function") return;
    if (!supportsNativeWindowsScreenShare()) return;

    const proto = window.RTCPeerConnection.prototype;
    if (proto.__EQUIRUST_NATIVE_SDP_PATCHED__) {
      state.nativeSdpPatchReady = true;
      return;
    }
    try {
      Object.defineProperty(proto, "__EQUIRUST_NATIVE_SDP_PATCHED__", {
        value: true,
        configurable: false,
        enumerable: false,
        writable: false,
      });

      const originalCreateOffer = proto.createOffer;
      if (typeof originalCreateOffer === "function") {
        proto.createOffer = async function(...args) {
          const offer = await originalCreateOffer.apply(this, args);
          const target = getNativeShareNegotiationTarget(this);
          if (!target || !offer || typeof offer.sdp !== "string") {
            return offer;
          }

          const before = collectVideoSdpDiagnostics(offer.sdp);
          const patchedSdp = patchNativeScreenShareOfferSdp(offer.sdp, target);
          const after = collectVideoSdpDiagnostics(patchedSdp);
          const patched = patchedSdp !== offer.sdp;

          report(
            `desktop_stream_sdp_patch createOffer fps=${target.frameRate} size=${target.width}x${
              target.height
            } patched=${patched} before=${JSON.stringify(before)} after=${JSON.stringify(after)}`
          );
          if (!patched) {
            return offer;
          }
          return {
            type: String(offer.type || "offer"),
            sdp: patchedSdp,
          };
        };
      }

      const originalSetLocalDescription = proto.setLocalDescription;
      if (typeof originalSetLocalDescription === "function") {
        proto.setLocalDescription = function(description, ...args) {
          let nextDescription = description;
          try {
            const type = String(description?.type || "");
            const sdp = description?.sdp;
            if (type === "offer" && typeof sdp === "string" && sdp.length > 0) {
              const target = getNativeShareNegotiationTarget(this);
              if (target) {
                const before = collectVideoSdpDiagnostics(sdp);
                const patchedSdp = patchNativeScreenShareOfferSdp(sdp, target);
                const after = collectVideoSdpDiagnostics(patchedSdp);
                if (patchedSdp !== sdp) {
                  nextDescription = {
                    type,
                    sdp: patchedSdp,
                  };
                }
                report(
                  `desktop_stream_sdp_patch setLocalDescription fps=${target.frameRate} size=${
                    target.width
                  }x${target.height} patched=${patchedSdp !== sdp} before=${
                    JSON.stringify(before)
                  } after=${JSON.stringify(after)}`
                );
              }
            }
          } catch (error) {
            report(
              `desktop_stream_sdp_patch_failed=${
                error && error.message ? error.message : String(error)
              }`,
              { force: true }
            );
          }

          return originalSetLocalDescription.call(this, nextDescription, ...args);
        };
      }

      const originalSetRemoteDescription = proto.setRemoteDescription;
      if (typeof originalSetRemoteDescription === "function") {
        proto.setRemoteDescription = function(description, ...args) {
          let nextDescription = description;
          try {
            const type = String(description?.type || "");
            const sdp = description?.sdp;
            if (
              (type === "answer" || type === "pranswer") &&
              typeof sdp === "string" &&
              sdp.length > 0
            ) {
              const target = getNativeShareNegotiationTarget(this);
              if (target) {
                const before = collectVideoSdpDiagnostics(sdp);
                const patchedSdp = patchNativeScreenShareOfferSdp(sdp, target);
                const after = collectVideoSdpDiagnostics(patchedSdp);
                if (patchedSdp !== sdp) {
                  nextDescription = {
                    type,
                    sdp: patchedSdp,
                  };
                }
                report(
                  `desktop_stream_sdp_patch setRemoteDescription fps=${target.frameRate} size=${
                    target.width
                  }x${target.height} patched=${patchedSdp !== sdp} before=${
                    JSON.stringify(before)
                  } after=${JSON.stringify(after)}`
                );
              }
            }
          } catch (error) {
            report(
              `desktop_stream_sdp_patch_remote_failed=${
                error && error.message ? error.message : String(error)
              }`,
              { force: true }
            );
          }

          return originalSetRemoteDescription.call(this, nextDescription, ...args);
        };
      }

      state.nativeSdpPatchReady = true;
      report("desktop_stream_sdp_patch_installed=true");
    } catch (error) {
      state.nativeSdpPatchReady = false;
      report(
        `desktop_stream_sdp_patch_install_failed=${
          error && error.message ? error.message : String(error)
        }`,
        { force: true }
      );
    }
  };

  const installNativeScreenShareAbr = () => {
    if (state.nativeAbrReady) return;
    if (typeof window.RTCPeerConnection !== "function") return;
    if (!supportsNativeWindowsScreenShare()) return;

    const proto = window.RTCPeerConnection.prototype;
    if (proto.__EQUIRUST_NATIVE_ABR_PATCHED__) {
      state.nativeAbrReady = true;
      return;
    }
    try {
      Object.defineProperty(proto, "__EQUIRUST_NATIVE_ABR_PATCHED__", {
        value: true,
        configurable: false,
        enumerable: false,
        writable: false,
      });

      const maybeAttachController = (peerConnection, sender, track) => {
        if (!peerConnection || !sender) return;
        const senderTrack = track || sender.track;
        if (!senderTrack || senderTrack.kind !== "video") return;
        const meta = desktopStreamTrackMeta.get(senderTrack) || desktopStreamSenderPendingMeta.get(sender);
        if (!meta) return;
        if (senderTrack.readyState && senderTrack.readyState !== "live") {
          desktopStreamSenderPendingMeta.delete(sender);
          return;
        }
        if (desktopStreamClosingTracks.has(senderTrack) || isDesktopStreamClosing(meta)) {
          desktopStreamSenderPendingMeta.delete(sender);
          return;
        }
        if (!desktopStreamTrackMeta.get(senderTrack)) {
          desktopStreamTrackMeta.set(senderTrack, meta);
        }
        desktopStreamSenderPendingMeta.delete(sender);
        if (nativeAbrControllers.has(sender)) return;
        const controller = createNativeAbrController(peerConnection, sender, senderTrack, meta);
        nativeAbrControllers.set(sender, controller);
        report(
          `desktop_stream_abr attach_ok session=${meta.sessionId} source=${meta.sourceId} fps=${
            meta.baseFrameRate
          }`
        );
      };
      const ensureControllersForPeerConnection = peerConnection => {
        if (!peerConnection || typeof peerConnection.getSenders !== "function") return;
        for (const sender of peerConnection.getSenders()) {
          if (!sender) continue;
          if (!desktopStreamSenderPeerConnection.get(sender)) {
            desktopStreamSenderPeerConnection.set(sender, peerConnection);
          }
          const senderTrack = sender.track;
          if (senderTrack && senderTrack.kind === "video") {
            maybeAttachController(peerConnection, sender, senderTrack);
          }
        }
      };

      const originalAddTrack = proto.addTrack;
      if (typeof originalAddTrack === "function") {
        proto.addTrack = function(track, ...streams) {
          const sender = originalAddTrack.call(this, track, ...streams);
          try {
            if (sender && typeof sender === "object") {
              desktopStreamSenderPeerConnection.set(sender, this);
            }
            const streamMeta = getDesktopStreamMetaFromStreams(streams);
            if (streamMeta && track?.kind === "video" && !desktopStreamTrackMeta.get(track)) {
              desktopStreamTrackMeta.set(track, streamMeta);
            }
            maybeAttachController(this, sender, track);
          } catch (error) {
            report(
              `desktop_stream_abr attach_failed=${
                error && error.message ? error.message : String(error)
              }`,
              { force: true }
            );
          }
          return sender;
        };
      }

      const originalAddTransceiver = proto.addTransceiver;
      if (typeof originalAddTransceiver === "function") {
        proto.addTransceiver = function(trackOrKind, init) {
          const transceiver = originalAddTransceiver.call(this, trackOrKind, init);
          try {
            const sender = transceiver?.sender;
            if (sender) {
              desktopStreamSenderPeerConnection.set(sender, this);
            }
            const streamMeta = getDesktopStreamMetaFromStreams(init?.streams);
            if (trackOrKind && typeof trackOrKind === "object" && trackOrKind.kind === "video") {
              if (streamMeta && !desktopStreamTrackMeta.get(trackOrKind)) {
                desktopStreamTrackMeta.set(trackOrKind, streamMeta);
              }
              maybeAttachController(this, sender, trackOrKind);
            } else if (String(trackOrKind || "").toLowerCase() === "video" && sender && streamMeta) {
              desktopStreamSenderPendingMeta.set(sender, streamMeta);
            }
            ensureControllersForPeerConnection(this);
          } catch (error) {
            report(
              `desktop_stream_abr transceiver_attach_failed=${
                error && error.message ? error.message : String(error)
              }`,
              { force: true }
            );
          }
          return transceiver;
        };
      }
      const originalCreateOffer = proto.createOffer;
      if (typeof originalCreateOffer === "function") {
        proto.createOffer = async function(...args) {
          try {
            ensureControllersForPeerConnection(this);
          } catch {}
          return originalCreateOffer.apply(this, args);
        };
      }
      const originalSetLocalDescription = proto.setLocalDescription;
      if (typeof originalSetLocalDescription === "function") {
        proto.setLocalDescription = function(description, ...args) {
          try {
            ensureControllersForPeerConnection(this);
          } catch {}
          return originalSetLocalDescription.call(this, description, ...args);
        };
      }

      const senderProto = window.RTCRtpSender?.prototype;
      if (
        senderProto &&
        typeof senderProto.replaceTrack === "function" &&
        !senderProto.__EQUIRUST_NATIVE_ABR_REPLACETRACK_PATCHED__
      ) {
        Object.defineProperty(senderProto, "__EQUIRUST_NATIVE_ABR_REPLACETRACK_PATCHED__", {
          value: true,
          configurable: false,
          enumerable: false,
          writable: false,
        });
        const originalReplaceTrack = senderProto.replaceTrack;
        senderProto.replaceTrack = function(track, ...args) {
          try {
            if (track && track.kind === "video") {
              const pendingMeta = desktopStreamSenderPendingMeta.get(this);
              if (pendingMeta && !desktopStreamTrackMeta.get(track)) {
                desktopStreamTrackMeta.set(track, pendingMeta);
              }
              const peerConnection = desktopStreamSenderPeerConnection.get(this);
              if (peerConnection) {
                maybeAttachController(peerConnection, this, track);
              }
            }
          } catch (error) {
            report(
              `desktop_stream_abr replace_track_attach_failed=${
                error && error.message ? error.message : String(error)
              }`,
              { force: true }
            );
          }
          return originalReplaceTrack.call(this, track, ...args);
        };
      }
      if (
        senderProto &&
        typeof senderProto.setParameters === "function" &&
        !senderProto.__EQUIRUST_NATIVE_ABR_SETPARAMETERS_PATCHED__
      ) {
        Object.defineProperty(senderProto, "__EQUIRUST_NATIVE_ABR_SETPARAMETERS_PATCHED__", {
          value: true,
          configurable: false,
          enumerable: false,
          writable: false,
        });
        const originalSetParameters = senderProto.setParameters;
        senderProto.setParameters = function(parameters, ...args) {
          let nextParameters =
            parameters && typeof parameters === "object" ? { ...parameters } : {};
          try {
            const meta =
              desktopStreamTrackMeta.get(this?.track) || desktopStreamSenderPendingMeta.get(this);
            if (meta) {
              const requestedFrameRate = Math.max(
                1,
                Math.round(Number(meta?.requestedFrameRate || meta?.baseFrameRate || 60) || 60)
              );
              const requestedWidth = Math.max(
                2,
                Math.round(Number(meta?.baseWidth || 1920) || 1920)
              );
              const requestedHeight = Math.max(
                2,
                Math.round(Number(meta?.baseHeight || 1080) || 1080)
              );
              const negotiationHints = desktopStreamSenderNegotiationHints.get(this) || null;
              const hintFrameRateCap = Number(negotiationHints?.frameRateCap || 0);
              const desiredMaxFramerate =
                Number.isFinite(hintFrameRateCap) && hintFrameRateCap > 0
                  ? Math.max(1, Math.min(requestedFrameRate, Math.round(hintFrameRateCap)))
                  : requestedFrameRate;
              const encodings =
                Array.isArray(nextParameters.encodings) && nextParameters.encodings.length
                  ? nextParameters.encodings.map(encoding => ({ ...(encoding || {}) }))
                  : [{}];
              const primaryEncoding = encodings[0] || {};
              primaryEncoding.maxFramerate = desiredMaxFramerate;
              const hintScaleDownBy = Number(negotiationHints?.scaleResolutionDownBy || 0);
              if (Number.isFinite(hintScaleDownBy) && hintScaleDownBy > 1) {
                const currentScaleDownBy = Number(primaryEncoding.scaleResolutionDownBy || 0);
                primaryEncoding.scaleResolutionDownBy = Math.max(
                  1,
                  hintScaleDownBy,
                  Number.isFinite(currentScaleDownBy) && currentScaleDownBy > 0
                    ? currentScaleDownBy
                    : 1
                );
              }
              const hintWidthCap = Number(negotiationHints?.widthCap || 0);
              const hintHeightCap = Number(negotiationHints?.heightCap || 0);
              if (
                Number.isFinite(hintWidthCap) &&
                Number.isFinite(hintHeightCap) &&
                hintWidthCap > 0 &&
                hintHeightCap > 0
              ) {
                const capScaleDownBy = Math.max(
                  1,
                  requestedWidth / Math.max(2, Math.round(hintWidthCap)),
                  requestedHeight / Math.max(2, Math.round(hintHeightCap))
                );
                const currentScaleDownBy = Number(primaryEncoding.scaleResolutionDownBy || 0);
                primaryEncoding.scaleResolutionDownBy = Math.max(
                  capScaleDownBy,
                  Number.isFinite(currentScaleDownBy) && currentScaleDownBy > 0
                    ? currentScaleDownBy
                    : 1
                );
              }
              primaryEncoding.active ??= true;
              encodings[0] = primaryEncoding;
              nextParameters.encodings = encodings;
            }
          } catch (error) {
            report(
              `desktop_stream_abr set_parameters_guard_failed=${
                error && error.message ? error.message : String(error)
              }`,
              { force: true }
            );
          }
          return originalSetParameters.call(this, nextParameters, ...args);
        };
      }

      state.nativeAbrReady = true;
      report("desktop_stream_abr_installed=true");
    } catch (error) {
      state.nativeAbrReady = false;
      report(
        `desktop_stream_abr_install_failed=${
          error && error.message ? error.message : String(error)
        }`,
        { force: true }
      );
    }
  };

  const ensureNativeSurfaceStyles = () => {
    if (document.getElementById("equirust-surface-style")) {
      return;
    }

    const style = document.createElement("style");
    style.id = "equirust-surface-style";
    style.textContent = `
      :root {
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
      }
      .equirust-surface-backdrop {
        position: fixed;
        inset: 0;
        z-index: 2147483646;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 28px;
        background: var(--equirust-surface-backdrop);
        backdrop-filter: blur(18px);
      }
      .equirust-surface-dialog {
        color: var(--equirust-surface-fg);
        background: var(--equirust-surface-shell);
        border: 1px solid var(--equirust-surface-border);
        border-radius: var(--equirust-surface-radius);
        box-shadow: var(--equirust-surface-shadow);
      }
      .equirust-surface-panel {
        background: var(--equirust-surface-shell-alt);
        border-left: 1px solid var(--equirust-surface-border);
      }
      .equirust-surface-header {
        padding: 18px 20px 16px;
        border-bottom: 1px solid var(--equirust-surface-border);
      }
      .equirust-surface-eyebrow {
        margin: 0 0 6px;
        color: var(--equirust-surface-muted);
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      .equirust-surface-title {
        margin: 0;
        font-size: 22px;
        line-height: 1.1;
        font-weight: 800;
      }
      .equirust-surface-copy {
        margin: 8px 0 0;
        color: var(--equirust-surface-muted);
        font-size: 14px;
        line-height: 1.45;
      }
      .equirust-surface-footer {
        margin-top: auto;
        padding: 16px 18px 18px;
        border-top: 1px solid var(--equirust-surface-border);
        display: flex;
        justify-content: flex-end;
        gap: 10px;
      }
      .equirust-surface-button {
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
      }
      .equirust-surface-button:hover {
        filter: brightness(1.05);
        transform: translateY(-1px);
      }
      .equirust-surface-button:disabled {
        opacity: 0.45;
        cursor: default;
        transform: none;
        filter: none;
      }
      .equirust-surface-button--secondary {
        background: var(--background-modifier-hover, rgba(255,255,255,0.08));
        color: var(--equirust-surface-fg);
      }
      .equirust-surface-button--primary {
        background: var(--equirust-surface-accent);
        color: white;
      }
    `;
    document.documentElement.appendChild(style);
  };

  const ensureScreenSharePickerStyles = () => {
    ensureNativeSurfaceStyles();
    if (document.getElementById("equirust-screenshare-style")) {
      return;
    }

    const style = document.createElement("style");
    style.id = "equirust-screenshare-style";
    style.textContent = `
      .equirust-screenshare.equirust-surface-backdrop {
        --equirust-picker-safe-top: calc(var(--equirust-titlebar-height, 0px) + 6px);
        inset: var(--equirust-picker-safe-top) 0 0 0;
        z-index: 2147483645;
        padding: clamp(12px, 2vh, 18px) 18px 18px;
        align-items: center;
        justify-content: center;
        backdrop-filter: none;
        background: rgba(5, 8, 15, 0.46);
      }
      .equirust-screenshare__dialog {
        width: min(1040px, calc(100vw - 56px));
        height: min(670px, calc(100vh - var(--equirust-picker-safe-top) - 32px));
        max-height: calc(100vh - var(--equirust-picker-safe-top) - 32px);
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(300px, 324px);
        overflow: hidden;
        color: var(--header-primary, #fff);
        border-radius: 18px;
        border: 1px solid rgba(255, 255, 255, 0.08);
        box-shadow: 0 28px 72px rgba(0, 0, 0, 0.42);
      }
      .equirust-screenshare__main {
        min-width: 0;
        min-height: 0;
        display: flex;
        flex-direction: column;
        contain: layout paint style;
        background:
          radial-gradient(circle at top right, rgba(88, 101, 242, 0.12), transparent 34%),
          linear-gradient(180deg, color-mix(in srgb, var(--background-primary, #11141b) 94%, white 2%), var(--background-primary, #11141b));
      }
      .equirust-screenshare__sidebar {
        min-width: 0;
        min-height: 0;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        contain: layout paint style;
        background: color-mix(in srgb, var(--background-secondary, #171b27) 92%, black 8%);
        border-left: 1px solid rgba(255, 255, 255, 0.06);
      }
      .equirust-screenshare__header {
        gap: 10px;
      }
      .equirust-screenshare__headerTop {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
      }
      .equirust-screenshare__headerCopy {
        min-width: 0;
        display: grid;
        gap: 6px;
      }
      .equirust-screenshare__dismiss {
        flex: 0 0 auto;
        width: 34px;
        height: 34px;
        border: 0;
        border-radius: 10px;
        background: rgba(255, 255, 255, 0.06);
        color: var(--header-secondary, rgba(255,255,255,0.74));
        font-size: 20px;
        line-height: 1;
        cursor: pointer;
      }
      .equirust-screenshare__dismiss:hover {
        background: rgba(255, 255, 255, 0.1);
        color: var(--header-primary, #fff);
      }
      .equirust-screenshare__tabs {
        display: inline-flex;
        gap: 8px;
        margin-top: 2px;
        flex-wrap: wrap;
      }
      .equirust-screenshare__tab {
        border: 0;
        border-radius: 999px;
        min-width: 156px;
        min-height: 40px;
        padding: 8px 16px;
        background: var(--background-secondary-alt, rgba(255,255,255,0.06));
        color: var(--header-secondary, rgba(255,255,255,0.7));
        font-size: 13px;
        font-weight: 700;
        cursor: pointer;
        transition: background-color 140ms ease, color 140ms ease, transform 140ms ease;
      }
      .equirust-screenshare__tab[data-active="true"] {
        background: var(--brand-experiment, #5865f2);
        color: white;
        transform: translateY(-1px);
      }
      .equirust-screenshare__toolbar {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-top: 0;
        flex-wrap: wrap;
      }
      .equirust-screenshare__sourceCount {
        color: var(--text-muted, rgba(255,255,255,0.62));
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        white-space: nowrap;
        margin-right: auto;
      }
      .equirust-screenshare__search {
        min-width: 0;
        flex: 1 1 280px;
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 8px;
      }
      .equirust-screenshare__searchInput {
        width: 100%;
        min-height: 38px;
        border-radius: 11px;
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        background: var(--background-tertiary, #0f121a);
        color: var(--header-primary, #fff);
        padding: 0 12px;
        font: inherit;
      }
      .equirust-screenshare__searchInput::placeholder {
        color: var(--text-muted, rgba(255,255,255,0.52));
      }
      .equirust-screenshare__searchClear {
        min-width: 74px;
        min-height: 38px;
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        border-radius: 11px;
        background: var(--background-secondary-alt, rgba(255,255,255,0.06));
        color: var(--header-secondary, rgba(255,255,255,0.72));
        font-size: 12px;
        font-weight: 700;
        cursor: pointer;
      }
      .equirust-screenshare__searchClear:disabled {
        opacity: 0.45;
        cursor: default;
      }
      .equirust-screenshare__grid {
        padding: 14px 16px 16px;
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(min(100%, 208px), 1fr));
        gap: 12px;
        overflow: auto;
        align-content: start;
        contain: layout paint;
        scrollbar-gutter: stable both-edges;
        scrollbar-width: thin;
        scrollbar-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 48%, rgba(255,255,255,0.2)) transparent;
      }
      .equirust-screenshare__grid::-webkit-scrollbar {
        width: 11px;
        height: 11px;
      }
      .equirust-screenshare__grid::-webkit-scrollbar-track {
        background: transparent;
      }
      .equirust-screenshare__grid::-webkit-scrollbar-thumb {
        border-radius: 999px;
        border: 2px solid transparent;
        background-clip: padding-box;
        background: color-mix(in srgb, var(--brand-experiment, #5865f2) 42%, rgba(255,255,255,0.2));
      }
      .equirust-screenshare__grid::-webkit-scrollbar-thumb:hover {
        background: color-mix(in srgb, var(--brand-experiment, #5865f2) 62%, rgba(255,255,255,0.22));
      }
      .equirust-screenshare__card {
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        border-radius: 14px;
        overflow: hidden;
        background: var(--background-secondary, #171b27);
        cursor: pointer;
        transition: transform 140ms ease, border-color 140ms ease, background-color 140ms ease, box-shadow 140ms ease;
        text-align: left;
        display: grid;
        grid-template-rows: auto minmax(0, 1fr);
        contain: layout paint;
      }
      .equirust-screenshare__card:hover {
        transform: translateY(-1px);
        border-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 48%, rgba(255,255,255,0.12));
      }
      .equirust-screenshare__card[data-selected="true"] {
        border-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 78%, white 8%);
        background: color-mix(in srgb, var(--background-secondary, #171b27) 90%, var(--brand-experiment, #5865f2) 10%);
        box-shadow: 0 0 0 1px color-mix(in srgb, var(--brand-experiment, #5865f2) 55%, transparent);
      }
      .equirust-screenshare__cardPreview {
        aspect-ratio: 16 / 10;
        width: 100%;
        object-fit: cover;
        display: block;
        background:
          linear-gradient(135deg, rgba(88, 101, 242, 0.16), transparent),
          var(--background-tertiary, #0f121a);
      }
      .equirust-screenshare__cardMeta {
        display: grid;
        gap: 7px;
        padding: 14px 16px 18px;
        min-height: 88px;
        align-content: start;
      }
      .equirust-screenshare__cardName {
        font-size: 13px;
        font-weight: 700;
        line-height: 1.3;
        color: var(--header-primary, #fff);
        display: -webkit-box;
        -webkit-line-clamp: 3;
        -webkit-box-orient: vertical;
        overflow: hidden;
        word-break: break-word;
      }
      .equirust-screenshare__cardKind {
        color: var(--text-muted, rgba(255,255,255,0.62));
        font-size: 12px;
        line-height: 1.4;
        padding-bottom: 2px;
        word-break: break-word;
      }
      .equirust-screenshare__empty {
        padding: 30px 20px 24px;
        color: var(--text-muted, rgba(255,255,255,0.62));
        font-size: 14px;
      }
      .equirust-screenshare__previewWrap {
        padding: 12px 14px 8px;
      }
      .equirust-screenshare__preview {
        width: 100%;
        aspect-ratio: 16 / 9;
        border-radius: 12px;
        overflow: hidden;
        background:
          linear-gradient(135deg, rgba(88, 101, 242, 0.16), transparent),
          var(--background-tertiary, #0f121a);
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
      }
      .equirust-screenshare__preview img {
        width: 100%;
        height: 100%;
        object-fit: cover;
        display: block;
      }
      .equirust-screenshare__previewLabel {
        margin-top: 8px;
        font-size: 14px;
        font-weight: 700;
        line-height: 1.3;
      }
      .equirust-screenshare__summary {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        margin-top: 8px;
      }
      .equirust-screenshare__pill {
        display: inline-flex;
        align-items: center;
        min-height: 24px;
        padding: 0 10px;
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.06);
        border: 1px solid rgba(255, 255, 255, 0.06);
        color: var(--header-secondary, rgba(255,255,255,0.76));
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }
      .equirust-screenshare__controls {
        padding: 0 14px 14px;
        display: grid;
        gap: 9px;
        min-height: 0;
        overflow-y: auto;
        overflow-x: hidden;
        scrollbar-gutter: stable;
        scrollbar-width: thin;
        scrollbar-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 48%, rgba(255,255,255,0.2)) transparent;
      }
      .equirust-screenshare__controls::-webkit-scrollbar {
        width: 11px;
        height: 11px;
      }
      .equirust-screenshare__controls::-webkit-scrollbar-track {
        background: transparent;
      }
      .equirust-screenshare__controls::-webkit-scrollbar-thumb {
        border-radius: 999px;
        border: 2px solid transparent;
        background-clip: padding-box;
        background: color-mix(in srgb, var(--brand-experiment, #5865f2) 42%, rgba(255,255,255,0.2));
      }
      .equirust-screenshare__controls::-webkit-scrollbar-thumb:hover {
        background: color-mix(in srgb, var(--brand-experiment, #5865f2) 62%, rgba(255,255,255,0.22));
      }
      .equirust-screenshare__field {
        display: grid;
        gap: 6px;
      }
      .equirust-screenshare__fieldLabel {
        color: var(--header-secondary, rgba(255,255,255,0.7));
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }
      .equirust-screenshare__fieldHint {
        color: var(--text-muted, rgba(255,255,255,0.56));
        font-size: 12px;
        line-height: 1.3;
      }
      .equirust-screenshare__optionChips {
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
      }
      .equirust-screenshare__optionChip {
        min-height: 36px;
        padding: 0 12px;
        border-radius: 10px;
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        background: var(--background-tertiary, #0f121a);
        color: var(--header-secondary, rgba(255,255,255,0.74));
        font: inherit;
        font-size: 13px;
        font-weight: 700;
        cursor: pointer;
        transition: background-color 140ms ease, border-color 140ms ease, color 140ms ease;
      }
      .equirust-screenshare__optionChip[data-active="true"] {
        color: white;
        border-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 65%, white 6%);
        background: color-mix(in srgb, var(--background-tertiary, #0f121a) 72%, var(--brand-experiment, #5865f2) 28%);
      }
      .equirust-screenshare__optionChip[data-recommended="true"]::after {
        content: " ★";
        color: #8fd3ff;
      }
      .equirust-screenshare__select,
      .equirust-screenshare__toggle {
        width: 100%;
        min-height: 40px;
        border-radius: 11px;
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        background: var(--background-tertiary, #0f121a);
        color: var(--header-primary, #fff);
        padding: 0 12px;
        font: inherit;
      }
      .equirust-screenshare__toggle {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        cursor: pointer;
      }
      .equirust-screenshare__toggle input {
        accent-color: var(--brand-experiment, #5865f2);
      }
      .equirust-screenshare__audioNote {
        margin: -4px 2px 0;
        color: var(--text-muted, rgba(255,255,255,0.48));
        font-size: 12px;
        line-height: 1.35;
      }
      .equirust-screenshare__encoder {
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        border-radius: 11px;
        background: color-mix(
          in srgb,
          var(--background-secondary, #171b27) 84%,
          var(--brand-experiment, #5865f2) 16%
        );
        padding: 10px 12px;
        display: grid;
        gap: 4px;
      }
      .equirust-screenshare__encoderMode {
        font-size: 13px;
        font-weight: 800;
        line-height: 1.3;
        color: var(--header-primary, #fff);
      }
      .equirust-screenshare__encoderMode[data-kind="hardware"] {
        color: #7ae38f;
      }
      .equirust-screenshare__encoderMode[data-kind="software"] {
        color: #f0c66e;
      }
      .equirust-screenshare__encoderMode[data-kind="unsupported"] {
        color: #f28888;
      }
      .equirust-screenshare__encoderDetail {
        color: var(--text-muted, rgba(255,255,255,0.68));
        font-size: 12px;
        line-height: 1.3;
      }
      .equirust-screenshare__hintOptions {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 8px;
      }
      .equirust-screenshare__hint {
        min-height: 40px;
        border-radius: 11px;
        border: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
        background: var(--background-tertiary, #0f121a);
        color: var(--header-secondary, rgba(255,255,255,0.72));
        font-size: 13px;
        font-weight: 700;
        cursor: pointer;
      }
      .equirust-screenshare__hint[data-active="true"] {
        border-color: color-mix(in srgb, var(--brand-experiment, #5865f2) 65%, white 6%);
        background: color-mix(in srgb, var(--background-tertiary, #0f121a) 76%, var(--brand-experiment, #5865f2) 24%);
        color: white;
      }
      .equirust-screenshare__footer {
        margin-top: auto;
        border-top: 1px solid var(--background-modifier-accent, rgba(255,255,255,0.08));
      }
      @media (max-width: 980px) {
        .equirust-screenshare__dialog {
          width: min(960px, calc(100vw - 28px));
          grid-template-columns: minmax(0, 1fr) minmax(280px, 300px);
        }
      }
      @media (max-width: 900px) {
        .equirust-screenshare__dialog {
          width: calc(100vw - 24px);
          height: min(720px, calc(100vh - var(--equirust-picker-safe-top) - 18px));
          max-height: calc(100vh - var(--equirust-picker-safe-top) - 18px);
          grid-template-columns: 1fr;
          grid-template-rows: minmax(0, 1fr) auto;
        }
        .equirust-screenshare__sidebar {
          border-left: 0;
          border-top: 1px solid rgba(255, 255, 255, 0.06);
          max-height: min(44vh, 340px);
        }
        .equirust-screenshare__previewWrap {
          display: none;
        }
        .equirust-screenshare__controls {
          padding-top: 12px;
        }
        .equirust-screenshare__grid {
          grid-template-columns: repeat(auto-fill, minmax(min(100%, 192px), 1fr));
        }
      }
      @media (max-width: 760px) {
        .equirust-screenshare__toolbar {
          align-items: stretch;
        }
        .equirust-screenshare__sourceCount {
          margin-right: 0;
        }
        .equirust-screenshare__search {
          width: 100%;
          flex: 1 1 100%;
        }
        .equirust-screenshare__grid {
          grid-template-columns: repeat(auto-fill, minmax(min(100%, 168px), 1fr));
        }
      }
      @media (max-height: 760px) {
        .equirust-screenshare__dialog {
          height: min(620px, calc(100vh - var(--equirust-picker-safe-top) - 18px));
          max-height: calc(100vh - var(--equirust-picker-safe-top) - 18px);
        }
      }
    `;
    document.documentElement.appendChild(style);
  };

  const createAbortError = message => {
    try {
      return new DOMException(message, "AbortError");
    } catch {
      const error = new Error(message);
      error.name = "AbortError";
      return error;
    }
  };

  const escapeHtml = value =>
    String(value ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");

  const cacheGet = (cache, key) => {
    if (!(cache instanceof Map) || key == null) {
      return undefined;
    }
    const cacheKey = String(key);
    if (!cache.has(cacheKey)) {
      return undefined;
    }
    const value = cache.get(cacheKey);
    cache.delete(cacheKey);
    cache.set(cacheKey, value);
    return value;
  };

  const cacheHas = (cache, key) =>
    cache instanceof Map && key != null ? cache.has(String(key)) : false;

  const cacheSet = (cache, key, value, limit) => {
    if (!(cache instanceof Map) || key == null) {
      return value;
    }
    const cacheKey = String(key);
    if (cache.has(cacheKey)) {
      cache.delete(cacheKey);
    }
    cache.set(cacheKey, value);
    const maxEntries = Math.max(0, Number(limit) || 0);
    while (maxEntries > 0 && cache.size > maxEntries) {
      const oldestKey = cache.keys().next().value;
      if (typeof oldestKey !== "string") {
        break;
      }
      cache.delete(oldestKey);
    }
    return value;
  };

  const cachePrune = (cache, allowedKeys, limit) => {
    if (!(cache instanceof Map)) {
      return;
    }
    const allowed =
      Array.isArray(allowedKeys) && allowedKeys.length
        ? new Set(allowedKeys.map(value => String(value)))
        : null;
    if (allowed) {
      Array.from(cache.keys()).forEach(key => {
        if (!allowed.has(key)) {
          cache.delete(key);
        }
      });
    }
    const maxEntries = Math.max(0, Number(limit) || 0);
    while (maxEntries > 0 && cache.size > maxEntries) {
      const oldestKey = cache.keys().next().value;
      if (typeof oldestKey !== "string") {
        break;
      }
      cache.delete(oldestKey);
    }
  };

  const describeRuntimeError = error =>
    error && typeof error.message === "string" ? error.message : String(error);

  const delayMs = ms =>
    new Promise(resolve => {
      window.setTimeout(resolve, Math.max(0, Number(ms) || 0));
    });

  const waitForCondition = async (check, timeoutMs = 1500, intervalMs = 50) => {
    const deadline = Date.now() + Math.max(0, Number(timeoutMs) || 0);
    while (Date.now() <= deadline) {
      try {
        if (check()) {
          return true;
        }
      } catch {}
      await delayMs(intervalMs);
    }
    try {
      return check();
    } catch {
      return false;
    }
  };

  const DESKTOP_STREAM_GENERATED_TRACK_CACHE_KEY =
    "equirust.desktop_stream.generated_track_capability.v1";

  const currentGeneratedTrackRuntimeFingerprint = () => {
    let brands = "";
    try {
      brands = Array.isArray(navigator.userAgentData?.brands)
        ? navigator.userAgentData.brands
            .map(entry => `${entry?.brand || "unknown"}/${entry?.version || "0"}`)
            .join(",")
        : "";
    } catch {}
    return JSON.stringify({
      host: window.__TAURI_INTERNALS__ ? "tauri" : "browser",
      ua: String(navigator.userAgent || ""),
      brands,
    });
  };

  const readPersistedGeneratedTrackCapability = () => {
    try {
      const raw = window.localStorage?.getItem?.(DESKTOP_STREAM_GENERATED_TRACK_CACHE_KEY);
      if (!raw) {
        return null;
      }
      const parsed = JSON.parse(raw);
      if (
        !parsed ||
        parsed.supported !== true ||
        (parsed.path !== "main" && parsed.path !== "worker") ||
        parsed.fingerprint !== currentGeneratedTrackRuntimeFingerprint()
      ) {
        return null;
      }
      return {
        supported: true,
        path: parsed.path,
        mode: typeof parsed.mode === "string" ? parsed.mode : null,
        reason: typeof parsed.reason === "string" ? parsed.reason : null,
        cachedAt: Math.max(0, Number(parsed.cachedAt || 0) || 0),
      };
    } catch {
      return null;
    }
  };

  const persistGeneratedTrackCapability = probe => {
    try {
      if (!probe?.supported || (probe.path !== "main" && probe.path !== "worker")) {
        window.localStorage?.removeItem?.(DESKTOP_STREAM_GENERATED_TRACK_CACHE_KEY);
        return;
      }
      window.localStorage?.setItem?.(
        DESKTOP_STREAM_GENERATED_TRACK_CACHE_KEY,
        JSON.stringify({
          supported: true,
          path: probe.path,
          mode: typeof probe.mode === "string" ? probe.mode : null,
          reason: typeof probe.reason === "string" ? probe.reason : null,
          cachedAt: Date.now(),
          fingerprint: currentGeneratedTrackRuntimeFingerprint(),
        })
      );
    } catch {}
  };

  const clearPersistedGeneratedTrackCapability = () => {
    try {
      window.localStorage?.removeItem?.(DESKTOP_STREAM_GENERATED_TRACK_CACHE_KEY);
    } catch {}
  };

  const getDesktopStreamRuntimeCapability = () => {
    const isTauriRuntime = Boolean(window.__TAURI_INTERNALS__);
    const cachedCapability = isTauriRuntime ? readPersistedGeneratedTrackCapability() : null;
    const livePath =
      state.nativeGeneratedTrackSupport === true &&
      (state.nativeGeneratedTrackMode === "main" || state.nativeGeneratedTrackMode === "worker")
        ? state.nativeGeneratedTrackMode
        : null;
    const cachedPath =
      cachedCapability?.supported === true &&
      (cachedCapability.path === "main" || cachedCapability.path === "worker")
        ? cachedCapability.path
        : null;
    const generatedTrackPath = livePath || cachedPath || null;
    return {
      isTauriRuntime,
      generatedTrackSupported:
        generatedTrackPath === "main" || generatedTrackPath === "worker",
      generatedTrackPath,
      generatedTrackWorker: generatedTrackPath === "worker",
    };
  };

  const resetGeneratedTrackSupportState = reason => {
    state.nativeGeneratedTrackSupport = null;
    state.nativeGeneratedTrackSupportPromise = null;
    state.nativeGeneratedTrackMode = null;
    state.nativeGeneratedTrackProbeAt = Date.now();
    state.nativeGeneratedTrackProbeReason =
      typeof reason === "string" && reason.trim() ? reason.trim() : null;
    clearPersistedGeneratedTrackCapability();
  };

  const createGeneratedVideoTrackHandle = (options = {}) => {
    const allowInTauri = options?.allowInTauri === true;
    if (window.__TAURI_INTERNALS__ && !allowInTauri) {
      return null;
    }
    const candidates = [
      {
        ctor: typeof window.VideoTrackGenerator === "function" ? window.VideoTrackGenerator : null,
        label: "VideoTrackGenerator",
        args: [],
      },
      {
        ctor:
          typeof window.MediaStreamTrackGenerator === "function"
            ? window.MediaStreamTrackGenerator
            : null,
        label: "MediaStreamTrackGenerator",
        args: [{ kind: "video" }],
      },
    ];

    for (const candidate of candidates) {
      if (typeof candidate.ctor !== "function") continue;
      try {
        const track = new candidate.ctor(...candidate.args);
        const writer =
          track?.writable && typeof track.writable.getWriter === "function"
            ? track.writable.getWriter()
            : null;
        if (!track || !writer) {
          try {
            writer?.releaseLock?.();
          } catch {}
          try {
            track?.stop?.();
          } catch {}
          continue;
        }
        return {
          kind: "main",
          track,
          writer,
          mode: candidate.label,
          close: async () => {
            try {
              await writer?.close?.();
            } catch {}
            try {
              writer?.releaseLock?.();
            } catch {}
            try {
              track?.stop?.();
            } catch {}
          },
        };
      } catch (error) {
        console.warn("[Equirust] Failed to create generated video track", error);
      }
    }

    return null;
  };

  const disposeGeneratedVideoTrackHandle = async handle => {
    if (!handle || typeof handle !== "object") return;
    if (typeof handle.close === "function") {
      await handle.close();
      return;
    }
    try {
      await handle.writer?.close?.();
    } catch {}
    try {
      handle.writer?.releaseLock?.();
    } catch {}
    try {
      handle.track?.stop?.();
    } catch {}
  };

  const getDesktopStreamVideoWorkerUrl = () => {
    if (
      typeof state.desktopStreamVideoWorkerUrl === "string" &&
      state.desktopStreamVideoWorkerUrl
    ) {
      return state.desktopStreamVideoWorkerUrl;
    }

    const source = String.raw`
const delayMs = ms => new Promise(resolve => setTimeout(resolve, Math.max(0, Number(ms) || 0)));
let generator = null;
let outputTrack = null;
let writer = null;
let videoDecoder = null;
let generatedVideoPumpPromise = null;
let generatedVideoDroppedFrames = 0;
let decoderQueueDropCount = 0;
let frameDurationMicros = 16666;
const generatedVideoFrameQueue = [];

const describeWorkerError = error =>
  error && typeof error.message === "string" ? error.message : String(error);

const postEvent = payload => {
  try {
    self.postMessage(payload);
  } catch {}
};

const closeGeneratedVideoFrame = frame => {
  if (!frame || typeof frame.close !== "function") return;
  try {
    frame.close();
  } catch {}
};

const flushGeneratedVideoQueue = () => {
  while (generatedVideoFrameQueue.length) {
    closeGeneratedVideoFrame(generatedVideoFrameQueue.shift());
  }
};

const pumpGeneratedVideoFrames = () => {
  if (!writer || generatedVideoPumpPromise) {
    return;
  }

  generatedVideoPumpPromise = (async () => {
    while (writer && generatedVideoFrameQueue.length) {
      const frame = generatedVideoFrameQueue.shift();
      if (!frame) continue;
      try {
        await writer.write(frame);
      } finally {
        closeGeneratedVideoFrame(frame);
      }
    }
  })()
    .catch(error => {
      postEvent({
        type: "error",
        code: "worker_generator_write_failed",
        message: describeWorkerError(error),
      });
    })
    .finally(() => {
      generatedVideoPumpPromise = null;
      if (writer && generatedVideoFrameQueue.length) {
        pumpGeneratedVideoFrames();
      }
    });
};

const enqueueGeneratedVideoFrame = frame => {
  if (!frame) {
    return;
  }
  if (!writer) {
    closeGeneratedVideoFrame(frame);
    return;
  }

  while (generatedVideoFrameQueue.length >= 1) {
    generatedVideoDroppedFrames += 1;
    closeGeneratedVideoFrame(generatedVideoFrameQueue.shift());
    if (
      generatedVideoDroppedFrames === 1 ||
      generatedVideoDroppedFrames % 120 === 0
    ) {
      postEvent({
        type: "metric",
        name: "generator_queue_drop",
        count: generatedVideoDroppedFrames,
      });
    }
  }

  generatedVideoFrameQueue.push(frame);
  pumpGeneratedVideoFrames();
};

const createGeneratedVideoTrackWorkerHandle = () => {
  if (typeof self.VideoTrackGenerator === "function") {
    const videoTrackGenerator = new self.VideoTrackGenerator();
    const videoTrack =
      videoTrackGenerator?.track && typeof videoTrackGenerator.track.stop === "function"
        ? videoTrackGenerator.track
        : null;
    const videoWriter =
      videoTrackGenerator?.writable &&
      typeof videoTrackGenerator.writable.getWriter === "function"
        ? videoTrackGenerator.writable.getWriter()
        : null;
    if (videoTrack && videoWriter) {
      generator = videoTrackGenerator;
      outputTrack = videoTrack;
      writer = videoWriter;
      return "VideoTrackGenerator";
    }
    try {
      videoWriter?.releaseLock?.();
    } catch {}
    try {
      videoTrack?.stop?.();
    } catch {}
  }

  if (typeof self.MediaStreamTrackGenerator === "function") {
    const mediaStreamTrackGenerator = new self.MediaStreamTrackGenerator({ kind: "video" });
    const videoWriter =
      mediaStreamTrackGenerator?.writable &&
      typeof mediaStreamTrackGenerator.writable.getWriter === "function"
        ? mediaStreamTrackGenerator.writable.getWriter()
        : null;
    const videoTrack =
      typeof mediaStreamTrackGenerator?.clone === "function"
        ? mediaStreamTrackGenerator.clone()
        : null;
    if (videoWriter && videoTrack) {
      generator = mediaStreamTrackGenerator;
      outputTrack = videoTrack;
      writer = videoWriter;
      return "MediaStreamTrackGenerator";
    }
    try {
      videoWriter?.releaseLock?.();
    } catch {}
    try {
      mediaStreamTrackGenerator?.stop?.();
    } catch {}
    try {
      videoTrack?.stop?.();
    } catch {}
  }

  return null;
};

const createProbeFrame = timestampMicros => {
  if (typeof self.VideoFrame !== "function") {
    return null;
  }
  const timestamp = Math.max(0, Math.round(Number(timestampMicros || 0) || 0));
  const colorSeed = Math.floor(timestamp / 33333) % 3;
  const fillStyle = colorSeed === 0 ? "#ffffff" : colorSeed === 1 ? "#00ff88" : "#4488ff";
  const source =
    typeof self.OffscreenCanvas === "function"
      ? new self.OffscreenCanvas(8, 8)
      : null;
  if (!source) {
    return null;
  }
  const context = source.getContext("2d");
  if (context) {
    context.fillStyle = "#101418";
    context.fillRect(0, 0, 8, 8);
    context.fillStyle = fillStyle;
    context.fillRect(0, 0, 8, 8);
  }
  return new self.VideoFrame(source, {
    timestamp,
    duration: 33333,
  });
};

const configureVideoDecoder = codec => {
  if (videoDecoder || !codec || codec === "jpeg") {
    return;
  }
  if (typeof self.VideoDecoder !== "function") {
    throw new Error("VideoDecoder is unavailable in the desktop stream worker.");
  }

  videoDecoder = new self.VideoDecoder({
    output: frame => {
      enqueueGeneratedVideoFrame(frame);
    },
    error: error => {
      postEvent({
        type: "error",
        code: "worker_decoder_error",
        message: describeWorkerError(error),
      });
    },
  });
  videoDecoder.configure({
    codec,
    optimizeForLatency: true,
    hardwareAcceleration: "prefer-software",
    avc: { format: "annexb" },
  });
};

const decodeJpegFrame = async (buffer, timestampMicros) => {
  const blob = new Blob([buffer], { type: "image/jpeg" });
  const bitmap = await createImageBitmap(blob);
  try {
    const frame = new self.VideoFrame(bitmap, {
      timestamp: Math.max(0, Math.round(Number(timestampMicros || 0) || 0)),
      duration: frameDurationMicros,
    });
    enqueueGeneratedVideoFrame(frame);
  } finally {
    bitmap.close();
  }
};

const closeWorker = async () => {
  flushGeneratedVideoQueue();
  if (generatedVideoPumpPromise) {
    try {
      await generatedVideoPumpPromise;
    } catch {}
  }
  if (videoDecoder && videoDecoder.state !== "closed") {
    try {
      await videoDecoder.flush();
    } catch {}
    try {
      videoDecoder.close();
    } catch {}
  }
  videoDecoder = null;
  if (writer) {
    try {
      await writer.close();
    } catch {}
    try {
      writer.releaseLock?.();
    } catch {}
  }
  writer = null;
  try {
    outputTrack?.stop?.();
  } catch {}
  outputTrack = null;
  try {
    generator?.stop?.();
  } catch {}
  generator = null;
};

self.onmessage = async event => {
  const data = event?.data || null;
  if (!data || typeof data !== "object") {
    return;
  }

  try {
    if (data.type === "init") {
      frameDurationMicros = Math.max(
        1,
        Math.round(Number(data.frameDurationMicros || 16666) || 16666)
      );
      const mode = createGeneratedVideoTrackWorkerHandle();
      if (!mode || !outputTrack) {
        throw new Error("No transferable generated video track was available in the worker.");
      }
      self.postMessage(
        {
          type: "ready",
          mode,
          track: outputTrack,
        },
        [outputTrack]
      );
      return;
    }

    if (data.type === "probe_frame") {
      const frame = createProbeFrame(data.timestampMicros);
      if (!frame) {
        throw new Error("Worker probe frame creation failed.");
      }
      enqueueGeneratedVideoFrame(frame);
      return;
    }

    if (data.type === "video_packet") {
      frameDurationMicros = Math.max(
        1,
        Math.round(Number(data.durationMicros || frameDurationMicros) || frameDurationMicros)
      );
      if (data.codec === "jpeg") {
        await decodeJpegFrame(data.buffer, data.timestampMicros);
        return;
      }

      configureVideoDecoder(data.codec);
      const chunkType = data.chunkType === "key" ? "key" : "delta";
      if (chunkType === "key") {
        decoderQueueDropCount = 0;
      }
      if (videoDecoder.decodeQueueSize > 1 && chunkType !== "key") {
        decoderQueueDropCount += 1;
        if (
          decoderQueueDropCount === 1 ||
          decoderQueueDropCount % 120 === 0
        ) {
          postEvent({
            type: "metric",
            name: "decoder_queue_drop",
            count: decoderQueueDropCount,
            queue: Number(videoDecoder.decodeQueueSize || 0) || 0,
          });
        }
        return;
      }

      videoDecoder.decode(
        new EncodedVideoChunk({
          type: chunkType,
          timestamp: Math.max(0, Math.round(Number(data.timestampMicros || 0) || 0)),
          duration: frameDurationMicros,
          data: new Uint8Array(data.buffer),
        })
      );
      return;
    }

    if (data.type === "close") {
      await closeWorker();
      postEvent({ type: "closed" });
    }
  } catch (error) {
    postEvent({
      type: "error",
      code: typeof data.type === "string" ? data.type : "worker_runtime",
      message: describeWorkerError(error),
    });
  }
};
`;

    state.desktopStreamVideoWorkerUrl = URL.createObjectURL(
      new Blob([source], { type: "application/javascript" })
    );
    return state.desktopStreamVideoWorkerUrl;
  };

  const createWorkerGeneratedVideoTrackHandle = options =>
    new Promise((resolve, reject) => {
      if (typeof Worker !== "function") {
        reject(new Error("Worker is unavailable."));
        return;
      }

      let settled = false;
      let closed = false;
      const worker = new Worker(getDesktopStreamVideoWorkerUrl());
      const readyTimeout = window.setTimeout(() => {
        if (settled) return;
        settled = true;
        try {
          worker.terminate();
        } catch {}
        reject(new Error("Desktop stream worker initialization timed out."));
      }, 4000);

      const handle = {
        track: null,
        writer: null,
        mode: "WorkerGeneratedTrack",
        worker,
        postVideoPacket(packet) {
          if (closed) {
            return false;
          }
          try {
            const payload = {
              type: "video_packet",
              codec: String(packet?.codec || "avc1.42E034"),
              chunkType: packet?.chunkType === "key" ? "key" : "delta",
              timestampMicros: Math.max(
                0,
                Math.round(Number(packet?.timestampMicros || 0) || 0)
              ),
              durationMicros: Math.max(
                1,
                Math.round(Number(packet?.durationMicros || 16666) || 16666)
              ),
              buffer: packet?.buffer,
            };
            worker.postMessage(payload, payload.buffer ? [payload.buffer] : []);
            return true;
          } catch (error) {
            options?.onEvent?.({
              type: "error",
              code: "worker_post_video_packet_failed",
              message: describeRuntimeError(error),
            });
            return false;
          }
        },
        emitProbeFrame(timestampMicros) {
          if (closed) {
            return false;
          }
          try {
            worker.postMessage({
              type: "probe_frame",
              timestampMicros: Math.max(
                0,
                Math.round(Number(timestampMicros || 0) || 0)
              ),
            });
            return true;
          } catch (error) {
            options?.onEvent?.({
              type: "error",
              code: "worker_probe_frame_failed",
              message: describeRuntimeError(error),
            });
            return false;
          }
        },
        async close() {
          if (closed) {
            return;
          }
          closed = true;
          try {
            worker.postMessage({ type: "close" });
          } catch {}
          try {
            worker.terminate();
          } catch {}
        },
      };

      worker.onmessage = event => {
        const payload = event?.data || null;
        if (!payload || typeof payload !== "object") {
          return;
        }
        if (payload.type === "ready" && payload.track) {
          if (settled) {
            return;
          }
          settled = true;
          window.clearTimeout(readyTimeout);
          handle.track = payload.track;
          handle.mode = `Worker ${String(payload.mode || "GeneratedTrack")}`;
          resolve(handle);
          return;
        }
        if (payload.type === "error" && !settled) {
          settled = true;
          window.clearTimeout(readyTimeout);
          try {
            worker.terminate();
          } catch {}
          reject(
            new Error(
              typeof payload.message === "string" && payload.message.trim()
                ? payload.message.trim()
                : String(payload.code || "desktop stream worker failed")
            )
          );
          return;
        }
        options?.onEvent?.(payload);
      };

      worker.onerror = event => {
        if (settled) {
          options?.onEvent?.({
            type: "error",
            code: "worker_error",
            message: describeRuntimeError(event?.error || event?.message || "worker error"),
          });
          return;
        }
        settled = true;
        window.clearTimeout(readyTimeout);
        try {
          worker.terminate();
        } catch {}
        reject(
          new Error(
            describeRuntimeError(event?.error || event?.message || "worker error")
          )
        );
      };

      worker.postMessage({
        type: "init",
        frameDurationMicros: Math.max(
          1,
          Math.round(Number(options?.frameDurationMicros || 16666) || 16666)
        ),
      });
    });

  const createMainThreadGeneratedTrackProbeHandle = () => {
    const handle = createGeneratedVideoTrackHandle({ allowInTauri: true });
    if (!handle?.track || !handle?.writer) {
      return null;
    }

    return {
      track: handle.track,
      mode: String(handle.mode || "GeneratedTrack"),
      emitProbeFrame: timestampMicros =>
        writeGeneratedTrackProbeFrame(handle.writer, timestampMicros),
      close: () => disposeGeneratedVideoTrackHandle(handle),
    };
  };

  const createGeneratedTrackProbeFrame = timestampMicros => {
    if (typeof window.VideoFrame !== "function") {
      return null;
    }
    let source = null;
    const timestamp = Math.max(0, Math.round(Number(timestampMicros || 0) || 0));
    const colorSeed = Math.floor(timestamp / 33_333) % 3;
    const fillStyle = colorSeed === 0 ? "#ffffff" : colorSeed === 1 ? "#00ff88" : "#4488ff";
    if (typeof window.OffscreenCanvas === "function") {
      source = new window.OffscreenCanvas(8, 8);
      const context = source.getContext("2d");
      if (context) {
        context.fillStyle = "#101418";
        context.fillRect(0, 0, 8, 8);
        context.fillStyle = fillStyle;
        context.fillRect(0, 0, 8, 8);
      }
    } else {
      source = document.createElement("canvas");
      source.width = 8;
      source.height = 8;
      const context = source.getContext("2d");
      if (context) {
        context.fillStyle = "#101418";
        context.fillRect(0, 0, 8, 8);
        context.fillStyle = fillStyle;
        context.fillRect(0, 0, 8, 8);
      }
    }
    return new window.VideoFrame(source, {
      timestamp,
      duration: 33_333,
    });
  };

  const writeGeneratedTrackProbeFrame = async (writer, timestampMicros) => {
    const frame = createGeneratedTrackProbeFrame(timestampMicros);
    if (!frame) {
      throw new Error("VideoFrame is unavailable for the generated-track probe.");
    }
    try {
      await Promise.race([
        writer.write(frame),
        delayMs(300).then(() => {
          throw new Error("Generated-track probe frame write timed out.");
        }),
      ]);
    } finally {
      try {
        frame.close();
      } catch {}
    }
  };

  const runGeneratedTrackTransportProbe = async createHandle => {
    let handle = null;
    let pc1 = null;
    let pc2 = null;
    let sender = null;
    let remoteVideo = null;
    let remoteTrackSeen = false;
    let remoteTrackUnmuted = false;
    let supported = false;
    let reason = "unsupported";
    let mode = null;

    try {
      if (typeof window.RTCPeerConnection !== "function") {
        reason = "rtc_unavailable";
        return { supported, reason, mode };
      }

      handle = await createHandle();
      if (!handle?.track || typeof handle.emitProbeFrame !== "function") {
        reason = "generator_unavailable";
        return { supported, reason, mode };
      }
      mode = String(handle.mode || "");

      pc1 = new RTCPeerConnection();
      pc2 = new RTCPeerConnection();

      pc1.onicecandidate = event => {
        if (event?.candidate) {
          void pc2.addIceCandidate(event.candidate).catch(() => {});
        }
      };
      pc2.onicecandidate = event => {
        if (event?.candidate) {
          void pc1.addIceCandidate(event.candidate).catch(() => {});
        }
      };

      remoteVideo = document.createElement("video");
      remoteVideo.muted = true;
      remoteVideo.autoplay = true;
      remoteVideo.playsInline = true;
      remoteVideo.style.position = "fixed";
      remoteVideo.style.left = "-99999px";
      remoteVideo.style.top = "-99999px";
      remoteVideo.style.width = "1px";
      remoteVideo.style.height = "1px";
      remoteVideo.style.opacity = "0";
      (document.body || document.documentElement)?.appendChild?.(remoteVideo);

      const remoteStream = new MediaStream();
      remoteVideo.srcObject = remoteStream;
      pc2.ontrack = event => {
        remoteTrackSeen = true;
        const incomingTracks = event?.streams?.[0]?.getTracks?.()?.length
          ? event.streams[0].getTracks()
          : event?.track
          ? [event.track]
          : [];
        for (const track of incomingTracks) {
          if (!track) {
            continue;
          }
          track.addEventListener?.("unmute", () => {
            remoteTrackUnmuted = true;
          });
          if (!remoteStream.getTracks().includes(track)) {
            remoteStream.addTrack(track);
          }
        }
        void remoteVideo.play().catch(() => {});
      };

      sender = pc1.addTrack(handle.track, new MediaStream([handle.track]));
      const offer = await pc1.createOffer();
      await pc1.setLocalDescription(offer);
      await pc2.setRemoteDescription(offer);
      const answer = await pc2.createAnswer();
      await pc2.setLocalDescription(answer);
      await pc1.setRemoteDescription(answer);

      const connectionReady = () => {
        const stateA = String(pc1?.connectionState || "");
        const stateB = String(pc2?.connectionState || "");
        const iceA = String(pc1?.iceConnectionState || "");
        const iceB = String(pc2?.iceConnectionState || "");
        return (
          stateA === "connected" ||
          stateB === "connected" ||
          iceA === "connected" ||
          iceB === "connected" ||
          iceA === "completed" ||
          iceB === "completed"
        );
      };

      await waitForCondition(() => connectionReady() || remoteTrackSeen, 2000, 40);

      for (let attempt = 0; attempt < 24; attempt += 1) {
        if (handle.emitProbeFrame(attempt * 33_333) === false) {
          reason = "probe_emit_failed";
          break;
        }
        await delayMs(100);

        if (
          remoteTrackUnmuted ||
          (remoteVideo &&
            remoteVideo.readyState >= 2 &&
            Number(remoteVideo.videoWidth || 0) > 0 &&
            Number(remoteVideo.videoHeight || 0) > 0)
        ) {
          supported = true;
          reason = remoteTrackUnmuted ? "remote_unmuted" : "remote_video_ready";
          break;
        }

        const senderStats = await sender.getStats().catch(() => null);
        if (senderStats) {
          for (const stat of senderStats.values()) {
            if (
              stat?.type === "outbound-rtp" &&
              stat?.isRemote !== true &&
              (stat?.kind === "video" || stat?.mediaType === "video")
            ) {
              if (
                Number(stat?.bytesSent || 0) > 0 ||
                Number(stat?.packetsSent || 0) > 0 ||
                Number(stat?.framesSent || 0) > 0 ||
                Number(stat?.framesEncoded || 0) > 0
              ) {
                supported = true;
                reason = "outbound_rtp";
                break;
              }
            }
          }
        }
        if (supported) {
          break;
        }

        const receiverStats = await pc2.getStats().catch(() => null);
        if (receiverStats) {
          for (const stat of receiverStats.values()) {
            if (
              stat?.type === "inbound-rtp" &&
              (stat?.kind === "video" || stat?.mediaType === "video")
            ) {
              if (
                Number(stat?.bytesReceived || 0) > 0 ||
                Number(stat?.packetsReceived || 0) > 0 ||
                Number(stat?.framesDecoded || 0) > 0
              ) {
                supported = true;
                reason = "inbound_rtp";
                break;
              }
            }
          }
        }
        if (supported) {
          break;
        }
      }

      if (!supported) {
        reason = remoteTrackSeen ? "no_rtp_stats" : "no_remote_track";
      }
      return { supported, reason, mode };
    } catch (error) {
      return {
        supported: false,
        reason: describeRuntimeError(error),
        mode,
      };
    } finally {
      try {
        pc1?.close?.();
      } catch {}
      try {
        pc2?.close?.();
      } catch {}
      if (remoteVideo) {
        try {
          remoteVideo.srcObject = null;
        } catch {}
        remoteVideo.remove();
      }
      await handle?.close?.();
    }
  };

  const resolveTauriGeneratedVideoTrackSupport = async (options = {}) => {
    if (!window.__TAURI_INTERNALS__) {
      return true;
    }
    if (!supportsNativeWindowsScreenShare()) {
      state.nativeGeneratedTrackSupport = false;
      state.nativeGeneratedTrackMode = null;
      state.nativeGeneratedTrackProbeAt = Date.now();
      state.nativeGeneratedTrackProbeReason = "desktop_stream_unavailable";
      return false;
    }
    if (options?.ignorePersistedCache !== true) {
      const cachedCapability = readPersistedGeneratedTrackCapability();
      if (cachedCapability) {
        state.nativeGeneratedTrackSupport = true;
        state.nativeGeneratedTrackMode = cachedCapability.path;
        state.nativeGeneratedTrackProbeAt = cachedCapability.cachedAt || Date.now();
        state.nativeGeneratedTrackProbeReason =
          cachedCapability.reason || "persisted_generated_track_capability";
        report(
          "desktop_stream_generated_track_cache_hit=" +
            JSON.stringify({
              path: cachedCapability.path,
              mode: cachedCapability.mode,
              reason: cachedCapability.reason,
            })
        );
        return true;
      }
    }
    if (typeof state.nativeGeneratedTrackSupport === "boolean") {
      if (state.nativeGeneratedTrackSupport || options?.preferFreshOnFailure !== true) {
        return state.nativeGeneratedTrackSupport;
      }
      report(
        "desktop_stream_generated_track_probe_retry=" +
          JSON.stringify({
            reason:
              typeof state.nativeGeneratedTrackProbeReason === "string"
                ? state.nativeGeneratedTrackProbeReason
                : null,
            ageMs: Math.max(0, Date.now() - (Number(state.nativeGeneratedTrackProbeAt || 0) || 0)),
          })
      );
    }
    if (state.nativeGeneratedTrackSupportPromise) {
      return state.nativeGeneratedTrackSupportPromise;
    }

    state.nativeGeneratedTrackSupportPromise = (async () => {
      let probe = {
        supported: false,
        path: null,
        mode: null,
        reason: "unsupported",
      };

      try {
        const workerProbe = await runGeneratedTrackTransportProbe(() =>
          createWorkerGeneratedVideoTrackHandle({ frameDurationMicros: 33_333 })
        );
        report(
          "desktop_stream_generated_track_candidate=" +
            JSON.stringify({
              candidate: "worker",
              supported: workerProbe.supported === true,
              mode: workerProbe.mode || null,
              reason: workerProbe.reason || null,
            })
        );
        if (workerProbe.supported) {
          probe = {
            supported: true,
            path: "worker",
            mode: workerProbe.mode,
            reason: workerProbe.reason,
          };
          return true;
        }

        const mainProbe = await runGeneratedTrackTransportProbe(() =>
          Promise.resolve(createMainThreadGeneratedTrackProbeHandle())
        );
        report(
          "desktop_stream_generated_track_candidate=" +
            JSON.stringify({
              candidate: "main",
              supported: mainProbe.supported === true,
              mode: mainProbe.mode || null,
              reason: mainProbe.reason || null,
            })
        );
        if (mainProbe.supported) {
          probe = {
            supported: true,
            path: "main",
            mode: mainProbe.mode,
            reason: mainProbe.reason,
          };
          return true;
        }

        probe = {
          supported: false,
          path: null,
          mode: mainProbe.mode || workerProbe.mode || null,
          reason: `worker:${workerProbe.reason};main:${mainProbe.reason}`,
        };
        return false;
      } catch (error) {
        probe = {
          supported: false,
          path: null,
          mode: probe.mode,
          reason: `probe_error:${describeRuntimeError(error)}`,
        };
        return false;
      } finally {
        state.nativeGeneratedTrackSupport = probe.supported;
        state.nativeGeneratedTrackMode = probe.path;
        state.nativeGeneratedTrackProbeAt = Date.now();
        state.nativeGeneratedTrackProbeReason = probe.reason;
        persistGeneratedTrackCapability(probe);
        report(
          "desktop_stream_generated_track_probe=" +
            JSON.stringify({
              supported: probe.supported,
              path: probe.path,
              mode: probe.mode,
              reason: probe.reason,
              host: window.__TAURI_INTERNALS__ ? "tauri" : "browser",
            })
        );
        state.nativeGeneratedTrackSupportPromise = null;
      }
    })();

    return state.nativeGeneratedTrackSupportPromise;
  };

  const createAsyncTaskScheduler = maxConcurrent => {
    const concurrency = Math.max(1, Number(maxConcurrent) || 1);
    const inFlight = new Map();
    const queue = [];
    let active = 0;

    const pump = () => {
      while (active < concurrency && queue.length) {
        const job = queue.shift();
        if (!job) {
          continue;
        }
        active += 1;
        Promise.resolve()
          .then(job.run)
          .then(job.resolve, job.reject)
          .finally(() => {
            active = Math.max(0, active - 1);
            inFlight.delete(job.key);
            pump();
          });
      }
    };

    return (key, run) => {
      const taskKey = String(key || "");
      if (!taskKey) {
        return Promise.resolve(null);
      }
      if (inFlight.has(taskKey)) {
        return inFlight.get(taskKey);
      }
      const promise = new Promise((resolve, reject) => {
        queue.push({ key: taskKey, run, resolve, reject });
        pump();
      });
      inFlight.set(taskKey, promise);
      return promise;
    };
  };

  const scheduleScreenShareThumbnailTask = createAsyncTaskScheduler(4);
  const scheduleScreenSharePreviewTask = createAsyncTaskScheduler(1);

  const warmDesktopStreamStartupCaches = async () => {
    if (!supportsNativeWindowsScreenShare()) {
      return;
    }

    try {
      await resolveTauriGeneratedVideoTrackSupport({
        ignorePersistedCache: false,
        preferFreshOnFailure: false,
      });
    } catch (error) {
      report(
        `desktop_stream_startup_warmup_generated_track_failed=${describeRuntimeError(error)}`
      );
    }

    try {
      const warmQuality = {
        width: 1280,
        height: 720,
        frameRate: 60,
      };
      const previewKey = `1280x720@60`;
      if (!cacheHas(state.screenShareEncoderPreviewCache, previewKey)) {
        const response = await invoke("get_desktop_stream_encoder_preview", {
          request: warmQuality,
        });
        cacheSet(
          state.screenShareEncoderPreviewCache,
          previewKey,
          {
            videoCodec:
              typeof response?.videoCodec === "string" ? response.videoCodec : "avc1",
            encoderMode:
              typeof response?.encoderMode === "string" && response.encoderMode.trim()
                ? response.encoderMode.trim()
                : "Software H.264",
            encoderDetail:
              typeof response?.encoderDetail === "string" && response.encoderDetail.trim()
                ? response.encoderDetail.trim()
                : null,
            colorMode:
              typeof response?.colorMode === "string" && response.colorMode.trim()
                ? response.colorMode.trim()
                : null,
          },
          24
        );
      }
      report("desktop_stream_startup_warmup_complete=true");
    } catch (error) {
      report(
        `desktop_stream_startup_warmup_encoder_failed=${describeRuntimeError(error)}`
      );
    }
  };

  const loadScreenShareSources = async (reason = "unknown") => {
    const startedAt =
      typeof performance !== "undefined" && typeof performance.now === "function"
        ? performance.now()
        : Date.now();
    try {
      const sources = await invoke("get_capturer_sources");
      const normalized = Array.isArray(sources)
        ? sources.filter(
            source =>
              source &&
              typeof source.id === "string" &&
              typeof source.name === "string" &&
              typeof source.url === "string"
          )
        : [];
      const endedAt =
        typeof performance !== "undefined" && typeof performance.now === "function"
          ? performance.now()
          : Date.now();
      void report(
        `screen_share_picker_sources_loaded reason=${reason} count=${normalized.length} duration_ms=${
          Math.max(0, Math.round(endedAt - startedAt))
        }`
      );
      return normalized;
    } catch (error) {
      const endedAt =
        typeof performance !== "undefined" && typeof performance.now === "function"
          ? performance.now()
          : Date.now();
      void report(
        `screen_share_picker_sources_failed reason=${reason} duration_ms=${
          Math.max(0, Math.round(endedAt - startedAt))
        } message=${error && error.message ? error.message : String(error)}`,
        { force: true }
      );
      throw error;
    }
  };

  const loadScreenShareThumbnail = async sourceId => {
    if (!sourceId) return null;
    const cached = cacheGet(state.screenShareThumbnailCache, sourceId);
    if (typeof cached === "string") {
      return cached;
    }

    return scheduleScreenShareThumbnailTask(`thumb:${sourceId}`, async () => {
      try {
        const url = await invoke("get_capturer_thumbnail", { id: sourceId });
        if (typeof url === "string" && url.length) {
          return cacheSet(state.screenShareThumbnailCache, sourceId, url, 48);
        }
      } catch (error) {
        console.warn("[Equirust] Failed to load screen share thumbnail", error);
      }

      return null;
    });
  };

  const loadLargeScreenSharePreview = async sourceId => {
    if (!sourceId) return null;
    const cached = cacheGet(state.screenSharePreviewCache, sourceId);
    if (typeof cached === "string") {
      return cached;
    }

    return scheduleScreenSharePreviewTask(`preview:${sourceId}`, async () => {
      try {
        const url = await invoke("get_capturer_large_thumbnail", { id: sourceId });
        if (typeof url === "string" && url.length) {
          return cacheSet(state.screenSharePreviewCache, sourceId, url, 12);
        }
      } catch (error) {
        console.warn("[Equirust] Failed to load large screen share preview", error);
      }

      return null;
    });
  };

  const openScreenSharePicker = (sources, defaults = {}) => {
    ensureScreenSharePickerStyles();

    if (state.screenSharePickerBusy) {
      return Promise.reject(createAbortError("Screen share picker is already open."));
    }

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
            `${source.id}|${source.name}|${source.kind}|${source.processName || ""}`
        )
        .sort()
        .join("||");
    let pickerSources = normalizePickerSources(sources);
    const quality = readScreenShareQuality();
    const getSourceGroups = () => ({
      window: pickerSources.filter(
        source => String(source.kind || "").toLowerCase() === "window"
      ),
      screen: pickerSources.filter(
        source => String(source.kind || "").toLowerCase() === "screen"
      ),
    });
    let groups = getSourceGroups();
    const normalizeSourceSearch = value =>
      String(value || "")
        .toLocaleLowerCase()
        .replace(/\s+/g, " ")
        .trim();
    const sourceMatchesSearch = (source, term) => {
      const normalizedTerm = normalizeSourceSearch(term);
      if (!normalizedTerm) {
        return true;
      }

      const haystack = [
        source?.name,
        source?.processName,
        source?.kind === "screen" ? "screen display monitor" : "window app application",
      ]
        .filter(value => typeof value === "string" && value.trim())
        .map(value => normalizeSourceSearch(value))
        .join(" ");

      return haystack.includes(normalizedTerm);
    };
    let searchTerm = "";
    const baseResolutionOptions = [720, 1080, 1440, "source"];
    const baseFrameRateOptions = [15, 30, 60];
    const nativeH264MaxWidth = 3840;
    const nativeH264MaxHeight = 2160;
    const toPositiveNumber = value => {
      const parsed = Number(value);
      return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
    };
    const normalizeEvenDimension = value => {
      const numeric = Math.max(2, Math.round(Number(value) || 0));
      return Math.max(2, Math.round(numeric / 2) * 2);
    };
    const clampAspectRatio = (ratio, kind) => {
      const safeRatio = Number(ratio);
      if (!Number.isFinite(safeRatio) || safeRatio <= 0) {
        return 16 / 9;
      }
      const sourceKind = String(kind || "").toLowerCase() === "screen" ? "screen" : "window";
      const minRatio = 9 / 32;
      const maxRatio = sourceKind === "window" ? 32 / 9 : 48 / 9;
      return Math.min(maxRatio, Math.max(minRatio, safeRatio));
    };
    const clampDesktopStreamDimensions = (width, height, kind = "window") => {
      let safeWidth = normalizeEvenDimension(width);
      let safeHeight = normalizeEvenDimension(height);
      const sourceKind = String(kind || "").toLowerCase() === "screen" ? "screen" : "window";
      const aspectRatio = clampAspectRatio(safeWidth / Math.max(1, safeHeight), sourceKind);
      const widthOverflow = safeWidth / nativeH264MaxWidth;
      const heightOverflow = safeHeight / nativeH264MaxHeight;
      const overflowScale = Math.max(1, widthOverflow, heightOverflow);
      if (overflowScale > 1) {
        safeWidth = normalizeEvenDimension(safeWidth / overflowScale);
        safeHeight = normalizeEvenDimension(safeHeight / overflowScale);
      }
      safeWidth = Math.min(nativeH264MaxWidth, safeWidth);
      safeHeight = Math.min(nativeH264MaxHeight, safeHeight);
      safeWidth = normalizeEvenDimension(Math.max(2, safeHeight * aspectRatio));
      if (safeWidth > nativeH264MaxWidth) {
        safeWidth = nativeH264MaxWidth;
        safeHeight = normalizeEvenDimension(Math.max(2, safeWidth / Math.max(0.1, aspectRatio)));
      }
      safeHeight = Math.min(nativeH264MaxHeight, safeHeight);
      safeWidth = Math.min(nativeH264MaxWidth, safeWidth);
      return {
        width: safeWidth,
        height: safeHeight,
        clamped: safeWidth !== normalizeEvenDimension(width) || safeHeight !== normalizeEvenDimension(height),
      };
    };
    const getSourceReferenceGeometry = source => {
      const nativeWidth = toPositiveNumber(source?.nativeWidth);
      const nativeHeight = toPositiveNumber(source?.nativeHeight);
      const captureWidth = toPositiveNumber(source?.captureWidth);
      const captureHeight = toPositiveNumber(source?.captureHeight);
      const referenceWidth = captureWidth || nativeWidth || 1920;
      const referenceHeight = captureHeight || nativeHeight || 1080;
      const aspectRatio =
        referenceWidth > 0 && referenceHeight > 0
          ? clampAspectRatio(referenceWidth / referenceHeight, source?.kind)
          : 16 / 9;
      return {
        referenceWidth,
        referenceHeight,
        aspectRatio,
      };
    };
    const resolveScreenShareQualitySelection = (source, qualityLike) => {
      const geometry = getSourceReferenceGeometry(source);
      const requestedFrameRate = toPositiveNumber(qualityLike?.frameRate) || 60;
      const sourceMaxFrameRate = toPositiveNumber(source?.maxFrameRate);
      const frameRateCap =
        sourceMaxFrameRate > 0 ? Math.max(15, Math.min(60, sourceMaxFrameRate)) : 60;
      const frameRateOptions = baseFrameRateOptions.filter(option => option <= frameRateCap);
      const frameRate =
        frameRateOptions
          .filter(option => option <= requestedFrameRate)
          .pop() ||
        frameRateOptions[frameRateOptions.length - 1] ||
        60;

      const requestedResolutionMode =
        String(qualityLike?.resolutionMode || "").toLowerCase() === "source"
          ? "source"
          : String(Math.max(720, toPositiveNumber(qualityLike?.height) || 1080));

      let requestedWidth = geometry.referenceWidth;
      let requestedHeight = geometry.referenceHeight;
      let resolvedResolutionMode = requestedResolutionMode;
      if (requestedResolutionMode !== "source") {
        requestedHeight = Math.max(720, Number(requestedResolutionMode) || 1080);
        requestedWidth =
          requestedHeight === geometry.referenceHeight
            ? Math.round(geometry.referenceWidth)
            : Math.max(2, Math.round(requestedHeight * geometry.aspectRatio));
      }

      // Upscaling the source adds load on both the native bridge and WebRTC without
      // improving stream detail, so clamp explicit quality targets to the source bounds.
      if (
        geometry.referenceWidth > 0 &&
        geometry.referenceHeight > 0 &&
        (requestedWidth > geometry.referenceWidth || requestedHeight > geometry.referenceHeight)
      ) {
        requestedWidth = geometry.referenceWidth;
        requestedHeight = geometry.referenceHeight;
        resolvedResolutionMode = "source";
      }

      const clamped = clampDesktopStreamDimensions(
        requestedWidth,
        requestedHeight,
        source?.kind
      );
      return {
        width: clamped.width,
        height: clamped.height,
        frameRate,
        resolutionMode: resolvedResolutionMode,
      };
    };
    const getRecommendedResolutionMode = source => {
      const geometry = getSourceReferenceGeometry(source);
      const sourceKind = String(source?.kind || "").toLowerCase() === "screen" ? "screen" : "window";
      const sourceHeight = Math.max(0, Math.round(Number(geometry.referenceHeight || 0) || 0));
      const runtimeCapability = getDesktopStreamRuntimeCapability();
      if (sourceHeight > 0 && sourceHeight <= 720) {
        return "source";
      }

      const displayHeights = pickerSources
        .filter(item => String(item?.kind || "").toLowerCase() === "screen")
        .map(item =>
          Math.max(
            toPositiveNumber(item?.captureHeight),
            toPositiveNumber(item?.nativeHeight),
            toPositiveNumber(item?.height)
          )
        )
        .filter(value => value >= 480);
      const highestDisplayHeight = displayHeights.length ? Math.max(...displayHeights) : 1080;
      const baseRecommendedHeight =
        highestDisplayHeight >= 1440
          ? 1440
          : highestDisplayHeight >= 1080
            ? 1080
            : 720;
      const maxRecommendedHeight =
        runtimeCapability.isTauriRuntime && !runtimeCapability.generatedTrackSupported
          ? Math.min(1080, baseRecommendedHeight)
          : baseRecommendedHeight;
      const effectiveRecommendedHeight =
        sourceHeight > 0 ? Math.min(sourceHeight, maxRecommendedHeight) : maxRecommendedHeight;
      if (sourceHeight > 0 && effectiveRecommendedHeight >= sourceHeight) {
        return "source";
      }
      if (effectiveRecommendedHeight >= 1440) return "1440";
      if (effectiveRecommendedHeight >= 1080) return "1080";
      return "720";
    };
    const getResolutionOptions = source => {
      void source;
      return [...baseResolutionOptions];
    };
    const getFrameRateOptions = source => {
      const nativeMaxFrameRate = toPositiveNumber(source?.maxFrameRate);
      const maxAvailableFrameRate =
        nativeMaxFrameRate > 0 ? Math.max(15, Math.min(60, nativeMaxFrameRate)) : 60;
      const options = baseFrameRateOptions.filter(option => option <= maxAvailableFrameRate);
      return options.length ? options : [15];
    };
    const formatResolutionOptionLabel = value =>
      String(value).toLowerCase() === "source" ? "Source" : `${value}p`;
    const chooseSupportedQuality = (source, fallbackQuality) => {
      return resolveScreenShareQualitySelection(source, fallbackQuality);
    };
    const getDefaultQualityForSource = source => {
      const recommendedResolutionMode = getRecommendedResolutionMode(source);
      return chooseSupportedQuality(
        source,
        rememberQuality
          ? quality
          : {
              ...quality,
              resolutionMode: recommendedResolutionMode,
              height: Number(recommendedResolutionMode) || 1080,
              width: Math.round((Number(recommendedResolutionMode) || 1080) * (16 / 9)),
            }
      );
    };
    const hasExplicitDefaultQuality =
      toPositiveNumber(defaults.width) > 0 ||
      toPositiveNumber(defaults.height) > 0 ||
      toPositiveNumber(defaults.frameRate) > 0;
    const initialTab = groups.window.length ? "window" : "screen";
    let activeTab = initialTab;
    let selectedId =
      (groups[initialTab][0] || groups.screen[0] || groups.window[0] || {}).id || "";
    const findSourceById = sourceId =>
      pickerSources.find(source => source.id === sourceId) || null;
    const getVisibleSources = () =>
      groups[activeTab].filter(source => sourceMatchesSearch(source, searchTerm));
    const ensureValidSelection = () => {
      const visible = getVisibleSources();
      if (!visible.length) {
        selectedId = "";
        return;
      }

      if (!visible.some(source => source.id === selectedId)) {
        selectedId = visible[0]?.id || "";
      }
    };
    const currentSource = () => {
      const visible = getVisibleSources();
      const selectedSource = findSourceById(selectedId);
      if (
        selectedSource &&
        String(selectedSource.kind || "").toLowerCase() === activeTab &&
        sourceMatchesSearch(selectedSource, searchTerm)
      ) {
        return selectedSource;
      }
      return visible[0] || null;
    };
    const initialSource = currentSource();
    const storedRememberQuality = readScreenShareQualityRememberPreference();
    let rememberQuality = storedRememberQuality;
    let currentQuality = hasExplicitDefaultQuality
      ? resolveScreenShareQualitySelection(initialSource, {
          width: toPositiveNumber(defaults.width) || quality.width || 1920,
          height: toPositiveNumber(defaults.height) || quality.height || 1080,
          frameRate: toPositiveNumber(defaults.frameRate) || quality.frameRate || 60,
          resolutionMode:
            String(defaults.resolutionMode || "").toLowerCase() === "source"
              ? "source"
              : quality.resolutionMode,
        })
      : getDefaultQualityForSource(initialSource);
    let qualityDirty = hasExplicitDefaultQuality;
    let includeAudio = defaults.audio !== false;
    let contentHint = defaults.contentHint === "detail" ? "detail" : "motion";
    let settingsExpanded = false;
    let encoderPreview = null;
    let pendingEncoderPreviewKey = "";

    const getEncoderPreviewKey = quality =>
      `${
        Math.max(2, Number(quality?.width || 0) || 0)
      }x${Math.max(2, Number(quality?.height || 0) || 0)}@${
        Math.max(1, Number(quality?.frameRate || 0) || 0)
      }`;

    const inferEncoderKind = mode => {
      const normalized = String(mode || "").toLowerCase();
      if (normalized.includes("hardware")) return "hardware";
      if (normalized.includes("software")) return "software";
      if (normalized.includes("jpeg") || normalized.includes("unavailable")) return "unsupported";
      return "unknown";
    };

    const ensureEncoderPreview = async quality => {
      if (!supportsNativeWindowsScreenShare()) {
        encoderPreview = {
          encoderMode: "Desktop stream encoder unavailable",
          encoderDetail: "Windows desktop streaming is required for encoder diagnostics.",
          colorMode: null,
        };
        return;
      }

      const width = Math.max(2, Number(quality?.width || 0) || 2);
      const height = Math.max(2, Number(quality?.height || 0) || 2);
      const frameRate = Math.max(1, Number(quality?.frameRate || 0) || 1);
      const previewKey = `${width}x${height}@${frameRate}`;
      const cached = cacheGet(state.screenShareEncoderPreviewCache, previewKey);
      if (cached) {
        encoderPreview = cached;
        return;
      }
      if (pendingEncoderPreviewKey === previewKey) {
        return;
      }

      pendingEncoderPreviewKey = previewKey;
      try {
        const response = await invoke("get_desktop_stream_encoder_preview", {
          request: { width, height, frameRate },
        });
        const normalized = {
          videoCodec:
            typeof response?.videoCodec === "string" ? response.videoCodec : "avc1",
          encoderMode:
            typeof response?.encoderMode === "string" && response.encoderMode.trim()
              ? response.encoderMode.trim()
              : "Software H.264",
          encoderDetail:
            typeof response?.encoderDetail === "string" && response.encoderDetail.trim()
              ? response.encoderDetail.trim()
              : null,
          colorMode:
            typeof response?.colorMode === "string" && response.colorMode.trim()
              ? response.colorMode.trim()
              : null,
        };
        cacheSet(state.screenShareEncoderPreviewCache, previewKey, normalized, 24);
        if (pendingEncoderPreviewKey === previewKey) {
          encoderPreview = normalized;
        }
      } catch (error) {
        report(
          `screen_share_encoder_preview_failed key=${previewKey} message=${
            error && error.message ? error.message : String(error)
          }`,
          { force: true }
        );
      } finally {
        if (pendingEncoderPreviewKey === previewKey) {
          pendingEncoderPreviewKey = "";
      }
    }
  };

    return new Promise((resolve, reject) => {
      const overlay = document.createElement("div");
      overlay.className = "equirust-screenshare equirust-surface-backdrop";
      let closed = false;
      let refreshTimer = null;
      let deferredRenderWorkTimer = null;
      let refreshInFlight = false;
      const liveRefreshEnabled = false;
      const refreshIdleMs = 15000;
      const refreshStableMs = 30000;
      const maxAdditionalPreviewNodes = 1;
      const maxStableRefreshCycles = 3;
      let thumbnailObserver = null;
      let lastSourceSignature = sourcesSignature(pickerSources);
      let stableRefreshCycles = 0;
      let restoreSearchSelection = null;
      const onWindowFocus = () => {
        void refreshSources("focus");
      };
      const onVisibilityChange = () => {
        if (!document.hidden) {
          void refreshSources("visibility");
        }
      };

      const refreshGroupsAndSelection = () => {
        groups = getSourceGroups();
        if (!groups[activeTab]?.length) {
          activeTab = groups.window.length ? "window" : "screen";
        }
        ensureValidSelection();
        if (!qualityDirty) {
          currentQuality = getDefaultQualityForSource(currentSource());
        }
      };

      const disconnectThumbnailObserver = () => {
        if (thumbnailObserver) {
          thumbnailObserver.disconnect();
          thumbnailObserver = null;
        }
      };

      const clearRefreshTimer = () => {
        if (refreshTimer) {
          window.clearTimeout(refreshTimer);
          refreshTimer = null;
        }
      };

      const clearDeferredRenderWorkTimer = () => {
        if (deferredRenderWorkTimer) {
          window.clearTimeout(deferredRenderWorkTimer);
          deferredRenderWorkTimer = null;
        }
      };

      const scheduleRefresh = delay => {
        if (!liveRefreshEnabled) {
          return;
        }
        clearRefreshTimer();
        if (closed) {
          return;
        }
        refreshTimer = window.setTimeout(() => {
          void refreshSources("timer");
        }, Math.max(750, Number(delay) || refreshIdleMs));
      };

      const cleanup = result => {
        if (closed) {
          return;
        }
        closed = true;
        clearRefreshTimer();
        clearDeferredRenderWorkTimer();
        disconnectThumbnailObserver();
        document.removeEventListener("keydown", onKeyDown, true);
        document.removeEventListener("visibilitychange", onVisibilityChange);
        window.removeEventListener("focus", onWindowFocus);
        overlay.remove();
        state.screenSharePickerBusy = false;
        void report(
          `screen_share_picker_close ok=${result?.ok === true}`
        );
        if (result?.ok) {
          resolve(result.value);
        } else {
          reject(result?.error || createAbortError("Screen share was cancelled."));
        }
      };

        const selectSource = nextId => {
        selectedId = String(nextId || "");
        if (!qualityDirty) {
          currentQuality = getDefaultQualityForSource(currentSource());
        }
        render();
      };

      const onKeyDown = event => {
        if (event.key === "Escape") {
          event.preventDefault();
          cleanup({ ok: false });
        }
      };

      const render = () => {
        if (closed) {
          return;
        }
        disconnectThumbnailObserver();
        clearDeferredRenderWorkTimer();
        const visibleSources = getVisibleSources();
        const chosen = currentSource();
        const sourceKind = String(chosen?.kind || activeTab || "screen");
        const resolutionOptions = getResolutionOptions(chosen);
        const frameRateOptions = getFrameRateOptions(chosen);
        const totalAvailableCount = groups[activeTab]?.length || 0;
        const visibleCountLabel =
          activeTab === "window" ? "window captures" : "display captures";
        const sourceCountLabel = searchTerm
          ? `${visibleSources.length} of ${totalAvailableCount} ${visibleCountLabel}`
          : `${totalAvailableCount} ${visibleCountLabel}`;
        const previewUrl =
          (chosen?.id && cacheGet(state.screenShareThumbnailCache, chosen.id)) ||
          chosen?.url ||
          "";
        const encoderPreviewKey = getEncoderPreviewKey(currentQuality);
        const cachedEncoderPreview = cacheGet(
          state.screenShareEncoderPreviewCache,
          encoderPreviewKey
        );
        if (cachedEncoderPreview) {
          encoderPreview = cachedEncoderPreview;
        }
        const encoderModeLabel =
          typeof encoderPreview?.encoderMode === "string" && encoderPreview.encoderMode.trim()
            ? encoderPreview.encoderMode.trim()
            : "Detecting encoder...";
        const encoderKind = inferEncoderKind(encoderModeLabel);
        const encoderDetailParts = [];
        if (typeof encoderPreview?.encoderDetail === "string" && encoderPreview.encoderDetail.trim()) {
          encoderDetailParts.push(encoderPreview.encoderDetail.trim());
        }
        if (typeof encoderPreview?.colorMode === "string" && encoderPreview.colorMode.trim()) {
          encoderDetailParts.push(encoderPreview.colorMode.trim());
        }
        const encoderDetailText =
          encoderDetailParts.length > 0
            ? encoderDetailParts.join(" • ")
            : "Encoder details update with your selected quality.";
        const encoderUnavailable = encoderKind === "unsupported";
        const selectedProcessId = Number(chosen?.processId);
        const windowSupportsAppAudio =
          sourceKind === "window" &&
          Number.isFinite(selectedProcessId) &&
          selectedProcessId > 0;
        const audioToggleLabel =
          sourceKind === "window"
            ? windowSupportsAppAudio
              ? "Include App Audio"
              : "Include System Audio"
            : "Include System Audio";
        const audioNoteText =
          sourceKind === "window"
            ? windowSupportsAppAudio
              ? "Only the selected app's audio will be shared."
              : "App-audio capture is unavailable for this window. System audio (excluding Equirust and Discord output) will be shared."
            : "System audio excludes Equirust and Discord output to reduce feedback.";
        const sourceKindLabel = sourceKind === "window" ? "Window Capture" : "Display Capture";
        const currentAudioModeLabel = includeAudio
          ? sourceKind === "window" && windowSupportsAppAudio
            ? "App Audio"
            : "System Audio"
          : "No Audio";
        const currentResolutionLabel =
          String(currentQuality?.resolutionMode || "").toLowerCase() === "source"
            ? "Source"
            : `${currentQuality.height}p`;
        const recommendedResolutionMode = getRecommendedResolutionMode(chosen);
        const recommendedResolutionLabel =
          String(recommendedResolutionMode).toLowerCase() === "source"
            ? "Source"
            : `${recommendedResolutionMode}p`;
        const emptyStateMessage = searchTerm
          ? `No ${activeTab === "window" ? "window captures" : "display captures"} match "${escapeHtml(
              searchTerm
            )}".`
          : `No ${
              activeTab === "window" ? "shareable window captures" : "display captures"
            } are available right now.`;

        overlay.innerHTML = `
          <div class="equirust-screenshare__dialog equirust-surface-dialog" role="dialog" aria-modal="true" aria-label="Screen Share Picker">
            <section class="equirust-screenshare__main">
              <div class="equirust-screenshare__header equirust-surface-header">
                <div class="equirust-screenshare__headerTop">
                  <div class="equirust-screenshare__headerCopy">
                    <p class="equirust-screenshare__eyebrow equirust-surface-eyebrow">Screen Share</p>
                    <h2 class="equirust-screenshare__title equirust-surface-title">Choose what to stream</h2>
                    <p class="equirust-screenshare__description equirust-surface-copy">Choose a source and stream it through the desktop stream pipeline.</p>
                  </div>
                  <button class="equirust-screenshare__dismiss" type="button" data-action="dismiss" aria-label="Close">×</button>
                </div>
                <div class="equirust-screenshare__tabs">
                  <button class="equirust-screenshare__tab" type="button" data-tab="window" data-active="${activeTab === "window"}">Window Capture</button>
                  <button class="equirust-screenshare__tab" type="button" data-tab="screen" data-active="${activeTab === "screen"}">Display Capture</button>
                </div>
                <div class="equirust-screenshare__toolbar">
                  <span class="equirust-screenshare__sourceCount">${escapeHtml(sourceCountLabel)}</span>
                  <button
                    class="equirust-screenshare__searchClear"
                    type="button"
                    data-action="refresh-sources"
                  >Refresh</button>
                  <label class="equirust-screenshare__search">
                    <input
                      class="equirust-screenshare__searchInput"
                      type="search"
                      data-control="search"
                      value="${escapeHtml(searchTerm)}"
                      placeholder="Filter by window title or process"
                      autocapitalize="off"
                      autocomplete="off"
                      spellcheck="false"
                    />
                    <button
                      class="equirust-screenshare__searchClear"
                      type="button"
                      data-action="clear-search"
                      ${searchTerm ? "" : "disabled"}
                    >Clear</button>
                  </label>
                </div>
              </div>
              <div class="equirust-screenshare__grid">
                ${
                  visibleSources.length
                    ? visibleSources
                        .map(
                          source => `
                            <button class="equirust-screenshare__card" type="button" data-source-id="${source.id}" data-selected="${source.id === selectedId}">
                              <img class="equirust-screenshare__cardPreview" data-preview-id="${source.id}" src="${cacheGet(state.screenShareThumbnailCache, source.id) || source.url}" alt="" />
                              <div class="equirust-screenshare__cardMeta">
                                <div class="equirust-screenshare__cardName">${source.name}</div>
                                <div class="equirust-screenshare__cardKind">${source.kind === "window" ? escapeHtml(source.processName || "Unknown App") : "Display"}</div>
                              </div>
                            </button>
                          `
                        )
                        .join("")
                    : `<div class="equirust-screenshare__empty">${emptyStateMessage}</div>`
                }
              </div>
            </section>
            <aside class="equirust-screenshare__sidebar equirust-surface-panel">
              <div class="equirust-screenshare__sidebarHeader equirust-surface-header">
                <p class="equirust-screenshare__eyebrow equirust-surface-eyebrow">Stream Settings</p>
                <h3 class="equirust-screenshare__title equirust-surface-title" style="font-size:18px;">${chosen?.name || "Nothing selected"}</h3>
                <div class="equirust-screenshare__summary">
                  <span class="equirust-screenshare__pill">${escapeHtml(sourceKindLabel)}</span>
                  <span class="equirust-screenshare__pill">${escapeHtml(`${currentResolutionLabel} • ${currentQuality.frameRate} FPS`)}</span>
                  <span class="equirust-screenshare__pill">${escapeHtml(currentAudioModeLabel)}</span>
                </div>
              </div>
              <div class="equirust-screenshare__previewWrap">
                <div class="equirust-screenshare__preview">
                  ${
                    previewUrl
                      ? `<img src="${previewUrl}" alt="" />`
                      : ""
                  }
                </div>
                <div class="equirust-screenshare__previewLabel">${
                  sourceKind === "window"
                    ? escapeHtml(chosen?.processName || "Unknown App")
                    : "Display"
                }</div>
              </div>
              <div class="equirust-screenshare__controls">
                <label class="equirust-screenshare__field">
                  <span class="equirust-screenshare__fieldLabel">Resolution</span>
                  <div class="equirust-screenshare__optionChips">
                    ${resolutionOptions
                      .map(value => {
                        const normalizedValue = String(value).toLowerCase();
                        const active =
                          String(currentQuality.resolutionMode || "").toLowerCase() ===
                          normalizedValue;
                        const recommended =
                          normalizedValue ===
                          String(recommendedResolutionMode).toLowerCase();
                        return `<button class="equirust-screenshare__optionChip" type="button" data-resolution-option="${value}" data-active="${active}" data-recommended="${recommended}">${escapeHtml(
                          formatResolutionOptionLabel(value)
                        )}</button>`;
                      })
                      .join("")}
                  </div>
                  <span class="equirust-screenshare__fieldHint">Recommended: ${escapeHtml(
                    recommendedResolutionLabel
                  )}</span>
                </label>
                <label class="equirust-screenshare__field">
                  <span class="equirust-screenshare__fieldLabel">Frame Rate</span>
                  <div class="equirust-screenshare__optionChips">
                    ${frameRateOptions
                      .map(
                        value =>
                          `<button class="equirust-screenshare__optionChip" type="button" data-framerate-option="${value}" data-active="${
                            currentQuality.frameRate === value
                          }">${value} FPS</button>`
                      )
                      .join("")}
                  </div>
                </label>
                <div class="equirust-screenshare__field">
                  <span class="equirust-screenshare__fieldLabel">Content Type</span>
                  <div class="equirust-screenshare__hintOptions">
                    <button class="equirust-screenshare__hint" type="button" data-hint="motion" data-active="${contentHint === "motion"}">Prefer Smoothness</button>
                    <button class="equirust-screenshare__hint" type="button" data-hint="detail" data-active="${contentHint === "detail"}">Prefer Clarity</button>
                  </div>
                </div>
                <label class="equirust-screenshare__toggle">
                  <span>${audioToggleLabel}</span>
                  <input type="checkbox" data-control="audio" ${
                    includeAudio ? "checked" : ""
                  } />
                </label>
                <label class="equirust-screenshare__toggle">
                  <span>Save these settings</span>
                  <input type="checkbox" data-control="remember-quality" ${
                    rememberQuality ? "checked" : ""
                  } />
                </label>
                <p class="equirust-screenshare__audioNote">${audioNoteText}</p>
                <div class="equirust-screenshare__encoder">
                  <span class="equirust-screenshare__fieldLabel">Encoding</span>
                  <span class="equirust-screenshare__encoderMode" data-kind="${encoderKind}">${escapeHtml(encoderModeLabel)}</span>
                  <span class="equirust-screenshare__encoderDetail">${escapeHtml(encoderDetailText)}</span>
                </div>
              </div>
              <div class="equirust-screenshare__footer equirust-surface-footer">
                <button class="equirust-screenshare__button equirust-surface-button equirust-surface-button--secondary" type="button" data-action="cancel">Cancel</button>
                <button class="equirust-screenshare__button equirust-surface-button equirust-surface-button--primary" type="button" data-action="continue" ${
                  chosen && !encoderUnavailable ? "" : "disabled"
                }>Continue</button>
              </div>
            </aside>
          </div>
        `;

        overlay.querySelectorAll("[data-tab]").forEach(button => {
          button.addEventListener("click", () => {
            activeTab = button.getAttribute("data-tab") === "screen" ? "screen" : "window";
            ensureValidSelection();
            if (!qualityDirty) {
              currentQuality = getDefaultQualityForSource(currentSource());
            }
            render();
          });
        });

        overlay.querySelectorAll("[data-source-id]").forEach(button => {
          button.addEventListener("click", () => {
            selectSource(button.getAttribute("data-source-id"));
          });
        });

        overlay.querySelector('[data-control="search"]')?.addEventListener("input", event => {
          const nextSearch = String(event.target?.value || "");
          restoreSearchSelection = {
            start:
              typeof event.target?.selectionStart === "number"
                ? event.target.selectionStart
                : nextSearch.length,
            end:
              typeof event.target?.selectionEnd === "number"
                ? event.target.selectionEnd
                : nextSearch.length,
          };
          searchTerm = normalizeSourceSearch(nextSearch);
          ensureValidSelection();
          if (!qualityDirty) {
            currentQuality = getDefaultQualityForSource(currentSource());
          }
          render();
        });

        overlay.querySelector('[data-action="clear-search"]')?.addEventListener("click", () => {
          restoreSearchSelection = { start: 0, end: 0 };
          searchTerm = "";
          ensureValidSelection();
          if (!qualityDirty) {
            currentQuality = getDefaultQualityForSource(currentSource());
          }
          render();
        });

        overlay.querySelectorAll("[data-resolution-option]").forEach(button => {
          button.addEventListener("click", () => {
            qualityDirty = true;
            const chosen = currentSource();
            const resolutionMode = String(
              button.getAttribute("data-resolution-option") || "1080"
            );
            currentQuality = resolveScreenShareQualitySelection(chosen, {
              ...currentQuality,
              resolutionMode:
                resolutionMode.toLowerCase() === "source" ? "source" : resolutionMode,
            });
            render();
          });
        });

        overlay.querySelectorAll("[data-framerate-option]").forEach(button => {
          button.addEventListener("click", () => {
            qualityDirty = true;
            currentQuality = resolveScreenShareQualitySelection(currentSource(), {
              ...currentQuality,
              frameRate:
                Number(button.getAttribute("data-framerate-option") || 60) || 60,
            });
            render();
          });
        });

        overlay.querySelector('[data-control="audio"]')?.addEventListener("change", event => {
          includeAudio = event.target.checked === true;
          render();
        });

        overlay.querySelector('[data-control="remember-quality"]')?.addEventListener("change", event => {
          rememberQuality = event.target.checked === true;
          persistScreenShareRememberPreference(rememberQuality);
        });

        overlay.querySelectorAll("[data-hint]").forEach(button => {
          button.addEventListener("click", () => {
            contentHint = button.getAttribute("data-hint") === "detail" ? "detail" : "motion";
            render();
          });
        });

        overlay.querySelector('[data-action="cancel"]')?.addEventListener("click", () => {
          cleanup({ ok: false });
        });

        overlay.querySelector('[data-action="dismiss"]')?.addEventListener("click", () => {
          cleanup({ ok: false });
        });

        overlay.querySelector('[data-action="refresh-sources"]')?.addEventListener("click", () => {
          void refreshSources("manual");
        });

        overlay.querySelector('[data-action="continue"]')?.addEventListener("click", () => {
          if (encoderUnavailable) {
            return;
          }
          const picked = currentSource();
          if (!picked) return;
          const clamped = clampDesktopStreamDimensions(
            currentQuality.width,
            currentQuality.height,
            picked?.kind
          );
          if (clamped.clamped) {
            report(
              `screen_share_quality_clamped source=${picked.id} requested=${
                currentQuality.width
              }x${currentQuality.height} applied=${clamped.width}x${clamped.height}`
            );
          }
          const appliedQuality = {
            ...currentQuality,
            width: clamped.width,
            height: clamped.height,
          };
          if (rememberQuality) {
            persistScreenShareQuality(appliedQuality);
          }
          cleanup({
            ok: true,
            value: {
              id: picked.id,
              kind: picked.kind === "window" ? "window" : "screen",
              previewUrl:
                (picked.id && cacheGet(state.screenShareThumbnailCache, picked.id)) ||
                picked.url ||
                null,
              processId:
                typeof picked.processId === "number" && Number.isFinite(picked.processId)
                  ? picked.processId
                  : null,
              audio: includeAudio,
              contentHint,
              frameRate: currentQuality.frameRate,
              height: appliedQuality.height,
              nativeHeight: toPositiveNumber(picked?.nativeHeight) || null,
              nativeWidth: toPositiveNumber(picked?.nativeWidth) || null,
              captureHeight: toPositiveNumber(picked?.captureHeight) || null,
              captureWidth: toPositiveNumber(picked?.captureWidth) || null,
              maxFrameRate: toPositiveNumber(picked?.maxFrameRate) || null,
              resolutionMode: appliedQuality.resolutionMode,
              width: appliedQuality.width,
            },
          });
        });

        const chosenSource = currentSource();
        deferredRenderWorkTimer = window.setTimeout(() => {
          deferredRenderWorkTimer = null;
          if (closed) {
            return;
          }
          const loadPreviewNode = image => {
            const sourceId = image?.getAttribute?.("data-preview-id");
            if (!sourceId || cacheHas(state.screenShareThumbnailCache, sourceId)) {
              if (sourceId) {
                const cachedUrl = cacheGet(state.screenShareThumbnailCache, sourceId);
                if (cachedUrl && image) {
                  image.src = cachedUrl;
                }
              }
              return;
            }

            loadScreenShareThumbnail(sourceId).then(url => {
              if (!url) return;
              const previewImage = overlay.querySelector(
                `[data-preview-id="${CSS.escape(sourceId)}"]`
              );
              if (previewImage) {
                previewImage.src = url;
              }
              if (currentSource()?.id === sourceId) {
                const selectedPreviewImage = overlay.querySelector(
                  ".equirust-screenshare__preview img"
                );
                if (selectedPreviewImage) {
                  selectedPreviewImage.src = url;
                }
              }
            });
          };
          const previewNodes = Array.from(overlay.querySelectorAll("[data-preview-id]"));
          const prioritizedPreviewNode =
            chosenSource?.id
              ? overlay.querySelector(`[data-preview-id="${CSS.escape(chosenSource.id)}"]`)
              : null;
          if (prioritizedPreviewNode) {
            loadPreviewNode(prioritizedPreviewNode);
          }
          const remainingPreviewNodes = previewNodes.filter(
            node => node !== prioritizedPreviewNode
          );
          remainingPreviewNodes
            .slice(0, maxAdditionalPreviewNodes)
            .forEach(loadPreviewNode);
          const deferredPreviewNodes = remainingPreviewNodes.slice(maxAdditionalPreviewNodes);
          if (
            deferredPreviewNodes.length &&
            typeof IntersectionObserver === "function"
          ) {
            const gridRoot = overlay.querySelector(".equirust-screenshare__grid");
            thumbnailObserver = new IntersectionObserver(
              entries => {
                entries.forEach(entry => {
                  if (!entry.isIntersecting) {
                    return;
                  }
                  thumbnailObserver?.unobserve(entry.target);
                  loadPreviewNode(entry.target);
                });
              },
              {
                root: gridRoot instanceof Element ? gridRoot : null,
                rootMargin: "180px 0px",
              }
            );
            deferredPreviewNodes.forEach(node => thumbnailObserver.observe(node));
          } else {
            deferredPreviewNodes.forEach((node, index) => {
              window.setTimeout(() => loadPreviewNode(node), Math.min(220, index * 36));
            });
          }
          const previewKeyAtRender = getEncoderPreviewKey(currentQuality);
          void ensureEncoderPreview(currentQuality).then(() => {
            if (closed) return;
            const activeKey = getEncoderPreviewKey(currentQuality);
            if (activeKey !== previewKeyAtRender) return;
            const latest = cacheGet(state.screenShareEncoderPreviewCache, previewKeyAtRender);
            if (!latest || encoderPreview === latest) return;
            encoderPreview = latest;
            render();
          });
        }, 24);

        if (restoreSearchSelection) {
          const nextInput = overlay.querySelector('[data-control="search"]');
          const selection = restoreSearchSelection;
          restoreSearchSelection = null;
          if (nextInput instanceof HTMLInputElement) {
            window.requestAnimationFrame(() => {
              nextInput.focus();
              try {
                nextInput.setSelectionRange(selection.start, selection.end);
              } catch {}
            });
          }
        }
      };

      const refreshSources = async (reason = "timer") => {
        if (closed || refreshInFlight) {
          if (reason === "timer") {
            scheduleRefresh(refreshIdleMs);
          }
          return;
        }
        if (reason === "timer" && document.hidden) {
          scheduleRefresh(refreshStableMs);
          return;
        }
        refreshInFlight = true;
        const startedAt =
          typeof performance !== "undefined" && typeof performance.now === "function"
            ? performance.now()
            : Date.now();
        try {
          const refreshed = normalizePickerSources(await loadScreenShareSources(reason));
          const refreshedSignature = sourcesSignature(refreshed);
          const changed = refreshedSignature !== lastSourceSignature;
          const refreshedIds = refreshed.map(source => source.id);
          cachePrune(state.screenShareThumbnailCache, refreshedIds, 48);
          cachePrune(state.screenSharePreviewCache, refreshedIds, 12);
          cachePrune(state.screenShareEncoderPreviewCache, null, 24);
          if (changed) {
            pickerSources = refreshed;
            lastSourceSignature = refreshedSignature;
            stableRefreshCycles = 0;
            refreshGroupsAndSelection();
            render();
          } else {
            stableRefreshCycles = Math.min(stableRefreshCycles + 1, maxStableRefreshCycles);
          }
          const endedAt =
            typeof performance !== "undefined" && typeof performance.now === "function"
              ? performance.now()
              : Date.now();
          void report(
            `screen_share_picker_refresh reason=${reason} changed=${changed} count=${
              refreshed.length
            } duration_ms=${
              Math.max(0, Math.round(endedAt - startedAt))
            }`
          );
        } catch (error) {
          console.warn("[Equirust] Failed to refresh screen share sources", error);
          const endedAt =
            typeof performance !== "undefined" && typeof performance.now === "function"
              ? performance.now()
              : Date.now();
          void report(
            `screen_share_picker_refresh_failed reason=${reason} duration_ms=${
              Math.max(0, Math.round(endedAt - startedAt))
            } message=${error && error.message ? error.message : String(error)}`,
            { force: true }
          );
        } finally {
          refreshInFlight = false;
          if (!closed && reason === "timer") {
            scheduleRefresh(
              stableRefreshCycles >= maxStableRefreshCycles
                ? refreshStableMs
                : refreshIdleMs
            );
          }
        }
      };

      overlay.querySelector(".equirust-screenshare__dialog")?.addEventListener("pointerdown", event => {
        event.stopPropagation();
      });

      overlay.addEventListener("pointerdown", event => {
        if (event.target === overlay) {
          cleanup({ ok: false });
        }
      });

      document.addEventListener("keydown", onKeyDown, true);
      document.addEventListener("visibilitychange", onVisibilityChange);
      window.addEventListener("focus", onWindowFocus);
      document.body.appendChild(overlay);
      void report(
        `screen_share_picker_open sources=${pickerSources.length} windows=${
          groups.window.length
        } displays=${groups.screen.length}`
      );
      refreshGroupsAndSelection();
      render();
      scheduleRefresh(refreshIdleMs);
    });
  };

  const startNativeDisplayMediaStream = async picked => {
    const sourceKind = picked.kind === "window" ? "window" : "screen";
    const normalizeErrorMessage = error =>
      error && typeof error.message === "string" ? error.message : String(error);
    const nativeH264MaxWidth = 3840;
    const nativeH264MaxHeight = 2160;
    const toPositiveNumber = value => {
      const parsed = Number(value);
      return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
    };
    const normalizeEvenDimension = value => {
      const numeric = Math.max(2, Math.round(Number(value) || 0));
      return Math.max(2, Math.round(numeric / 2) * 2);
    };
    const clampAspectRatio = ratio => {
      const safeRatio = Number(ratio);
      if (!Number.isFinite(safeRatio) || safeRatio <= 0) {
        return 16 / 9;
      }
      const minRatio = 9 / 32;
      const maxRatio = sourceKind === "window" ? 32 / 9 : 48 / 9;
      return Math.min(maxRatio, Math.max(minRatio, safeRatio));
    };
    const clampDesktopStreamDimensions = (width, height) => {
      let safeWidth = normalizeEvenDimension(width);
      let safeHeight = normalizeEvenDimension(height);
      const aspectRatio = clampAspectRatio(safeWidth / Math.max(1, safeHeight));
      const widthOverflow = safeWidth / nativeH264MaxWidth;
      const heightOverflow = safeHeight / nativeH264MaxHeight;
      const overflowScale = Math.max(1, widthOverflow, heightOverflow);
      if (overflowScale > 1) {
        safeWidth = normalizeEvenDimension(safeWidth / overflowScale);
        safeHeight = normalizeEvenDimension(safeHeight / overflowScale);
      }
      safeWidth = Math.min(nativeH264MaxWidth, safeWidth);
      safeHeight = Math.min(nativeH264MaxHeight, safeHeight);
      safeWidth = normalizeEvenDimension(Math.max(2, safeHeight * aspectRatio));
      if (safeWidth > nativeH264MaxWidth) {
        safeWidth = nativeH264MaxWidth;
        safeHeight = normalizeEvenDimension(
          Math.max(2, safeWidth / Math.max(0.1, aspectRatio))
        );
      }
      safeHeight = Math.min(nativeH264MaxHeight, safeHeight);
      safeWidth = Math.min(nativeH264MaxWidth, safeWidth);
      return {
        width: safeWidth,
        height: safeHeight,
        clamped:
          safeWidth !== normalizeEvenDimension(width) ||
          safeHeight !== normalizeEvenDimension(height),
      };
    };
    const fitDesktopStreamDimensionsToBounds = (width, height, maxWidth, maxHeight) => {
      let safeWidth = normalizeEvenDimension(width);
      let safeHeight = normalizeEvenDimension(height);
      const boundedWidth = Math.max(2, normalizeEvenDimension(maxWidth));
      const boundedHeight = Math.max(2, normalizeEvenDimension(maxHeight));
      const aspectRatio = clampAspectRatio(safeWidth / Math.max(1, safeHeight));
      const widthOverflow = safeWidth / boundedWidth;
      const heightOverflow = safeHeight / boundedHeight;
      const overflowScale = Math.max(1, widthOverflow, heightOverflow);
      if (overflowScale > 1) {
        safeWidth = normalizeEvenDimension(safeWidth / overflowScale);
        safeHeight = normalizeEvenDimension(safeHeight / overflowScale);
      }
      safeWidth = Math.min(boundedWidth, safeWidth);
      safeHeight = Math.min(boundedHeight, safeHeight);
      safeWidth = normalizeEvenDimension(Math.max(2, safeHeight * aspectRatio));
      if (safeWidth > boundedWidth) {
        safeWidth = boundedWidth;
        safeHeight = normalizeEvenDimension(
          Math.max(2, safeWidth / Math.max(0.1, aspectRatio))
        );
      }
      safeHeight = Math.min(boundedHeight, safeHeight);
      safeWidth = Math.min(boundedWidth, safeWidth);
      return {
        width: safeWidth,
        height: safeHeight,
      };
    };
    const getSourceReferenceGeometry = source => {
      const nativeWidth = toPositiveNumber(source?.nativeWidth);
      const nativeHeight = toPositiveNumber(source?.nativeHeight);
      const captureWidth = toPositiveNumber(source?.captureWidth);
      const captureHeight = toPositiveNumber(source?.captureHeight);
      const referenceWidth = captureWidth || nativeWidth || toPositiveNumber(source?.width) || 1920;
      const referenceHeight = captureHeight || nativeHeight || toPositiveNumber(source?.height) || 1080;
      const aspectRatio = clampAspectRatio(referenceWidth / Math.max(1, referenceHeight));
      return {
        referenceWidth,
        referenceHeight,
        aspectRatio,
      };
    };
    const resolveDesktopStreamBridgeBudget = source => {
      const runtimeCapability = getDesktopStreamRuntimeCapability();
      if (!runtimeCapability.isTauriRuntime) {
        return null;
      }
      const detailMode = String(source?.contentHint || "").toLowerCase() === "detail";
      const geometry = getSourceReferenceGeometry(source);
      const sourceHeight = Math.max(0, Math.round(Number(geometry.referenceHeight || 0) || 0));
      let maxHeight = detailMode ? 1080 : 720;
      let maxFrameRate = detailMode ? 30 : 60;
      if (runtimeCapability.generatedTrackSupported) {
        if (runtimeCapability.generatedTrackWorker && sourceHeight >= 2160) {
          maxHeight = 2160;
        } else if (sourceHeight >= 1440) {
          maxHeight = 1440;
        } else if (sourceHeight >= 1080) {
          maxHeight = 1080;
        } else {
          maxHeight = 720;
        }
        if (detailMode && maxHeight > 1080 && !runtimeCapability.generatedTrackWorker) {
          maxFrameRate = 30;
        } else {
          maxFrameRate = 60;
        }
      }
      const bounded = clampDesktopStreamDimensions(
        Math.max(2, Math.round(maxHeight * geometry.aspectRatio)),
        maxHeight
      );
      return {
        profile:
          `${sourceKind}_${detailMode ? "detail" : "motion"}_` +
          `${runtimeCapability.generatedTrackSupported ? `generated_${runtimeCapability.generatedTrackPath || "main"}` : "compat"}_` +
          `${bounded.height}p${maxFrameRate}`,
        maxWidth: bounded.width,
        maxHeight: bounded.height,
        maxFrameRate,
      };
    };
    const applyDesktopStreamBridgeBudget = (width, height, frameRate, source) => {
      const budget = resolveDesktopStreamBridgeBudget(source);
      if (!budget) {
        return {
          width,
          height,
          frameRate,
          clamped: false,
          profile: null,
        };
      }
      const fitted = fitDesktopStreamDimensionsToBounds(
        width,
        height,
        budget.maxWidth,
        budget.maxHeight
      );
      const safeFrameRate = Math.max(1, Number(frameRate || 0) || 1);
      const nextFrameRate = Math.min(budget.maxFrameRate, safeFrameRate);
      return {
        width: fitted.width,
        height: fitted.height,
        frameRate: nextFrameRate,
        clamped:
          fitted.width !== width ||
          fitted.height !== height ||
          nextFrameRate !== safeFrameRate,
        profile: budget.profile,
      };
    };
    const requestedFrameRate = Number(picked.frameRate || 60) || 60;
    const requestedDimensions = clampDesktopStreamDimensions(
      Number(picked.width || 1920) || 1920,
      Number(picked.height || 1080) || 1080,
    );
    if (requestedDimensions.clamped) {
      report(
        `desktop_stream_request_clamped source=${picked.id} kind=${sourceKind} ` +
          `requested=${Math.round(Number(picked.width || 0) || 0)}x${
            Math.round(Number(picked.height || 0) || 0)
          } applied=${requestedDimensions.width}x${requestedDimensions.height}`
      );
    }
    const bridgeBudgetedQuality = applyDesktopStreamBridgeBudget(
      requestedDimensions.width,
      requestedDimensions.height,
      requestedFrameRate,
      picked
    );
    if (bridgeBudgetedQuality.clamped) {
      report(
        `desktop_stream_bridge_budget_clamped source=${picked.id} kind=${sourceKind} profile=${String(
          bridgeBudgetedQuality.profile || "unknown"
        )} requested=${requestedDimensions.width}x${requestedDimensions.height}@${requestedFrameRate} applied=${bridgeBudgetedQuality.width}x${bridgeBudgetedQuality.height}@${bridgeBudgetedQuality.frameRate}`,
        { force: true }
      );
    }
    const includeAudio = picked.audio === true;
    const sourceProcessId =
      typeof picked.processId === "number" &&
      Number.isFinite(picked.processId) &&
      picked.processId > 0
        ? Math.floor(picked.processId)
        : null;
    const windowAppAudioAvailable = sourceKind === "window" && sourceProcessId !== null;
    const audioMode = includeAudio
      ? windowAppAudioAvailable
        ? "window_app"
        : "system_excluding_host"
      : "off";
    const previewUrl =
      typeof picked.previewUrl === "string" && picked.previewUrl.trim()
        ? picked.previewUrl.trim()
        : "";
    if (includeAudio && sourceKind === "window" && !windowAppAudioAvailable) {
      report(
        `desktop_stream_audio_mode_fallback source=${picked.id} mode=system_excluding_host reason=missing_window_process_id`,
        { force: true }
      );
    }
    let session;
    try {
      session = await invoke("start_desktop_stream_session", {
        request: {
          sourceId: picked.id,
          sourceKind,
          sourceProcessId,
          captureBackend: "auto",
          audioMode,
          width: bridgeBudgetedQuality.width,
          height: bridgeBudgetedQuality.height,
          frameRate: bridgeBudgetedQuality.frameRate,
          contentHint: picked.contentHint === "detail" ? "detail" : "motion",
          includeSystemAudio: includeAudio,
        },
      });
    } catch (error) {
      report(
        "desktop_stream_start_failed=" +
          JSON.stringify({
            sourceId: picked.id,
            sourceKind,
            requestedWidth: bridgeBudgetedQuality.width,
            requestedHeight: bridgeBudgetedQuality.height,
            requestedFrameRate: bridgeBudgetedQuality.frameRate,
            includeAudio,
            message: normalizeErrorMessage(error),
          }),
        { force: true }
      );
      throw error;
    }
    const sinkDescriptor = normalizeDesktopStreamSinkDescriptor(session.sinkDescriptor);
    if (sinkDescriptor.sinkKind !== "browserMediaStream") {
      await invoke("stop_desktop_stream_session", { sessionId: session.sessionId }).catch(() => {});
      throw new Error(
        `Desktop stream sink is not supported by this runtime: ${String(sinkDescriptor.sinkKind)}`
      );
    }
    const preferGeneratedTrack = desktopStreamSinkUsesGeneratedTrack(sinkDescriptor);
    const allowCanvasFallback = desktopStreamSinkAllowsCanvasFallback(sinkDescriptor);

    const targetVideoWidth = Math.max(1, Number(session.width || picked.width || 1920) || 1920);
    const targetVideoHeight = Math.max(1, Number(session.height || picked.height || 1080) || 1080);
    const targetVideoFrameRate = Math.max(
      1,
      Number(session.frameRate || picked.frameRate || 60) || 60
    );
    const videoCodec = typeof session.videoCodec === "string" ? session.videoCodec : "jpeg";
    const frameDurationMicros = Math.max(
      1,
      Math.round(1_000_000 / Math.max(1, Number(session.frameRate || picked.frameRate || 60) || 60))
    );
    const encoderMode = String(session.encoderMode || "Software H.264");
    const encoderDetail =
      typeof session.encoderDetail === "string" && session.encoderDetail.trim()
        ? session.encoderDetail.trim()
        : "";
    const colorMode = String(session.colorMode || "SDR-safe");
    const nativeReport = (message, options = {}) =>
      reportMedia(`desktop_stream session=${session.sessionId} ${message}`, options);
    const nativeVideoPacketHeaderBytes = 18;
    const maxNativeVideoPacketBytes = 16 * 1024 * 1024;
    const maxNativeAudioPacketBytes = 512 * 1024;
    const maxLiveVideoPacketAgeMs = 350;
    const maxLiveKeyframeAgeMs = 700;
    let handleGeneratedVideoEvent = () => {};
    let canvas = null;
    let context = null;
    let canvasCaptureTrack = null;
    const tryCreateTauriGeneratedVideoHandle = async probeOptions => {
      if (!preferGeneratedTrack) {
        return null;
      }
      const generatorSupported = await resolveTauriGeneratedVideoTrackSupport({
        preferFreshOnFailure: true,
        ...(probeOptions || {}),
      });
      if (!generatorSupported) {
        return null;
      }
      if (state.nativeGeneratedTrackMode === "worker") {
        try {
          const workerHandle = await createWorkerGeneratedVideoTrackHandle({
            frameDurationMicros,
            onEvent: event => handleGeneratedVideoEvent(event),
          });
          if (workerHandle?.track) {
            workerHandle.kind = "worker";
            return workerHandle;
          }
        } catch (error) {
          nativeReport(
            `worker_generated_track_init_failed=${normalizeErrorMessage(error)}`,
            { force: true }
          );
        }
      }
      const mainHandle = createGeneratedVideoTrackHandle({ allowInTauri: true });
      return mainHandle?.track ? mainHandle : null;
    };
    const createPreferredGeneratedVideoHandle = async () => {
      if (window.__TAURI_INTERNALS__) {
        let generatedHandle = await tryCreateTauriGeneratedVideoHandle();
        if (generatedHandle?.track) {
          return generatedHandle;
        }
        if (state.nativeGeneratedTrackSupport === true) {
          const expectedMode = String(state.nativeGeneratedTrackMode || "unknown");
          nativeReport(`direct_track_regression expected=${expectedMode} retrying=true`, {
            force: true,
          });
          resetGeneratedTrackSupportState("direct_track_regression");
          generatedHandle = await tryCreateTauriGeneratedVideoHandle({
            ignorePersistedCache: true,
          });
          if (generatedHandle?.track) {
            nativeReport(
              `direct_track_regression_recovered expected=${expectedMode} actual=${String(
                generatedHandle.mode || generatedHandle.kind || "unknown"
              )}`
            );
            return generatedHandle;
          }
          nativeReport(
            allowCanvasFallback
              ? `direct_track_regression_fallback=canvas expected=${expectedMode}`
              : `direct_track_regression_unavailable expected=${expectedMode} fallback=disabled`,
            { force: true }
          );
        }
        if (!allowCanvasFallback) {
          nativeReport(
            `direct_track_unavailable preferred=${sinkDescriptor.preferredVideoIngress} fallback=disabled transport=${sinkDescriptor.transportKind}`,
            { force: true }
          );
        }
        return null;
      }
      return preferGeneratedTrack ? createGeneratedVideoTrackHandle() : null;
    };
    const generatedVideo = await createPreferredGeneratedVideoHandle();
    const workerGeneratedVideo =
      generatedVideo?.kind === "worker" ? generatedVideo : null;
    let generatedVideoTrack = generatedVideo?.track || null;
    let generatedVideoWriter = generatedVideo?.writer || null;
    let generatedVideoPumpPromise = null;
    let generatedVideoDroppedFrames = 0;
    const generatedVideoFrameQueue = [];
    const videoTrackSourceMode =
      workerGeneratedVideo
        ? String(generatedVideo.mode || "worker_generator")
        : generatedVideoTrack && generatedVideoWriter
        ? String(generatedVideo?.mode || "generator")
        : "canvas";

    if (videoTrackSourceMode === "canvas" && !allowCanvasFallback) {
      await invoke("stop_desktop_stream_session", { sessionId: session.sessionId }).catch(() => {});
      throw new Error(
        "Desktop stream could not start: generated-track ingestion is required by the active sink contract."
      );
    }

    const ensureCanvasVideoPipeline = async () => {
      if (canvas && context) {
        return context;
      }

      canvas = document.createElement("canvas");
      canvas.width = targetVideoWidth;
      canvas.height = targetVideoHeight;
      canvas.setAttribute("aria-hidden", "true");
      canvas.style.position = "fixed";
      canvas.style.left = "-99999px";
      canvas.style.top = "-99999px";
      canvas.style.width = `${canvas.width}px`;
      canvas.style.height = `${canvas.height}px`;
      canvas.style.pointerEvents = "none";
      canvas.style.opacity = "0";
      document.body.appendChild(canvas);

      context = canvas.getContext("2d", {
        alpha: false,
        desynchronized: true,
      });
      if (!context) {
        canvas.remove();
        canvas = null;
        await invoke("stop_desktop_stream_session", { sessionId: session.sessionId }).catch(() => {});
        throw new Error("Desktop stream could not start: canvas initialization failed.");
      }
      context.imageSmoothingEnabled = false;
      return context;
    };

    if (videoTrackSourceMode === "canvas") {
      await ensureCanvasVideoPipeline();
    }

    let socket = null;
    let closed = false;
    let stopIssued = false;
    let teardownRequested = false;
    let decodeBusy = false;
    let pendingFrame = null;
    let decoderQueueDropCount = 0;
    let nativeKeyframeRequestCount = 0;
    let lastNativeKeyframeRequestAt = 0;
    let nativePacingHintFrameRate = targetVideoFrameRate;
    let nativePacingSeverity = 0;
    let lastNativePacingHintAt = 0;
    let lastNativeBackpressureAt = 0;
    let nativePacingRecoveryTimer = null;
    let staleVideoDropCount = 0;
    let waitingForFreshVideoKeyframe = false;
    let nativeVideoPacketsReceived = 0;
    let nativeVideoBytesReceived = 0;
    let nativeVideoPacketAgeTotalMs = 0;
    let nativeVideoPacketAgeSamples = 0;
    let nativeVideoPacketAgeMaxMs = 0;
    let nativeDecodeQueueMax = 0;
    let nativeGeneratedQueueMax = 0;
    let nativeStreamStatsTimer = null;
    let lastNativeStatsSnapshot = {
      videoPacketsReceived: 0,
      videoBytesReceived: 0,
      staleVideoDropCount: 0,
      decoderQueueDropCount: 0,
      generatedVideoDroppedFrames: 0,
      nativeKeyframeRequestCount: 0,
      nativeVideoPacketAgeSamples: 0,
      nativeVideoPacketAgeTotalMs: 0,
      nativeVideoPacketAgeMaxMs: 0,
    };
    reportMedia(
      "display_media_native_session=" +
        JSON.stringify({
          sessionId: session.sessionId,
          requestedWidth: Number(picked.width || 1920) || 1920,
          requestedHeight: Number(picked.height || 1080) || 1080,
          requestedFrameRate,
          bridgeBudgetProfile: bridgeBudgetedQuality.profile,
          bridgeBudgetWidth: bridgeBudgetedQuality.width,
          bridgeBudgetHeight: bridgeBudgetedQuality.height,
          bridgeBudgetFrameRate: bridgeBudgetedQuality.frameRate,
          actualWidth: targetVideoWidth,
          actualHeight: targetVideoHeight,
          actualFrameRate: Number(session.frameRate || picked.frameRate || 60) || 60,
          codec: videoCodec,
          captureBackend: String(session.captureBackend || "auto"),
          encoderMode,
          encoderDetail: encoderDetail || null,
          colorMode,
          audioEnabled: session.audioEnabled === true,
          sinkKind: sinkDescriptor.sinkKind,
          transportKind: sinkDescriptor.transportKind,
          preferredVideoIngress: sinkDescriptor.preferredVideoIngress,
          fallbackVideoIngress: sinkDescriptor.fallbackVideoIngress,
          preferredAudioIngress: sinkDescriptor.preferredAudioIngress,
          browserOwnedPeerConnection: sinkDescriptor.browserOwnedPeerConnection,
          browserOwnedEncoder: sinkDescriptor.browserOwnedEncoder,
          trackSourceMode: videoTrackSourceMode,
        })
    );
    let videoDecoder = null;
    let audioChannels = Math.max(1, Number(session.audioChannels || 0) || 0);
    let audioSampleRate = Math.max(1, Number(session.audioSampleRate || 0) || 0);
    let audioContext =
      session.audioEnabled && audioChannels > 0 && audioSampleRate > 0
        ? new AudioContext({ sampleRate: audioSampleRate, latencyHint: "interactive" })
        : null;
    const audioDestination = audioContext ? audioContext.createMediaStreamDestination() : null;
    let audioRendererMode = audioContext ? "pending" : "disabled";
    let audioWorkletNode = null;
    let audioProcessor = null;
    let audioWorkletModuleUrl = null;
    const fallbackAudioQueue = [];
    const fallbackAudioNodeState = { chunkOffset: 0 };

    if (audioContext && audioDestination) {
      if (
        typeof AudioWorkletNode === "function" &&
        audioContext.audioWorklet &&
        typeof audioContext.audioWorklet.addModule === "function"
      ) {
        try {
          const workletSource = `
class EquirustNativeAudioProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const processorOptions = options?.processorOptions || {};
    this.channelCount = Math.max(1, Number(processorOptions.channelCount || 2) || 2);
    this.maxQueueFrames = Math.max(
      2048,
      Number(processorOptions.maxQueueFrames || Math.round(sampleRate * 0.5)) || 2048
    );
    this.queue = [];
    this.chunkOffset = 0;
    this.queuedFrames = 0;
    this.dropCount = 0;
    this.port.onmessage = event => {
      const data = event?.data || null;
      if (!data || typeof data !== "object") return;
      if (data.type === "clear") {
        this.queue = [];
        this.chunkOffset = 0;
        this.queuedFrames = 0;
        return;
      }
      if (data.type !== "audio" || !(data.buffer instanceof ArrayBuffer)) return;
      const samples = new Float32Array(data.buffer);
      const frames = Math.floor(samples.length / this.channelCount);
      if (frames <= 0) return;
      this.queue.push(samples);
      this.queuedFrames += frames;
      while (this.queuedFrames > this.maxQueueFrames && this.queue.length > 1) {
        const dropped = this.queue.shift();
        this.queuedFrames -= Math.floor((dropped?.length || 0) / this.channelCount);
        this.chunkOffset = 0;
        this.dropCount += 1;
      }
      if (this.dropCount > 0 && (this.dropCount === 1 || this.dropCount % 120 === 0)) {
        this.port.postMessage({ type: "drop", count: this.dropCount });
      }
    };
  }

  process(_inputs, outputs) {
    const output = outputs[0];
    if (!output || !output.length) return true;
    const frameCount = output[0].length;
    for (let channel = 0; channel < output.length; channel += 1) {
      output[channel].fill(0);
    }

    let frameIndex = 0;
    while (frameIndex < frameCount && this.queue.length) {
      const head = this.queue[0];
      const availableSamples = head.length - this.chunkOffset;
      const availableFrames = Math.floor(availableSamples / this.channelCount);
      if (availableFrames <= 0) {
        this.queue.shift();
        this.chunkOffset = 0;
        continue;
      }

      const framesToCopy = Math.min(frameCount - frameIndex, availableFrames);
      for (let index = 0; index < framesToCopy; index += 1) {
        const sourceOffset = this.chunkOffset + index * this.channelCount;
        for (let channel = 0; channel < output.length; channel += 1) {
          output[channel][frameIndex + index] = head[sourceOffset + channel] ?? 0;
        }
      }

      this.chunkOffset += framesToCopy * this.channelCount;
      this.queuedFrames = Math.max(0, this.queuedFrames - framesToCopy);
      frameIndex += framesToCopy;
      if (this.chunkOffset >= head.length) {
        this.queue.shift();
        this.chunkOffset = 0;
      }
    }

    return true;
  }
}

registerProcessor("equirust-native-audio-processor", EquirustNativeAudioProcessor);
`;
          audioWorkletModuleUrl = URL.createObjectURL(
            new Blob([workletSource], { type: "application/javascript" })
          );
          await audioContext.audioWorklet.addModule(audioWorkletModuleUrl);
          audioWorkletNode = new AudioWorkletNode(
            audioContext,
            "equirust-native-audio-processor",
            {
              numberOfInputs: 0,
              numberOfOutputs: 1,
              outputChannelCount: [audioChannels],
              channelCount: audioChannels,
              channelCountMode: "explicit",
              channelInterpretation: "speakers",
              processorOptions: {
                channelCount: audioChannels,
                maxQueueFrames: Math.max(2048, Math.round(audioSampleRate * 0.5)),
              },
            }
          );
          audioWorkletNode.port.onmessage = event => {
            if (event?.data?.type === "drop") {
              nativeReport(`audio_worklet_drop count=${Number(event.data.count || 0) || 0}`);
            }
          };
          audioWorkletNode.connect(audioDestination);
          audioRendererMode = "worklet";
        } catch (error) {
          if (audioWorkletModuleUrl) {
            URL.revokeObjectURL(audioWorkletModuleUrl);
            audioWorkletModuleUrl = null;
          }
          nativeReport(
            `audio_worklet_init_failed=${normalizeErrorMessage(error)}`,
            { force: true }
          );
        }
      }

      if (!audioWorkletNode) {
        audioProcessor = audioContext.createScriptProcessor(2048, 0, audioChannels);
        audioProcessor.onaudioprocess = event => {
          const outputBuffer = event.outputBuffer;
          const frameCount = outputBuffer.length;
          for (let channel = 0; channel < audioChannels; channel += 1) {
            outputBuffer.getChannelData(channel).fill(0);
          }

          let frameIndex = 0;
          while (frameIndex < frameCount && fallbackAudioQueue.length) {
            const head = fallbackAudioQueue[0];
            const availableSamples = head.length - fallbackAudioNodeState.chunkOffset;
            const availableFrames = Math.floor(availableSamples / audioChannels);
            if (availableFrames <= 0) {
              fallbackAudioQueue.shift();
              fallbackAudioNodeState.chunkOffset = 0;
              continue;
            }

            const framesToCopy = Math.min(frameCount - frameIndex, availableFrames);
            for (let index = 0; index < framesToCopy; index += 1) {
              const sourceOffset = fallbackAudioNodeState.chunkOffset + index * audioChannels;
              for (let channel = 0; channel < audioChannels; channel += 1) {
                outputBuffer.getChannelData(channel)[frameIndex + index] =
                  head[sourceOffset + channel] ?? 0;
              }
            }

            fallbackAudioNodeState.chunkOffset += framesToCopy * audioChannels;
            frameIndex += framesToCopy;
            if (fallbackAudioNodeState.chunkOffset >= head.length) {
              fallbackAudioQueue.shift();
              fallbackAudioNodeState.chunkOffset = 0;
            }
          }
        };
        audioProcessor.connect(audioDestination);
        audioRendererMode = "script_processor";
      }

      nativeReport(`audio_renderer_mode=${audioRendererMode} channels=${audioChannels} sampleRate=${audioSampleRate}`);
      void audioContext.resume().catch(() => {});
    }

    const requestCanvasFrame = () => {
      if (!canvasCaptureTrack || typeof canvasCaptureTrack.requestFrame !== "function") return;
      try {
        canvasCaptureTrack.requestFrame();
      } catch {}
    };
    const requestNativeKeyframe = (reason, details = {}) => {
      if (closed || teardownRequested || videoCodec === "jpeg") {
        return false;
      }
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        return false;
      }
      const now = Date.now();
      if (now - lastNativeKeyframeRequestAt < 800) {
        return false;
      }
      lastNativeKeyframeRequestAt = now;
      nativeKeyframeRequestCount += 1;
      try {
        socket.send(
          JSON.stringify({
            type: "request_keyframe",
            reason: String(reason || "unknown"),
            queue: Math.max(0, Number(details?.queue || 0) || 0),
            dropped: Math.max(0, Number(details?.dropped || 0) || 0),
          })
        );
        if (
          nativeKeyframeRequestCount === 1 ||
          nativeKeyframeRequestCount % 30 === 0
        ) {
          nativeReport(
            `keyframe_request count=${nativeKeyframeRequestCount} reason=${String(
              reason || "unknown"
            )} queue=${Math.max(0, Number(details?.queue || 0) || 0)}`
          );
        }
        return true;
      } catch (error) {
        nativeReport(`keyframe_request_failed=${normalizeErrorMessage(error)}`);
        return false;
      }
    };
    const clearNativePacingRecoveryTimer = () => {
      if (nativePacingRecoveryTimer !== null) {
        window.clearTimeout(nativePacingRecoveryTimer);
        nativePacingRecoveryTimer = null;
      }
    };
    const selectNativePacingHintFrameRate = (baseFrameRate, severity) => {
      const base = Math.max(1, Number(baseFrameRate || 0) || 1);
      const ladder = [base];
      if (base >= 60) {
        ladder.push(45, 30, 20, 15);
      } else if (base >= 45) {
        ladder.push(30, 20, 15);
      } else if (base >= 30) {
        ladder.push(24, 15);
      } else if (base >= 24) {
        ladder.push(20, 15);
      } else {
        ladder.push(15);
      }
      const unique = Array.from(
        new Set(ladder.map(value => Math.min(base, Math.max(10, Number(value || 0) || 10))))
      );
      return unique[Math.min(Math.max(0, severity), unique.length - 1)] || base;
    };
    const sendNativePacingHint = (frameRate, reason, details = {}) => {
      if (closed || teardownRequested) {
        return false;
      }
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        return false;
      }
      const nextFrameRate = Math.min(
        targetVideoFrameRate,
        Math.max(10, Number(frameRate || targetVideoFrameRate) || targetVideoFrameRate)
      );
      const now = Date.now();
      if (
        nextFrameRate === nativePacingHintFrameRate &&
        now - lastNativePacingHintAt < 500
      ) {
        return false;
      }
      lastNativePacingHintAt = now;
      nativePacingHintFrameRate = nextFrameRate;
      try {
        socket.send(
          JSON.stringify({
            type: "set_pacing_hint",
            maxFrameRate: nextFrameRate,
            reason: String(reason || "unknown"),
            queue: Math.max(0, Number(details?.queue || 0) || 0),
            dropped: Math.max(0, Number(details?.dropped || 0) || 0),
            severity: Math.max(0, Number(details?.severity || 0) || 0),
          })
        );
        nativeReport(
          `pacing_hint frameRate=${nextFrameRate} reason=${String(
            reason || "unknown"
          )} severity=${Math.max(0, Number(details?.severity || 0) || 0)}`
        );
        return true;
      } catch (error) {
        nativeReport(`pacing_hint_failed=${normalizeErrorMessage(error)}`);
        return false;
      }
    };
    const scheduleNativePacingRecovery = () => {
      clearNativePacingRecoveryTimer();
      if (nativePacingHintFrameRate >= targetVideoFrameRate) {
        return;
      }
      nativePacingRecoveryTimer = window.setTimeout(() => {
        nativePacingRecoveryTimer = null;
        if (closed || teardownRequested) {
          return;
        }
        if (Date.now() - lastNativeBackpressureAt < 2400) {
          scheduleNativePacingRecovery();
          return;
        }
        nativePacingSeverity = 0;
        sendNativePacingHint(targetVideoFrameRate, "recovery", { severity: 0 });
      }, 2600);
    };
    const noteNativeBackpressure = (reason, severity = 1, details = {}) => {
      const nextSeverity = Math.min(3, Math.max(1, Number(severity || 0) || 1));
      const now = Date.now();
      if (now - lastNativeBackpressureAt > 2000) {
        nativePacingSeverity = 0;
      }
      nativePacingSeverity = Math.max(nativePacingSeverity, nextSeverity);
      lastNativeBackpressureAt = now;
      sendNativePacingHint(
        selectNativePacingHintFrameRate(targetVideoFrameRate, nativePacingSeverity),
        reason,
        {
          ...details,
          severity: nativePacingSeverity,
        }
      );
      scheduleNativePacingRecovery();
    };
    const measureNativeVideoPacketAgeMs = sentAtMicros => {
      const sentAt = Number(sentAtMicros || 0) || 0;
      if (!Number.isFinite(sentAt) || sentAt <= 0) {
        return 0;
      }
      const ageMs = Date.now() - sentAt / 1000;
      if (!Number.isFinite(ageMs)) {
        return 0;
      }
      return Math.max(0, ageMs);
    };
    const dropStaleNativeVideoPacket = (chunkType, sentAtMicros, queueSize = 0) => {
      const ageMs = measureNativeVideoPacketAgeMs(sentAtMicros);
      const isKeyframe = chunkType === "key";
      const tooOld =
        ageMs > (isKeyframe ? maxLiveKeyframeAgeMs : maxLiveVideoPacketAgeMs);
      if (!tooOld && !(waitingForFreshVideoKeyframe && !isKeyframe)) {
        if (isKeyframe) {
          waitingForFreshVideoKeyframe = false;
        }
        return false;
      }

      staleVideoDropCount += 1;
      waitingForFreshVideoKeyframe = true;
      if (staleVideoDropCount === 1 || staleVideoDropCount % 120 === 0) {
        nativeReport(
          `stale_video_drop count=${staleVideoDropCount} ageMs=${Math.round(ageMs)} keyframe=${isKeyframe}`
        );
      }
      noteNativeBackpressure(
        "stale_video_drop",
        ageMs >= 1500 ? 3 : ageMs >= 700 ? 2 : 1,
        {
          queue: Math.max(0, Number(queueSize || 0) || 0),
          dropped: staleVideoDropCount,
        }
      );
      requestNativeKeyframe("stale_video_drop", {
        queue: Math.max(0, Number(queueSize || 0) || 0),
        dropped: staleVideoDropCount,
      });
      return true;
    };
    const emitNativeStreamStats = reason => {
      const deltaPackets =
        nativeVideoPacketsReceived - lastNativeStatsSnapshot.videoPacketsReceived;
      const deltaBytes =
        nativeVideoBytesReceived - lastNativeStatsSnapshot.videoBytesReceived;
      const deltaStaleDrops =
        staleVideoDropCount - lastNativeStatsSnapshot.staleVideoDropCount;
      const deltaDecoderDrops =
        decoderQueueDropCount - lastNativeStatsSnapshot.decoderQueueDropCount;
      const deltaGeneratorDrops =
        generatedVideoDroppedFrames - lastNativeStatsSnapshot.generatedVideoDroppedFrames;
      const deltaKeyframeRequests =
        nativeKeyframeRequestCount - lastNativeStatsSnapshot.nativeKeyframeRequestCount;
      const deltaAgeSamples =
        nativeVideoPacketAgeSamples - lastNativeStatsSnapshot.nativeVideoPacketAgeSamples;
      const deltaAgeTotalMs =
        nativeVideoPacketAgeTotalMs - lastNativeStatsSnapshot.nativeVideoPacketAgeTotalMs;
      const ageAvgMs =
        deltaAgeSamples > 0 ? deltaAgeTotalMs / deltaAgeSamples : 0;
      const ageMaxMs = Math.max(
        0,
        nativeVideoPacketAgeMaxMs,
        lastNativeStatsSnapshot.nativeVideoPacketAgeMaxMs
      );
      lastNativeStatsSnapshot = {
        videoPacketsReceived: nativeVideoPacketsReceived,
        videoBytesReceived: nativeVideoBytesReceived,
        staleVideoDropCount,
        decoderQueueDropCount,
        generatedVideoDroppedFrames,
        nativeKeyframeRequestCount,
        nativeVideoPacketAgeSamples,
        nativeVideoPacketAgeTotalMs,
        nativeVideoPacketAgeMaxMs,
      };
      nativeVideoPacketAgeMaxMs = 0;
      nativeDecodeQueueMax = 0;
      nativeGeneratedQueueMax = 0;
      return nativeReport(
        "stream_stats=" +
          JSON.stringify({
            reason: String(reason || "interval"),
            trackSourceMode: videoTrackSourceMode,
            packets: Math.max(0, deltaPackets),
            kbps: Math.round((Math.max(0, deltaBytes) * 8) / 1024 / 2),
            packetAgeAvgMs: Math.round(ageAvgMs),
            packetAgeMaxMs: Math.round(ageMaxMs),
            staleDrops: Math.max(0, deltaStaleDrops),
            decoderDrops: Math.max(0, deltaDecoderDrops),
            generatorDrops: Math.max(0, deltaGeneratorDrops),
            keyframeRequests: Math.max(0, deltaKeyframeRequests),
            decodeQueueMax: nativeDecodeQueueMax,
            generatorQueueMax: nativeGeneratedQueueMax,
            pacingFps: nativePacingHintFrameRate,
          }),
        { force: true }
      );
    };
    const startNativeStreamStatsTimer = () => {
      if (nativeStreamStatsTimer !== null) {
        return;
      }
      nativeStreamStatsTimer = window.setInterval(() => {
        if (closed || teardownRequested) {
          return;
        }
        void emitNativeStreamStats("interval");
      }, 2000);
    };
    const stopNativeStreamStatsTimer = reason => {
      if (nativeStreamStatsTimer !== null) {
        window.clearInterval(nativeStreamStatsTimer);
        nativeStreamStatsTimer = null;
      }
      return emitNativeStreamStats(reason || "stop");
    };

    const closeGeneratedVideoFrame = frame => {
      if (!frame || typeof frame.close !== "function") return;
      try {
        frame.close();
      } catch {}
    };

    const flushGeneratedVideoQueue = () => {
      while (generatedVideoFrameQueue.length) {
        closeGeneratedVideoFrame(generatedVideoFrameQueue.shift());
      }
    };

    const pumpGeneratedVideoFrames = () => {
      if (!generatedVideoWriter || generatedVideoPumpPromise || closed || teardownRequested) {
        return;
      }

      generatedVideoPumpPromise = (async () => {
        while (!closed && !teardownRequested && generatedVideoFrameQueue.length && generatedVideoWriter) {
          const frame = generatedVideoFrameQueue.shift();
          if (!frame) continue;
          try {
            await generatedVideoWriter.write(frame);
          } finally {
            closeGeneratedVideoFrame(frame);
          }
        }
      })()
        .catch(error => {
          if (closed || teardownRequested) {
            return;
          }
          nativeReport(
            `generator_write_failed=${normalizeErrorMessage(error)}`,
            { force: true }
          );
          void forceStopTracks(false, "generator_write_failed");
        })
        .finally(() => {
          generatedVideoPumpPromise = null;
          if (!closed && !teardownRequested && generatedVideoFrameQueue.length) {
            pumpGeneratedVideoFrames();
          }
        });
    };

    const enqueueGeneratedVideoFrame = frame => {
      if (!frame) return;
      if (!generatedVideoWriter || closed || teardownRequested) {
        closeGeneratedVideoFrame(frame);
        return;
      }

      while (generatedVideoFrameQueue.length >= 1) {
        generatedVideoDroppedFrames += 1;
        closeGeneratedVideoFrame(generatedVideoFrameQueue.shift());
        if (
          generatedVideoDroppedFrames === 1 ||
          generatedVideoDroppedFrames % 120 === 0
        ) {
          nativeReport(`generator_queue_drop count=${generatedVideoDroppedFrames}`);
        }
        noteNativeBackpressure("generator_queue_drop", 1, {
          dropped: generatedVideoDroppedFrames,
        });
      }

      generatedVideoFrameQueue.push(frame);
      nativeGeneratedQueueMax = Math.max(
        nativeGeneratedQueueMax,
        generatedVideoFrameQueue.length
      );
      pumpGeneratedVideoFrames();
    };

    const primeNativePreviewFrame = async () => {
      if (!previewUrl || previewUrl.includes("R0lGODlhAQABAPAAAAAAAAAA")) {
        return;
      }

      let imageSource = null;
      try {
        imageSource = await new Promise((resolve, reject) => {
          const image = new Image();
          image.decoding = "async";
          image.onload = () => resolve(image);
          image.onerror = () => reject(new Error("preview image failed to load"));
          image.src = previewUrl;
        });

        if (closed || !imageSource) {
          return;
        }

        if (workerGeneratedVideo && typeof workerGeneratedVideo.emitProbeFrame === "function") {
          await workerGeneratedVideo.emitProbeFrame(0);
        } else if (generatedVideoWriter && typeof VideoFrame === "function") {
          const frame = new VideoFrame(imageSource, {
            timestamp: 0,
            duration: frameDurationMicros,
          });
          enqueueGeneratedVideoFrame(frame);
        } else {
          const canvasContext = await ensureCanvasVideoPipeline();
          canvasContext.drawImage(imageSource, 0, 0, targetVideoWidth, targetVideoHeight);
          requestCanvasFrame();
        }
        nativeReport(`preview_frame_primed source=${picked.id}`);
      } catch (error) {
        nativeReport(`preview_frame_prime_failed=${normalizeErrorMessage(error)}`);
      } finally {
        closeGeneratedVideoFrame(imageSource);
      }
    };

    if (videoCodec !== "jpeg" && !workerGeneratedVideo) {
      if (typeof VideoDecoder !== "function") {
        if (canvas) {
          canvas.remove();
          canvas = null;
        }
        if (audioProcessor) {
          try {
            audioProcessor.disconnect();
          } catch {}
        }
        if (audioWorkletNode) {
          try {
            audioWorkletNode.disconnect();
          } catch {}
        }
        if (audioWorkletModuleUrl) {
          URL.revokeObjectURL(audioWorkletModuleUrl);
          audioWorkletModuleUrl = null;
        }
        if (audioContext && audioContext.state !== "closed") {
          await audioContext.close().catch(() => {});
        }
        await invoke("stop_desktop_stream_session", { sessionId: session.sessionId }).catch(() => {});
        throw new Error("Desktop stream could not start: H.264 decode is unavailable in this runtime.");
      }

      const decoderConfig = {
        codec: videoCodec,
        optimizeForLatency: true,
        hardwareAcceleration: "prefer-software",
        avc: { format: "annexb" },
      };
      const support = typeof VideoDecoder.isConfigSupported === "function"
        ? await VideoDecoder.isConfigSupported(decoderConfig).catch(() => null)
        : null;
      if (support && support.supported === false) {
        if (canvas) {
          canvas.remove();
          canvas = null;
        }
        if (audioProcessor) {
          try {
            audioProcessor.disconnect();
          } catch {}
        }
        if (audioWorkletNode) {
          try {
            audioWorkletNode.disconnect();
          } catch {}
        }
        if (audioWorkletModuleUrl) {
          URL.revokeObjectURL(audioWorkletModuleUrl);
          audioWorkletModuleUrl = null;
        }
        if (audioContext && audioContext.state !== "closed") {
          await audioContext.close().catch(() => {});
        }
        await invoke("stop_desktop_stream_session", { sessionId: session.sessionId }).catch(() => {});
        throw new Error("Desktop stream could not start: H.264 decode is unsupported in this runtime.");
      }

      videoDecoder = new VideoDecoder({
        output: frame => {
          if (generatedVideoWriter) {
            enqueueGeneratedVideoFrame(frame);
            return;
          }

          try {
            context?.drawImage(frame, 0, 0, targetVideoWidth, targetVideoHeight);
            requestCanvasFrame();
          } finally {
            closeGeneratedVideoFrame(frame);
          }
        },
        error: error => {
          console.warn("[Equirust] Desktop stream decode error:", error);
          nativeReport(`decoder_error=${normalizeErrorMessage(error)}`, { force: true });
          void forceStopTracks(false, "decoder_error");
        },
      });
      videoDecoder.configure(support?.config || decoderConfig);
    }

    let stream = null;
    if (generatedVideoTrack) {
      stream = new MediaStream([generatedVideoTrack]);
    } else {
      // Prefer a pull-driven canvas stream so each requestFrame lines up with a freshly
      // decoded desktop stream frame instead of a separate timer-driven capture clock.
      stream = canvas.captureStream(0);
      canvasCaptureTrack = stream.getVideoTracks?.()?.[0] || null;
      if (!canvasCaptureTrack) {
        stream = canvas.captureStream(targetVideoFrameRate);
        canvasCaptureTrack = stream.getVideoTracks?.()?.[0] || null;
      }
    }
    if (audioDestination?.stream?.getAudioTracks?.().length) {
      const audioTrack = audioDestination.stream.getAudioTracks()[0];
      if (audioTrack) {
        stream.addTrack(audioTrack);
      }
    }
    startNativeStreamStatsTimer();
    const tracks = stream.getTracks();
    const nativeVideoTrack = stream.getVideoTracks?.()?.[0] || null;
    let forcingTrackStop = false;
    const nativeStreamMeta = {
      sessionId: String(session.sessionId || ""),
      sourceId: String(picked.id || ""),
      sourceKind,
      baseWidth: Math.max(2, Number(session.width || picked.width || targetVideoWidth) || targetVideoWidth),
      baseHeight: Math.max(2, Number(session.height || picked.height || targetVideoHeight) || targetVideoHeight),
      baseFrameRate: Math.max(1, Number(session.frameRate || picked.frameRate || 60) || 60),
      encoderMode,
      encoderDetail,
      contentHint: picked.contentHint === "detail" ? "detail" : "motion",
      videoIngressMode: videoTrackSourceMode,
    };
    const appliedNativeQuality = {
      frameRate: nativeStreamMeta.baseFrameRate,
      width: nativeStreamMeta.baseWidth,
      height: nativeStreamMeta.baseHeight,
      resolutionMode: String(nativeStreamMeta.baseHeight),
    };
    state.pendingScreenShareQuality = { ...appliedNativeQuality };
    window.__EQUIRUST_PENDING_NATIVE_SHARE_QUALITY__ = { ...appliedNativeQuality };
    setDesktopStreamStreamMeta(stream, nativeStreamMeta);
    if (nativeVideoTrack) {
      setDesktopStreamTrackMeta(nativeVideoTrack, nativeStreamMeta);
    }

    const snapshotTrackState = track => {
      let settings = null;
      try {
        settings = typeof track?.getSettings === "function" ? track.getSettings() : null;
      } catch {}
      return {
        kind: String(track?.kind || "<unknown>"),
        enabled: track?.enabled === true,
        muted: track?.muted === true,
        readyState: String(track?.readyState || "<unknown>"),
        id: String(track?.id || ""),
        width: Number(settings?.width || 0) || null,
        height: Number(settings?.height || 0) || null,
        frameRate: Number(settings?.frameRate || 0) || null,
      };
    };

    const snapshotNativeStreamState = () =>
      JSON.stringify({
        streamActive: stream?.active === true,
        closed,
        stopIssued,
        socketReadyState: Number(socket?.readyState ?? -1),
        visibilityState: document.visibilityState,
        tracks: tracks.map(snapshotTrackState),
      });

    const emitSyntheticTrackEnded = track => {
      try {
        track.dispatchEvent?.(new Event("ended"));
      } catch (error) {
        nativeReport(`track_ended_dispatch_failed kind=${track?.kind || "<unknown>"} message=${normalizeErrorMessage(error)}`, {
          force: true,
        });
      }
    };

    let stopSessionTimer = null;
    let stopSessionRequest = null;

    const clearScheduledStopSession = () => {
      if (stopSessionTimer !== null) {
        window.clearTimeout(stopSessionTimer);
        stopSessionTimer = null;
      }
    };

    const scheduleStopSession = (stopNativeSession, reason = "unknown", delayMs = 64) => {
      if (closed) return;
      if (stopSessionRequest) {
        stopSessionRequest.stopNativeSession =
          stopSessionRequest.stopNativeSession || stopNativeSession === true;
        stopSessionRequest.reason = String(reason || stopSessionRequest.reason || "unknown");
        return;
      }
      stopSessionRequest = {
        stopNativeSession: stopNativeSession === true,
        reason: String(reason || "unknown"),
      };
      clearScheduledStopSession();
      stopSessionTimer = window.setTimeout(() => {
        const request = stopSessionRequest;
        stopSessionRequest = null;
        stopSessionTimer = null;
        if (!request) return;
        void stopSession(request.stopNativeSession, request.reason);
      }, Math.max(0, Number(delayMs || 0) || 0));
    };

    const stopSession = async (stopNativeSession, reason = "unknown") => {
      if (closed) return;
      closed = true;
      teardownRequested = true;
      clearScheduledStopSession();
      stopSessionRequest = null;
      rememberDesktopStreamClosingSession(session.sessionId);
      stopScreenShareReinforcement(stream);
      stopNativeAbrControllersForSession(session.sessionId, tracks);
      tracks.forEach(track => {
        desktopStreamClosingTracks.add(track);
        if (track.kind === "video") {
          desktopStreamTrackMeta.delete(track);
        }
      });
      nativeReport(
        `stop_session reason=${reason} stopNativeSession=${stopNativeSession} stopIssued=${stopIssued}`
      );

      try {
        if (socket && socket.readyState === WebSocket.OPEN) {
          socket.close();
        }
      } catch {}

      socket = null;
      pendingFrame = null;
      clearNativePacingRecoveryTimer();
      await stopNativeStreamStatsTimer("stop_session").catch(() => {});
      if (typeof stream.id === "string" && stream.id) {
        desktopStreamStreamMeta.delete(stream.id);
      }
      flushGeneratedVideoQueue();
      if (generatedVideoPumpPromise) {
        await generatedVideoPumpPromise.catch(() => {});
      }
      if (generatedVideo && typeof generatedVideo.close === "function") {
        await generatedVideo.close().catch(() => {});
        generatedVideoWriter = null;
      }
      if (canvas) {
        canvas.remove();
        canvas = null;
      }
      if (videoDecoder && videoDecoder.state !== "closed") {
        try {
          await videoDecoder.flush();
        } catch {}
        try {
          videoDecoder.close();
        } catch {}
      }
      fallbackAudioQueue.length = 0;
      if (audioProcessor) {
        try {
          audioProcessor.disconnect();
        } catch {}
      }
      if (audioWorkletNode) {
        try {
          audioWorkletNode.port.postMessage({ type: "clear" });
        } catch {}
        try {
          audioWorkletNode.disconnect();
        } catch {}
        audioWorkletNode = null;
      }
      if (audioDestination) {
        try {
          audioDestination.disconnect?.();
        } catch {}
      }
      if (audioWorkletModuleUrl) {
        URL.revokeObjectURL(audioWorkletModuleUrl);
        audioWorkletModuleUrl = null;
      }
      if (audioContext && audioContext.state !== "closed") {
        await audioContext.close().catch(() => {});
      }

      if (stopNativeSession && !stopIssued) {
        stopIssued = true;
        await invoke("stop_desktop_stream_session", { sessionId: session.sessionId }).catch(error => {
          nativeReport(`stop_native_failed=${normalizeErrorMessage(error)}`, { force: true });
        });
      }
      nativeReport(`stop_session_complete state=${snapshotNativeStreamState()}`);
    };

    const forceStopTracks = async (stopNativeSession, reason = "unknown") => {
      if (closed || teardownRequested) {
        nativeReport(`force_stop_tracks_ignored reason=${reason} closed=${closed} teardownRequested=${teardownRequested}`);
        return;
      }
      teardownRequested = true;
      nativeReport(`force_stop_tracks reason=${reason} stopNativeSession=${stopNativeSession}`);
      nativeReport(`force_stop_tracks_state_before=${snapshotNativeStreamState()}`);
      const shouldEmitEnded =
        String(reason || "").startsWith("ws_control:") ||
        reason === "ws_error" ||
        reason === "ws_close" ||
        reason === "decoder_error" ||
        reason === "jpeg_decode_failed" ||
        reason === "packet_decode_failed" ||
        reason === "oversized_video_packet" ||
        reason === "oversized_audio_packet";
      forcingTrackStop = true;
      tracks.forEach(track => {
        try {
          track.enabled = false;
        } catch {}
        try {
          track.stop();
        } catch {}
        if (shouldEmitEnded) {
          emitSyntheticTrackEnded(track);
        }
      });
      forcingTrackStop = false;
      nativeReport(`force_stop_tracks_state_after=${snapshotNativeStreamState()}`);
      scheduleStopSession(stopNativeSession, `force_stop_tracks:${reason}`, 96);
    };

    handleGeneratedVideoEvent = event => {
      const payload = event && typeof event === "object" ? event : null;
      if (!payload) {
        return;
      }
      if (payload.type === "metric") {
        if (payload.name === "generator_queue_drop") {
          nativeReport(
            `worker_generator_queue_drop count=${Math.max(
              0,
              Number(payload.count || 0) || 0
            )}`
          );
          noteNativeBackpressure("worker_generator_queue_drop", 1, {
            dropped: Math.max(0, Number(payload.count || 0) || 0),
          });
          return;
        }
        if (payload.name === "decoder_queue_drop") {
          const queue = Math.max(0, Number(payload.queue || 0) || 0);
          const dropped = Math.max(0, Number(payload.count || 0) || 0);
          nativeReport(
            `worker_decoder_queue_drop count=${dropped} queue=${queue}`
          );
          noteNativeBackpressure(
            "worker_decoder_queue_drop",
            queue >= 6 || dropped >= 24 ? 3 : queue >= 4 || dropped >= 8 ? 2 : 1,
            {
              queue,
              dropped,
            }
          );
          requestNativeKeyframe("worker_decoder_queue_drop", {
            queue,
            dropped,
          });
        }
        return;
      }
      if (payload.type === "error") {
        const code =
          typeof payload.code === "string" && payload.code.trim()
            ? payload.code.trim()
            : "worker_error";
        nativeReport(
          `${code}=${normalizeErrorMessage(payload.message || code)}`,
          { force: true }
        );
        void forceStopTracks(false, code);
      }
    };

    tracks.forEach(track => {
      track.contentHint = picked.contentHint === "detail" ? "detail" : "motion";
      const shouldStopSessionForTrack = track.kind === "video";
      try {
        const originalStop = track.stop.bind(track);
        track.stop = () => {
          if (forcingTrackStop) {
            nativeReport(`track_stop_forced kind=${track.kind} state=${JSON.stringify(snapshotTrackState(track))}`);
          } else {
            nativeReport(`track_stop_requested kind=${track.kind} state_before=${JSON.stringify(snapshotTrackState(track))}`);
          }
          originalStop();
          if (!forcingTrackStop && shouldStopSessionForTrack) {
            nativeReport(`track_stop_completed kind=${track.kind} state_after=${JSON.stringify(snapshotTrackState(track))}`);
            scheduleStopSession(true, `track_stop:${track.kind}`, 96);
          } else if (!forcingTrackStop) {
            nativeReport(`track_stop_ignored kind=${track.kind}`);
          }
        };
      } catch {}
      track.addEventListener("ended", () => {
        nativeReport(`track_ended kind=${track.kind} state=${JSON.stringify(snapshotTrackState(track))}`);
        if (shouldStopSessionForTrack) {
          scheduleStopSession(true, `track_ended:${track.kind}`, 96);
        } else {
          nativeReport(`track_ended_ignored kind=${track.kind}`);
        }
      });
      track.addEventListener("mute", () => {
        nativeReport(`track_muted kind=${track.kind} state=${JSON.stringify(snapshotTrackState(track))}`);
      });
      track.addEventListener("unmute", () => {
        nativeReport(`track_unmuted kind=${track.kind} state=${JSON.stringify(snapshotTrackState(track))}`);
      });
    });
    try {
      stream.addEventListener("inactive", () => {
        nativeReport(`stream_inactive state=${snapshotNativeStreamState()}`);
        scheduleStopSession(true, "stream_inactive", 96);
      });
    } catch {}

    const drawFrame = async (bytes, timestampMicros = 0) => {
      decodeBusy = true;
      try {
        const blob = new Blob([bytes], { type: "image/jpeg" });
        const bitmap = await createImageBitmap(blob);
        if (generatedVideoWriter && typeof VideoFrame === "function") {
          const frame = new VideoFrame(bitmap, {
            timestamp: Math.max(0, Math.round(Number(timestampMicros || 0) || 0)),
            duration: frameDurationMicros,
          });
          bitmap.close();
          enqueueGeneratedVideoFrame(frame);
        } else {
          context?.drawImage(bitmap, 0, 0, targetVideoWidth, targetVideoHeight);
          bitmap.close();
          requestCanvasFrame();
        }
      } catch (error) {
        nativeReport(`jpeg_decode_failed=${normalizeErrorMessage(error)}`, { force: true });
        void forceStopTracks(false, "jpeg_decode_failed");
      } finally {
        decodeBusy = false;
        if (pendingFrame && !closed && !teardownRequested) {
          const nextFrame = pendingFrame;
          pendingFrame = null;
          void drawFrame(nextFrame.bytes, nextFrame.timestamp);
        }
      }
    };

    const handleFrame = payload => {
      if (closed || teardownRequested) return;
      const bytes = new Uint8Array(payload);
      if (bytes.byteLength <= nativeVideoPacketHeaderBytes || bytes[0] !== 0x01) return;
      if (bytes.byteLength > maxNativeVideoPacketBytes + nativeVideoPacketHeaderBytes) {
        nativeReport(`oversized_video_packet bytes=${bytes.byteLength}`, { force: true });
        void forceStopTracks(false, "oversized_video_packet");
        return;
      }
      const flags = bytes[1];
      const timestamp = Number(
        new DataView(bytes.buffer, bytes.byteOffset + 2, 8).getBigUint64(0, true)
      );
      const sentAtMicros = Number(
        new DataView(bytes.buffer, bytes.byteOffset + 10, 8).getBigUint64(0, true)
      );
      const frameBytes = bytes.slice(nativeVideoPacketHeaderBytes);
      if (!frameBytes.byteLength) {
        nativeReport("empty_video_packet", { force: true });
        return;
      }
      const chunkType = (flags & 0x01) === 0x01 ? "key" : "delta";
      const decoderQueueSize = Number(videoDecoder?.decodeQueueSize || 0) || 0;
      const packetAgeMs = measureNativeVideoPacketAgeMs(sentAtMicros);
      nativeVideoPacketsReceived += 1;
      nativeVideoBytesReceived += frameBytes.byteLength;
      nativeVideoPacketAgeSamples += 1;
      nativeVideoPacketAgeTotalMs += packetAgeMs;
      nativeVideoPacketAgeMaxMs = Math.max(nativeVideoPacketAgeMaxMs, packetAgeMs);
      nativeDecodeQueueMax = Math.max(nativeDecodeQueueMax, decoderQueueSize);

      if (dropStaleNativeVideoPacket(chunkType, sentAtMicros, decoderQueueSize)) {
        return;
      }

      if (videoCodec === "jpeg") {
        if (chunkType === "key") {
          waitingForFreshVideoKeyframe = false;
        }
        if (workerGeneratedVideo) {
          const packetBuffer = frameBytes.buffer.slice(
            frameBytes.byteOffset,
            frameBytes.byteOffset + frameBytes.byteLength
          );
          if (
            !workerGeneratedVideo.postVideoPacket({
              codec: videoCodec,
              chunkType,
              timestampMicros: timestamp,
              durationMicros: frameDurationMicros,
              buffer: packetBuffer,
            })
          ) {
            nativeReport("worker_post_video_packet_failed=jpeg", { force: true });
            void forceStopTracks(false, "worker_post_video_packet_failed");
          }
          return;
        }
        if (decodeBusy) {
          pendingFrame = {
            bytes: frameBytes,
            timestamp,
          };
          return;
        }
        void drawFrame(frameBytes, timestamp);
        return;
      }

      if (workerGeneratedVideo) {
        if (chunkType === "key") {
          waitingForFreshVideoKeyframe = false;
        }
        const packetBuffer = frameBytes.buffer.slice(
          frameBytes.byteOffset,
          frameBytes.byteOffset + frameBytes.byteLength
        );
        if (
          !workerGeneratedVideo.postVideoPacket({
            codec: videoCodec,
            chunkType,
            timestampMicros: timestamp,
            durationMicros: frameDurationMicros,
            buffer: packetBuffer,
          })
        ) {
          nativeReport("worker_post_video_packet_failed=h264", { force: true });
          void forceStopTracks(false, "worker_post_video_packet_failed");
        }
        return;
      }

      if (!videoDecoder || videoDecoder.state === "closed") return;
      if (chunkType === "key") {
        decoderQueueDropCount = 0;
        waitingForFreshVideoKeyframe = false;
      }
      if (videoDecoder.decodeQueueSize > 1 && chunkType !== "key") {
        decoderQueueDropCount += 1;
        if (decoderQueueDropCount === 1 || decoderQueueDropCount % 120 === 0) {
          nativeReport(
            `decoder_queue_drop count=${decoderQueueDropCount} queue=${Number(videoDecoder.decodeQueueSize || 0) || 0}`
          );
        }
        noteNativeBackpressure(
          "decoder_queue_drop",
          Number(videoDecoder.decodeQueueSize || 0) >= 4 || decoderQueueDropCount >= 24
            ? 3
            : Number(videoDecoder.decodeQueueSize || 0) >= 2 || decoderQueueDropCount >= 8
            ? 2
            : 1,
          {
            queue: Number(videoDecoder.decodeQueueSize || 0) || 0,
            dropped: decoderQueueDropCount,
          }
        );
        requestNativeKeyframe("decoder_queue_drop", {
          queue: Number(videoDecoder.decodeQueueSize || 0) || 0,
          dropped: decoderQueueDropCount,
        });
        return;
      }

      try {
        videoDecoder.decode(
          new EncodedVideoChunk({
            type: chunkType,
            timestamp,
            duration: frameDurationMicros,
            data: frameBytes,
          })
        );
      } catch (error) {
        console.warn("[Equirust] Desktop stream packet decode failed:", error);
        nativeReport(`packet_decode_failed=${normalizeErrorMessage(error)}`, { force: true });
        void forceStopTracks(false, "packet_decode_failed");
      }
    };

    const handleAudio = payload => {
      if (closed || teardownRequested) return;
      if (!audioContext || audioChannels <= 0) return;
      const bytes = new Uint8Array(payload);
      if (!bytes.length || bytes[0] !== 0x02) return;
      if (bytes.byteLength > maxNativeAudioPacketBytes + 1) {
        nativeReport(`oversized_audio_packet bytes=${bytes.byteLength}`, { force: true });
        void forceStopTracks(false, "oversized_audio_packet");
        return;
      }
      const pcmBytes = bytes.slice(1);
      if (pcmBytes.byteLength % 4 !== 0) {
        nativeReport(`misaligned_audio_packet bytes=${pcmBytes.byteLength}`, { force: true });
        return;
      }
      const pcmBuffer = pcmBytes.buffer.slice(
        pcmBytes.byteOffset,
        pcmBytes.byteOffset + pcmBytes.byteLength
      );
      if (audioWorkletNode) {
        audioWorkletNode.port.postMessage({
          type: "audio",
          buffer: pcmBuffer,
        }, [pcmBuffer]);
      } else if (audioProcessor) {
        const samples = new Float32Array(pcmBuffer);
        fallbackAudioQueue.push(samples);
        if (fallbackAudioQueue.length > 32) {
          fallbackAudioQueue.splice(0, fallbackAudioQueue.length - 32);
        }
      } else {
        return;
      }
      if (audioContext.state === "suspended") {
        void audioContext.resume().catch(() => {});
      }
    };

    await primeNativePreviewFrame();

    await new Promise((resolve, reject) => {
      let socketOpened = false;
      let helloReceived = false;
      const timeout = window.setTimeout(() => {
        if (socketOpened || closed) return;
        nativeReport("socket_open_timeout", { force: true });
        void stopSession(true, "socket_open_timeout");
        reject(new Error("Desktop stream could not start: transport connection timed out."));
      }, 3000);

      socket = new WebSocket(session.websocketUrl);
      socket.binaryType = "arraybuffer";
      socket.onopen = () => {
        nativeReport("ws_open");
        socketOpened = true;
        window.clearTimeout(timeout);
        requestNativeKeyframe("stream_start", { queue: 0, dropped: 0 });
        resolve();
      };

      socket.onmessage = event => {
        if (typeof event.data === "string") {
          let payload = null;
          try {
            payload = JSON.parse(event.data);
          } catch {
            return;
          }

          if (payload?.type === "hello") {
            helloReceived = true;
            const helloSink = normalizeDesktopStreamSinkDescriptor(payload?.sink);
            nativeReport(
              `ws_hello codec=${String(payload?.video?.codec || videoCodec)} audio=${payload?.audio?.enabled === true} adapter=${String(payload?.source?.adapterName || "<unknown>")} output=${String(payload?.source?.outputName || "<unknown>")} sink=${helloSink.sinkKind} videoIngress=${helloSink.preferredVideoIngress} transport=${helloSink.transportKind}`
            );
            return;
          }

          if (
            payload?.type === "source_closed" ||
            payload?.type === "fatal" ||
            payload?.type === "ended" ||
            payload?.type === "audio_device_lost"
          ) {
            const payloadType = String(payload?.type || "unknown");
            const payloadMessage =
              typeof payload?.message === "string" && payload.message.trim()
                ? payload.message.trim()
                : "<none>";
            nativeReport(
              `ws_control type=${payloadType} helloReceived=${helloReceived} message=${payloadMessage}`,
              { force: payloadType !== "ended" }
            );
            if (!socketOpened) {
              window.clearTimeout(timeout);
              void stopSession(true, `ws_control_before_open:${payloadType}`);
              reject(new Error(payload?.message || "Desktop stream could not start."));
              return;
            }
            void forceStopTracks(true, `ws_control:${payloadType}`);
          }
          return;
        }

        if (event.data instanceof ArrayBuffer) {
          const bytes = new Uint8Array(event.data);
          if (!bytes.length) return;
          if (bytes[0] === 0x01) {
            handleFrame(event.data);
          } else if (bytes[0] === 0x02) {
            handleAudio(event.data);
          }
        } else if (event.data?.arrayBuffer) {
          void event.data
            .arrayBuffer()
            .then(buffer => {
              const bytes = new Uint8Array(buffer);
              if (!bytes.length) return;
              if (bytes[0] === 0x01) {
                handleFrame(buffer);
              } else if (bytes[0] === 0x02) {
                handleAudio(buffer);
              }
            })
            .catch(() => {});
        }
      };

      socket.onerror = event => {
        nativeReport(
          `ws_error socketOpened=${socketOpened} helloReceived=${helloReceived} hasEvent=${event ? "true" : "false"}`,
          { force: true }
        );
        if (!socketOpened) {
          window.clearTimeout(timeout);
          void stopSession(true, "ws_error_before_open");
          reject(new Error("Desktop stream could not start: transport connection failed."));
        } else {
          if (!teardownRequested) {
            void forceStopTracks(false, "ws_error");
          }
        }
      };

      socket.onclose = event => {
        nativeReport(
          `ws_close code=${Number(event?.code || 0)} clean=${event?.wasClean === true} socketOpened=${socketOpened} helloReceived=${helloReceived} reason=${String(event?.reason || "")} closed=${closed}`,
          { force: true }
        );
        if (!socketOpened && !closed) {
          window.clearTimeout(timeout);
          void stopSession(true, "ws_close_before_open");
          reject(new Error("Desktop stream could not start: transport closed before startup."));
          return;
        }
        if (!closed && !teardownRequested) {
          void forceStopTracks(false, "ws_close");
        }
      };
    });

    return stream;
  };

  const installDisplayMediaCompatibilityPatches = () => {
    if (state.displayMediaCompatReady || !isDiscordHost()) return;
    if (typeof navigator.mediaDevices?.getDisplayMedia !== "function") return;
    try {
      const originalGetDisplayMedia = navigator.mediaDevices.getDisplayMedia.bind(navigator.mediaDevices);

      const normalizeDisplayMediaRequest = options => {
        const quality = readScreenShareQuality();
        const next = options && typeof options === "object" ? { ...options } : {};

        next.systemAudio ??= "include";
        next.surfaceSwitching ??= "include";
        next.monitorTypeSurfaces ??= "include";

      if (next.audio) {
        if (next.audio === true) {
          next.audio = {};
        } else if (typeof next.audio !== "object") {
          next.audio = {};
        } else {
          next.audio = { ...next.audio };
        }

        next.audio.suppressLocalAudioPlayback ??= false;
        next.audio.echoCancellation ??= false;
        next.audio.noiseSuppression ??= false;
        next.audio.autoGainControl ??= false;
        next.audio.channelCount ??= 2;
        next.audio.sampleRate ??= 48000;
      }

      if (next.video !== false) {
        if (next.video === true || typeof next.video !== "object") {
          next.video = {};
        } else {
          next.video = { ...next.video };
        }

        next.video.frameRate ??= { ideal: quality.frameRate, max: quality.frameRate };
        next.video.width ??= { ideal: quality.width, max: quality.width };
        next.video.height ??= { ideal: quality.height, max: quality.height };
        next.video.resizeMode ??= "none";
      }

        return next;
      };

      navigator.mediaDevices.getDisplayMedia = async function(options) {
      const requestedAudio = options?.audio !== false;
      const sources = await loadScreenShareSources("initial").catch(error => {
        console.warn("[Equirust] Failed to load capturer sources", error);
        return [];
      });
      const picked = sources.length
        ? await openScreenSharePicker(sources, {
            audio: requestedAudio,
          })
        : null;
      if (!picked) {
        throw createAbortError("Screen share was cancelled.");
      }

      const normalized = normalizeDisplayMediaRequest(options);
      const quality = {
        frameRate: Number(picked.frameRate || 60),
        width: Number(picked.width || 1920),
        height: Number(picked.height || 1080),
        resolutionMode:
          String(picked.resolutionMode || "").toLowerCase() === "source"
            ? "source"
            : String(Number(picked.height || 1080) || 1080),
      };
      state.pendingScreenShareQuality = { ...quality };
      window.__EQUIRUST_PENDING_NATIVE_SHARE_QUALITY__ = { ...quality };
      installNitroStreamQualityBypassPatch();
      installGoLiveQualityPatch();
      installGoLiveDispatchPatch();
      primeGoLiveQualityForCurrentConnections(quality);
      if (!picked.audio) {
        normalized.audio = false;
        normalized.systemAudio = "exclude";
      } else {
        normalized.audio = normalized.audio || {};
        normalized.systemAudio = "include";
      }
      normalized.surfaceSwitching = "exclude";
      normalized.selfBrowserSurface ??= "exclude";
      normalized.monitorTypeSurfaces = picked.kind === "screen" ? "include" : "exclude";
      if (normalized.video && typeof normalized.video === "object") {
        normalized.video.preferCurrentTab = false;
        normalized.video.logicalSurface ??= picked.kind === "window";
        normalized.video.displaySurface ??= picked.kind === "window" ? "window" : "monitor";
      }
      report(
        "display_media_request=" +
          JSON.stringify({
            pickedId: picked.id,
            pickedKind: picked.kind,
            audio: Boolean(normalized.audio),
            frameRate: quality.frameRate,
            width: quality.width,
            height: quality.height,
            systemAudio: normalized.systemAudio ?? null,
            native: supportsNativeWindowsScreenShare(),
          })
      );

      let stream;
      try {
        stream = supportsNativeWindowsScreenShare()
          ? await startNativeDisplayMediaStream(picked)
          : await originalGetDisplayMedia(normalized);
      } catch (error) {
        report(
          "display_media_start_failed=" +
            JSON.stringify({
              pickedId: picked.id,
              pickedKind: picked.kind,
              native: supportsNativeWindowsScreenShare(),
              frameRate: quality.frameRate,
              width: quality.width,
              height: quality.height,
              message:
                error && typeof error.message === "string"
                  ? error.message
                  : String(error),
            }),
          { force: true }
        );
        throw error;
      }
      const videoTrack = stream.getVideoTracks()[0];
      const effectiveQuality = supportsNativeWindowsScreenShare()
        ? resolveEffectiveNativeShareQuality(stream, quality)
        : quality;
      if (
        effectiveQuality.width !== quality.width ||
        effectiveQuality.height !== quality.height ||
        effectiveQuality.frameRate !== quality.frameRate
      ) {
        state.pendingScreenShareQuality = { ...effectiveQuality };
        window.__EQUIRUST_PENDING_NATIVE_SHARE_QUALITY__ = { ...effectiveQuality };
        report(
          "desktop_stream_effective_quality=" +
            JSON.stringify({
              requestedWidth: quality.width,
              requestedHeight: quality.height,
              requestedFrameRate: quality.frameRate,
              appliedWidth: effectiveQuality.width,
              appliedHeight: effectiveQuality.height,
              appliedFrameRate: effectiveQuality.frameRate,
            })
        );
      }

      if (videoTrack) {
        await applyScreenShareTrackConstraints(videoTrack, effectiveQuality, picked.contentHint);
      }
      primeGoLiveQualityForCurrentConnections(effectiveQuality);
      reinforceScreenShareQuality(stream, effectiveQuality, picked.contentHint);

      const settings = videoTrack?.getSettings?.() || {};
      report(
        "display_media_result=" +
          JSON.stringify({
            videoTracks: stream.getVideoTracks().length,
            audioTracks: stream.getAudioTracks().length,
            width: settings.width ?? null,
            height: settings.height ?? null,
            frameRate: settings.frameRate ?? null,
            displaySurface: settings.displaySurface ?? null,
          })
      );

      return stream;
      };

      state.displayMediaCompatReady = true;
      report("display_media_compat_installed=true");
    } catch (error) {
      state.displayMediaCompatReady = false;
      report(
        `display_media_compat_install_failed=${
          error && error.message ? error.message : String(error)
        }`,
        { force: true }
      );
    }
  };



