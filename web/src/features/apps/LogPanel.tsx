import { useState } from "react";
import { Card, CardBody, CardHeader } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Segmented } from "@/components/ui/Segmented";
import { SampleData } from "@/components/SampleData";


type Source = "app" | "access" | "build";

const LINES: Record<Source, Array<[string, string, string]>> = {
  app: [
    ["12:04:41", "info", "Listening on 127.0.0.1:41204"],
    ["12:04:41", "info", "Applied 3 pending migrations"],
    ["12:04:52", "info", "GET /api/statements 200 in 31ms"],
    ["12:05:03", "warn", "Statement export took 2.4s, above the 1s budget"],
    ["12:05:14", "info", "GET /api/statements/4821 200 in 12ms"],
    ["12:05:19", "error", "Reconciliation job failed: upstream returned 503"],
    ["12:05:19", "info", "Retrying reconciliation in 30s"],
  ],
  access: [
    ["12:05:14", "info", '203.0.113.44 "GET /api/statements HTTP/2" 200 4211'],
    ["12:05:15", "info", '198.51.100.7 "POST /api/webhooks/stripe HTTP/2" 204 0'],
  ],
  build: [
    ["12:01:07", "info", "dotnet publish -c Release -o out"],
    ["12:02:41", "info", "Build succeeded in 94.2s"],
  ],
};

const TONE: Record<string, string> = {
  info: "text-ink-3",
  warn: "text-run",
  error: "text-fail",
};

export function LogPanel({ slug }: { slug: string }) {
  const [source, setSource] = useState<Source>("app");

  return (
    <Card>
      <CardHeader
        title={
          <span className="flex items-center gap-2">
            Logs
            <Badge tone="ok">Live</Badge>
            <SampleData />
          </span>
        }
        hint={`journalctl -u ferrum-app-${slug} --follow`}
        action={
          <Segmented
            value={source}
            onChange={setSource}
            options={[
              { value: "app", label: "App" },
              { value: "access", label: "Access" },
              { value: "build", label: "Build" },
            ]}
          />
        }
      />
      <CardBody>
        <div className="bg-inset border border-line rounded-inset p-3 font-mono text-[12px] leading-[1.7] overflow-x-auto">
          {LINES[source].map(([time, level, text], i) => (
            <div key={i} className="flex gap-3 whitespace-pre">
              <span className="text-ink-4 tnum shrink-0">{time}</span>
              <span className={TONE[level]}>{text}</span>
            </div>
          ))}
        </div>
      </CardBody>
    </Card>
  );
}
