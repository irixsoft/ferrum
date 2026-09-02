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
import { EmptyState } from "@/components/ui/EmptyState";
import { ago } from "@/lib/utils";

export function AppsPage() {
  const { data: apps = [], isLoading } = useApps();
  const { shell } = useShell();

  return (
    <>
      <PageTitle
        above="Every application Ferrum runs on this box"
        title="Apps"
        action={
          <Link to="/apps/new">
            <Button variant="primary">
              <Plus size={15} />
              New app
            </Button>
          </Link>
        }
      />

      {!isLoading && apps.length === 0 ? (
        <Card>
          <EmptyState
            title="Nothing deployed yet"
            body="Pick a GitHub repository and Ferrum will detect the runtime, prefill the build commands, and show you every field before anything touches the server."
            action={
              <Link to="/apps/new">
                <Button variant="primary">Create an app</Button>
              </Link>
            }
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
                <th className="font-medium px-3 py-2.5">Limits</th>
                <th className="font-medium px-3 py-2.5">Created</th>
                <th className="font-medium px-5 py-2.5 text-right">Status</th>
              </tr>
            </thead>
            <tbody>
              {apps.map((app) => (
                <tr
                  key={app.slug}
                  className="border-b border-line last:border-0 hover:bg-inset/60 transition-colors duration-75"
                >
                  <td className="px-5 py-3">
                    <Link to="/apps/$slug" params={{ slug: app.slug }} className="block">
                      <span className="text-[14px] font-medium text-ink">{app.name}</span>
                      <span className="block text-[12.5px] text-ink-4 truncate">
                        {app.domains[0] ?? "No domain"}
                      </span>
                    </Link>
                  </td>
                  <td className="px-3 py-3">
                    <RuntimeMark runtime={app.runtime} version={app.runtime_version} />
                  </td>
                  <td className="px-3 py-3 font-mono text-[12.5px] text-ink-3">
                    {app.repository}
                    <span className="text-ink-4"> @ {app.git_ref}</span>
                  </td>
                  <td className="px-3 py-3 text-[12.5px] text-ink-3">
                    {app.runtime === "static" ? (
                      <span className="text-ink-4">No process</span>
                    ) : (
                      <span className="font-mono tnum">
                        {app.memory_mb} MB · {app.cpu_percent}%
                      </span>
                    )}
                  </td>
                  <td className="px-3 py-3 text-[12.5px] text-ink-3">{ago(app.created_at)}</td>
                  <td className="px-5 py-3 text-right">
                    <StatusPill status="new" />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
    </>
  );
}
