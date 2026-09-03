import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ExternalLink } from "lucide-react";
import {
  ApiError,
  useCancelDeploy,
  useDeleteApp,
  useDeploys,
  useMetrics,
  useReleases,
  useRestartApp,
  useRestoreSnapshot,
  useRetryCertificate,
  useTriggerDeploy,
  useUpdateApp,
} from "@/lib/api";
import { useApp } from "@/lib/api";
import { ChartKey, MetricChart, type Band } from "@/components/MetricChart";
import { Meter } from "@/components/ui/Meter";
import { useShell } from "@/shells/useShell";
import { PageTitle } from "@/components/PageTitle";
import { RuntimeMark, runtimeLabel } from "@/components/RuntimeMark";
import { NEVER_LIVE, StatusPill } from "@/components/StatusPill";
import { DeployLadder, DeployRail } from "@/components/DeployLadder";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Row } from "@/components/ui/Row";
import { Tabs } from "@/components/ui/Tabs";
import { Sheet } from "@/components/ui/Sheet";
import { Segmented } from "@/components/ui/Segmented";
import { ConfigForm, draftFromApp, toChanges, type Draft } from "./ConfigForm";
import { DataCard } from "./DataCard";
import { DeployLog } from "./DeployLog";
import { EnvironmentPanel } from "./EnvironmentPanel";
import { NginxPanel } from "./NginxPanel";
import { LogPanel } from "./LogPanel";
import { RollbackDialog } from "./RollbackDialog";
import { ago, bytes, daysUntil, duration } from "@/lib/utils";
import type { AppDetail, CertStatus, Deploy, Release } from "@/types/api";

type Tab = "overview" | "configuration" | "environment" | "deploys" | "logs" | "nginx";

const message = (e: unknown) => (e instanceof ApiError ? e.message : e ? String(e) : null);
const short = (sha: string | null) => (sha ? sha.slice(0, 7) : "");
const MB = 1024 * 1024;

const MEMORY_BAND: Band[] = [{ key: "memory", label: "Memory (MB)", varName: "--c-hold", fill: true }];
const CPU_BAND: Band[] = [{ key: "cpu", label: "CPU (% of a core)", varName: "--c-accent", fill: true }];

