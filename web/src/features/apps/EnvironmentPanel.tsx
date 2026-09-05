import { useState } from "react";
import { Plus, X } from "lucide-react";
import { ApiError, useSetEnv } from "@/lib/api";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import type { EnvChange, EnvEntry, EnvHint } from "@/types/api";

/** `value` is null while the stored value is untouched; a hint row starts unstored with "". */
export interface EnvRow {
  key: string;
  value: string | null;
  stored: boolean;
  source: string | null;
  optional: boolean;
  suggestAppUrl: boolean;
}

const INPUT =
  "h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 font-mono text-[13px]";

export const HINTS_NOTE =
  "These keys came from the repository's example and schema files. Check the code for anything they miss; a stale example file is common.";

export function rowsFromHints(hints: EnvHint[]): EnvRow[] {
  return hints.map((h) => ({
    key: h.key,
    value: "",
    stored: false,
    source: h.source,
    optional: h.optional,
    suggestAppUrl: h.suggest_app_url,
  }));
}

function rowsFromEntries(entries: EnvEntry[]): EnvRow[] {
  return entries.map((e) => ({
    key: e.key,
    value: e.set ? null : "",
    stored: e.set,
    source: e.source,
    optional: e.optional,
    suggestAppUrl: false,
  }));
}

export function blankRow(): EnvRow {
  return { key: "", value: "", stored: false, source: null, optional: false, suggestAppUrl: false };
}

export function EnvRows({
  rows,
  onChange,
  managed = [],
}: {
  rows: EnvRow[];
  onChange: (rows: EnvRow[]) => void;
  managed?: string[];
}) {
  const update = (i: number, row: EnvRow) => onChange(rows.map((r, j) => (j === i ? row : r)));
  return (
    <>
      {managed.map((key) => (
        <div key={key} className="flex items-center gap-2 h-9">
          <span className={`${INPUT} w-56 flex items-center opacity-70`}>{key}</span>
          <span className={`${INPUT} flex-1 min-w-0 flex items-center text-ink-4`}>••••••••</span>
          <Badge tone="accent" className="shrink-0">
            set by Ferrum
          </Badge>
        </div>
      ))}
      {rows.length === 0 && managed.length === 0 ? (
        <p className="text-[13.5px] text-ink-3">No variables yet.</p>
      ) : null}
      {rows.map((row, i) => (
        <div key={i} className="flex items-center gap-2">
          <input
            value={row.key}
            disabled={row.stored || row.source !== null}
            onChange={(e) => update(i, { ...row, key: e.target.value.toUpperCase() })}
            placeholder="KEY"
            className={`${INPUT} w-56 disabled:opacity-70`}
          />
          <input
            value={row.value ?? ""}
            onChange={(e) => update(i, { ...row, value: e.target.value })}
            placeholder={row.value === null ? "••••••••" : row.stored ? "value" : "not set"}
            className={`${INPUT} flex-1 min-w-0`}
          />
          {row.source ? (
            <Badge className="shrink-0 hidden sm:inline-flex" title={row.source}>
              {row.optional ? "optional" : row.source}
            </Badge>
          ) : null}
          <Button
            size="icon"
            variant="ghost"
            aria-label={`Remove ${row.key}`}
            onClick={() => onChange(rows.filter((_, j) => j !== i))}
          >
            <X size={14} />
          </Button>
        </div>
      ))}
    </>
  );
}

/** Values never come back from the server: a stored row shows dots until it is edited. */
export function EnvironmentPanel({
  slug,
  entries,
  managed,
}: {
  slug: string;
  entries: EnvEntry[];
  managed: string[];
}) {
  const [rows, setRows] = useState<EnvRow[]>(() => rowsFromEntries(entries));
  const [dirty, setDirty] = useState(false);
  const save = useSetEnv(slug);
  const hinted = rows.some((r) => r.source !== null);

  const change = (next: EnvRow[]) => {
    setRows(next);
    setDirty(true);
  };

  const submit = async () => {
    const sending = rows.filter((r) => r.key.trim() && (r.stored || r.value !== ""));
    const changes: EnvChange[] = sending.map((r) =>
      r.value === null ? { key: r.key.trim() } : { key: r.key.trim(), value: r.value },
    );
    await save.mutateAsync(changes);
    setRows(
      rows
        .filter((r) => r.key.trim())
        .map((r) => (sending.includes(r) ? { ...r, key: r.key.trim(), value: null, stored: true } : r)),
    );
    setDirty(false);
  };

  return (
    <Card>
      <CardHeader
        title="Environment"
        hint="Written to shared/.env at 0600, owned by the app user, read by the unit and the build"
        action={
          <Button size="sm" variant="ghost" onClick={() => change([...rows, blankRow()])}>
            <Plus size={14} />
            Add
          </Button>
        }
      />
      <CardBody className="grid gap-2">
        <EnvRows rows={rows} onChange={change} managed={managed} />
        {hinted ? <p className="text-[12.5px] text-ink-4 mt-1">{HINTS_NOTE}</p> : null}
        {save.error ? (
          <p className="text-[12.5px] text-fail">
            {save.error instanceof ApiError ? save.error.message : String(save.error)}
          </p>
        ) : null}
      </CardBody>
      <CardFoot>
        <span>
          Your variables are set for the build too, so the <Code>NEXT_PUBLIC_*</Code> and{" "}
          <Code>VITE_*</Code> values you enter reach the client bundle. Ferrum itself adds only{" "}
          <Code>PORT</Code> and <Code>HOST</Code>.
        </span>
        <Button size="sm" variant="primary" disabled={!dirty || save.isPending} onClick={submit}>
          Save
        </Button>
      </CardFoot>
    </Card>
  );
}
