# @tasker-systems/temper-design

temper's theme contract, its themes, and the pure modules every temper surface renders
through — the one source every temper surface paints from.
The web UI (`packages/temper-ui`) and the desktop (`tasker-systems/temper-contrib`) both
consume it, so the same role, the same term and the same state read alike on each.

A theme is **data**: a `theme.json` that gives a value to every role in the contract.
Components consume roles, never values, so any theme that satisfies the contract restyles
everything a surface paints — its chrome, shadcn-svelte primitives, and anything rendered
from a json-render spec — with no per-theme code.

A closed JSON Schema is the source of truth, the files are validated by a self-check, and a
hand-broken fixture must be refused for the check to count.

```
packages/temper-design/
├── themes.json              # the theme index — name, title, appearance, counterpart (generated)
├── contract/
│   ├── theme.schema.json    # the contract — every role, closed (draft 2020-12)
│   ├── tailwind.css         # roles → Tailwind v4 utilities (generated from the schema)
│   ├── tailwind-roles.css   # the same utilities without the palette reset (generated)
│   ├── shadcn-bridge.css    # shadcn-svelte's variable names answered from roles
│   ├── base.css             # base styles + t-*/ed-* recipes, roles only
│   ├── region.css           # the four region states' recipe, roles only
│   ├── graph.css            # the graph's edge recipe: colour per stroke role, roles only
│   └── markdown.css         # the rendered document's recipe (`.md-body`), roles only
├── quiet-instrument/        # dark — temper's house theme
│   ├── theme.json           # source of truth
│   └── theme.css            # generated — never edit
├── quiet-instrument-paper/  # light counterpart
├── brand/                   # the threaded-t mark and wordmark, in currentColor
├── preview/index.html       # every role and the foundational components, both themes
├── scripts/themes.sh        # build · check · contrast · --self-check
├── src/                     # the shared modules: markdown, sanitize, links, highlight, region, omission, theme, vocabulary, graph
├── dist/                    # their compiled output (committed; see "The shared modules")
├── toolchain/               # the toolchain majors every surface builds on, and their check
└── tests/fixtures/broken/   # themes the check must refuse
```

## Installing

Published to npm in lockstep with temper's releases. Inside this repository, consume it as a
`file:` dependency, as `packages/temper-ui` does with `clients/temper-telemetry-ts`; outside
it, depend on the published version:

```json
{ "dependencies": { "@tasker-systems/temper-design": "<temper version>" } }
```

bun **copies** a `file:` dependency into the consumer's install; it does not link it. An edit
here reaches a consumer only after that consumer runs `bun install` again — until then it paints
from the copy it took, and stale styling will look like a bug in your change. Reinstall after
every edit you want to see in a consuming surface.

What a consumer imports:

