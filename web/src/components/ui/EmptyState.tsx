import type { ReactNode } from "react";

export function EmptyState({
  title,
  body,
  action,
}: {
  title: string;
  body: string;
  action?: ReactNode;
}) {
  return (
    <div className="px-6 py-14 text-center max-w-sm mx-auto">
      <h3 className="font-display text-lg text-ink">{title}</h3>
      <p className="text-[13.5px] text-ink-3 mt-1.5 leading-relaxed">{body}</p>
      {action ? <div className="mt-5 flex justify-center">{action}</div> : null}
    </div>
  );
}
