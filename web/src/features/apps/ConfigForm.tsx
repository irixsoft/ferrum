import { useState, type ReactNode } from "react";
import { Plus, X } from "lucide-react";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Code } from "@/components/ui/Code";
import { Segmented } from "@/components/ui/Segmented";
import { runtimeLabel } from "@/components/RuntimeMark";
import { useGithubTags } from "@/lib/api";
import { isFolder, reconcileDomains, reconcileRoutes, renameInDomains, renameInRoutes, routable } from "@/lib/processes";
import { DomainsEditor, type DomainDraft } from "./DomainsEditor";
import { SharedDirHint } from "./SharedDirHint";
import type {
  App,
  AppChanges,
  Detection,
  Detected,
  NewApp,
  Process,
  ProcessInput,
  RouteInput,
  Runtime,
} from "@/types/api";

export interface Draft {
  slug: string;
  name: string;
  git_ref: string;
  root: string;
  runtime: Runtime;
  toolchain: Runtime;
  runtime_version: string;
  install: string;
  build: string;
  migrate: string;
  startup_budget_secs: number;
  cpu_percent: number;
  pause_for_migrations: boolean;
  follow_repo_file: boolean;
  processes: ProcessInput[];
  routes: RouteInput[];
  packages: string[];
  domains: DomainDraft[];
}

/** Which fields detection filled and why; `follow_repo_file` names the file that decides the app's shape. */
export type Sources = Partial<Record<keyof Draft, string>>;

type CommandField = "install" | "build" | "migrate";

const INPUT =
  "w-full h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 disabled:opacity-50";
const MONO = `${INPUT} font-mono text-[13px]`;
const RUNTIMES: Runtime[] = ["node", "bun", "dotnet"];
const SWITCH = [
  { value: "off", label: "Off" },
  { value: "on", label: "On" },
] as const;
const KINDS = [
  { value: "command", label: "Command" },
  { value: "folder", label: "Folder" },
] as const;

export const slugify = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);

const orNull = (s: string) => (s.trim() ? s.trim() : null);

function normalized(p: ProcessInput): ProcessInput {
  return isFolder(p)
    ? { ...p, start: null, port: false, health: null }
    : { ...p, static_dir: undefined, port: p.port !== false };
}

export function draftFromDetection(
  repository: string,
  gitRef: string,
  root: string,
  detected: Detected,
  candidate: Detection | null,
): { draft: Draft; sources: Sources } {
  const name = repository.split("/")[1] ?? repository;
  const manifest = detected.manifest;
  const file = manifest ? "from ferrum.toml" : null;
  const sources: Sources = {};
  const found = candidate?.reasons.join(", ") ?? "";
  const mark = (field: keyof Draft, why: string) => {
    sources[field] = why;
  };

  const runtime = candidate?.kind ?? "node";
  if (candidate) mark("runtime", found);
  if (candidate?.version) mark("runtime_version", found);

  const command = (field: CommandField) => {
    const stated = manifest?.commands[field];
    if (stated && file) {
      mark(field, file);
      return stated;
    }
    const guessed = candidate?.commands[field];
    if (guessed) {
      mark(field, found);
      return guessed;
    }
    return "";
  };

  let processes: ProcessInput[] = [{ name: "web", start: "", dir: "", port: true, health: "/" }];
  if (manifest?.processes.length && file) {
    processes = manifest.processes;
    mark("processes", file);
  } else if (candidate?.processes.length) {
    processes = candidate.processes;
    mark("processes", found);
  }
  processes = processes.map(normalized);
  if (file) mark("follow_repo_file", file);

  const targets = routable(processes);
  const routes: RouteInput[] = manifest?.routes.length
    ? manifest.routes
    : [{ path: "/", process: targets.includes("web") ? "web" : (targets[0] ?? "web"), websocket: false }];

  const draft: Draft = {
    slug: slugify(name),
    name,
    git_ref: gitRef,
    root,
    runtime,
    toolchain: runtime,
    runtime_version: candidate?.version ?? "",
    install: command("install"),
    build: command("build"),
    migrate: command("migrate"),
    startup_budget_secs: candidate?.health.startup_budget_secs ?? 60,
    cpu_percent: 100,
    pause_for_migrations: true,
    follow_repo_file: manifest !== null,
    processes,
    routes,
    packages: manifest?.packages ?? [],
    domains: [],
  };
  if (draft.packages.length && file) mark("packages", file);
  return { draft, sources };
}

