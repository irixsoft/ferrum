import { Badge } from "@/components/ui/Badge";

/** Anything still reading src/lib/mock.ts must carry this, or the panel lies. */
export function SampleData({ className }: { className?: string }) {
  return (
    <Badge tone="hold" className={className}>
      Sample data
    </Badge>
  );
}
