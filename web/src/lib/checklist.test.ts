import { describe, expect, test } from "bun:test";
import { rows } from "./checklist";
import type { Security, User } from "@/types/api";

const security = (on: boolean): Security => ({
  firewall: { enabled: on, ssh_port: 22, rules: [], persisted: false },
  bans: { installed: on, jails: [], banned: [], allowlist: [] },
  updates: { enabled: on },
  ssh: { port: 22, password_auth: true, keys: [] },
  jobs: {
    firewall: { running: false, error: null },
    fail2ban: { running: false, error: null },
    updates: { running: false, error: null },
  },
});

const user = (name: string, passkeys: number): User => ({
  id: name,
  name,
  created_at: "2026-09-05T00:00:00Z",
  credential_count: passkeys,
  passkeys: Array.from({ length: passkeys }, (_, i) => ({
    id: `${name}-${i}`,
    label: null,
    added_at: "2026-09-05T00:00:00Z",
    last_used_at: null,
  })),
});

describe("rows", () => {
  test("nothing loaded means nothing done", () => {
    const all = rows({});
    expect(all).toHaveLength(6);
    expect(all.every((r) => !r.done)).toBe(true);
  });

  test("every row reads its own fact, and the passkey row is the caller's own count", () => {
    const all = rows({
      github: {
        connected: true,
        app_id: 1,
        app_slug: "ferrum-panel",
        app_name: "ferrum-panel",
        account: "saeed",
        installation_id: 7,
        connected_at: "2026-09-05T00:00:00Z",
      },
      postgres: { installed: true, major: 18, installing: false, error: null, tunnel: "", extensions: [] },
      security: security(true),
      users: [user("saeed", 2), user("other", 0)],
      me: { kind: "user", name: "saeed", read_only: false },
    });
    expect(all.every((r) => r.done)).toBe(true);

    const other = rows({
      users: [user("saeed", 2)],
      me: { kind: "user", name: "nobody", read_only: false },
    });
    expect(other.find((r) => r.id === "passkey")?.done).toBe(false);
  });
});