export function AppDetailPage({ slug }: { slug: string }) {
  const { data: app, isLoading } = useApp(slug);
  const { data: deploys = [] } = useDeploys(slug);
  const trigger = useTriggerDeploy(slug);
  const restart = useRestartApp(slug);
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
  const active = deploys.find((d) => d.state !== null);
  const deployLabel = active
    ? active.state === "Queued" && active.queue_position
      ? `Queued, ${active.queue_position} ahead`
      : "Deploying…"
    : "Deploy";

  return (
    <>
      <PageTitle
        above={
          <span className="inline-flex items-center gap-2 flex-wrap">
            <StatusPill status={app.status} neverLive={app.never_live} />
            <RuntimeMark runtime={app.runtime} version={app.runtime_version} />
          </span>
        }
        title={app.name}
        action={
          <div className="flex items-center gap-2">
            {trigger.error || restart.error ? (
              <span className="text-[12.5px] text-fail">{message(trigger.error ?? restart.error)}</span>
            ) : restart.isSuccess && !restart.isPending ? (
              <span className="text-[12.5px] text-ok">Restarted.</span>
            ) : null}
            {app.runtime !== "static" ? (
              <Button
                size="md"
                variant="ghost"
                disabled={active !== undefined || app.never_live || restart.isPending}
                title={app.never_live ? NEVER_LIVE : "systemctl restart the unit"}
                onClick={() => restart.mutate(undefined)}
              >
                Restart
              </Button>
            ) : null}
            <Button
              size="md"
              variant="primary"
              disabled={active !== undefined || trigger.isPending}
              title={app.tracking === "releases" ? "Deploys the latest release" : `Deploys the tip of ${app.git_ref}`}
              onClick={() => {
                trigger.mutate(undefined);
                setTab("deploys");
              }}
            >
              {deployLabel}
            </Button>
          </div>
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
          { value: "deploys", label: "Deploys", count: deploys.length || undefined },
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
      {tab === "deploys" && <Deploys app={app} deploys={deploys} />}
      {tab === "logs" && <LogPanel slug={app.slug} hasProcess={app.runtime !== "static"} />}
      {tab === "nginx" && <NginxPanel slug={app.slug} />}
    </>
  );
}

function Overview({ app }: { app: AppDetail }) {
  const isStatic = app.runtime === "static";
  const retry = useRetryCertificate(app.slug);
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
          <CardHeader
            title="Host"
            hint={app.current_release ? "Serving from the current release" : "Provisioned, waiting for a first release"}
          />
          <CardBody>
            <dl>
              <Row label="Current release" hint={app.current_release ? `built ${ago(app.current_release.built_at)}` : undefined}>
                {app.current_release ? (
                  <span className="font-mono text-[13px]">
                    {short(app.current_release.commit_sha)} <span className="text-ink-4">on {app.current_release.git_ref}</span>
                  </span>
                ) : (
                  <span className="text-ink-4">None yet</span>
                )}
              </Row>
              <Row label="System user">
                <Code>ferrum-{app.slug}</Code>
              </Row>
              <Row label="Directory">
                <Code>/var/lib/ferrum/apps/{app.slug}</Code>
              </Row>
              {isStatic ? null : (
                <Row label="Unit" hint={app.current_release ? undefined : "inactive until the first deploy"}>
                  <Code>ferrum-app-{app.slug}.service</Code>
                </Row>
              )}
              <Row label="nginx">
                <Code>ferrum-{app.slug}.conf</Code>
              </Row>
            </dl>
          </CardBody>
        </Card>

        {isStatic ? null : <Resources app={app} />}

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
          <CardHeader
            title="Domains"
            hint="Certificates are issued once DNS points here"
            action={
              app.certificates.some((c) => c.status.kind !== "issued") ? (
                <Button size="sm" variant="ghost" disabled={retry.isPending} onClick={() => retry.mutate(undefined)}>
                  Retry now
                </Button>
              ) : null
            }
          />
          <CardBody>
            {app.domains.length === 0 ? (
              <p className="text-[13.5px] text-ink-3">No domain yet. Add one under Configuration.</p>
            ) : (
              <ul className="divide-y divide-line">
                {app.certificates.map((c, i) => (
                  <li key={c.domain} className="py-2.5">
                    <div className="flex items-center gap-2">
                      <span className="text-[13.5px] text-ink truncate">{c.domain}</span>
                      {i === 0 ? <Badge>Primary</Badge> : <Badge>Redirects</Badge>}
                      <Certificate status={c.status} />
                    </div>
                    {c.status.kind === "waiting_for_dns" || c.status.kind === "failed" ? (
                      <p className="text-[12px] text-ink-4 mt-1">{c.status.detail}</p>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
            {retry.error ? <p className="text-[12.5px] text-fail mt-2">{message(retry.error)}</p> : null}
          </CardBody>
        </Card>
      </div>
    </div>
  );
}

/** Memory and CPU straight from the unit's cgroup, with the last hour sampled every 10 seconds. */
function Resources({ app }: { app: AppDetail }) {
  const { data: series } = useMetrics(app.slug, "1h");
  const [band, setBand] = useState<"memory" | "cpu">("memory");
  const limit = app.memory_mb * MB;
  const running = app.memory_bytes !== null;
  const used = app.memory_bytes ?? 0;
  const peak = app.memory_peak_bytes ?? 0;
  const share = limit ? Math.round((used / limit) * 100) : 0;

  return (
    <Card>
      <CardHeader
        title="Resources"
        hint={running ? "From the unit's cgroup, exact rather than sampled" : "No process is running"}
        action={
          <Segmented
            value={band}
            onChange={setBand}
            options={[
              { value: "memory", label: "Memory" },
              { value: "cpu", label: "CPU" },
            ]}
          />
        }
      />
      <CardBody className="grid gap-4">
        <div>
          <div className="flex items-baseline justify-between mb-1.5">
            <span className="text-[13px] text-ink-3">Memory</span>
            <span className="font-mono text-[12.5px] text-ink-4 tnum">
              {running ? `${bytes(used)} now · ${bytes(peak)} peak · ${app.memory_mb} MB limit` : `${app.memory_mb} MB limit`}
            </span>
          </div>
          <Meter value={share} tone={share > 90 ? "fail" : share > 75 ? "run" : "accent"} />
        </div>
        <dl>
          <Row label="CPU" hint="of one core, over the last 10 seconds">
            {running && app.cpu_pct !== null ? `${app.cpu_pct.toFixed(1)}%` : <span className="text-ink-4">—</span>}
          </Row>
        </dl>
        {series && series.t.length > 1 ? (
          <>
            <MetricChart {...series} bands={band === "memory" ? MEMORY_BAND : CPU_BAND} height={150} unit={band === "memory" ? " MB" : "%"} />
            <ChartKey bands={band === "memory" ? MEMORY_BAND : CPU_BAND} />
          </>
        ) : (
          <p className="text-[12.5px] text-ink-4">The chart fills in after a minute of running.</p>
        )}
      </CardBody>
    </Card>
  );
}

function Certificate({ status }: { status: CertStatus }) {
  switch (status.kind) {
    case "issued": {
      const days = daysUntil(status.not_after);
      return (
        <Badge tone={days < 30 ? "hold" : "ok"} className="ml-auto">
          TLS · {days}d
        </Badge>
      );
    }
    case "waiting_for_dns":
      return (
        <Badge tone="hold" className="ml-auto">
          Waiting for DNS
        </Badge>
      );
    case "failed":
      return (
        <Badge tone="fail" className="ml-auto">
          Retry {ago(status.retry_at).replace(" ago", "")}
        </Badge>
      );
    default:
      return (
        <Badge className="ml-auto">
          No certificate
        </Badge>
      );
  }
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

function Deploys({ app, deploys }: { app: AppDetail; deploys: Deploy[] }) {
  const { shell } = useShell();
  const { data: releases = [] } = useReleases(app.slug);
  const cancel = useCancelDeploy();
  const restore = useRestoreSnapshot();
  const [logOf, setLogOf] = useState<Deploy | null>(null);
  const [rollbackTo, setRollbackTo] = useState<Release | null>(null);
  const running = deploys.find((d) => d.state !== null);
  const history = deploys.filter((d) => d.state === null);
  const onDisk = (d: Deploy) => (d.release_id ? releases.find((r) => r.id === d.release_id) ?? null : null);
  const snapshotOf = (release: Release) => {
    const index = deploys.findIndex((d) => d.release_id === release.id);
    const replacedBy = deploys.slice(0, index).reverse().find((d) => d.snapshots.length > 0 && d.outcome === "Live");
    return replacedBy ?? null;
  };

  return (
    <div className="grid gap-4">
      {running ? (
        <Card>
          <CardHeader
            title={running.state === "Queued" ? "Queued" : "Running now"}
            hint={`${short(running.commit_sha) || running.git_ref}${running.commit_message ? ` — ${running.commit_message}` : ""}`}
            action={
              <>
                <Button size="sm" variant="ghost" onClick={() => setLogOf(running)}>
                  Log
                </Button>
                {running.state === "Queued" ? (
                  <Button size="sm" variant="danger" disabled={cancel.isPending} onClick={() => cancel.mutate(running.id)}>
                    Cancel
                  </Button>
                ) : null}
              </>
            }
          />
          <CardBody>
            {running.state === "Queued" && running.queue_position ? (
              <p className="text-[13px] text-ink-3 mb-3">
                {running.queue_position} deploy{running.queue_position > 1 ? "s" : ""} ahead. Builds run one at a time so live apps keep their memory.
              </p>
            ) : null}
            <DeployLadder deploy={running} />
          </CardBody>
          {app.commands.migrate && app.pause_for_migrations ? (
            <CardFoot>
              <span>Traffic is paused while migrations run. It resumes once the health check passes.</span>
            </CardFoot>
          ) : null}
        </Card>
      ) : null}

      <Card>
        <CardHeader
          title="History"
          hint="The last 5 releases stay on disk, so a roll back needs no rebuild"
        />
        {history.length === 0 ? (
          <CardBody>
            <p className="text-[13.5px] text-ink-3">
              {running ? "The first deploy is running." : "Never deployed."}
            </p>
          </CardBody>
        ) : shell === "mobile" ? (
          <div className="px-5 pb-4 divide-y divide-line">
            {history.map((d) => (
              <div key={d.id} className="py-3">
                <div className="flex items-center justify-between gap-3">
                  <span className="font-mono text-[13px] text-ink">{short(d.commit_sha) || d.git_ref}</span>
                  <Outcome deploy={d} />
                </div>
                <p className="text-[13px] text-ink-2 mt-1">{d.commit_message ?? d.git_ref}</p>
                <p className="text-[12px] text-ink-4 mt-1">
                  {d.author ?? d.trigger} · {ago(d.started_at)}
                  {d.duration_secs ? ` · ${duration(d.duration_secs)}` : ""}
                </p>
                {d.failure_reason ? (
                  <p className="text-[12.5px] text-fail mt-1.5">{d.failure_reason}</p>
                ) : null}
                <Actions deploy={d} release={onDisk(d)} onLog={() => setLogOf(d)} onRollback={setRollbackTo} onRestore={(id) => restore.mutate(id)} />
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
                <th className="font-medium px-3 py-2">Outcome</th>
                <th className="font-medium px-5 py-2 text-right"></th>
              </tr>
            </thead>
            <tbody>
              {history.map((d) => (
                <tr key={d.id} className="border-b border-line last:border-0 align-top">
                  <td className="px-5 py-3">
                    <span className="font-mono text-[13px] text-ink">{short(d.commit_sha) || d.git_ref}</span>
                    <span className="block text-[12.5px] text-ink-3 truncate max-w-xs">
                      {d.commit_message ?? d.git_ref}
                      {d.author ? <span className="text-ink-4"> · {d.author}</span> : null}
                    </span>
                    {d.failure_reason ? (
                      <span className="block text-[12.5px] text-fail mt-1 max-w-sm">{d.failure_reason}</span>
                    ) : null}
                  </td>
                  <td className="px-3 py-3">
                    <DeployRail deploy={d} />
                  </td>
                  <td className="px-3 py-3 text-[12.5px] text-ink-3 whitespace-nowrap">{ago(d.started_at)}</td>
                  <td className="px-3 py-3 font-mono text-[12.5px] text-ink-3 tnum">
                    {d.duration_secs !== null ? duration(d.duration_secs) : "—"}
                  </td>
                  <td className="px-3 py-3">
                    <Outcome deploy={d} />
                  </td>
                  <td className="px-5 py-3 text-right whitespace-nowrap">
                    <Actions deploy={d} release={onDisk(d)} onLog={() => setLogOf(d)} onRollback={setRollbackTo} onRestore={(id) => restore.mutate(id)} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {restore.error || cancel.error ? (
          <CardFoot>
            <span className="text-fail">{message(restore.error ?? cancel.error)}</span>
          </CardFoot>
        ) : restore.isSuccess ? (
          <CardFoot>
            <span className="text-ok">Snapshot restored.</span>
          </CardFoot>
        ) : null}
      </Card>

      <Sheet
        open={logOf !== null}
        onClose={() => setLogOf(null)}
        title={logOf ? `Log of ${short(logOf.commit_sha) || logOf.git_ref}` : ""}
      >
        {logOf ? <DeployLog id={logOf.id} /> : null}
      </Sheet>

      <RollbackDialog
        slug={app.slug}
        release={rollbackTo}
        snapshotOf={rollbackTo ? snapshotOf(rollbackTo) : null}
        onClose={() => setRollbackTo(null)}
      />
    </div>
  );
}

function Actions({
  deploy,
  release,
  onLog,
  onRollback,
  onRestore,
}: {
  deploy: Deploy;
  release: Release | null;
  onLog: () => void;
  onRollback: (release: Release) => void;
  onRestore: (snapshotId: string) => void;
}) {
  const failedMigration = deploy.outcome === "Failed" && deploy.snapshots.length > 0;
  return (
    <span className="inline-flex items-center gap-1 flex-wrap justify-end mt-1 sm:mt-0">
      <Button size="sm" variant="ghost" onClick={onLog}>
        Log
      </Button>
      {release && !release.current ? (
        <Button size="sm" variant="ghost" onClick={() => onRollback(release)}>
          Roll back
        </Button>
      ) : null}
      {failedMigration ? (
        <Button
          size="sm"
          variant="danger"
          title={`Restores ${deploy.snapshots[0].database} to ${new Date(deploy.snapshots[0].taken_at).toLocaleString()}`}
          onClick={() => onRestore(deploy.snapshots[0].id)}
        >
          Restore snapshot
        </Button>
      ) : null}
    </span>
  );
}

function Outcome({ deploy }: { deploy: { outcome: string | null } }) {
  if (deploy.outcome === "Live") return <Badge tone="ok">Live</Badge>;
  if (deploy.outcome === "Failed") return <Badge tone="fail">Failed</Badge>;
  if (deploy.outcome === "RolledBack") return <Badge tone="hold">Rolled back</Badge>;
  return <Badge tone="run">Running</Badge>;
}