| Path | What it is |
|---|---|
| `@tasker-systems/temper-design/contract/tailwind.css` | the roles as `tp-` Tailwind v4 utilities; resets the default palette |
| `@tasker-systems/temper-design/contract/tailwind-roles.css` | the same utilities without the reset — for a surface still moving off the default palette one area at a time; it switches to `tailwind.css` once nothing reads the palette |
| `@tasker-systems/temper-design/contract/shadcn-bridge.css` | optional: shadcn-svelte's variable names answered from roles |
| `@tasker-systems/temper-design/contract/base.css` | base styles and the `t-*`/`ed-*` recipes |
| `@tasker-systems/temper-design/<theme>/theme.css` | one theme's values, under `[data-theme="<theme>"]` |
| `@tasker-systems/temper-design/<theme>/theme.json` | one theme as data, for applying it without a build |
| `@tasker-systems/temper-design/themes.json` | every theme's name, title, appearance and counterpart — list and pair themes from this, never by globbing the package's directories |
| `@tasker-systems/temper-design/contract/theme.schema.json` | the contract itself |
| `@tasker-systems/temper-design/brand/*.svg` | the mark and wordmark, in `currentColor` |
| `@tasker-systems/temper-design/toolchain/majors.json` | the toolchain majors every surface builds on |
| `@tasker-systems/temper-design/toolchain/check.mjs` | the check a surface runs against them (see below) |
| `@tasker-systems/temper-design/guard/colours.mjs` | the colour guard a surface runs over the components it has moved onto roles (see below) |
| `@tasker-systems/temper-design/contract/region.css` | the `region-state` recipe: the four region states' colour and border |
| `@tasker-systems/temper-design/contract/graph.css` | the `graph-edge` recipe: a graph line, its heads and its cross, coloured by the stroke role `./graph` decides |
| `@tasker-systems/temper-design/contract/markdown.css` | the `md-body` recipe: how a rendered document reads — headings, links, lists, code, tables, refusals |
| `@tasker-systems/temper-design/markdown` | markdown → HTML with highlighted fences, bounded; DOM-free |
| `@tasker-systems/temper-design/sanitize` | the one sanitizer pass for that HTML; needs a DOM. Given a body's link answers, it rewrites every `./<uuid>` link: the held title when temper resolved it, inert when temper answered there is nothing you can see, and marked but still followable when nothing was verified |
| `@tasker-systems/temper-design/links` | body links: the exact `./<uuid>` matcher, `extractVaultLinks` (bounded, DOM-free, total — run before render to resolve the ids), the answer types, the cap and the hover words |
| `@tasker-systems/temper-design/highlight` | the one highlight.js registration, for direct code viewers |
| `@tasker-systems/temper-design/region` | the four region states' words, markers and ARIA roles |
| `@tasker-systems/temper-design/omission` | the omission sentence a bounded view ends with — counted (`omissionSentence`), uncounted (`uncountedSentence`: a read that reports only whether more exist, a cursor page, or nothing), drawn (`drawnSentence`: a read ranked from a bounded draw, which says what it drew from and never "all"), or a graph read (`graphSentence`) |
| `@tasker-systems/temper-design/theme` | the theme model: a reader's preference (follow the system within a family, or one theme) resolved against `themes.json` |
| `@tasker-systems/temper-design/vocabulary` | bound vocabularies: core's doc type → slot binding, `tintFor` (a doc type's slot, or the neutral), and `bindingsFrom`, which refuses a binding to anything but a slot |
| `@tasker-systems/temper-design/graph` | the graph's geometry and edge grammar: `settle` (deterministic; unconnected nodes listed, not placed; one line per pair; radial only when the input holds both core and ring), `placeCaptions`, `edgeGeometry` (lines end at the marks' rims, heads as shapes), and `stroke`/`pairStroke`/`pairEnds` (role, dash, width over weights in (0, 1], heads, the contradicts cross). A surface supplies its plane and its caption room |

The theme checker (`scripts/`, `tests/`, `pyproject.toml`) is not published: it runs in this
repository's CI, and the package carries only what it proves. The toolchain check is
published, because each surface runs it itself; its harness is not.

## The toolchain

Every surface builds on the same majors of one named set: `typescript`, `svelte`,
`@sveltejs/kit`, `@sveltejs/vite-plugin-svelte`, `svelte-check`, `vite`, `vitest`,
`tailwindcss`, `@tailwindcss/vite` and `@biomejs/biome`, plus each surface's own SvelteKit
adapter. A major upgrade of any of them reaches every surface together. `toolchain/majors.json`
declares the majors, and each surface checks what it resolves against them:

```bash
node <package>/toolchain/check.mjs --adapter @sveltejs/adapter-static
```

The check fails when the surface resolves a different major of a named tool, does not
resolve one, or names an adapter the declaration lacks. It looks a tool up the way Node
resolves a bare import, from the surface's directory upward; it reads no lockfile.

- **The web** (`packages/temper-ui`, `bun run toolchain`) reads the declaration from this
  working tree. A web major cannot move unless `majors.json` moves in the same change.
- **The desktop** reads it at the version it pins. The pin bump that carries a new major
  fails until the desktop has taken it too.

So between a release that moves a major and the desktop's next pin bump, the two surfaces
can differ and nothing fails. That gap is where the check stops, not something it covers.

