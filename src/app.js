/* yt-live-wallpaper — library window logic.
   Vanilla JS, no bundler. Runs against Tauri commands; falls back to an
   in-memory demo when opened in a plain browser (design preview only). */

(() => {
  "use strict";

  const TAURI = window.__TAURI__;
  const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
  const listen = TAURI && TAURI.event ? TAURI.event.listen : null;
  const isTauri = typeof invoke === "function";

  const $ = (selector) => document.querySelector(selector);

  const els = {
    addForm: $("#add-form"),
    urlInput: $("#url-input"),
    urlState: $("#url-state"),
    addBtn: $("#add-btn"),
    qualitySelect: $("#quality-select"),
    fitSelect: $("#fit-select"),
    engineChip: $("#engine-chip"),
    engineChipText: $("#engine-chip-text"),
    composerNote: $("#composer-note"),

    spotPreview: $("#spot-preview"),
    spotThumb: $("#spot-thumb"),
    spotEmpty: $("#spot-empty"),
    spotEmptyText: $("#spot-empty-text"),
    spotPreviewStatus: $("#spot-preview-status"),
    spotPreviewLabel: $("#spot-preview-label"),
    spotStatus: $("#spot-status"),
    spotStatusText: $("#spot-status-text"),
    spotTitle: $("#spot-title"),
    spotSub: $("#spot-sub"),
    spotRes: $("#spot-res"),
    spotQuality: $("#spot-quality"),
    spotFit: $("#spot-fit"),
    spotEngine: $("#spot-engine"),
    spotFps: $("#spot-fps"),
    spotPause: $("#spot-pause"),
    pauseLabel: $("#pause-label"),
    pausePath: $("#pause-icon path"),
    spotStop: $("#spot-stop"),

    searchInput: $("#search-input"),
    searchShell: $("#search-shell"),
    libraryGrid: $("#library-grid"),
    libraryEmpty: $("#library-empty"),
    libraryCount: $("#library-count"),

    pauseFullscreen: $("#pause-fullscreen"),
    pauseMaximized: $("#pause-maximized"),
    autostart: $("#autostart"),
    startMinimized: $("#start-minimized"),
    ytdlpPath: $("#ytdlp-path"),
    checkYtdlp: $("#check-ytdlp"),
    ytdlpStatus: $("#ytdlp-status"),
    ytdlpStatusText: $("#ytdlp-status-text"),
    ytdlpVersion: $("#ytdlp-version"),

    telNode: $("#tel-node"),
    telCpu: $("#tel-cpu"),
    telWebview: $("#tel-webview"),
    telMode: $("#tel-mode"),

    toastStack: $("#toast-stack"),
    btnMinimize: $("#btn-minimize"),
    btnClose: $("#btn-close"),
  };

  const DEMO_LIBRARY = [
    {
      id: "demo-1",
      video_id: "LXb3EKWsInQ",
      url: "https://www.youtube.com/watch?v=LXb3EKWsInQ",
      title: "Costa Rica in 4K 60fps HDR (ULTRA HD)",
      thumbnail_url: "https://i.ytimg.com/vi/LXb3EKWsInQ/hqdefault.jpg",
      quality: "4k",
      fit_mode: "cover",
      added_at: "2026-11-18T10:00:00Z",
      last_used_at: "2026-11-21T09:00:00Z",
      channel: "Jacob + Katie Schwarz",
    },
    {
      id: "demo-2",
      video_id: "aqz-KE-bpKQ",
      url: "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
      title: "Big Buck Bunny 60fps 4K — Blender Open Movie",
      thumbnail_url: "https://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg",
      quality: "1080p",
      fit_mode: "cover",
      added_at: "2026-11-17T18:30:00Z",
      last_used_at: "2026-11-19T20:10:00Z",
      channel: "Blender",
    },
    {
      id: "demo-3",
      video_id: "dQw4w9WgXcQ",
      url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      title: "Rick Astley — Never Gonna Give You Up (Official Video)",
      thumbnail_url: "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg",
      quality: "1080p",
      fit_mode: "contain",
      added_at: "2026-11-15T12:00:00Z",
      last_used_at: null,
      channel: "Rick Astley",
    },
    {
      id: "demo-4",
      video_id: "hHW1oY26kxQ",
      url: "https://www.youtube.com/watch?v=hHW1oY26kxQ",
      title: "4K Aquarium — Relaxing Coral Reef",
      thumbnail_url: "https://i.ytimg.com/vi/hHW1oY26kxQ/hqdefault.jpg",
      quality: "1440p",
      fit_mode: "cover",
      added_at: "2026-11-12T08:20:00Z",
      last_used_at: null,
      channel: "4K Relaxation Channel",
    },
  ];

  const idlePlayback = () => ({
    id: null,
    videoId: null,
    title: null,
    thumbnailUrl: null,
    quality: "1080p",
    fit: "cover",
    engine: "idle",
    streamUrl: null,
    paused: false,
    pauseReason: null,
    streamWidth: 0,
    streamHeight: 0,
    fps: 0,
  });

  let state = {
    playback: idlePlayback(),
    settings: {
      pauseFullscreen: true,
      pauseMaximized: true,
      autostart: false,
      startMinimized: false,
      ytdlpPath: "",
      quality: "1080p",
      fit: "cover",
    },
    ytdlp: { found: false, path: null, version: null },
    library: [],
  };

  let addBusy = false;
  let telemetryTimer = 0;
  let refreshing = false;

  // ------------------------------------------------------------------ helpers

  const escapeHtml = (value) =>
    String(value == null ? "" : value).replace(/[&<>"']/g, (char) => {
      switch (char) {
        case "&":
          return "&amp;";
        case "<":
          return "&lt;";
        case ">":
          return "&gt;";
        case '"':
          return "&quot;";
        default:
          return "&#39;";
      }
    });

  const qualityLabel = (quality) =>
    ({ auto: "Auto", "4k": "4K", "1440p": "1440p", "1080p": "1080p", "720p": "720p" }[quality] ||
      quality ||
      "—");

  const fitLabel = (fit) =>
    ({ cover: "Заполнение", contain: "По размеру", fill: "Растянуть" }[fit] || "Заполнение");

  const engineLabel = (engine) =>
    ({ direct: "yt-dlp", embed: "embed", idle: "—" }[engine] || "—");

  const formatMb = (value) => {
    const number = Number(value);
    if (!Number.isFinite(number) || number <= 0) return "—";
    return `${number.toFixed(1)} MB`;
  };

  function parseVideoId(raw) {
    const input = String(raw || "").trim();
    if (!input) return null;
    const withScheme = /^https?:\/\//i.test(input) ? input : `https://${input}`;
    let parsed;
    try {
      parsed = new URL(withScheme);
    } catch {
      return null;
    }
    const host = parsed.hostname.replace(/^www\./i, "").toLowerCase();
    let id = null;
    if (host === "youtu.be") {
      id = parsed.pathname.split("/").filter(Boolean)[0] || null;
    } else if (
      ["youtube.com", "m.youtube.com", "music.youtube.com", "youtube-nocookie.com"].includes(host)
    ) {
      if (parsed.pathname.replace(/\/+$/, "") === "/watch") {
        id = parsed.searchParams.get("v");
      } else {
        const parts = parsed.pathname.split("/").filter(Boolean);
        if (parts.length >= 2 && ["shorts", "embed", "live", "v"].includes(parts[0])) {
          id = parts[1];
        }
      }
    }
    return id && /^[A-Za-z0-9_-]{11}$/.test(id) ? id : null;
  }

  function toast(message, kind = "info") {
    if (!message) return;
    const node = document.createElement("div");
    node.className = `toast${kind === "error" ? " is-error" : ""}`;
    node.textContent = String(message);
    els.toastStack.appendChild(node);
    requestAnimationFrame(() => node.classList.add("is-in"));
    window.setTimeout(() => {
      node.classList.remove("is-in");
      window.setTimeout(() => node.remove(), 220);
    }, 3400);
  }

  function errorText(error) {
    if (typeof error === "string") return error;
    if (error && typeof error.message === "string") return error.message;
    return "Не удалось выполнить действие";
  }

  // ------------------------------------------------------------------- demo

  function demoApply(id) {
    const entry = state.library.find((item) => item.id === id);
    if (!entry) return null;
    return {
      id: entry.id,
      videoId: entry.video_id,
      title: entry.title,
      thumbnailUrl: entry.thumbnail_url,
      quality: entry.quality,
      fit: entry.fit_mode,
      engine: "direct",
      streamUrl: "demo",
      paused: state.playback.paused,
      pauseReason: state.playback.pauseReason,
      streamWidth: entry.quality === "4k" ? 3840 : entry.quality === "1440p" ? 2560 : 1920,
      streamHeight: entry.quality === "4k" ? 2160 : entry.quality === "1440p" ? 1440 : 1080,
      fps: 60,
    };
  }

  async function demoInvoke(command, args = {}) {
    switch (command) {
      case "get_app_state":
        return structuredClone({
          playback: state.playback,
          settings: state.settings,
          ytdlp: state.ytdlp,
          library: state.library,
        });
      case "get_library":
        return structuredClone(state.library);
      case "get_playback":
        return structuredClone(state.playback);
      case "get_runtime_stats":
        return { nodeRamMb: 96.4, webviewRamMb: 184.2, cpu: 3.7, processes: 7, sampledAt: Date.now() };
      case "add_wallpaper": {
        const videoId = parseVideoId(args.url);
        if (!videoId) throw new Error("Ссылка не похожа на YouTube-видео");
        const existing = state.library.find((item) => item.video_id === videoId);
        const entry =
          existing ||
          {
            id: `demo-${Date.now()}`,
            video_id: videoId,
            url: `https://www.youtube.com/watch?v=${videoId}`,
            title: `YouTube · ${videoId}`,
            thumbnail_url: `https://i.ytimg.com/vi/${videoId}/hqdefault.jpg`,
            quality: String(args.quality || "1080p"),
            fit_mode: String(args.fit || "cover"),
            added_at: new Date().toISOString(),
            last_used_at: null,
            channel: null,
          };
        if (existing) {
          existing.quality = String(args.quality || existing.quality);
          existing.fit_mode = String(args.fit || existing.fit_mode);
        } else {
          state.library.unshift(entry);
        }
        state.playback = demoApply(entry.id);
        return structuredClone(entry);
      }
      case "apply_wallpaper": {
        const playback = demoApply(args.id);
        if (!playback) throw new Error("Обои не найдены в библиотеке");
        state.playback = playback;
        return structuredClone(playback);
      }
      case "remove_wallpaper": {
        if (state.playback.id === args.id) {
          state.playback = {
            ...idlePlayback(),
            quality: state.settings.quality,
            fit: state.settings.fit,
          };
        }
        state.library = state.library.filter((item) => item.id !== args.id);
        return structuredClone(state.library);
      }
      case "toggle_pause":
        state.playback.paused = !state.playback.paused;
        state.playback.pauseReason = state.playback.paused ? "user" : null;
        return state.playback.paused;
      case "stop_wallpaper":
        state.playback = {
          ...idlePlayback(),
          quality: state.settings.quality,
          fit: state.settings.fit,
        };
        return null;
      case "set_fit_mode": {
        state.playback.fit = args.mode;
        state.settings.fit = args.mode;
        const entry = state.library.find((item) => item.id === state.playback.id);
        if (entry) entry.fit_mode = args.mode;
        return structuredClone(state.playback);
      }
      case "set_quality": {
        state.playback.quality = args.quality;
        state.settings.quality = args.quality;
        const entry = state.library.find((item) => item.id === state.playback.id);
        if (entry) {
          entry.quality = args.quality;
          state.playback.streamWidth = args.quality === "4k" ? 3840 : 1920;
          state.playback.streamHeight = args.quality === "4k" ? 2160 : 1080;
        }
        return structuredClone(state.playback);
      }
      case "update_settings":
        state.settings = { ...state.settings, ...args.settings };
        return structuredClone(state.settings);
      case "check_ytdlp": {
        const found = String(args.path || "").trim().toLowerCase().endsWith("yt-dlp.exe");
        state.ytdlp = found
          ? { found: true, path: String(args.path), version: "2026.11.18" }
          : { found: false, path: null, version: null };
        return structuredClone(state.ytdlp);
      }
      case "report_stream":
      case "minimize_main":
      case "hide_main":
      case "show_main_window":
      case "emit_note":
        return null;
      case "stream_failed":
        return structuredClone(state.playback);
      default:
        throw new Error(`Неизвестная команда: ${command}`);
    }
  }

  function call(command, args) {
    if (isTauri) return invoke(command, args);
    return demoInvoke(command, args);
  }

  // ----------------------------------------------------------------- render

  function renderEngine() {
    const found = Boolean(state.ytdlp && state.ytdlp.found);
    els.engineChip.classList.toggle("is-on", found);
    els.engineChipText.textContent = found ? "yt-dlp: найден" : "yt-dlp: fallback embed";
    els.engineChip.title = found
      ? `${state.ytdlp.path || "yt-dlp"}${state.ytdlp.version ? ` · ${state.ytdlp.version}` : ""}`
      : "yt-dlp не найден — используется YouTube IFrame API";

    els.ytdlpStatus.classList.toggle("is-on", found);
    els.ytdlpStatusText.textContent = found ? "yt-dlp: найден" : "fallback embed";
    els.ytdlpVersion.textContent = found
      ? `${state.ytdlp.version ? `yt-dlp ${state.ytdlp.version} · ` : ""}${state.ytdlp.path || ""}`
      : "Прямой поток недоступен: воспроизведение через embed.";
  }

  function renderSpotlight() {
    const playback = state.playback || idlePlayback();
    const active = Boolean(playback.id) && playback.engine !== "idle";
    const paused = Boolean(playback.paused);
    const fullscreen = playback.pauseReason === "fullscreen";
    const maximized = playback.pauseReason === "maximized";

    const thumb = playback.thumbnailUrl;
    els.spotPreview.classList.toggle("has-thumb", Boolean(thumb));
    els.spotPreview.classList.toggle("is-active", active && !paused);
    if (thumb) {
      if (els.spotThumb.getAttribute("src") !== thumb) els.spotThumb.src = thumb;
      els.spotThumb.alt = playback.title || "";
    } else {
      els.spotThumb.removeAttribute("src");
      els.spotThumb.alt = "";
    }
    els.spotEmpty.hidden = Boolean(thumb);
    els.spotEmptyText.textContent = active ? "Превью недоступно" : "Нет активных обоев";

    let statusText = "Не запущено";
    els.spotStatus.classList.remove("is-active", "is-paused");
    els.spotPreviewStatus.classList.remove("is-active", "is-paused");
    els.spotPreviewLabel.textContent = "IDLE";
    if (active && paused) {
      statusText = fullscreen
        ? "Пауза · полный экран"
        : maximized
          ? "Пауза · развёрнутое окно"
          : "Пауза";
      els.spotStatus.classList.add("is-paused");
      els.spotPreviewStatus.classList.add("is-paused");
      els.spotPreviewLabel.textContent = "PAUSED";
    } else if (active) {
      statusText = "Active";
      els.spotStatus.classList.add("is-active");
      els.spotPreviewStatus.classList.add("is-active");
      els.spotPreviewLabel.textContent = "ACTIVE";
    }
    els.spotStatusText.textContent = statusText;

    els.spotTitle.textContent = active ? playback.title || "Без названия" : "Обои не выбраны";
    els.spotSub.textContent = active
      ? `YouTube · ${playback.videoId || "—"}`
      : "Добавьте ссылку выше или выберите видео из библиотеки.";

    const width = Number(playback.streamWidth) || 0;
    const height = Number(playback.streamHeight) || 0;
    els.spotRes.textContent = width && height ? `${width} × ${height}` : "—";
    els.spotQuality.textContent = active ? qualityLabel(playback.quality) : "—";
    els.spotFit.textContent = fitLabel(playback.fit || state.settings.fit);
    els.spotEngine.textContent = active ? engineLabel(playback.engine) : "—";
    els.spotFps.textContent =
      active && Number(playback.fps) > 0 ? Number(playback.fps).toFixed(1) : "—";

    els.spotPause.disabled = !active;
    els.spotStop.disabled = !active;
    els.pauseLabel.textContent = paused ? "Возобновить" : "Пауза";
    els.pausePath.setAttribute(
      "d",
      paused ? "M5.5 3.2 12 8l-6.5 4.8V3.2Z" : "M5.5 3.5v9M10.5 3.5v9"
    );

    document.querySelectorAll(".seg[data-fit]").forEach((button) => {
      const activeFit = playback.fit || state.settings.fit;
      const isActive = button.dataset.fit === activeFit;
      button.classList.toggle("is-active", isActive);
      button.setAttribute("aria-pressed", isActive ? "true" : "false");
    });
  }

  function visibleLibrary() {
    const query = els.searchInput.value.trim().toLowerCase();
    if (!query) return state.library;
    return state.library.filter((entry) => {
      const haystack = `${entry.title || ""} ${entry.channel || ""} ${entry.video_id || ""}`.toLowerCase();
      return haystack.includes(query);
    });
  }

  function renderLibrary() {
    const entries = visibleLibrary();
    const total = state.library.length;
    const activeId = state.playback && state.playback.id;

    els.libraryGrid.innerHTML = entries
      .map((entry) => {
        const isActive = entry.id === activeId;
        return `<article class="card${isActive ? " is-active" : ""}" data-id="${escapeHtml(entry.id)}">
  <button class="card-preview" type="button" data-apply="${escapeHtml(entry.id)}" aria-label="Применить обои: ${escapeHtml(entry.title)}">
    <img src="${escapeHtml(entry.thumbnail_url)}" alt="" width="480" height="270" loading="lazy" decoding="async" />
    <span class="card-live"><span class="status-dot"></span>На экране</span>
    <span class="card-quality mono">${escapeHtml(qualityLabel(entry.quality))}</span>
    <span class="card-apply-overlay">
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M5.5 3.2 12 8l-6.5 4.8V3.2Z"/></svg>
      Применить
    </span>
  </button>
  <div class="card-body">
    <p class="card-title" title="${escapeHtml(entry.title)}">${escapeHtml(entry.title)}</p>
    <div class="card-meta">
      <span class="mono">${escapeHtml(fitLabel(entry.fit_mode))}</span>
      <button class="icon-btn danger" type="button" data-remove="${escapeHtml(entry.id)}" aria-label="Удалить из библиотеки">
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path d="M3.5 4.5h9M6.5 4.5V3h3v1.5M5 4.5l.6 8h4.8l.6-8"/>
        </svg>
      </button>
    </div>
  </div>
</article>`;
      })
      .join("");

    const filtered = els.searchInput.value.trim().length > 0;
    els.libraryCount.textContent = filtered ? `${entries.length} из ${total}` : String(total);
    els.libraryEmpty.hidden = entries.length > 0;
    els.libraryEmpty.textContent =
      total === 0 ? "Библиотека пуста — вставьте ссылку выше." : "Ничего не найдено.";
  }

  function renderSettings() {
    const settings = state.settings || {};
    const playback = state.playback || {};
    const active = Boolean(playback.id) && playback.engine !== "idle";
    els.pauseFullscreen.checked = Boolean(settings.pauseFullscreen);
    els.pauseMaximized.checked = Boolean(settings.pauseMaximized);
    els.autostart.checked = Boolean(settings.autostart);
    els.startMinimized.checked = Boolean(settings.startMinimized);
    if (document.activeElement !== els.ytdlpPath) {
      els.ytdlpPath.value = settings.ytdlpPath || "";
    }
    // The composer doubles as the current-wallpaper control: when something is
    // on screen, show its quality/fit instead of the stored defaults.
    const quality = active && playback.quality ? playback.quality : settings.quality;
    const fit = active && playback.fit ? playback.fit : settings.fit;
    if (document.activeElement !== els.qualitySelect && quality) {
      els.qualitySelect.value = quality;
    }
    if (document.activeElement !== els.fitSelect && fit) {
      els.fitSelect.value = fit;
    }
  }

  function renderTelemetry() {
    const playback = state.playback || idlePlayback();
    const active = Boolean(playback.id) && playback.engine !== "idle";
    els.telMode.textContent = !active
      ? "IDLE"
      : `${playback.paused ? "PAUSED" : "PLAYING"} · ${qualityLabel(playback.quality)} · ${fitLabel(
          playback.fit
        ).toUpperCase()}`;
  }

  function renderAll() {
    renderSpotlight();
    renderLibrary();
    renderSettings();
    renderEngine();
    renderTelemetry();
  }

  // ---------------------------------------------------------------- settings

  function collectSettings() {
    return {
      pauseFullscreen: els.pauseFullscreen.checked,
      pauseMaximized: els.pauseMaximized.checked,
      autostart: els.autostart.checked,
      startMinimized: els.startMinimized.checked,
      ytdlpPath: els.ytdlpPath.value.trim(),
      quality: els.qualitySelect.value,
      fit: els.fitSelect.value,
    };
  }

  async function saveSettings(options = {}) {
    const silent = options.silent !== false;
    const previous = state.settings;
    state.settings = { ...state.settings, ...collectSettings() };
    try {
      state.settings = await call("update_settings", { settings: collectSettings() });
      renderSettings();
      renderTelemetry();
      return true;
    } catch (error) {
      state.settings = previous;
      renderSettings();
      if (!silent) toast(errorText(error), "error");
      return false;
    }
  }

  // ----------------------------------------------------------------- actions

  async function refreshState() {
    if (refreshing) return;
    refreshing = true;
    try {
      const view = await call("get_app_state");
      state = { ...state, ...view };
      renderAll();
    } catch {
      /* the window may be closing; ignore */
    } finally {
      refreshing = false;
    }
  }

  function setAddBusy(busy) {
    addBusy = busy;
    els.addBtn.classList.toggle("is-loading", busy);
    els.addBtn.disabled = busy;
    els.addBtn.setAttribute("aria-busy", busy ? "true" : "false");
  }

  async function addFromForm(event) {
    event.preventDefault();
    if (addBusy) return;

    const url = els.urlInput.value;
    const videoId = parseVideoId(url);
    if (!videoId) {
      els.urlState.textContent = "ссылка не распознана";
      els.urlState.classList.remove("is-valid");
      els.urlInput.focus();
      return;
    }

    setAddBusy(true);
    try {
      await call("add_wallpaper", {
        url: url.trim(),
        quality: els.qualitySelect.value,
        fit: els.fitSelect.value,
      });
      els.urlInput.value = "";
      els.urlState.textContent = "";
      els.urlState.classList.remove("is-valid");
      toast("Обои добавлены и применены");
      await refreshState();
      await saveSettings();
      els.urlInput.focus();
    } catch (error) {
      toast(errorText(error), "error");
    } finally {
      setAddBusy(false);
    }
  }

  async function applyFromCard(button) {
    const id = button.getAttribute("data-apply");
    if (!id) return;
    button.setAttribute("aria-busy", "true");
    try {
      state.playback = await call("apply_wallpaper", { id });
      renderSpotlight();
      renderLibrary();
      renderTelemetry();
      toast("Обои применены");
    } catch (error) {
      toast(errorText(error), "error");
    } finally {
      button.removeAttribute("aria-busy");
    }
  }

  async function removeFromCard(button) {
    const id = button.getAttribute("data-remove");
    if (!id) return;
    const entry = state.library.find((item) => item.id === id);
    try {
      state.library = await call("remove_wallpaper", { id });
      if (state.playback.id === id) {
        state.playback = idlePlayback();
        renderSpotlight();
      }
      renderLibrary();
      renderTelemetry();
      toast(entry ? `Удалено: ${entry.title}` : "Удалено из библиотеки");
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  async function togglePause() {
    if (!state.playback.id) return;
    try {
      const paused = await call("toggle_pause");
      state.playback.paused = Boolean(paused);
      state.playback.pauseReason = paused ? "user" : null;
      renderSpotlight();
      renderTelemetry();
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  async function stopWallpaper() {
    if (!state.playback.id) return;
    try {
      await call("stop_wallpaper");
      state.playback = idlePlayback();
      renderSpotlight();
      renderLibrary();
      renderTelemetry();
      toast("Обои остановлены, фон Windows восстановлен");
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  async function setFit(mode) {
    if (!state.playback.id) {
      state.playback.fit = mode;
      renderSpotlight();
      return;
    }
    try {
      state.playback = await call("set_fit_mode", { mode });
      renderSpotlight();
      renderTelemetry();
      renderLibrary();
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  async function onQualityChange() {
    const quality = els.qualitySelect.value;
    if (!state.playback.id) {
      await saveSettings({ silent: false });
      return;
    }
    const previous = state.settings.quality;
    state.settings.quality = quality;
    els.qualitySelect.disabled = true;
    try {
      state.playback = await call("set_quality", { quality });
      renderSpotlight();
      renderTelemetry();
      renderLibrary();
      toast(`Качество: ${qualityLabel(quality)}`);
    } catch (error) {
      state.settings.quality = previous;
      renderSettings();
      toast(errorText(error), "error");
    } finally {
      els.qualitySelect.disabled = false;
    }
  }

  async function onFitChange() {
    const fit = els.fitSelect.value;
    if (state.playback.id) {
      state.settings.fit = fit;
      await setFit(fit);
    } else {
      await saveSettings({ silent: true });
    }
  }

  async function checkYtdlp() {
    els.checkYtdlp.disabled = true;
    try {
      await saveSettings();
      state.ytdlp = await call("check_ytdlp", { path: els.ytdlpPath.value.trim() });
      renderEngine();
      toast(
        state.ytdlp.found
          ? `yt-dlp найден${state.ytdlp.version ? ` · ${state.ytdlp.version}` : ""}`
          : "yt-dlp не найден — будет использован embed",
        state.ytdlp.found ? "info" : "error"
      );
    } catch (error) {
      toast(errorText(error), "error");
    } finally {
      els.checkYtdlp.disabled = false;
    }
  }

  function validateUrl() {
    const raw = els.urlInput.value.trim();
    if (!raw) {
      els.urlState.textContent = "";
      els.urlState.classList.remove("is-valid");
      return;
    }
    const id = parseVideoId(raw);
    els.urlState.textContent = id || "ссылка не распознана";
    els.urlState.classList.toggle("is-valid", Boolean(id));
    els.urlState.title = id ? `VIDEO_ID: ${id}` : "Поддерживаются watch, youtu.be, shorts";
  }

  // ---------------------------------------------------------------- telemetry

  async function pollTelemetry() {
    try {
      const stats = await call("get_runtime_stats");
      els.telNode.textContent = formatMb(stats.nodeRamMb);
      els.telCpu.textContent = Number.isFinite(Number(stats.cpu))
        ? `${Number(stats.cpu).toFixed(1)}%`
        : "—";
      els.telWebview.textContent = formatMb(stats.webviewRamMb);
    } catch {
      /* ignore a single failed sample */
    }
  }

  function startTelemetry() {
    if (telemetryTimer) return;
    pollTelemetry();
    telemetryTimer = window.setInterval(pollTelemetry, 2000);
  }

  function stopTelemetry() {
    if (!telemetryTimer) return;
    window.clearInterval(telemetryTimer);
    telemetryTimer = 0;
  }

  function syncVisibility() {
    const hidden = document.hidden;
    document.body.classList.toggle("ui-paused", hidden);
    if (hidden) {
      stopTelemetry();
    } else {
      startTelemetry();
      refreshState();
    }
  }

  // ------------------------------------------------------------------- wiring

  function bindEvents() {
    els.addForm.addEventListener("submit", addFromForm);
    els.urlInput.addEventListener("input", validateUrl);
    els.urlInput.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        els.urlInput.value = "";
        validateUrl();
      }
    });

    els.searchInput.addEventListener("input", renderLibrary);
    document.addEventListener("keydown", (event) => {
      const target = event.target;
      const typing =
        target instanceof HTMLInputElement ||
        target instanceof HTMLSelectElement ||
        target instanceof HTMLTextAreaElement;
      if (event.key === "/" && !typing) {
        event.preventDefault();
        els.searchInput.focus();
        els.searchInput.select();
      }
      if (event.key === "Escape" && document.activeElement === els.searchInput) {
        els.searchInput.value = "";
        renderLibrary();
        els.searchInput.blur();
      }
    });

    els.libraryGrid.addEventListener("click", (event) => {
      const applyButton = event.target.closest("[data-apply]");
      if (applyButton) {
        applyFromCard(applyButton);
        return;
      }
      const removeButton = event.target.closest("[data-remove]");
      if (removeButton) removeFromCard(removeButton);
    });

    els.libraryGrid.addEventListener(
      "error",
      (event) => {
        if (event.target instanceof HTMLImageElement) {
          const preview = event.target.closest(".card-preview");
          if (preview) preview.classList.add("no-thumb");
        }
      },
      true
    );

    els.spotThumb.addEventListener("error", () => {
      els.spotPreview.classList.remove("has-thumb");
      els.spotEmpty.hidden = false;
      els.spotEmptyText.textContent = "Превью недоступно";
    });
    els.spotThumb.addEventListener("load", () => {
      els.spotPreview.classList.add("has-thumb");
      els.spotEmpty.hidden = true;
    });

    els.spotPause.addEventListener("click", togglePause);
    els.spotStop.addEventListener("click", stopWallpaper);
    document.querySelectorAll(".seg[data-fit]").forEach((button) => {
      button.addEventListener("click", () => setFit(button.dataset.fit));
    });

    [els.pauseFullscreen, els.pauseMaximized, els.autostart, els.startMinimized].forEach((input) => {
      input.addEventListener("change", async () => {
        const ok = await saveSettings({ silent: false });
        if (!ok) renderSettings();
      });
    });

    els.ytdlpPath.addEventListener("change", () => saveSettings({ silent: true }));
    els.qualitySelect.addEventListener("change", onQualityChange);
    els.fitSelect.addEventListener("change", onFitChange);
    els.ytdlpPath.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        els.ytdlpPath.blur();
        checkYtdlp();
      }
    });
    els.checkYtdlp.addEventListener("click", checkYtdlp);

    els.btnMinimize.addEventListener("click", () => {
      if (isTauri) call("minimize_main");
    });
    els.btnClose.addEventListener("click", () => {
      if (isTauri) call("hide_main");
    });

    document.addEventListener("visibilitychange", syncVisibility);
    window.addEventListener("pagehide", () => document.body.classList.add("ui-paused"));
  }

  async function subscribe() {
    if (!isTauri || !listen) return;
    try {
      await listen("state:changed", (event) => {
        state.playback = event.payload;
        renderSpotlight();
        renderLibrary();
        renderTelemetry();
      });
      await listen("library:changed", (event) => {
        state.library = Array.isArray(event.payload) ? event.payload : [];
        renderLibrary();
      });
      await listen("ytdlp:changed", (event) => {
        state.ytdlp = event.payload;
        renderEngine();
      });
      await listen("note", (event) => {
        if (event.payload && event.payload.message) toast(event.payload.message);
      });
    } catch {
      /* events are optional; polling below keeps the UI fresh */
    }
  }

  async function init() {
    bindEvents();
    validateUrl();

    if (isTauri) {
      try {
        const view = await call("get_app_state");
        state = { ...state, ...view };
      } catch (error) {
        toast(errorText(error), "error");
      }
      await subscribe();
    } else {
      state.library = DEMO_LIBRARY.map((entry) => ({ ...entry }));
      state.playback = demoApply("demo-1");
      state.ytdlp = { found: false, path: null, version: null };
    }

    renderAll();
    startTelemetry();
    els.urlInput.focus();
    // `ytdlp:changed` may fire before this page subscribes on a cold start;
    // one delayed re-read covers that race (and any missed startup event).
    window.setTimeout(refreshState, 1500);
  }

  init();
})();
