import { useEffect, useState } from "react";
import { KeyRound, Plus } from "lucide-react";
import {
  ApiError,
  useBuildLimits,
  useCreateToken,
  useCreateUser,
  useEnrollmentLink,
  useRevokeSession,
  useRevokeToken,
  useSessions,
  useSetBuildLimits,
  useTokens,
  useUsers,
  useVersion,
} from "@/lib/api";
import type { BuildLimits } from "@/types/api";
import { useShell } from "@/shells/useShell";
import { useTheme, type Theme } from "@/lib/theme";
import { PageTitle } from "@/components/PageTitle";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Row } from "@/components/ui/Row";
import { Segmented } from "@/components/ui/Segmented";
import { Tabs } from "@/components/ui/Tabs";
import { GithubCard } from "@/features/settings/GithubCard";
import { ago } from "@/lib/utils";

type Tab = "people" | "tokens" | "connections" | "builds" | "appearance" | "about";

const message = (e: unknown) => (e instanceof ApiError ? e.message : e ? String(e) : null);

/** GitHub sends the browser back to `/settings?github=…`, and that belongs on the Connections tab. */
const initialTab = (): Tab =>
  new URLSearchParams(window.location.search).has("github") ? "connections" : "people";

export function SettingsPage() {
  const [tab, setTab] = useState<Tab>(initialTab);

  return (
    <>
      <PageTitle above="This panel and who can reach it" title="Settings" />

      <Tabs
        value={tab}
        onChange={setTab}
        className="mb-5"
        tabs={[
          { value: "people", label: "People" },
          { value: "tokens", label: "API tokens" },
          { value: "connections", label: "Connections" },
          { value: "builds", label: "Builds" },
          { value: "appearance", label: "Appearance" },
          { value: "about", label: "About" },
        ]}
      />

      {tab === "people" && <People />}
      {tab === "tokens" && <Tokens />}
      {tab === "connections" && <Connections />}
      {tab === "builds" && <Builds />}
      {tab === "appearance" && <Appearance />}
      {tab === "about" && <About />}
    </>
  );
}

