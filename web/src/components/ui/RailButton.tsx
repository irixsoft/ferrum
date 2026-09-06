import { Link } from "@tanstack/react-router";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

/** The rail is icon-only, so every button needs both a visible label and an aria-label. */
export function RailButton({
  to,
  label,
  icon: Icon,
  active,
  dot,
}: {
  to: string;
  label: string;
  icon: LucideIcon;
  active: boolean;
  dot?: boolean;
}) {
  return (
    <Link
      to={to}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      className="group relative grid place-items-center"
    >
      <span
        className={cn(
          "relative h-12 w-12 grid place-items-center rounded-full transition-colors duration-100",
          active
            ? "bg-ink text-canvas"
            : "bg-surface text-ink-3 border border-line group-hover:text-ink group-hover:border-line-strong",
        )}
      >
        <Icon size={19} strokeWidth={active ? 2.1 : 1.8} />
        {dot ? (
          <span className="absolute top-0.5 right-0.5 h-2.5 w-2.5 rounded-full bg-run border-2 border-canvas" />
        ) : null}
      </span>

      <span
        role="tooltip"
        className={cn(
          "pointer-events-none absolute left-[calc(100%+10px)] top-1/2 -translate-y-1/2 z-40",
          "whitespace-nowrap rounded-control bg-ink text-canvas text-[12.5px] font-medium px-2.5 py-1.5",
          "opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100 transition-opacity duration-100",
        )}
      >
        {label}
      </span>
    </Link>
  );
}
