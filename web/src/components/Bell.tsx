import { useState } from "react";
import { Bell as BellIcon } from "lucide-react";
import { EventsSheet } from "@/features/events/EventsSheet";
import { useUnreadCount } from "@/lib/api";
import { cn } from "@/lib/utils";

/** `compact` is the mobile header's ghost icon; otherwise it matches the desktop top-bar pills. */
export function Bell({ compact = false }: { compact?: boolean }) {
  const [open, setOpen] = useState(false);
  const { data } = useUnreadCount();
  const count = data?.count ?? 0;
  const label = count > 0 ? `Events, ${count} unread` : "Events";

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={label}
        title={label}
        className={cn(
          "relative grid place-items-center rounded-full text-ink-2 hover:text-ink transition-colors duration-100",
          compact ? "h-9 w-9 hover:bg-inset" : "h-12 w-12 border border-line-strong/70 hover:border-ink-3",
        )}
      >
        <BellIcon size={compact ? 16 : 17} />
        {count > 0 ? (
          <span
            className={cn(
              "absolute min-w-[18px] h-[18px] px-1 rounded-full bg-ink text-canvas border-2 border-canvas",
              "text-[10.5px] font-semibold leading-[14px] text-center tnum",
              compact ? "-top-0.5 -right-0.5" : "top-0 right-0",
            )}
          >
            {count > 99 ? "99+" : count}
          </span>
        ) : null}
      </button>
      <EventsSheet open={open} onClose={() => setOpen(false)} side={compact ? "bottom" : "center"} />
    </>
  );
}
