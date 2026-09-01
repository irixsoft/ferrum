# Ferrum panel

The React panel for Ferrum. It is built with Bun and embedded into the Ferrum
binary, so it ships with no network dependency and no separate deployment.

**Read `DESIGN.md` before adding UI.** It is short, and it is what keeps new
screens from drifting.

```bash
bun install
bun run dev        # http://localhost:5173, proxying /api to a local daemon
bun run build      # → dist/, embedded by the Rust build
bun run typecheck
bun test
bun run icons      # regenerate PWA icons from public/icon.svg
```

Bun only. No `npm`, no `npx`, no `node`.

## Stack

React 19 · Vite 7 · TypeScript · Tailwind v4 · TanStack Router · TanStack Query ·
uPlot · vite-plugin-pwa.

UI primitives are hand-rolled in `src/components/ui/` rather than pulled from
shadcn — same `cn` / `cva` conventions, so a shadcn component still drops in
cleanly when one is genuinely wanted.

Charts are uPlot: canvas, repaints only when data changes.

## Layout

```
src/
  main.tsx                  providers: theme → query → router
  router.tsx                ONE route tree, both shells. 3 lines per screen.
  styles/index.css          all design tokens live here
  types/api.ts              API shapes — replace with ts-rs output
  lib/
    api.ts                  the ONLY seam between UI and backend
    mock.ts                 fixtures for screens the backend does not serve yet
    webauthn.ts             the two passkey ceremonies, base64url ↔ ArrayBuffer
    theme.tsx               light / dark / system
    utils.ts                cn, bytes, duration, ago, pct
  shells/
    Shell.tsx               picks a chrome, mounts the two global banners
    DesktopShell.tsx        rail + persistent host strip
    MobileShell.tsx         bottom tabs, safe areas, app-like header
    useShell.ts             matchMedia + "always desktop" override
  components/
    DeployLadder.tsx        ← the signature element
    MetricChart.tsx         uPlot wrapper, theme-aware
    AppCard.tsx  RuntimeMark.tsx  StatusPill.tsx  PageTitle.tsx
    UpdatePrompt.tsx        version-skew guard — required
    ConnectionBanner.tsx    "cannot reach the server" — required
    Brand.tsx               mark + wordmark, inlined, takes currentColor
    nav.ts                  the IA, stated once, read by both shells
    ui/                     Button Card Badge Meter Code Row Tabs
                            Segmented Sheet EmptyState
  features/
    auth/  dashboard/  apps/  databases/  system/  settings/
```

## Wiring the backend

One file. In `src/lib/api.ts`, replace a `queryFn` body with `request(...)`:

```ts
export function useApps() {
  return useQuery({ queryKey: keys.apps, queryFn: () => request<App[]>("/apps") });
}
```

Every screen still reading `src/lib/mock.ts` says so on its face, with a "Sample
data" badge. When the last reader is gone, delete the file.

Point `src/types/api.ts` at `ts-rs` output once the generator is wired up, so a
Rust change that breaks the panel fails at typecheck rather than at runtime.

Deploy logs and app logs arrive over SSE. `LogPanel` and `DeployLadder` both take
their data as props, so an `EventSource` hook slots in without touching either.

## Authentication

Passkeys only, and the browser half lives in `src/lib/webauthn.ts`. The login
page is a single button — `navigator.credentials.get()` with an empty
`allowCredentials` and no `mediation`, so the browser's own picker supplies the
identity. Do not add a username field: conditional-UI autofill needs one, and
that is the flow this design deliberately does not use.

`/enroll/:token` runs the registration ceremony and is the only route reachable
without a session.

## PWA

`vite-plugin-pwa` in `prompt` mode. Icons are generated from `public/icon.svg` by
`scripts/gen-icons.mjs` — maskable variants sit inside the 80% safe zone on an
opaque field, plus an `apple-touch-icon` because iOS still needs its own.

The service worker precaches the shell and static assets **only**. `/api/*` and
`/mcp` are `NetworkOnly`: a service worker cache is written to disk and outlives
the session, and env vars, database passwords and connection strings must never
land there.

`sw.js` is served with `Cache-Control: no-cache`. A cached service worker script
is a service worker that never updates.

CI sets `FERRUM_BUILD_ID` at build time so `UpdatePrompt` can compare the running
bundle against `/api/version`.

## Not built yet

App creation, the rollback dialog with its two clearly-labelled choices, the
command palette, routes and system-packages editors, and most mutations.

The rollback dialog in particular must never pick for the user: *roll back code
only* and *roll back code and restore the pre-migration snapshot* are different
decisions with different consequences, and the panel must say so with the actual
timestamp.

## Licence

AGPL-3.0-only, matching Ferrum itself.
