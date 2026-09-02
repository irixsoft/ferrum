import { useEffect, useState } from "react";
import { Lock } from "lucide-react";
import { ApiError, useNginx, useSetCustomNginx } from "@/lib/api";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Code } from "@/components/ui/Code";

const message = (e: unknown) => (e instanceof ApiError ? e.message : e ? String(e) : null);

export function NginxPanel({ slug }: { slug: string }) {
  const { data: files } = useNginx(slug);
  const save = useSetCustomNginx(slug);
  const [draft, setDraft] = useState<string | null>(null);
  useEffect(() => {
    setDraft(null);
  }, [slug]);

  if (!files) return null;
  const custom = draft ?? files.custom;
  const dirty = custom !== files.custom;

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader
          title="Managed by Ferrum"
          hint={`/etc/nginx/conf.d/ferrum-${slug}.conf`}
          action={
            <span className="inline-flex items-center gap-1.5 text-[12.5px] text-ink-4">
              <Lock size={12} />
              Read only
            </span>
          }
        />
        <CardBody>
          <pre className="bg-inset border border-line rounded-inset p-3 font-mono text-[12px] leading-relaxed text-ink-3 overflow-x-auto">
            {files.managed || "Not written yet."}
          </pre>
        </CardBody>
        <CardFoot>
          <span>Regenerated whenever a domain, route or port changes.</span>
        </CardFoot>
      </Card>

      <Card>
        <CardHeader title="Your directives" hint={`/etc/nginx/ferrum-custom/${slug}.conf, included inside the server block`} />
        <CardBody>
          <textarea
            rows={16}
            spellCheck={false}
            value={custom}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={"location /downloads/ {\n  alias /var/lib/ferrum/apps/" + slug + "/shared/storage/;\n}"}
            className="w-full bg-inset border border-line rounded-inset p-3 font-mono text-[12px] leading-relaxed text-ink placeholder:text-ink-4 resize-y"
          />
          {save.error ? <p className="text-[12.5px] text-fail mt-2 font-mono whitespace-pre-wrap">{message(save.error)}</p> : null}
          {save.isSuccess && !dirty ? <p className="text-[12.5px] text-ok mt-2">Saved and reloaded.</p> : null}
        </CardBody>
        <CardFoot>
          <span>
            Checked with <Code>nginx -t</Code> before any reload. A failing edit is rejected with the
            error, rather than taking every site on the box down.
          </span>
          <Button
            size="sm"
            variant="primary"
            disabled={!dirty || save.isPending}
            onClick={async () => {
              await save.mutateAsync(custom);
              setDraft(null);
            }}
          >
            Save and reload
          </Button>
        </CardFoot>
      </Card>
    </div>
  );
}
