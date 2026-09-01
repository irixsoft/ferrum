import { Plus, Terminal } from "lucide-react";
import { useDatabases } from "@/lib/api";
import { PageTitle } from "@/components/PageTitle";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Meter } from "@/components/ui/Meter";
import { Row } from "@/components/ui/Row";
import { bytes, pct } from "@/lib/utils";

export function DatabasesPage() {
  const { data } = useDatabases();
  if (!data) return null;

  return (
    <>
      <PageTitle
        above="One PostgreSQL cluster, one Redis instance per app that asks for one"
        title="Databases"
        action={
          <Button variant="primary">
            <Plus size={15} />
            New database
          </Button>
        }
      />

      <div className="grid gap-4">
        {data.databases.map((db) => {
          const conns = pct(db.connections_active, db.connection_limit);
          return (
            <Card key={db.name}>
              <CardHeader
                title={db.name}
                hint={`Role ${db.role} · ${bytes(db.size_bytes)}`}
                action={
                  <>
                    <Button size="sm" variant="ghost">
                      <Terminal size={14} />
                      Connect
                    </Button>
                    <Button size="sm">Manage</Button>
                  </>
                }
              />
              <CardBody>
                <div className="grid gap-5 sm:grid-cols-2">
                  <div>
                    <div className="flex items-baseline justify-between mb-1.5">
                      <span className="text-[13px] text-ink-3">Connections</span>
                      <span className="font-mono text-[12.5px] text-ink-4 tnum">
                        {db.connections_active} of {db.connection_limit}
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
                        db.linked_apps.map((a) => <Code key={a}>{a}</Code>)
                      ) : (
                        <span className="text-ink-4">Nothing</span>
                      )}
                    </Row>
                    <Row label="Extensions">
                      <span className="flex flex-wrap gap-1 justify-end">
                        {db.extensions.map((e) => (
                          <Code key={e}>{e}</Code>
                        ))}
                      </span>
                    </Row>
                  </dl>
                </div>
              </CardBody>
              <CardFoot>
                <span>
                  Reachable only over loopback. For a client on your machine, tunnel it:{" "}
                  <Code>ssh -L 5432:127.0.0.1:5432 root@panel.example.com</Code>
                </span>
              </CardFoot>
            </Card>
          );
        })}

        <div className="mt-2">
          <h2 className="font-display text-[22px] text-ink mb-3">Redis</h2>
          <div className="grid gap-3 sm:grid-cols-2">
            {data.redis.map((r) => {
              const used = pct(r.used_memory_mb, r.maxmemory_mb);
              return (
                <Card key={r.slug}>
                  <CardHeader
                    title={`ferrum-redis-${r.slug}`}
                    hint={`Port ${r.port} · used by ${r.app_slug}`}
                  />
                  <CardBody>
                    <div className="flex items-baseline justify-between mb-1.5">
                      <span className="text-[13px] text-ink-3">Memory</span>
                      <span className="font-mono text-[12.5px] text-ink-4 tnum">
                        {r.used_memory_mb} / {r.maxmemory_mb} MB
                      </span>
                    </div>
                    <Meter value={used} tone={used > 85 ? "fail" : used > 70 ? "run" : "neutral"} />
                    <div className="flex gap-2 mt-4">
                      <Badge tone="ok">{r.maxmemory_policy}</Badge>
                      {r.appendonly ? <Badge tone="ok">AOF on</Badge> : <Badge tone="fail">No AOF</Badge>}
                    </div>
                    <p className="text-[12.5px] text-ink-4 mt-3 leading-relaxed">
                      Writes fail loudly when this instance is full, instead of quietly evicting
                      queued jobs. Restarting it does not touch any other app.
                    </p>
                  </CardBody>
                </Card>
              );
            })}
          </div>
        </div>
      </div>
    </>
  );
}
