import { useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import {
  ApiError,
  useApps,
  useCreateDatabase,
  useDatabases,
  useDeleteDatabase,
  useEnableExtension,
  usePostgres,
  useRedisInstances,
  useRestoreDatabase,
} from "@/lib/api";
import { GZIP_REFUSED, describeDump, sniffFile, type DumpFormat } from "@/lib/dump";
import { PageTitle } from "@/components/PageTitle";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Meter } from "@/components/ui/Meter";
import { Row } from "@/components/ui/Row";
import { EmptyState } from "@/components/ui/EmptyState";
import { InstallPostgres } from "./InstallPostgres";
import { bytes, pct } from "@/lib/utils";
import type { Database } from "@/types/api";

export const DATABASE_NAME = /^[a-z][a-z0-9_]{0,62}$/;

const INPUT =
  "h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4";

const message = (e: unknown) => (e instanceof ApiError ? e.message : e ? String(e) : null);

export function DatabasesPage() {
  const { data: postgres } = usePostgres();
  const { data: databases } = useDatabases();
  const { data: redis = [] } = useRedisInstances();
  const [creating, setCreating] = useState(false);
  if (!postgres || !databases) return null;

  return (
    <>
      <PageTitle
        above={
          postgres.installed
            ? `One PostgreSQL ${postgres.major ?? ""} cluster, one Redis instance per app that asks for one`
            : "One PostgreSQL cluster, one Redis instance per app that asks for one"
        }
        title="Databases"
        action={
          postgres.installed ? (
            <Button variant="primary" onClick={() => setCreating(true)}>
              <Plus size={15} />
              New database
            </Button>
          ) : null
        }
      />

      <div className="grid gap-4">
        {postgres.installed ? null : (
          <Card>
            <CardHeader title="PostgreSQL" hint="Installed on first use, then pinned to that major" />
            <CardBody>
              <InstallPostgres />
            </CardBody>
          </Card>
        )}

        {creating ? (
          <CreateDatabase extensions={postgres.extensions} onDone={() => setCreating(false)} />
        ) : null}

        {postgres.installed && databases.length === 0 && !creating ? (
          <Card>
            <EmptyState
              title="No databases yet"
              body="Each database gets its own role and password, and no other role can connect to it."
              action={
                <Button variant="primary" onClick={() => setCreating(true)}>
                  <Plus size={15} />
                  New database
                </Button>
              }
            />
          </Card>
        ) : null}

        {databases.map((db) => (
          <DatabaseCard key={db.name} db={db} extensions={postgres.extensions} tunnel={postgres.tunnel} />
        ))}

        {redis.length ? (
          <div className="mt-2">
            <h2 className="font-display text-[22px] text-ink mb-3">Redis</h2>
            <div className="grid gap-3 sm:grid-cols-2">
              {redis.map((r) => (
                <Card key={r.app_slug}>
                  <CardHeader
                    title={`ferrum-redis-${r.app_slug}`}
                    hint={`127.0.0.1:${r.port} · ${r.maxmemory_mb} MB`}
                    action={
                      <Link to="/apps/$slug" params={{ slug: r.app_slug }}>
                        <Button size="sm" variant="ghost">
                          Open app
                        </Button>
                      </Link>
                    }
                  />
                  <CardBody>
                    <div className="flex gap-2">
                      <Badge tone="ok">noeviction</Badge>
                      <Badge tone="ok">AOF on</Badge>
                      <Badge>password</Badge>
                    </div>
                    <p className="text-[12.5px] text-ink-4 mt-3 leading-relaxed">
                      Writes fail loudly when this instance is full, instead of quietly evicting
                      queued jobs. Restarting it does not touch any other app.
                    </p>
                  </CardBody>
                </Card>
              ))}
            </div>
          </div>
        ) : null}
      </div>
    </>
  );
}

