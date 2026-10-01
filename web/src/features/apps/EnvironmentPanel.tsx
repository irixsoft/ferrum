import { useRef, useState, type ReactNode } from "react";
import { Plus, Upload, X } from "lucide-react";
import { ApiError, useApp, useSetEnv, useSetLabels } from "@/lib/api";
import { describeImport, importDotenv, parseDotenv, processPortKeys, type PortedProcess } from "@/lib/dotenv";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import type { EnvChange, EnvEntry, EnvRequirement, LabelChanges, ManagedVar } from "@/types/api";

/** `value` is null while the stored value is untouched; a required row starts unstored with its default or "". */
export interface EnvRow {
  key: string;
  value: string | null;
  stored: boolean;
  source: string | null;
  about: string | null;
}

const INPUT =
  "h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 font-mono text-[13px]";

export const REQUIRED_NOTE =
  "These keys are required by the repo's ferrum.toml. A deploy is refused while one of them has no value.";

/** The default is shown as it will be written: `{{shared}}` becomes the app's shared directory on the server. */
export function rowsFromRequired(required: EnvRequirement[], slug: string): EnvRow[] {
  return required.map((r) => ({
    key: r.key,
    value: r.default ? r.default.replaceAll("{{shared}}", `/var/lib/ferrum/apps/${slug}/shared`) : "",
    stored: false,
    source: "ferrum.toml",
    about: r.about,
  }));
}

function rowsFromEntries(entries: EnvEntry[]): EnvRow[] {
  return entries.map((e) => ({
    key: e.key,
    value: e.set ? null : "",
    stored: e.set,
    source: e.source,
    about: e.about,
  }));
}

export function blankRow(): EnvRow {
  return { key: "", value: "", stored: false, source: null, about: null };
}

/** The file is read in the browser and fills the rows; it is never uploaded or kept. */
export function ImportEnv({
  rows,
  managed,
  processes,
  onImport,
}: {
  rows: EnvRow[];
  managed: string[];
  processes: PortedProcess[];
  onImport: (rows: EnvRow[], note: string) => void;
}) {
  const picker = useRef<HTMLInputElement>(null);
  const pick = async (file: File | null) => {
    if (!file) return;
    const result = importDotenv(rows, parseDotenv(await file.text()), [...managed, ...processPortKeys(processes)]);
    onImport(result.rows, describeImport(file.name, result));
    if (picker.current) picker.current.value = "";
  };
  return (
    <>
      <input ref={picker} type="file" hidden onChange={(e) => pick(e.target.files?.[0] ?? null)} />
      <Button size="sm" variant="ghost" onClick={() => picker.current?.click()}>
        <Upload size={14} />
        Import .env
      </Button>
    </>
  );
}

