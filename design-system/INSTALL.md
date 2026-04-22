# Temper design system — install & usage

This bundle is a **design-system reference**, not a shipping npm package. It's meant to live alongside the production code in the `temper` repo so designers, engineers, and Claude Code have a single source of truth for visual language, component patterns, and implementation handoffs.

---

## What's in here

| Path | What it is |
|---|---|
| `README.md` | The design system in prose — principles, tokens, type, color, components, knowledge-graph visual language. |
| `SKILL.md` | How Claude Code (or any agent) should use this system when making Temper UI. |
| `INSTALL.md` | This file. |
| `colors_and_type.css` | Every color + type token as CSS custom properties. Drop-in import. |
| `tailwind.config.preset.js` | Tailwind preset consumers can extend in their own `tailwind.config`. |
| `tailwind.theme.css` | CSS-var theme for non-Tailwind consumers. |
| `docs/kg-handoff.md` | Engineering handoff for the knowledge-graph visualization. PR sequencing, data contract, open questions. |
| `preview/` | Static HTML design cards — one per component, token, or pattern. Open any in a browser. |
| `ui_kits/landing/` | JSX reference for the public landing site (Hero, Nav, Section, CliBlock, etc.). |
| `ui_kits/app/` | JSX reference for the authed vault shell. Includes `graph.html` — the working KG prototype. |
| `assets/` | Brand mark, favicon, social preview, existing production diagrams. |

---

## Recommended repo location

Drop the whole folder into the `temper` monorepo as either:

- `design-system/` at the repo root — treats the bundle as a first-class sibling to `packages/`.
- `docs/design-system/` — treats it as documentation.

Either works. The handoff doc (`docs/kg-handoff.md`) assumes repo-root paths; adjust if you nest it deeper.

**Do not** drop it into `packages/` — this isn't a publishable code package, and putting it there implies a build target that doesn't exist.

---

## Running the preview cards

The preview cards are static HTML with inline or adjacent CSS. No build step.

```sh
cd design-system/preview
python3 -m http.server 8000
# open http://localhost:8000/
```

Or any static server (`npx serve`, `caddy file-server`, etc.).

The KG prototype (`ui_kits/app/graph.html`) uses in-browser Babel to transpile JSX on load — no build required. It does need a server (file:// doesn't work because of ES module loading rules), so serve it the same way.

---

## Using the tokens in production code

### Tailwind consumers

In `packages/temper-ui/tailwind.config.ts`:

```ts
import preset from '../../design-system/tailwind.config.preset.js';

export default {
  presets: [preset],
  content: ['./src/**/*.{svelte,ts}'],
  // your own extensions…
};
```

### Non-Tailwind consumers

Import the CSS-var theme once at the root of your app:

```css
@import '../../design-system/tailwind.theme.css';
```

Then reference tokens via `var(--temper-blue)`, `var(--temper-ground)`, etc.

### Raw tokens

If you just want the values (for Figma, Storybook, or another tool that doesn't eat CSS), `colors_and_type.css` is the canonical list. Every token is commented with its semantic role.

---

## The knowledge graph work specifically

`docs/kg-handoff.md` is the engineering bridge doc for the `/vault/<ctx>/graph` route. It assumes:

1. The existing D3-based graph is being replaced.
2. The renderer swap lands first, followed by the peek panel, then breadcrumb traversal, then hover states.
3. Production is Svelte 5 — the JSX in `ui_kits/app/` is a *reference*, not a port. Read the shapes and state names; write idiomatic Svelte.

The prototype at `ui_kits/app/graph.html` is runnable and clickable. Open it alongside the handoff doc when planning each PR — it answers "what does the behavior feel like?" faster than any prose can.

---

## For Claude Code

When invoking Claude Code against this bundle:

1. Point it at `docs/kg-handoff.md` as the spec for the KG work.
2. Point it at `SKILL.md` for general Temper UI patterns (voice, color, type, editorial chrome).
3. The `preview/` cards are reference-only — don't copy the HTML into production. They exist so you can see a pattern rendered, not to be lifted verbatim.
4. The `ui_kits/` JSX is likewise reference-only. Production is Svelte.

---

## What *isn't* in this bundle (and why)

- **`_source/`** — read-only imports of the production repo used for design research. Not shipped because the real code in the repo is the canonical source now.
- **`uploads/`** — user-provided context from the design session. Not shipped because it's project-specific.
- **`screenshots/`** — intermediate work-product from the design iterations. Not shipped because the preview cards are the canonical visual record.
- **A package build.** This is a reference, not a runtime dependency. If you want a publishable design-token package later, it's a small wrapper around `colors_and_type.css` — but that should be its own decision.
