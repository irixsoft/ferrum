import type {
  ApiToken,
  App,
  BannedIp,
  Database,
  Deploy,
  DeployStep,
  FirewallRule,
  GithubConnection,
  HostStatus,
  MetricSeries,
  RedisInstance,
  Session,
  User,
} from "@/types/api";

const t = (minsAgo: number) => new Date(Date.now() - minsAgo * 60_000).toISOString();
const day = (daysAhead: number) => new Date(Date.now() + daysAhead * 86_400_000).toISOString();

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

export const apps: App[] = [
  {
    slug: "atlas",
    name: "atlas",
    repo: "irixsoft/atlas",
    ref: "main",
    runtime: "node",
    runtime_version: "22.11.0",
    status: "live",
    domains: [
      { host: "atlas.example.com", primary: true, dns_ok: true, cert_expires_at: day(61) },
      { host: "www.atlas.example.com", primary: false, dns_ok: true, cert_expires_at: day(61) },
    ],
    routes: [{ path: "/", port_name: "PORT", port: 41201, websocket: false }],
    resources: {
      memory_high_mb: 512,
      memory_max_mb: 768,
      memory_current_mb: 411,
      memory_peak_mb: 604,
      cpu_quota_pct: 80,
      cpu_current_pct: 12,
    },
    migration_command: "pnpm run db:migrate",
    pause_traffic_during_migration: true,
    system_packages: ["libvips"],
    linked_databases: ["atlas_prod"],
    linked_redis: "atlas",
    env_count: 18,
    last_deploy: {
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
  },
  {
    slug: "hooks",
    name: "hooks",
    repo: "irixsoft/hooks",
    ref: "main",
    runtime: "bun",
    runtime_version: "1.2.3",
    status: "live",
    domains: [{ host: "hooks.example.com", primary: true, dns_ok: true, cert_expires_at: day(74) }],
    routes: [
      { path: "/", port_name: "PORT", port: 41202, websocket: false },
      { path: "/socket", port_name: "WS_PORT", port: 41203, websocket: true },
    ],
    resources: {
      memory_high_mb: 256,
      memory_max_mb: 384,
      memory_current_mb: 92,
      memory_peak_mb: 141,
      cpu_quota_pct: 50,
      cpu_current_pct: 3,
    },
    migration_command: "",
    pause_traffic_during_migration: true,
    system_packages: [],
    linked_databases: [],
    linked_redis: "hooks",
    env_count: 7,
    last_deploy: {
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
  },
  {
    slug: "ledger",
    name: "ledger",
    repo: "irixsoft/ledger",
    ref: "main",
    runtime: "dotnet",
    runtime_version: "9.0.104",
    status: "building",
    domains: [{ host: "ledger.example.com", primary: true, dns_ok: true, cert_expires_at: day(12) }],
    routes: [{ path: "/", port_name: "PORT", port: 41204, websocket: false }],
    resources: {
      memory_high_mb: 640,
      memory_max_mb: 900,
      memory_current_mb: 508,
      memory_peak_mb: 812,
      cpu_quota_pct: 100,
      cpu_current_pct: 64,
    },
    migration_command: "dotnet ef database update",
    pause_traffic_during_migration: true,
    system_packages: ["libgdiplus", "poppler-utils", "fonts-liberation"],
    linked_databases: ["ledger_prod"],
    linked_redis: null,
    env_count: 24,
    last_deploy: runningDeploy,
  },
  {
    slug: "docs",
    name: "docs",
    repo: "irixsoft/handbook",
    ref: "v2.4.0",
    runtime: "static",
    runtime_version: "—",
    status: "live",
    domains: [{ host: "docs.example.com", primary: true, dns_ok: true, cert_expires_at: day(58) }],
    routes: [],
    resources: {
      memory_high_mb: 0,
      memory_max_mb: 0,
      memory_current_mb: 0,
      memory_peak_mb: 0,
      cpu_quota_pct: 0,
      cpu_current_pct: 0,
    },
    migration_command: "",
    pause_traffic_during_migration: false,
    system_packages: [],
    linked_databases: [],
    linked_redis: null,
    env_count: 2,
    last_deploy: {
      id: "dpl_8c02",
      app_slug: "docs",
      state: null,
      outcome: "Live",
      commit_sha: "e5510aa",
      commit_message: "Release 2.4.0",
      author: "saeed",
      ref: "v2.4.0",
      started_at: t(4310),
      duration_secs: 38,
      steps: [],
    },
  },
  {
    slug: "pulse",
    name: "pulse",
    repo: "irixsoft/pulse",
    ref: "main",
    runtime: "node",
    runtime_version: "20.18.1",
    status: "failed",
    domains: [{ host: "pulse.example.com", primary: true, dns_ok: false, cert_expires_at: null }],
    routes: [{ path: "/", port_name: "PORT", port: 41205, websocket: false }],
    resources: {
      memory_high_mb: 384,
      memory_max_mb: 512,
      memory_current_mb: 0,
      memory_peak_mb: 498,
      cpu_quota_pct: 60,
      cpu_current_pct: 0,
    },
    migration_command: "npx prisma migrate deploy",
    pause_traffic_during_migration: true,
    system_packages: ["ffmpeg"],
    linked_databases: ["pulse_prod"],
    linked_redis: null,
    env_count: 11,
    last_deploy: {
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
  },
];

export const deployHistory: Deploy[] = [
  runningDeploy,
  ...apps
    .filter((a) => a.slug !== "ledger")
    .map((a) => a.last_deploy!)
    .filter(Boolean),
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

export const users: User[] = [
  {
    id: "usr_1",
    name: "Saeed",
    email: "saeed@irixsoft.com",
    enrolled: true,
    passkeys: [
      { id: "pk_1", label: "MacBook Pro — Touch ID", added_at: t(60 * 24 * 96), last_used_at: t(12) },
      { id: "pk_2", label: "iPhone", added_at: t(60 * 24 * 90), last_used_at: t(60 * 30) },
    ],
  },
  {
    id: "usr_2",
    name: "Safiya",
    email: "safiya@irixsoft.com",
    enrolled: true,
    passkeys: [{ id: "pk_3", label: "1Password", added_at: t(60 * 24 * 40), last_used_at: t(60 * 26) }],
  },
  { id: "usr_3", name: "Bilal", email: "bilal@irixsoft.com", enrolled: false, passkeys: [] },
];

export const tokens: ApiToken[] = [
  {
    id: "tok_1",
    name: "Claude Code",
    prefix: "ferr_7Kq2",
    read_only: false,
    created_at: t(60 * 24 * 30),
    last_used_at: t(41),
  },
  {
    id: "tok_2",
    name: "Status page",
    prefix: "ferr_p04X",
    read_only: true,
    created_at: t(60 * 24 * 12),
    last_used_at: t(2),
  },
];

export const sessions: Session[] = [
  { id: "s1", device: "Chrome on macOS", ip: "203.0.113.44", started_at: t(12), current: true },
  { id: "s2", device: "Safari on iOS", ip: "203.0.113.90", started_at: t(60 * 26), current: false },
];

export const github: GithubConnection = {
  connected: true,
  app_name: "ferrum-panel-example",
  account: "irixsoft",
  repos_accessible: 9,
  installed_at: t(60 * 24 * 96),
};

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
