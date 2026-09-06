# Ferrum panel — design language

This file exists so the next person (or model) to touch this codebase can add a
screen that looks like it belongs, without having to reverse-engineer the taste
from the components. Read it before adding UI.

---

## What this product is

An instrument panel for **one box you own**. The audience runs its own servers:
*nothing is hidden and nothing is decided silently*. That single idea drives most
of what follows.

The panel is not a marketing surface and not a fleet dashboard. It is closer to
the readout on a machine than to a SaaS console.

## The three rules that shape everything

**1. Colour encodes, never decorates.**

There are exactly two things colour is allowed to mean:

- **What a thing is** — runtime identity. Node is `--c-node`, Bun `--c-bun`,
  .NET `--c-dotnet`, static `--c-static`. Nothing else may borrow these four
  hues, or they stop meaning anything.
- **What state a thing is in** — `--c-ok` live, `--c-run` working, `--c-fail`
  failed, `--c-hold` paused. These carry status and nothing else.

`--c-accent` is for selection, focus and links. Everything else is the neutral
ramp. If you are reaching for a colour and it is not answering "what is this" or
"what state is it in", use grey.

The neutral ramp is built around **`#61666a`, the grey of the Ferrum mark**. It
is muted body text in light mode (`--c-ink-3`) and the faintest legible text in
dark (`--c-ink-4`). The brand colour is load-bearing rather than decorative,
which is why the panel does not need an invented brand hue on top.

**2. No continuously animating CSS.**

A real constraint, not a preference: a looping animation pegs the GPU on a high-refresh display, and this panel is left
open all day. There are **no keyframes anywhere in `index.css`** — no spinners,
no pulse, no shimmer, no skeleton sweep.

Transitions answer a user action (opening a sheet, hovering a row) and are
short. A number that changes because the data changed is fine — that is data,
not decoration.

Progress is shown as **discrete state**. See rule 3.

**3. The deploy ladder is the signature element.**

`src/components/DeployLadder.tsx`. A deploy is an explicit state machine in the
backend, so it is drawn as one: twelve states, each banking its real
elapsed time, skipped steps saying *why* they were skipped. `DeployRail` is the
compressed sideways reading for table rows and the top bar.

Spend the boldness here and keep everything around it quiet. **Do not put a
spinner next to it**, and do not add a second progress metaphor elsewhere.

---

## Type

| Role | Face | Where |
|---|---|---|
| Display | Archivo Variable, `.font-display`, tracking `-0.022em` | page titles, app names, section headings |
| UI | IBM Plex Sans, 14px base | everything else |
| Mono | IBM Plex Mono | ports, commit SHAs, paths, commands, env keys, IPs, log bodies |

Mono is a **functional** role, not a third personality: use it when the string is
something the user would type or paste, and not for flavour. Small data labels
in mono are a generated-page tell; small data labels here are Plex Sans.

Any number that changes in place gets `.tnum` so its column does not jitter.

Self-hosted via fontsource, because the bundle ends up inside a binary on a box
that may have no outbound network.

## Frame and surfaces

Three levels, and they are the reason the panel reads as one object rather than
a page of boxes:

| Token | Role |
|---|---|
| `--c-shell` | the frame the app floats on — visible only as a 12px margin |
| `--c-canvas` | the panel itself, `rounded-shell` |
| `--c-surface` | cards inside the panel |

Cards are bounded by a **hairline, not a drop shadow**. A uniform soft shadow
under every panel flattens hierarchy and is the commonest SaaS-kit tell. The one
shadow token, `--shadow-lift`, is for things that genuinely float: sheets,
dialogs, the update prompt.

Radii are differentiated by role rather than uniform: `--radius-card` 14px,
`--radius-control` 9px, `--radius-inset` 10px, pills fully round.

Filled ink means *chosen*, everywhere, with no exceptions: the primary button,
the selected segment, the active tab underline, the active top-bar pill, and the
active rail button. That single rule is why the panel needs no brand colour for
emphasis — and why adding a second "selected" treatment would cost more than it
looks like it would.

