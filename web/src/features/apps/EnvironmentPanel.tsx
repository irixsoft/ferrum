import { useState } from "react";
import { Plus, X } from "lucide-react";
import { ApiError, useSetEnv } from "@/lib/api";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import type { EnvChange } from "@/types/api";

type Row = { key: string; value: string | null; stored: boolean };

const INPUT =
  "h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 font-mono text-[13px]";

/** Values never come back from the server: a stored row shows dots until it is edited. */
export function EnvironmentPanel({
  slug,
  keys,
  managed,
}: {
  slug: string;
  keys: string[];
  managed: string[];
}) {
  const [rows, setRows] = useState<Row[]>(() =>
    keys.map((key) => ({ key, value: null, stored: true })),
  );
  const [dirty, setDirty] = useState(false);
  const save = useSetEnv(slug);

  const update = (i: number, row: Row) => {
    setRows(rows.map((r, j) => (j === i ? row : r)));
    setDirty(true);
  };

  const submit = async () => {
    const changes: EnvChange[] = rows
      .filter((r) => r.key.trim())
      .map((r) => (r.value === null ? { key: r.key.trim() } : { key: r.key.trim(), value: r.value }));
    await save.mutateAsync(changes);
    setRows(changes.map((c) => ({ key: c.key, value: null, stored: true })));
    setDirty(false);
  };

  return (
    <Card>
      <CardHeader
        title="Environment"
        hint="Written to shared/.env at 0600, owned by the app user, read by the unit and the build"
        action={
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              setRows([...rows, { key: "", value: "", stored: false }]);
              setDirty(true);
            }}
          >
            <Plus size={14} />
            Add
          </Button>
        }
      />
      <CardBody className="grid gap-2">
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
              disabled={row.stored}
              onChange={(e) => update(i, { ...row, key: e.target.value.toUpperCase() })}
              placeholder="KEY"
              className={`${INPUT} w-56 disabled:opacity-70`}
            />
            <input
              value={row.value ?? ""}
              onChange={(e) => update(i, { ...row, value: e.target.value })}
              placeholder={row.value === null ? "••••••••" : "value"}
              className={`${INPUT} flex-1 min-w-0`}
            />
            <Button
              size="icon"
              variant="ghost"
              aria-label={`Remove ${row.key}`}
              onClick={() => {
                setRows(rows.filter((_, j) => j !== i));
                setDirty(true);
              }}
            >
              <X size={14} />
            </Button>
          </div>
        ))}
        {save.error ? (
          <p className="text-[12.5px] text-fail">
            {save.error instanceof ApiError ? save.error.message : String(save.error)}
          </p>
        ) : null}
      </CardBody>
      <CardFoot>
        <span>
          Injected at build time as well as runtime, so <Code>NEXT_PUBLIC_*</Code> and{" "}
          <Code>VITE_*</Code> reach the client bundle. <Code>PORT</Code> and <Code>HOST</Code> are
          added by Ferrum.
        </span>
        <Button size="sm" variant="primary" disabled={!dirty || save.isPending} onClick={submit}>
          Save
        </Button>
      </CardFoot>
    </Card>
  );
}
