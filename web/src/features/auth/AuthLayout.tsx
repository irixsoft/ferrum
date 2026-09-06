import type { ReactNode } from "react";
import { Wordmark } from "@/components/Brand";
import { cn } from "@/lib/utils";

export function AuthLayout({
  title,
  above,
  children,
  footer,
}: {
  title: string;
  above?: string;
  children: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <main className="min-h-dvh bg-shell flex items-center justify-center p-5">
      <div className="w-full max-w-[380px]">
        <Wordmark height={24} className="text-ink mx-auto mb-8" />

        <div className="bg-surface border border-line rounded-card overflow-hidden">
          <div className="p-6">
            {above ? <p className="text-[12.5px] text-ink-4 mb-1">{above}</p> : null}
            <h1 className="font-display text-[22px] text-ink mb-5">{title}</h1>
            {children}
          </div>
          {footer ? (
            <div className="px-6 py-3.5 bg-inset border-t border-line text-[12.5px] text-ink-3 leading-relaxed">
              {footer}
            </div>
          ) : null}
        </div>
      </div>
    </main>
  );
}

export function Notice({ tone, children }: { tone: "fail" | "ok"; children: ReactNode }) {
  return (
    <p
      className={cn(
        "mt-4 border rounded-inset px-3 py-2.5 text-[13px] leading-relaxed",
        tone === "fail"
          ? "bg-fail-soft border-fail/25 text-fail"
          : "bg-ok-soft border-ok/25 text-ok",
      )}
    >
      {children}
    </p>
  );
}