function People() {
  const { data: users = [] } = useUsers();
  const createUser = useCreateUser();
  const reissue = useEnrollmentLink();
  const [link, setLink] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [inviting, setInviting] = useState(false);

  const invite = async () => {
    if (!name.trim()) return;
    const created = await createUser.mutateAsync(name.trim());
    setLink(created.enrollment_url);
    setName("");
    setInviting(false);
  };

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader
          title="People"
          hint="Everyone here is a full administrator. Scoped permissions come later."
          action={
            <Button size="sm" variant="primary" onClick={() => setInviting(true)}>
              <Plus size={14} />
              Invite
            </Button>
          }
        />
        <CardBody className="pb-3">
          {inviting ? (
            <div className="flex gap-2 mb-4">
              <input
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && invite()}
                placeholder="Their name"
                className="flex-1 h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4"
              />
              <Button variant="primary" onClick={invite} disabled={createUser.isPending}>
                Create link
              </Button>
              <Button variant="ghost" onClick={() => setInviting(false)}>
                Cancel
              </Button>
            </div>
          ) : null}

          {link ? <Handoff label="Send them this link" value={link} onDone={() => setLink(null)} /> : null}

          <ul>
            {users.map((u) => (
              <li key={u.id} className="py-3 border-b border-line last:border-0">
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-[14px] font-medium text-ink">{u.name}</span>
                  {!u.passkeys.length ? <Badge tone="run">No passkey yet</Badge> : null}
                  <Button
                    size="sm"
                    variant="ghost"
                    className="ml-auto"
                    disabled={reissue.isPending}
                    onClick={async () => setLink((await reissue.mutateAsync(u.id)).enrollment_url)}
                  >
                    New enrollment link
                  </Button>
                </div>
                {u.passkeys.length ? (
                  <ul className="mt-2 grid gap-1">
                    {u.passkeys.map((p) => (
                      <li key={p.id} className="flex items-center gap-2 text-[12.5px] text-ink-3">
                        <KeyRound size={12} className="text-ink-4" />
                        {p.label ?? "Passkey"}
                        <span className="text-ink-4">
                          · added {ago(p.added_at)}
                          {p.last_used_at ? `, used ${ago(p.last_used_at)}` : ", never used"}
                        </span>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="mt-2 text-[12.5px] text-ink-4">
                    Send them a fresh enrollment link — links are single-use and expire after an
                    hour.
                  </p>
                )}
              </li>
            ))}
          </ul>
        </CardBody>
        <CardFoot>
          <span>
            There are no passwords and no recovery codes. If every passkey is lost, run{" "}
            <Code>ferrum passkey enroll</Code> over SSH.
          </span>
        </CardFoot>
      </Card>

      <Sessions />
    </div>
  );
}

function Sessions() {
  const { data: sessions = [] } = useSessions();
  const revoke = useRevokeSession();

  return (
    <Card>
      <CardHeader title="Your sessions" />
      <CardBody className="pb-3">
        <ul>
          {sessions.map((s) => (
            <li
              key={s.id}
              className="flex items-center gap-x-3 gap-y-1 flex-wrap py-2.5 border-b border-line last:border-0"
            >
              <span className="text-[13.5px] text-ink">{s.device ?? "Unknown device"}</span>
              {s.current ? <Badge tone="ok">This device</Badge> : null}
              <span className="ml-auto font-mono text-[12.5px] text-ink-4">{s.ip ?? "—"}</span>
              <span className="text-[12.5px] text-ink-4">seen {ago(s.last_seen)}</span>
              {!s.current ? (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={revoke.isPending}
                  onClick={() => revoke.mutate(s.id)}
                >
                  Sign out
                </Button>
              ) : null}
            </li>
          ))}
        </ul>
      </CardBody>
    </Card>
  );
}

function Tokens() {
  const { data: tokens = [] } = useTokens();
  const create = useCreateToken();
  const revoke = useRevokeToken();
  const [secret, setSecret] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [readOnly, setReadOnly] = useState(false);
  const [creating, setCreating] = useState(false);

  const mint = async () => {
    if (!name.trim()) return;
    const minted = await create.mutateAsync({ name: name.trim(), read_only: readOnly });
    setSecret(minted.secret);
    setName("");
    setCreating(false);
  };

  return (
    <Card>
      <CardHeader
        title="API tokens"
        hint="For machines. The same tokens authenticate the MCP endpoint."
        action={
          <Button size="sm" variant="primary" onClick={() => setCreating(true)}>
            <Plus size={14} />
            New token
          </Button>
        }
      />
      <CardBody className="pb-3">
        {creating ? (
          <div className="flex gap-2 mb-4 flex-wrap">
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && mint()}
              placeholder="What is it for?"
              className="flex-1 min-w-[160px] h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4"
            />
            <Segmented
              value={readOnly ? "read" : "write"}
              onChange={(v) => setReadOnly(v === "read")}
              options={[
                { value: "write", label: "Read and write" },
                { value: "read", label: "Read only" },
              ]}
            />
            <Button variant="primary" onClick={mint} disabled={create.isPending}>
              Create
            </Button>
            <Button variant="ghost" onClick={() => setCreating(false)}>
              Cancel
            </Button>
          </div>
        ) : null}

        {secret ? (
          <Handoff
            label="Copy it now — this is the only time it is shown"
            value={secret}
            onDone={() => setSecret(null)}
          />
        ) : null}

        <ul>
          {tokens.map((t) => (
            <li
              key={t.id}
              className="flex items-center gap-x-3 gap-y-1 flex-wrap py-3 border-b border-line last:border-0"
            >
              <div className="min-w-0">
                <span className="text-[13.5px] text-ink">{t.name}</span>
                <span className="block font-mono text-[12px] text-ink-4">{t.prefix}…</span>
              </div>
              {t.read_only ? <Badge tone="accent">Read only</Badge> : <Badge>Read and write</Badge>}
              <span className="ml-auto text-[12.5px] text-ink-4">
                {t.last_used ? `used ${ago(t.last_used)}` : "never used"}
              </span>
              <Button
                size="sm"
                variant="ghost"
                disabled={revoke.isPending}
                onClick={() => revoke.mutate(t.id)}
              >
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      </CardBody>
      <CardFoot>
        <span>
          A read-only token sees only the read tools over MCP. The write tools are absent from its
          tool list, so an agent never proposes an action it cannot take.
        </span>
      </CardFoot>
    </Card>
  );
}

function Handoff({
  label,
  value,
  onDone,
}: {
  label: string;
  value: string;
  onDone: () => void;
}) {
  return (
    <div className="mb-4 bg-inset border border-line rounded-inset p-3">
      <p className="text-[12.5px] text-ink-3 mb-2">{label}</p>
      <div className="flex items-center gap-2">
        <code className="flex-1 font-mono text-[12px] text-ink break-all">{value}</code>
        <Button size="sm" onClick={() => navigator.clipboard?.writeText(value)}>
          Copy
        </Button>
        <Button size="sm" variant="ghost" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}

function Connections() {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <GithubCard />

      <Card>
        <CardHeader title="MCP" hint="Point your own agent at this server" />
        <CardBody>
          <pre className="bg-inset border border-line rounded-inset p-3 font-mono text-[12px] leading-relaxed text-ink-3 overflow-x-auto">
{`{
  "mcpServers": {
    "ferrum": {
      "type": "http",
      "url": "${location.origin}/mcp",
      "headers": { "Authorization": "Bearer ferr_…" }
    }
  }
}`}
          </pre>
        </CardBody>
        <CardFoot>
          <span>
            Deleting apps or databases, user management and firewall changes are never exposed over
            MCP. Those stay behind type-the-name confirmations here.
          </span>
        </CardFoot>
      </Card>
    </div>
  );
}

function Builds() {
  const { data: current } = useBuildLimits();
  const save = useSetBuildLimits();
  const [draft, setDraft] = useState<BuildLimits | null>(null);
  useEffect(() => {
    if (current && !draft) setDraft(current);
  }, [current, draft]);

  if (!current || !draft) return null;
  const dirty =
    draft.memory_mb !== current.memory_mb ||
    draft.build_secs !== current.build_secs ||
    draft.migrate_secs !== current.migrate_secs;
  const field = (key: keyof BuildLimits) => (e: React.ChangeEvent<HTMLInputElement>) =>
    setDraft({ ...draft, [key]: Number(e.target.value) });

  return (
    <Card>
      <CardHeader title="Builds" hint="Applied to the next deploy; a running build keeps its limits" />
      <CardBody>
        <dl>
          <Row
            label="Memory limit"
            hint={`A build over this is stopped as itself. This host has ${current.memory_total_mb} MB; the default leaves 512 MB for what is running.`}
          >
            <Limit value={draft.memory_mb} onChange={field("memory_mb")} min={512} max={current.memory_total_mb} unit="MB" />
          </Row>
          <Row label="Build timeout" hint="Install and build commands, each">
            <Limit value={draft.build_secs} onChange={field("build_secs")} min={60} max={7200} unit="s" />
          </Row>
          <Row label="Migration timeout" hint="The migrate command, while the app is paused">
            <Limit value={draft.migrate_secs} onChange={field("migrate_secs")} min={60} max={7200} unit="s" />
          </Row>
        </dl>
        <div className="mt-4 flex items-center gap-3 flex-wrap">
          <Button
            variant="primary"
            disabled={!dirty || save.isPending}
            onClick={async () => setDraft(await save.mutateAsync(draft))}
          >
            Save
          </Button>
          {save.error ? <span className="text-[12.5px] text-fail">{message(save.error)}</span> : null}
          {save.isSuccess && !dirty ? <span className="text-[12.5px] text-ok">Saved.</span> : null}
        </div>
      </CardBody>
      <CardFoot>
        <span>
          A Next.js build peaks around 1.5 to 2.5 GB. If a build is stopped for memory, raise the
          limit here rather than fighting the kernel.
        </span>
      </CardFoot>
    </Card>
  );
}

function Limit({
  value,
  onChange,
  min,
  max,
  unit,
}: {
  value: number;
  onChange: (e: React.ChangeEvent<HTMLInputElement>) => void;
  min: number;
  max: number;
  unit: string;
}) {
  return (
    <span className="inline-flex items-center gap-2">
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        onChange={onChange}
        className="w-28 h-9 px-3 bg-inset border border-line-strong rounded-control font-mono text-sm text-ink text-right tnum"
      />
      <span className="text-[12.5px] text-ink-4 w-6">{unit}</span>
    </span>
  );
}

function Appearance() {
  const { theme, setTheme } = useTheme();
  const { forceDesktop, setForceDesktop, overridable } = useShell();

  return (
    <Card>
      <CardHeader title="Appearance" hint="Stored on this device only" />
      <CardBody>
        <dl>
          <Row label="Theme">
            <Segmented<Theme>
              value={theme}
              onChange={setTheme}
              options={[
                { value: "light", label: "Light" },
                { value: "dark", label: "Dark" },
                { value: "system", label: "System" },
              ]}
            />
          </Row>
          <Row
            label="Layout"
            hint={
              overridable
                ? "Useful on a tablet, where the right answer is a matter of taste"
                : "Ignored on this screen — too narrow for the desktop layout"
            }
          >
            <Segmented
              value={forceDesktop ? "desktop" : "auto"}
              onChange={(v) => setForceDesktop(v === "desktop")}
              options={[
                { value: "auto", label: "Match screen" },
                { value: "desktop", label: "Always desktop" },
              ]}
            />
          </Row>
        </dl>
      </CardBody>
    </Card>
  );
}

/** AGPL §13: the running commit must stay linked here, not reduced to a version. */
function About() {
  const { data: version } = useVersion();
  if (!version) return null;

  const source = "https://github.com/irixsoft/ferrum";

  return (
    <Card>
      <CardHeader title="About Ferrum" />
      <CardBody>
        <dl>
          <Row label="Version">{version.version}</Row>
          <Row label="Build">
            <Code>{version.build_id}</Code>
          </Row>
          <Row label="Built from commit">
            <a
              href={`${source}/commit/${version.commit_sha}`}
              target="_blank"
              rel="noreferrer"
              className="font-mono text-[13px] text-accent hover:underline"
            >
              {version.commit_sha.slice(0, 12)}
            </a>
          </Row>
          <Row label="Host">
            {version.os} · {version.arch}
          </Row>
          <Row label="Licence">AGPL-3.0-only</Row>
          <Row label="Source">
            <a href={source} target="_blank" rel="noreferrer" className="text-accent hover:underline">
              github.com/irixsoft/ferrum
            </a>
          </Row>
        </dl>
        <p className="text-[12.5px] text-ink-4 mt-4 leading-relaxed max-w-prose">
          Ferrum is free software under the GNU Affero General Public License. Because you are using
          it over a network, you are entitled to the complete source of this exact build — the
          commit above is it.
        </p>
      </CardBody>
    </Card>
  );
}
