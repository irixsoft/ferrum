import type { GithubStatus, Me, PostgresStatus, Security, User } from "@/types/api";

export type ChecklistId = "github" | "postgres" | "firewall" | "fail2ban" | "updates" | "passkey";

export interface ChecklistRow {
  id: ChecklistId;
  label: string;
  done: boolean;
}

export interface ChecklistInput {
  github?: GithubStatus;
  postgres?: PostgresStatus;
  security?: Security;
  users?: User[];
  me?: Me;
}

/** A row is done only when the box says so; anything not loaded yet counts as not done. */
export function rows({ github, postgres, security, users, me }: ChecklistInput): ChecklistRow[] {
  const mine = users?.find((u) => u.name === me?.name);
  return [
    { id: "github", label: "Connect GitHub", done: github?.connected === true },
    { id: "postgres", label: "Install PostgreSQL", done: postgres?.installed === true },
    { id: "firewall", label: "Enable the firewall", done: security?.firewall.enabled === true },
    { id: "fail2ban", label: "Enable fail2ban", done: security?.bans.installed === true },
    { id: "updates", label: "Enable security updates", done: security?.updates.enabled === true },
    { id: "passkey", label: "Add a second passkey", done: (mine?.passkeys.length ?? 0) >= 2 },
  ];
}
