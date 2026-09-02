import { useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { GitBranch, Lock, Search } from "lucide-react";
import {
  ApiError,
  installRuntime,
  resolveVersion,
  useCreateApp,
  useDetect,
  useGithub,
  useGithubRepos,
  useRuntimes,
} from "@/lib/api";
import { PageTitle } from "@/components/PageTitle";
import { RuntimeMark, runtimeLabel } from "@/components/RuntimeMark";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { Segmented } from "@/components/ui/Segmented";
import { ConfigForm, draftFromDetection, toNewApp, type Draft, type Sources } from "./ConfigForm";
import { ago, bytes } from "@/lib/utils";
import type { Detected, GithubRepo, Progress, Tracking } from "@/types/api";

type Step = { kind: "pick" } | { kind: "detecting" } | { kind: "review"; detected: Detected };

const INPUT =
  "w-full h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4";

export function NewAppPage() {
  const { data: github, isLoading } = useGithub();
  const [step, setStep] = useState<Step>({ kind: "pick" });
  const [repo, setRepo] = useState<GithubRepo | null>(null);
  const [gitRef, setGitRef] = useState("");
  const [tracking, setTracking] = useState<Tracking>("releases");
  const [root, setRoot] = useState("");
  const [draft, setDraft] = useState<Draft | null>(null);
  const [sources, setSources] = useState<Sources>({});
  const [candidate, setCandidate] = useState(0);
  const detect = useDetect();

  const pick = (r: GithubRepo) => {
    setRepo(r);
    setGitRef(r.default_branch);
  };

  const inspect = async () => {
    if (!repo) return;
    setStep({ kind: "detecting" });
    try {
      const detected = await detect.mutateAsync({ repository: repo.full_name, ref: gitRef, root });
      const best = detected.candidates[0] ?? null;
      const built = draftFromDetection(repo.full_name, gitRef, tracking, root, detected, best);
      setCandidate(0);
      setSources(built.sources);
      setDraft(await withResolvedVersion(built.draft));
      setStep({ kind: "review", detected });
    } catch {
      setStep({ kind: "pick" });
    }
  };

  const chooseCandidate = async (index: number) => {
    if (step.kind !== "review" || !repo) return;
    const built = draftFromDetection(
      repo.full_name,
      gitRef,
      tracking,
      root,
      step.detected,
      step.detected.candidates[index] ?? null,
    );
    setCandidate(index);
    setSources(built.sources);
    setDraft(await withResolvedVersion(built.draft));
  };

  if (isLoading) return null;
  if (!github?.connected) {
    return (
      <>
        <PageTitle above="Deploy from a GitHub repository" title="New app" />
        <Card>
          <CardBody className="pt-5 grid gap-3">
            <p className="text-[13.5px] text-ink-2 leading-relaxed max-w-prose">
              Ferrum reads repositories through a GitHub App that lives in your account. Connect
              it once and every repository you grant it appears here.
            </p>
            <div>
              <Link to="/settings" search={{ github: "connect" }}>
                <Button variant="primary">Connect GitHub in Settings</Button>
              </Link>
            </div>
          </CardBody>
        </Card>
      </>
    );
  }

  return (
    <>
      <PageTitle
        above={
          repo ? (
            <span className="inline-flex items-center gap-2">
              <span className="font-mono">{repo.full_name}</span>
              <span className="text-ink-4">@ {gitRef}</span>
            </span>
          ) : (
            "Deploy from a GitHub repository"
          )
        }
        title={step.kind === "review" && draft ? draft.name : "New app"}
      />

      {step.kind === "pick" ? (
        <Picker
          repo={repo}
          onPick={pick}
          gitRef={gitRef}
          setGitRef={setGitRef}
          tracking={tracking}
          setTracking={setTracking}
          root={root}
          setRoot={setRoot}
          onInspect={inspect}
          error={detect.error instanceof ApiError ? detect.error.message : null}
        />
      ) : step.kind === "detecting" ? (
        <Card>
          <CardBody className="pt-5">
            <p className="text-[13.5px] text-ink-2">Reading the repository…</p>
            <p className="text-[12.5px] text-ink-4 mt-1">
              The tree listing and a handful of files. Nothing is cloned yet.
            </p>
          </CardBody>
        </Card>
      ) : draft ? (
        <Review
          repo={repo!}
          detected={step.detected}
          candidate={candidate}
          onCandidate={chooseCandidate}
          draft={draft}
          setDraft={setDraft}
          sources={sources}
          onBack={() => setStep({ kind: "pick" })}
        />
      ) : null}
    </>
  );
}

async function withResolvedVersion(draft: Draft): Promise<Draft> {
  try {
    const version = await resolveVersion(draft.toolchain, draft.runtime_version || null);
    return { ...draft, runtime_version: version };
  } catch {
    return draft;
  }
}

function Picker({
  repo,
  onPick,
  gitRef,
  setGitRef,
  tracking,
  setTracking,
  root,
  setRoot,
  onInspect,
  error,
}: {
  repo: GithubRepo | null;
  onPick: (r: GithubRepo) => void;
  gitRef: string;
  setGitRef: (v: string) => void;
  tracking: Tracking;
  setTracking: (v: Tracking) => void;
  root: string;
  setRoot: (v: string) => void;
  onInspect: () => void;
  error: string | null;
}) {
  const repos = useGithubRepos();
  const [query, setQuery] = useState("");
  const matching = (repos.data ?? []).filter((r) =>
    r.full_name.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const notInstalled =
    repos.error instanceof ApiError && repos.error.status === 503 && repos.error.message.includes("not installed");

  return (
    <div className="grid gap-4 lg:grid-cols-12">
      <Card className="lg:col-span-7">
        <CardHeader
          title="Repository"
          hint="Everything the GitHub App can read"
          action={
            <label className="relative">
              <Search size={13} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-4" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Filter"
                className="h-8 w-44 pl-8 pr-3 bg-inset border border-line rounded-full text-[13px] text-ink placeholder:text-ink-4"
              />
            </label>
          }
        />
        <CardBody className="pb-3">
          {notInstalled ? (
            <p className="text-[13.5px] text-ink-3">
              The App is not installed on any repository yet. Install it from Settings → Connections.
            </p>
          ) : repos.error ? (
            <p className="text-[13.5px] text-fail">{repos.error.message}</p>
          ) : (
            <ul className="divide-y divide-line max-h-[26rem] overflow-y-auto -mx-5 px-5">
              {matching.map((r) => (
                <li key={r.full_name}>
                  <button
                    onClick={() => onPick(r)}
                    aria-pressed={repo?.full_name === r.full_name}
                    className="w-full text-left py-2.5 flex items-center gap-3 hover:bg-inset/60 -mx-2 px-2 rounded-control aria-pressed:bg-inset"
                  >
                    <span className="font-mono text-[13px] text-ink truncate">{r.full_name}</span>
                    {r.private ? <Lock size={12} className="text-ink-4 shrink-0" /> : null}
                    <span className="ml-auto text-[12px] text-ink-4 shrink-0">
                      {r.pushed_at ? `pushed ${ago(r.pushed_at)}` : ""}
                    </span>
                  </button>
                </li>
              ))}
              {repos.data && matching.length === 0 ? (
                <li className="py-3 text-[13px] text-ink-4">Nothing matches.</li>
              ) : null}
            </ul>
          )}
        </CardBody>
      </Card>

      <Card className="lg:col-span-5">
        <CardHeader title="What to deploy" />
        <CardBody className="grid gap-4">
          <div>
            <label className="block text-[13px] text-ink-3 mb-1.5">Branch or tag</label>
            <div className="relative">
              <GitBranch size={13} className="absolute left-3 top-1/2 -translate-y-1/2 text-ink-4" />
              <input
                value={gitRef}
                onChange={(e) => setGitRef(e.target.value)}
                disabled={!repo}
                className={`${INPUT} pl-8 font-mono text-[13px]`}
              />
            </div>
          </div>
          <div>
            <label className="block text-[13px] text-ink-3 mb-1.5">Deploy on</label>
            <Segmented
              value={tracking}
              onChange={setTracking}
              options={[
                { value: "releases", label: "Releases" },
                { value: "branch", label: "Every push" },
              ]}
            />
            <p className="text-[12px] text-ink-4 mt-1.5">
              Releases by default: a push to main should not take production down with it.
            </p>
          </div>
          <div>
            <label className="block text-[13px] text-ink-3 mb-1.5">Root directory</label>
            <input
              value={root}
              onChange={(e) => setRoot(e.target.value)}
              placeholder="Leave empty for the repository root"
              disabled={!repo}
              className={`${INPUT} font-mono text-[13px]`}
            />
          </div>
          {error ? <p className="text-[12.5px] text-fail">{error}</p> : null}
        </CardBody>
        <CardFoot>
          <span>Ferrum inspects the tree through the GitHub API. Nothing is cloned until the first deploy.</span>
          <Button variant="primary" disabled={!repo || !gitRef.trim()} onClick={onInspect}>
            Inspect
          </Button>
        </CardFoot>
      </Card>
    </div>
  );
}

function Review({
  repo,
  detected,
  candidate,
  onCandidate,
  draft,
  setDraft,
  sources,
  onBack,
}: {
  repo: GithubRepo;
  detected: Detected;
  candidate: number;
  onCandidate: (i: number) => void;
  draft: Draft;
  setDraft: (d: Draft) => void;
  sources: Sources;
  onBack: () => void;
}) {
  const navigate = useNavigate();
  const runtimes = useRuntimes();
  const create = useCreateApp();
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const installed = runtimes.data?.installed.some(
    (t) => t.kind === draft.toolchain && t.version === draft.runtime_version,
  );

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      if (!installed) {
        await installRuntime(draft.toolchain, draft.runtime_version, setProgress);
        await runtimes.refetch();
      }
      const created = await create.mutateAsync(toNewApp(draft, repo.full_name));
      navigate({ to: "/apps/$slug", params: { slug: created.slug } });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setProgress(null);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid gap-4">
      {detected.candidates.length === 0 ? (
        <Card>
          <CardBody className="pt-5">
            <p className="text-[13.5px] text-ink-2">
              No runtime was recognised. Fill the fields in by hand, or add a <Code>ferrum.toml</Code>.
            </p>
          </CardBody>
        </Card>
      ) : (
        <Card>
          <CardHeader
            title="Detected"
            hint="Every field below is prefilled from the repository and editable"
            action={
              <Button size="sm" variant="ghost" onClick={onBack}>
                Change repository
              </Button>
            }
          />
          <CardBody className="grid gap-2">
            {detected.candidates.map((c, i) => (
              <button
                key={`${c.kind}-${i}`}
                onClick={() => onCandidate(i)}
                aria-pressed={i === candidate}
                className="text-left flex items-center gap-3 flex-wrap bg-inset border border-line rounded-inset px-3 py-2 aria-pressed:border-line-strong"
              >
                <RuntimeMark runtime={c.kind} version={c.version ?? undefined} />
                {c.kind === "static" ? (
                  <span className="text-[12.5px] text-ink-4">built with {runtimeLabel(c.toolchain)}</span>
                ) : null}
                <span className="text-[12.5px] text-ink-3">{c.reasons.join(" · ")}</span>
                <Badge className="ml-auto" mono>
                  {c.confidence}%
                </Badge>
              </button>
            ))}
            {detected.aptfile_rejected.length ? (
              <p className="text-[12.5px] text-fail">
                Rejected from the Aptfile: {detected.aptfile_rejected.map((p) => `"${p}"`).join(", ")}.
                Package names may only contain lowercase letters, digits and <Code>+._-</Code>.
              </p>
            ) : null}
          </CardBody>
        </Card>
      )}

      <ConfigForm draft={draft} onChange={setDraft} sources={sources} creating />

      <Card>
        <CardBody className="pt-5 grid gap-3">
          {!installed && runtimes.data ? (
            <p className="text-[13px] text-ink-3">
              {runtimeLabel(draft.toolchain)} {draft.runtime_version} is not installed yet. Creating
              the app downloads it first.
            </p>
          ) : null}
          {progress ? <ProgressLine progress={progress} /> : null}
          {error ? <p className="text-[12.5px] text-fail">{error}</p> : null}
          <div className="flex items-center gap-3">
            <Button variant="primary" size="lg" onClick={submit} disabled={busy}>
              {busy ? "Working…" : "Create"}
            </Button>
            <span className="text-[12.5px] text-ink-4">
              Creates the system user, the unit and the nginx site. Nothing is deployed yet.
            </span>
          </div>
        </CardBody>
      </Card>
    </div>
  );
}

function ProgressLine({ progress }: { progress: Progress }) {
  const width =
    progress.state === "downloading" && progress.total
      ? Math.round((progress.received / progress.total) * 100)
      : progress.state === "ready"
        ? 100
        : progress.state === "downloading"
          ? 0
          : 100;
  const label =
    progress.state === "downloading"
      ? `Downloading ${bytes(progress.received)}${progress.total ? ` of ${bytes(progress.total)}` : ""}`
      : progress.state === "extracting"
        ? "Unpacking"
        : progress.state === "installing"
          ? "Running the installer"
          : progress.state === "ready"
            ? "Installed"
            : progress.error;
  return (
    <div>
      <div className="h-1.5 bg-inset rounded-full overflow-hidden">
        <div
          className="h-full bg-ink rounded-full transition-[width] duration-200"
          style={{ width: `${width}%` }}
        />
      </div>
      <p className="text-[12.5px] text-ink-3 mt-1.5">{label}</p>
    </div>
  );
}
