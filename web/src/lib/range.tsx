import { createContext, useContext, useState, type ReactNode } from "react";
import type { MetricRange } from "@/types/api";

export type Range = MetricRange;

export const RANGES: Array<{ value: Range; label: string }> = [
  { value: "1h", label: "Last hour" },
  { value: "24h", label: "24 hours" },
  { value: "7d", label: "7 days" },
];

const Ctx = createContext<{ range: Range; setRange: (r: Range) => void }>({
  range: "24h",
  setRange: () => {},
});

export function RangeProvider({ children }: { children: ReactNode }) {
  const [range, setRange] = useState<Range>("24h");
  return <Ctx.Provider value={{ range, setRange }}>{children}</Ctx.Provider>;
}

export const useRange = () => useContext(Ctx);
