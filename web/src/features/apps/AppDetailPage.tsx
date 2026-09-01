import { useState } from "react";
import { ExternalLink, RotateCcw, Upload } from "lucide-react";
import { useApp, useDeploys } from "@/lib/api";
import { useShell } from "@/shells/useShell";
import { PageTitle } from "@/components/PageTitle";
import { SampleData } from "@/components/SampleData";
import { RuntimeMark } from "@/components/RuntimeMark";
import { StatusPill } from "@/components/StatusPill";
import { DeployLadder, DeployRail } from "@/components/DeployLadder";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Meter } from "@/components/ui/Meter";
import { Row } from "@/components/ui/Row";
import { Tabs } from "@/components/ui/Tabs";
import { EnvironmentPanel } from "./EnvironmentPanel";
import { NginxPanel } from "./NginxPanel";
import { LogPanel } from "./LogPanel";
import { ago, duration, pct } from "@/lib/utils";
import type { App } from "@/types/api";

type Tab = "overview" | "deploys" | "logs" | "environment" | "nginx";

export function AppDetailPage({ slug }: { slug: string }) {
  const { data: app, isLoading } = useApp(slug);
  const [tab, setTab] = useState<Tab>("overview");

  if (isLoading) return null;
  if (!app) {
    return (
      <Card>
        <CardBody className="pt-5">
          <p className="text-[13.5px] text-ink-3">
            No app called <Code>{slug}</Code> exists on this box.
          </p>
        </CardBody>
      </Card>
    );
  }

  const primary = app.domains.find((d) => d.primary) ?? app.domains[0];

  return (
    <>
      <PageTitle
        above={
          <span className="inline-flex items-center gap-2 flex-wrap">
            <StatusPill status={app.status} />
            <RuntimeMark runtime={app.runtime} version={app.runtime_version} />
            <SampleData />
          </span>
        }
        title={app.name}
        action={
          <>
            <Button size="md" variant="secondary">
              <RotateCcw size={14} />
              Roll back
            </Button>
            <Button size="md" variant="primary">
              <Upload size={14} />
              Deploy
            </Button>
          </>
        }
      />

      {primary ? (
        <a
          href={`https://${primary.host}`}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1.5 -mt-2 mb-5 text-[13.5px] text-ink-2 hover:text-ink"
        >
          {primary.host}
          <ExternalLink size={13} className="text-ink-4" />
        </a>
      ) : null}

      <Tabs
        value={tab}
        onChange={setTab}
        className="mb-5"
        tabs={[
          { value: "overview", label: "Overview" },
          { value: "deploys", label: "Deploys" },
          { value: "logs", label: "Logs" },
          { value: "environment", label: "Environment", count: app.env_count },
          { value: "nginx", label: "nginx" },
        ]}
      />

      {tab === "overview" && <Overview app={app} />}
      {tab === "deploys" && <Deploys slug={app.slug} />}
      {tab === "logs" && <LogPanel slug={app.slug} />}
      {tab === "environment" && <EnvironmentPanel />}
      {tab === "nginx" && <NginxPanel slug={app.slug} />}
    </>
  );
}