export function EnvRows({
  rows,
  onChange,
  managed = [],
  managedRows,
}: {
  rows: EnvRow[];
  onChange: (rows: EnvRow[]) => void;
  managed?: string[];
  managedRows?: ReactNode;
}) {
  const update = (i: number, row: EnvRow) => onChange(rows.map((r, j) => (j === i ? row : r)));
  return (
    <>
      {managedRows ??
        managed.map((key) => (
          <div key={key} className="flex items-center gap-2 h-9">
            <span className={`${INPUT} w-32 sm:w-56 shrink-0 flex items-center opacity-70`}>
              <span className="truncate">{key}</span>
            </span>
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
        <div key={i} className="grid gap-1">
          <div className="flex items-center gap-2">
            <input
              value={row.key}
              disabled={row.stored || row.source !== null}
              onChange={(e) => update(i, { ...row, key: e.target.value.toUpperCase() })}
              placeholder="KEY"
              className={`${INPUT} w-32 sm:w-56 shrink-0 disabled:opacity-70`}
            />
            <input
              value={row.value ?? ""}
              onChange={(e) => update(i, { ...row, value: e.target.value })}
              placeholder={row.value === null ? "••••••••" : row.stored ? "value" : "not set"}
              className={`${INPUT} flex-1 min-w-0`}
            />
            {row.source ? (
              <Badge className="shrink-0 hidden sm:inline-flex" title={row.source}>
                {row.source}
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
          {row.about ? <p className="text-[12px] text-ink-4 pl-1">{row.about}</p> : null}
        </div>
      ))}
    </>
  );
}

const originId = (v: ManagedVar) =>
  v.kind === "owner" ? `owner:${v.database}` : v.kind === "role" ? `role:${v.database}/${v.role}` : "redis";

function describeOrigin(v: ManagedVar): string {
  if (v.kind === "redis") return "Redis";
  if (v.kind === "owner") return `owner of ${v.database}`;
  const prefix = `${v.database}_`;
  const role = v.role?.startsWith(prefix) ? v.role.slice(prefix.length) : v.role;
  return `role ${role} of ${v.database}`;
}

function labelChanges(vars: ManagedVar[], edits: Record<string, string>): LabelChanges {
  const changes: LabelChanges = {};
  for (const v of vars) {
    const label = edits[originId(v)];
    if (label === undefined || label === v.key) continue;
    if (v.kind === "owner" && v.database) changes.database = { ...changes.database, [v.database]: label };
    if (v.kind === "role" && v.database && v.role)
      changes.roles = { ...changes.roles, [`${v.database}/${v.role}`]: label };
    if (v.kind === "redis") changes.redis = label;
  }
  return changes;
}

/** Each variable Ferrum sets, with where it comes from; its name is the repo file's when the app follows one. */
function ManagedRows({ slug }: { slug: string }) {
  const { data: app } = useApp(slug);
  const vars = app?.managed_vars ?? [];
  const follows = app?.follow_repo_file ?? false;
  const save = useSetLabels(slug);
  const [edits, setEdits] = useState<Record<string, string>>({});
  const changes = labelChanges(vars, edits);
  const dirty = Object.keys(changes).length > 0;

  const submit = async () => {
    await save.mutateAsync(changes);
    setEdits({});
  };

  return (
    <>
      {vars.map((v) => (
        <div key={originId(v)} className="flex items-center gap-2 h-9">
          {follows ? (
            <span className={`${INPUT} w-32 sm:w-56 shrink-0 flex items-center opacity-70`}>
              <span className="truncate">{v.key}</span>
            </span>
          ) : (
            <input
              value={edits[originId(v)] ?? v.key}
              onChange={(e) => setEdits({ ...edits, [originId(v)]: e.target.value.toUpperCase() })}
              aria-label={`Label for ${describeOrigin(v)}`}
              className={`${INPUT} w-32 sm:w-56 shrink-0`}
            />
          )}
          <span className={`${INPUT} flex-1 min-w-0 flex items-center text-ink-4 font-sans`}>
            <span className="truncate">{describeOrigin(v)}</span>
          </span>
          <Badge tone="accent" className="shrink-0">
            {follows ? "from ferrum.toml" : "set by Ferrum"}
          </Badge>
        </div>
      ))}
      {dirty ? (
        <div className="flex items-center justify-end gap-2">
          <Button size="sm" variant="ghost" onClick={() => setEdits({})} disabled={save.isPending}>
            Cancel
          </Button>
          <Button size="sm" variant="primary" onClick={submit} disabled={save.isPending}>
            Save labels
          </Button>
        </div>
      ) : null}
      {save.error ? (
        <p className="text-[12.5px] text-fail">
          {save.error instanceof ApiError ? save.error.message : String(save.error)}
        </p>
      ) : null}
    </>
  );
}

/** Values never come back from the server: a stored row shows dots until it is edited. */
export function EnvironmentPanel({
  slug,
  entries,
  managed,
  processes,
}: {
  slug: string;
  entries: EnvEntry[];
  managed: string[];
  processes: PortedProcess[];
}) {
  const [rows, setRows] = useState<EnvRow[]>(() => rowsFromEntries(entries));
  const [dirty, setDirty] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const save = useSetEnv(slug);
  const required = rows.some((r) => r.source === "ferrum.toml");

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
    setNote(null);
  };

  return (
    <Card>
      <CardHeader
        title="Environment"
        hint="Written to shared/.env at 0600, owned by the app user, read by the unit and the build"
        action={
          <span className="flex gap-1">
            <ImportEnv
              rows={rows}
              managed={managed}
              processes={processes}
              onImport={(next, text) => {
                change(next);
                setNote(text);
              }}
            />
            <Button size="sm" variant="ghost" onClick={() => change([...rows, blankRow()])}>
              <Plus size={14} />
              Add
            </Button>
          </span>
        }
      />
      <CardBody className="grid gap-2">
        <EnvRows rows={rows} onChange={change} managed={managed} managedRows={<ManagedRows slug={slug} />} />
        {note ? <p className="text-[12.5px] text-ink-3 mt-1">{note}</p> : null}
        {required ? <p className="text-[12.5px] text-ink-4 mt-1">{REQUIRED_NOTE}</p> : null}
        {save.error ? (
          <p className="text-[12.5px] text-fail">
            {save.error instanceof ApiError ? save.error.message : String(save.error)}
          </p>
        ) : null}
      </CardBody>
      <CardFoot>
        <span>
          Your variables are set for the build too, so the <Code>NEXT_PUBLIC_*</Code> and{" "}
          <Code>VITE_*</Code> values you enter reach the client bundle. <Code>PORT</Code> is set per
          process, each listening process is also named as <Code>NAME_PORT</Code>, and{" "}
          <Code>HOST</Code> is always <Code>127.0.0.1</Code>. Import a .env to fill the rows; nothing
          is saved until you click Save.
        </span>
        <Button size="sm" variant="primary" disabled={!dirty || save.isPending} onClick={submit}>
          Save
        </Button>
      </CardFoot>
    </Card>
  );
}