function CreateDatabase({
  extensions,
  onDone,
}: {
  extensions: string[];
  onDone: () => void;
}) {
  const { data: apps = [] } = useApps();
  const create = useCreateDatabase();
  const restore = useRestoreDatabase();
  const picker = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");
  const [limit, setLimit] = useState(20);
  const [picked, setPicked] = useState<string[]>([]);
  const [appSlug, setAppSlug] = useState("");
  const [dump, setDump] = useState<File | null>(null);
  const [format, setFormat] = useState<DumpFormat | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const valid = DATABASE_NAME.test(name);
  const busy = create.isPending || restore.isPending;

  const pick = async (file: File | null) => {
    setDump(file);
    setFormat(file ? await sniffFile(file) : null);
  };

  const submit = async () => {
    await create.mutateAsync({
      name,
      connection_limit: limit,
      extensions: picked,
      app_slug: appSlug || undefined,
    });
    if (dump) {
      setProgress(0);
      try {
        await restore.mutateAsync({ name, file: dump, onProgress: setProgress });
      } finally {
        setProgress(null);
      }
    }
    onDone();
  };

  return (
    <Card>
      <CardHeader
        title="New database"
        hint="A role with the same name, a generated password, and CONNECT revoked from everyone else"
      />
      <CardBody className="grid gap-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <label className="grid gap-1.5">
            <span className="text-[13px] text-ink-3">Name</span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value.toLowerCase())}
              placeholder="ledger_prod"
              className={`${INPUT} font-mono text-[13px]`}
            />
            {name && !valid ? (
              <span className="text-[12px] text-fail">
                Lowercase letters, digits and underscores, starting with a letter.
              </span>
            ) : null}
          </label>
          <label className="grid gap-1.5">
            <span className="text-[13px] text-ink-3">Connection limit</span>
            <input
              type="number"
              min={1}
              max={500}
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
              className={INPUT}
            />
          </label>
          <label className="grid gap-1.5">
            <span className="text-[13px] text-ink-3">Link to an app</span>
            <select value={appSlug} onChange={(e) => setAppSlug(e.target.value)} className={INPUT}>
              <option value="">Not now</option>
              {apps.map((a) => (
                <option key={a.slug} value={a.slug}>
                  {a.name}
                </option>
              ))}
            </select>
          </label>
          <div className="grid gap-1.5">
            <span className="text-[13px] text-ink-3">Extensions</span>
            <div className="flex flex-wrap gap-2">
              {extensions.map((ext) => {
                const on = picked.includes(ext);
                return (
                  <button
                    key={ext}
                    type="button"
                    onClick={() => setPicked(on ? picked.filter((p) => p !== ext) : [...picked, ext])}
                    className={`h-8 px-3 rounded-control border text-[13px] font-mono ${
                      on ? "bg-ink text-canvas border-ink" : "bg-inset text-ink-2 border-line-strong"
                    }`}
                  >
                    {ext}
                  </button>
                );
              })}
            </div>
          </div>
          <div className="grid gap-1.5">
            <span className="text-[13px] text-ink-3">Restore a dump into it</span>
            <input
              ref={picker}
              type="file"
              hidden
              onChange={(e) => pick(e.target.files?.[0] ?? null)}
            />
            <div className="flex items-center gap-2 flex-wrap">
              <Button size="sm" variant="ghost" onClick={() => picker.current?.click()}>
                {dump ? "Choose another" : "Choose a file"}
              </Button>
              {dump && format ? (
                <span className="text-[12.5px] text-ink-4 font-mono">
                  {dump.name} · {bytes(dump.size)} · {describeDump(format)}
                </span>
              ) : (
                <span className="text-[12.5px] text-ink-4">Optional. pg_dump custom format or plain SQL.</span>
              )}
            </div>
            {format === "gzip" ? <span className="text-[12px] text-fail">{GZIP_REFUSED}</span> : null}
          </div>
        </div>
        {progress !== null ? (
          <div className="grid gap-1.5">
            <Meter value={progress * 100} tone="run" />
            <span className="text-[12.5px] text-ink-4">Uploading {dump?.name}… {Math.round(progress * 100)}%</span>
          </div>
        ) : null}
        {create.error || restore.error ? (
          <p className="text-[12.5px] text-fail">{message(create.error ?? restore.error)}</p>
        ) : null}
      </CardBody>
      <CardFoot className="justify-end">
        <span className="flex gap-2">
          <Button size="sm" variant="ghost" onClick={onDone} disabled={busy}>
            Cancel
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={!valid || busy || format === "gzip"}
            onClick={submit}
          >
            {progress !== null ? "Uploading…" : dump ? "Create and restore" : "Create"}
          </Button>
        </span>
      </CardFoot>
    </Card>
  );
}

