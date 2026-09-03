import type { AppStatus } from "@/types/api";
import { Badge, type Tone } from "./ui/Badge";

const STATUS: Record<AppStatus, { label: string; tone: Tone }> = {
  new: { label: "Not deployed", tone: "neutral" },
  live: { label: "Live", tone: "ok" },
  building: { label: "Deploying", tone: "run" },
  failed: { label: "Failed", tone: "fail" },
  stopped: { label: "Stopped", tone: "neutral" },
  maintenance: { label: "Paused", tone: "hold" },
};

export const NEVER_LIVE = "This app has never gone live; the next good deploy starts it.";

export function StatusPill({ status, neverLive = false }: { status: AppStatus; neverLive?: boolean }) {
  const s = STATUS[status];
  const stuck = status === "failed" && neverLive;
  return (
    <Badge tone={s.tone} title={stuck ? NEVER_LIVE : undefined}>
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
      {stuck ? "Failed, never live" : s.label}
    </Badge>
  );
}

export const statusLabel = (s: AppStatus) => STATUS[s].label;
