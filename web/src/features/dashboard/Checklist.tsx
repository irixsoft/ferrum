import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Circle, CircleCheck } from "lucide-react";
import {
  ApiError,
  useEnableFail2ban,
  useEnableFirewall,
  useEnableUpdates,
  useEnrollmentLink,
  useGithub,
  useHideChecklist,
  useMe,
  usePostgres,
  useSecurity,
  useUsers,
} from "@/lib/api";
import { rows, type ChecklistId, type ChecklistRow } from "@/lib/checklist";
import type { HostStatus, JobStatus } from "@/types/api";
import { Card, CardBody, CardHeader } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { EnableButton, enableFailure } from "@/components/EnableButton";
import { Handoff } from "@/components/Handoff";
import { InstallPostgres } from "@/features/databases/InstallPostgres";

const IDLE: JobStatus = { running: false, error: null };
const message = (e: unknown) => (e instanceof ApiError ? e.message : null);

/** Shown after the first passkey until every row is done or the box is told to hide it. */
export function Checklist({ host }: { host: HostStatus }) {
  const { data: github } = useGithub();
  const { data: postgres } = usePostgres();
  const { data: security, error: securityError } = useSecurity();
  const { data: users } = useUsers();
  const { data: me } = useMe();
  const hide = useHideChecklist();
  const navigate = useNavigate();
  const firewall = useEnableFirewall();
  const fail2ban = useEnableFail2ban();
  const updates = useEnableUpdates();

  const all = rows({ github, postgres, security, users, me });
  const done = all.filter((r) => r.done).length;
  if (host.checklist_hidden || done === all.length) return null;

  const mine = users?.find((u) => u.name === me?.name);
  const hardening: Partial<Record<ChecklistId, Hardening>> = {
    firewall: [security?.jobs.firewall ?? IDLE, firewall, "Enable firewall"],
    fail2ban: [security?.jobs.fail2ban ?? IDLE, fail2ban, "Enable fail2ban"],
    updates: [security?.jobs.updates ?? IDLE, updates, "Enable updates"],
  };
  const action = (row: ChecklistRow) => {
    const job = hardening[row.id];
    if (job) {
      const [status, mutation, label] = job;
      const failed = message(securityError) ?? enableFailure(status, mutation);
      return (
        <>
          {failed ? <span className="text-[12.5px] text-fail">{failed}</span> : null}
          <EnableButton label={label} job={status} mutation={mutation} />
        </>
      );
    }
    if (row.id === "github") {
      return (
        <Button size="sm" variant="primary" onClick={() => navigate({ href: "/settings?tab=connections" })}>
          Connect GitHub
        </Button>
      );
    }
    if (row.id === "postgres") return <InstallPostgres compact />;
    return <SecondPasskey userId={mine?.id} />;
  };
  return (
    <Card>
      <CardHeader
        title="Getting started"
        hint={`${done} of ${all.length} done`}
        action={
          <Button size="sm" variant="ghost" disabled={hide.isPending} onClick={() => hide.mutate(true)}>
            Hide
          </Button>
        }
      />
      <CardBody className="pb-3">
        <ul>
          {all.map((row) => (
            <li key={row.id} className="flex items-center gap-3 py-2.5 border-b border-line last:border-0 flex-wrap">
              {row.done ? (
                <CircleCheck size={15} className="text-ok shrink-0" />
              ) : (
                <Circle size={15} className="text-ink-4 shrink-0" />
              )}
              <span className={row.done ? "text-[13.5px] text-ink-4 line-through" : "text-[13.5px] text-ink"}>
                {row.label}
              </span>
              {row.done ? null : <span className="ml-auto flex items-center gap-3">{action(row)}</span>}
            </li>
          ))}
        </ul>
      </CardBody>
    </Card>
  );
}

type Hardening = [JobStatus, ReturnType<typeof useEnableFirewall>, string];

function SecondPasskey({ userId }: { userId: string | undefined }) {
  const reissue = useEnrollmentLink();
  const [link, setLink] = useState<string | null>(null);
  if (link) return <Handoff label="Open this on the other device" value={link} onDone={() => setLink(null)} />;
  return (
    <>
      {reissue.error ? <span className="text-[12.5px] text-fail">{message(reissue.error)}</span> : null}
      <Button
        size="sm"
        variant="primary"
        disabled={!userId || reissue.isPending}
        onClick={async () => setLink((await reissue.mutateAsync(userId ?? "")).enrollment_url)}
      >
        New enrollment link
      </Button>
    </>
  );
}