function inputOf(p: Process): ProcessInput {
  return p.kind === "folder"
    ? { name: p.name, static_dir: p.static_dir ?? "", dir: p.dir, port: false, health: null, memory_mb: p.memory_mb }
    : {
        name: p.name,
        start: p.start ?? "",
        dir: p.dir,
        port: p.port !== null,
        health: p.health_path,
        memory_mb: p.memory_mb,
      };
}

export function draftFromApp(app: App): Draft {
  return {
    slug: app.slug,
    name: app.name,
    git_ref: app.git_ref,
    root: app.root,
    runtime: app.runtime,
    toolchain: app.runtime,
    runtime_version: app.runtime_version,
    install: app.commands.install ?? "",
    build: app.commands.build ?? "",
    migrate: app.commands.migrate ?? "",
    startup_budget_secs: app.startup_budget_secs,
    cpu_percent: app.cpu_percent,
    pause_for_migrations: app.pause_for_migrations,
    follow_repo_file: app.follow_repo_file,
    processes: app.processes.map(inputOf),
    routes: app.routes.map((r) => ({ path: r.path, process: r.process, websocket: r.websocket })),
    packages: app.packages,
    domains: app.domains.map(({ wildcard: _, ...d }) => d),
  };
}

function outputOf(p: ProcessInput): ProcessInput {
  const memory = p.memory_mb !== null && p.memory_mb !== undefined ? { memory_mb: p.memory_mb } : {};
  const base = { name: p.name.trim(), dir: (p.dir ?? "").trim(), ...memory };
  if (isFolder(p)) return { ...base, static_dir: (p.static_dir ?? "").trim(), port: false, health: null };
  const listens = p.port !== false;
  return { ...base, start: (p.start ?? "").trim(), port: listens, health: listens ? orNull(p.health ?? "") : null };
}

function fields(d: Draft): Omit<NewApp, "slug" | "repository" | "env" | "env_required"> {
  return {
    name: d.name.trim(),
    git_ref: d.git_ref.trim(),
    root: d.root.trim(),
    runtime: d.runtime,
    toolchain: d.runtime,
    runtime_version: d.runtime_version.trim(),
    commands: { install: orNull(d.install), build: orNull(d.build), migrate: orNull(d.migrate) },
    startup_budget_secs: d.startup_budget_secs,
    cpu_percent: d.cpu_percent,
    pause_for_migrations: d.pause_for_migrations,
    follow_repo_file: d.follow_repo_file,
    processes: d.processes.map(outputOf),
    routes: d.routes.map((r) => ({ ...r, path: r.path.trim() })),
    packages: d.packages,
    domains: d.domains,
  };
}

export const toChanges = (d: Draft): AppChanges => fields(d);

export function toNewApp(d: Draft, repository: string): NewApp {
  return { ...fields(d), slug: d.slug.trim(), repository, env: [], env_required: [] };
}

function nextName(processes: ProcessInput[]) {
  const taken = new Set(processes.map((p) => p.name));
  if (!taken.has("worker")) return "worker";
  let n = 2;
  while (taken.has(`worker_${n}`)) n++;
  return `worker_${n}`;
}

