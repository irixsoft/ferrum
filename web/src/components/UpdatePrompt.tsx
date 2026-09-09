import { useEffect, useRef, useState } from "react";
import { useRegisterSW } from "virtual:pwa-register/react";
import { useUpdate } from "@/lib/api";
import { Button } from "./ui/Button";

/** Without this a browser keeps running the pre-update bundle against the new API. */
export function UpdatePrompt() {
  const registration = useRef<ServiceWorkerRegistration | null>(null);
  const {
    needRefresh: [needRefresh],
    updateServiceWorker,
  } = useRegisterSW({
    onRegisteredSW(_url, r) {
      registration.current = r ?? null;
    },
  });
  const { data: update } = useUpdate();
  const restarting = update?.restarting ?? false;
  const sawRestart = useRef(false);
  if (restarting) sawRestart.current = true;
  const [skewed, setSkewed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      try {
        const res = await fetch("/api/version", { credentials: "same-origin" });
        if (!res.ok) return;
        const { build_id } = (await res.json()) as { build_id: string };
        if (cancelled || !build_id || build_id === __FERRUM_BUILD_ID__) return;
        setSkewed(true);
        await registration.current?.update();
      } catch {
        // The connection banner owns unreachability.
      }
    };
    check();
    const id = setInterval(check, restarting ? 2_000 : 60_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [restarting]);

  useEffect(() => {
    if (!skewed || !sawRestart.current) return;
    if (needRefresh) void updateServiceWorker(true);
    else if (!registration.current) window.location.reload();
  }, [skewed, needRefresh, updateServiceWorker]);

  if (!needRefresh && !skewed) return null;

  return (
    <div className="fixed bottom-4 left-1/2 -translate-x-1/2 z-50 w-[min(26rem,calc(100vw-2rem))]">
      <div className="bg-surface border border-line-strong rounded-card shadow-lift px-4 py-3 flex items-center gap-4">
        <div className="min-w-0">
          <p className="text-[13.5px] font-medium text-ink">Ferrum was updated</p>
          <p className="text-[12.5px] text-ink-3 mt-0.5">
            This tab is running the previous build. Reload to catch up.
          </p>
        </div>
        <Button size="sm" variant="primary" className="ml-auto shrink-0" onClick={() => updateServiceWorker(true)}>
          Reload
        </Button>
      </div>
    </div>
  );
}
