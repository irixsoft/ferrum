import { cn } from "@/lib/utils";

export function Meter({
  value,
  tone = "neutral",
  className,
}: {
  value: number;
  tone?: "neutral" | "ok" | "run" | "fail" | "accent";
  className?: string;
}) {
  const fill = {
    neutral: "bg-ink-3",
    ok: "bg-ok",
    run: "bg-run",
    fail: "bg-fail",
    accent: "bg-accent",
  }[tone];
  return (
    <div className={cn("h-1.5 rounded-full bg-line overflow-hidden", className)}>
      <div
        className={cn("h-full rounded-full", fill)}
        style={{ width: `${Math.min(100, Math.max(0, value))}%` }}
      />
    </div>
  );
}
