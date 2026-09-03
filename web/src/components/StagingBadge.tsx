import { Badge } from "./ui/Badge";

const HINT =
  "Browsers do not trust these; run ferrum setup without --staging once the DNS is right.";

export function StagingBadge() {
  return (
    <Badge tone="hold" title={HINT}>
      Staging
    </Badge>
  );
}
