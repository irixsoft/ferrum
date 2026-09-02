import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ExternalLink } from "lucide-react";
import { ApiError, useDeleteApp, useDeploys, useUpdateApp } from "@/lib/api";
import { useApp } from "@/lib/api";
import { useShell } from "@/shells/useShell";
import { PageTitle } from "@/components/PageTitle";
import { SampleData } from "@/components/SampleData";
import { RuntimeMark, runtimeLabel } from "@/components/RuntimeMark";
import { StatusPill } from "@/components/StatusPill";
import { DeployLadder, DeployRail } from "@/components/DeployLadder";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Row } from "@/components/ui/Row";
import { Tabs } from "@/components/ui/Tabs";
import { ConfigForm, draftFromApp, toChanges, type Draft } from "./ConfigForm";
import { DataCard } from "./DataCard";
import { EnvironmentPanel } from "./EnvironmentPanel";
import { NginxPanel } from "./NginxPanel";
import { LogPanel } from "./LogPanel";
import { ago, duration } from "@/lib/utils";
import type { AppDetail } from "@/types/api";

type Tab = "overview" | "configuration" | "environment" | "deploys" | "logs" | "nginx";

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

  const primary = app.domains[0];

  return (
    <>
      <PageTitle
        above={
          <span className="inline-flex items-center gap-2 flex-wrap">
            <StatusPill status="new" />
            <RuntimeMark runtime={app.runtime} version={app.runtime_version} />
          </span>
        }
        title={app.name}
        action={
          <Button size="md" variant="primary" disabled title="Deploys arrive with the pipeline">
            Deploy
          </Button>
        }
      />

      {primary ? (
        <a
          href={`https://${primary}`}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1.5 -mt-2 mb-5 text-[13.5px] text-ink-2 hover:text-ink"
        >
          {primary}
          <ExternalLink size={13} className="text-ink-4" />
        </a>
      ) : null}

      <Tabs
        value={tab}
        onChange={setTab}
        className="mb-5"
        tabs={[
          { value: "overview", label: "Overview" },
          { value: "configuration", label: "Configuration" },
          { value: "environment", label: "Environment", count: app.env.length },
          { value: "deploys", label: "Deploys" },
          { value: "logs", label: "Logs" },
          { value: "nginx", label: "nginx" },
        ]}
      />

      {tab === "overview" && <Overview app={app} />}
      {tab === "configuration" && <Configuration key={app.updated_at} app={app} />}
      {tab === "environment" && (
        <EnvironmentPanel
          key={[...app.env.map((e) => e.key), ...app.managed].join(",")}
          slug={app.slug}
          keys={app.env.map((e) => e.key)}
          managed={app.managed}
        />
      )}
      {tab === "deploys" && <Deploys slug={app.slug} />}
      {tab === "logs" && <LogPanel slug={app.slug} />}
      {tab === "nginx" && <NginxPanel slug={app.slug} />}
    </>
  );
}

