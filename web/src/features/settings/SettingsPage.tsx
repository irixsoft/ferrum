import { useState } from "react";
import { Github, KeyRound, Plus } from "lucide-react";
import { useHost, useSettings } from "@/lib/api";
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
import { ago } from "@/lib/utils";

type Tab = "people" | "tokens" | "connections" | "appearance" | "about";

export function SettingsPage() {
  const [tab, setTab] = useState<Tab>("people");
  const { data } = useSettings();
  if (!data) return null;

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
          { value: "appearance", label: "Appearance" },
          { value: "about", label: "About" },
        ]}
      />

      {tab === "people" && (
        <div className="grid gap-4">
          <Card>
            <CardHeader
              title="People"
              hint="Everyone here is a full administrator. Scoped permissions come later."
              action={
                <Button size="sm" variant="primary">
                  <Plus size={14} />
                  Invite
                </Button>
              }
            />
            <CardBody className="pb-3">
              <ul>
                {data.users.map((u) => (
                  <li key={u.id} className="py-3 border-b border-line last:border-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-[14px] font-medium text-ink">{u.name}</span>
                      <span className="text-[13px] text-ink-4">{u.email}</span>
                      {!u.enrolled ? <Badge tone="run">Enrollment link not used</Badge> : null}
                    </div>
                    {u.passkeys.length ? (
                      <ul className="mt-2 grid gap-1">
                        {u.passkeys.map((p) => (
                          <li key={p.id} className="flex items-center gap-2 text-[12.5px] text-ink-3">
                            <KeyRound size={12} className="text-ink-4" />
                            {p.label}
                            <span className="text-ink-4">
                              · {p.last_used_at ? `used ${ago(p.last_used_at)}` : "never used"}
                            </span>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <p className="mt-2 text-[12.5px] text-ink-4">
                        No passkey yet. Send them a fresh enrollment link — links are single-use and
                        expire after an hour.
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

          <Card>
            <CardHeader title="Your sessions" />
            <CardBody className="pb-3">
              <ul>
                {data.sessions.map((s) => (
                  <li
                    key={s.id}
                    className="flex items-center gap-x-3 gap-y-1 flex-wrap py-2.5 border-b border-line last:border-0"
                  >
                    <span className="text-[13.5px] text-ink">{s.device}</span>
                    {s.current ? <Badge tone="ok">This device</Badge> : null}
                    <span className="ml-auto font-mono text-[12.5px] text-ink-4">{s.ip}</span>
                    {!s.current ? (
                      <Button size="sm" variant="ghost">
                        Sign out
                      </Button>
                    ) : null}
                  </li>
                ))}
              </ul>
            </CardBody>
          </Card>
        </div>
      )}

      {tab === "tokens" && (
        <Card>
          <CardHeader
            title="API tokens"
            hint="For machines. The same tokens authenticate the MCP endpoint."
            action={
              <Button size="sm" variant="primary">
                <Plus size={14} />
                New token
              </Button>
            }
          />
          <CardBody className="pb-3">
            <ul>
              {data.tokens.map((t) => (
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
                    {t.last_used_at ? `used ${ago(t.last_used_at)}` : "never used"}
                  </span>
                  <Button size="sm" variant="ghost">
                    Revoke
                  </Button>
                </li>
              ))}
            </ul>
          </CardBody>
          <CardFoot>
            <span>
              A read-only token sees only the read tools over MCP. The write tools are absent from
              its tool list, so an agent never proposes an action it cannot take.
            </span>
          </CardFoot>
        </Card>
      )}

      {tab === "connections" && (
        <div className="grid gap-4 lg:grid-cols-2">
          <Card>
            <CardHeader
              title="GitHub"
              hint="A private GitHub App that exists only in your account"
              action={<Github size={16} className="text-ink-3" />}
            />
            <CardBody>
              <dl>
                <Row label="App">{data.github.app_name}</Row>
                <Row label="Account">{data.github.account}</Row>
                <Row label="Repositories">{data.github.repos_accessible} accessible</Row>
                <Row label="Installed">{ago(data.github.installed_at)}</Row>
              </dl>
            </CardBody>
            <CardFoot>
              <span>
                Installation tokens expire hourly and refresh themselves. No long-lived personal
                access token is stored.
              </span>
            </CardFoot>
          </Card>

          <Card>
            <CardHeader title="MCP" hint="Point your own agent at this server" />
            <CardBody>
              <pre className="bg-inset border border-line rounded-inset p-3 font-mono text-[12px] leading-relaxed text-ink-3 overflow-x-auto">
{`{
  "mcpServers": {
    "ferrum": {
      "type": "http",
      "url": "https://panel.example.com/mcp",
      "headers": { "Authorization": "Bearer ferr_…" }
    }
  }
}`}
              </pre>
            </CardBody>
            <CardFoot>
              <span>
                Deleting apps or databases, user management and firewall changes are never exposed
                over MCP. Those stay behind type-the-name confirmations here.
              </span>
            </CardFoot>
          </Card>
        </div>
      )}

      {tab === "appearance" && <Appearance />}
      {tab === "about" && <About />}
    </>
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
  const { data: host } = useHost();
  if (!host) return null;

  return (
    <Card>
      <CardHeader title="About Ferrum" />
      <CardBody>
        <dl>
          <Row label="Version">{host.ferrum_version}</Row>
          <Row label="Build">
            <Code>{host.build_id}</Code>
          </Row>
          <Row label="Built from commit">
            <a
              href={`https://github.com/irixsoft/ferrum/commit/${host.commit_sha}`}
              target="_blank"
              rel="noreferrer"
              className="font-mono text-[13px] text-accent hover:underline"
            >
              {host.commit_sha.slice(0, 12)}
            </a>
          </Row>
          <Row label="Licence">AGPL-3.0-only</Row>
          <Row label="Source">
            <a
              href="https://github.com/irixsoft/ferrum"
              target="_blank"
              rel="noreferrer"
              className="text-accent hover:underline"
            >
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
