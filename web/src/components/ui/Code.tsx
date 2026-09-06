import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export function Code({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <code
      className={cn(
        "font-mono text-[12.5px] bg-inset border border-line rounded px-1.5 py-0.5 text-ink-2",
        className,
      )}
    >
      {children}
    </code>
  );
}
