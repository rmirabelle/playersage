import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Pause, Play, Square, X, ZoomIn } from "lucide-react";
import { AboutDialog } from "./components/AboutDialog";
import { checkForUpdate, type UpdateInfo } from "./lib/updater";

type PlayerState = {
  loaded: boolean;
  playing: boolean;
  position: number;
  duration: number;
  path: string | null;
  zoom: number;
  speed: number;
  loop_a: number | null;
  loop_b: number | null;
};

const SPEEDS: { value: number; label: string }[] = [
  { value: 0.125, label: "⅛" },
  { value: 0.25, label: "¼" },
  { value: 0.5, label: "½" },
  { value: 0.75, label: "¾" },
  { value: 1, label: "1×" },
];

const VIDEO_EXT = /\.(mov|mp4|m4v|mkv|webm|avi)$/i;

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export default function App() {
  const videoRegionRef = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<PlayerState>({
    loaded: false,
    playing: false,
    position: 0,
    duration: 0,
    path: null,
    zoom: 1,
    speed: 1,
    loop_a: null,
    loop_b: null,
  });
  const [aboutOpen, setAboutOpen] = useState(false);
  const [appVersion, setAppVersion] = useState("");
  const [startupUpdate, setStartupUpdate] = useState<UpdateInfo | null>(null);

  const reportGeometry = useCallback(async () => {
    const el = videoRegionRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const scale = window.devicePixelRatio || 1;
    const args = {
      x: Math.round(rect.left * scale),
      y: Math.round(rect.top * scale),
      width: Math.round(rect.width * scale),
      height: Math.round(rect.height * scale),
    };
    try {
      await invoke("set_video_region", args);
    } catch (e) {
      console.error("set_video_region failed:", e);
    }
  }, []);

  useEffect(() => {
    reportGeometry();
    const onResize = () => reportGeometry();
    window.addEventListener("resize", onResize);
    const obs = new ResizeObserver(onResize);
    if (videoRegionRef.current) obs.observe(videoRegionRef.current);
    const win = getCurrentWindow();
    const unlistenPromise = win.onResized(() => reportGeometry());
    return () => {
      window.removeEventListener("resize", onResize);
      obs.disconnect();
      unlistenPromise.then((un) => un());
    };
  }, [reportGeometry]);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const s = await invoke<PlayerState>("get_state");
        if (!cancelled) setState(s);
      } catch {
        // ignore
      }
    };
    const id = setInterval(tick, 100);
    tick();
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  useEffect(() => {
    const p = listen<PlayerState>("player-state", (e) => setState(e.payload));
    return () => {
      p.then((un) => un());
    };
  }, []);

  useEffect(() => {
    const webview = getCurrentWebview();
    const unlistenPromise = webview.onDragDropEvent((event) => {
      if (event.payload.type === "drop") {
        const paths = event.payload.paths;
        const target = paths.find((p) => VIDEO_EXT.test(p)) ?? paths[0];
        if (target) {
          invoke("load_file", { path: target }).catch((e) =>
            console.error("load_file from drop failed:", e),
          );
        }
      }
    });
    return () => {
      unlistenPromise.then((un) => un());
    };
  }, []);

  useEffect(() => {
    // Ensure the webview captures keyboard events from the very first render
    // (otherwise the OS keyboard focus may sit on the main HWND with no
    // child claiming it, so neither our JS keydown handler nor the native
    // wndproc fires until the user manually clicks something).
    window.focus();
  }, []);

  useEffect(() => {
    invoke<string>("get_app_version").then(setAppVersion).catch(() => {});
  }, []);

  useEffect(() => {
    const p = listen("show-about", () => setAboutOpen(true));
    return () => {
      p.then((un) => un());
    };
  }, []);

  // Silent startup update check — fires once, results auto-populate the About dialog.
  useEffect(() => {
    let cancelled = false;
    checkForUpdate()
      .then((info) => {
        if (!cancelled && info) setStartupUpdate(info);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      // Only bail when the user is actually typing — not for range sliders,
      // checkboxes, or focused buttons, where shortcuts should still work.
      const typing =
        !!t &&
        (t.tagName === "TEXTAREA" ||
          t.isContentEditable ||
          (t.tagName === "INPUT" &&
            ["text", "search", "email", "url", "tel", "password", "number"].includes(
              (t as HTMLInputElement).type,
            )));
      if (typing) return;
      if (e.code === "Space") {
        e.preventDefault();
        invoke("toggle_play_pause").catch(() => {});
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        const step = e.ctrlKey ? 1 : e.shiftKey ? 20 : 5;
        const delta = e.key === "ArrowLeft" ? -step : step;
        invoke("seek_relative", { delta }).catch(() => {});
      } else if (e.key === "0" || e.key.toLowerCase() === "r") {
        invoke("reset_view").catch(() => {});
      } else if (e.key.toLowerCase() === "a") {
        invoke("set_loop_a").catch(() => {});
      } else if (e.key.toLowerCase() === "b") {
        invoke("set_loop_b").catch(() => {});
      } else if (e.key.toLowerCase() === "c") {
        invoke("clear_loop").catch(() => {});
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const openFile = async () => {
    const selected = await openDialog({
      multiple: false,
      filters: [
        { name: "Video", extensions: ["mov", "mp4", "m4v", "mkv", "webm", "avi"] },
      ],
    });
    if (typeof selected === "string") {
      await invoke("load_file", { path: selected });
      await reportGeometry();
    }
  };

  const togglePlay = async () => {
    if (!state.loaded) return;
    if (state.playing) await invoke("pause");
    else await invoke("play");
  };

  const onScrub = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const target = Number(e.target.value);
    await invoke("seek", { seconds: target });
  };

  return (
    <div className="h-full w-full flex flex-col bg-background text-foreground">
      <div
        ref={videoRegionRef}
        id="video-region"
        className="flex-1 min-h-0 bg-black relative flex items-center justify-center"
      >
        {!state.loaded && (
          <div className="text-muted-foreground text-sm pointer-events-none">
            Open a video to begin
          </div>
        )}
      </div>

      <div className="border-t border-border bg-card px-4 py-3 flex items-center gap-3">
        <button
          onClick={openFile}
          className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-muted hover:bg-accent text-sm transition"
          title="Open video"
        >
          <FolderOpen size={16} />
          Open
        </button>

        <button
          onClick={() => invoke("stop").catch(() => {})}
          disabled={!state.loaded}
          className="flex items-center justify-center w-9 h-9 rounded-md bg-muted hover:bg-accent disabled:opacity-40 disabled:cursor-not-allowed transition"
          title="Close current video"
        >
          <Square size={14} />
        </button>

        <button
          onClick={togglePlay}
          disabled={!state.loaded}
          className="flex items-center justify-center w-9 h-9 rounded-md bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-40 disabled:cursor-not-allowed transition"
          title={state.playing ? "Pause" : "Play"}
        >
          {state.playing ? <Pause size={16} /> : <Play size={16} />}
        </button>

        <span className="text-xs tabular-nums text-muted-foreground w-20 text-right">
          {formatTime(state.position)}
        </span>

        <input
          type="range"
          min={0}
          max={state.duration || 0}
          step={0.01}
          value={state.position}
          onChange={onScrub}
          disabled={!state.loaded}
          className="flex-1 accent-[var(--color-primary)] disabled:opacity-40"
        />

        <span className="text-xs tabular-nums text-muted-foreground w-20">
          {formatTime(state.duration)}
        </span>

        <div className="flex items-center gap-1.5 px-2 py-1 rounded-md bg-muted/60 text-xs font-semibold tabular-nums">
          <span
            className={
              state.loop_a !== null
                ? "text-emerald-400"
                : "text-muted-foreground/40"
            }
            title={
              state.loop_a !== null
                ? `Loop start: ${formatTime(state.loop_a)}  (press A to update)`
                : "Press A to set loop start"
            }
          >
            A
          </span>
          <span
            className={
              state.loop_b !== null
                ? "text-emerald-400"
                : "text-muted-foreground/40"
            }
            title={
              state.loop_b !== null
                ? `Loop end: ${formatTime(state.loop_b)}  (press B to update)`
                : "Press B to set loop end"
            }
          >
            B
          </span>
          {(state.loop_a !== null || state.loop_b !== null) && (
            <button
              onClick={() => invoke("clear_loop").catch(() => {})}
              className="opacity-60 hover:opacity-100 ml-0.5"
              title="Clear loop (C)"
            >
              <X size={12} />
            </button>
          )}
        </div>

        <div className="flex items-center gap-0.5 px-1.5 py-0.5 rounded-md bg-muted/60">
          {SPEEDS.map((s) => {
            const active = Math.abs((state.speed || 1) - s.value) < 0.001;
            return (
              <button
                key={s.value}
                onClick={() => invoke("set_speed", { speed: s.value }).catch(() => {})}
                disabled={!state.loaded}
                className={
                  "px-2 py-0.5 rounded text-xs tabular-nums transition disabled:opacity-40 disabled:cursor-not-allowed " +
                  (active
                    ? "bg-primary text-primary-foreground"
                    : "hover:bg-accent text-muted-foreground")
                }
                title={`Playback speed ${s.label}`}
              >
                {s.label}
              </button>
            );
          })}
        </div>

        <button
          onClick={() => invoke("reset_view").catch(() => {})}
          disabled={!state.loaded}
          className="flex items-center gap-1.5 px-2 py-1 rounded-md bg-muted hover:bg-accent text-xs tabular-nums disabled:opacity-40 disabled:cursor-not-allowed transition"
          title="Reset zoom & pan (or double-click the video)"
        >
          <ZoomIn size={14} />
          {Math.round((state.zoom || 1) * 100)}%
        </button>
      </div>

      <AboutDialog
        open={aboutOpen}
        version={appVersion}
        initialUpdateInfo={startupUpdate}
        onClose={() => setAboutOpen(false)}
      />
    </div>
  );
}
