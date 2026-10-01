import { useRouter } from "@tanstack/react-router";
import { Sheet } from "@/components/ui/Sheet";
import { Button } from "@/components/ui/Button";
import { EmptyState } from "@/components/ui/EmptyState";
import { useEvents, useMarkRead } from "@/lib/api";
import { ago, cn } from "@/lib/utils";
import type { EventKind, FerrumEvent } from "@/types/api";

const TITLES: Record<EventKind, string> = {
  deploy_refused: "Deploy refused",
  deploy_failed: "Deploy failed",
  deploy_live: "Deploy live",
  broke_on_its_own: "Something broke",
  update_available: "Update available",
  package_dropped: "Package dropped from the Aptfile",
  port_removed: "Route removed",
  role_kept: "Role kept",
  processes_changed: "Processes changed",
};

export function EventsSheet({
  open,
  onClose,
  side,
}: {
  open: boolean;
  onClose: () => void;
  side: "bottom" | "center";
}) {
  const markRead = useMarkRead();

  return (
    <Sheet
      open={open}
      onClose={onClose}
      side={side}
      title="Events"
      footer={
        <div className="flex items-center justify-between gap-3">
          <span className="text-[12.5px] text-ink-4">The last 50, newest first</span>
          <Button size="sm" disabled={markRead.isPending} onClick={() => markRead.mutate({ all: true })}>
            Mark all read
          </Button>
        </div>
      }
    >
      {open ? <EventList onClose={onClose} /> : null}
    </Sheet>
  );
}

function EventList({ onClose }: { onClose: () => void }) {
  const { data: events, error } = useEvents();
  const markRead = useMarkRead();
  const router = useRouter();

  if (error) return <p className="text-[13px] text-fail">{error.message}</p>;
  if (!events) return null;
  if (events.length === 0) {
    return (
      <EmptyState
        title="Nothing has happened yet"
        body="Deploys, updates, and anything that stops on its own will be listed here."
      />
    );
  }

  const open = (event: FerrumEvent) => {
    if (!event.read_at) markRead.mutate({ ids: [event.id] });
    if (!event.link) return;
    onClose();
    router.history.push(event.link);
  };

  return (
    <ul className="-mx-5 -my-4">
      {events.map((event) => (
        <li key={event.id} className="border-b border-line last:border-0">
          <button
            type="button"
            onClick={() => open(event)}
            className="w-full text-left px-5 py-3 flex items-start gap-3 hover:bg-inset transition-colors duration-100"
          >
            <span
              aria-hidden
              className={cn("mt-1.5 h-2 w-2 shrink-0 rounded-full", event.read_at ? "bg-transparent" : "bg-ink")}
            />
            <span className="min-w-0 flex-1">
              <span className="flex items-baseline gap-3">
                <span className={cn("text-[13.5px] text-ink", !event.read_at && "font-medium")}>
                  {TITLES[event.kind] ?? event.kind}
                </span>
                <span className="ml-auto shrink-0 text-[12px] text-ink-4 tnum">{ago(event.created_at)}</span>
              </span>
              <span className="block text-[13px] text-ink-3 mt-0.5">{event.sentence}</span>
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}
