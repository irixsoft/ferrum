import type { Runtime } from "@/types/api";
import { cn } from "@/lib/utils";


const RUNTIMES: Record<Runtime, { label: string; className: string }> = {
  node: { label: "Node", className: "text-node" },
  bun: { label: "Bun", className: "text-bun" },
  dotnet: { label: ".NET", className: "text-dotnet" },
  static: { label: "Static", className: "text-static" },
};

export function RuntimeMark({
  runtime,
  version,
  className,
}: {
  runtime: Runtime;
  version?: string;
  className?: string;
}) {
  const r = RUNTIMES[runtime];
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-[12.5px]", className)}>
      <span className={cn("h-2 w-2 rounded-full bg-current shrink-0", r.className)} />
      <span className="text-ink-2 font-medium">{r.label}</span>
      {version && version !== "—" ? (
        <span className="font-mono text-[11.5px] text-ink-4 tnum">{version}</span>
      ) : null}
    </span>
  );
}

export const runtimeLabel = (r: Runtime) => RUNTIMES[r].label;
export const runtimeColor = (r: Runtime) => RUNTIMES[r].className;
