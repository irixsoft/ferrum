import { describe, expect, test } from "bun:test";
import { expandShared, followSlug, slugify } from "./slug";
import type { EnvRow } from "@/features/apps/EnvironmentPanel";
import type { EnvRequirement } from "@/types/api";

describe("slugify", () => {
  test("lowercases and joins words with single hyphens", () => {
    expect(slugify("  My Shop -- API!  ")).toBe("my-shop-api");
  });

  test("stays within 25 characters so ferrum- plus the slug is a valid user name", () => {
    expect(slugify("a".repeat(40))).toBe("a".repeat(25));
  });

  test("a cut that lands on a separator leaves no trailing hyphen", () => {
    expect(slugify(`${"a".repeat(24)} tail`)).toBe("a".repeat(24));
  });
});

const required: EnvRequirement[] = [
  { key: "UPLOADS_DIR", about: null, default: "{{shared}}/uploads", optional: false },
  { key: "LOG_LEVEL", about: null, default: "info", optional: true },
];

const fromFile = (key: string, value: string): EnvRow => ({
  key,
  value,
  stored: false,
  source: "ferrum.toml",
  about: null,
  optional: false,
});

describe("followSlug", () => {
  test("an untouched {{shared}} default moves to the new slug's directory", () => {
    const rows = [fromFile("UPLOADS_DIR", expandShared("{{shared}}/uploads", "shop")), fromFile("LOG_LEVEL", "info")];
    expect(followSlug(rows, required, "shop", "store").map((r) => r.value)).toEqual([
      "/var/lib/ferrum/apps/store/shared/uploads",
      "info",
    ]);
  });

  test("a value the operator edited stays as typed", () => {
    const rows = [fromFile("UPLOADS_DIR", "/srv/uploads")];
    expect(followSlug(rows, required, "shop", "store")).toEqual(rows);
  });

  test("a row added by hand under the same key is left alone", () => {
    const rows = [{ ...fromFile("UPLOADS_DIR", "/var/lib/ferrum/apps/shop/shared/uploads"), source: null }];
    expect(followSlug(rows, required, "shop", "store")).toEqual(rows);
  });
});
