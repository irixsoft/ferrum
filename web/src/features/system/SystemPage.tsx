import { useState } from "react";
import { KeyRound, ShieldOff } from "lucide-react";
import {
  ApiError,
  useAllowlist,
  useDisablePasswords,
  useEnableFail2ban,
  useEnableFirewall,
  useEnableUpdates,
  useHost,
  useMetrics,
  useSecurity,
  useUnban,
} from "@/lib/api";
import type { JobStatus, Security } from "@/types/api";
import { PageTitle } from "@/components/PageTitle";
import { EnableButton, enableFailure } from "@/components/EnableButton";
import { ChartKey, MetricChart, type Band } from "@/components/MetricChart";
import { Card, CardBody, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Meter } from "@/components/ui/Meter";
import { Row } from "@/components/ui/Row";
import { Segmented } from "@/components/ui/Segmented";
import { Sheet } from "@/components/ui/Sheet";
import { ago, pct, uptime } from "@/lib/utils";
import { useRange } from "@/lib/range";

const BANDS: Record<"cpu" | "memory", Band[]> = {
  cpu: [{ key: "cpu", label: "CPU", varName: "--c-accent", fill: true }],
  memory: [{ key: "memory", label: "Memory", varName: "--c-hold", fill: true }],
};

const message = (e: unknown) => (e instanceof ApiError ? e.message : e ? String(e) : null);
const failure = enableFailure;

export function SystemPage() {
  const { data: host } = useHost();
  const { range } = useRange();
  const { data: metrics } = useMetrics("host", range);
  const { data: security, error: securityError } = useSecurity();
  const [band, setBand] = useState<"cpu" | "memory">("cpu");

  if (!host) return null;

  const mem = pct(host.memory_used_mb, host.memory_total_mb);
  const disk = pct(host.disk_used_gb, host.disk_total_gb);
  const swap = pct(host.swap_used_mb, host.swap_total_mb);

  return (
    <>
      <PageTitle above={`${host.os} · ${host.arch} · up ${uptime(host.uptime_secs)}`} title="System" />

      <div className="grid gap-4 lg:grid-cols-12">
        <div className="lg:col-span-8">
          <Card>
            <CardHeader
              title="Load"
              hint="From /proc, sampled into a ring buffer every 10 seconds"
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
              {metrics ? (
                <>
                  <MetricChart {...metrics} bands={BANDS[band]} height={210} />
                  <div className="mt-3">
                    <ChartKey bands={BANDS[band]} />
                  </div>
                </>
              ) : null}
            </CardBody>
          </Card>
        </div>

        <div className="lg:col-span-4">
          <Card>
            <CardHeader title="Capacity" />
            <CardBody className="space-y-4">
              <Gauge
                label="Memory"
                value={mem}
                detail={`${(host.memory_used_mb / 1024).toFixed(1)} / ${(host.memory_total_mb / 1024).toFixed(1)} GB`}
              />
              <Gauge label="Swap" value={swap} detail={`${host.swap_used_mb} / ${host.swap_total_mb} MB`} />
              <Gauge label="Disk" value={disk} detail={`${host.disk_used_gb} / ${host.disk_total_gb} GB`} />
            </CardBody>
          </Card>
        </div>

        {security ? (
          <>
            <div className="lg:col-span-5">
              <Firewall firewall={security.firewall} job={security.jobs.firewall} />
            </div>

            <div className="lg:col-span-7">
              <Bans bans={security.bans} job={security.jobs.fail2ban} />
            </div>

            <div className="lg:col-span-5">
              <Updates updates={security.updates} job={security.jobs.updates} />
            </div>

            <div className="lg:col-span-7">
              <Ssh ssh={security.ssh} hostname={host.hostname} />
            </div>
          </>
        ) : securityError ? (
          <div className="lg:col-span-12">
            <Card>
              <CardHeader title="Hardening" hint="The host did not answer" />
              <CardBody>
                <p className="text-[12.5px] text-fail">{message(securityError)}</p>
              </CardBody>
            </Card>
          </div>
        ) : null}
      </div>
    </>
  );
}

function Gauge({ label, value, detail }: { label: string; value: number; detail: string }) {
  return (
    <div>
      <div className="flex items-baseline justify-between mb-1.5">
        <span className="text-[13px] text-ink-3">{label}</span>
        <span className="font-mono text-[12.5px] text-ink-4 tnum">{detail}</span>
      </div>
      <Meter value={value} tone={value > 85 ? "fail" : value > 70 ? "run" : "neutral"} />
    </div>
  );
}