**Navigation is a rail of circular buttons**, not a labelled sidebar. It carries
no text, so `RailButton` gives every item a hover/focus tooltip and an
`aria-label`. An icon-only nav without those is a guessing game — if you add a
rail item, do not skip them.

**Top-bar pills are spaced apart, not joined** into a segmented track. Each one
is its own control. `Segmented` is the joined variant and belongs inside cards,
where it switches a view rather than setting global state.

Search sits on the **title row**, not in the top bar, because that is where the
eye already is when a page loads. Pages pass `<SearchPill />` through
`PageTitle`'s `action` slot.

## Writing

- Sentence case. No ALL-CAPS eyebrow labels, no `WORD — fragment` constructions,
  no `→` glued to link text.
- Name things as the user understands them. The tab is **Deploys**, not
  *Deployment pipeline*. The status is **Deploying**, not *Building*.
- A button says exactly what happens, and the same word survives the whole flow.
- Empty states are an invitation to act, and they say what would be here.
- Failures state what happened and what to do. They do not apologise and they
  are never vague. `"Build exceeded 512 MB and was stopped. Raise the build
  limit or reduce peak memory."` — not `"Deploy failed."`
- Card footers are the right home for the *why*: the ownership boundary, the
  isolation guarantee, the reason traffic pauses. This audience wants it.

---

## Shells

`DesktopShell` and `MobileShell` own **chrome only** — navigation, layout,
density, how a screen is presented. They are the only files that know which
shell is rendering. Nothing under `src/features/` may import from
`src/shells/` except `useShell`.

`useShell()` is for the minority of places where the *content itself* must
differ. It is used in exactly three places today, all justified:

- apps list — dense table vs cards
- deploy history — same
- `EnvironmentPanel` — editable table vs card-per-row with an edit sheet

A feature reaching for it more than incidentally means the boundary has slipped.
That is the signal to stop and reconsider, not to add a fourth.

**`EnvironmentPanel` is the reference pattern** for the three screens that are
genuinely costly on mobile (routes, env vars, system packages). Copy its
shape rather than inventing a new one: one state, one validation, two
presentations. `Sheet` takes `side="bottom" | "center"` so mobile and desktop
share one editor instead of maintaining two.

Both shells reach **full feature parity**. Nothing is unavailable because of the
device it was opened on.

## Two components that look optional and are not

`UpdatePrompt` — the build ID is compiled into the bundle and compared against
`/api/version`. Without it, a browser holding the old service worker runs old
JavaScript against a new API after a self-update, and the symptom is assorted
broken screens with no obvious cause.

`ConnectionBanner` — there is no offline data mode on purpose. A server manager
showing stale state is more dangerous than one that says it cannot reach the
server.

---

## Adding a screen

1. Page component under `src/features/<section>/`. It must not know which shell
   it is in.
2. Route in `src/router.tsx` (three lines).
3. Nav entry in `src/components/nav.ts` if it is top-level — both shells pick it
   up from that one list.
4. Read data through a hook in `src/lib/api.ts`. Nothing fetches directly.

Before you commit, check: does every colour on the screen answer "what is this"
or "what state is it in"? Is there anything moving that no one asked to move?

## Known gaps, deliberately left

- `src/lib/mock.ts` stands in for the API. Replace the `queryFn` bodies in
  `src/lib/api.ts` with `request(...)` and delete it.
- `src/types/api.ts` should be **generated from the Rust structs with `ts-rs`**.
  It is hand-written for now so the shapes are visible.
- No mutations, no forms that submit, no auth. The passkey login screen, the app
  creation flow (one review screen on desktop, stepped on mobile), rollback
  dialog, and the command palette are all unbuilt.
- The `dark:` custom variant is declared in `index.css` but unused — theming
  goes through CSS variables, so components never branch on theme. Keep it that
  way; it is why `MetricChart` is the only file that has to know.
