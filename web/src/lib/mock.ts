/**
 * Sample data, still read by: useHost, useMetrics, useSecurity.
 * Delete this file with the last one.
 */
import type { BannedIp, FirewallRule, HostStatus, MetricSeries } from "@/types/api";

const t = (minsAgo: number) => new Date(Date.now() - minsAgo * 60_000).toISOString();

export const host: HostStatus = {
  hostname: "panel.example.com",
  ferrum_version: "0.1.0",
  build_id: "2026.08.30-a3f9c2d",
  commit_sha: "a3f9c2d4e81b06f5c9a2",
  os: "Ubuntu 24.04 LTS",
  arch: "aarch64",
  uptime_secs: 60 * 60 * 24 * 41 + 60 * 60 * 7,
  cpu_cores: 2,
  cpu_pct: 38,
  memory_used_mb: 2712,
  memory_total_mb: 3934,
  swap_used_mb: 208,
  swap_total_mb: 2048,
  disk_used_gb: 21.4,
  disk_total_gb: 76,
  services: [
    { name: "nginx", ok: true, detail: "active, reloaded 3 hours ago" },
    { name: "PostgreSQL", ok: true, detail: "17.4, 3 databases" },
    { name: "Redis", ok: true, detail: "2 instances" },
    { name: "fail2ban", ok: true, detail: "4 jails, 12 banned" },
    { name: "ufw", ok: true, detail: "deny incoming, 3 rules" },
    { name: "Certificates", ok: false, detail: "ledger.example.com renews in 12 days" },
  ],
};

export const firewall: FirewallRule[] = [
  { port: "22", action: "allow", from: "anywhere", note: "SSH, read from sshd_config at setup" },
  { port: "80", action: "allow", from: "anywhere", note: "HTTP, redirects to 443" },
  { port: "443", action: "allow", from: "anywhere", note: "HTTPS" },
];

export const bans: BannedIp[] = [
  { ip: "45.148.10.87", jail: "sshd", banned_at: t(22), failures: 34 },
  { ip: "185.220.101.4", jail: "nginx-botsearch", banned_at: t(96), failures: 12 },
  { ip: "104.244.76.13", jail: "sshd", banned_at: t(310), failures: 51 },
  { ip: "91.240.118.222", jail: "nginx-limit-req", banned_at: t(640), failures: 8 },
];

export function hostMetrics(points = 2016): MetricSeries {
  const now = Math.floor(Date.now() / 1000);
  const step = 300;
  const time: number[] = [];
  const cpu: number[] = [];
  const mem: number[] = [];
  let c = 30;
  let m = 62;
  for (let i = points - 1; i >= 0; i--) {
    time.push(now - i * step);
    c = Math.max(4, Math.min(96, c + Math.sin(i / 7) * 6 + Math.cos(i / 23) * 4));
    m = Math.max(35, Math.min(94, m + Math.sin(i / 31) * 1.6));
    cpu.push(Math.round(c));
    mem.push(Math.round(m));
  }
  return { t: time, values: { cpu, memory: mem } };
}