function Overview({ app }: { app: App }) {
  const mem = pct(app.resources.memory_current_mb, app.resources.memory_max_mb);

  return (
    <div className="grid gap-4 lg:grid-cols-12">
      <div className="lg:col-span-7 grid gap-4">
        <Card>
          <CardHeader title="Build" hint="Prefilled by detection, editable at any time" />
          <CardBody>
            <dl>
              <Row label="Repository">
                <span className="font-mono text-[13px]">{app.repo}</span>
              </Row>
              <Row label="Tracking">
                <span className="font-mono text-[13px]">{app.ref}</span>
              </Row>
              <Row
                label="Install"
                hint={app.runtime === "node" ? "found pnpm-lock.yaml" : undefined}
              >
                <Code>{installFor(app)}</Code>
              </Row>
              <Row label="Build">
                <Code>{buildFor(app)}</Code>
              </Row>
              <Row label={app.runtime === "static" ? "Output directory" : "Start"}>
                <Code>{startFor(app)}</Code>
              </Row>
              <Row
                label="Migrations"
                hint={app.migration_command ? undefined : "no migrations will run"}
              >
                {app.migration_command ? (
                  <Code>{app.migration_command}</Code>
                ) : (
                  <span className="text-ink-4">None</span>
                )}
              </Row>
              <Row
                label="System packages"
                hint="Installed box-wide and shared by every app"
              >
                {app.system_packages.length ? (
                  <span className="flex flex-wrap gap-1 justify-end">
                    {app.system_packages.map((p) => (
                      <Code key={p}>{p}</Code>
                    ))}
                  </span>
                ) : (
                  <span className="text-ink-4">None</span>
                )}
              </Row>
            </dl>
          </CardBody>
        </Card>

        <Card>
          <CardHeader title="Routes" hint="Each named port is reserved and injected as an env var" />
          <CardBody>
            {app.routes.length === 0 ? (
              <p className="text-[13.5px] text-ink-3">
                Static output is served straight from disk. There is no process and no port.
              </p>
            ) : (
              <ul className="divide-y divide-line">
                {app.routes.map((r) => (
                  <li key={r.path} className="flex items-center gap-3 py-2.5">
                    <Code>{r.path}</Code>
                    <span className="text-ink-4">→</span>
                    <span className="font-mono text-[13px] text-ink-2">
                      {r.port_name}={r.port}
                    </span>
                    {r.websocket ? (
                      <Badge tone="accent" className="ml-auto">
                        WebSocket
                      </Badge>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </CardBody>
          {app.routes.some((r) => r.websocket) ? (
            <CardFoot>
              <span>
                <Code>proxy_read_timeout</Code> is raised on WebSocket routes so idle connections
                are not closed after 60 seconds.
              </span>
            </CardFoot>
          ) : null}
        </Card>
      </div>

      <div className="lg:col-span-5 grid gap-4 content-start">
        <Card>
          <CardHeader title="Resources" hint="Real cgroup limits from the systemd unit" />
          <CardBody>
            {app.runtime === "static" ? (
              <p className="text-[13.5px] text-ink-3">
                No unit, no limits. nginx serves the built output directly.
              </p>
            ) : (
              <>
                <div className="flex items-baseline justify-between mb-1.5">
                  <span className="text-[13px] text-ink-3">Memory</span>
                  <span className="font-mono text-[12.5px] text-ink-4 tnum">
                    {app.resources.memory_current_mb} MB of {app.resources.memory_max_mb} MB
                  </span>
                </div>
                <Meter value={mem} tone={mem > 85 ? "fail" : mem > 70 ? "run" : "neutral"} />
                <dl className="mt-4">
                  <Row label="Reclaim starts at">{app.resources.memory_high_mb} MB</Row>
                  <Row label="Hard limit">{app.resources.memory_max_mb} MB</Row>
                  <Row label="Peak since start">{app.resources.memory_peak_mb} MB</Row>
                  <Row label="CPU quota">{app.resources.cpu_quota_pct}%</Row>
                </dl>
              </>
            )}
          </CardBody>
        </Card>

        <Card>
          <CardHeader title="Domains" />
          <CardBody>
            <ul className="divide-y divide-line">
              {app.domains.map((d) => (
                <li key={d.host} className="py-2.5">
                  <div className="flex items-center gap-2">
                    <span className="text-[13.5px] text-ink truncate">{d.host}</span>
                    {d.primary ? <Badge>Primary</Badge> : null}
                  </div>
                  <p className="text-[12px] text-ink-4 mt-0.5">
                    {!d.dns_ok
                      ? "DNS does not point at this server yet"
                      : d.cert_expires_at
                        ? `Certificate renews ${ago(d.cert_expires_at).replace(" ago", "")} from now`
                        : "No certificate issued"}
                  </p>
                </li>
              ))}
            </ul>
          </CardBody>
        </Card>

        <Card>
          <CardHeader title="Data" />
          <CardBody>
            <dl>
              <Row label="Databases">
                {app.linked_databases.length ? (
                  app.linked_databases.map((d) => <Code key={d}>{d}</Code>)
                ) : (
                  <span className="text-ink-4">None linked</span>
                )}
              </Row>
              <Row label="Redis" hint={app.linked_redis ? "own instance, own password" : undefined}>
                {app.linked_redis ? <Code>ferrum-redis-{app.linked_redis}</Code> : <span className="text-ink-4">None</span>}
              </Row>
            </dl>
          </CardBody>
        </Card>
      </div>
    </div>
  );
}

function Deploys({ slug }: { slug: string }) {
  const { data: all = [] } = useDeploys();
  const { shell } = useShell();
  const deploys = all.filter((d) => d.app_slug === slug);
  const running = deploys.find((d) => d.state !== null);

  return (
    <div className="grid gap-4">
      {running ? (
        <Card>
          <CardHeader title="Running now" hint={`${running.commit_sha} — ${running.commit_message}`} />
          <CardBody>
            <DeployLadder deploy={running} />
          </CardBody>
        </Card>
      ) : null}

      <Card>
        <CardHeader title="History" hint="The last 5 releases stay on disk, so a roll back needs no rebuild" />
        {shell === "mobile" ? (
          <div className="px-5 pb-4 divide-y divide-line">
            {deploys.map((d) => (
              <div key={d.id} className="py-3">
                <div className="flex items-center justify-between gap-3">
                  <span className="font-mono text-[13px] text-ink">{d.commit_sha}</span>
                  <Outcome deploy={d} />
                </div>
                <p className="text-[13px] text-ink-2 mt-1">{d.commit_message}</p>
                <p className="text-[12px] text-ink-4 mt-1">
                  {d.author} · {ago(d.started_at)}
                  {d.duration_secs ? ` · ${duration(d.duration_secs)}` : ""}
                </p>
                {d.failure_reason ? (
                  <p className="text-[12.5px] text-fail mt-1.5">{d.failure_reason}</p>
                ) : null}
              </div>
            ))}
          </div>
        ) : (
          <table className="w-full text-left">
            <thead>
              <tr className="border-y border-line text-[12px] text-ink-3">
                <th className="font-medium px-5 py-2">Commit</th>
                <th className="font-medium px-3 py-2">Pipeline</th>
                <th className="font-medium px-3 py-2">Started</th>
                <th className="font-medium px-3 py-2">Took</th>
                <th className="font-medium px-5 py-2 text-right">Outcome</th>
              </tr>
            </thead>
            <tbody>
              {deploys.map((d) => (
                <tr key={d.id} className="border-b border-line last:border-0">
                  <td className="px-5 py-3">
                    <span className="font-mono text-[13px] text-ink">{d.commit_sha}</span>
                    <span className="block text-[12.5px] text-ink-3 truncate max-w-xs">
                      {d.commit_message}
                    </span>
                  </td>
                  <td className="px-3 py-3">
                    {d.steps.length ? <DeployRail deploy={d} /> : <span className="text-ink-4">—</span>}
                  </td>
                  <td className="px-3 py-3 text-[12.5px] text-ink-3">{ago(d.started_at)}</td>
                  <td className="px-3 py-3 font-mono text-[12.5px] text-ink-3 tnum">
                    {d.duration_secs ? duration(d.duration_secs) : "—"}
                  </td>
                  <td className="px-5 py-3 text-right">
                    <Outcome deploy={d} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </div>
  );
}

function Outcome({ deploy }: { deploy: { outcome: string | null } }) {
  if (deploy.outcome === "Live") return <Badge tone="ok">Live</Badge>;
  if (deploy.outcome === "Failed") return <Badge tone="fail">Failed</Badge>;
  if (deploy.outcome === "RolledBack") return <Badge tone="hold">Rolled back</Badge>;
  return <Badge tone="run">Running</Badge>;
}

const installFor = (a: App) =>
  ({ node: "pnpm install --frozen-lockfile", bun: "bun install", dotnet: "dotnet restore", static: "npm ci" })[
    a.runtime
  ];
const buildFor = (a: App) =>
  ({ node: "pnpm build", bun: "bun run build", dotnet: "dotnet publish -c Release -o out", static: "npm run build" })[
    a.runtime
  ];
const startFor = (a: App) =>
  ({ node: "pnpm start", bun: "bun run start", dotnet: "dotnet out/Ledger.dll", static: "dist" })[a.runtime];
