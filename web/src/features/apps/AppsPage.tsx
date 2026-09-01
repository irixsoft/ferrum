import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useApps } from "@/lib/api";
import { useShell } from "@/shells/useShell";
import { PageTitle } from "@/components/PageTitle";
import { AppCard } from "@/components/AppCard";
import { RuntimeMark } from "@/components/RuntimeMark";
import { StatusPill } from "@/components/StatusPill";
import { Card } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Meter } from "@/components/ui/Meter";
import { EmptyState } from "@/components/ui/EmptyState";
import { ago, pct } from "@/lib/utils";

export function AppsPage() {
  const { data: apps = [], isLoading } = useApps();
  const { shell } = useShell();

  return (
    <>
      <PageTitle
        above="Every application Ferrum runs on this box"
        title="Apps"
        action={
          <Button variant="primary">
            <Plus size={15} />
            New app
          </Button>
        }
      />

      {!isLoading && apps.length === 0 ? (
        <Card>
          <EmptyState
            title="Nothing deployed yet"
            body="Connect a GitHub repository and Ferrum will detect the runtime, prefill the build commands, and show you every field before anything touches the server."
            action={<Button variant="primary">Connect a repository</Button>}
          />
        </Card>
      ) : shell === "mobile" ? (
        <div className="grid gap-3">
          {apps.map((a) => (
            <AppCard key={a.slug} app={a} />
          ))}
        </div>
      ) : (
        <Card>
          <table className="w-full text-left">
            <thead>
              <tr className="border-b border-line text-[12px] text-ink-3">
                <th className="font-medium px-5 py-2.5">App</th>
                <th className="font-medium px-3 py-2.5">Runtime</th>
                <th className="font-medium px-3 py-2.5">Tracking</th>
                <th className="font-medium px-3 py-2.5 w-44">Memory</th>
                <th className="font-medium px-3 py-2.5">Last deploy</th>
                <th className="font-medium px-5 py-2.5 text-right">Status</th>
              </tr>
            </thead>
            <tbody>
              {apps.map((app) => {
                const primary = app.domains.find((d) => d.primary) ?? app.domains[0];
                const mem = pct(app.resources.memory_current_mb, app.resources.memory_max_mb);
                return (
                  <tr
                    key={app.slug}
                    className="border-b border-line last:border-0 hover:bg-inset/60 transition-colors duration-75"
                  >
                    <td className="px-5 py-3">
                      <Link to="/apps/$slug" params={{ slug: app.slug }} className="block">
                        <span className="text-[14px] font-medium text-ink">{app.name}</span>
                        <span className="block text-[12.5px] text-ink-4 truncate">
                          {primary ? primary.host : "No domain"}
                        </span>
                      </Link>
                    </td>
                    <td className="px-3 py-3">
                      <RuntimeMark runtime={app.runtime} version={app.runtime_version} />
                    </td>
                    <td className="px-3 py-3 font-mono text-[12.5px] text-ink-3">
                      {app.repo}
                      <span className="text-ink-4"> @ {app.ref}</span>
                    </td>
                    <td className="px-3 py-3">
                      {app.runtime === "static" ? (
                        <span className="text-[12.5px] text-ink-4">No process</span>
                      ) : (
                        <>
                          <Meter value={mem} tone={mem > 85 ? "fail" : mem > 70 ? "run" : "neutral"} />
                          <span className="block mt-1 font-mono text-[11.5px] text-ink-4 tnum">
                            {app.resources.memory_current_mb} / {app.resources.memory_max_mb} MB
                          </span>
                        </>
                      )}
                    </td>
                    <td className="px-3 py-3 text-[12.5px] text-ink-3">
                      {app.last_deploy ? (
                        <>
                          <span className="font-mono">{app.last_deploy.commit_sha}</span>
                          <span className="block text-ink-4">{ago(app.last_deploy.started_at)}</span>
                        </>
                      ) : (
                        <span className="text-ink-4">Never</span>
                      )}
                    </td>
                    <td className="px-5 py-3 text-right">
                      <StatusPill status={app.status} />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </Card>
      )}
    </>
  );
}