Both repositories' `dependabot.yml` hold semver-majors of this set, so a major arrives as a
deliberate change rather than an automatic one. `node --test toolchain/check.test.mjs` is
the check's harness.

## The shared modules

`src/` holds what both surfaces render through, so a rendered body, a region's state and a
bounded view's omission read the same on each: one sanitizer policy (what one surface strips
or refuses, the other does not render), one set of region words, one omission sentence, one graph geometry. They
compile with `tsc` into `dist/`, which is committed — bun copies a `file:` dependency rather
than building it — and CI rebuilds it and fails when it is stale.

A shared module never binds a surface's runtime: no Tauri, no SvelteKit, no `svelte`, no
`$lib`/`$app`/`$env`, no Node. The DOM is common ground (the sanitizer needs one; `markdown`
deliberately does not, so a surface can prepare a body wherever it renders).
`node scripts/no-runtime.mjs` refuses such an import, and `node --test
scripts/no-runtime.test.mjs` proves each one is refused. Why a given surface's CSP backs the
sanitizer up is that surface's comment to keep, next to its CSP — not this package's.

## The contract

`contract/theme.schema.json` names every role a component may read. A theme must supply
all of them and may not invent more (`additionalProperties: false`): a component that
needs a colour the contract lacks is a change to the contract, reviewed as one, not a
theme quietly growing a private name.

| Group | Roles | Used for |
|---|---|---|
| Ground | `ground` `surface` `surface-raised` `overlay` | window, panels, popovers, modal scrim |
| Text | `text` `text-muted` `text-subtle` `text-faint` | reading, secondary, labels, disabled-only |
| Rule | `rule` `rule-strong` | hairlines, input outlines |
| Accent | `accent` `accent-muted` `accent-line` `accent-line-soft` `accent-wash` `accent-hover` `on-accent` `focus` | the single accent and its tints; the rail; keyboard focus |
| Conditions | `notice` `success` `danger` `pending` (+ `-wash` each) | attention, confirmation, destructive, awaiting-you |
| Region states | `region-arriving` `region-empty` `region-gave-up` `region-failed` (+ `-wash` each) | the four states a read can present as |
| Authorship | `author-human` `author-agent` | gutter and attribution marks |
| Categorical | `cat-1` … `cat-8` `cat-neutral` | bound vocabularies — core's doc types and plugins' (see below) |
| Code | `code-key` `code-string` `code-number` `code-comment` | syntax highlighting |
| Font | `reading` `doing` `ui` | serif prose, mono technical, dense UI |
| Radius, motion, tracking | `chip` `panel` · `quick` `settle` `easing` · `label` `strip` `eyebrow` `ui` `mark` | shape and rhythm |

Every role becomes a CSS custom property `--tp-<role>` (`--tp-text-muted`,
`--tp-font-reading`, `--tp-motion-quick`) and a Tailwind utility with a `tp-` prefix
(`bg-tp-ground`, `text-tp-text-muted`, `border-tp-rule`, `font-tp-doing`). The prefix keeps
temper's roles apart from shadcn's names, where `accent` and `muted` mean other things.

### What the check enforces

`scripts/themes.sh --self-check` runs in CI. For every theme it checks:

- **The contract.** Every role present, none invented, every value `#rrggbb` or `rgba()`.
- **Legibility floors.** Contrast is measured on the colours as they paint — alpha
  composited over `ground`, washes composited before the text on them is measured.
  `text` 7:1; secondary and label text, the accent, the categorical slots, conditions,
  region states and code 4.5:1, the slots on every ground text sits on (`ground`, `surface`,
  `surface-raised`); focus rings, authorship marks 3:1; `text-faint` 2.5:1, which is why it
  is reserved for disabled and decorative marks and never carries information. `themes.sh contrast <theme.json>` prints every pair.
- **Distinctness.** The four region states, the nine categorical marks (eight slots and
  the neutral), the two authorship marks and the conditions must stay apart; and no
  slot may read as a chrome or state role (`accent`, `author-*`, `notice`, `region-*`)
  — a vocabulary bound to a slot that paints like the accent would dress its terms in
  meaning they do not carry (CIE76 ΔE on the painted colour).
  `themes.sh slots <theme.json>` prints the slot canvas. Colour is only one of the
  channels that separates a failed read from a refused one, but a theme that collapses
  two of them has removed a channel, and that is checkable.
