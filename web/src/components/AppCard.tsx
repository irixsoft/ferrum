import { Link } from "@tanstack/react-router";
import { ArrowUpRight, GitBranch, Tag } from "lucide-react";
import type { App } from "@/types/api";
import { RuntimeMark } from "./RuntimeMark";
import { StatusPill } from "./StatusPill";
import { ago } from "@/lib/utils";

export function AppCard({ app }: { app: App }) {
  const primary = app.domains[0];
  const isStatic = app.runtime === "static";

  return (
    <Link
      to="/apps/$slug"
      params={{ slug: app.slug }}
      className="group block bg-surface border border-line rounded-card p-4 hover:border-line-strong transition-colors duration-100"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="font-display text-[19px] text-ink leading-none">{app.name}</h3>
          <p className="mt-1.5 text-[13px] text-ink-3 truncate">{primary ?? "No domain yet"}</p>
        </div>
        <StatusPill status={app.status} neverLive={app.never_live} />
      </div>

      <div className="mt-4 flex items-center gap-3 flex-wrap">
        <RuntimeMark runtime={app.runtime} version={app.runtime_version} />
        <span className="inline-flex items-center gap-1 text-[12.5px] text-ink-4 font-mono">
          {app.tracking === "releases" ? <Tag size={11} /> : <GitBranch size={11} />}
          {app.git_ref}
        </span>
      </div>

      <p className="mt-4 text-[12.5px] text-ink-4">
        {isStatic
          ? "Served by nginx from disk — no process, no memory limit."
          : `Up to ${app.memory_mb} MB and ${app.cpu_percent}% CPU.`}
      </p>

      <div className="mt-4 pt-3 border-t border-line flex items-center justify-between gap-2">
        <span className="text-[12.5px] text-ink-4 truncate">
          {app.current_release_id ? "Deployed" : "Never deployed"} · created {ago(app.created_at)}
        </span>
        <ArrowUpRight
          size={14}
          className="text-ink-4 group-hover:text-ink transition-colors duration-100 shrink-0"
        />
      </div>
    </Link>
  );
}
