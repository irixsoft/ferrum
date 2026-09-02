/** The one seam between the panel and the server. Nothing else may fetch. */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as mock from "./mock";
import type {
  ApiToken,
  App,
  AppChanges,
  AppDetail,
  AppLogLine,
  Database,
  Deploy,
  DeployOutcome,
  Detected,
  Enrolled,
  EnvChange,
  GithubHandoff,
  GithubRepo,
  GithubStatus,
  HostStatus,
  LogLine,
  LogSource,
  Me,
  MetricRange,
  MetricSeries,
  MintedToken,
  NewApp,
  NewDatabase,
  PostgresStatus,
  Progress,
  RedisInstance,
  RedisListed,
  Release,
  Runtimes,
  Session,
  Toolchain,
  User,
  VersionInfo,
} from "@/types/api";

export class ApiError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`/api${path}`, {
      credentials: "same-origin",
      headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
      ...init,
    });
  } catch {
    throw new ApiError(0, "Could not reach the server.");
  }

  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new ApiError(res.status, body?.error ?? `${res.status} ${res.statusText}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json() as Promise<T>;
}

const settle = <T>(value: T, ms = 120) =>
  new Promise<T>((resolve) => setTimeout(() => resolve(value), ms));

export const keys = {
  me: ["me"] as const,
  version: ["version"] as const,
  host: ["host"] as const,
  apps: ["apps"] as const,
  app: (slug: string) => ["apps", slug] as const,
  deploys: ["deploys"] as const,
  running: ["deploys", "running"] as const,
  deploy: (id: string) => ["deploys", "one", id] as const,
  appDeploys: (slug: string) => ["deploys", "app", slug] as const,
  releases: (slug: string) => ["releases", slug] as const,
  postgres: ["postgres"] as const,
  databases: ["databases"] as const,
  redis: ["redis"] as const,
  metrics: (scope: string, range: MetricRange) => ["metrics", scope, range] as const,
  logs: (slug: string, source: LogSource) => ["logs", slug, source] as const,
  security: ["security"] as const,
  users: ["users"] as const,
  sessions: ["sessions"] as const,
  tokens: ["tokens"] as const,
  github: ["github"] as const,
  githubRepos: ["github", "repos"] as const,
  runtimes: ["runtimes"] as const,
};

export function useMe(enabled = true) {
  return useQuery({
    queryKey: keys.me,
    queryFn: () => request<Me>("/me"),
    enabled,
    retry: false,
    staleTime: 30_000,
  });
}

export function useVersion() {
  return useQuery({
    queryKey: keys.version,
    queryFn: () => request<VersionInfo>("/version"),
    staleTime: Infinity,
  });
}

export function useHost() {
  return useQuery({
    queryKey: keys.host,
    queryFn: () => request<HostStatus>("/host"),
    refetchInterval: 10_000,
  });
}

export function useApps() {
  return useQuery({ queryKey: keys.apps, queryFn: () => request<App[]>("/apps") });
}

export function useApp(slug: string) {
  return useQuery({
    queryKey: keys.app(slug),
    queryFn: () =>
      request<AppDetail>(`/apps/${slug}`).catch((e: unknown) => {
        if (e instanceof ApiError && e.status === 404) return undefined;
        throw e;
      }),
    retry: false,
  });
}

const anyRunning = (deploys: Deploy[] | undefined) => deploys?.some((d) => d.state !== null) ?? false;

export function useDeploys(slug?: string) {
  return useQuery({
    queryKey: slug ? keys.appDeploys(slug) : keys.deploys,
    queryFn: () => request<Deploy[]>(slug ? `/apps/${slug}/deploys` : "/deploys"),
    refetchInterval: (query) => (anyRunning(query.state.data) ? 2000 : false),
  });
}

/** Polls quickly while one runs and slowly otherwise, so a push shows up without a reload. */
export function useRunningDeploy() {
  return useQuery({
    queryKey: keys.running,
    queryFn: () => request<Deploy | null>("/deploys?running=1"),
    refetchInterval: (query) => (query.state.data ? 2000 : 15_000),
  });
}

export function useDeploy(id: string | null) {
  return useQuery({
    queryKey: keys.deploy(id ?? ""),
    queryFn: () => request<Deploy>(`/deploys/${id}`),
    enabled: id !== null,
    refetchInterval: (query) => (query.state.data?.state !== null ? 2000 : false),
  });
}

export function useReleases(slug: string) {
  return useQuery({
    queryKey: keys.releases(slug),
    queryFn: () => request<Release[]>(`/apps/${slug}/releases`),
  });
}

export function usePostgres() {
  return useQuery({
    queryKey: keys.postgres,
    queryFn: () => request<PostgresStatus>("/postgres"),
    refetchInterval: (query) => (query.state.data?.installing ? 1500 : false),
  });
}

export function useDatabases() {
  return useQuery({ queryKey: keys.databases, queryFn: () => request<Database[]>("/databases") });
}

export function useRedisInstances() {
  return useQuery({ queryKey: keys.redis, queryFn: () => request<RedisListed[]>("/redis") });
}

/** `scope` is "host" or an app slug; the server buckets the window to at most 360 points. */
export function useMetrics(scope: string, range: MetricRange) {
  const path = scope === "host" ? `/metrics?range=${range}` : `/apps/${scope}/metrics?range=${range}`;
  return useQuery({
    queryKey: keys.metrics(scope, range),
    queryFn: () => request<MetricSeries>(path),
    refetchInterval: 30_000,
  });
}

export function useAppLogs(slug: string, source: LogSource, lines = 200, enabled = true) {
  return useQuery({
    queryKey: keys.logs(slug, source),
    queryFn: () => request<AppLogLine[]>(`/apps/${slug}/logs?source=${source}&lines=${lines}`),
    enabled,
  });
}

export function useSecurity() {
  return useQuery({
    queryKey: keys.security,
    queryFn: () => settle({ firewall: mock.firewall, bans: mock.bans }),
  });
}

export function useUsers() {
  return useQuery({ queryKey: keys.users, queryFn: () => request<User[]>("/users") });
}

export function useSessions() {
  return useQuery({ queryKey: keys.sessions, queryFn: () => request<Session[]>("/sessions") });
}

export function useTokens() {
  return useQuery({ queryKey: keys.tokens, queryFn: () => request<ApiToken[]>("/tokens") });
}

export function useGithub() {
  return useQuery({ queryKey: keys.github, queryFn: () => request<GithubStatus>("/github/status") });
}

export function useGithubRepos() {
  return useQuery({
    queryKey: keys.githubRepos,
    queryFn: () => request<GithubRepo[]>("/github/repos"),
    retry: false,
  });
}

export function useRuntimes() {
  return useQuery({ queryKey: keys.runtimes, queryFn: () => request<Runtimes>("/runtimes") });
}

function useInvalidating<TArgs, TResult>(
  key: readonly unknown[] | Array<readonly unknown[]>,
  run: (args: TArgs) => Promise<TResult>,
) {
  const client = useQueryClient();
  const affected = key.length > 0 && Array.isArray(key[0]) ? (key as Array<readonly unknown[]>) : [key];
  return useMutation({
    mutationFn: run,
    onSuccess: () =>
      Promise.all(affected.map((queryKey) => client.invalidateQueries({ queryKey }))),
  });
}

const body = (value: unknown, method = "POST") => ({ method, body: JSON.stringify(value) });

export function useCreateUser() {
  return useInvalidating(keys.users, (name: string) =>
    request<Enrolled>("/users", body({ name })),
  );
}

export function useEnrollmentLink() {
  return useInvalidating(keys.users, (id: string) =>
    request<{ enrollment_url: string }>(`/users/${id}/enrollment`, { method: "POST" }),
  );
}

export function useCreateToken() {
  return useInvalidating(keys.tokens, (token: { name: string; read_only: boolean }) =>
    request<MintedToken>("/tokens", body(token)),
  );
}

export function useRevokeToken() {
  return useInvalidating(keys.tokens, (id: string) =>
    request<void>(`/tokens/${id}`, { method: "DELETE" }),
  );
}

export function useRevokeSession() {
  return useInvalidating(keys.sessions, (id: string) =>
    request<void>(`/sessions/${id}`, { method: "DELETE" }),
  );
}

export function useDetect() {
  return useMutation({
    mutationFn: (input: { repository: string; ref: string; root: string }) =>
      request<Detected>("/apps/detect", body(input)),
  });
}

export function useCreateApp() {
  return useInvalidating(keys.apps, (app: NewApp) => request<App>("/apps", body(app)));
}

export function useUpdateApp(slug: string) {
  return useInvalidating(keys.apps, (changes: AppChanges) =>
    request<App>(`/apps/${slug}`, body(changes, "PATCH")),
  );
}

export function useDeleteApp(slug: string) {
  return useInvalidating(keys.apps, (name: string) =>
    request<void>(`/apps/${slug}`, body({ name }, "DELETE")),
  );
}

export function useSetEnv(slug: string) {
  return useInvalidating(keys.app(slug), (vars: EnvChange[]) =>
    request<void>(`/apps/${slug}/env`, body(vars, "PUT")),
  );
}

export function useInstallPostgres() {
  return useInvalidating(keys.postgres, () =>
    request<PostgresStatus>("/postgres/install", { method: "POST" }),
  );
}

export function useCreateDatabase() {
  return useInvalidating([keys.databases, keys.apps], (database: NewDatabase) =>
    request<Database>("/databases", body(database)),
  );
}

export function useDeleteDatabase() {
  return useInvalidating([keys.databases, keys.apps], (name: string) =>
    request<void>(`/databases/${name}`, body({ name }, "DELETE")),
  );
}

export function useEnableExtension() {
  return useInvalidating(keys.databases, (input: { database: string; extension: string }) =>
    request<void>(`/databases/${input.database}/extensions`, body({ name: input.extension })),
  );
}

export function useLinkDatabase(slug: string) {
  return useInvalidating([keys.apps, keys.databases], (name: string) =>
    request<void>(`/apps/${slug}/databases/${name}`, { method: "POST" }),
  );
}

export function useUnlinkDatabase(slug: string) {
  return useInvalidating([keys.apps, keys.databases], (name: string) =>
    request<void>(`/apps/${slug}/databases/${name}`, { method: "DELETE" }),
  );
}

export function useRequestRedis(slug: string) {
  return useInvalidating([keys.apps, keys.redis], (maxmemory_mb: number) =>
    request<RedisInstance>(`/apps/${slug}/redis`, body({ maxmemory_mb })),
  );
}

export function useReleaseRedis(slug: string) {
  return useInvalidating([keys.apps, keys.redis], () =>
    request<void>(`/apps/${slug}/redis`, { method: "DELETE" }),
  );
}

export function useTriggerDeploy(slug: string) {
  return useInvalidating([keys.deploys, keys.apps], (ref?: string) =>
    request<Deploy>(`/apps/${slug}/deploys`, body(ref ? { ref } : {})),
  );
}

export function useRollback(slug: string) {
  return useInvalidating([keys.deploys, keys.apps], (input: { release_id: string; restore_deploy_id?: string }) =>
    request<Deploy>(`/apps/${slug}/rollback`, body(input)),
  );
}

export function useRestoreSnapshot() {
  return useInvalidating(keys.deploys, (id: string) =>
    request<void>(`/snapshots/${id}/restore`, { method: "POST" }),
  );
}

export function useCancelDeploy() {
  return useInvalidating([keys.deploys, keys.apps], (id: string) =>
    request<void>(`/deploys/${id}/cancel`, { method: "POST" }),
  );
}

export function useRetryCertificate(slug: string) {
  return useInvalidating(keys.apps, () =>
    request<void>(`/apps/${slug}/certificates`, { method: "POST" }),
  );
}

export function useRestartApp(slug: string) {
  return useInvalidating([keys.apps, keys.app(slug)], () =>
    request<void>(`/apps/${slug}/restart`, { method: "POST" }),
  );
}

/** Opens an SSE response and hands each `event`/`data` frame over until the body ends. */
async function readFrames(
  path: string,
  onFrame: (event: string, data: string) => boolean | void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(`/api${path}`, {
    credentials: "same-origin",
    headers: { accept: "text/event-stream" },
    signal,
  });
  if (!res.ok || !res.body) {
    const failed = await res.json().catch(() => null);
    throw new ApiError(res.status, failed?.error ?? `${res.status} ${res.statusText}`);
  }
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    buffered += decoder.decode(value, { stream: true });
    const frames = buffered.split("\n\n");
    buffered = frames.pop() ?? "";
    for (const frame of frames) {
      let event = "message";
      const data: string[] = [];
      for (const line of frame.split("\n")) {
        if (line.startsWith("event:")) event = line.slice(6).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).trim());
      }
      if (data.length === 0) continue;
      if (onFrame(event, data.join("\n")) === true) {
        await reader.cancel();
        return;
      }
    }
  }
}

/** Stored lines first, then live ones; resolves with the outcome once the deploy ends. */
export async function followDeployLog(
  id: string,
  onLine: (line: LogLine) => void,
  signal?: AbortSignal,
): Promise<DeployOutcome> {
  let outcome: DeployOutcome | null = null;
  await readFrames(
    `/deploys/${id}/log`,
    (event, data) => {
      if (event === "done") {
        outcome = (JSON.parse(data) as { outcome: DeployOutcome }).outcome;
        return true;
      }
      if (event === "line") onLine(JSON.parse(data) as LogLine);
    },
    signal,
  );
  if (outcome === null) throw new ApiError(0, "The log ended before the deploy did.");
  return outcome;
}

/** The last lines, then live ones from journald; ends only when `signal` aborts. */
export async function followAppLog(
  slug: string,
  lines: number,
  onLine: (line: AppLogLine) => void,
  signal?: AbortSignal,
): Promise<void> {
  await readFrames(
    `/apps/${slug}/logs?follow=1&lines=${lines}`,
    (event, data) => {
      if (event === "line") onLine(JSON.parse(data) as AppLogLine);
    },
    signal,
  );
}

export async function resolveVersion(kind: Toolchain, wanted: string | null): Promise<string> {
  const query = wanted ? `?version=${encodeURIComponent(wanted)}` : "";
  const { version } = await request<{ version: string }>(`/runtimes/${kind}/resolve${query}`);
  return version;
}

/** The install streams progress over SSE, and `EventSource` cannot POST. */
export async function installRuntime(
  kind: Toolchain,
  version: string,
  onProgress: (p: Progress) => void,
): Promise<void> {
  const res = await fetch(`/api/runtimes/${kind}/${version}`, {
    method: "POST",
    credentials: "same-origin",
  });
  if (!res.ok || !res.body) {
    const failed = await res.json().catch(() => null);
    throw new ApiError(res.status, failed?.error ?? `${res.status} ${res.statusText}`);
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  let last: Progress | null = null;
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buffered += decoder.decode(value, { stream: true });
    const events = buffered.split("\n\n");
    buffered = events.pop() ?? "";
    for (const event of events) {
      const data = event
        .split("\n")
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).trim())
        .join("\n");
      if (!data) continue;
      last = JSON.parse(data) as Progress;
      onProgress(last);
      if (last.state === "failed") throw new ApiError(0, last.error);
    }
  }
  if (last?.state !== "ready") throw new ApiError(0, "The install ended without finishing.");
}

/** GitHub renders a confirmation page, which only a form the browser submits can reach. */
export function useConnectGithub() {
  return useMutation({
    mutationFn: async () => {
      const { manifest, action } = await request<GithubHandoff>("/github/connect", { method: "POST" });
      const form = document.createElement("form");
      form.method = "POST";
      form.action = action;
      const field = document.createElement("input");
      field.type = "hidden";
      field.name = "manifest";
      field.value = JSON.stringify(manifest);
      form.appendChild(field);
      document.body.appendChild(form);
      form.submit();
    },
  });
}

export function useDisconnectGithub() {
  return useInvalidating(keys.github, () => request<void>("/github", { method: "DELETE" }));
}

export function useSignOut() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => request<void>("/auth/logout", { method: "POST" }),
    onSuccess: () => client.clear(),
  });
}
