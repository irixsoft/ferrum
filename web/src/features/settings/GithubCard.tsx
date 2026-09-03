import { useState } from "react";
import { Github } from "lucide-react";
import { ApiError, useConnectGithub, useDisconnectGithub, useGithub, useGithubRepos } from "@/lib/api";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Row } from "@/components/ui/Row";
import { ago } from "@/lib/utils";

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
  const problem = connect.error?.message ?? failure;

  return (
    <Card>
      <CardHeader
        title="GitHub"
        hint="A private GitHub App that exists only in your account"
        action={<Github size={16} className="text-ink-3" />}
      />
      <CardBody>
        {isLoading || !github ? null : github.connected ? (
          <Connected
            appSlug={github.app_slug}
            appName={github.app_name}
            account={github.account}
            connectedAt={github.connected_at}
          />
        ) : (
          <div className="grid gap-3">
            <p className="text-[13.5px] text-ink-2 leading-relaxed max-w-prose">
              Ferrum creates an App in your GitHub account with read-only access to the repositories
              you choose. Nothing is shared with anyone else.
            </p>
            <div className="flex items-center gap-3 flex-wrap">
              <Button variant="primary" onClick={() => connect.mutate()} disabled={connect.isPending}>
                Connect GitHub
              </Button>
              {problem ? <span className="text-[12.5px] text-fail">{problem}</span> : null}
            </div>
          </div>
        )}
      </CardBody>
      <CardFoot>
        <span>
          Installation tokens expire hourly and refresh themselves. No long-lived personal access
          token is stored.
        </span>
      </CardFoot>
    </Card>
  );
}

function Connected({
  appSlug,
  appName,
  account,
  connectedAt,
}: {
  appSlug: string;
  appName: string;
  account: string;
  connectedAt: string;
}) {
  const repos = useGithubRepos();
  const disconnect = useDisconnectGithub();
  const [confirming, setConfirming] = useState(false);

  const notInstalled =
    repos.error instanceof ApiError && repos.error.status === 503 && repos.error.message.includes("not installed");
  const appUrl = `https://github.com/apps/${appSlug}`;

  return (
    <div className="grid gap-4">
      <dl>
        <Row label="App">
          <a href={appUrl} target="_blank" rel="noreferrer" className="text-accent hover:underline">
            {appName}
          </a>
        </Row>
        <Row label="Account">{account}</Row>
        <Row label="Repositories">
          {repos.data
            ? `${repos.data.length} accessible`
            : notInstalled
              ? "Not installed yet"
              : repos.error
                ? repos.error.message
                : "…"}
        </Row>
        <Row label="Connected">{ago(connectedAt)}</Row>
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
            This deletes the App's private key from this server, so deploys from GitHub stop.{" "}
            <strong className="text-ink">The App itself keeps existing in your GitHub account</strong>{" "}
            with the access you gave it, until you delete it there.
          </p>
          <div className="flex gap-2">
            <Button variant="danger" onClick={() => disconnect.mutate()} disabled={disconnect.isPending}>
              Disconnect
            </Button>
            <a href={`${appUrl}/advanced`} target="_blank" rel="noreferrer">
              <Button variant="ghost">Delete the App on GitHub</Button>
            </a>
            <Button variant="ghost" className="ml-auto" onClick={() => setConfirming(false)}>
              Cancel
            </Button>
          </div>
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
