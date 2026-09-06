/** The first paragraph of a release's notes, without markdown headings or list markers. */
export function summary(notes: string) {
  const paragraph = notes
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .find((p) => p && !p.startsWith("#"));
  return paragraph?.replace(/^[-*]\s+/gm, "").replace(/\s+/g, " ") ?? "";
}
