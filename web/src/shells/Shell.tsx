import type { ReactNode } from "react";
import { DesktopShell } from "./DesktopShell";
import { MobileShell } from "./MobileShell";
import { useShell } from "./useShell";
import { ConnectionBanner } from "@/components/ConnectionBanner";
import { UpdateBanner } from "@/components/UpdateBanner";
import { UpdatePrompt } from "@/components/UpdatePrompt";

export function Shell({ children }: { children: ReactNode }) {
  const { shell } = useShell();
  const Chrome = shell === "desktop" ? DesktopShell : MobileShell;

  return (
    <>
      <ConnectionBanner />
      <UpdateBanner />
      <Chrome>{children}</Chrome>
      <UpdatePrompt />
    </>
  );
}
