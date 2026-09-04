import { describe, expect, test } from "bun:test";
import { sniffDump } from "./dump";

const bytes = (...b: number[]) => new Uint8Array(b);
const text = (s: string) => new TextEncoder().encode(s);

describe("sniffDump", () => {
  test("pg_dump's custom format announces itself", () => {
    expect(sniffDump(text("PGDMP\x01\x0e\x00"))).toBe("custom");
  });

  test("a gzip stream is caught before any of it is uploaded", () => {
    expect(sniffDump(bytes(0x1f, 0x8b, 0x08, 0x00))).toBe("gzip");
  });

  test("anything else is treated as plain SQL, including a short or empty file", () => {
    expect(sniffDump(text("--\n-- PostgreSQL database dump\n"))).toBe("plain");
    expect(sniffDump(text("PGD"))).toBe("plain");
    expect(sniffDump(bytes())).toBe("plain");
  });
});
