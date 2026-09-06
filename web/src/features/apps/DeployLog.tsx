import { useEffect, useRef, useState } from "react";
import { ApiError, followDeployLog } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { DeployOutcome, LogLine } from "@/types/api";

const STREAM: Record<LogLine["stream"], string> = {
  system: "text-ink font-medium",
  stdout: "text-ink-2",
  stderr: "text-hold",
};

/** Follows the SSE stream and sticks to the bottom until the reader scrolls up. */
export function DeployLog({ id, className }: { id: string; className?: string }) {
  const [lines, setLines] = useState<LogLine[]>([]);
  const [outcome, setOutcome] = useState<DeployOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pinned, setPinned] = useState(true);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const controller = new AbortController();
    setLines([]);
    setOutcome(null);
    setError(null);
    followDeployLog(id, (line) => setLines((prev) => [...prev, line]), controller.signal)
      .then(setOutcome)
      .catch((e: unknown) => {
        if (controller.signal.aborted) return;
        setError(e instanceof ApiError ? e.message : String(e));
      });
    return () => controller.abort();
  }, [id]);

  useEffect(() => {
    if (pinned && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines, pinned]);

  return (
    <div className={cn("grid gap-2", className)}>
      <div
        ref={box}
        onScroll={(e) => {
          const el = e.currentTarget;
          setPinned(el.scrollHeight - el.scrollTop - el.clientHeight < 24);
        }}
        className="max-h-[50vh] overflow-y-auto rounded-control bg-inset border border-line px-3 py-2 font-mono text-[12px] leading-5"
      >
        {lines.length === 0 && !outcome && !error ? (
          <p className="text-ink-4">Waiting for the first line…</p>
        ) : null}
        {lines.map((l) => (
          <div key={l.seq} className={cn("whitespace-pre-wrap break-all", STREAM[l.stream])}>
            {l.text}
          </div>
        ))}
        {outcome ? (
          <div
            className={cn(
              "mt-2 pt-2 border-t border-line font-medium",
              outcome === "Live" ? "text-ok" : "text-fail",
            )}
          >
            {outcome === "Live" ? "Live" : outcome === "RolledBack" ? "Rolled back" : "Failed"}
          </div>
        ) : null}
        {error ? <div className="mt-2 text-fail">{error}</div> : null}
      </div>
      {!pinned && !outcome ? (
        <button
          onClick={() => setPinned(true)}
          className="justify-self-end text-[12.5px] text-ink-3 hover:text-ink"
        >
          Follow the log
        </button>
      ) : null}
    </div>
  );
}
