import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export type Tone = "neutral" | "ok" | "run" | "fail" | "hold" | "accent";

const tones: Record<Tone, string> = {
  neutral: "bg-inset text-ink-2 border-line",
  ok: "bg-ok-soft text-ok border-ok/20",
  run: "bg-run-soft text-run border-run/20",
  fail: "bg-fail-soft text-fail border-fail/20",
  hold: "bg-hold-soft text-hold border-hold/20",
  accent: "bg-accent-soft text-accent border-accent/20",
};

export function Badge({
  tone = "neutral",
  children,
  className,
  mono,
}: {
  tone?: Tone;
  children: ReactNode;
  className?: string;
  mono?: boolean;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 text-[12px] font-medium leading-5 py-0.5",
        mono && "font-mono text-[11.5px]",
        tones[tone],
        className,
      )}
    >
      {children}
    </span>
  );
}
