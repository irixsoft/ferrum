import { useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import {
  ApiError,
  useApps,
  useCreateDatabase,
  useCreateFromDump,
  useCreateRole,
  useDatabases,
  useDeleteDatabase,
  useEnableExtension,
  usePostgres,
  useRedisInstances,
  useRemoveRole,
  useRoleUrl,
  useRoles,
  useRotateRole,
} from "@/lib/api";
import { GZIP_REFUSED, describeDump, sniffFile, type DumpFormat } from "@/lib/dump";
import { Handoff } from "@/components/Handoff";
import { PageTitle } from "@/components/PageTitle";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Meter } from "@/components/ui/Meter";
import { Row } from "@/components/ui/Row";
import { EmptyState } from "@/components/ui/EmptyState";
import { ExtensionPicker } from "./ExtensionPicker";
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

        {creating ? <CreateDatabase onDone={() => setCreating(false)} /> : null}

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
          <DatabaseCard key={db.name} db={db} tunnel={postgres.tunnel} tunnelUser={postgres.tunnel_user} />
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

function CreateDatabase({ onDone }: { onDone: () => void }) {
  const { data: apps = [] } = useApps();
  const create = useCreateDatabase();
  const fromDump = useCreateFromDump();
  const picker = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");
  const [limit, setLimit] = useState(20);
  const [picked, setPicked] = useState<string[]>([]);
  const [appSlug, setAppSlug] = useState("");
  const [dump, setDump] = useState<File | null>(null);
  const [format, setFormat] = useState<DumpFormat | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const valid = DATABASE_NAME.test(name);
  const busy = create.isPending || fromDump.isPending;

  const pick = async (file: File | null) => {
    setDump(file);
    setFormat(file ? await sniffFile(file) : null);
  };

  const submit = async () => {
    if (dump) {
      setProgress(0);
      try {
        await fromDump.mutateAsync({
          name,
          file: dump,
          connection_limit: limit,
          extensions: picked,
          app_slug: appSlug || undefined,
          onProgress: setProgress,
        });
      } finally {
        setProgress(null);
      }
    } else {
      await create.mutateAsync({
        name,
        connection_limit: limit,
        extensions: picked,
        app_slug: appSlug || undefined,
      });
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
            <ExtensionPicker
              picked={picked}
              onPick={(ext) =>
                setPicked(picked.includes(ext) ? picked.filter((p) => p !== ext) : [...picked, ext])
              }
            />
          </div>
          <div className="grid gap-1.5">
            <span className="text-[13px] text-ink-3">Create it from a dump</span>
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
                <span className="text-[12.5px] text-ink-4">
                  Optional. pg_dump custom format or plain SQL from any PostgreSQL host; the
                  extensions it needs are turned on for you.
                </span>
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
        {create.error || fromDump.error ? (
          <p className="text-[12.5px] text-fail">{message(create.error ?? fromDump.error)}</p>
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
            {progress !== null ? "Uploading…" : dump ? "Create from dump" : "Create"}
          </Button>
        </span>
      </CardFoot>
    </Card>
  );
}

function DatabaseCard({ db, tunnel, tunnelUser }: { db: Database; tunnel: string; tunnelUser: string }) {
  const [confirm, setConfirm] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const remove = useDeleteDatabase();
  const enable = useEnableExtension();
  const active = db.connections_active;
  const conns = active === null ? 0 : pct(active, db.connection_limit);

  return (
    <Card>
      <CardHeader
        title={db.name}
        hint={`Role ${db.role} · ${db.size_bytes === null ? "size unknown" : bytes(db.size_bytes)}`}
        action={
          confirm === null ? (
            <Button
              size="sm"
              variant="ghost"
              disabled={db.linked_apps.length > 0 || db.restore.running}
              title={db.linked_apps.length ? `Linked to ${db.linked_apps.join(", ")}; unlink first` : undefined}
              onClick={() => setConfirm("")}
            >
              Delete
            </Button>
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
                {db.extensions.length === 0 ? <span className="text-ink-4">None</span> : null}
                <button
                  type="button"
                  onClick={() => setAdding(!adding)}
                  className="font-mono text-[12px] text-ink-4 border border-dashed border-line-strong rounded px-1.5 py-0.5 hover:text-ink"
                >
                  {adding ? "close" : "+ add"}
                </button>
              </span>
            </Row>
          </dl>
          {adding ? (
            <div className="mt-2">
              <ExtensionPicker
                picked={[]}
                hidden={db.extensions}
                disabled={enable.isPending}
                onPick={(extension) => enable.mutate({ database: db.name, extension })}
              />
            </div>
          ) : null}
        </div>
        {remove.error || enable.error ? (
          <p className="text-[12.5px] text-fail mt-3">{message(remove.error ?? enable.error)}</p>
        ) : null}
        {db.restore.running ? (
          <p className="text-[12.5px] text-ink-3 mt-3">Loading the dump into {db.name}…</p>
        ) : db.restore.error ? (
          <p className="text-[12.5px] text-fail mt-3">
            The dump failed to load: {db.restore.error} Delete this database and create it again
            from a corrected dump.
          </p>
        ) : null}
        <Roles database={db.name} linked={db.linked_apps.length > 0} />
      </CardBody>
      <CardFoot className="block">
        <p>
          Reachable only over loopback. For a client on your machine, tunnel it
          {tunnelUser ? ":" : ", after setting your SSH login under Settings › Connections:"}
        </p>
        <div className="flex items-center gap-2 mt-2">
          <code className="flex-1 min-w-0 font-mono text-[12px] text-ink break-all">{tunnel}</code>
          <Button size="sm" onClick={() => navigator.clipboard?.writeText(tunnel)}>
            Copy
          </Button>
        </div>
        <p className="mt-2">
          If Postgres already listens on 5432 on your machine, use <Code>-L 15432:127.0.0.1:5432</Code>{" "}
          and connect to port 15432.
        </p>
      </CardFoot>
    </Card>
  );
}

const ROLE_NAME = /^[a-z][a-z0-9_]{0,62}$/;
const LABEL = /^[A-Za-z_][A-Za-z0-9_]*$/;

function Roles({ database, linked }: { database: string; linked: boolean }) {
  const { data: roles = [] } = useRoles(database);
  const create = useCreateRole(database);
  const remove = useRemoveRole(database);
  const rotate = useRotateRole(database);
  const reveal = useRoleUrl(database);
  const [shown, setShown] = useState<{ role: string; url: string } | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [rotated, setRotated] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [label, setLabel] = useState("");
  const [limit, setLimit] = useState(20);
  const validName = ROLE_NAME.test(name) && database.length + 1 + name.length <= 63;
  const validLabel = label === "" || LABEL.test(label);
  const failed = create.error ?? remove.error ?? rotate.error ?? reveal.error;

  const add = async () => {
    await create.mutateAsync({ name, env_label: label || undefined, connection_limit: limit });
    setName("");
    setLabel("");
    setLimit(20);
    setAdding(false);
  };

  return (
    <div className="mt-5">
      <div className="flex items-baseline justify-between mb-2">
        <span className="text-[13px] text-ink-3">Roles</span>
        <button
          type="button"
          onClick={() => setAdding(!adding)}
          className="font-mono text-[12px] text-ink-4 border border-dashed border-line-strong rounded px-1.5 py-0.5 hover:text-ink"
        >
          {adding ? "close" : "+ add role"}
        </button>
      </div>
      {shown ? (
        <Handoff label={`Connection URL for ${shown.role}`} value={shown.url} onDone={() => setShown(null)} />
      ) : null}
      <ul className="border border-line rounded-inset divide-y divide-line">
        {roles.map((role) => (
          <li key={role.name} className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3 py-2.5">
            <span className="flex items-center gap-2 min-w-0 flex-1">
              <span className="font-mono text-[13px] text-ink truncate">{role.name}</span>
              {role.owner ? <Badge>owner</Badge> : null}
              {role.bypass_rls ? <Badge>bypasses RLS</Badge> : null}
            </span>
            <Code>{role.env_label}</Code>
            <span className="text-[12.5px] text-ink-4 tnum">{role.connection_limit} connections</span>
            <span className="flex gap-1">
              {removing === role.name ? (
                <>
                  <Button size="sm" variant="ghost" onClick={() => setRemoving(null)}>
                    Cancel
                  </Button>
                  <Button
                    size="sm"
                    variant="danger"
                    disabled={remove.isPending}
                    onClick={() => remove.mutate(role.name, { onSuccess: () => setRemoving(null) })}
                  >
                    Remove {role.name}
                  </Button>
                </>
              ) : (
                <>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={reveal.isPending}
                    onClick={() =>
                      reveal.mutate(role.name, { onSuccess: ({ url }) => setShown({ role: role.name, url }) })
                    }
                  >
                    Copy URL
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={rotate.isPending}
                    onClick={() =>
                      rotate.mutate(role.name, {
                        onSuccess: () => {
                          setShown(null);
                          setRotated(role.name);
                        },
                      })
                    }
                  >
                    Rotate
                  </Button>
                  {role.owner ? null : (
                    <Button size="sm" variant="ghost" onClick={() => setRemoving(role.name)}>
                      Remove
                    </Button>
                  )}
                </>
              )}
            </span>
          </li>
        ))}
        {adding ? (
          <li className="flex flex-wrap items-center gap-2 px-3 py-2.5">
            <span className="flex items-center font-mono text-[13px] text-ink-4">{database}_</span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value.toLowerCase())}
              placeholder="app"
              aria-label="Role name"
              className={`${INPUT} w-28 font-mono text-[13px]`}
            />
            <input
              value={label}
              onChange={(e) => setLabel(e.target.value.toUpperCase())}
              placeholder={name ? `DATABASE_URL_${name.toUpperCase()}` : "DATABASE_URL_APP"}
              aria-label="Env label"
              className={`${INPUT} flex-1 min-w-40 font-mono text-[13px]`}
            />
            <input
              type="number"
              min={1}
              max={500}
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
              aria-label="Connection limit"
              className={`${INPUT} w-20`}
            />
            <Button size="sm" variant="primary" disabled={!validName || !validLabel || create.isPending} onClick={add}>
              Add
            </Button>
          </li>
        ) : null}
      </ul>
      {rotated ? (
        <p className="text-[12.5px] text-ink-3 mt-2">
          {rotated} has a new password.
          {linked
            ? " The linked apps have it in their env file; restart them, since a new connection with the old one is refused."
            : null}
        </p>
      ) : null}
      {failed ? <p className="text-[12.5px] text-fail mt-2">{message(failed)}</p> : null}
      <p className="text-[12.5px] text-ink-4 mt-2 leading-relaxed">
        A role other than the owner can connect and nothing more. Grant it what it needs from a
        migration that runs as the owner, and every app linked here gets its URL under its label.
      </p>
    </div>
  );
}