function Firewall({ firewall, job }: { firewall: Security["firewall"]; job: JobStatus }) {
  const enable = useEnableFirewall();
  return (
    <Card className="h-full">
      <CardHeader
        title="Firewall"
        hint={firewall.enabled ? "Default deny inbound, allow outbound" : "Not enabled"}
        action={
          firewall.enabled ? (
            <Badge tone="ok">Enabled</Badge>
          ) : (
            <EnableButton label="Enable firewall" job={job} mutation={enable} />
          )
        }
      />
      <CardBody>
        {firewall.enabled ? (
          <dl>
            {firewall.rules.map((r) => (
              <Row key={r.port} label={<Code>{r.port}</Code>} hint={`from ${r.from}`}>
                <Badge tone={r.action === "allow" ? "ok" : "neutral"}>{r.action}</Badge>
              </Row>
            ))}
          </dl>
        ) : (
          <p className="text-[13px] text-ink-2 leading-relaxed">
            SSH on <Code>{firewall.ssh_port}</Code>, 80 and 443 stay open; everything else is denied.
          </p>
        )}
        {firewall.persisted ? (
          <p className="text-[12.5px] text-ink-4 mt-3 leading-relaxed">
            This host keeps its own rules in <Code>/etc/iptables/rules.v4</Code>; SSH, 80 and 443 are
            open there too. Once the firewall is enabled, the image's rule restore is switched off
            so ufw is the only owner at boot.
          </p>
        ) : null}
        {failure(job, enable) ? <p className="text-[12.5px] text-fail mt-3">{failure(job, enable)}</p> : null}
      </CardBody>
    </Card>
  );
}

