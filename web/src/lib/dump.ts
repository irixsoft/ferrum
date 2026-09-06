export type DumpFormat = "custom" | "plain" | "gzip";

const PGDMP = [0x50, 0x47, 0x44, 0x4d, 0x50];

export function sniffDump(head: Uint8Array): DumpFormat {
  if (head.length >= PGDMP.length && PGDMP.every((b, i) => head[i] === b)) return "custom";
  if (head.length >= 2 && head[0] === 0x1f && head[1] === 0x8b) return "gzip";
  return "plain";
}

export async function sniffFile(file: File): Promise<DumpFormat> {
  return sniffDump(new Uint8Array(await file.slice(0, PGDMP.length).arrayBuffer()));
}

export const GZIP_REFUSED = "That is a gzip stream. Ferrum restores what pg_dump wrote; gunzip it first.";

export function describeDump(format: DumpFormat): string {
  return format === "custom" ? "pg_dump custom format" : format === "plain" ? "plain SQL" : "gzip";
}
