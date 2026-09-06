import { describe, expect, test } from "bun:test";
import { describeImport, importDotenv, parseDotenv, routePortKeys } from "./dotenv";
import type { EnvRow } from "@/features/apps/EnvironmentPanel";

const vars = (text: string) => parseDotenv(text).vars;

describe("parseDotenv", () => {
  test("plain assignments, export prefixes, blank lines and comments", () => {
    expect(vars("# mail\n\nSMTP_HOST=smtp.example.com\nexport SMTP_PORT = 587\n")).toEqual([
      { key: "SMTP_HOST", value: "smtp.example.com" },
      { key: "SMTP_PORT", value: "587" },
    ]);
  });

  test("an unquoted value stops at an inline comment but keeps a bare hash", () => {
    expect(vars("A=one two # note\nB=abc#def\nC=\n")).toEqual([
      { key: "A", value: "one two" },
      { key: "B", value: "abc#def" },
      { key: "C", value: "" },
    ]);
  });

  test("single quotes are literal, double quotes unescape and may span lines", () => {
    const text = `A='raw \\n # not a comment'\nB="line one\\nline two \\"quoted\\""\nKEY="-----BEGIN\nabc\n-----END"\nD=after\n`;
    expect(vars(text)).toEqual([
      { key: "A", value: "raw \\n # not a comment" },
      { key: "B", value: 'line one\nline two "quoted"' },
      { key: "KEY", value: "-----BEGIN\nabc\n-----END" },
      { key: "D", value: "after" },
    ]);
  });

  test("CRLF, a BOM, a repeated key and lines without an equals sign", () => {
    expect(parseDotenv("﻿A=1\r\nA=2\r\njust words\r\n")).toEqual({
      vars: [{ key: "A", value: "2" }],
      skipped: [],
    });
  });

  test("a key that is not a variable name is skipped and named", () => {
    expect(parseDotenv("9LIVES=x\nMY-KEY=y\nOK=z\n")).toEqual({
      vars: [{ key: "OK", value: "z" }],
      skipped: ["9LIVES", "MY-KEY"],
    });
  });
});

const row = (key: string, extra: Partial<EnvRow> = {}): EnvRow => ({
  key,
  value: "",
  stored: false,
  source: null,
  optional: false,
  suggestAppUrl: false,
  ...extra,
});

describe("importDotenv", () => {
  test("fills matching rows, adds the rest, and skips what Ferrum sets", () => {
    const rows = [
      row("STRIPE_KEY", { value: null, stored: true }),
      row("SMTP_HOST", { source: "from .env.example" }),
    ];
    const parsed = parseDotenv(
      "STRIPE_KEY=sk_live\nSMTP_HOST=smtp\nNEW_ONE=1\nPORT=3000\nWS_PORT=1\nANALYTICS_DATABASE_URL=x\nREDIS_URL=y\n",
    );
    const result = importDotenv(rows, parsed, routePortKeys([{ port_name: "main" }, { port_name: "ws" }]));
    expect(result.rows).toEqual([
      row("STRIPE_KEY", { value: "sk_live", stored: true }),
      row("SMTP_HOST", { value: "smtp", source: "from .env.example" }),
      row("NEW_ONE", { value: "1" }),
    ]);
    expect(result.filled).toBe(2);
    expect(result.added).toBe(1);
    expect(result.skipped).toEqual(["PORT", "WS_PORT", "ANALYTICS_DATABASE_URL", "REDIS_URL"]);
    expect(rows[0].value).toBeNull();
  });

  test("the summary names the file, the counts and the skipped keys", () => {
    const result = importDotenv([row("A")], parseDotenv("A=1\nB=2\nPORT=1\n"), []);
    expect(describeImport(".env.production", result)).toBe(
      "1 value filled from .env.production, 1 new key · skipped PORT (set by Ferrum or not a key)",
    );
    expect(describeImport(".env", importDotenv([], parseDotenv("A=1\nB=2"), []))).toBe(
      "0 values filled from .env, 2 new keys",
    );
  });
});