function Bans({ bans, job }: { bans: Security["bans"]; job: JobStatus }) {
  const enable = useEnableFail2ban();
  const unban = useUnban();
  const allow = useAllowlist();
  const [ip, setIp] = useState("");
  const [adding, setAdding] = useState(false);

  const submit = async () => {
    if (!ip.trim()) return;
    await allow.mutateAsync(ip.trim());
    setIp("");
    setAdding(false);
  };

  return (
    <Card className="h-full">
      <CardHeader
        title="Banned addresses"
        hint={
          bans.installed
            ? `${bans.banned.length} currently banned across ${bans.jails.length} jails`
            : "fail2ban is not enabled"
        }
        action={
          bans.installed ? (
            <Button size="sm" variant="ghost" onClick={() => setAdding(true)}>
              Allowlist an IP
            </Button>
          ) : (
            <EnableButton label="Enable fail2ban" job={job} mutation={enable} />
          )
        }
      />
      <CardBody className="pb-3">
        {adding ? (
          <div className="flex gap-2 mb-3">
            <input
              autoFocus
              value={ip}
              onChange={(e) => setIp(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && submit()}
              placeholder="203.0.113.9"
              className="flex-1 h-9 px-3 bg-inset border border-line-strong rounded-control font-mono text-sm text-ink placeholder:text-ink-4"
            />
            <Button variant="primary" onClick={submit} disabled={allow.isPending}>
              Never ban
            </Button>
            <Button variant="ghost" onClick={() => setAdding(false)}>
              Cancel
            </Button>
          </div>
        ) : null}
        {allow.error ? <p className="text-[12.5px] text-fail mb-3">{message(allow.error)}</p> : null}
        {failure(job, enable) ? <p className="text-[12.5px] text-fail mb-3">{failure(job, enable)}</p> : null}
        {!bans.installed ? (
          <p className="text-[13px] text-ink-2 leading-relaxed">
            Jails for sshd and the three nginx patterns, banning for an hour after five failures in
            ten minutes.
          </p>
        ) : bans.banned.length === 0 ? (
          <p className="text-[13px] text-ink-4">Nothing is banned right now.</p>
        ) : (
          <ul>
            {bans.banned.map((b) => (
              <li
                key={`${b.jail}-${b.ip}`}
                className="flex items-center gap-x-3 gap-y-1 flex-wrap py-2.5 border-b border-line last:border-0"
              >
                <span className="font-mono text-[13px] text-ink">{b.ip}</span>
                <Badge>{b.jail}</Badge>
                <Button
                  size="sm"
                  variant="ghost"
                  className="ml-auto order-last sm:order-none"
                  disabled={unban.isPending}
                  onClick={() => unban.mutate(b.ip)}
                >
                  <ShieldOff size={13} />
                  Unban
                </Button>
                {b.banned_at ? (
                  <span className="text-[12.5px] text-ink-4 basis-full sm:basis-auto sm:order-none">
                    banned {ago(b.banned_at)}
                  </span>
                ) : null}
              </li>
            ))}
          </ul>
        )}
        {bans.allowlist.length ? (
          <p className="text-[12.5px] text-ink-4 mt-3">
            Never banned: {bans.allowlist.map((a) => <Code key={a} className="mr-1">{a}</Code>)}
          </p>
        ) : null}
      </CardBody>
    </Card>
  );
}

function Updates({ updates, job }: { updates: Security["updates"]; job: JobStatus }) {
  const enable = useEnableUpdates();
  return (
    <Card className="h-full">
      <CardHeader
        title="Security updates"
        hint={updates.enabled ? "Applied automatically by unattended-upgrades" : "Not enabled"}
        action={
          updates.enabled ? (
            <Badge tone="ok">Enabled</Badge>
          ) : (
            <EnableButton label="Enable updates" job={job} mutation={enable} />
          )
        }
      />
      <CardBody>
        <p className="text-[13px] text-ink-2 leading-relaxed">
          Ubuntu's security pocket, installed daily without asking.
        </p>
        {failure(job, enable) ? <p className="text-[12.5px] text-fail mt-3">{failure(job, enable)}</p> : null}
      </CardBody>
    </Card>
  );
}

function Ssh({ ssh, hostname }: { ssh: Security["ssh"]; hostname: string }) {
  const disable = useDisablePasswords();
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const hasKeys = ssh.keys.length > 0;

  return (
    <Card className="h-full">
      <CardHeader
        title="SSH"
        hint={`Port ${ssh.port} · password login ${ssh.password_auth ? "allowed" : "disabled"}`}
        action={
          !ssh.password_auth ? (
            <Badge tone="ok">Keys only</Badge>
          ) : hasKeys ? (
            <Button size="sm" variant="primary" onClick={() => setOpen(true)}>
              Disable password login
            </Button>
          ) : null
        }
      />
      <CardBody className="pb-3">
        {hasKeys ? (
          <ul>
            {ssh.keys.map((k) => (
              <li key={k.fingerprint} className="flex items-center gap-2 py-2 border-b border-line last:border-0 min-w-0">
                <KeyRound size={13} className="text-ink-4 shrink-0" />
                <span className="font-mono text-[12px] text-ink-2 truncate">{k.fingerprint}</span>
                <span className="text-[12.5px] text-ink-4 ml-auto shrink-0">
                  {k.comment} · {k.kind} {k.bits}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-[13px] text-ink-2 leading-relaxed">
            No SSH key is installed, so password login stays on: disabling it now would lock you
            out. Add your public key to <Code>/root/.ssh/authorized_keys</Code> and it will appear
            here.{" "}
            <a
              href="https://documentation.ubuntu.com/server/how-to/security/openssh-server/"
              target="_blank"
              rel="noreferrer"
              className="text-accent hover:underline"
            >
              How to add a key
            </a>
          </p>
        )}
        {disable.error && !open ? <p className="text-[12.5px] text-fail mt-3">{message(disable.error)}</p> : null}
      </CardBody>
      <Sheet
        open={open}
        onClose={() => setOpen(false)}
        title="Disable password login"
        side="center"
        footer={
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="danger"
              disabled={typed !== hostname || disable.isPending}
              onClick={async () => {
                await disable.mutateAsync(typed);
                setOpen(false);
                setTyped("");
              }}
            >
              Disable
            </Button>
          </div>
        }
      >
        <p className="text-[13px] text-ink-2 leading-relaxed">
          From now on only the {ssh.keys.length === 1 ? "key" : `${ssh.keys.length} keys`} above can
          sign in. Make sure one of them is on the machine you are using. Type{" "}
          <strong className="text-ink">{hostname}</strong> to confirm.
        </p>
        <input
          autoFocus
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          placeholder={hostname}
          className="mt-3 w-full h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4"
        />
        {disable.error ? <p className="text-[12.5px] text-fail mt-2">{message(disable.error)}</p> : null}
      </Sheet>
    </Card>
  );
}
