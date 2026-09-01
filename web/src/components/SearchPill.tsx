import { Command, Search } from "lucide-react";

export function SearchPill() {
  return (
    <button
      type="button"
      className="group flex items-center gap-3 h-12 sm:h-14 pl-2.5 sm:pl-3 pr-4 sm:pr-5 rounded-full bg-surface border border-line hover:border-line-strong transition-colors duration-100 flex-1 sm:flex-none sm:w-[20rem] min-w-0"
    >
      <span className="h-8 w-8 sm:h-9 sm:w-9 grid place-items-center rounded-full bg-inset text-ink-3 shrink-0">
        <Search size={16} />
      </span>
      <span className="text-[14px] sm:text-[14.5px] text-ink-4 group-hover:text-ink-3 truncate">
        Search or jump to
      </span>
      <kbd className="ml-auto hidden sm:flex items-center gap-0.5 font-mono text-[11px] text-ink-4 border border-line rounded px-1.5 py-1 shrink-0">
        <Command size={10} />K
      </kbd>
    </button>
  );
}
