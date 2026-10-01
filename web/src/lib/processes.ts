import type { DomainJob, Process, ProcessInput, RouteInput } from "@/types/api";

export const isFolder = (p: ProcessInput) => p.static_dir !== undefined && p.static_dir !== null;

/** A route can only reach a process that listens on a port, or a folder nginx serves. */
export const routable = (processes: ProcessInput[]) =>
  processes.filter((p) => isFolder(p) || p.port !== false).map((p) => p.name);

/** Points every route whose process can no longer take traffic at the first one that can. */
export function reconcileRoutes(processes: ProcessInput[], routes: RouteInput[]): RouteInput[] {
  const targets = routable(processes);
  const fallback = targets[0];
  const kept = routes.map((r) => (targets.includes(r.process) || !fallback ? r : { ...r, process: fallback }));
  return kept.length || !fallback ? kept : [{ path: "/", process: fallback, websocket: false }];
}

export function renameInRoutes(routes: RouteInput[], from: string, to: string): RouteInput[] {
  return routes.map((r) => (r.process === from ? { ...r, process: to } : r));
}

type Served = { job: DomainJob; target: string };

/** A served name follows its process through a rename, and moves to the first that can take traffic when it is gone. */
export function reconcileDomains<D extends Served>(processes: ProcessInput[], domains: D[]): D[] {
  const targets = routable(processes);
  const fallback = targets[0];
  return domains.map((d) =>
    d.job === "serve" && !targets.includes(d.target) && fallback ? { ...d, target: fallback } : d,
  );
}

export function renameInDomains<D extends Served>(domains: D[], from: string, to: string): D[] {
  return domains.map((d) => (d.job === "serve" && d.target === from ? { ...d, target: to } : d));
}

/** "Served by nginx" for a folder-only app, else the count and what the units may use. */
export function limitsLine(processes: Pick<Process, "kind" | "memory_mb">[], cpuPercent: number): string {
  const commands = processes.filter((p) => p.kind === "command");
  if (commands.length === 0) return "Served by nginx";
  const memory = commands.reduce((sum, p) => sum + p.memory_mb, 0);
  const count = `${processes.length} process${processes.length === 1 ? "" : "es"}`;
  return `${count} · up to ${memory} MB · ${cpuPercent}% CPU`;
}
