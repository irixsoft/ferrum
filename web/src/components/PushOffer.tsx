import { useEffect, useState } from "react";
import { current, permission, supported, useThisDevice } from "@/lib/push";
import { cn } from "@/lib/utils";
import { Button } from "./ui/Button";

const SEEN = "ferrum.push-offer";

function seen(): boolean {
  try {
    return localStorage.getItem(SEEN) !== null;
  } catch {
    return true;
  }
}

function remember() {
  try {
    localStorage.setItem(SEEN, "1");
  } catch {
    return;
  }
}

/** Asked once per browser; Settings > About keeps the switch afterwards. */
export function PushOffer({ bottom = "bottom-4" }: { bottom?: string }) {
  const device = useThisDevice();
  const [show, setShow] = useState(false);

  useEffect(() => {
    if (!supported() || permission() !== "default" || seen()) return;
    let cancelled = false;
    void current().then((subscription) => {
      if (!cancelled && !subscription) setShow(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (device.on) {
      remember();
      setShow(false);
    }
  }, [device.on]);

  if (!show) return null;

  const dismiss = () => {
    remember();
    setShow(false);
  };

  return (
    <div className={cn("fixed right-4 z-50 w-[min(24rem,calc(100vw-2rem))]", bottom)}>
      <div className="bg-surface border border-line-strong rounded-card shadow-lift px-4 py-3">
        <p className="text-[13.5px] font-medium text-ink">Get notified on this device?</p>
        <p className="text-[12.5px] text-ink-3 mt-0.5">
          A failed deploy, an app that stops on its own, or a new Ferrum release, sent straight from this server.
        </p>
        {device.error ? <p className="text-[12.5px] text-fail mt-2">{device.error}</p> : null}
        <div className="flex items-center justify-end gap-2 mt-3">
          <Button size="sm" variant="ghost" onClick={dismiss}>
            Not now
          </Button>
          <Button size="sm" variant="primary" disabled={device.busy} onClick={() => void device.enable()}>
            Turn on notifications
          </Button>
        </div>
      </div>
    </div>
  );
}
