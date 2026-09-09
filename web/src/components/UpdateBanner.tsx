import { ArrowUpCircle, ShieldAlert } from "lucide-react";
import { useUpdate } from "@/lib/api";
import { summary } from "@/lib/release";
import { cn } from "@/lib/utils";
import { UpdateAction } from "./UpdateAction";

const STEPS: Record<string, string> = {
  download: "Downloading the release",
  verify: "Verifying the signature and checksum",
  "self-check": "Running the new binary's self-check",
  install: "Installing the new binary",
  restart: "Scheduling the restart",
};

export function UpdateBanner() {
  const { data } = useUpdate();

  if (!data?.latest || (!data.available && !data.restarting)) return null;
  const { latest } = data;
  const security = latest.security;

  return (
    <div
      role="status"
      className={cn(
        "shrink-0 border-b px-5 py-2.5 flex items-center gap-3 flex-wrap",
        security ? "bg-fail-soft border-fail/25" : "bg-accent-soft border-accent/20",
      )}
    >
      {security ? (
        <ShieldAlert size={15} className="text-fail shrink-0" />
      ) : (
        <ArrowUpCircle size={15} className="text-accent shrink-0" />
      )}
      <div className="min-w-0 flex-1 text-[13px]">
        {data.restarting ? (
          <span className="text-ink">
            Ferrum {latest.tag} is installed and restarts in a moment. This tab reloads when the new
            build answers.
          </span>
        ) : data.running ? (
          <span className="text-ink">
            Updating to {latest.tag}: {STEPS[data.step ?? ""] ?? "working"}…
          </span>
        ) : (
          <>
            <span className={cn("font-medium", security ? "text-fail" : "text-ink")}>
              {security ? "Security release: " : ""}Ferrum {latest.tag} is available.
            </span>{" "}
            <span className="text-ink-3">{summary(latest.notes)}</span>{" "}
            <a href={latest.url} target="_blank" rel="noreferrer" className="text-accent hover:underline">
              Release notes
            </a>
            {data.error ? <span className="text-fail"> · {data.error}</span> : null}
          </>
        )}
      </div>
      {!data.running && !data.restarting ? <UpdateAction security={security} error={data.error} /> : null}
    </div>
  );
}
