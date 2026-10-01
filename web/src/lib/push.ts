import { useEffect, useState } from "react";
import { request, useRegisterDevice, useUnregisterDevice } from "./api";
import { fromBase64Url, toBase64Url } from "./webauthn";

export const HOME_SCREEN = "Add Ferrum to your Home Screen first, then turn this on here.";
const BLOCKED = "Notifications are blocked for this site. Allow them in the browser's site settings, then try again.";
const UNSUPPORTED = "This browser cannot receive notifications.";

export function supported(): boolean {
  return (
    typeof window !== "undefined" &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  );
}

export function standalone(): boolean {
  if (typeof window === "undefined") return false;
  return (
    window.matchMedia?.("(display-mode: standalone)").matches ||
    (navigator as Navigator & { standalone?: boolean }).standalone === true
  );
}

/** Safari on an iPhone or iPad offers push only to a Home Screen web app. */
export function needsHomeScreen(): boolean {
  if (typeof navigator === "undefined" || standalone()) return false;
  return /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
}

export function permission(): NotificationPermission | "unsupported" {
  return supported() ? Notification.permission : "unsupported";
}

export async function current(): Promise<PushSubscription | null> {
  if (!supported()) return null;
  const registration = await navigator.serviceWorker.getRegistration();
  return registration ? registration.pushManager.getSubscription() : null;
}

/** Call straight from a tap: Safari refuses the permission prompt otherwise. */
export async function subscribe(): Promise<PushSubscriptionJSON> {
  if (!supported()) throw new Error(needsHomeScreen() ? HOME_SCREEN : UNSUPPORTED);
  if ((await Notification.requestPermission()) !== "granted") throw new Error(BLOCKED);
  const registration = await navigator.serviceWorker.ready;
  const { key } = await request<{ key: string }>("/push/vapid");
  let subscription = await registration.pushManager.getSubscription();
  const bound = subscription?.options.applicationServerKey;
  if (subscription && (!bound || toBase64Url(bound) !== key)) {
    await subscription.unsubscribe();
    subscription = null;
  }
  subscription ??= await registration.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: fromBase64Url(key),
  });
  return subscription.toJSON();
}

/** The endpoint this browser gave up, so the server can forget it. */
export async function unsubscribe(): Promise<string | null> {
  const subscription = await current();
  if (!subscription) return null;
  await subscription.unsubscribe();
  return subscription.endpoint;
}

/** Whether this browser receives pushes, and the two taps that change it. */
export function useThisDevice() {
  const register = useRegisterDevice();
  const unregister = useUnregisterDevice();
  const [on, setOn] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void current().then((subscription) => {
      if (!cancelled) setOn(subscription !== null && permission() === "granted");
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const run = async (step: () => Promise<boolean>) => {
    setBusy(true);
    setError(null);
    try {
      setOn(await step());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const enable = () =>
    run(async () => {
      await register.mutateAsync(await subscribe());
      return true;
    });

  const disable = () =>
    run(async () => {
      const endpoint = await unsubscribe();
      if (endpoint) await unregister.mutateAsync(endpoint);
      return false;
    });

  return { on, busy, error, enable, disable };
}
