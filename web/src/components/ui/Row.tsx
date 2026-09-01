import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export function Row({
  label,
  children,
  hint,
  className,
}: {
  label: ReactNode;
  children: ReactNode;
  hint?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex items-baseline justify-between gap-6 py-2.5 border-b border-line last:border-0", className)}>
      <div className="min-w-0">
        <dt className="text-[13px] text-ink-3">{label}</dt>
        {hint ? <p className="text-[12px] text-ink-4 mt-0.5">{hint}</p> : null}
      </div>
      <dd className="text-[13.5px] text-ink text-right min-w-0 truncate">{children}</dd>
    </div>
  );
}
