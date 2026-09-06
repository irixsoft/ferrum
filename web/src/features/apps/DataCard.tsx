import { useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  ApiError,
  useCreateDatabase,
  useDatabases,
  useLinkDatabase,
  usePostgres,
  useReleaseRedis,
  useRequestRedis,
  useUnlinkDatabase,
} from "@/lib/api";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { InstallPostgres } from "@/features/databases/InstallPostgres";
import { DATABASE_NAME } from "@/features/databases/DatabasesPage";
import type { AppDetail } from "@/types/api";

const INPUT =
  "h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4";

const message = (e: unknown) => (e instanceof ApiError ? e.message : e ? String(e) : null);

export function DataCard({ app }: { app: AppDetail }) {
  const { data: postgres } = usePostgres();
  const { data: databases = [] } = useDatabases();
  const create = useCreateDatabase();
  const link = useLinkDatabase(app.slug);
  const unlink = useUnlinkDatabase(app.slug);
  const request = useRequestRedis(app.slug);
  const release = useReleaseRedis(app.slug);
  const [name, setName] = useState(`${app.slug.replace(/-/g, "_")}_prod`);
  const [creating, setCreating] = useState(false);
  const [existing, setExisting] = useState("");
  const [memory, setMemory] = useState(64);
  const unlinked = databases.filter((d) => !app.databases.includes(d.name));
  const error = message(create.error ?? link.error ?? unlink.error ?? request.error ?? release.error);

  return (
    <Card>
      <CardHeader title="Data" hint="Linked databases and Redis, injected into the env file" />
      <CardBody className="grid gap-4">
        <div>
          <p className="text-[13px] text-ink-3 mb-1.5">PostgreSQL</p>
          {app.databases.length === 0 ? (
            <p className="text-[13.5px] text-ink-3">No database linked.</p>
          ) : (
            <ul className="divide-y divide-line">
              {app.databases.map((db, i) => (
                <li key={db} className="py-2 flex items-center gap-2">
                  <Link to="/databases">
                    <Code>{db}</Code>
                  </Link>
                  <span className="font-mono text-[12px] text-ink-4 truncate">
                    {i === 0 ? "DATABASE_URL" : `${db.toUpperCase()}_DATABASE_URL`}
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="ml-auto"
                    disabled={unlink.isPending}
                    onClick={() => unlink.mutate(db)}
                  >
                    Unlink
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </div>

        {postgres && !postgres.installed ? (
          <InstallPostgres compact />
        ) : creating ? (
          <div className="flex items-center gap-2 flex-wrap">
            <input
              value={name}
              onChange={(e) => setName(e.target.value.toLowerCase())}
              className={`${INPUT} w-56 font-mono text-[13px]`}
            />
            <Button
              size="sm"
              variant="primary"
              disabled={!DATABASE_NAME.test(name) || create.isPending}
              onClick={async () => {
                await create.mutateAsync({ name, app_slug: app.slug });
                setCreating(false);
              }}
            >
              Create and link
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setCreating(false)}>
              Cancel
            </Button>
          </div>
        ) : (
          <div className="flex items-center gap-2 flex-wrap">
            <Button size="sm" onClick={() => setCreating(true)}>
              Create database
            </Button>
            {unlinked.length ? (
              <>
                <select value={existing} onChange={(e) => setExisting(e.target.value)} className={INPUT}>
                  <option value="">Link existing…</option>
                  {unlinked.map((d) => (
                    <option key={d.name} value={d.name}>
                      {d.name}
                    </option>
                  ))}
                </select>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={!existing || link.isPending}
                  onClick={async () => {
                    await link.mutateAsync(existing);
                    setExisting("");
                  }}
                >
                  Link
                </Button>
              </>
            ) : null}
          </div>
        )}

        <div className="border-t border-line pt-4">
          <p className="text-[13px] text-ink-3 mb-1.5">Redis</p>
          {app.redis ? (
            <div className="flex items-center gap-2 flex-wrap">
              <Code>ferrum-redis-{app.slug}</Code>
              <span className="font-mono text-[12px] text-ink-4">
                127.0.0.1:{app.redis.port} · {app.redis.maxmemory_mb} MB
              </span>
              <Badge tone="ok">noeviction</Badge>
              <Button
                size="sm"
                variant="ghost"
                className="ml-auto"
                disabled={release.isPending}
                onClick={() => release.mutate()}
              >
                Remove
              </Button>
            </div>
          ) : (
            <div className="flex items-center gap-2 flex-wrap">
              <label className="flex items-center gap-2 text-[13px] text-ink-3">
                <input
                  type="number"
                  min={16}
                  max={16384}
                  value={memory}
                  onChange={(e) => setMemory(Number(e.target.value))}
                  className={`${INPUT} w-24`}
                />
                MB
              </label>
              <Button size="sm" disabled={request.isPending} onClick={() => request.mutate(memory)}>
                {request.isPending ? "Starting…" : "Add Redis"}
              </Button>
            </div>
          )}
        </div>
        {error ? <p className="text-[12.5px] text-fail">{error}</p> : null}
      </CardBody>
      <CardFoot>
        <span>
          <Code>DATABASE_URL</Code> and <Code>REDIS_URL</Code> are rewritten into{" "}
          <Code>shared/.env</Code> on every change. Unlinking never deletes a database.
        </span>
      </CardFoot>
    </Card>
  );
}
