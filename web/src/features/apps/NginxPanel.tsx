import { Lock } from "lucide-react";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Code } from "@/components/ui/Code";

export function NginxPanel({ slug }: { slug: string }) {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader
          title="Managed by Ferrum"
          hint={`/etc/nginx/sites-available/ferrum-${slug}.conf`}
          action={
            <span className="inline-flex items-center gap-1.5 text-[12.5px] text-ink-4">
              <Lock size={12} />
              Read only
            </span>
          }
        />
        <CardBody>
          <pre className="bg-inset border border-line rounded-inset p-3 font-mono text-[12px] leading-relaxed text-ink-3 overflow-x-auto">
{`# managed by Ferrum — do not edit
server {
  listen 443 ssl;
  http2 on;
  server_name ledger.example.com;

  ssl_certificate     /var/lib/ferrum/certs/ledger.example.com/fullchain.pem;
  ssl_certificate_key /var/lib/ferrum/certs/ledger.example.com/key.pem;
  add_header Strict-Transport-Security "max-age=31536000";

  client_max_body_size 25m;
  include /etc/nginx/sites-available/ferrum-ledger.custom.conf;

  location / {
    proxy_pass http://127.0.0.1:41204;
    proxy_set_header Upgrade    $http_upgrade;
    proxy_set_header Connection $connection_upgrade;
    proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_read_timeout 3600s;
  }
}`}
          </pre>
        </CardBody>
        <CardFoot>
          <span>Regenerated whenever a domain, route or port changes.</span>
        </CardFoot>
      </Card>

      <Card>
        <CardHeader
          title="Your directives"
          hint={`ferrum-${slug}.custom.conf, included inside the server block`}
        />
        <CardBody>
          <textarea
            rows={16}
            spellCheck={false}
            defaultValue={`location /downloads/ {\n  alias /var/lib/ferrum/apps/ledger/shared/storage/;\n  add_header Cache-Control "public, max-age=604800";\n}`}
            className="w-full bg-inset border border-line rounded-inset p-3 font-mono text-[12px] leading-relaxed text-ink resize-y"
          />
        </CardBody>
        <CardFoot>
          <span>
            Checked with <Code>nginx -t</Code> before any reload. A failing edit is rejected with the
            error, rather than taking every site on the box down.
          </span>
          <Button size="sm" variant="primary">
            Save and reload
          </Button>
        </CardFoot>
      </Card>
    </div>
  );
}
