import { Link, useRouterState } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Moon, Sun } from "lucide-react";
import { Wordmark } from "@/components/Brand";
import { DeployRail } from "@/components/DeployLadder";
import { RailButton } from "@/components/ui/RailButton";
import { Pill, PillIcon } from "@/components/ui/Pill";
import { HELP, NAV } from "@/components/nav";
import { useHost, useRunningDeploy } from "@/lib/api";
import { RANGES, useRange } from "@/lib/range";
import { useTheme } from "@/lib/theme";
import { uptime } from "@/lib/utils";

/** Chrome only. Nothing under src/features may import from here. */
export function DesktopShell({ children }: { children: ReactNode }) {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const { data: deploy } = useRunningDeploy();
  const { data: host } = useHost();
  const { range, setRange } = useRange();
  const { resolved, setTheme } = useTheme();

  const isActive = (to: string) => (to === "/" ? path === "/" : path.startsWith(to));
  const head = NAV.filter((n) => !("foot" in n && n.foot));
  const foot = NAV.filter((n) => "foot" in n && n.foot);

  return (
    <div className="h-dvh bg-shell p-3">
      <div className="h-full rounded-shell bg-canvas overflow-hidden flex flex-col">
        <header className="h-20 shrink-0 flex items-center gap-4 px-8">
          <Link to="/" className="text-ink shrink-0" aria-label="Ferrum, dashboard">
            <Wordmark height={26} />
          </Link>

          <div className="ml-auto flex items-center gap-2.5">
            {deploy ? (
              <Link
                to="/apps/$slug"
                params={{ slug: deploy.app_slug }}
                className="flex items-center gap-2.5 h-12 pl-4 pr-5 rounded-full bg-surface border border-line hover:border-line-strong transition-colors duration-100"
              >
                <DeployRail deploy={deploy} />
                <span className="text-[13.5px] text-ink-2">
                  Deploying <span className="font-medium text-ink">{deploy.app_slug}</span>
                </span>
              </Link>
            ) : null}

            {RANGES.map((r) => (
              <Pill key={r.value} active={range === r.value} onClick={() => setRange(r.value)}>
                {r.label}
              </Pill>
            ))}

            <PillIcon
              label={`Switch to ${resolved === "dark" ? "light" : "dark"} theme`}
              onClick={() => setTheme(resolved === "dark" ? "light" : "dark")}
            >
              {resolved === "dark" ? <Sun size={17} /> : <Moon size={17} />}
            </PillIcon>
          </div>
        </header>

        <div className="flex-1 min-h-0 flex">
          <nav className="w-[92px] shrink-0 flex flex-col items-center gap-3 pt-1 pb-8">
            {head.map((n) => (
              <RailButton
                key={n.to}
                to={n.to}
                label={n.label}
                icon={n.icon}
                active={isActive(n.to)}
                dot={n.to === "/apps" && Boolean(deploy)}
              />
            ))}

            <div className="mt-auto flex flex-col items-center gap-3">
              <a
                href="https://github.com/irixsoft/ferrum#readme"
                target="_blank"
                rel="noreferrer"
                aria-label={HELP.label}
                title={HELP.label}
                className="h-12 w-12 grid place-items-center rounded-full bg-surface border border-line text-ink-3 hover:text-ink hover:border-line-strong transition-colors duration-100"
              >
                <HELP.icon size={19} strokeWidth={1.8} />
              </a>
              {foot.map((n) => (
                <RailButton
                  key={n.to}
                  to={n.to}
                  label={n.label}
                  icon={n.icon}
                  active={isActive(n.to)}
                />
              ))}
            </div>
          </nav>

          <main className="flex-1 min-w-0 overflow-y-auto pl-2 pr-8 pb-8">
            <div className="max-w-[1240px]">{children}</div>
          </main>
        </div>

        {host ? (
          <div className="shrink-0 h-9 px-8 flex items-center gap-2 text-[12px] text-ink-4 border-t border-line">
            <span className="h-1.5 w-1.5 rounded-full bg-ok" />
            <span className="font-mono">{host.hostname}</span>
            <span>·</span>
            <span>{host.os}</span>
            <span>·</span>
            <span>up {uptime(host.uptime_secs)}</span>
            <span className="ml-auto font-mono">Ferrum {host.ferrum_version}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
