import type { EnvRow } from "@/features/apps/EnvironmentPanel";
import type { EnvRequirement } from "@/types/api";

export const SLUG_MAX = 25;

export const slugify = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+/, "")
    .slice(0, SLUG_MAX)
    .replace(/-+$/, "");

export function sharedDir(slug: string): string {
  return `/var/lib/ferrum/apps/${slug.trim() || "<slug>"}/shared`;
}

export const expandShared = (template: string, slug: string) => template.replaceAll("{{shared}}", sharedDir(slug));

/** A `{{shared}}` default moves to the new slug's directory; a value the operator changed stays as typed. */
export function followSlug(rows: EnvRow[], required: EnvRequirement[], from: string, to: string): EnvRow[] {
  return rows.map((row) => {
    const template = required.find((r) => r.key === row.key)?.default;
    if (row.source !== "ferrum.toml" || !template?.includes("{{shared}}")) return row;
    return row.value === expandShared(template, from) ? { ...row, value: expandShared(template, to) } : row;
  });
}
