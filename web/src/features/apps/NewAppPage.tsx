import { useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { Lock, Plus, Search, Tag } from "lucide-react";
import {
  ApiError,
  installRuntime,
  resolveVersion,
  useCreateApp,
  useCreateDatabase,
  useDetect,
  useGithub,
  useGithubRepos,
  useGithubTags,
  usePostgres,
  useRequestRedis,
  useRuntimes,
} from "@/lib/api";
import { PageTitle } from "@/components/PageTitle";
import { RuntimeMark, runtimeLabel } from "@/components/RuntimeMark";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Code } from "@/components/ui/Code";
import { ConfigForm, draftFromDetection, toNewApp, type Draft, type Sources } from "./ConfigForm";
import { EnvRows, HINTS_NOTE, blankRow, rowsFromHints, type EnvRow } from "./EnvironmentPanel";
import { ago, bytes } from "@/lib/utils";
import type { App, Detected, GithubRepo, Progress } from "@/types/api";

type Step = { kind: "pick" } | { kind: "detecting" } | { kind: "review"; detected: Detected };

const INPUT =
  "w-full h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4";
const REDIS_DEFAULT_MB = 64;

export function NewAppPage() {
  const { data: github, isLoading } = useGithub();
  const [step, setStep] = useState<Step>({ kind: "pick" });
  const [repo, setRepo] = useState<GithubRepo | null>(null);
  const [gitRef, setGitRef] = useState("");
  const [root, setRoot] = useState("");
  const [draft, setDraft] = useState<Draft | null>(null);
  const [sources, setSources] = useState<Sources>({});
  const [candidate, setCandidate] = useState(0);
  const [envRows, setEnvRows] = useState<EnvRow[]>([]);
  const detect = useDetect();

  const pick = (r: GithubRepo) => {
    setRepo(r);
    setGitRef("");
  };

  const inspect = async () => {
    if (!repo) return;
    setStep({ kind: "detecting" });
    try {
      const detected = await detect.mutateAsync({ repository: repo.full_name, ref: gitRef, root });
      const best = detected.candidates[0] ?? null;
      const built = draftFromDetection(repo.full_name, gitRef, root, detected, best);
      setCandidate(0);
      setSources(built.sources);
      setEnvRows(rowsFromHints(detected.env_hints));
      setDraft(await withResolvedVersion(built.draft));
      setStep({ kind: "review", detected });
    } catch {
      setStep({ kind: "pick" });
    }
  };

  const changeToolchain = async (d: Draft) => {
    setSources(({ runtime_version: _dropped, ...rest }) => rest);
    setDraft(d);
    try {
      const version = await resolveVersion(d.toolchain, null);
      setDraft((current) =>
        current && current.toolchain === d.toolchain ? { ...current, runtime_version: version } : current,
      );
    } catch {
      /* the field stays empty for the user to fill in */
    }
  };

  const chooseCandidate = async (index: number) => {
    if (step.kind !== "review" || !repo) return;
    const built = draftFromDetection(
      repo.full_name,
      gitRef,
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
              Ferrum reads repositories through a private GitHub App in your account or your
              organisation. Connect one once and every repository you grant it appears here.
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
          onToolchainChange={changeToolchain}
          sources={sources}
          envRows={envRows}
          setEnvRows={setEnvRows}
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
  root,
  setRoot,
  onInspect,
  error,
}: {
  repo: GithubRepo | null;
  onPick: (r: GithubRepo) => void;
  gitRef: string;
  setGitRef: (v: string) => void;
  root: string;
  setRoot: (v: string) => void;
  onInspect: () => void;
  error: string | null;
}) {
  const repos = useGithubRepos();
  const tags = useGithubTags(repo?.full_name ?? null);
  const [query, setQuery] = useState("");
  const tagNames = (tags.data ?? []).map((t) => t.name);
  const untagged = tags.data !== undefined && tagNames.length === 0;
  const refOptions = untagged && repo ? [repo.default_branch] : tagNames;
  const newest = refOptions[0] ?? "";
  useEffect(() => {
    if (repo && tags.data) setGitRef(newest);
  }, [repo, tags.data, newest, setGitRef]);
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
              No App is installed on a repository yet. Install one from Settings → Connections.
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
            <label className="block text-[13px] text-ink-3 mb-1.5">Tag</label>
            <div className="relative">
              <Tag size={13} className="absolute left-3 top-1/2 -translate-y-1/2 text-ink-4" />
              <select
                value={gitRef}
                onChange={(e) => setGitRef(e.target.value)}
                disabled={!repo || refOptions.length === 0}
                className={`${INPUT} pl-8 font-mono text-[13px]`}
              >
                {refOptions.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </div>
            <p className="text-[12px] text-ink-4 mt-1.5">
              {untagged
                ? "No tags yet. The default branch is inspected; the first tag you push deploys."
                : "Every tag you push deploys. A push to a branch does not."}
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

function databaseName(slug: string) {
  return `${slug.trim().replace(/-/g, "_")}_prod`;
}

function Review({
  repo,
  detected,
  candidate,
  onCandidate,
  draft,
  setDraft,
  onToolchainChange,
  sources,
  envRows,
  setEnvRows,
  onBack,
}: {
  repo: GithubRepo;
  detected: Detected;
  candidate: number;
  onCandidate: (i: number) => void;
  draft: Draft;
  setDraft: (d: Draft) => void;
  onToolchainChange: (d: Draft) => void;
  sources: Sources;
  envRows: EnvRow[];
  setEnvRows: React.Dispatch<React.SetStateAction<EnvRow[]>>;
  onBack: () => void;
}) {
  const navigate = useNavigate();
  const runtimes = useRuntimes();
  const postgres = usePostgres();
  const create = useCreateApp();
  const createDatabase = useCreateDatabase();
  const requestRedis = useRequestRedis(draft.slug.trim());
  const [wantPostgres, setWantPostgres] = useState(detected.wants.postgres !== null);
  const [wantRedis, setWantRedis] = useState(detected.wants.redis !== null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<App | null>(null);
  const [busy, setBusy] = useState(false);

  const installed = runtimes.data?.installed.some(
    (t) => t.kind === draft.toolchain && t.version === draft.runtime_version,
  );
  const postgresReady = postgres.data?.installed === true;
  const dbName = databaseName(draft.slug);

  const primary = draft.domains[0]?.trim() ?? "";
  const lastSuggestion = useRef("");
  useEffect(() => {
    const suggestion = primary ? `https://${primary}` : "";
    const previous = lastSuggestion.current;
    lastSuggestion.current = suggestion;
    setEnvRows((rows) =>
      rows.map((r) =>
        r.suggestAppUrl && (r.value === "" || r.value === previous) ? { ...r, value: suggestion } : r,
      ),
    );
  }, [primary, setEnvRows]);

  const submit = async () => {
    setBusy(true);
    setError(null);
    let app: App | null = null;
    try {
      if (!installed) {
        await installRuntime(draft.toolchain, draft.runtime_version, setProgress);
        await runtimes.refetch();
      }
      const rows = envRows.filter((r) => r.key.trim());
      app = await create.mutateAsync({
        ...toNewApp(draft, repo.full_name),
        env: rows.filter((r) => r.value).map((r) => ({ key: r.key.trim(), value: r.value ?? "" })),
        env_hints: rows
          .filter((r) => r.source !== null)
          .map((r) => ({
            key: r.key.trim(),
            source: r.source ?? "",
            optional: r.optional,
            suggest_app_url: r.suggestAppUrl,
          })),
      });
      setCreated(app);
      if (wantPostgres && postgresReady) {
        await createDatabase.mutateAsync({ name: dbName, app_slug: app.slug });
      }
      if (wantRedis) {
        await requestRedis.mutateAsync(REDIS_DEFAULT_MB);
      }
      navigate({ to: "/apps/$slug", params: { slug: app.slug } });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(app ? `${app.name} was created, but the data step failed: ${message}` : message);
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

      <ConfigForm
        draft={draft}
        repository={repo.full_name}
        onChange={setDraft}
        onToolchainChange={onToolchainChange}
        sources={sources}
        creating
      />

      <Card>
        <CardHeader
          title="Environment"
          hint="Set the values now or later from the app's Environment tab"
          action={
            <Button size="sm" variant="ghost" onClick={() => setEnvRows([...envRows, blankRow()])}>
              <Plus size={14} />
              Add
            </Button>
          }
        />
        <CardBody className="grid gap-2">
          <EnvRows rows={envRows} onChange={setEnvRows} />
          {envRows.some((r) => r.source !== null) ? (
            <p className="text-[12.5px] text-ink-4 mt-1">{HINTS_NOTE}</p>
          ) : null}
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="Data" hint="Created right after the app, linked and injected as DATABASE_URL and REDIS_URL" />
        <CardBody className="grid gap-3">
          <label className="flex items-start gap-2 text-[13px] text-ink-2">
            <input
              type="checkbox"
              className="mt-1"
              checked={wantPostgres && postgresReady}
              disabled={!postgresReady}
              onChange={(e) => setWantPostgres(e.target.checked)}
            />
            <span>
              Create and link <Code>{dbName}</Code>
              {detected.wants.postgres ? <span className="text-ink-4"> · {detected.wants.postgres}</span> : null}
              {postgres.data && !postgresReady ? (
                <span className="block text-[12.5px] text-ink-4">
                  PostgreSQL is not installed yet. Enable it on the Databases page first.
                </span>
              ) : null}
            </span>
          </label>
          <label className="flex items-start gap-2 text-[13px] text-ink-2">
            <input
              type="checkbox"
              className="mt-1"
              checked={wantRedis}
              onChange={(e) => setWantRedis(e.target.checked)}
            />
            <span>
              Create a Redis instance ({REDIS_DEFAULT_MB} MB)
              {detected.wants.redis ? <span className="text-ink-4"> · {detected.wants.redis}</span> : null}
            </span>
          </label>
        </CardBody>
      </Card>

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
            {created ? (
              <Button
                variant="primary"
                size="lg"
                onClick={() => navigate({ to: "/apps/$slug", params: { slug: created.slug } })}
              >
                Open {created.name}
              </Button>
            ) : (
              <Button variant="primary" size="lg" onClick={submit} disabled={busy}>
                {busy ? "Working…" : "Create"}
              </Button>
            )}
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