- **Generated files agree.** `theme.css` matches its `theme.json`; `contract/tailwind.css` and
  `contract/tailwind-roles.css` cover every role in the schema.
- **Counterparts pair.** A theme's `counterpart` exists, points back, and has the
  opposite `appearance`.

And across the tree: the contract CSS carries no literal colour and references no role the
contract lacks; the brand marks paint in `currentColor`; every fixture in
`tests/fixtures/broken/` is refused.

## Using a theme

```css
@import "tailwindcss";
@import "<themes>/contract/tailwind.css";      /* roles as utilities; default palette reset */
@import "<themes>/contract/shadcn-bridge.css"; /* optional: shadcn-svelte components in-theme */
@import "<themes>/quiet-instrument/theme.css";
@import "<themes>/quiet-instrument-paper/theme.css";
@import "<themes>/contract/base.css";
```

Then choose the theme with one attribute: `<html data-theme="quiet-instrument">`.

`contract/tailwind.css` resets Tailwind's default palette. A component cannot reach for
`zinc-800` or `white`; if it needs a colour, it names a role. A surface still painting from
the default palette imports `contract/tailwind-roles.css` instead until nothing reads it —
the reset would otherwise turn every unmoved `zinc-*` into a class that paints nothing.

A theme can also be applied without a build: a surface can read a `theme.json` and set
each `--tp-*` property on the root element. That is how a theme installed after the app
was built — or one shipped by a plugin — takes effect.

## The colour guard

The reset makes a stock colour paint nothing; it does not say so. `guard/colours.mjs` does: it
fails a surface's build when a component it guards paints anything but a role.

- **A literal colour**: hex, a colour function, or a named colour.
- **A Tailwind colour utility** naming the stock palette, `white` or `black`, or a colour
  name the surface declares for itself (`--color-<name>` in the theme files it lists).
- **An opacity modifier on a role** (`bg-tp-accent/15`): a strength no theme chose. Use the
  role's `-wash` or `-hover` sibling.
- **A colour read from a variable that is not a role**, in a declaration
  (`color: var(--parchment)`), an SVG attribute or Svelte style directive
  (`fill="var(--obsidian)"`, `style:color=…`), or an arbitrary utility (`bg-[var(--obsidian)]`).
  A declared carrier may be read (below).
- **A role mixed down** (`color-mix(in srgb, var(--tp-accent) 15%, transparent)`): the opacity
  modifier spelled in CSS, and the same strength no theme chose. A bridged name mixed down is a
  role mixed down one hop removed, and is refused the same way.
- **A custom property declared from a variable that is not a role**
  (`--wx-color-font: var(--color-quiet-fg)`): a private name laundered through a property the
  colour-declaration rule does not read by name. Fonts are held too: `var(--tp-font-ui)`, not
  `var(--font-sans)`. A bridge stays legal: a name declared within the guarded paths whose every
  declaration there resolves to roles (`--background: var(--tp-ground)`, then
  `--color-background: var(--background)`) may be read. A declaration that reads no variable
  (`style="--hue: {hue}"`, a colour from script) unbridges its name. Svelte's `style:--x=`
  directive is a declaration like any other. The guard sees only the paths: a name also declared
  elsewhere (a vendor stylesheet, a selector scope it is not bridged in) is not seen there, and a
  declaration wrapped across lines is not read. The rule cannot tell what a custom property
  holds, so it over-reaches: a spacing alias (`--gap: var(--space-column)`) is refused as though
  it were a colour. Read the variable where it is used (`gap: var(--space-column)`) instead.