export function ConfigForm({
  draft,
  repository,
  onChange,
  sources = {},
  creating,
  onToolchainChange,
}: {
  draft: Draft;
  repository: string;
  onChange: (d: Draft) => void;
  sources?: Sources;
  creating: boolean;
  /** Called instead of `onChange` when the toolchain changes, with the version cleared. */
  onToolchainChange?: (d: Draft) => void;
}) {
  const set = <K extends keyof Draft>(field: K, value: Draft[K]) => onChange({ ...draft, [field]: value });
  const tags = useGithubTags(repository);
  const tagNames = (tags.data ?? []).map((t) => t.name);
  const refOptions = tagNames.includes(draft.git_ref) ? tagNames : [draft.git_ref, ...tagNames];
  const file = sources.follow_repo_file;
  const locked = draft.follow_repo_file && file !== undefined;
  const lockedCommand = (field: CommandField) => locked && sources[field] === file;
  const targets = routable(draft.processes);
  const hasFolder = draft.processes.some(isFolder);

  const pick = (runtime: Runtime) => {
    const next = { ...draft, runtime, toolchain: runtime };
    if (runtime !== draft.toolchain && onToolchainChange) {
      onToolchainChange({ ...next, runtime_version: "" });
    } else {
      onChange(next);
    }
  };
  const setProcesses = (processes: ProcessInput[], routes = draft.routes, domains = draft.domains) =>
    onChange({
      ...draft,
      processes,
      routes: reconcileRoutes(processes, routes),
      domains: reconcileDomains(processes, domains),
    });
  const setProcess = (i: number, next: ProcessInput) => {
    const previous = draft.processes[i];
    const renamed = previous.name !== next.name;
    setProcesses(
      replaceAt(draft.processes, i, next),
      renamed ? renameInRoutes(draft.routes, previous.name, next.name) : draft.routes,
      renamed ? renameInDomains(draft.domains, previous.name, next.name) : draft.domains,
    );
  };

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader title="Application" hint="The slug names the system user, the units and the directory" />
        <CardBody className="grid gap-4 sm:grid-cols-2">
          <Field label="Name">
            <input
              value={draft.name}
              onChange={(e) =>
                onChange(
                  creating
                    ? { ...draft, name: e.target.value, slug: slugify(e.target.value) }
                    : { ...draft, name: e.target.value },
                )
              }
              className={INPUT}
            />
          </Field>
          <Field label="Slug" hint={creating ? undefined : "Cannot change after creation"}>
            <input
              value={draft.slug}
              disabled={!creating}
              onChange={(e) => set("slug", e.target.value)}
              className={MONO}
            />
          </Field>
          <Field label="Tag" source={sources.git_ref} hint="Every tag you push deploys and becomes the tag here">
            <select value={draft.git_ref} onChange={(e) => set("git_ref", e.target.value)} className={MONO}>
              {refOptions.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Root directory" hint="Leave empty for the repository root">
            <input
              value={draft.root}
              onChange={(e) => set("root", e.target.value)}
              placeholder="apps/web"
              className={MONO}
            />
          </Field>
          <SharedDirHint slug={draft.slug} className="sm:col-span-2" />
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="Runtime" hint="Toolchains install into /var/lib/ferrum/runtimes, one version per app" />
        <CardBody className="grid gap-4 sm:grid-cols-2">
          <Field label="Runtime" source={sources.runtime}>
            <select value={draft.runtime} onChange={(e) => pick(e.target.value as Runtime)} className={INPUT}>
              {RUNTIMES.map((r) => (
                <option key={r} value={r}>
                  {runtimeLabel(r)}
                </option>
              ))}
            </select>
          </Field>
          <Field
            label={`${runtimeLabel(draft.runtime)} version`}
            source={sources.runtime_version}
            hint={draft.runtime === "dotnet" ? "A channel, such as 10.0" : "A full version, such as 22.11.0"}
          >
            <input
              value={draft.runtime_version}
              onChange={(e) => set("runtime_version", e.target.value)}
              className={MONO}
            />
          </Field>
        </CardBody>
      </Card>

      <Card>
        <CardHeader
          title="Processes"
          hint="What runs under systemd, and what nginx serves from the release"
          action={locked ? <Badge tone="accent">{file}</Badge> : null}
        />
        <CardBody className="grid gap-3">
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <div className="min-w-0">
              <p className="text-[13px] text-ink-2">Follow the repo&apos;s file</p>
              <p className="text-[12px] text-ink-4 mt-0.5">
                <Code>ferrum.toml</Code> sets processes, paths, commands, packages and required variables on every deploy
              </p>
            </div>
            <Segmented
              value={draft.follow_repo_file ? "on" : "off"}
              onChange={(v) => set("follow_repo_file", v === "on")}
              options={[...SWITCH]}
            />
          </div>
          {!locked && sources.processes && sources.processes !== file ? (
            <div>
              <Badge tone="accent" className="max-w-full">
                <span className="truncate">Detected — {sources.processes}</span>
              </Badge>
            </div>
          ) : null}
          {draft.processes.map((p, i) => (
            <ProcessRow
              key={i}
              process={p}
              locked={locked}
              removable={draft.processes.length > 1}
              onChange={(next) => setProcess(i, next)}
              onRemove={() => setProcesses(draft.processes.filter((_, j) => j !== i))}
            />
          ))}
          {locked ? null : (
            <div>
              <Button
                size="sm"
                variant="ghost"
                onClick={() =>
                  setProcesses([
                    ...draft.processes,
                    { name: nextName(draft.processes), start: "", dir: "", port: false, health: null },
                  ])
                }
              >
                <Plus size={13} />
                Add process
              </Button>
            </div>
          )}
        </CardBody>
        <CardFoot>
          {locked ? (
            <span>
              The repo&apos;s file decides processes, paths and commands on every deploy. Memory limits stay
              editable here, because they belong to the server.
            </span>
          ) : (
            <span>
              Each process runs as its own systemd unit. A process that listens gets its own port as{" "}
              <Code>PORT</Code>; every listening process is also named in the shared env as{" "}
              <Code>NAME_PORT</Code>.
            </span>
          )}
        </CardFoot>
      </Card>

      <Card>
        <CardHeader title="Commands" hint="Run as the app user, through sh -c, from the release directory" />
        <CardBody className="grid gap-4">
          <Field label="Install" source={sources.install}>
            <input
              value={draft.install}
              disabled={lockedCommand("install")}
              onChange={(e) => set("install", e.target.value)}
              className={MONO}
            />
          </Field>
          <Field
            label="Build"
            source={sources.build}
            hint={hasFolder ? "A folder process needs the build that produces it" : undefined}
          >
            <input
              value={draft.build}
              disabled={lockedCommand("build")}
              onChange={(e) => set("build", e.target.value)}
              className={MONO}
            />
          </Field>
          <Field label="Migrations" source={sources.migrate} hint="Empty means none run">
            <input
              value={draft.migrate}
              disabled={lockedCommand("migrate")}
              onChange={(e) => set("migrate", e.target.value)}
              className={MONO}
            />
          </Field>
          <label className="flex items-center gap-2 text-[13px] text-ink-2">
            <input
              type="checkbox"
              checked={draft.pause_for_migrations}
              onChange={(e) => set("pause_for_migrations", e.target.checked)}
            />
            Pause traffic while migrations run
          </label>
        </CardBody>
      </Card>

      <Card>
        <CardHeader
          title="Paths"
          hint="Which process answers each path"
          action={locked ? <Badge tone="accent">{file}</Badge> : null}
        />
        <CardBody className="grid gap-2">
          {draft.routes.map((route, i) => (
            <div key={i} className="flex items-center gap-2 flex-wrap">
              <input
                value={route.path}
                disabled={locked}
                onChange={(e) => set("routes", replaceAt(draft.routes, i, { ...route, path: e.target.value }))}
                placeholder="/"
                aria-label="Path"
                className={`${MONO} w-auto flex-1 min-w-[7rem]`}
              />
              <span className="text-ink-4">→</span>
              <select
                value={route.process}
                disabled={locked}
                aria-label="Process"
                onChange={(e) => set("routes", replaceAt(draft.routes, i, { ...route, process: e.target.value }))}
                className={`${MONO} w-auto flex-1 min-w-[7rem] sm:flex-none sm:w-40`}
              >
                {(targets.includes(route.process) ? targets : [route.process, ...targets]).map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
              <label className="flex items-center gap-1.5 text-[12.5px] text-ink-3">
                <input
                  type="checkbox"
                  checked={route.websocket}
                  disabled={locked}
                  onChange={(e) =>
                    set("routes", replaceAt(draft.routes, i, { ...route, websocket: e.target.checked }))
                  }
                />
                WebSocket
              </label>
              <Button
                size="icon"
                variant="ghost"
                aria-label="Remove path"
                disabled={locked || draft.routes.length === 1}
                onClick={() => set("routes", draft.routes.filter((_, j) => j !== i))}
              >
                <X size={14} />
              </Button>
            </div>
          ))}
          {locked ? null : (
            <div>
              <Button
                size="sm"
                variant="ghost"
                disabled={targets.length === 0}
                onClick={() =>
                  set("routes", [
                    ...draft.routes,
                    { path: "/ws", process: targets[targets.length - 1] ?? "", websocket: true },
                  ])
                }
              >
                <Plus size={13} />
                Add path
              </Button>
            </div>
          )}
        </CardBody>
        <CardFoot>
          <span>
            A path applies on every served name. A process without a path answers /. WebSocket paths get a
            24-hour read timeout.
          </span>
        </CardFoot>
      </Card>

      <Card>
        <CardHeader title="Domains" hint="Each name serves a process or redirects to another name" />
        <CardBody>
          <DomainsEditor
            domains={draft.domains}
            processes={routable(draft.processes)}
            onChange={(domains) => set("domains", domains)}
          />
        </CardBody>
        <CardFoot>
          <span>Ferrum never writes DNS records. Point each name at this server before deploying.</span>
        </CardFoot>
      </Card>

      <Card>
        <CardHeader title="System packages" hint="apt packages installed before the first build" />
        <CardBody>
          <ListEditor
            items={draft.packages}
            onChange={(items) => set("packages", items)}
            placeholder="ffmpeg"
            mono
            source={sources.packages}
          />
        </CardBody>
        <CardFoot className="flex-col items-start gap-1">
          <span>
            Package names must match <Code>^[a-z0-9][a-z0-9+._-]*$</Code>.
          </span>
          <span>
            Packages are system-wide and shared by every application on the box, so two
            applications needing conflicting versions of the same library will collide.
          </span>
          <span>
            A deploy adds what the tag&apos;s <Code>ferrum.toml</Code> lists and keeps what it dropped,
            so the list only shrinks here. A package removed here is uninstalled on Save, unless
            another application lists it or the server had it before Ferrum.
          </span>
        </CardFoot>
      </Card>

      <Card>
        <CardHeader title="Limits and health" hint="CPUQuota for every unit, and how long a deploy waits to be healthy" />
        <CardBody className="grid gap-4 sm:grid-cols-2">
          <Field label="CPU (%)" hint="100% is one core">
            <input
              type="number"
              min={10}
              value={draft.cpu_percent}
              onChange={(e) => set("cpu_percent", Number(e.target.value))}
              className={MONO}
            />
          </Field>
          <Field label="Startup budget (seconds)" hint="How long a deploy waits for the health checks">
            <input
              type="number"
              min={5}
              value={draft.startup_budget_secs}
              onChange={(e) => set("startup_budget_secs", Number(e.target.value))}
              className={MONO}
            />
          </Field>
        </CardBody>
      </Card>
    </div>
  );
}

function ProcessRow({
  process: p,
  locked,
  removable,
  onChange,
  onRemove,
}: {
  process: ProcessInput;
  locked: boolean;
  removable: boolean;
  onChange: (p: ProcessInput) => void;
  onRemove: () => void;
}) {
  const folder = isFolder(p);
  const listens = !folder && p.port !== false;
  const toKind = (kind: "command" | "folder") =>
    onChange(
      kind === "folder"
        ? { name: p.name, dir: p.dir ?? "", static_dir: "", start: null, port: false, health: null, memory_mb: p.memory_mb }
        : { name: p.name, dir: p.dir ?? "", start: "", port: true, health: "/", memory_mb: p.memory_mb },
    );

  return (
    <div className="grid gap-3 border border-line rounded-inset p-3">
      <div className="flex items-center gap-2 flex-wrap">
        <input
          value={p.name}
          disabled={locked}
          aria-label="Process name"
          onChange={(e) => onChange({ ...p, name: e.target.value.toLowerCase() })}
          placeholder="web"
          maxLength={16}
          className={`${MONO} w-auto flex-1 min-w-[8rem] sm:flex-none sm:w-44`}
        />
        {locked ? (
          <Badge>{folder ? "Folder" : "Command"}</Badge>
        ) : (
          <Segmented value={folder ? "folder" : "command"} onChange={toKind} options={[...KINDS]} />
        )}
        {locked ? null : (
          <Button
            size="icon"
            variant="ghost"
            aria-label={`Remove ${p.name || "process"}`}
            disabled={!removable}
            onClick={onRemove}
            className="ml-auto"
          >
            <X size={14} />
          </Button>
        )}
      </div>
      {folder ? (
        <Field label="Folder" hint="Relative to the application; the build produces it and nginx serves it">
          <input
            value={p.static_dir ?? ""}
            disabled={locked}
            onChange={(e) => onChange({ ...p, static_dir: e.target.value })}
            placeholder="dist"
            className={MONO}
          />
        </Field>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="sm:col-span-2">
            <Field label="Start command">
              <input
                value={p.start ?? ""}
                disabled={locked}
                onChange={(e) => onChange({ ...p, start: e.target.value })}
                placeholder="bun run start"
                className={MONO}
              />
            </Field>
          </div>
          <Field label="Start folder" hint="Relative to the application">
            <input
              value={p.dir ?? ""}
              disabled={locked}
              onChange={(e) => onChange({ ...p, dir: e.target.value })}
              placeholder="apps/web"
              className={MONO}
            />
          </Field>
          <Field label="Memory (MB)" hint="MemoryMax for this unit">
            <input
              type="number"
              min={64}
              value={p.memory_mb ?? ""}
              onChange={(e) => onChange({ ...p, memory_mb: e.target.value === "" ? null : Number(e.target.value) })}
              placeholder="512"
              className={MONO}
            />
          </Field>
          <label className="flex items-center gap-2 text-[13px] text-ink-2 sm:self-end sm:h-9">
            <input
              type="checkbox"
              checked={listens}
              disabled={locked}
              onChange={(e) =>
                onChange({ ...p, port: e.target.checked, health: e.target.checked ? (p.health ?? "/") : null })
              }
            />
            Listens on a port
          </label>
          {listens ? (
            <Field label="Health check path">
              <input
                value={p.health ?? ""}
                disabled={locked}
                onChange={(e) => onChange({ ...p, health: e.target.value })}
                placeholder="/"
                className={MONO}
              />
            </Field>
          ) : null}
        </div>
      )}
    </div>
  );
}

function replaceAt<T>(items: T[], index: number, value: T) {
  return items.map((item, i) => (i === index ? value : item));
}

function Field({
  label,
  hint,
  source,
  children,
}: {
  label: string;
  hint?: string;
  source?: string;
  children: ReactNode;
}) {
  return (
    <div className="min-w-0">
      <div className="flex items-center gap-2 mb-1.5 flex-wrap">
        <label className="text-[13px] text-ink-3">{label}</label>
        {source ? (
          <Badge tone="accent" className="max-w-full">
            <span className="truncate">{source.startsWith("from ") ? source : `Detected — ${source}`}</span>
          </Badge>
        ) : null}
      </div>
      {children}
      {hint ? <p className="text-[12px] text-ink-4 mt-1.5">{hint}</p> : null}
    </div>
  );
}

function ListEditor({
  items,
  onChange,
  placeholder,
  mono,
  source,
}: {
  items: string[];
  onChange: (items: string[]) => void;
  placeholder: string;
  mono?: boolean;
  source?: string;
}) {
  const [pending, setPending] = useState("");
  const add = () => {
    const value = pending.trim().toLowerCase();
    if (!value || items.includes(value)) return;
    onChange([...items, value]);
    setPending("");
  };

  return (
    <div className="grid gap-2">
      {items.length ? (
        <div className="flex flex-wrap gap-1.5">
          {items.map((item) => (
            <Badge key={item} mono={mono}>
              {item}
              <button
                onClick={() => onChange(items.filter((i) => i !== item))}
                aria-label={`Remove ${item}`}
                className="text-ink-4 hover:text-ink"
              >
                <X size={11} />
              </button>
            </Badge>
          ))}
          {source ? <Badge tone="accent">Detected — {source}</Badge> : null}
        </div>
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
          placeholder={placeholder}
          className={`${mono ? MONO : INPUT} flex-1`}
        />
        <Button variant="secondary" onClick={add}>
          Add
        </Button>
      </div>
    </div>
  );
}
