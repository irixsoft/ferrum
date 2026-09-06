import { useEffect, useRef, useState } from "react";
import { ApiError, followAppLog, useAppLogs } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Card, CardBody, CardFoot, CardHeader } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Segmented } from "@/components/ui/Segmented";
import type { AppLogLine, LogLevel, LogSource } from "@/types/api";

const LINES = 200;
const KEEP = 2000;

const TONE: Record<LogLevel, string> = {
  info: "text-ink-2",
  warn: "text-run",
  error: "text-fail",
};

const clock = (iso: string) => (iso ? iso.slice(11, 19) : "");

/** The app log follows journald over SSE; nginx logs are read as a tail on request. */
export function LogPanel({ slug, hasProcess }: { slug: string; hasProcess: boolean }) {
  const [source, setSource] = useState<LogSource>(hasProcess ? "app" : "access");
  const [following, setFollowing] = useState(hasProcess);
  const live = source === "app" && following;
  const tail = useAppLogs(slug, source, LINES, !live);
  const [lines, setLines] = useState<AppLogLine[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [pinned, setPinned] = useState(true);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!live) return;
    const controller = new AbortController();
    setLines([]);
    setError(null);
    followAppLog(
      slug,
      LINES,
      (line) => setLines((prev) => (prev.length >= KEEP ? [...prev.slice(-KEEP + 1), line] : [...prev, line])),
      controller.signal,
    ).catch((e: unknown) => {
      if (controller.signal.aborted) return;
      setError(e instanceof ApiError ? e.message : String(e));
    });
    return () => controller.abort();
  }, [slug, live]);

  useEffect(() => {
    if (!live && tail.data) setLines(tail.data);
  }, [live, tail.data]);

  useEffect(() => {
    if (pinned && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines, pinned]);

  const options: Array<{ value: LogSource; label: string }> = [
    ...(hasProcess ? [{ value: "app" as const, label: "App" }] : []),
    { value: "access", label: "Access" },
    { value: "error", label: "Errors" },
  ];

  return (
    <Card>
      <CardHeader
        title={
          <span className="flex items-center gap-2">
            Logs
            {live ? <Badge tone="ok">Live</Badge> : <Badge>Last {LINES}</Badge>}
          </span>
        }
        hint={
          source === "app"
            ? `journalctl -u ferrum-app-${slug} --follow`
            : `/var/log/nginx/ferrum-${slug}.${source}.log`
        }
        action={
          <>
            {source === "app" ? (
              <Button size="sm" variant="ghost" onClick={() => setFollowing((f) => !f)}>
                {following ? "Pause" : "Follow"}
              </Button>
            ) : (
              <Button size="sm" variant="ghost" disabled={tail.isFetching} onClick={() => tail.refetch()}>
                Reload
              </Button>
            )}
            <Segmented value={source} onChange={setSource} options={options} />
          </>
        }
      />
      <CardBody>
        <div
          ref={box}
          onScroll={(e) => {
            const el = e.currentTarget;
            setPinned(el.scrollHeight - el.scrollTop - el.clientHeight < 24);
          }}
          className="max-h-[60vh] overflow-y-auto bg-inset border border-line rounded-inset p-3 font-mono text-[12px] leading-[1.7]"
        >
          {lines.length === 0 && !error ? (
            <p className="text-ink-4">
              {tail.isLoading || live ? "Waiting for the first line…" : "Nothing logged yet."}
            </p>
          ) : null}
          {lines.map((l, i) => (
            <div key={i} className="flex gap-3">
              <span className="text-ink-4 tnum shrink-0">{clock(l.at)}</span>
              <span className={cn("whitespace-pre-wrap break-all", TONE[l.level])}>{l.text}</span>
            </div>
          ))}
          {error ? <div className="mt-2 text-fail">{error}</div> : null}
          {tail.error ? (
            <div className="mt-2 text-fail">{tail.error instanceof ApiError ? tail.error.message : String(tail.error)}</div>
          ) : null}
        </div>
      </CardBody>
      {!pinned ? (
        <CardFoot>
          <span>Scrolled up; new lines keep arriving below.</span>
          <button onClick={() => setPinned(true)} className="text-ink-3 hover:text-ink">
            Follow the log
          </button>
        </CardFoot>
      ) : null}
    </Card>
  );
}
