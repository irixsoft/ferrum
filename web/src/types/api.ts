export type Runtime = "node" | "bun" | "dotnet" | "static";
export type Toolchain = Exclude<Runtime, "static">;
export type Tracking = "branch" | "releases";
export type PackageManager = "npm" | "pnpm" | "yarn" | "bun";

export type AppStatus = "new" | "live" | "building" | "failed" | "stopped" | "maintenance";

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
export type DeployTrigger = "webhook" | "manual" | "cli" | "rollback";

export interface DeployStep {
  state: DeployState;
  elapsed_secs: number | null;
  status: "done" | "active" | "pending" | "skipped" | "failed";
  note: string | null;
}

export interface Snapshot {
  id: string;
  database_id: string;
  database: string;
  deploy_id: string | null;
  path: string;
  taken_at: string;
}

export interface Deploy {
  id: string;
  app_id: string;
  app_slug: string;
  trigger: DeployTrigger;
  git_ref: string;
  commit_sha: string | null;
  commit_message: string | null;
  author: string | null;
  state: DeployState | null;
  outcome: DeployOutcome | null;
  failure_reason: string | null;
  queue_position: number | null;
  release_id: string | null;
  restore_deploy_id: string | null;
  started_at: string;
  finished_at: string | null;
  duration_secs: number | null;
  steps: DeployStep[];
  snapshots: Snapshot[];
}

export interface Release {
  id: string;
  app_id: string;
  dir: string;
  git_ref: string;
  commit_sha: string;
  commit_message: string | null;
  built_at: string;
  current: boolean;
}

export interface LogLine {
  seq: number;
  at: string;
  stream: "system" | "stdout" | "stderr";
  text: string;
}

export type CertStatus =
  | { kind: "issued"; not_after: string }
  | { kind: "waiting_for_dns"; detail: string }
  | { kind: "failed"; detail: string; retry_at: string }
  | { kind: "none" };

export interface DomainCert {
  domain: string;
  status: CertStatus;
}

export interface Route {
  path: string;
  port_name: string;
  port: number;
  websocket: boolean;
}

export interface RouteInput {
  path: string;
  port_name: string;
  websocket: boolean;
}

export interface Commands {
  install: string | null;
  build: string | null;
  start: string | null;
  migrate: string | null;
}

export interface Health {
  path: string;
  startup_budget_secs: number;
}

export interface App {
  id: string;
  slug: string;
  name: string;
  repository: string;
  git_ref: string;
  tracking: Tracking;
  root: string;
  runtime: Runtime;
  toolchain: Toolchain;
  runtime_version: string;
  commands: Commands;
  output_dir: string | null;
  health: Health;
  memory_mb: number;
  cpu_percent: number;
  pause_for_migrations: boolean;
  routes: Route[];
  packages: string[];
  domains: string[];
  current_release_id: string | null;
  status: AppStatus;
  /** No deploy has ever gone live, so there is no unit to restart. */
  never_live: boolean;
  created_at: string;
  updated_at: string;
}

export interface AppDetail extends App {
  env: Array<{ key: string }>;
  deployed: boolean;
  current_release: Release | null;
  certificates: DomainCert[];
  databases: string[];
  redis: RedisInstance | null;
  managed: string[];
  /** From the unit's cgroup; null while no process runs. */
  memory_bytes: number | null;
  memory_peak_bytes: number | null;
  cpu_pct: number | null;
}

export type LogSource = "app" | "access" | "error";
export type LogLevel = "info" | "warn" | "error";

export interface AppLogLine {
  at: string;
  level: LogLevel;
  text: string;
}

export type MetricRange = "1h" | "24h" | "7d";

export interface EnvVar {
  key: string;
  value: string;
}

/** A row without a value keeps the value already stored; values are never read back. */
export interface EnvChange {
  key: string;
  value?: string;
}

export interface NewApp {
  slug: string;
  name: string;
  repository: string;
  git_ref: string;
  tracking: Tracking;
  root: string;
  runtime: Runtime;
  toolchain: Toolchain;
  runtime_version: string;
  commands: Commands;
  output_dir: string | null;
  health: Health;
  memory_mb: number;
  cpu_percent: number;
  pause_for_migrations: boolean;
  routes: RouteInput[];
  packages: string[];
  domains: string[];
  env: EnvVar[];
}

export type AppChanges = Partial<Omit<NewApp, "slug" | "repository" | "env">>;

export interface Detection {
  kind: Runtime;
  toolchain: Toolchain;
  version: string | null;
  confidence: number;
  reasons: string[];
  commands: Commands;
  output_dir: string | null;
  health: Health;
  package_manager: PackageManager | null;
}

