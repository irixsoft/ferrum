import type { EnvRow } from "@/features/apps/EnvironmentPanel";

export interface DotenvVar {
  key: string;
  value: string;
}

export interface Parsed {
  vars: DotenvVar[];
  skipped: string[];
}

const KEY = /^[A-Za-z_][A-Za-z0-9_]*$/;
const MANAGED = ["PORT", "HOST", "DATABASE_URL", "REDIS_URL"];

/** Assignments in the order first seen; a repeated key keeps its last value. */
export function parseDotenv(text: string): Parsed {
  const vars: DotenvVar[] = [];
  const skipped: string[] = [];
  const lines = text.replace(/^﻿/, "").split(/\r?\n/);
  const set = (key: string, value: string) => {
    const existing = vars.find((v) => v.key === key);
    if (existing) existing.value = value;
    else vars.push({ key, value });
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq < 0) continue;
    const key = line.slice(0, eq).trim().replace(/^export\s+/, "");
    if (!KEY.test(key)) {
      skipped.push(key || line);
      continue;
    }
    let rest = line.slice(eq + 1).trim();
    const quote = rest[0];
    if (quote === '"' || quote === "'") {
      let body = rest.slice(1);
      let end = closingQuote(body, quote);
      while (end < 0 && i + 1 < lines.length) {
        body += "\n" + lines[++i];
        end = closingQuote(body, quote);
      }
      if (end < 0) {
        set(key, unescape(body, quote));
        continue;
      }
      set(key, unescape(body.slice(0, end), quote));
      continue;
    }
    const hash = rest.search(/\s#/);
    if (hash >= 0) rest = rest.slice(0, hash).trim();
    set(key, rest);
  }
  return { vars, skipped };
}

function closingQuote(body: string, quote: string): number {
  for (let i = 0; i < body.length; i++) {
    if (quote === '"' && body[i] === "\\") {
      i++;
      continue;
    }
    if (body[i] === quote) return i;
  }
  return -1;
}

function unescape(body: string, quote: string): string {
  if (quote === "'") return body;
  return body.replace(/\\(n|r|t|"|\\)/g, (_, c: string) =>
    c === "n" ? "\n" : c === "r" ? "\r" : c === "t" ? "\t" : c,
  );
}

export function routePortKeys(routes: Array<{ port_name: string }>): string[] {
  return routes.map((r) => (r.port_name === "main" ? "PORT" : `${r.port_name.toUpperCase()}_PORT`));
}

export function isManagedKey(key: string, managed: string[]): boolean {
  return managed.includes(key) || MANAGED.includes(key) || key.endsWith("_DATABASE_URL");
}

export interface Imported {
  rows: EnvRow[];
  filled: number;
  added: number;
  skipped: string[];
}

/** Fills the rows on screen from a parsed file; keys Ferrum sets are skipped and named. */
export function importDotenv(rows: EnvRow[], parsed: Parsed, managed: string[]): Imported {
  const next = rows.map((r) => ({ ...r }));
  let filled = 0;
  let added = 0;
  const skipped = [...parsed.skipped];
  for (const { key, value } of parsed.vars) {
    if (isManagedKey(key, managed)) {
      skipped.push(key);
      continue;
    }
    const row = next.find((r) => r.key === key);
    if (row) {
      row.value = value;
      filled++;
    } else {
      next.push({ key, value, stored: false, source: null, optional: false, suggestAppUrl: false });
      added++;
    }
  }
  return { rows: next, filled, added, skipped };
}

export function describeImport(file: string, result: Imported): string {
  const parts = [`${count(result.filled, "value")} filled from ${file}`];
  if (result.added) parts.push(`${count(result.added, "new key")}`);
  let text = parts.join(", ");
  if (result.skipped.length) text += ` · skipped ${result.skipped.join(", ")} (set by Ferrum or not a key)`;
  return text;
}

function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? "" : "s"}`;
}