function Overview({ app }: { app: AppDetail }) {
  const isStatic = app.runtime === "static";
  return (
    <div className="grid gap-4 lg:grid-cols-12">
      <div className="lg:col-span-7 grid gap-4">
        <Card>
          <CardHeader title="Build" hint="Prefilled by detection, editable under Configuration" />
          <CardBody>
            <dl>
              <Row label="Repository">
                <span className="font-mono text-[13px]">{app.repository}</span>
              </Row>
              <Row label={app.tracking === "releases" ? "Releases from" : "Every push to"}>
                <span className="font-mono text-[13px]">{app.git_ref}</span>
              </Row>
              {app.root ? (
                <Row label="Root directory">
                  <span className="font-mono text-[13px]">{app.root}</span>
                </Row>
              ) : null}
              {isStatic ? (
                <Row label="Built with">{runtimeLabel(app.toolchain)} {app.runtime_version}</Row>
              ) : null}
              <Row label="Install">
                <Command value={app.commands.install} />
              </Row>
              <Row label="Build">
                <Command value={app.commands.build} />
              </Row>
              <Row label={isStatic ? "Output directory" : "Start"}>
                <Command value={isStatic ? app.output_dir : app.commands.start} />
              </Row>
              <Row label="Migrations" hint={app.commands.migrate ? undefined : "no migrations will run"}>
                <Command value={app.commands.migrate} />
              </Row>
              <Row label="System packages" hint="Installed box-wide and shared by every app">
                {app.packages.length ? (
                  <span className="flex flex-wrap gap-1 justify-end">
                    {app.packages.map((p) => (
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
            {isStatic ? (
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
                      {r.port_name === "main" ? "PORT" : `${r.port_name.toUpperCase()}_PORT`}={r.port}
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

        <DataCard app={app} />
      </div>

      <div className="lg:col-span-5 grid gap-4 content-start">
        <Card>
          <CardHeader title="Host" hint="Provisioned, waiting for a first release" />
          <CardBody>
            <dl>
              <Row label="System user">
                <Code>ferrum-{app.slug}</Code>
              </Row>
              <Row label="Directory">
                <Code>/var/lib/ferrum/apps/{app.slug}</Code>
              </Row>
              {isStatic ? null : (
                <Row label="Unit" hint="inactive until the first deploy">
                  <Code>ferrum-app-{app.slug}.service</Code>
                </Row>
              )}
              <Row label="nginx">
                <Code>ferrum-{app.slug}.conf</Code>
              </Row>
            </dl>
          </CardBody>
        </Card>

        {isStatic ? null : (
          <Card>
            <CardHeader title="Limits" hint="Real cgroup limits from the systemd unit" />
            <CardBody>
              <dl>
                <Row label="Memory">{app.memory_mb} MB</Row>
                <Row label="CPU quota">{app.cpu_percent}%</Row>
                <Row label="Health check">
                  <Code>{app.health.path}</Code>
                </Row>
                <Row label="Startup budget">{app.health.startup_budget_secs}s</Row>
                <Row label="Traffic during migrations">
                  {app.pause_for_migrations ? "Paused" : "Kept flowing"}
                </Row>
              </dl>
            </CardBody>
          </Card>
        )}

        <Card>
          <CardHeader title="Domains" />
          <CardBody>
            {app.domains.length === 0 ? (
              <p className="text-[13.5px] text-ink-3">No domain yet. Add one under Configuration.</p>
            ) : (
              <ul className="divide-y divide-line">
                {app.domains.map((d, i) => (
                  <li key={d} className="py-2.5 flex items-center gap-2">
                    <span className="text-[13.5px] text-ink truncate">{d}</span>
                    {i === 0 ? <Badge>Primary</Badge> : <Badge>Redirects</Badge>}
                  </li>
                ))}
              </ul>
            )}
          </CardBody>
        </Card>
      </div>
    </div>
  );
}

function Command({ value }: { value: string | null }) {
  return value ? <Code>{value}</Code> : <span className="text-ink-4">None</span>;
}

function Configuration({ app }: { app: AppDetail }) {
  const navigate = useNavigate();
  const [draft, setDraft] = useState<Draft>(() => draftFromApp(app));
  const [confirm, setConfirm] = useState("");
  const update = useUpdateApp(app.slug);
  const remove = useDeleteApp(app.slug);
  const message = (e: unknown) => (e instanceof ApiError ? e.message : e ? String(e) : null);

  return (
    <div className="grid gap-4">
      <ConfigForm draft={draft} onChange={setDraft} creating={false} />
      <Card>
        <CardBody className="pt-5 flex items-center gap-3 flex-wrap">
          <Button variant="primary" onClick={() => update.mutate(toChanges(draft))} disabled={update.isPending}>
            Save changes
          </Button>
          <span className="text-[12.5px] text-ink-4">
            Rewrites the env file, the unit and the nginx site. A running app is not restarted.
          </span>
          {update.error ? <span className="text-[12.5px] text-fail">{message(update.error)}</span> : null}
          {update.isSuccess && !update.isPending ? (
            <span className="text-[12.5px] text-ok">Saved.</span>
          ) : null}
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="Delete this app" hint="Removes the unit, the nginx site, every release and the system user" />
        <CardBody className="grid gap-3">
          <p className="text-[13px] text-ink-2">
            Linked databases are not deleted. Type <strong className="text-ink">{app.name}</strong> to confirm.
          </p>
          <div className="flex gap-2 flex-wrap">
            <input
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              placeholder={app.name}
              className="h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 w-64"
            />
            <Button
              variant="danger"
              disabled={confirm !== app.name || remove.isPending}
              onClick={async () => {
                await remove.mutateAsync(confirm);
                navigate({ to: "/apps" });
              }}
            >
              Delete
            </Button>
          </div>
          {remove.error ? <p className="text-[12.5px] text-fail">{message(remove.error)}</p> : null}
        </CardBody>
      </Card>
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
          <CardHeader title="Running now" hint={`${running.commit_sha} — ${running.commit_message}`} action={<SampleData />} />
          <CardBody>
            <DeployLadder deploy={running} />
          </CardBody>
        </Card>
      ) : null}

      <Card>
        <CardHeader
          title="History"
          hint="The last 5 releases stay on disk, so a roll back needs no rebuild"
          action={<SampleData />}
        />
        {deploys.length === 0 ? (
          <CardBody>
            <p className="text-[13.5px] text-ink-3">Never deployed.</p>
          </CardBody>
        ) : shell === "mobile" ? (
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
