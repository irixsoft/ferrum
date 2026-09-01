import { useEffect, useRef, useState } from "react";
import uPlot from "uplot";
import { useTheme } from "@/lib/theme";


export interface Band {
  key: string;
  label: string;
  varName: string;
  fill?: boolean;
}

export function MetricChart({
  t: time,
  bands,
  values,
  height = 180,
  unit = "%",
}: {
  t: number[];
  bands: Band[];
  values: Record<string, number[]>;
  height?: number;
  unit?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);
  const { resolved } = useTheme();
  const [width, setWidth] = useState(0);

  useEffect(() => {
    if (!host.current) return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.floor(e.contentRect.width)));
    ro.observe(host.current);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (!host.current || !width) return;
    const css = getComputedStyle(document.documentElement);
    const read = (n: string) => css.getPropertyValue(n).trim();
    // Canvas support for color-mix() in fillStyle is uneven; resolve it here.
    const tint = (hex: string, alpha: number) => {
      const m = /^#?([0-9a-f]{6})$/i.exec(hex);
      if (!m) return hex;
      const n = parseInt(m[1], 16);
      return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
    };

    const grid = read("--c-line");
    const label = read("--c-ink-4");

    const opts: uPlot.Options = {
      width,
      height,
      padding: [8, 8, 0, 0],
      cursor: { points: { size: 6 }, drag: { x: false, y: false } },
      legend: { show: false },
      scales: { x: { time: true } },
      axes: [
        {
          stroke: label,
          grid: { stroke: grid, width: 1 },
          ticks: { show: false },
          font: "11px 'IBM Plex Sans', sans-serif",
          size: 26,
        },
        {
          stroke: label,
          grid: { stroke: grid, width: 1 },
          ticks: { show: false },
          font: "11px 'IBM Plex Sans', sans-serif",
          size: 38,
          values: (_u, ticks) => ticks.map((v) => `${v}${unit}`),
        },
      ],
      series: [
        {},
        ...bands.map((b) => {
          const stroke = read(b.varName);
          return {
            label: b.label,
            stroke,
            width: 1.5,
            fill: b.fill ? tint(stroke, 0.14) : undefined,
            points: { show: false },
          } satisfies uPlot.Series;
        }),
      ],
    };

    const data = [time, ...bands.map((b) => values[b.key] ?? [])] as unknown as uPlot.AlignedData;
    plot.current?.destroy();
    plot.current = new uPlot(opts, data, host.current);
    return () => {
      plot.current?.destroy();
      plot.current = null;
    };
  }, [width, height, resolved, time, values, bands, unit]);

  return <div ref={host} className="w-full" style={{ height }} />;
}

export function ChartKey({ bands }: { bands: Band[] }) {
  return (
    <div className="flex items-center gap-4">
      {bands.map((b) => (
        <span key={b.key} className="inline-flex items-center gap-1.5 text-[12.5px] text-ink-3">
          <span
            className="h-0.5 w-4 rounded-full"
            style={{ background: `var(${b.varName})` }}
          />
          {b.label}
        </span>
      ))}
    </div>
  );
}
