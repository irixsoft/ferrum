export type PaletteKind = "page" | "app" | "database";

export interface PaletteItem {
  kind: PaletteKind;
  label: string;
  hint?: string;
  href: string;
}

const score = (query: string, item: PaletteItem) => {
  const label = item.label.toLowerCase();
  if (label.split(/\s+/).some((word) => word.startsWith(query))) return 2;
  if (label.includes(query) || item.hint?.toLowerCase().includes(query)) return 1;
  return 0;
};

/** Word-prefix matches first, then substrings; ties keep the order the items came in. */
export function rank(query: string, items: PaletteItem[]): PaletteItem[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return items;
  return items
    .map((item, index) => ({ item, index, score: score(needle, item) }))
    .filter((m) => m.score > 0)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((m) => m.item);
}
