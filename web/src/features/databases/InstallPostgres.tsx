import { ApiError, useInstallPostgres, usePostgres } from "@/lib/api";
import { Button } from "@/components/ui/Button";

/** apt runs in the background; the status query polls until it settles. */
export function InstallPostgres({ compact = false }: { compact?: boolean }) {
  const { data: postgres } = usePostgres();
  const install = useInstallPostgres();
  const error = install.error instanceof ApiError ? install.error.message : postgres?.error;
  const installing = postgres?.installing || install.isPending;

  return (
    <div className={compact ? "flex items-center gap-3 flex-wrap" : "grid gap-3"}>
      {compact ? null : (
        <p className="text-[13.5px] text-ink-2 leading-relaxed">
          PostgreSQL is not installed. Ferrum installs the newest major from the PostgreSQL apt
          repository, tunes it for this box's memory, binds it to loopback and pins the major so
          a repository moving on never becomes a surprise upgrade.
        </p>
      )}
      <div className="flex items-center gap-3 flex-wrap">
        <Button variant="primary" size="sm" disabled={installing} onClick={() => install.mutate()}>
          {installing ? "Installing…" : "Install PostgreSQL"}
        </Button>
        {installing ? (
          <span className="text-[12.5px] text-ink-4">This takes about a minute.</span>
        ) : null}
        {error && !installing ? <span className="text-[12.5px] text-fail">{error}</span> : null}
      </div>
    </div>
  );
}
