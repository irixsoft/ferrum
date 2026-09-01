import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { Code } from "@/components/ui/Code";
import { keys } from "@/lib/api";
import { CANCELLED, enroll, platformAuthenticator, supported } from "@/lib/webauthn";
import { AuthLayout, Notice } from "./AuthLayout";

type Stage = "ready" | "working" | "done";

export function EnrollPage({ token }: { token: string }) {
  const client = useQueryClient();
  const navigate = useNavigate();
  const [stage, setStage] = useState<Stage>("ready");
  const [error, setError] = useState<string | null>(null);
  const [usable, setUsable] = useState(true);
  const [platform, setPlatform] = useState(true);

  useEffect(() => {
    setUsable(supported());
    platformAuthenticator().then(setPlatform);
  }, []);

  const create = async () => {
    setStage("working");
    setError(null);
    try {
      await enroll(token);
      setStage("done");
      await client.invalidateQueries({ queryKey: keys.me });
      await navigate({ to: "/" });
    } catch (e) {
      setError(e instanceof Error ? e.message : "The passkey was not created.");
      setStage("ready");
    }
  };

  if (!usable) return <NoPasskeys />;

  if (stage === "done") {
    return (
      <AuthLayout above="Ferrum" title="You are signed in">
        <p className="text-[13.5px] text-ink-3 leading-relaxed">
          Your passkey is registered. Loading the panel…
        </p>
      </AuthLayout>
    );
  }

  return (
    <AuthLayout
      above="Ferrum"
      title="Create your passkey"
      footer={
        <span>
          This link works once and expires an hour after it was issued. For another, run{" "}
          <Code>ferrum passkey enroll</Code> over SSH.
        </span>
      }
    >
      <p className="text-[13.5px] text-ink-3 leading-relaxed mb-5">
        Your browser or device stores the passkey. Ferrum keeps only the public half, so there is no
        password to steal and nothing to reset.
      </p>

      <Button
        variant="primary"
        size="lg"
        className="w-full"
        onClick={create}
        disabled={stage === "working"}
      >
        <KeyRound size={16} />
        {stage === "working" ? "Waiting for your device…" : "Create a passkey"}
      </Button>

      {!platform ? (
        <p className="mt-3 text-[12.5px] text-ink-4 leading-relaxed">
          This device has no built-in authenticator. A security key works, as long as it can store a
          resident credential.
        </p>
      ) : null}

      {error ? <Notice tone="fail">{error === CANCELLED ? "Cancelled." : error}</Notice> : null}
    </AuthLayout>
  );
}

function NoPasskeys() {
  return (
    <AuthLayout above="Ferrum" title="This browser cannot create passkeys">
      <p className="text-[13.5px] text-ink-3 leading-relaxed">
        Ferrum has no passwords, so there is nothing else to enrol here. Open this link in a current
        browser, or reach the server over SSH.
      </p>
    </AuthLayout>
  );
}
