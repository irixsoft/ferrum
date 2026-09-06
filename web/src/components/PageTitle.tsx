import type { ReactNode } from "react";

export function PageTitle({
  above,
  title,
  action,
}: {
  above?: ReactNode;
  title: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col sm:flex-row sm:items-start sm:justify-between gap-4 sm:gap-6 mb-7">
      <div className="min-w-0">
        {above ? <p className="text-[14px] sm:text-[15px] text-ink-3 mb-1.5">{above}</p> : null}
        <h1 className="font-display text-[30px] sm:text-[38px] lg:text-[44px] leading-[1.05] sm:leading-[1] text-ink break-words">
          {title}
        </h1>
      </div>
      {action ? (
        <div className="shrink-0 flex items-center gap-2.5 w-full sm:w-auto">{action}</div>
      ) : null}
    </div>
  );
}
