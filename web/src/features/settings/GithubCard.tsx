import { useState } from "react";
import { Github } from "lucide-react";
import { ApiError, useConnectGithub, useDisconnectGithub, useGithub, useGithubRepos } from "@/lib/api";
import { Card, CardBody, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Row } from "@/components/ui/Row";
import { ago } from "@/lib/utils";
import type { GithubConnection } from "@/types/api";

const INPUT =
  "h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 font-mono text-[13px]";

/** GitHub sends the browser back with the sentence in the URL; read it once, then take it out. */
function handoffFailure(): string | null {
  const params = new URLSearchParams(window.location.search);
  if (params.get("github") !== "failed") return null;
  const reason = params.get("reason");
  window.history.replaceState(null, "", window.location.pathname);
  return reason ?? "GitHub refused the connection. Start again from Settings.";
}

export function GithubCard() {
  const { data: github, isLoading } = useGithub();
  const connect = useConnectGithub();
  const [failure] = useState(handoffFailure);
  const [organization, setOrganization] = useState("");
  const problem = connect.error?.message ?? failure;
  const connections = github?.connections ?? [];
  const hasPersonal = connections.some((c) => c.account_type === "user");

  return (
    <Card>
      <CardHeader
        title="GitHub"
        hint="Private GitHub Apps, one per account, each with read-only access to the repositories you choose"
        action={<Github size={16} className="text-ink-3" />}
      />
      <CardBody className="grid gap-5">
        {isLoading || !github ? null : (
          <>
            {connections.length === 0 ? (
              <p className="text-[13.5px] text-ink-2 leading-relaxed max-w-prose">
                Ferrum creates an App in your GitHub account with read-only access to the
                repositories you choose. Nothing is shared with anyone else.
              </p>
            ) : null}
            {connections.map((c) => (
              <Connected key={c.app_id} connection={c} />
            ))}
            <div className="grid gap-3 pt-1 border-t border-line">
              {!hasPersonal ? (
                <div className="flex items-center gap-3 flex-wrap pt-3">
                  <Button variant="primary" onClick={() => connect.mutate(undefined)} disabled={connect.isPending}>
                    Connect GitHub
                  </Button>
                  <span className="text-[12.5px] text-ink-4">Your personal account.</span>
                </div>
              ) : null}
              <div className="flex items-center gap-2 flex-wrap pt-3">
                <input
                  value={organization}
                  onChange={(e) => setOrganization(e.target.value)}
                  placeholder="organisation"
                  className={`${INPUT} w-48`}
                />
                <Button
                  variant={hasPersonal ? "primary" : "ghost"}
                  disabled={!organization.trim() || connect.isPending}
                  onClick={() => connect.mutate(organization.trim())}
                >
                  Connect an organisation
                </Button>
                <span className="text-[12.5px] text-ink-4">
                  Registers a private App owned by the organisation. You need to be one of its owners.
                </span>
              </div>
              {problem ? <span className="text-[12.5px] text-fail">{problem}</span> : null}
            </div>
          </>
        )}
      </CardBody>
    </Card>
  );
}

function Connected({ connection }: { connection: GithubConnection }) {
  const repos = useGithubRepos();
  const disconnect = useDisconnectGithub();
  const [confirming, setConfirming] = useState(false);
  const appUrl = `https://github.com/apps/${connection.app_slug}`;
  const prefix = `${connection.account.toLowerCase()}/`;
  const accessible = repos.data?.filter((r) => r.full_name.toLowerCase().startsWith(prefix)).length;
  const notInstalled = connection.installation_id === null && (repos.data !== undefined || repos.error !== null);

  return (
    <div className="grid gap-3">
      <dl>
        <Row label="App">
          <span className="inline-flex items-center gap-2">
            <a href={appUrl} target="_blank" rel="noreferrer" className="text-accent hover:underline">
              {connection.app_name}
            </a>
            <Badge>{connection.account_type === "organization" ? "organisation" : "personal"}</Badge>
          </span>
        </Row>
        <Row label="Account">{connection.account}</Row>
        <Row label="Repositories">
          {accessible !== undefined && accessible > 0
            ? `${accessible} accessible`
            : notInstalled
              ? "Not installed yet"
              : repos.error
                ? repos.error.message
                : accessible === 0
                  ? "None yet"
                  : "…"}
        </Row>
        <Row label="Connected">{ago(connection.connected_at)}</Row>
      </dl>

      {notInstalled ? (
        <div className="flex items-center gap-3 flex-wrap">
          <a href={`${appUrl}/installations/new`}>
            <Button variant="primary">Install on a repository</Button>
          </a>
          <span className="text-[12.5px] text-ink-4">
            Pick the repositories Ferrum may read. You can change this on GitHub at any time.
          </span>
        </div>
      ) : null}

      {confirming ? (
        <div className="bg-inset border border-line rounded-inset p-3 grid gap-3">
          <p className="text-[13px] text-ink-2 leading-relaxed">
            This deletes the App's private key from this server, so deploys from {connection.account}{" "}
            stop. <strong className="text-ink">The App itself keeps existing on GitHub</strong> with
            the access you gave it, until you delete it there.
          </p>
          <div className="flex gap-2">
            <Button
              variant="danger"
              onClick={() => disconnect.mutate(connection.app_id)}
              disabled={disconnect.isPending}
            >
              Disconnect
            </Button>
            <a href={`${appUrl}/advanced`} target="_blank" rel="noreferrer">
              <Button variant="ghost">Delete the App on GitHub</Button>
            </a>
            <Button variant="ghost" className="ml-auto" onClick={() => setConfirming(false)}>
              Cancel
            </Button>
          </div>
          {disconnect.error ? (
            <p className="text-[12.5px] text-fail">
              {disconnect.error instanceof ApiError ? disconnect.error.message : String(disconnect.error)}
            </p>
          ) : null}
        </div>
      ) : (
        <div>
          <Button size="sm" variant="ghost" onClick={() => setConfirming(true)}>
            Disconnect
          </Button>
        </div>
      )}
    </div>
  );
}
