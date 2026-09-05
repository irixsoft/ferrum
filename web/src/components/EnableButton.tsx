import { ApiError } from "@/lib/api";
import type { JobStatus } from "@/types/api";
import { Button } from "@/components/ui/Button";

interface Mutation {
  isPending: boolean;
  error: unknown;
  mutate: (v: undefined) => void;
}

/** apt runs in the background; the security query polls until the job settles. */
export function EnableButton({ label, job, mutation }: { label: string; job: JobStatus; mutation: Mutation }) {
  const running = job.running || mutation.isPending;
  return (
    <span className="flex items-center gap-3">
      {running ? <span className="text-[12.5px] text-ink-4">About a minute.</span> : null}
      <Button size="sm" variant="primary" disabled={running} onClick={() => mutation.mutate(undefined)}>
        {running ? "Installing…" : label}
      </Button>
    </span>
  );
}

/** The sentence to show under a button once its job is no longer running, if any. */
export function enableFailure(job: JobStatus, mutation: { error: unknown }): string | null {
  if (job.running) return null;
  if (mutation.error instanceof ApiError) return mutation.error.message;
  if (mutation.error) return String(mutation.error);
  return job.error;
}
