import { useState, type ReactNode } from "react";
import { Plus, X } from "lucide-react";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Code } from "@/components/ui/Code";
import { Segmented } from "@/components/ui/Segmented";
import { runtimeLabel } from "@/components/RuntimeMark";
import type {
  App,
  AppChanges,
  Detection,
  Detected,
  NewApp,
  RouteInput,
  Runtime,
  Toolchain,
  Tracking,
} from "@/types/api";

export interface Draft {
  slug: string;
  name: string;
  git_ref: string;
  tracking: Tracking;
  root: string;
  runtime: Runtime;
  toolchain: Toolchain;
  runtime_version: string;
  install: string;
  build: string;
  start: string;
  migrate: string;
  output_dir: string;
  health_path: string;
  startup_budget_secs: number;
  memory_mb: number;
  cpu_percent: number;
  pause_for_migrations: boolean;
  routes: RouteInput[];
  packages: string[];
  domains: string[];
}

/** Which fields detection filled and why, keyed by the draft field. */
export type Sources = Partial<Record<keyof Draft, string>>;

const INPUT =
  "w-full h-9 px-3 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 disabled:opacity-50";
const MONO = `${INPUT} font-mono text-[13px]`;

export const slugify = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);

export function draftFromDetection(
  repository: string,
  gitRef: string,
  tracking: Tracking,
  root: string,
  detected: Detected,
  candidate: Detection | null,
): { draft: Draft; sources: Sources } {
  const name = repository.split("/")[1] ?? repository;
  const toml = detected.ferrum_toml;
  const sources: Sources = {};
  const found = candidate?.reasons.join(", ") ?? "";
  const mark = (field: keyof Draft, why: string) => {
    sources[field] = why;
  };

  const runtime = toml?.runtime ?? candidate?.kind ?? "node";
  const toolchain: Toolchain =
    runtime === "static" ? (candidate?.toolchain ?? "node") : (runtime as Toolchain);
  if (toml?.runtime) mark("runtime", "from ferrum.toml");
  else if (candidate) mark("runtime", found);

  const pick = (field: keyof Draft, fromToml: string | null | undefined, fromDetection: string | null | undefined) => {
    if (fromToml) {
      mark(field, "from ferrum.toml");
      return fromToml;
    }
    if (fromDetection) {
      mark(field, found);
      return fromDetection;
    }
    return "";
  };

  const draft: Draft = {
    slug: slugify(name),
    name,
    git_ref: gitRef,
    tracking,
    root,
    runtime,
    toolchain,
    runtime_version: pick("runtime_version", toml?.version, candidate?.version),
    install: pick("install", toml?.install, candidate?.commands.install),
    build: pick("build", toml?.build, candidate?.commands.build),
    start: pick("start", toml?.start, candidate?.commands.start),
    migrate: pick("migrate", toml?.migrate, candidate?.commands.migrate),
    output_dir: pick("output_dir", toml?.output_dir, candidate?.output_dir),
    health_path: toml?.health_path ?? candidate?.health.path ?? "/",
    startup_budget_secs: candidate?.health.startup_budget_secs ?? 60,
    memory_mb: 512,
    cpu_percent: 100,
    pause_for_migrations: true,
    routes: [{ path: "/", port_name: "main", websocket: false }],
    packages: [...new Set([...(toml?.packages ?? []), ...detected.aptfile])],
    domains: [],
  };
  if (draft.packages.length) mark("packages", detected.aptfile.length ? "from Aptfile" : "from ferrum.toml");
  return { draft, sources };
}

export function draftFromApp(app: App): Draft {
  return {
    slug: app.slug,
    name: app.name,
    git_ref: app.git_ref,
    tracking: app.tracking,
    root: app.root,
    runtime: app.runtime,
    toolchain: app.toolchain,
    runtime_version: app.runtime_version,
    install: app.commands.install ?? "",
    build: app.commands.build ?? "",
    start: app.commands.start ?? "",
    migrate: app.commands.migrate ?? "",
    output_dir: app.output_dir ?? "",
    health_path: app.health.path,
    startup_budget_secs: app.health.startup_budget_secs,
    memory_mb: app.memory_mb,
    cpu_percent: app.cpu_percent,
    pause_for_migrations: app.pause_for_migrations,
    routes: app.routes.map((r) => ({ path: r.path, port_name: r.port_name, websocket: r.websocket })),
    packages: app.packages,
    domains: app.domains,
  };
}

