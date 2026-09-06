import { Link, useRouterState } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { ChevronLeft } from "lucide-react";
import { Mark } from "@/components/Brand";
import { ThemeToggle } from "@/components/ThemeToggle";
import { DeployRail } from "@/components/DeployLadder";
import { NAV } from "@/components/nav";
import { useRunningDeploy } from "@/lib/api";
import { cn } from "@/lib/utils";

/** Chrome only. Nothing under src/features may import from here. */
export function MobileShell({ children }: { children: ReactNode }) {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const { data: deploy } = useRunningDeploy();

  const detail = /^\/apps\/(.+)$/.exec(path);
  const isDetail = detail !== null;
  const heading = detail?.[1] === "new" ? "New app" : detail ? decodeURIComponent(detail[1]) : null;
  const current = NAV.find((n) => (n.to === "/" ? path === "/" : path.startsWith(n.to)));

  return (
    <div className="min-h-dvh flex flex-col bg-canvas">
      <header className="sticky top-0 z-40 bg-canvas/90 backdrop-blur border-b border-line pt-safe">
        <div className="h-13 flex items-center gap-2 px-4">
          {isDetail ? (
            <Link
              to="/apps"
              className="-ml-2 h-9 w-9 grid place-items-center rounded-full text-ink-2 active:bg-inset"
              aria-label="Back to apps"
            >
              <ChevronLeft size={20} />
            </Link>
          ) : (
            <Mark size={18} className="text-ink-3" />
          )}
          <span className="font-display text-[17px] text-ink">
            {heading ?? current?.label ?? "Ferrum"}
          </span>
          <div className="ml-auto flex items-center gap-1">
            {deploy ? (
              <Link
                to="/apps/$slug"
                params={{ slug: deploy.app_slug }}
                className="h-8 px-2.5 rounded-full border border-line bg-surface flex items-center"
                aria-label={`Deploying ${deploy.app_slug}`}
              >
                <DeployRail deploy={deploy} />
              </Link>
            ) : null}
            <ThemeToggle />
          </div>
        </div>
      </header>

      <main className="flex-1 px-4 py-5 pb-28">{children}</main>

      <nav className="fixed inset-x-3 bottom-0 z-40 pb-safe">
        <div className="mb-3 h-16 grid grid-cols-5 rounded-shell bg-surface/95 backdrop-blur border border-line shadow-lift">
          {NAV.map(({ to, label, icon: Icon }) => {
            const active = to === "/" ? path === "/" : path.startsWith(to);
            return (
              <Link
                key={to}
                to={to}
                className={cn(
                  "flex flex-col items-center justify-center gap-1 transition-colors duration-100",
                  active ? "text-ink" : "text-ink-4",
                )}
              >
                <Icon size={19} strokeWidth={active ? 2.2 : 1.8} />
                <span className={cn("text-[10.5px]", active && "font-medium")}>{label}</span>
              </Link>
            );
          })}
        </div>
      </nav>
    </div>
  );
}
