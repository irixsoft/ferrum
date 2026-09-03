import { describe, expect, test } from "bun:test";
import { rank, type PaletteItem } from "./palette";

const items: PaletteItem[] = [
  { kind: "page", label: "Dashboard", href: "/" },
  { kind: "page", label: "Deploys", href: "/deploys" },
  { kind: "page", label: "System", href: "/system" },
  { kind: "app", label: "ledger", hint: "ledger", href: "/apps/ledger" },
  { kind: "app", label: "Ecosystem panel", hint: "panel", href: "/apps/panel" },
  { kind: "database", label: "ledger_prod", hint: "PostgreSQL", href: "/databases" },
];

describe("rank", () => {
  test("an empty query keeps the given order", () => {
    expect(rank("", items)).toEqual(items);
    expect(rank("   ", items)).toEqual(items);
  });

  test("a word prefix beats a substring, case-insensitively", () => {
    expect(rank("SYS", items).map((i) => i.label)).toEqual(["System", "Ecosystem panel"]);
    expect(rank("led", items).map((i) => i.label)).toEqual(["ledger", "ledger_prod"]);
  });

  test("the hint counts as a substring", () => {
    expect(rank("postgres", items).map((i) => i.label)).toEqual(["ledger_prod"]);
  });

  test("nothing matching is nothing", () => {
    expect(rank("zzz", items)).toEqual([]);
  });
});
