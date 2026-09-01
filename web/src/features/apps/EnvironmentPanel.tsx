import { useState } from "react";
import { Eye, EyeOff, Pencil, Plus } from "lucide-react";
import { useShell } from "@/shells/useShell";
import { Card, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Sheet } from "@/components/ui/Sheet";

/** The reference pattern for routes, environment variables and system packages. */

type Entry = { key: string; value: string; managed?: string };

const ENTRIES: Entry[] = [
  { key: "DATABASE_URL", value: "postgres://ledger:••••••••@127.0.0.1:5432/ledger_prod", managed: "linked database" },
  { key: "NODE_ENV", value: "production" },
  { key: "NEXT_PUBLIC_API_URL", value: "https://ledger.example.com/api" },
  { key: "STRIPE_SECRET_KEY", value: "sk_live_••••••••••••••••••••" },
  { key: "SENTRY_DSN", value: "https://••••••@o41.ingest.sentry.io/4501" },
  { key: "S3_BUCKET", value: "ledger-statements-eu" },
];

export function EnvironmentPanel() {
  const { shell } = useShell();
  const [revealed, setRevealed] = useState(false);
  const [editing, setEditing] = useState<Entry | null>(null);

  const show = (e: Entry) => (revealed ? e.value : e.value.replace(/[^:@/.\s]/g, "•").slice(0, 42));

  return (
    <>
      <Card>
        <CardHeader
          title="Environment"
          hint="Encrypted at rest, written to shared/.env at 0600, owned by the app user"
          action={
            <>
              <Button size="sm" variant="ghost" onClick={() => setRevealed((v) => !v)}>
                {revealed ? <EyeOff size={14} /> : <Eye size={14} />}
                {revealed ? "Hide values" : "Reveal values"}
              </Button>
              <Button size="sm" variant="primary">
                <Plus size={14} />
                Add
              </Button>
            </>
          }
        />

        {shell === "desktop" ? (
          <table className="w-full text-left">
            <thead>
              <tr className="border-y border-line text-[12px] text-ink-3">
                <th className="font-medium px-5 py-2 w-64">Key</th>
                <th className="font-medium px-3 py-2">Value</th>
                <th className="px-5 py-2" />
              </tr>
            </thead>
            <tbody>
              {ENTRIES.map((e) => (
                <tr key={e.key} className="border-b border-line last:border-0 group">
                  <td className="px-5 py-2.5">
                    <span className="font-mono text-[13px] text-ink">{e.key}</span>
                    {e.managed ? (
                      <Badge tone="accent" className="ml-2">
                        set by Ferrum
                      </Badge>
                    ) : null}
                  </td>
                  <td className="px-3 py-2.5 font-mono text-[12.5px] text-ink-3 truncate max-w-md">
                    {show(e)}
                  </td>
                  <td className="px-5 py-2.5 text-right">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                      onClick={() => setEditing(e)}
                    >
                      <Pencil size={13} />
                      Edit
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <div className="px-4 pb-4 grid gap-2">
            {ENTRIES.map((e) => (
              <button
                key={e.key}
                onClick={() => setEditing(e)}
                className="text-left bg-inset border border-line rounded-inset p-3 active:border-line-strong"
              >
                <div className="flex items-center gap-2">
                  <span className="font-mono text-[13px] text-ink truncate">{e.key}</span>
                  {e.managed ? <Badge tone="accent">Ferrum</Badge> : null}
                </div>
                <p className="font-mono text-[12px] text-ink-3 mt-1 truncate">{show(e)}</p>
              </button>
            ))}
          </div>
        )}

        <CardFoot>
          <span>
            Injected at build time as well as runtime, so <Code>NEXT_PUBLIC_*</Code> and{" "}
            <Code>VITE_*</Code> reach the client bundle.
          </span>
        </CardFoot>
      </Card>

      <Sheet
        open={editing !== null}
        onClose={() => setEditing(null)}
        side={shell === "desktop" ? "center" : "bottom"}
        title={editing?.key ?? ""}
        footer={
          <div className="flex gap-2">
            <Button variant="ghost" className="flex-1" onClick={() => setEditing(null)}>
              Cancel
            </Button>
            <Button variant="primary" className="flex-1" onClick={() => setEditing(null)}>
              Save changes
            </Button>
          </div>
        }
      >
        <label className="block text-[13px] text-ink-3 mb-1.5">Value</label>
        <textarea
          defaultValue={editing?.value}
          rows={4}
          className="w-full bg-inset border border-line rounded-inset px-3 py-2 font-mono text-[13px] text-ink resize-none"
        />
        <p className="text-[12.5px] text-ink-4 mt-3">
          The app restarts when you save. A deploy is not required.
        </p>
      </Sheet>
    </>
  );
}
