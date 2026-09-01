import { useEffect, useState } from "react";
import type { Deploy, DeployState, DeployStep } from "@/types/api";
import { cn, duration } from "@/lib/utils";

/** Never add a spinner beside this: the panel has no continuously animating CSS. */

const LABELS: Record<DeployState, string> = {
  Queued: "Queued",
  Cloning: "Cloning repository",
  InstallingSystemPackages: "Installing system packages",
  InstallingDeps: "Installing dependencies",
  Building: "Building",
  Snapshotting: "Snapshotting database",
  MaintenanceOn: "Pausing traffic",
  Migrating: "Running migrations",
  Swapping: "Swapping release",
  Restarting: "Restarting",
  HealthChecking: "Health checking",
  MaintenanceOff: "Restoring traffic",
};

const MARK: Record<DeployStep["status"], string> = {
  done: "bg-ok",
  active: "bg-run",
  failed: "bg-fail",
  pending: "bg-line-strong",
  skipped: "bg-transparent border border-dashed border-line-strong",
};

function useTicker(active: boolean, from: string) {
  const [secs, setSecs] = useState(0);
  useEffect(() => {
    if (!active) return;
    const start = new Date(from).getTime();
    const tick = () => setSecs((Date.now() - start) / 1000);
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [active, from]);
  return secs;
}

export function DeployLadder({ deploy, className }: { deploy: Deploy; className?: string }) {
  const running = deploy.state !== null;
  const activeIdx = deploy.steps.findIndex((s) => s.status === "active");
  const banked = deploy.steps.reduce((n, s) => n + (s.elapsed_secs ?? 0), 0);
  const live = useTicker(running, deploy.started_at);

  return (
    <ol className={cn("relative", className)}>
      {deploy.steps.map((step, i) => {
        const isActive = step.status === "active";
        const isLast = i === deploy.steps.length - 1;
        const reached = i <= activeIdx || activeIdx === -1;
        return (
          <li key={step.state} className="relative flex items-start gap-3 pb-3 last:pb-0">
            <div className="relative flex flex-col items-center pt-1">
              <span className={cn("h-2.5 w-2.5 rounded-[3px] shrink-0", MARK[step.status])} />
              {!isLast ? (
                <span
                  className={cn(
                    "w-px flex-1 min-h-5 mt-1",
                    reached && step.status !== "pending" ? "bg-line-strong" : "bg-line",
                  )}
                />
              ) : null}
            </div>

            <div className="flex-1 min-w-0 flex items-baseline justify-between gap-3">
              <div className="min-w-0">
                <span
                  className={cn(
                    "text-[13.5px]",
                    isActive && "text-ink font-medium",
                    step.status === "done" && "text-ink-2",
                    step.status === "failed" && "text-fail font-medium",
                    step.status === "pending" && "text-ink-4",
                    step.status === "skipped" && "text-ink-4",
                  )}
                >
                  {LABELS[step.state]}
                </span>
                {step.note ? (
                  <span className="ml-2 text-[12px] text-ink-4">— {step.note}</span>
                ) : null}
              </div>
              <span
                className={cn(
                  "font-mono text-[12px] tnum shrink-0",
                  isActive ? "text-run" : "text-ink-4",
                )}
              >
                {step.status === "skipped"
                  ? "skipped"
                  : isActive
                    ? duration(Math.max(0, live - banked))
                    : step.elapsed_secs !== null
                      ? duration(step.elapsed_secs)
                      : ""}
              </span>
            </div>
          </li>
        );
      })}
    </ol>
  );
}

export function DeployRail({ deploy, className }: { deploy: Deploy; className?: string }) {
  return (
    <div className={cn("flex items-center gap-[3px]", className)} aria-hidden="true">
      {deploy.steps.map((s) => (
        <span
          key={s.state}
          className={cn(
            "h-3.5 w-[3px] rounded-full",
            s.status === "done" && "bg-ok",
            s.status === "active" && "bg-run",
            s.status === "failed" && "bg-fail",
            s.status === "pending" && "bg-line",
            s.status === "skipped" && "bg-line opacity-50",
          )}
        />
      ))}
    </div>
  );
}

export { LABELS as DEPLOY_STATE_LABELS };
