import { Link } from "@tanstack/react-router";
import { ArrowUpRight, GitBranch, Tag } from "lucide-react";
import type { App } from "@/types/api";
import { RuntimeMark } from "./RuntimeMark";
import { StatusPill } from "./StatusPill";
import { Meter } from "./ui/Meter";
import { ago, pct } from "@/lib/utils";

export function AppCard({ app }: { app: App }) {
  const primary = app.domains.find((d) => d.primary) ?? app.domains[0];
  const memory = pct(app.resources.memory_current_mb, app.resources.memory_max_mb);
  const isStatic = app.runtime === "static";
  const tracksTag = /^v?\d/.test(app.ref); // a release tag rather than a branch

  return (
    <Link
      to="/apps/$slug"
      params={{ slug: app.slug }}
      className="group block bg-surface border border-line rounded-card p-4 hover:border-line-strong transition-colors duration-100"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="font-display text-[19px] text-ink leading-none">{app.name}</h3>
          <p className="mt-1.5 text-[13px] text-ink-3 truncate">
            {primary ? primary.host : "No domain yet"}
          </p>
        </div>
        <StatusPill status={app.status} />
      </div>

      <div className="mt-4 flex items-center gap-3 flex-wrap">
        <RuntimeMark runtime={app.runtime} version={app.runtime_version} />
        <span className="inline-flex items-center gap-1 text-[12.5px] text-ink-4 font-mono">
          {tracksTag ? <Tag size={11} /> : <GitBranch size={11} />}
          {app.ref}
        </span>
      </div>

      {isStatic ? (
        <p className="mt-4 text-[12.5px] text-ink-4">
          Served by nginx from disk — no process, no memory limit.
        </p>
      ) : (
        <div className="mt-4">
          <div className="flex items-baseline justify-between mb-1.5">
            <span className="text-[12px] text-ink-3">Memory</span>
            <span className="font-mono text-[12px] text-ink-4 tnum">
              {app.resources.memory_current_mb} / {app.resources.memory_max_mb} MB
            </span>
          </div>
          <Meter value={memory} tone={memory > 85 ? "fail" : memory > 70 ? "run" : "neutral"} />
        </div>
      )}

      <div className="mt-4 pt-3 border-t border-line flex items-center justify-between gap-2">
        <span className="text-[12.5px] text-ink-4 truncate">
          {app.last_deploy
            ? `${app.last_deploy.commit_sha} · ${ago(app.last_deploy.started_at)}`
            : "Never deployed"}
        </span>
        <ArrowUpRight
          size={14}
          className="text-ink-4 group-hover:text-ink transition-colors duration-100 shrink-0"
        />
      </div>
    </Link>
  );
}