export interface FerrumToml {
  runtime: Runtime | null;
  version: string | null;
  install: string | null;
  build: string | null;
  start: string | null;
  migrate: string | null;
  output_dir: string | null;
  health_path: string | null;
  packages: string[];
}

export interface Detected {
  candidates: Detection[];
  ferrum_toml: FerrumToml | null;
  aptfile: string[];
  aptfile_rejected: string[];
}

export interface InstalledToolchain {
  kind: Toolchain;
  version: string;
  path: string;
  size_bytes: number;
  installed_at: string;
}

export interface Runtimes {
  installed: InstalledToolchain[];
  dotnet_channels: string[];
}

export type Progress =
  | { state: "downloading"; received: number; total: number | null }
  | { state: "extracting" }
  | { state: "installing" }
  | { state: "ready" }
  | { state: "failed"; error: string };

export interface Database {
  name: string;
  role: string;
  connection_limit: number;
  connections_active: number | null;
  size_bytes: number | null;
  extensions: string[];
  linked_apps: string[];
  created_at: string;
  restore: JobStatus;
}

export interface NewDatabase {
  name: string;
  connection_limit?: number;
  extensions?: string[];
  app_slug?: string;
}

export interface PostgresStatus {
  installed: boolean;
  major: number | null;
  installing: boolean;
  error: string | null;
  tunnel: string;
  extensions: string[];
}

export interface RedisInstance {
  app_id: string;
  port: number;
  maxmemory_mb: number;
  created_at: string;
}

export interface RedisListed {
  app_slug: string;
  port: number;
  maxmemory_mb: number;
  created_at: string;
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
  /** Certificates come from Let's Encrypt's staging directory, which browsers do not trust. */
  certificates_staging: boolean;
  services: ServiceStatus[];
}

export interface MetricSeries {
  t: number[];
  values: Record<string, number[]>;
}

export interface FirewallRule {
  port: string;
  action: string;
  from: string;
}

export interface BannedIp {
  ip: string;
  jail: string;
  /** fail2ban's local time read as UTC; null when the client printed no time. */
  banned_at: string | null;
}

export interface KeyFingerprint {
  bits: number;
  fingerprint: string;
  comment: string;
  kind: string;
}

export interface JobStatus {
  running: boolean;
  error: string | null;
}

export interface Security {
  firewall: { enabled: boolean; ssh_port: number; rules: FirewallRule[]; persisted: boolean };
  bans: { installed: boolean; jails: string[]; banned: BannedIp[]; allowlist: string[] };
  updates: { enabled: boolean };
  ssh: { port: number; password_auth: boolean; keys: KeyFingerprint[] };
  jobs: { firewall: JobStatus; fail2ban: JobStatus; updates: JobStatus };
}

export interface BuildLimits {
  memory_mb: number;
  build_secs: number;
  migrate_secs: number;
}

export interface BuildSettings extends BuildLimits {
  memory_total_mb: number;
}

export interface NginxFiles {
  managed: string;
  custom: string;
}

export interface Me {
  kind: "user" | "machine";
  name: string;
  read_only: boolean;
}

export interface VersionInfo {
  version: string;
  build_id: string;
  commit_sha: string;
  os: string;
  arch: string;
}

export interface LatestRelease {
  tag: string;
  version: string;
  name: string;
  notes: string;
  security: boolean;
  published_at: string | null;
  url: string;
  size_bytes: number;
}

export interface UpdateStatus {
  current: string;
  latest: LatestRelease | null;
  available: boolean;
  checked_at: string | null;
  auto: boolean;
  running: boolean;
  step: string | null;
  error: string | null;
  restarting: boolean;
}

export interface Passkey {
  id: string;
  label: string | null;
  added_at: string;
  last_used_at: string | null;
}

export interface User {
  id: string;
  name: string;
  created_at: string;
  credential_count: number;
  passkeys: Passkey[];
}

export interface ApiToken {
  id: string;
  name: string;
  prefix: string;
  read_only: boolean;
  created_at: string;
  last_used: string | null;
}

export interface MintedToken {
  token: ApiToken;
  secret: string;
}

export interface Enrolled {
  user: User;
  enrollment_url: string;
}

export interface Session {
  id: string;
  device: string | null;
  ip: string | null;
  started_at: string;
  last_seen: string;
  current: boolean;
}

export type GithubStatus =
  | { connected: false }
  | {
      connected: true;
      app_id: number;
      app_slug: string;
      app_name: string;
      account: string;
      installation_id: number | null;
      connected_at: string;
    };

export interface GithubHandoff {
  manifest: Record<string, unknown>;
  state: string;
  action: string;
}

export interface GithubRepo {
  full_name: string;
  private: boolean;
  default_branch: string;
  pushed_at: string | null;
}