- **A colour read from a bridge that is not a declared carrier.** A bridge may be declared from,
  but painting with it needs the surface to vouch for it in `"carriers"`: a bridge proves only
  the declarations inside the paths, and a vendor stylesheet may set the same name to a literal
  (SVAR's `--wx-color-danger: #fe6158`). A carrier (`--hue` on a view's root, carrying a doc
  type's slot) must be declared by writing one role out — `var(--tp-cat-2)` or
  `var(--tp-{tint})`, no fallback, no other expression — must resolve to roles, and must be
  read. A declaration inside `<script>` declares nothing.

A surface commits a config naming what it guards, relative to the config's own directory,
and runs the guard against it:

```json
{ "paths": ["src/lib/components/Sidebar.svelte", "src/routes/(app)"], "themes": ["src/app.css"] }
```

```sh
node node_modules/@tasker-systems/temper-design/guard/colours.mjs colour-guard.json
```

A surface moving off the stock palette lists the areas it has moved, and each area that
moves adds its paths. An empty list, a listed path that does not exist, and a listed
directory with no components are refused rather than passed. `node --test
guard/colours.test.mjs` plants each class and requires its refusal.

A variable that is not a role yet, but that a named task will make one, is declared rather than
exempted:

```json
"pending": [{ "var": "--tint", "until": "T12", "why": "plugin tint; the plugin binds its doc types to slots" }]
```

Its reads pass, and so does any `color-mix()` reading it, a role inside that mix included,
since the result is part of the hole. Every other variable beside it is still held. Every run
prints how many reads there are and what closes them. An entry missing `var`, `until` or
`why`, or naming a role, is refused, and so is an entry nothing reads any more: its hole has
closed, and the entry would outlive it.

## Bound vocabularies and categorical slots

Colour is bound, never named. The contract owns eight categorical slots and a neutral;
**vocabularies own the bindings to them** — core's doc types through core's declared
default binding, a plugin's doc types through its own definition. A doc type bound
nowhere renders in the neutral role. Many doc types may share a slot; a binding to
anything but a slot is refused.

```json
{ "doctype": "story", "tint": "cat-1" }
```

The binding is part of the vocabulary that declares the doc type — core's default for
core's kinds, the package's entry for a plugin's — so the same term keeps the same slot
in every theme and on every surface that honours it. Nothing in any app names a
`doctype-*` role, because no such role exists: a view tints by **one vocabulary per
colour channel**, so one slot never means two words in the same view. Where a hue marks
a doc type, the doc type's word is shown beside it — hue never carries identity alone.

Core's default binding is `./vocabulary`'s `CORE_BINDINGS`, grouped by what the doc types are
for: `task` cat-2; `goal`, `commitment` cat-5; `session`, `memory` cat-3; `research`, `question`
cat-7; `decision`, `principle` cat-4; `concept`, `domain`, `theme` cat-6; `fact` cat-8; `concern`
cat-1. A surface paints the slot as written role text (`var(--tp-{tint})`), so the colour guard
reads it. `bindingsFrom` checks a plugin's binding and resolves it alone; a surface honouring
several vocabularies merges them before resolving.

## Adding a theme

1. Copy a theme directory and rename it; set `name` to the directory name.
2. Change values in `theme.json` — only values; the roles are fixed.
3. `scripts/themes.sh build` to regenerate `theme.css`.
4. `scripts/themes.sh check <name>/theme.json`, and `contrast` to see where you stand.
5. If it has a light/dark partner, set `counterpart` on both.

## The house theme

**Quiet Instrument** carries temper's visual voice to every surface: obsidian ground,
parchment text, one steel-blue accent; Source Serif 4 for reading, JetBrains Mono for
doing, Inter only for dense UI. Semantic colour is a whisper — the conditions and the
categorical slots exist for meaning, not decoration. Hover shifts colour and nothing
else; there is no bounce, no shadow, no gradient. The rail — a 2px accent-soft left
border — is the markdown blockquote marker made structural.

**Quiet Instrument — Paper** keeps the same restraint on a warm paper ground with ink
text and a deepened accent; every colour was re-chosen to clear the floors, not inverted.

Fonts are bundled by each surface (`@fontsource-variable/*`), never fetched by the contract:
a surface must be able to render offline. `preview/index.html` loads them from Google Fonts for convenience only.

## Setup

```bash
cd packages/temper-design
uv sync
scripts/themes.sh --self-check
```