function DatabaseCard({
  db,
  extensions,
  tunnel,
}: {
  db: Database;
  extensions: string[];
  tunnel: string;
}) {
  const [confirm, setConfirm] = useState<string | null>(null);
  const [restoring, setRestoring] = useState(false);
  const remove = useDeleteDatabase();
  const enable = useEnableExtension();
  const active = db.connections_active;
  const conns = active === null ? 0 : pct(active, db.connection_limit);
  const missing = extensions.filter((e) => !db.extensions.includes(e));

  return (
    <Card>
      <CardHeader
        title={db.name}
        hint={`Role ${db.role} · ${db.size_bytes === null ? "size unknown" : bytes(db.size_bytes)}`}
        action={
          confirm === null ? (
            <>
              <Button
                size="sm"
                variant="ghost"
                disabled={restoring || db.restore.running}
                onClick={() => setRestoring(true)}
              >
                Restore
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={db.linked_apps.length > 0}
                title={db.linked_apps.length ? `Linked to ${db.linked_apps.join(", ")}; unlink first` : undefined}
                onClick={() => setConfirm("")}
              >
                Delete
              </Button>
            </>
          ) : (
            <>
              <input
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                placeholder={db.name}
                className={`${INPUT} w-44 font-mono text-[13px]`}
              />
              <Button size="sm" variant="ghost" onClick={() => setConfirm(null)}>
                Cancel
              </Button>
              <Button
                size="sm"
                variant="danger"
                disabled={confirm !== db.name || remove.isPending}
                onClick={() => remove.mutate(db.name)}
              >
                Delete
              </Button>
            </>
          )
        }
      />
      <CardBody>
        <div className="grid gap-5 sm:grid-cols-2">
          <div>
            <div className="flex items-baseline justify-between mb-1.5">
              <span className="text-[13px] text-ink-3">Connections</span>
              <span className="font-mono text-[12.5px] text-ink-4 tnum">
                {active === null ? "—" : active} of {db.connection_limit}
              </span>
            </div>
            <Meter value={conns} tone={conns > 80 ? "run" : "neutral"} />
            <p className="text-[12.5px] text-ink-4 mt-2">
              The limit is per role, so one leaking app cannot exhaust the cluster.
            </p>
          </div>
          <dl>
            <Row label="Linked to">
              {db.linked_apps.length ? (
                <span className="flex flex-wrap gap-1 justify-end">
                  {db.linked_apps.map((a) => (
                    <Link key={a} to="/apps/$slug" params={{ slug: a }}>
                      <Code>{a}</Code>
                    </Link>
                  ))}
                </span>
              ) : (
                <span className="text-ink-4">Nothing</span>
              )}
            </Row>
            <Row label="Extensions">
              <span className="flex flex-wrap gap-1 justify-end">
                {db.extensions.map((e) => (
                  <Code key={e}>{e}</Code>
                ))}
                {missing.map((e) => (
                  <button
                    key={e}
                    type="button"
                    disabled={enable.isPending}
                    onClick={() => enable.mutate({ database: db.name, extension: e })}
                    className="font-mono text-[12px] text-ink-4 border border-dashed border-line-strong rounded px-1.5 py-0.5 hover:text-ink"
                  >
                    + {e}
                  </button>
                ))}
              </span>
            </Row>
          </dl>
        </div>
        {remove.error || enable.error ? (
          <p className="text-[12.5px] text-fail mt-3">{message(remove.error ?? enable.error)}</p>
        ) : null}
        {db.restore.running ? (
          <p className="text-[12.5px] text-ink-3 mt-3">Restoring the dump… everything in {db.name} is being replaced.</p>
        ) : db.restore.error ? (
          <p className="text-[12.5px] text-fail mt-3">The last restore failed: {db.restore.error}</p>
        ) : null}
        {restoring ? <RestoreDump db={db} onClose={() => setRestoring(false)} /> : null}
      </CardBody>
      <CardFoot>
        <span>
          Reachable only over loopback. For a client on your machine, tunnel it: <Code>{tunnel}</Code>
        </span>
      </CardFoot>
    </Card>
  );
}

function RestoreDump({ db, onClose }: { db: Database; onClose: () => void }) {
  const restore = useRestoreDatabase();
  const picker = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [format, setFormat] = useState<DumpFormat | null>(null);
  const [typed, setTyped] = useState("");
  const [progress, setProgress] = useState<number | null>(null);

  const pick = async (picked: File | null) => {
    setFile(picked);
    setFormat(picked ? await sniffFile(picked) : null);
    setTyped("");
  };

  const start = async () => {
    if (!file) return;
    setProgress(0);
    try {
      await restore.mutateAsync({ name: db.name, file, onProgress: setProgress });
      onClose();
    } finally {
      setProgress(null);
    }
  };

  return (
    <div className="mt-4 pt-4 border-t border-line grid gap-3">
      <input ref={picker} type="file" hidden onChange={(e) => pick(e.target.files?.[0] ?? null)} />
      <div className="flex items-center gap-2 flex-wrap">
        <Button size="sm" variant="ghost" disabled={progress !== null} onClick={() => picker.current?.click()}>
          {file ? "Choose another" : "Choose a dump"}
        </Button>
        {file && format ? (
          <span className="text-[12.5px] text-ink-4 font-mono">
            {file.name} · {bytes(file.size)} · {describeDump(format)}
          </span>
        ) : (
          <span className="text-[12.5px] text-ink-4">
            pg_dump custom format or plain SQL, from any PostgreSQL host.
          </span>
        )}
      </div>
      {format === "gzip" ? <p className="text-[12.5px] text-fail">{GZIP_REFUSED}</p> : null}
      {file && format && format !== "gzip" ? (
        <div className="flex items-center gap-2 flex-wrap">
          <span className="text-[12.5px] text-ink-3">Type {db.name} to replace everything in it.</span>
          <input
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            placeholder={db.name}
            disabled={progress !== null}
            className={`${INPUT} w-44 font-mono text-[13px]`}
          />
        </div>
      ) : null}
      {progress !== null ? (
        <div className="grid gap-1.5">
          <Meter value={progress * 100} tone="run" />
          <span className="text-[12.5px] text-ink-4">Uploading… {Math.round(progress * 100)}%</span>
        </div>
      ) : null}
      {restore.error ? <p className="text-[12.5px] text-fail">{message(restore.error)}</p> : null}
      <div className="flex gap-2 justify-end">
        <Button size="sm" variant="ghost" disabled={progress !== null} onClick={onClose}>
          Cancel
        </Button>
        <Button
          size="sm"
          variant="danger"
          disabled={!file || format === "gzip" || typed !== db.name || restore.isPending}
          onClick={start}
        >
          Replace contents
        </Button>
      </div>
    </div>
  );
}
