/** The one seam between the panel and the server. Nothing else may fetch. */
import { useQuery } from "@tanstack/react-query";
import * as mock from "./mock";
import type { App, Deploy, HostStatus, MetricSeries } from "@/types/api";

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api${path}`, {
    credentials: "same-origin",
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
    ...init,
  });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.json() as Promise<T>;
}

const settle = <T>(value: T, ms = 120) =>
  new Promise<T>((resolve) => setTimeout(() => resolve(value), ms));

export const keys = {
  host: ["host"] as const,
  apps: ["apps"] as const,
  app: (slug: string) => ["apps", slug] as const,
  deploys: ["deploys"] as const,
  databases: ["databases"] as const,
  redis: ["redis"] as const,
  metrics: ["metrics"] as const,
  security: ["security"] as const,
  settings: ["settings"] as const,
};

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

export function useSettings() {
  return useQuery({
    queryKey: keys.settings,
    queryFn: () =>
      settle({
        users: mock.users,
        tokens: mock.tokens,
        sessions: mock.sessions,
        github: mock.github,
      }),
  });
}
