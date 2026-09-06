import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  useRouterState,
} from "@tanstack/react-router";
import { Shell } from "@/shells/Shell";
import { ApiError, useMe } from "@/lib/api";
import { LoginPage } from "@/features/auth/LoginPage";
import { EnrollPage } from "@/features/auth/EnrollPage";
import { DashboardPage } from "@/features/dashboard/DashboardPage";
import { AppsPage } from "@/features/apps/AppsPage";
import { NewAppPage } from "@/features/apps/NewAppPage";
import { AppDetailPage } from "@/features/apps/AppDetailPage";
import { DatabasesPage } from "@/features/databases/DatabasesPage";
import { SystemPage } from "@/features/system/SystemPage";
import { SettingsPage } from "@/features/settings/SettingsPage";

const ENROLL = "/enroll/";

/** `/api/me` decides: the enrollment link is the only route reachable without a session. */
function Gate() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const enrolling = pathname.startsWith(ENROLL);
  const { data, error, isLoading } = useMe(!enrolling);

  if (enrolling) return <Outlet />;
  if (isLoading) return null;
  if (!data) {
    return error instanceof ApiError && error.status === 401 ? <LoginPage /> : <Unreachable />;
  }

  return (
    <Shell>
      <Outlet />
    </Shell>
  );
}

function Unreachable() {
  return (
    <main className="min-h-dvh bg-shell flex items-center justify-center p-5 text-center">
      <p className="text-[13.5px] text-ink-3 max-w-[300px] leading-relaxed">
        Ferrum is not answering. The service may be restarting.
      </p>
    </main>
  );
}

const rootRoute = createRootRoute({ component: Gate });

const enrollRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/enroll/$token",
  component: function EnrollRoute() {
    const { token } = enrollRoute.useParams();
    return <EnrollPage token={token} />;
  },
});

const dashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: DashboardPage,
});

const appsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps",
  component: AppsPage,
});

const newAppRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps/new",
  component: NewAppPage,
});

const appDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps/$slug",
  component: function AppDetailRoute() {
    const { slug } = appDetailRoute.useParams();
    return <AppDetailPage slug={slug} />;
  },
});

const databasesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/databases",
  component: DatabasesPage,
});

const systemRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/system",
  component: SystemPage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsPage,
});

const routeTree = rootRoute.addChildren([
  enrollRoute,
  dashboardRoute,
  appsRoute,
  newAppRoute,
  appDetailRoute,
  databasesRoute,
  systemRoute,
  settingsRoute,
]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  history: import.meta.env.VITE_PREVIEW ? createHashHistory() : undefined,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
