import { useCallback, useEffect, useState } from "react";

/** Read this only where the content itself must differ, never for chrome. */

const KEY = "ferrum.forceDesktop";
const QUERY = "(min-width: 900px)";
/** Below this the desktop override is ignored: a phone cannot render the rail. */
const OVERRIDE_FLOOR = "(min-width: 600px)";

export type Shell = "desktop" | "mobile";

export function useShell(): {
  shell: Shell;
  forceDesktop: boolean;
  setForceDesktop: (v: boolean) => void;
  overridable: boolean;
} {
  const [wide, setWide] = useState(() => window.matchMedia(QUERY).matches);
  const [overridable, setOverridable] = useState(() => window.matchMedia(OVERRIDE_FLOOR).matches);
  const [forceDesktop, setForce] = useState(() => {
    try {
      return localStorage.getItem(KEY) === "1";
    } catch {
      return false;
    }
  });

  useEffect(() => {
    const wideMq = window.matchMedia(QUERY);
    const floorMq = window.matchMedia(OVERRIDE_FLOOR);
    const on = () => {
      setWide(wideMq.matches);
      setOverridable(floorMq.matches);
    };
    wideMq.addEventListener("change", on);
    floorMq.addEventListener("change", on);
    return () => {
      wideMq.removeEventListener("change", on);
      floorMq.removeEventListener("change", on);
    };
  }, []);

  const setForceDesktop = useCallback((v: boolean) => {
    try {
      localStorage.setItem(KEY, v ? "1" : "0");
    } catch {
      /* preference simply does not persist */
    }
    setForce(v);
  }, []);

  const shell: Shell = wide || (forceDesktop && overridable) ? "desktop" : "mobile";
  return { shell, forceDesktop, setForceDesktop, overridable };
}
