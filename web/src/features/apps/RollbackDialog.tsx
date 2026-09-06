import { useState } from "react";
import { ApiError, useRollback } from "@/lib/api";
import { Sheet } from "@/components/ui/Sheet";
import { Button } from "@/components/ui/Button";
import { Code } from "@/components/ui/Code";
import { cn } from "@/lib/utils";
import type { Deploy, Release } from "@/types/api";

type Choice = "code" | "restore";

/**
 * Rolling back code never rolls back schema; the two choices are spelled out and nothing is
 * preselected. `snapshotOf` is the deploy that took the pre-migration snapshot, if any.
 */
export function RollbackDialog({
  slug,
  release,
  snapshotOf,
  onClose,
}: {
  slug: string;
  release: Release | null;
  snapshotOf: Deploy | null;
  onClose: () => void;
}) {
  const [choice, setChoice] = useState<Choice | null>(null);
  const rollback = useRollback(slug);
  const snapshot = snapshotOf?.snapshots[0] ?? null;
  const sha = release?.commit_sha.slice(0, 7) ?? "";

  const close = () => {
    setChoice(null);
    rollback.reset();
    onClose();
  };

  return (
    <Sheet open={release !== null} onClose={close} title={`Roll back to ${sha}`} side="center">
      <div className="grid gap-3">
        <p className="text-[13.5px] text-ink-2">
          <Code>{release?.git_ref}</Code> built {release ? new Date(release.built_at).toLocaleString() : ""}.
          The release is on disk, so this is a repoint and a restart with no build.
        </p>

        <Option
          selected={choice === "code"}
          onSelect={() => setChoice("code")}
          title="Roll back code only"
          body="Instant, all data preserved. Right when the migration was additive, which most are."
        />
        <Option
          selected={choice === "restore"}
          onSelect={() => setChoice("restore")}
          disabled={snapshot === null}
          title="Roll back code and restore the pre-migration snapshot"
          body={
            snapshot
              ? `Schema and data return to the state of ${new Date(snapshot.taken_at).toLocaleString()}. Everything written to ${snapshot.database} since then is lost.`
              : "No snapshot was taken for the deploy that replaced this release."
          }
        />

        {rollback.error ? (
          <p className="text-[12.5px] text-fail">
            {rollback.error instanceof ApiError ? rollback.error.message : String(rollback.error)}
          </p>
        ) : null}

        <div className="flex justify-end gap-2 pt-1">
          <Button variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button
            variant={choice === "restore" ? "danger" : "primary"}
            disabled={choice === null || rollback.isPending || release === null}
            onClick={async () => {
              if (!release || !choice) return;
              await rollback.mutateAsync({
                release_id: release.id,
                restore_deploy_id: choice === "restore" && snapshotOf ? snapshotOf.id : undefined,
              });
              close();
            }}
          >
            {choice === "restore" ? "Roll back and restore" : "Roll back"}
          </Button>
        </div>
      </div>
    </Sheet>
  );
}

function Option({
  selected,
  onSelect,
  disabled,
  title,
  body,
}: {
  selected: boolean;
  onSelect: () => void;
  disabled?: boolean;
  title: string;
  body: string;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onSelect}
      aria-pressed={selected}
      className={cn(
        "text-left rounded-card border px-4 py-3 transition-colors duration-100 disabled:opacity-50",
        selected ? "border-ink bg-inset" : "border-line hover:border-line-strong",
      )}
    >
      <span className="block text-[13.5px] font-medium text-ink">{title}</span>
      <span className="block text-[12.5px] text-ink-3 mt-0.5">{body}</span>
    </button>
  );
}
