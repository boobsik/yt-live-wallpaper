/* yt-live-wallpaper — wallpaper renderer.
   Two stacked <video> elements crossfade on loop (seamless, no black frame),
   or the YouTube IFrame API is used when yt-dlp is unavailable. */

(() => {
  "use strict";

  const TAURI = window.__TAURI__;
  const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
  const listen = TAURI && TAURI.event ? TAURI.event.listen : null;
  const isTauri = typeof invoke === "function";

  const stage = document.getElementById("stage");
  const embedContainer = document.getElementById("embed-container");
  const playerHost = document.getElementById("player");
  const videos = [document.getElementById("video-a"), document.getElementById("video-b")];

  let playback = null;
  let activeIndex = 0;
  let loadToken = 0;
  let lastKey = "";
  let player = null;
  let ytPromise = null;
  let crossfading = false;
  let failedAt = 0;
  let fpsMeasuredFor = "";
  let revealTimer = 0;

  const activeVideo = () => videos[activeIndex];
  const otherVideo = () => videos[1 - activeIndex];

  function clearReveal() {
    if (revealTimer) {
      window.clearTimeout(revealTimer);
      revealTimer = 0;
    }
  }

  // The Rust side keeps the wallpaper hidden while a stream starts. Reveal it
  // only after an actual PLAYING event plus a short grace period.
  function scheduleReveal() {
    if (!isTauri || revealTimer || !playback || playback.paused) return;
    const id = playback.id;
    revealTimer = window.setTimeout(() => {
      revealTimer = 0;
      if (!playback || playback.paused || playback.id !== id) return;
      invoke("wallpaper_playing", { id }).catch(() => {});
    }, 300);
  }

  function setFit(fit) {
    stage.dataset.fit = ["cover", "contain", "fill"].includes(fit) ? fit : "cover";
    if (playback) playback.fit = stage.dataset.fit;
  }

  function setPaused(paused) {
    const isPaused = Boolean(paused);
    if (isPaused) clearReveal();
    if (playback) playback.paused = isPaused;
    if (player && typeof player.pauseVideo === "function") {
      try {
        isPaused ? player.pauseVideo() : player.playVideo();
      } catch {
        /* the player can be mid-destroy while an event arrives */
      }
    }
    if (isPaused) {
      videos.forEach((video) => {
        try {
          video.pause();
        } catch {
          /* ignore */
        }
      });
    } else if (playback && playback.engine === "direct") {
      const video = activeVideo();
      if (video.currentSrc) {
        const attempt = video.play();
        if (attempt && typeof attempt.catch === "function") attempt.catch(() => {});
      }
    }
  }

  function stopVideos() {
    videos.forEach((video) => {
      try {
        video.pause();
      } catch {
        /* ignore */
      }
      video.classList.remove("is-visible");
      video.removeAttribute("src");
      try {
        video.load();
      } catch {
        /* ignore */
      }
    });
  }

  function destroyPlayer() {
    if (player) {
      try {
        player.destroy();
      } catch {
        /* ignore */
      }
      player = null;
    }
    playerHost.innerHTML = "";
    embedContainer.hidden = true;
  }

  function stopAll() {
    clearReveal();
    stopVideos();
    destroyPlayer();
    crossfading = false;
    activeIndex = 0;
  }

  function reportFailure() {
    if (!isTauri || !playback || playback.engine === "idle") return;
    const now = Date.now();
    if (now - failedAt < 6000) return;
    failedAt = now;
    invoke("stream_failed")
      .then((next) => {
        if (next) onPlayback(next);
      })
      .catch(() => {
        /* embed is already the last resort */
      });
  }

  function measureFps(video) {
    if (!("requestVideoFrameCallback" in video)) return;
    let frames = 0;
    let started = 0;
    const tick = (now) => {
      if (!started) started = now;
      frames += 1;
      const elapsed = now - started;
      if (elapsed >= 1000) {
        const fps = Math.round((frames * 1000 * 10) / elapsed) / 10;
        if (isTauri) {
          invoke("report_stream", {
            width: video.videoWidth,
            height: video.videoHeight,
            fps,
          }).catch(() => {});
        }
      } else {
        video.requestVideoFrameCallback(tick);
      }
    };
    video.requestVideoFrameCallback(tick);
  }

  function reportStream(video) {
    if (!isTauri || !video.videoWidth) return;
    invoke("report_stream", {
      width: video.videoWidth,
      height: video.videoHeight,
      fps: 0,
    }).catch(() => {});
    const url = video.currentSrc || video.src;
    if (url && fpsMeasuredFor !== url) {
      fpsMeasuredFor = url;
      measureFps(video);
    }
  }

  function playDirect(url, token) {
    activeIndex = 0;
    videos[1].classList.remove("is-visible");

    const video = videos[0];
    video.loop = false;
    video.src = url;
    video.classList.add("is-visible");
    video.load();

    const start = () => {
      if (token !== loadToken) return;
      if (Number.isFinite(video.duration) && video.duration > 0 && video.duration < 6) {
        video.loop = true;
      }
      // Applying a wallpaper while the desktop is paused (fullscreen app or a
      // manual pause) must not autoplay it behind the user's back.
      if (!(playback && playback.paused)) {
        const attempt = video.play();
        if (attempt && typeof attempt.catch === "function") {
          // Autoplay is allowed for muted media; a rejection means the stream is
          // unplayable (403/expired URL) and will surface through `error`.
          attempt.catch(() => {});
        }
      }
      reportStream(video);
    };

    if (video.readyState >= 1) start();
    else video.addEventListener("loadedmetadata", start, { once: true });

    window.setTimeout(() => {
      if (token === loadToken && !(playback && playback.paused) && video.paused && video.currentSrc) {
        const attempt = video.play();
        if (attempt && typeof attempt.catch === "function") attempt.catch(() => {});
      }
    }, 1500);
  }

  function crossfade() {
    if (crossfading || !playback || playback.engine !== "direct" || playback.paused) return;
    const current = activeVideo();
    const next = otherVideo();
    const url = current.currentSrc || current.src;
    if (!url) return;
    if (!Number.isFinite(current.duration) || current.duration < 6) return;

    crossfading = true;
    // Safety net: if the second decoder is slow to start, keep the picture
    // moving instead of freezing on the last frame.
    current.loop = true;

    const begin = () => {
      if (!crossfading || !playback || playback.paused) {
        crossfading = false;
        return;
      }
      next.loop = false;
      try {
        next.currentTime = 0;
      } catch {
        /* ignore */
      }
      const attempt = next.play();
      if (!attempt || typeof attempt.then !== "function") {
        crossfading = false;
        return;
      }
      attempt
        .then(() => {
          next.classList.add("is-visible");
          current.classList.remove("is-visible");
          activeIndex = 1 - activeIndex;
          window.setTimeout(() => {
            try {
              current.pause();
              current.currentTime = 0;
            } catch {
              /* ignore */
            }
            current.loop = false;
            crossfading = false;
          }, 620);
        })
        .catch(() => {
          crossfading = false;
        });
    };

    if (next.readyState >= 1 && (next.currentSrc || next.src) === url) {
      begin();
    } else {
      next.src = url;
      next.load();
      next.addEventListener("loadedmetadata", begin, { once: true });
      // A stalled metadata load must not lock the crossfade forever.
      window.setTimeout(() => {
        if (crossfading) {
          crossfading = false;
        }
      }, 5000);
    }
  }

  function onTimeUpdate(event) {
    const video = event.target;
    if (video !== activeVideo() || crossfading) return;
    if (!Number.isFinite(video.duration) || video.duration < 6) return;
    if (video.paused || video.loop) return;
    if (video.duration - video.currentTime <= 1.7) crossfade();
  }

  function loadYouTubeApi() {
    if (window.YT && window.YT.Player) return Promise.resolve(window.YT);
    if (ytPromise) return ytPromise;
    ytPromise = new Promise((resolve) => {
      const previous = window.onYouTubeIframeAPIReady;
      window.onYouTubeIframeAPIReady = () => {
        try {
          if (typeof previous === "function") previous();
        } catch {
          /* ignore */
        }
        resolve(window.YT || null);
      };
      const script = document.createElement("script");
      script.src = "https://www.youtube.com/iframe_api";
      script.async = true;
      script.onerror = () => resolve(null);
      document.head.appendChild(script);
      window.setTimeout(() => resolve(window.YT || null), 10000);
    });
    return ytPromise;
  }

  async function playEmbed(videoId, token) {
    const yt = await loadYouTubeApi();
    if (!yt || token !== loadToken) {
      if (!yt) ytPromise = null;
      return;
    }

    playerHost.innerHTML = "";
    const frame = document.createElement("div");
    frame.id = "yt-frame";
    playerHost.appendChild(frame);
    embedContainer.hidden = false;

    player = new yt.Player("yt-frame", {
      videoId,
      playerVars: {
        autoplay: 1,
        controls: 0,
        disablekb: 1,
        fs: 0,
        rel: 0,
        iv_load_policy: 3,
        playsinline: 1,
        mute: 1,
        loop: 1,
        playlist: videoId,
      },
      events: {
        onReady: (event) => {
          try {
            event.target.mute();
            if (playback && playback.paused) event.target.pauseVideo();
            else event.target.playVideo();
          } catch {
            /* ignore */
          }
        },
        onStateChange: (event) => {
          if (event.data === yt.PlayerState.PLAYING) {
            scheduleReveal();
          }
          // Seamless loop: jump back to the first frame when the video ends.
          if (event.data === yt.PlayerState.ENDED) {
            try {
              event.target.seekTo(0);
              event.target.playVideo();
            } catch {
              /* ignore */
            }
          }
        },
        onError: () => reportFailure(),
      },
    });
  }

  async function onPlayback(next) {
    if (!next || !next.engine || next.engine === "idle") {
      playback = next || null;
      lastKey = "";
      stopAll();
      stage.dataset.engine = "idle";
      return;
    }

    playback = next;
    setFit(next.fit);
    const key = `${next.engine}:${next.videoId}:${next.streamUrl || ""}`;
    if (key === lastKey) {
      setPaused(next.paused);
      return;
    }

    lastKey = key;
    const token = ++loadToken;
    stopAll();
    fpsMeasuredFor = "";

    if (next.engine === "direct" && next.streamUrl) {
      stage.dataset.engine = "direct";
      playDirect(next.streamUrl, token);
    } else if (next.engine === "embed" && next.videoId) {
      stage.dataset.engine = "embed";
      await playEmbed(next.videoId, token);
    }

    if (token === loadToken) setPaused(next.paused);
  }

  function bindVideoEvents() {
    videos.forEach((video) => {
      video.addEventListener("timeupdate", onTimeUpdate);
      video.addEventListener("playing", () => {
        if (video === activeVideo()) scheduleReveal();
      });
      video.addEventListener("error", () => {
        if (!playback || playback.engine !== "direct") return;
        if (!video.currentSrc || !video.error) return;
        reportFailure();
      });
      video.addEventListener("loadeddata", () => {
        if (video === activeVideo()) reportStream(video);
      });
    });
  }

  async function init() {
    bindVideoEvents();

    if (isTauri && listen) {
      try {
        await listen("wallpaper:play", (event) => onPlayback(event.payload));
        await listen("wallpaper:pause", (event) =>
          setPaused(Boolean(event.payload && event.payload.paused))
        );
        await listen("wallpaper:fit", (event) => setFit(event.payload));
        await listen("wallpaper:stop", () => {
          lastKey = "";
          playback = null;
          stopAll();
          stage.dataset.engine = "idle";
        });
      } catch {
        /* ignore */
      }
    }

    // The renderer is the source of truth after a reload / WebView2 recycle.
    try {
      const current = await invoke("get_playback");
      if (current) await onPlayback(current);
    } catch {
      /* ignore */
    }

    document.addEventListener("visibilitychange", () => {
      if (document.hidden || !playback || playback.paused) return;
      if (playback.engine === "direct") {
        const video = activeVideo();
        if (video.currentSrc) {
          const attempt = video.play();
          if (attempt && typeof attempt.catch === "function") attempt.catch(() => {});
        }
      } else if (player && typeof player.playVideo === "function") {
        try {
          player.playVideo();
        } catch {
          /* ignore */
        }
      }
    });
  }

  init();
})();
