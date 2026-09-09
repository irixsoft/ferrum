import { useState } from "react";
import { ApiError, useApplyUpdate } from "@/lib/api";
import { Button } from "./ui/Button";

export function UpdateAction({ security, error }: { security: boolean; error: string | null }) {
  const apply = useApplyUpdate();
  const [confirming, setConfirming] = useState(false);
  const failed = apply.error instanceof ApiError ? apply.error.message : error;

  if (confirming) {
    return (
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-[12.5px] text-ink-2">
          Ferrum restarts for a few seconds; your applications keep running.
        </span>
        <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>
          Not now
        </Button>
        <Button
          size="sm"
          variant={security ? "danger" : "primary"}
          disabled={apply.isPending}
          onClick={async () => {
            await apply.mutateAsync().catch(() => undefined);
            setConfirming(false);
          }}
        >
          Update now
        </Button>
      </div>
    );
  }
  return (
    <Button size="sm" variant={security ? "danger" : "primary"} onClick={() => setConfirming(true)}>
      {failed ? "Try again" : "Update now"}
    </Button>
  );
}
