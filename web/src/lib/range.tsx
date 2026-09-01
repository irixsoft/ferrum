import { createContext, useContext, useState, type ReactNode } from "react";
import type { MetricSeries } from "@/types/api";


export type Range = "1h" | "24h" | "7d";

export const RANGES: Array<{ value: Range; label: string }> = [
  { value: "1h", label: "Last hour" },
  { value: "24h", label: "24 hours" },
  { value: "7d", label: "7 days" },
];

const SECONDS: Record<Range, number> = { "1h": 3600, "24h": 86_400, "7d": 604_800 };

const Ctx = createContext<{ range: Range; setRange: (r: Range) => void }>({
  range: "24h",
  setRange: () => {},
});

export function RangeProvider({ children }: { children: ReactNode }) {
  const [range, setRange] = useState<Range>("24h");
  return <Ctx.Provider value={{ range, setRange }}>{children}</Ctx.Provider>;
}

export const useRange = () => useContext(Ctx);

export function sliceRange(series: MetricSeries, range: Range): MetricSeries {
  const cutoff = (series.t.at(-1) ?? 0) - SECONDS[range];
  const from = series.t.findIndex((t) => t >= cutoff);
  const start = from < 0 ? 0 : from;
  return {
    t: series.t.slice(start),
    values: Object.fromEntries(
      Object.entries(series.values).map(([k, v]) => [k, v.slice(start)]),
    ),
  };
}
