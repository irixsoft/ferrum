import { useState } from "react";
import { useExtensions } from "@/lib/api";
import { Code } from "@/components/ui/Code";

const INPUT =
  "h-8 px-2.5 bg-inset border border-line-strong rounded-control text-sm text-ink placeholder:text-ink-4 font-mono text-[12.5px]";

/** Every extension the server offers, filtered as you type; `picked` are shown first as chips. */
export function ExtensionPicker({
  picked,
  onPick,
  disabled = false,
  hidden = [],
}: {
  picked: string[];
  onPick: (name: string) => void;
  disabled?: boolean;
  hidden?: string[];
}) {
  const { data: offered = [], isError } = useExtensions();
  const [filter, setFilter] = useState("");
  const needle = filter.trim().toLowerCase();
  const matches = offered.filter(
    (e) => !picked.includes(e) && !hidden.includes(e) && (!needle || e.toLowerCase().includes(needle)),
  );
  const shown = needle ? matches : matches.slice(0, 12);

  return (
    <div className="grid gap-2">
      {picked.length ? (
        <span className="flex flex-wrap gap-1">
          {picked.map((e) => (
            <button
              key={e}
              type="button"
              disabled={disabled}
              onClick={() => onPick(e)}
              className="font-mono text-[12px] bg-ink text-canvas rounded px-1.5 py-0.5"
              title={`Remove ${e}`}
            >
              {e}
            </button>
          ))}
        </span>
      ) : null}
      <input
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        placeholder={offered.length ? `Search ${offered.length} extensions` : "Loading…"}
        disabled={disabled}
        className={INPUT}
      />
      {isError ? <span className="text-[12px] text-fail">Could not list what this server offers.</span> : null}
      <span className="flex flex-wrap gap-1">
        {shown.map((e) => (
          <button
            key={e}
            type="button"
            disabled={disabled}
            onClick={() => {
              onPick(e);
              setFilter("");
            }}
            className="font-mono text-[12px] text-ink-4 border border-dashed border-line-strong rounded px-1.5 py-0.5 hover:text-ink"
          >
            + {e}
          </button>
        ))}
        {!needle && matches.length > shown.length ? (
          <span className="text-[12px] text-ink-4 self-center">
            and {matches.length - shown.length} more, type to search
          </span>
        ) : null}
        {needle && matches.length === 0 ? (
          <span className="text-[12px] text-ink-4">
            Nothing called <Code>{filter.trim()}</Code> on this server.
          </span>
        ) : null}
      </span>
    </div>
  );
}
