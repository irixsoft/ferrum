import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Row } from "@/components/ui/Row";
import { Segmented } from "@/components/ui/Segmented";
import { ApiError, usePushPrefs, useSetPushPref, useTestPush, type PushTried } from "@/lib/api";
import { HOME_SCREEN, needsHomeScreen, supported, useThisDevice } from "@/lib/push";
import type { PushPref } from "@/types/api";

const KINDS: Array<{ pref: PushPref; label: string }> = [
  { pref: "deploy_failed", label: "Deploy refused or failed" },
  { pref: "deploy_live", label: "Deploy went live" },
  { pref: "broke", label: "Something broke on its own" },
  { pref: "update", label: "Update available" },
];

const SWITCH = [
  { value: "off", label: "Off" },
  { value: "on", label: "On" },
] as const;

type Switch = (typeof SWITCH)[number]["value"];

function outcome(tried: PushTried[]): string {
  if (tried.length === 0) return "No device is set up to receive notifications yet.";
  const delivered = tried.filter((t) => t.result === "delivered").length;
  const gone = tried.filter((t) => t.result === "gone").length;
  const parts = [`Sent to ${delivered} of ${tried.length} device${tried.length > 1 ? "s" : ""}.`];
  if (gone) parts.push(`${gone} had unsubscribed and ${gone > 1 ? "were" : "was"} removed.`);
  const refused = tried.find((t) => t.result === "rejected");
  if (refused) parts.push(`A push service refused one with ${refused.status}.`);
  if (tried.some((t) => t.result === "failed")) parts.push("One could not be reached; the service log has why.");
  return parts.join(" ");
}

export function NotificationsCard() {
  const device = useThisDevice();
  const { data: prefs } = usePushPrefs();
  const setPref = useSetPushPref();
  const test = useTestPush();
  const homeScreen = needsHomeScreen();
  const canPush = supported();

  const deviceHint = homeScreen
    ? HOME_SCREEN
    : canPush
      ? "Asks this browser for permission the first time"
      : "This browser cannot receive notifications";

  return (
    <Card>
      <CardHeader
        title="Notifications"
        hint="Pushed to your phone or this browser when something needs you"
        action={
          <Button size="sm" disabled={test.isPending} onClick={() => test.mutate()}>
            {test.isPending ? "Sending…" : "Send a test notification"}
          </Button>
        }
      />
      <CardBody>
        <dl>
          <Row label="Notify this device" hint={deviceHint}>
            <Segmented<Switch>
              value={device.on ? "on" : "off"}
              onChange={(v) => {
                if (v === "on" && !device.on) void device.enable();
                if (v === "off" && device.on) void device.disable();
              }}
              options={[...SWITCH]}
              className={!canPush || device.busy ? "opacity-45 pointer-events-none" : undefined}
            />
          </Row>
          {KINDS.map(({ pref, label }) => (
            <Row key={pref} label={label}>
              <Segmented<Switch>
                value={prefs?.enabled[pref] === false ? "off" : "on"}
                onChange={(v) => setPref.mutate({ [pref]: v === "on" })}
                options={[...SWITCH]}
              />
            </Row>
          ))}
        </dl>
        {device.error ? <p className="text-[12.5px] text-fail mt-3">{device.error}</p> : null}
        {test.data ? <p className="text-[12.5px] text-ink-3 mt-3">{outcome(test.data)}</p> : null}
        {test.error ? (
          <p className="text-[12.5px] text-fail mt-3">
            {test.error instanceof ApiError ? test.error.message : String(test.error)}
          </p>
        ) : null}
      </CardBody>
      <CardFoot>
        <span>
          Notifications go straight from this server to your device's push service, encrypted so
          only that device can read them. No third-party notification account is involved.
        </span>
      </CardFoot>
    </Card>
  );
}
