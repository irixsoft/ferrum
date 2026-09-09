import { useState } from "react";
import { ApiError, useCommandRuns, useRunCommand } from "@/lib/api";
import { NEVER_LIVE } from "@/components/StatusPill";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { ago, cn } from "@/lib/utils";
import { CommandLog } from "./CommandLog";
import type { CommandRun } from "@/types/api";

const INPUT =
  "w-full h-9 px-3 bg-inset border border-line-strong rounded-control font-mono text-[13px] text-ink placeholder:text-ink-4 disabled:opacity-50";

export function RunPanel({ slug, neverLive, deploying }: { slug: string; neverLive: boolean; deploying: boolean }) {
  const { data: runs = [] } = useCommandRuns(slug);
  const run = useRunCommand(slug);
  const [command, setCommand] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const open = runs.find((r) => r.finished_at === null);
  const shown = selected ?? open?.id ?? runs[0]?.id ?? null;
  const blocked = neverLive || deploying || open !== undefined || run.isPending;
  const error = run.error instanceof ApiError ? run.error.message : run.error ? String(run.error) : null;

  const submit = async () => {
    if (blocked || command.trim() === "") return;
    const started = await run.mutateAsync(command).catch(() => null);
    if (started) {
      setSelected(started.id);
      setCommand("");
    }
  };

  return (
    <div className="grid gap-4">
      <Card>
        <CardHeader title="Run a command" hint="In the app's current release, as its user, with its environment" />
        <CardBody>
          <form
            className="flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <input
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              disabled={neverLive}
              placeholder="bun run seed:admin"
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              className={INPUT}
            />
            <Button
              type="submit"
              size="md"
              variant="primary"
              disabled={blocked || command.trim() === ""}
              title={neverLive ? NEVER_LIVE : deploying ? "Wait for the deploy to finish" : undefined}
            >
              {open ? "Running…" : "Run"}
            </Button>
          </form>
          {error ? <p className="text-[12.5px] text-fail mt-2">{error}</p> : null}
          {shown ? <CommandLog id={shown} className="mt-4" /> : null}
        </CardBody>
        <CardFoot>
          <span>
            {neverLive
              ? NEVER_LIVE
              : "The output is kept with the last 20 runs. A command is stopped after the migration time limit."}
          </span>
        </CardFoot>
      </Card>

      {runs.length > 0 ? (
        <Card>
          <CardHeader title="Previous runs" />
          <div className="px-5 pb-4 divide-y divide-line">
            {runs.map((r) => (
              <RunRow key={r.id} run={r} active={r.id === shown} onSelect={() => setSelected(r.id)} />
            ))}
          </div>
        </Card>
      ) : null}
    </div>
  );
}

function RunRow({ run, active, onSelect }: { run: CommandRun; active: boolean; onSelect: () => void }) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "w-full text-left py-3 flex items-center gap-3",
        active ? "text-ink" : "text-ink-2 hover:text-ink",
      )}
    >
      <span className="font-mono text-[13px] truncate flex-1">{run.command}</span>
      <span
        className={cn(
          "text-[12.5px] shrink-0",
          run.exit === null ? "text-ink-3" : run.exit === "ok" ? "text-ok" : "text-fail",
        )}
      >
        {run.exit === null ? "Running" : run.exit === "ok" ? "Finished" : "Failed"}
      </span>
      <span className="text-[12px] text-ink-4 shrink-0">{ago(run.started_at)}</span>
    </button>
  );
}
