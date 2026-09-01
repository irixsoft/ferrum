import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
} from "@tanstack/react-router";
import { Shell } from "@/shells/Shell";
import { DashboardPage } from "@/features/dashboard/DashboardPage";
import { AppsPage } from "@/features/apps/AppsPage";
import { AppDetailPage } from "@/features/apps/AppDetailPage";
import { DatabasesPage } from "@/features/databases/DatabasesPage";
import { SystemPage } from "@/features/system/SystemPage";
import { SettingsPage } from "@/features/settings/SettingsPage";

const rootRoute = createRootRoute({
  component: () => (
    <Shell>
      <Outlet />
    </Shell>
  ),
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
  dashboardRoute,
  appsRoute,
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
