/**
 * Sample data, still read by: useSecurity.
 * Delete this file with the last one.
 */
import type { BannedIp, FirewallRule } from "@/types/api";

const t = (minsAgo: number) => new Date(Date.now() - minsAgo * 60_000).toISOString();

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
