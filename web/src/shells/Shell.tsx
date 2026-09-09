import type { ReactNode } from "react";
import { DesktopShell } from "./DesktopShell";
import { MobileShell } from "./MobileShell";
import { useShell } from "./useShell";
import { CommandPalette } from "@/components/CommandPalette";
import { UpdatePrompt } from "@/components/UpdatePrompt";

export function Shell({ children }: { children: ReactNode }) {
  const { shell } = useShell();
  const Chrome = shell === "desktop" ? DesktopShell : MobileShell;

  return (
    <>
      <Chrome>{children}</Chrome>
      <CommandPalette />
      <UpdatePrompt />
    </>
  );
}