const orNull = (s: string) => (s.trim() ? s.trim() : null);

export function toChanges(d: Draft): AppChanges {
  return {
    name: d.name.trim(),
    git_ref: d.git_ref.trim(),
    tracking: d.tracking,
    root: d.root.trim(),
    runtime: d.runtime,
    toolchain: d.toolchain,
    runtime_version: d.runtime_version.trim(),
    commands: {
      install: orNull(d.install),
      build: orNull(d.build),
      start: d.runtime === "static" ? null : orNull(d.start),
      migrate: orNull(d.migrate),
    },
    output_dir: d.runtime === "static" ? d.output_dir.trim() : "",
    health: { path: d.health_path.trim() || "/", startup_budget_secs: d.startup_budget_secs },
    memory_mb: d.memory_mb,
    cpu_percent: d.cpu_percent,
    pause_for_migrations: d.pause_for_migrations,
    routes: d.routes,
    packages: d.packages,
    domains: d.domains,
  };
}

export function toNewApp(d: Draft, repository: string): NewApp {
  const changes = toChanges(d);
  return {
    slug: d.slug.trim(),
    repository,
    env: [],
    ...changes,
    output_dir: d.runtime === "static" ? d.output_dir.trim() : null,
  } as NewApp;
}

export function ConfigForm({
  draft,
  onChange,
  sources = {},
  creating,
}: {
  draft: Draft;
  onChange: (d: Draft) => void;
  sources?: Sources;
  creating: boolean;
}) {
  const set = <K extends keyof Draft>(field: K, value: Draft[K]) =>
    onChange({ ...draft, [field]: value });
  const isStatic = draft.runtime === "static";

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader title="Application" hint="The slug names the system user, the unit and the directory" />
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
          <Field label="Branch or tag" source={sources.git_ref}>
            <input value={draft.git_ref} onChange={(e) => set("git_ref", e.target.value)} className={MONO} />
          </Field>
          <Field label="Deploy on">
            <Segmented
              value={draft.tracking}
              onChange={(v) => set("tracking", v)}
              options={[
                { value: "releases", label: "Releases" },
                { value: "branch", label: "Every push" },
              ]}
            />
            <p className="text-[12px] text-ink-4 mt-1.5">
              {draft.tracking === "releases"
                ? "A published release deploys. Pushes to the branch do not."
                : "Every push to the branch deploys. Fine for a staging box."}
            </p>
          </Field>
          <Field label="Root directory" hint="Leave empty for the repository root">
            <input
              value={draft.root}
              onChange={(e) => set("root", e.target.value)}
              placeholder="apps/web"
              className={MONO}
            />
          </Field>
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="Runtime" hint="Toolchains install into /var/lib/ferrum/runtimes, one version per app" />
        <CardBody className="grid gap-4 sm:grid-cols-2">
          <Field label="Runtime" source={sources.runtime}>
            <select
              value={draft.runtime}
              onChange={(e) => {
                const runtime = e.target.value as Runtime;
                onChange({
                  ...draft,
                  runtime,
                  toolchain: runtime === "static" ? draft.toolchain : (runtime as Toolchain),
                });
              }}
              className={INPUT}
            >
              {(["node", "bun", "static", "dotnet"] as Runtime[]).map((r) => (
                <option key={r} value={r}>
                  {runtimeLabel(r)}
                </option>
              ))}
            </select>
          </Field>
          {isStatic ? (
            <Field label="Built with">
              <select
                value={draft.toolchain}
                onChange={(e) => set("toolchain", e.target.value as Toolchain)}
                className={INPUT}
              >
                {(["node", "bun", "dotnet"] as Toolchain[]).map((r) => (
                  <option key={r} value={r}>
                    {runtimeLabel(r)}
                  </option>
                ))}
              </select>
            </Field>
          ) : null}
          <Field
            label={`${runtimeLabel(draft.toolchain)} version`}
            source={sources.runtime_version}
            hint={draft.toolchain === "dotnet" ? "A channel, such as 10.0" : "A full version, such as 22.11.0"}
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
        <CardHeader title="Commands" hint="Run as the app user, through sh -c, from the release directory" />
        <CardBody className="grid gap-4">
          <Field label="Install" source={sources.install}>
            <input value={draft.install} onChange={(e) => set("install", e.target.value)} className={MONO} />
          </Field>
          <Field label="Build" source={sources.build}>
            <input value={draft.build} onChange={(e) => set("build", e.target.value)} className={MONO} />
          </Field>
          {isStatic ? (
            <Field label="Output directory" source={sources.output_dir} hint="Relative to the release; nginx serves it">
              <input value={draft.output_dir} onChange={(e) => set("output_dir", e.target.value)} className={MONO} />
            </Field>
          ) : (
            <Field label="Start" source={sources.start}>
              <input value={draft.start} onChange={(e) => set("start", e.target.value)} className={MONO} />
            </Field>
          )}
          <Field label="Migrations" source={sources.migrate} hint="Empty means none run">
            <input value={draft.migrate} onChange={(e) => set("migrate", e.target.value)} className={MONO} />
          </Field>
          {isStatic ? null : (
            <label className="flex items-center gap-2 text-[13px] text-ink-2">
              <input
                type="checkbox"
                checked={draft.pause_for_migrations}
                onChange={(e) => set("pause_for_migrations", e.target.checked)}
              />
              Pause traffic while migrations run
            </label>
          )}
        </CardBody>
      </Card>

      {isStatic ? null : (
        <Card>
          <CardHeader title="Routes" hint="Each named port is reserved and injected as PORT, WS_PORT, …" />
          <CardBody className="grid gap-2">
            {draft.routes.map((route, i) => (
              <div key={i} className="flex items-center gap-2 flex-wrap">
                <input
                  value={route.path}
                  onChange={(e) => set("routes", replaceAt(draft.routes, i, { ...route, path: e.target.value }))}
                  placeholder="/"
                  className={`${MONO} w-40 flex-1`}
                />
                <span className="text-ink-4">→</span>
                <input
                  value={route.port_name}
                  onChange={(e) =>
                    set("routes", replaceAt(draft.routes, i, { ...route, port_name: e.target.value }))
                  }
                  placeholder="main"
                  className={`${MONO} w-32`}
                />
                <label className="flex items-center gap-1.5 text-[12.5px] text-ink-3">
                  <input
                    type="checkbox"
                    checked={route.websocket}
                    onChange={(e) =>
                      set("routes", replaceAt(draft.routes, i, { ...route, websocket: e.target.checked }))
                    }
                  />
                  WebSocket
                </label>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label="Remove route"
                  disabled={draft.routes.length === 1}
                  onClick={() => set("routes", draft.routes.filter((_, j) => j !== i))}
                >
                  <X size={14} />
                </Button>
              </div>
            ))}
            <div>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => set("routes", [...draft.routes, { path: "/ws", port_name: "ws", websocket: true }])}
              >
                <Plus size={13} />
                Add route
              </Button>
            </div>
          </CardBody>
          <CardFoot>
            <span>
              The main port is always <Code>PORT</Code>. WebSocket routes get a 24-hour read timeout.
            </span>
          </CardFoot>
        </Card>
      )}

      <Card>
        <CardHeader title="Domains" hint="The first one is primary; the others redirect to it" />
        <CardBody>
          <ListEditor
            items={draft.domains}
            onChange={(items) => set("domains", items)}
            placeholder="app.example.com"
            mono
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
            Package names are validated against <Code>^[a-z0-9][a-z0-9+._-]*$</Code> and passed as
            argv, never interpolated into a shell string.
          </span>
          <span>
            Packages are system-wide and shared by every application on the box, so two
            applications needing conflicting versions of the same library will collide.
          </span>
        </CardFoot>
      </Card>

      {isStatic ? null : (
        <Card>
          <CardHeader title="Limits and health" hint="Written into the systemd unit as MemoryMax and CPUQuota" />
          <CardBody className="grid gap-4 sm:grid-cols-2">
            <Field label="Memory (MB)">
              <input
                type="number"
                min={64}
                value={draft.memory_mb}
                onChange={(e) => set("memory_mb", Number(e.target.value))}
                className={MONO}
              />
            </Field>
            <Field label="CPU (%)" hint="100% is one core">
              <input
                type="number"
                min={10}
                value={draft.cpu_percent}
                onChange={(e) => set("cpu_percent", Number(e.target.value))}
                className={MONO}
              />
            </Field>
            <Field label="Health check path">
              <input
                value={draft.health_path}
                onChange={(e) => set("health_path", e.target.value)}
                className={MONO}
              />
            </Field>
            <Field label="Startup budget (seconds)" hint="How long a deploy waits for the health check">
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
            <span className="truncate">Detected — {source}</span>
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
