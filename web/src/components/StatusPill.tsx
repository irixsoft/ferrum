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

export function StatusPill({ status }: { status: AppStatus }) {
  const s = STATUS[status];
  return (
    <Badge tone={s.tone}>
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
      {s.label}
    </Badge>
  );
}

export const statusLabel = (s: AppStatus) => STATUS[s].label;
