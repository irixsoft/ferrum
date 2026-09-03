import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Boxes, Database, Search } from "lucide-react";
import { useApps, useDatabases } from "@/lib/api";
import { rank, type PaletteItem } from "@/lib/palette";
import { NAV } from "./nav";
import { Sheet } from "./ui/Sheet";
import { cn } from "@/lib/utils";

const OPEN_EVENT = "ferrum:palette";

export const openPalette = () => window.dispatchEvent(new Event(OPEN_EVENT));

const typing = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable);

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();
  const { data: apps = [] } = useApps();
  const { data: databases = [] } = useDatabases();

  const items = useMemo<PaletteItem[]>(
    () => [
      ...NAV.map((n) => ({ kind: "page" as const, label: n.label, href: n.to })),
      ...apps.map((a) => ({ kind: "app" as const, label: a.name, hint: a.slug, href: `/apps/${a.slug}` })),
      ...databases.map((d) => ({ kind: "database" as const, label: d.name, hint: "PostgreSQL", href: "/databases" })),
    ],
    [apps, databases],
  );
  const shown = rank(query, items);
  const current = Math.min(selected, Math.max(shown.length - 1, 0));

  useEffect(() => {
    const show = () => setOpen(true);
    const keys = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((was) => !was);
      } else if (e.key === "/" && !typing(e.target) && !e.metaKey && !e.ctrlKey && !e.altKey) {
        e.preventDefault();
        setOpen(true);
      }
    };
    window.addEventListener(OPEN_EVENT, show);
    window.addEventListener("keydown", keys);
    return () => {
      window.removeEventListener(OPEN_EVENT, show);
      window.removeEventListener("keydown", keys);
    };
  }, []);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelected(0);
      requestAnimationFrame(() => input.current?.focus());
    }
  }, [open]);

  const go = (item: PaletteItem | undefined) => {
    if (!item) return;
    setOpen(false);
    navigate({ href: item.href });
  };

  const onKey = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected(Math.min(current + 1, shown.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected(Math.max(current - 1, 0));
    } else if (e.key === "Enter") {
      go(shown[current]);
    } else if (e.key === "Escape") {
      setOpen(false);
    }
  };

  return (
    <Sheet open={open} onClose={() => setOpen(false)} side="center" title="Jump to">
      <label className="flex items-center gap-2 h-10 px-3 bg-inset border border-line-strong rounded-control">
        <Search size={15} className="text-ink-4 shrink-0" />
        <input
          ref={input}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setSelected(0);
          }}
          onKeyDown={onKey}
          placeholder="A page, an app or a database"
          className="flex-1 min-w-0 bg-transparent text-sm text-ink placeholder:text-ink-4 outline-none"
        />
      </label>
      <ul className="mt-3 -mx-2 max-h-[50vh] overflow-y-auto" role="listbox">
        {shown.length === 0 ? (
          <li className="px-2 py-3 text-[13px] text-ink-4">Nothing here is called that.</li>
        ) : (
          shown.map((item, i) => (
            <li
              key={`${item.kind}:${item.href}:${item.label}`}
              role="option"
              aria-selected={i === current}
              onMouseEnter={() => setSelected(i)}
              onClick={() => go(item)}
              className={cn(
                "flex items-center gap-3 px-2 py-2 rounded-control cursor-pointer",
                i === current ? "bg-inset text-ink" : "text-ink-2",
              )}
            >
              <Kind kind={item.kind} />
              <span className="text-[13.5px] truncate">{item.label}</span>
              {item.hint && item.hint !== item.label ? (
                <span className="ml-auto font-mono text-[11.5px] text-ink-4 truncate">{item.hint}</span>
              ) : null}
            </li>
          ))
        )}
      </ul>
    </Sheet>
  );
}

function Kind({ kind }: { kind: PaletteItem["kind"] }) {
  const Icon = kind === "app" ? Boxes : kind === "database" ? Database : Search;
  return (
    <span className="h-7 w-7 grid place-items-center rounded-full bg-surface border border-line text-ink-3 shrink-0">
      <Icon size={13} />
    </span>
  );
}
