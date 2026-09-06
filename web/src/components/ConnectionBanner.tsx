import { useEffect, useState } from "react";
import { CloudOff } from "lucide-react";
import { useHost } from "@/lib/api";

export function ConnectionBanner() {
  const { isError } = useHost();
  const [offline, setOffline] = useState(() => !navigator.onLine);

  useEffect(() => {
    const on = () => setOffline(!navigator.onLine);
    window.addEventListener("online", on);
    window.addEventListener("offline", on);
    return () => {
      window.removeEventListener("online", on);
      window.removeEventListener("offline", on);
    };
  }, []);

  if (!offline && !isError) return null;

  return (
    <div className="bg-fail-soft border-b border-fail/25 px-5 py-2 flex items-center gap-2.5">
      <CloudOff size={14} className="text-fail shrink-0" />
      <p className="text-[13px] text-fail">
        {offline
          ? "This device is offline. Nothing on this screen is current."
          : "Cannot reach the server. Nothing on this screen is current."}
      </p>
    </div>
  );
}
