import { describe, expect, test } from "bun:test";
import { limitsLine, reconcileRoutes, routable } from "./processes";
import type { ProcessInput } from "@/types/api";

const web: ProcessInput = { name: "web", start: "bun run start", port: true };
const jobs: ProcessInput = { name: "jobs", start: "bun run jobs", port: false };
const site: ProcessInput = { name: "site", static_dir: "dist", port: false };

describe("routes", () => {
  test("only a listening process or a folder takes traffic", () => {
    expect(routable([web, jobs, site])).toEqual(["web", "site"]);
  });

  test("a route whose process stopped listening moves to the first that listens", () => {
    const routes = [
      { path: "/", process: "web", websocket: false },
      { path: "/admin", process: "jobs", websocket: false },
    ];
    expect(reconcileRoutes([site, web, jobs], routes)).toEqual([
      { path: "/", process: "web", websocket: false },
      { path: "/admin", process: "site", websocket: false },
    ]);
    expect(reconcileRoutes([jobs, site], [])).toEqual([{ path: "/", process: "site", websocket: false }]);
  });
});

describe("limitsLine", () => {
  test("a folder-only app is served by nginx", () => {
    expect(limitsLine([{ kind: "folder", memory_mb: 512 }], 100)).toBe("Served by nginx");
  });

  test("the memory limit sums what the units may use", () => {
    expect(
      limitsLine(
        [
          { kind: "command", memory_mb: 512 },
          { kind: "command", memory_mb: 256 },
        ],
        100,
      ),
    ).toBe("2 processes · up to 768 MB · 100% CPU");
  });
});
