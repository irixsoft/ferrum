import { useState, type ReactNode } from "react";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Row } from "@/components/ui/Row";
import { Segmented } from "@/components/ui/Segmented";
import { Sheet } from "@/components/ui/Sheet";
import { useCreateDnsProvider, useDnsProviders, useRemoveDnsProvider } from "@/lib/api";
import { ago } from "@/lib/utils";
import type { DnsProvider, NewDnsProvider } from "@/types/api";

type Kind = DnsProvider["kind"];

const KINDS: Array<{ value: Kind; label: string }> = [
  { value: "cloudflare", label: "Cloudflare" },
  { value: "route53", label: "Route 53" },
];

const LABEL: Record<Kind, string> = { cloudflare: "Cloudflare", route53: "Route 53" };

const INPUT =
  "h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 w-full";

export function DnsProvidersCard() {
  const { data: providers } = useDnsProviders();
  const [adding, setAdding] = useState(false);

  return (
    <Card>
      <CardHeader
        title="DNS providers"
        hint="Wildcard certificates are proven by a TXT record at your DNS host"
        action={
          <Button size="sm" onClick={() => setAdding(true)}>
            Add provider
          </Button>
        }
      />
      <CardBody>
        {providers === undefined ? null : providers.length === 0 ? (
          <p className="text-[13.5px] text-ink-2 leading-relaxed max-w-prose">
            Add a Cloudflare token or a Route 53 key here, then choose it on a wildcard domain such
            as *.example.com.
          </p>
        ) : (
          <dl>
            {providers.map((p) => (
              <ProviderRow key={p.id} provider={p} />
            ))}
          </dl>
        )}
      </CardBody>
      <CardFoot>
        Ferrum only ever writes the one _acme-challenge record Let's Encrypt asks for, at every
        renewal, and removes it after.
      </CardFoot>
      <AddProvider open={adding} onClose={() => setAdding(false)} />
    </Card>
  );
}

function ProviderRow({ provider }: { provider: DnsProvider }) {
  const remove = useRemoveDnsProvider();
  return (
    <Row
      label={provider.name}
      hint={
        remove.error ? (
          <span className="text-fail">{remove.error.message}</span>
        ) : (
          `${LABEL[provider.kind]}, added ${ago(provider.created_at)}`
        )
      }
    >
      <Button
        size="sm"
        variant="ghost"
        disabled={remove.isPending}
        onClick={() => remove.mutate(provider.id)}
      >
        Remove
      </Button>
    </Row>
  );
}

function AddProvider({ open, onClose }: { open: boolean; onClose: () => void }) {
  const create = useCreateDnsProvider();
  const [kind, setKind] = useState<Kind>("cloudflare");
  const [name, setName] = useState("");
  const [zone, setZone] = useState("");
  const [token, setToken] = useState("");
  const [accessKeyId, setAccessKeyId] = useState("");
  const [secretAccessKey, setSecretAccessKey] = useState("");

  const close = () => {
    create.reset();
    setName("");
    setZone("");
    setToken("");
    setAccessKeyId("");
    setSecretAccessKey("");
    onClose();
  };

  const provider: NewDnsProvider =
    kind === "cloudflare"
      ? { kind, name: name.trim(), zone: zone.trim(), credentials: { token: token.trim() } }
      : {
          kind,
          name: name.trim(),
          zone: zone.trim(),
          credentials: {
            access_key_id: accessKeyId.trim(),
            secret_access_key: secretAccessKey.trim(),
          },
        };
  const complete =
    provider.name !== "" &&
    provider.zone !== "" &&
    Object.values(provider.credentials).every((v) => v !== "");

  return (
    <Sheet
      open={open}
      onClose={close}
      side="center"
      title="Add a DNS provider"
      footer={
        <div className="flex items-center justify-between gap-3">
          <span className="text-[12.5px] text-ink-4">
            Save writes and removes a test record in that zone first.
          </span>
          <Button
            variant="primary"
            disabled={!complete || create.isPending}
            onClick={() => create.mutate(provider, { onSuccess: close })}
          >
            {create.isPending ? "Testing…" : "Save"}
          </Button>
        </div>
      }
    >
      <div className="grid gap-4">
        <Segmented<Kind> options={KINDS} value={kind} onChange={setKind} className="justify-self-start" />
        <Field label="Name">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={kind === "cloudflare" ? "Cloudflare" : "AWS"}
            className={INPUT}
          />
        </Field>
        {kind === "cloudflare" ? (
          <Field label="API token" hint="Zone, DNS, Edit on the zones Ferrum will use">
            <input
              type="password"
              autoComplete="off"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              className={`${INPUT} font-mono text-[13px]`}
            />
          </Field>
        ) : (
          <>
            <Field
              label="Access key ID"
              hint="An IAM user allowed route53:ListHostedZonesByName and route53:ChangeResourceRecordSets"
            >
              <input
                autoComplete="off"
                value={accessKeyId}
                onChange={(e) => setAccessKeyId(e.target.value)}
                className={`${INPUT} font-mono text-[13px]`}
              />
            </Field>
            <Field label="Secret access key">
              <input
                type="password"
                autoComplete="off"
                value={secretAccessKey}
                onChange={(e) => setSecretAccessKey(e.target.value)}
                className={`${INPUT} font-mono text-[13px]`}
              />
            </Field>
          </>
        )}
        <Field label="Zone to test against" hint="A domain this provider serves">
          <input
            value={zone}
            onChange={(e) => setZone(e.target.value)}
            placeholder="example.com"
            className={`${INPUT} font-mono text-[13px]`}
          />
        </Field>
        {create.error ? <p className="text-[12.5px] text-fail">{create.error.message}</p> : null}
      </div>
    </Sheet>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="grid gap-1.5">
      <span className="text-[13px] text-ink-3">{label}</span>
      {children}
      {hint ? <span className="text-[12px] text-ink-4">{hint}</span> : null}
    </label>
  );
}
