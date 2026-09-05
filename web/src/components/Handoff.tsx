import { Button } from "@/components/ui/Button";

/** A secret or link shown once, with Copy and a Done that takes it off the screen. */
export function Handoff({ label, value, onDone }: { label: string; value: string; onDone: () => void }) {
  return (
    <div className="mb-4 bg-inset border border-line rounded-inset p-3">
      <p className="text-[12.5px] text-ink-3 mb-2">{label}</p>
      <div className="flex items-center gap-2">
        <code className="flex-1 font-mono text-[12px] text-ink break-all">{value}</code>
        <Button size="sm" onClick={() => navigator.clipboard?.writeText(value)}>
          Copy
        </Button>
        <Button size="sm" variant="ghost" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}
