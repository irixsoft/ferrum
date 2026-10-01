import { Button } from "@/components/ui/Button";
import { cn } from "@/lib/utils";

export function sharedDir(slug: string): string {
  return `/var/lib/ferrum/apps/${slug.trim() || "<slug>"}/shared`;
}

/** Shown wherever a value is typed, since nobody can guess where an app lives on the server. */
export function SharedDirHint({ slug, className }: { slug: string; className?: string }) {
  const dir = sharedDir(slug);
  return (
    <div className={cn("bg-inset border border-line rounded-inset px-3 py-2 text-[12.5px] text-ink-3", className)}>
      <p>The app's writable directory on the server, kept across every deploy:</p>
      <div className="flex items-center gap-2 mt-1">
        <code className="flex-1 min-w-0 font-mono text-[12.5px] text-ink break-all">{dir}</code>
        <Button size="sm" onClick={() => navigator.clipboard?.writeText(dir)}>
          Copy
        </Button>
      </div>
      <p className="mt-1">
        Point upload and storage variables at a folder inside it, such as{" "}
        <span className="font-mono text-ink-2 break-all">{dir}/uploads</span>. <span className="font-mono">storage</span>{" "}
        and <span className="font-mono">cache</span> are there already; the app creates any other folder itself.
      </p>
    </div>
  );
}
