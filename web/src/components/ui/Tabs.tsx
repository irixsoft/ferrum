import { cn } from "@/lib/utils";

export function Tabs<T extends string>({
  tabs,
  value,
  onChange,
  className,
}: {
  tabs: Array<{ value: T; label: string; count?: number }>;
  value: T;
  onChange: (v: T) => void;
  className?: string;
}) {
  return (
    <div className={cn("border-b border-line overflow-x-auto no-scrollbar", className)}>
      <div className="flex items-center gap-1 min-w-max">
        {tabs.map((t) => {
          const active = t.value === value;
          return (
            <button
              key={t.value}
              onClick={() => onChange(t.value)}
              className={cn(
                "relative h-10 px-3 text-[13.5px] font-medium whitespace-nowrap transition-colors duration-100",
                active ? "text-ink" : "text-ink-3 hover:text-ink-2",
              )}
            >
              {t.label}
              {typeof t.count === "number" ? (
                <span className="ml-1.5 text-ink-4 tnum">{t.count}</span>
              ) : null}
              {active ? (
                <span className="absolute inset-x-2 -bottom-px h-0.5 bg-ink rounded-full" />
              ) : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}
