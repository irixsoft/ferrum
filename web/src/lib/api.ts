/** The one seam between the panel and the server. Nothing else may fetch. */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as mock from "./mock";
import type {
  ApiToken,
  App,
  Deploy,
  Enrolled,
  GithubHandoff,
  GithubRepo,
  GithubStatus,
  HostStatus,
  Me,
  MetricSeries,
  MintedToken,
  Session,
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
  databases: ["databases"] as const,
  redis: ["redis"] as const,
  metrics: ["metrics"] as const,
  security: ["security"] as const,
  users: ["users"] as const,
  sessions: ["sessions"] as const,
  tokens: ["tokens"] as const,
  github: ["github"] as const,
  githubRepos: ["github", "repos"] as const,
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
  return useQuery({ queryKey: keys.host, queryFn: () => settle<HostStatus>(mock.host) });
}

export function useApps() {
  return useQuery({ queryKey: keys.apps, queryFn: () => settle<App[]>(mock.apps) });
}

export function useApp(slug: string) {
  return useQuery({
    queryKey: keys.app(slug),
    queryFn: () => settle<App | undefined>(mock.apps.find((a) => a.slug === slug)),
  });
}

export function useDeploys() {
  return useQuery({ queryKey: keys.deploys, queryFn: () => settle<Deploy[]>(mock.deployHistory) });
}

export function useRunningDeploy() {
  return useQuery({
    queryKey: [...keys.deploys, "running"],
    queryFn: () => settle<Deploy | null>(mock.runningDeploy),
  });
}

export function useDatabases() {
  return useQuery({
    queryKey: keys.databases,
    queryFn: () => settle({ databases: mock.databases, redis: mock.redisInstances }),
  });
}

export function useMetrics() {
  return useQuery({ queryKey: keys.metrics, queryFn: () => settle<MetricSeries>(mock.hostMetrics()) });
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

function useInvalidating<TArgs, TResult>(
  key: readonly unknown[],
  run: (args: TArgs) => Promise<TResult>,
) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: run,
    onSuccess: () => client.invalidateQueries({ queryKey: key }),
  });
}

const body = (value: unknown) => ({ method: "POST", body: JSON.stringify(value) });

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
