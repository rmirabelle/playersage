import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  CheckCircle2,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  X,
} from "lucide-react";
import { checkForUpdate, type UpdateInfo } from "../lib/updater";

type Props = {
  open: boolean;
  version: string;
  initialUpdateInfo?: UpdateInfo | null;
  onClose: () => void;
};

type CheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "upToDate" }
  | { kind: "available"; info: UpdateInfo }
  | { kind: "downloading"; info: UpdateInfo; downloaded: number; total: number }
  | { kind: "error"; message: string };

export function AboutDialog({ open, version, initialUpdateInfo, onClose }: Props) {
  const [state, setState] = useState<CheckState>({ kind: "idle" });

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && state.kind !== "downloading") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, state.kind, onClose]);

  useEffect(() => {
    if (!open) return;
    if (initialUpdateInfo && state.kind === "idle") {
      setState({ kind: "available", info: initialUpdateInfo });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, initialUpdateInfo]);

  useEffect(() => {
    if (state.kind !== "downloading") return;
    const unlisten = listen<{ downloaded: number; total: number }>(
      "update-progress",
      (ev) => {
        setState((s) =>
          s.kind === "downloading"
            ? { ...s, downloaded: ev.payload.downloaded, total: ev.payload.total }
            : s,
        );
      },
    );
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [state.kind]);

  if (!open) return null;

  async function handleCheck() {
    setState({ kind: "checking" });
    try {
      const info = await checkForUpdate();
      if (info) setState({ kind: "available", info });
      else setState({ kind: "upToDate" });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  }

  async function handleDownload() {
    if (state.kind !== "available") return;
    const info = state.info;
    setState({ kind: "downloading", info, downloaded: 0, total: 0 });
    try {
      await invoke("download_and_run_installer", {
        url: info.downloadUrl,
        assetName: info.assetName,
      });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  }

  const overlayClickable = state.kind !== "downloading";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/70 backdrop-blur-sm"
      onClick={() => overlayClickable && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
        className="relative w-[min(92vw,500px)] rounded-xl border border-border bg-card p-6 text-card-foreground shadow-2xl"
      >
        {state.kind !== "downloading" && (
          <button
            type="button"
            onClick={onClose}
            className="absolute right-3 top-3 rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
            aria-label="Close"
          >
            <X className="h-4 w-4" />
          </button>
        )}

        <div className="grid grid-cols-[auto_1fr] gap-6">
          <div className="flex items-start justify-center">
            <img
              src="/app-icon.png"
              alt="Player Sage"
              className="h-24 w-24 rounded-lg"
            />
          </div>

          <div className="flex flex-col gap-1 text-left">
            <div className="text-2xl font-semibold text-primary">Player Sage</div>
            <div className="text-xs text-muted-foreground">
              Version {version || "—"}
            </div>
            <div className="mt-2 text-sm">by Robert Mirabelle</div>
            <div className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
              A focused video player for inspecting frames closely. Powered by libmpv.
            </div>
          </div>
        </div>

        <div className="mt-6 border-t border-border pt-4">
          <UpdateSection
            state={state}
            onCheck={handleCheck}
            onDownload={handleDownload}
          />
        </div>

        {state.kind !== "downloading" && (
          <div className="mt-5 flex justify-center">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md border border-border bg-muted px-5 py-1.5 text-sm font-medium hover:bg-accent"
            >
              Close
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function UpdateSection({
  state,
  onCheck,
  onDownload,
}: {
  state: CheckState;
  onCheck: () => void;
  onDownload: () => void;
}) {
  switch (state.kind) {
    case "idle":
      return (
        <div className="flex items-center justify-between gap-3">
          <span className="text-xs text-muted-foreground">
            Check GitHub for a newer release.
          </span>
          <button
            type="button"
            onClick={onCheck}
            className="flex items-center gap-1.5 rounded-md border border-border bg-muted px-3 py-1.5 text-xs font-medium hover:bg-accent"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            Check for Updates
          </button>
        </div>
      );

    case "checking":
      return (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          Checking for updates{"…"}
        </div>
      );

    case "upToDate":
      return (
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-xs text-foreground">
            <CheckCircle2 className="h-4 w-4 text-primary" />
            You&apos;re up to date.
          </div>
          <button
            type="button"
            onClick={onCheck}
            className="flex items-center gap-1.5 rounded-md border border-border bg-muted px-2.5 py-1 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <RefreshCw className="h-3 w-3" />
            Check again
          </button>
        </div>
      );

    case "available":
      return (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span className="font-medium text-foreground">
              Update available: v{state.info.latestVersion}
            </span>
            <span className="text-muted-foreground">
              (you&apos;re on v{state.info.currentVersion})
            </span>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={onDownload}
              className="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:opacity-90"
            >
              <Download className="h-3.5 w-3.5" />
              Download &amp; install
            </button>
            <button
              type="button"
              onClick={() => void openUrl(state.info.releaseUrl)}
              className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              Release notes
            </button>
          </div>
        </div>
      );

    case "downloading": {
      const pct =
        state.total > 0
          ? Math.min(100, Math.round((state.downloaded / state.total) * 100))
          : null;
      return (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2 text-xs text-foreground">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            Downloading v{state.info.latestVersion}
            {pct !== null ? ` — ${pct}%` : "…"}
          </div>
          <div className="h-2 w-full overflow-hidden rounded bg-border">
            <div
              className="h-full bg-primary transition-[width] duration-150"
              style={{ width: pct !== null ? `${pct}%` : "10%" }}
            />
          </div>
          <div className="text-[11px] text-muted-foreground">
            The installer will launch when the download completes. Player Sage will
            close to apply the update.
          </div>
        </div>
      );
    }

    case "error":
      return (
        <div className="flex flex-col gap-2">
          <div className="text-xs text-red-400">{state.message}</div>
          <button
            type="button"
            onClick={onCheck}
            className="self-start rounded-md border border-border bg-muted px-2.5 py-1 text-[11px] hover:bg-accent"
          >
            Try again
          </button>
        </div>
      );
  }
}
