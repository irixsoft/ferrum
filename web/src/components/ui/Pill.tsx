import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export function Pill({
  active,
  onClick,
  children,
  className,
}: {
  active?: boolean;
  onClick?: () => void;
  children: ReactNode;
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "h-12 px-6 rounded-full text-[14.5px] font-medium whitespace-nowrap transition-colors duration-100",
        active
          ? "bg-ink text-canvas"
          : "bg-transparent text-ink-2 border border-line-strong/70 hover:border-ink-3 hover:text-ink",
        className,
      )}
    >
      {children}
    </button>
  );
}

export function PillIcon({
  onClick,
  label,
  children,
  className,
}: {
  onClick?: () => void;
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className={cn(
        "h-12 w-12 grid place-items-center rounded-full text-ink-2",
        "border border-line-strong/70 hover:border-ink-3 hover:text-ink transition-colors duration-100",
        className,
      )}
    >
      {children}
    </button>
  );
}
