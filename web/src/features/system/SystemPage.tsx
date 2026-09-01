import { useState } from "react";
import { ShieldOff } from "lucide-react";
import { useHost, useMetrics, useSecurity } from "@/lib/api";
import { PageTitle } from "@/components/PageTitle";
import { ChartKey, MetricChart, type Band } from "@/components/MetricChart";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Meter } from "@/components/ui/Meter";
import { Row } from "@/components/ui/Row";
import { Segmented } from "@/components/ui/Segmented";
import { ago, pct, uptime } from "@/lib/utils";
import { sliceRange, useRange } from "@/lib/range";

const BANDS: Record<"cpu" | "memory", Band[]> = {
  cpu: [{ key: "cpu", label: "CPU", varName: "--c-accent", fill: true }],
  memory: [{ key: "memory", label: "Memory", varName: "--c-hold", fill: true }],
};

export function SystemPage() {
  const { data: host } = useHost();
  const { data: metrics } = useMetrics();
  const { data: security } = useSecurity();
  const [band, setBand] = useState<"cpu" | "memory">("cpu");
  const { range } = useRange();

  if (!host || !security) return null;

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
                  <MetricChart {...sliceRange(metrics, range)} bands={BANDS[band]} height={210} />
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
            <CardFoot>
              <span>
                Builds run with a hard memory cap, so a runaway build is stopped as itself rather
                than the kernel picking a victim.
              </span>
            </CardFoot>
          </Card>
        </div>

        <div className="lg:col-span-5">
          <Card className="h-full">
            <CardHeader title="Firewall" hint="Default deny inbound, allow outbound" />
            <CardBody>
              <dl>
                {security.firewall.map((r) => (
                  <Row key={r.port} label={<Code>{r.port}</Code>} hint={r.note}>
                    <Badge tone={r.action === "allow" ? "ok" : "neutral"}>{r.action}</Badge>
                  </Row>
                ))}
              </dl>
              <p className="text-[12.5px] text-ink-4 mt-4 leading-relaxed">
                The app port range and 5432 stay closed by the default deny. That is what makes
                loopback-only PostgreSQL actually hold.
              </p>
            </CardBody>
          </Card>
        </div>

        <div className="lg:col-span-7">
          <Card className="h-full">
            <CardHeader
              title="Banned addresses"
              hint={`${security.bans.length} currently banned by fail2ban`}
              action={<Button size="sm" variant="ghost">Allowlist an IP</Button>}
            />
            <CardBody className="pb-3">
              <ul>
                {security.bans.map((b) => (
                  <li
                    key={b.ip}
                    className="flex items-center gap-x-3 gap-y-1 flex-wrap py-2.5 border-b border-line last:border-0"
                  >
                    <span className="font-mono text-[13px] text-ink">{b.ip}</span>
                    <Badge>{b.jail}</Badge>
                    <Button size="sm" variant="ghost" className="ml-auto order-last sm:order-none">
                      <ShieldOff size={13} />
                      Unban
                    </Button>
                    <span className="text-[12.5px] text-ink-4 basis-full sm:basis-auto sm:order-none">
                      {b.failures} failures · {ago(b.banned_at)}
                    </span>
                  </li>
                ))}
              </ul>
            </CardBody>
          </Card>
        </div>
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
