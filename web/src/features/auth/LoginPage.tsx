import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { Code } from "@/components/ui/Code";
import { keys } from "@/lib/api";
import { CANCELLED, signIn, supported } from "@/lib/webauthn";
import { AuthLayout, Notice } from "./AuthLayout";

export function LoginPage() {
  const client = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [usable, setUsable] = useState(true);

  useEffect(() => setUsable(supported()), []);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      await signIn();
      await client.invalidateQueries({ queryKey: keys.me });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Sign-in did not complete.");
    } finally {
      setBusy(false);
    }
  };

  if (!usable) return <NoPasskeys />;

  return (
    <AuthLayout
      above="Ferrum"
      title="Sign in"
      footer={
        <span>
          There are no passwords. If you have lost every passkey, run{" "}
          <Code>ferrum passkey enroll</Code> over SSH for a fresh link.
        </span>
      }
    >
      <p className="text-[13.5px] text-ink-3 leading-relaxed mb-5">
        Your browser will ask for the passkey you registered on this device.
      </p>

      <Button variant="primary" size="lg" className="w-full" onClick={start} disabled={busy}>
        <KeyRound size={16} />
        {busy ? "Waiting for your passkey…" : "Sign in with a passkey"}
      </Button>

      {error ? <Notice tone="fail">{error === CANCELLED ? "Cancelled." : error}</Notice> : null}
    </AuthLayout>
  );
}

function NoPasskeys() {
  return (
    <AuthLayout above="Ferrum" title="This browser cannot use passkeys">
      <p className="text-[13.5px] text-ink-3 leading-relaxed">
        Ferrum has no passwords, so there is nothing else to sign in with here. Open the panel in a
        current browser, or reach the server over SSH.
      </p>
    </AuthLayout>
  );
}
