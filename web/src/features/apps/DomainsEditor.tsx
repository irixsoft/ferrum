import { useState } from "react";
import { X } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { Segmented } from "@/components/ui/Segmented";
import { useDnsProviders } from "@/lib/api";
import type { Domain, DomainJob } from "@/types/api";

export type DomainDraft = Omit<Domain, "wildcard">;

const INPUT =
  "h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 disabled:opacity-50";
const MONO = `${INPUT} font-mono text-[13px]`;

const JOBS: Array<{ value: DomainJob; label: string }> = [
  { value: "serve", label: "Serve" },
  { value: "redirect", label: "Redirect" },
];

export const isWildcard = (name: string) => name.startsWith("*.");

const redirectTargets = (rows: DomainDraft[], self: string) =>
  rows.filter((r) => r.job === "serve" && r.domain !== self && !isWildcard(r.domain)).map((r) => r.domain);

/** Hands the primary to the first served name when the row holding it stops serving or goes. */
function keepPrimary(rows: DomainDraft[]): DomainDraft[] {
  if (rows.some((r) => r.primary && r.job === "serve")) return rows;
  const first = rows.findIndex((r) => r.job === "serve" && !isWildcard(r.domain));
  return rows.map((r, i) => ({ ...r, primary: i === first }));
}

export function DomainsEditor({
  domains,
  processes,
  onChange,
}: {
  domains: DomainDraft[];
  processes: string[];
  onChange: (domains: DomainDraft[]) => void;
}) {
  const [pending, setPending] = useState("");
  const providers = useDnsProviders().data ?? [];

  const update = (index: number, row: DomainDraft) =>
    onChange(keepPrimary(domains.map((r, i) => (i === index ? row : r))));

  const add = () => {
    const domain = pending.trim().toLowerCase();
    if (!domain || domains.some((r) => r.domain === domain)) return;
    const row: DomainDraft = {
      domain,
      job: "serve",
      target: processes[0] ?? "web",
      primary: false,
      dns_provider_id: null,
    };
    onChange(keepPrimary([...domains, row]));
    setPending("");
  };

  const setJob = (index: number, job: DomainJob) => {
    const row = domains[index];
    const target = job === "serve" ? (processes[0] ?? "web") : (redirectTargets(domains, row.domain)[0] ?? "");
    update(index, { ...row, job, target, primary: job === "serve" && row.primary });
  };

  return (
    <div className="grid gap-3">
      {domains.length ? (
        <ul className="divide-y divide-line">
          {domains.map((row, i) => {
            const targets = row.job === "serve" ? processes : redirectTargets(domains, row.domain);
            return (
              <li key={row.domain} className="py-2.5 flex flex-wrap items-center gap-2">
                <span className="font-mono text-[13px] text-ink min-w-0 flex-1 basis-48 break-all">
                  {row.domain}
                </span>
                <Segmented options={JOBS} value={row.job} onChange={(job) => setJob(i, job)} />
                <select
                  aria-label={row.job === "serve" ? "Process" : "Redirect to"}
                  value={row.target}
                  onChange={(e) => update(i, { ...row, target: e.target.value })}
                  className={`${MONO} w-44 max-w-full`}
                >
                  {targets.includes(row.target) ? null : (
                    <option value={row.target}>{row.target || "Choose…"}</option>
                  )}
                  {targets.map((t) => (
                    <option key={t} value={t}>
                      {t}
                    </option>
                  ))}
                </select>
                {isWildcard(row.domain) ? (
                  <select
                    aria-label="DNS provider"
                    value={row.dns_provider_id ?? ""}
                    onChange={(e) => update(i, { ...row, dns_provider_id: e.target.value || null })}
                    className={`${INPUT} w-44 max-w-full`}
                  >
                    <option value="">{providers.length ? "DNS provider…" : "No DNS provider yet"}</option>
                    {providers.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                  </select>
                ) : null}
                <label className="flex items-center gap-1.5 text-[12.5px] text-ink-3">
                  <input
                    type="radio"
                    name="primary-domain"
                    checked={row.primary}
                    disabled={row.job !== "serve"}
                    onChange={() => onChange(domains.map((r, j) => ({ ...r, primary: j === i })))}
                  />
                  Primary
                </label>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label={`Remove ${row.domain}`}
                  onClick={() => onChange(keepPrimary(domains.filter((_, j) => j !== i)))}
                >
                  <X size={14} />
                </Button>
              </li>
            );
          })}
        </ul>
      ) : null}
      <div className="flex gap-2">
        <input
          value={pending}
          onChange={(e) => setPending(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              add();
            }
          }}
          placeholder="app.example.com"
          className={`${MONO} flex-1 min-w-0`}
        />
        <Button variant="secondary" onClick={add}>
          Add
        </Button>
      </div>
      {domains.some((r) => isWildcard(r.domain)) && !providers.length ? (
        <p className="text-[12px] text-ink-4">A wildcard needs a DNS provider under Settings first.</p>
      ) : null}
    </div>
  );
}
