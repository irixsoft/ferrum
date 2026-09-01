import { Boxes, CircleHelp, Database, Gauge, Settings, ShieldCheck } from "lucide-react";

/** `foot` items sink to the bottom of the desktop rail; on mobile every item is a tab. */
export const NAV = [
  { to: "/", label: "Dashboard", icon: Gauge },
  { to: "/apps", label: "Apps", icon: Boxes },
  { to: "/databases", label: "Databases", icon: Database },
  { to: "/system", label: "System", icon: ShieldCheck },
  { to: "/settings", label: "Settings", icon: Settings, foot: true },
] as const;

export const HELP = { label: "Help", icon: CircleHelp } as const;
