export type Runtime = "node" | "bun" | "dotnet" | "static";

export type AppStatus = "live" | "building" | "failed" | "stopped" | "maintenance";

export const DEPLOY_STATES = [
  "Queued",
  "Cloning",
  "InstallingSystemPackages",
  "InstallingDeps",
  "Building",
  "Snapshotting",
  "MaintenanceOn",
  "Migrating",
  "Swapping",
  "Restarting",
  "HealthChecking",
  "MaintenanceOff",
] as const;

export type DeployState = (typeof DEPLOY_STATES)[number];
export type DeployOutcome = "Live" | "Failed" | "RolledBack";

export interface DeployStep {
  state: DeployState;
  elapsed_secs: number | null;
  status: "done" | "active" | "pending" | "skipped" | "failed";
  note?: string;
}

export interface Deploy {
  id: string;
  app_slug: string;
  state: DeployState | null;
  outcome: DeployOutcome | null;
  commit_sha: string;
  commit_message: string;
  author: string;
  ref: string;
  started_at: string;
  duration_secs: number | null;
  steps: DeployStep[];
  failure_reason?: string;
  queue_position?: number;
}

export interface Route {
  path: string;
  port_name: string;
  port: number;
  websocket: boolean;
}

export interface Domain {
  host: string;
  primary: boolean;
  dns_ok: boolean;
  cert_expires_at: string | null;
}

export interface AppResources {
  memory_high_mb: number;
  memory_max_mb: number;
  memory_current_mb: number;
  memory_peak_mb: number;
  cpu_quota_pct: number;
  cpu_current_pct: number;
}

export interface App {
  slug: string;
  name: string;
  repo: string;
  ref: string;
  runtime: Runtime;
  runtime_version: string;
  status: AppStatus;
  domains: Domain[];
  routes: Route[];
  resources: AppResources;
  migration_command: string;
  pause_traffic_during_migration: boolean;
  system_packages: string[];
  linked_databases: string[];
  linked_redis: string | null;
  env_count: number;
  last_deploy: Deploy | null;
}

export interface Database {
  name: string;
  role: string;
  size_bytes: number;
  connection_limit: number;
  connections_active: number;
  extensions: string[];
  linked_apps: string[];
  created_at: string;
}

export interface RedisInstance {
  slug: string;
  app_slug: string;
  port: number;
  maxmemory_mb: number;
  used_memory_mb: number;
  maxmemory_policy: string;
  appendonly: boolean;
}

export interface ServiceStatus {
  name: string;
  ok: boolean;
  detail: string;
}

export interface HostStatus {
  hostname: string;
  ferrum_version: string;
  build_id: string;
  commit_sha: string;
  os: string;
  arch: string;
  uptime_secs: number;
  cpu_cores: number;
  cpu_pct: number;
  memory_used_mb: number;
  memory_total_mb: number;
  swap_used_mb: number;
  swap_total_mb: number;
  disk_used_gb: number;
  disk_total_gb: number;
  services: ServiceStatus[];
}

export interface MetricSeries {
  t: number[];
  values: Record<string, number[]>;
}

export interface BannedIp {
  ip: string;
  jail: string;
  banned_at: string;
  failures: number;
}

export interface FirewallRule {
  port: string;
  action: "allow" | "deny";
  from: string;
  note: string;
}

export interface Passkey {
  id: string;
  label: string;
  added_at: string;
  last_used_at: string | null;
}

export interface User {
  id: string;
  name: string;
  email: string;
  passkeys: Passkey[];
  enrolled: boolean;
}

export interface ApiToken {
  id: string;
  name: string;
  prefix: string;
  read_only: boolean;
  created_at: string;
  last_used_at: string | null;
}

export interface Session {
  id: string;
  device: string;
  ip: string;
  started_at: string;
  current: boolean;
}

export interface GithubConnection {
  connected: boolean;
  app_name: string;
  account: string;
  repos_accessible: number;
  installed_at: string;
}
