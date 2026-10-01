import { useEffect, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Rendered into `document.body`: a `backdrop-filter` or `transform` on any ancestor would make
 * that ancestor the box `fixed` measures against, and the sheet would cover only the header.
 */
export function Sheet({
  open,
  onClose,
  title,
  children,
  footer,
  side = "bottom",
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  side?: "bottom" | "center";
}) {
  const centred = side === "center";

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  return createPortal(
    <div className={cn("fixed inset-0 z-50", !open && "pointer-events-none invisible")} aria-hidden={!open}>
      <div
        onClick={onClose}
        className={cn(
          "absolute inset-0 bg-ink/40 transition-opacity duration-150",
          open ? "opacity-100" : "opacity-0",
        )}
      />
      <div
        role="dialog"
        aria-modal="true"
        className={cn(
          "absolute flex flex-col bg-surface border-line shadow-lift ease-out",
          centred
            ? [
                "left-1/2 top-1/2 w-[min(30rem,calc(100vw-3rem))] max-h-[80dvh]",
                "-translate-x-1/2 border rounded-card transition-[transform,opacity] duration-150",
                open ? "-translate-y-1/2 opacity-100" : "-translate-y-[46%] opacity-0",
              ]
            : [
                "inset-x-0 bottom-0 max-h-[88dvh] border-t rounded-t-[20px] pb-safe",
                "transition-transform duration-200",
                open ? "translate-y-0" : "translate-y-full",
              ],
        )}
      >
        <div className="flex items-center justify-between px-5 pt-4 pb-3 border-b border-line">
          <h2 className="text-[15px] font-semibold">{title}</h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="h-8 w-8 grid place-items-center rounded-full text-ink-3 hover:bg-inset hover:text-ink"
          >
            <X size={16} />
          </button>
        </div>
        <div className="overflow-y-auto overscroll-contain px-5 py-4">{open ? children : null}</div>
        {footer ? <div className="px-5 py-3 border-t border-line">{footer}</div> : null}
      </div>
    </div>,
    document.body,
  );
}
