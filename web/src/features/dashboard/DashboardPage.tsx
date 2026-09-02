import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { CircleCheck, CircleX, Plus } from "lucide-react";
import { useApps, useCancelDeploy, useDeploys, useHost, useMetrics, useRunningDeploy } from "@/lib/api";
import { ago, duration } from "@/lib/utils";
import { PageTitle } from "@/components/PageTitle";
import { AppCard } from "@/components/AppCard";
import { DeployLadder } from "@/components/DeployLadder";
import { ChartKey, MetricChart, type Band } from "@/components/MetricChart";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { uptime } from "@/lib/utils";
import { useRange } from "@/lib/range";
import { SearchPill } from "@/components/SearchPill";
import { Segmented } from "@/components/ui/Segmented";

export function DashboardPage() {
  const { data: host } = useHost();
  const { data: apps = [] } = useApps();
  const { data: deploy } = useRunningDeploy();
  const { data: recent = [] } = useDeploys();
  const cancel = useCancelDeploy();

  if (!host) return null;

  const attention = host.services.filter((s) => !s.ok);
  const finished = recent.filter((d) => d.state === null).slice(0, 5);

  return (
    <>
      <PageTitle
        above={`${host.os} · ${host.arch} · up ${uptime(host.uptime_secs)}`}
        title={host.hostname}
        action={
          <>
            <SearchPill />
            <Link to="/apps/new">
              <Button variant="primary" className="h-12 sm:h-14 px-5 sm:px-6 rounded-full shrink-0">
                <Plus size={16} />
                New app
              </Button>
            </Link>
          </>
        }
      />

      <div className="grid gap-4 lg:grid-cols-12">
        <div className="lg:col-span-7">
          {deploy ? (
            <Card>
              <CardHeader
                title={
                  <span className="flex items-center gap-2">
                    {deploy.state === "Queued" ? "Queued" : "Deploying"} {deploy.app_slug}
                    <Badge tone="run">{deploy.state === "Queued" ? "Waiting" : "In progress"}</Badge>
                  </span>
                }
                hint={`${deploy.commit_sha?.slice(0, 7) ?? deploy.git_ref}${deploy.commit_message ? ` — ${deploy.commit_message}` : ""}`}
                action={
                  <Link to="/apps/$slug" params={{ slug: deploy.app_slug }}>
                    <Button size="sm" variant="ghost">
                      View log
                    </Button>
                  </Link>
                }
              />
              <CardBody>
                <DeployLadder deploy={deploy} />
              </CardBody>
              <CardFoot>
                <span>Builds run one at a time so live apps keep their memory.</span>
                {deploy.state === "Queued" ? (
                  <Button size="sm" variant="danger" disabled={cancel.isPending} onClick={() => cancel.mutate(deploy.id)}>
                    Cancel
                  </Button>
                ) : null}
              </CardFoot>
            </Card>
          ) : (
            <Card>
              <CardHeader
                title="No deploy running"
                hint={
                  finished[0]
                    ? `The last one, ${finished[0].app_slug} ${ago(finished[0].started_at)}, ${finished[0].outcome === "Live" ? "went live" : finished[0].outcome === "RolledBack" ? "was rolled back" : "failed"}.`
                    : "Nothing has been deployed yet."
                }
              />
              <CardBody>
                {finished.length === 0 ? (
                  <p className="text-[13.5px] text-ink-3">
                    Push to a tracked branch, or deploy a ref by hand from an app's page.
                  </p>
                ) : (
                  <ul className="divide-y divide-line">
                    {finished.map((d) => (
                      <li key={d.id} className="py-2 flex items-center gap-3 text-[13px]">
                        <Link to="/apps/$slug" params={{ slug: d.app_slug }} className="text-ink hover:underline shrink-0">
                          {d.app_slug}
                        </Link>
                        <span className="font-mono text-[12.5px] text-ink-3 truncate">
                          {d.commit_sha?.slice(0, 7) ?? d.git_ref}
                          {d.commit_message ? ` ${d.commit_message}` : ""}
                        </span>
                        <span className="ml-auto text-[12px] text-ink-4 shrink-0">
                          {ago(d.started_at)}
                          {d.duration_secs !== null ? ` · ${duration(d.duration_secs)}` : ""}
                        </span>
                        <Badge tone={d.outcome === "Live" ? "ok" : d.outcome === "RolledBack" ? "hold" : "fail"}>
                          {d.outcome === "Live" ? "Live" : d.outcome === "RolledBack" ? "Rolled back" : "Failed"}
                        </Badge>
                      </li>
                    ))}
                  </ul>
                )}
              </CardBody>
            </Card>
          )}
        </div>

        <div className="lg:col-span-5">
          <Card>
            <CardHeader
              title="Services"
              hint={
                attention.length
                  ? `${attention.length} need${attention.length > 1 ? "" : "s"} attention`
                  : "All healthy"
              }
            />
            <CardBody className="pb-3">
              <ul>
                {host.services.map((s) => (
                  <li
                    key={s.name}
                    className="flex items-center gap-3 py-2.5 border-b border-line last:border-0"
                  >
                    {s.ok ? (
                      <CircleCheck size={15} className="text-ok shrink-0" />
                    ) : (
                      <CircleX size={15} className="text-run shrink-0" />
                    )}
                    <span className="text-[13.5px] text-ink shrink-0">{s.name}</span>
                    <span className="ml-auto text-[12.5px] text-ink-4 truncate text-right min-w-0">
                      {s.detail}
                    </span>
                  </li>
                ))}
              </ul>
            </CardBody>
            <CardFoot>
              <span>
                Ferrum installs these from their own upstream repositories, so security
                patches arrive without waiting for a Ferrum release.
              </span>
            </CardFoot>
          </Card>
        </div>

        <div className="lg:col-span-12">
          <div className="flex items-end justify-between mb-3 mt-2">
            <h2 className="font-display text-[22px] text-ink">
              {apps.length} apps on this box
            </h2>
            <Link to="/apps" className="text-[13px] text-ink-3 hover:text-ink">
              See all
            </Link>
          </div>
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {apps.slice(0, 3).map((a) => (
              <AppCard key={a.slug} app={a} />
            ))}
          </div>
        </div>

        <div className="lg:col-span-12">
          <HostMetrics />
        </div>
      </div>
    </>
  );
}

const BANDS: Record<"cpu" | "memory", Band[]> = {
  cpu: [{ key: "cpu", label: "CPU", varName: "--c-accent", fill: true }],
  memory: [{ key: "memory", label: "Memory", varName: "--c-hold", fill: true }],
};

function HostMetrics() {
  const { range } = useRange();
  const { data: series } = useMetrics("host", range);
  const [band, setBand] = useState<"cpu" | "memory">("cpu");
  if (!series) return null;

  return (
    <Card>
      <CardHeader
        title="Host load"
        hint="Sampled every 10 seconds, kept for 7 days"
        action={
          <Segmented
            value={band}
            onChange={setBand}
            options={[
              { value: "cpu", label: "CPU" },
              { value: "memory", label: "Memory" },
            ]}
          />
        }
      />
      <CardBody>
        <MetricChart {...series} bands={BANDS[band]} height={190} />
        <div className="mt-3">
          <ChartKey bands={BANDS[band]} />
        </div>
      </CardBody>
    </Card>
  );
}
