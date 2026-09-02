/**
 * Sample data, still read by: useHost, useDeploys, useRunningDeploy, useDatabases, useMetrics,
 * useSecurity. Delete this file with the last one.
 */
import type {
  BannedIp,
  Database,
  Deploy,
  DeployStep,
  FirewallRule,
  HostStatus,
  MetricSeries,
  RedisInstance,
} from "@/types/api";

const t = (minsAgo: number) => new Date(Date.now() - minsAgo * 60_000).toISOString();

function steps(
  spec: Array<[DeployStep["state"], DeployStep["status"], number | null, string?]>,
): DeployStep[] {
  return spec.map(([state, status, elapsed_secs, note]) => ({
    state,
    status,
    elapsed_secs,
    note,
  }));
}

export const runningDeploy: Deploy = {
  id: "dpl_8f21",
  app_slug: "ledger",
  state: "Migrating",
  outcome: null,
  commit_sha: "a3f9c2d",
  commit_message: "Add reconciliation window to statement export",
  author: "saeed",
  ref: "main",
  started_at: t(3),
  duration_secs: null,
  steps: steps([
    ["Queued", "done", 2],
    ["Cloning", "done", 4],
    ["InstallingSystemPackages", "skipped", null, "all 3 packages already present"],
    ["InstallingDeps", "done", 31],
    ["Building", "done", 96],
    ["Snapshotting", "done", 8],
    ["MaintenanceOn", "done", 1],
    ["Migrating", "active", null],
    ["Swapping", "pending", null],
    ["Restarting", "pending", null],
    ["HealthChecking", "pending", null],
    ["MaintenanceOff", "pending", null],
  ]),
};

export const deployHistory: Deploy[] = [
  runningDeploy,
  {
    id: "dpl_8e04",
    app_slug: "atlas",
    state: null,
    outcome: "Live",
    commit_sha: "7c1b409",
    commit_message: "Cache tile manifests between requests",
    author: "saeed",
    ref: "main",
    started_at: t(184),
    duration_secs: 213,
    steps: [],
  },
  {
    id: "dpl_8d77",
    app_slug: "hooks",
    state: null,
    outcome: "Live",
    commit_sha: "b420f18",
    commit_message: "Drop the reconnect backoff to 2s",
    author: "safiya",
    ref: "main",
    started_at: t(1490),
    duration_secs: 64,
    steps: [],
  },
  {
    id: "dpl_8b91",
    app_slug: "pulse",
    state: null,
    outcome: "Failed",
    commit_sha: "1d0a7e6",
    commit_message: "Switch thumbnailer to sharp",
    author: "saeed",
    ref: "main",
    started_at: t(52),
    duration_secs: 141,
    failure_reason: "Build exceeded 512 MB and was stopped. Raise the build limit or reduce peak memory.",
    steps: [],
  },
  {
    id: "dpl_8a30",
    app_slug: "atlas",
    state: null,
    outcome: "RolledBack",
    commit_sha: "99fe201",
    commit_message: "Move tile cache to Redis",
    author: "safiya",
    ref: "main",
    started_at: t(2880),
    duration_secs: 268,
    failure_reason: "Health check did not pass within 60s. Rolled back to 7c1b409.",
    steps: [],
  },
];

export const databases: Database[] = [
  {
    name: "atlas_prod",
    role: "atlas",
    size_bytes: 1_842_000_000,
    connection_limit: 40,
    connections_active: 11,
    extensions: ["pgvector", "pg_trgm", "uuid-ossp"],
    linked_apps: ["atlas"],
    created_at: t(60 * 24 * 96),
  },
  {
    name: "ledger_prod",
    role: "ledger",
    size_bytes: 604_000_000,
    connection_limit: 30,
    connections_active: 6,
    extensions: ["pgcrypto", "uuid-ossp"],
    linked_apps: ["ledger"],
    created_at: t(60 * 24 * 54),
  },
  {
    name: "pulse_prod",
    role: "pulse",
    size_bytes: 92_400_000,
    connection_limit: 20,
    connections_active: 0,
    extensions: ["uuid-ossp"],
    linked_apps: ["pulse"],
    created_at: t(60 * 24 * 12),
  },
];

export const redisInstances: RedisInstance[] = [
  {
    slug: "atlas",
    app_slug: "atlas",
    port: 46379,
    maxmemory_mb: 128,
    used_memory_mb: 41,
    maxmemory_policy: "noeviction",
    appendonly: true,
  },
  {
    slug: "hooks",
    app_slug: "hooks",
    port: 46380,
    maxmemory_mb: 64,
    used_memory_mb: 9,
    maxmemory_policy: "noeviction",
    appendonly: true,
  },
];

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
