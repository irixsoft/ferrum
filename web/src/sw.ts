/// <reference lib="webworker" />
import { cleanupOutdatedCaches, createHandlerBoundToURL, precacheAndRoute } from "workbox-precaching";
import { NavigationRoute, registerRoute } from "workbox-routing";

declare let self: ServiceWorkerGlobalScope;

interface Payload {
  title?: string;
  body?: string;
  link?: string;
}

precacheAndRoute(self.__WB_MANIFEST);
cleanupOutdatedCaches();
// A cached API response would write secrets to disk; nothing under these is ever routed here.
registerRoute(new NavigationRoute(createHandlerBoundToURL("index.html"), { denylist: [/^\/api/, /^\/mcp/] }));

self.addEventListener("message", (event) => {
  if (event.data?.type === "SKIP_WAITING") void self.skipWaiting();
});

self.addEventListener("push", (event) => {
  let payload: Payload = {};
  try {
    payload = event.data?.json() ?? {};
  } catch {
    payload = { body: event.data?.text() };
  }
  const link = payload.link ?? "/";
  event.waitUntil(
    self.registration.showNotification(payload.title ?? "Ferrum", {
      body: payload.body,
      data: { link },
      icon: "/pwa-192.png",
      tag: link,
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const link = (event.notification.data as { link?: string } | null)?.link ?? "/";
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      const open = windows.find((w) => new URL(w.url).origin === self.location.origin);
      if (open) {
        await open.focus();
        await open.navigate(link).catch(() => undefined);
        return;
      }
      await self.clients.openWindow(link);
    })(),
  );
});

self.addEventListener("pushsubscriptionchange", (event) => {
  const change = event as ExtendableEvent & {
    oldSubscription?: PushSubscription | null;
    newSubscription?: PushSubscription | null;
  };
  change.waitUntil(
    (async () => {
      const key = change.oldSubscription?.options.applicationServerKey;
      const next =
        change.newSubscription ??
        (key ? await self.registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key }) : null);
      if (!next) return;
      await fetch("/api/push/devices", {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(next.toJSON()),
      });
    })(),
  );
});
