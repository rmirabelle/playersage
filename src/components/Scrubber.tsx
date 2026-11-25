import { useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

type Props = {
  position: number;
  duration: number;
  loopA: number | null;
  loopB: number | null;
  disabled: boolean;
};

/** Smallest gap between the A and B points, in seconds. */
const MIN_GAP = 0.05;

/**
 * Seek bar. When an A-B loop is set, it shows a handle at each loop point.
 * Drag a handle to move that point; the video shows the frame under it.
 */
export function Scrubber({ position, duration, loopA, loopB, disabled }: Props) {
  const trackRef = useRef<HTMLDivElement>(null);
  const inFlight = useRef(false);
  const pending = useRef<(() => Promise<unknown>) | null>(null);

  const pct = (t: number) => (duration > 0 ? Math.min(100, Math.max(0, (t / duration) * 100)) : 0);

  const timeAt = (clientX: number) => {
    const el = trackRef.current;
    if (!el || duration <= 0) return 0;
    const r = el.getBoundingClientRect();
    return Math.min(1, Math.max(0, (clientX - r.left) / r.width)) * duration;
  };

  /**
   * Send only one command at a time while the pointer moves. The newest
   * request waits and replaces any older one that did not go out yet.
   */
  const send = (fn: () => Promise<unknown>) => {
    if (inFlight.current) {
      pending.current = fn;
      return;
    }
    inFlight.current = true;
    fn()
      .catch(() => {})
      .finally(() => {
        inFlight.current = false;
        const next = pending.current;
        pending.current = null;
        if (next) send(next);
      });
  };

  const drag = (e: React.PointerEvent, onMove: (t: number) => void) => {
    if (disabled || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    onMove(timeAt(e.clientX));
    const move = (ev: PointerEvent) => onMove(timeAt(ev.clientX));
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };

  const seekTo = (t: number) => send(() => invoke("seek", { seconds: t }));

  const moveA = (t: number) => {
    const max = loopB !== null ? loopB - MIN_GAP : duration;
    const v = Math.max(0, Math.min(t, max));
    send(() => invoke("set_loop_point", { which: "a", seconds: v }));
  };

  const moveB = (t: number) => {
    const min = loopA !== null ? loopA + MIN_GAP : 0;
    const v = Math.min(duration, Math.max(t, min));
    send(() => invoke("set_loop_point", { which: "b", seconds: v }));
  };

  const showLoop = loopA !== null && loopB !== null && duration > 0;

  return (
    <div
      ref={trackRef}
      onPointerDown={(e) => drag(e, seekTo)}
      className={
        "relative flex-1 h-6 flex items-center " +
        (disabled ? "opacity-40 cursor-not-allowed" : "cursor-pointer")
      }
    >
      <div className="absolute inset-x-0 h-1.5 rounded-full bg-muted" />
      {showLoop && (
        <div
          className="absolute h-1.5 bg-emerald-400/35"
          style={{ left: `${pct(loopA!)}%`, width: `${pct(loopB!) - pct(loopA!)}%` }}
        />
      )}
      <div
        className="absolute left-0 h-1.5 rounded-full bg-primary"
        style={{ width: `${pct(position)}%` }}
      />
      <div
        className="absolute w-3.5 h-3.5 -ml-[7px] rounded-full bg-primary shadow pointer-events-none"
        style={{ left: `${pct(position)}%` }}
      />
      {loopA !== null && duration > 0 && (
        <Handle label="A" left={pct(loopA)} onPointerDown={(e) => drag(e, moveA)} />
      )}
      {loopB !== null && duration > 0 && (
        <Handle label="B" left={pct(loopB)} onPointerDown={(e) => drag(e, moveB)} />
      )}
    </div>
  );
}

function Handle({
  label,
  left,
  onPointerDown,
}: {
  label: string;
  left: number;
  onPointerDown: (e: React.PointerEvent) => void;
}) {
  return (
    <div
      onPointerDown={onPointerDown}
      className="absolute top-0 bottom-0 w-4 -ml-2 flex flex-col items-center cursor-ew-resize z-10 group"
      style={{ left: `${left}%` }}
      title={`Drag to move loop ${label === "A" ? "start" : "end"}`}
    >
      <span className="text-[9px] leading-none font-bold text-emerald-400 group-hover:text-emerald-300">
        {label}
      </span>
      <div className="flex-1 w-0.5 bg-emerald-400 group-hover:bg-emerald-300 rounded-full" />
    </div>
  );
}
