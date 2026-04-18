# Temper Design System

<p align="left">
  <img src="assets/brand-mark.svg" alt="temper" width="200" />
</p>

> **/ˈtempər/** — *to make stronger and more resilient through a deliberate process*

Temper is a knowledge base for builders — a structured markdown vault that gives AI coding sessions a **throughline** across time, so context compounds instead of decaying. This folder is the design system for building on the Temper brand.

---

## Index

| File / folder | What's in it |
|---|---|
| `README.md` | This file. Brand, content, visual, iconography fundamentals. |
| `SKILL.md` | Frontmatter-tagged skill file (works as a Claude Code Agent Skill). |
| `colors_and_type.css` | All color tokens + semantic type classes. The source of truth. |
| `assets/` | Logos, favicon, brand mark, social preview, diagrams. |
| `fonts/` | Font-loading instructions. (Temper doesn't self-host fonts.) |
| `preview/` | The Design System cards — each small HTML showing one concept. |
| `ui_kits/landing/` | Public marketing site components + a reconstructed landing page. |
| `ui_kits/app/` | Authed vault browser — sidebar, grid, markdown viewer, palette. |

---

## Sources consulted

All design decisions trace to real files in the Temper production repository. Paths below are relative to the repo root (`github.com/tasker-systems/temper`).

| Source | What we lifted |
|---|---|
| `github.com/tasker-systems/temper` → `README.md` | Product framing, tone of voice, command vocabulary |
| `docs/brand-direction.md` | Color names, persona voice, diagram strategy — this was the canonical brand brief |
| `packages/temper-ui/src/app.css` | Every color var, every `.ed-*` editorial class, Tailwind `@theme` scale |
| `packages/temper-ui/src/app.html` | Font stack (`JetBrains Mono` via Google; Georgia system), meta copy |
| `packages/temper-ui/src/lib/components/landing/*.svelte` | Hero, Section, CliBlock, AgentTranscript, Nav, Footer, Wordmark |
| `packages/temper-ui/src/lib/components/*.svelte` | Sidebar, VaultGrid, CommandPalette, MarkdownRenderer, FacetChips, ResourceMetaHeader |
| `packages/temper-ui/src/lib/graph/styling.ts` | Knowledge-graph node colors (research/task/session/concept) |
| `packages/temper-ui/src/routes/(public)/+page.svelte`, `.../builders/+page.svelte`, `.../agents/+page.svelte` | Landing page structure, CLI block patterns, section rhythms |
| `packages/temper-ui/src/routes/(app)/vault/...` | Authed shell, resource header, vault grid |
| `packages/temper-ui/static/brand-mark.svg`, `packages/temper-ui/static/social-preview.svg` | The "threaded t" brand mark at its canonical geometry |
| `packages/temper-ui/static/diagrams/*.svg` | Existing light-mode diagrams (context-rot, throughline-layers, session-continuity-cycle) |

If you need to see how a pattern renders in the real app, open the matching file in the `temper` repo.

---

## The product, briefly

Temper has two public surfaces:

1. **Marketing / landing site** (`temperkb.io`, `/builders`, `/agents`, `/docs`) — public, SvelteKit, dark editorial ground. Pitches the product and its thinking.
2. **Authed vault browser** (`/vault/...`) — dark app chrome, sidebar + grid + markdown reader. The same color palette as the landing but with denser UI.

Both share one visual voice: *Quiet Instrument* — obsidian ground, parchment text, one steel-blue accent, Georgia for reading, JetBrains Mono for doing.

---

## The principle above all others

> **Let the work speak for itself.**

Every decision in this design system traces back to this one sentence. It's why the palette is almost monochrome. Why the ground is obsidian, not branded. Why the icons don't exist. Why the diagrams use typeset words instead of boxes. Why color, when it appears, fades into parchment by the end of the letter. Why hover states shift an alpha channel and nothing else. Why the hero animations don't.

Temper is a knowledge base for builders. The builder's work is the subject; the interface is the frame. If a design choice draws attention to itself — a shadow, a gradient, an icon, a motion cue — it's probably wrong. When in doubt, remove it.

This principle sits above both halves of the system:
- **Content fundamentals** express it in language: no marketing adjectives, one italic per heading, sentences that earn their em-dashes.
- **Visual foundations** express it in form: flat rectangles, hairline rules, semantic colour as a whisper, typography doing the work that chrome would elsewhere.

---

## Content fundamentals

Temper's voice is **literate technical** — write as if explaining to a sharp colleague over coffee. Assume intelligence; don't oversimplify. The pattern throughout the site: *one italicized word per heading, always the conceptual anchor.*

### Tone rules

- **Register:** precise, opinionated, confident without being salesy.
- **Rhythm:** short declarative sentences for claims, longer sentences with subordinate clauses for elaboration. The em-dash is our friend — it lets us layer context without losing momentum.
- **Paragraphs:** 2–4 sentences. Never bullet points in marketing copy; the site should feel like *reading*, not *scanning*.
- **Pronoun:** "you" for the reader (a builder). Temper doesn't refer to itself in first person — it says *Temper* by name.
- **Casing:** Sentence case for headings. Product name `temper` is **lowercase** in running copy (the wordmark is always lowercase). "Temper Cloud" and "Temper Blue" take initial caps because they're proper nouns.
- **Emoji:** never. Not in copy, not in UI, not in diagrams.

### Vocabulary table (from `docs/brand-direction.md`)

| Prefer | Over |
|---|---|
| throughline | context window, memory |
| vault | workspace, repository |
| session | conversation, chat |
| temper (verb) | manage, organize, maintain |
| compounds | grows, scales |
| resilient | robust, reliable |
| intention | configuration, setup |
| narrative | data, information |
| resolves to markdown | outputs markdown |

### Avoid

- *AI-powered*, *intelligent*, *revolutionary*, *transform* — Temper is agent-aware, not agent-branded.
- *Simple*, *easy* — the tool respects complexity. Use *considered* or *deliberate*.
- *Never lose context again* and similar absolutes. The framing is *compounding*, not *perfection*.
- *Knowledge management* (sounds like enterprise middleware). This is a *knowledge base* with *structure*.

### Example hero phrases (real, from the codebase)

> Clarify your *intention*. Know what was decided, what's deferred, and what comes next. Every session builds on the last. **temper** your context.

> Remember what you *decided*. Every session builds on the last.

> Context that's always *ready to hand*. If it can read files, it can use temper.

### Example body phrasing

> Code is an expression of intent. So are specifications, plans, and decisions. But the context behind them — the *why*, the alternatives considered, the constraints that shaped the choice — scatters across conversations, documents, and memory.

> The frontmatter carries the throughline. The content carries the thinking. Git carries the history.

---

## Visual foundations

### Ground & palette

Temper runs on a **dark editorial ground** — obsidian `#0a0a0f`, not pure black. All chrome is monochrome plus a single muted steel-blue accent `#7eb8da`. Secondary semantic colors (session green, decision amber, deferred slate, rot red) exist **only in diagrams and knowledge-graph nodes** — they never appear in page chrome.

See `colors_and_type.css` for every token. Three thresholds of white are the whole UI:

- `rgba(255,255,255,0.65)` → secondary text (chalk)
- `rgba(255,255,255,0.45)` → tertiary text, labels (graphite)
- `rgba(255,255,255,0.06)` → hairline rules

### Typography

- **Serif (Georgia)** for every reading surface — hero h1, section h2, body p, taglines, blockquotes, markdown prose. Georgia is used *unmodified*, no custom serif webfont. Weight 300 for heroes and h2s. Italic Georgia for the one emphasized word per heading.
- **Mono (JetBrains Mono)** for every technical surface — nav, CLI blocks, section labels (the uppercase mono eyebrow above every h2), buttons, badges, wordmark, frontmatter keys. Loaded from Google Fonts at weights 300/400/500.
- **No sans in marketing.** Inter appears only inside the authed vault browser for dense grid UI (via Tailwind).

Hierarchy is disciplined: one size per role, set in `colors_and_type.css` as `.t-hero-title`, `.t-h2`, `.t-label`, `.t-body`, `.t-code`, `.t-tagline`, `.t-strip`.

### Spacing & layout

- **Single column, 800px max-width** for marketing pages. 640px for hero copy. No multi-column grids for body text — the reading experience is linear, like a well-typeset essay.
- **Section rhythm:** 5rem vertical padding between sections, separated by a thin `rgba(255,255,255,0.06)` horizontal rule.
- **The left-border motif:** Every `<Section>` content block has a 2px steel-blue left border, indented 2rem. This is the markdown-blockquote vocabulary made structural. It's the single most recognizable Temper layout cue.
- **Cards** are rare. When used (vault concept cards, block-cards on Builders page), they're flat: 1px `rgba(255,255,255,0.06)` border, no shadow, no rounded corners, `1.2rem` padding. Hover lifts the border color to `--temper-blue-border-dim`.
- **No drop shadows anywhere on marketing.** The authed grid uses minimal inset shadows only.

### Borders, rules, cards

- Dividers: 1px solid `rgba(255,255,255,0.06)`.
- Accent borders: 1–2px solid `rgba(126,184,218,0.25)` for soft, `0.50` for buttons.
- Cards/panels: flat fills with `rgba(255,255,255,0.02–0.03)` + a 1px rule border. **Radius is mostly 0** on marketing; authed UI uses `3–4px` for pill chips and code blocks.
- The editorial `ed-rail` motif: `border-left: 2px solid rgba(126,184,218,0.25); padding-left: 1.8rem`.

### Motion, hover, press

- **Transitions:** all default to `0.15–0.3s ease` on `color`, `border-color`, `background`, `opacity`. No spring, no bounce, no transform animations on hover.
- **Hover states** are uniformly subtle: color shifts from `--graphite` → `--chalk`, or `--temper-blue-dim` → `--temper-blue`. Buttons add a `rgba(126,184,218,0.1)` background fill. Cards raise their border color.
- **Active/press:** no shrink, no scale. Links and buttons just hold their hover state.
- **Page load:** no hero animations, no scroll-triggered reveals. The site feels *settled*. (The production repo hints that diagrams *could* animate a drawing-in thread on scroll; treat that as a future flourish, not a baseline expectation.)

### Protection gradients, blur, transparency

- **Fixed nav** uses `backdrop-filter: blur(12px)` and `rgba(10,10,15,0.95)` only after scroll > 40px. Before that, transparent.
- No scrim gradients, no glass panels, no neumorphism. Transparency is used purely to layer the card/panel tones over the obsidian ground.

### Imagery vibe

- **Cool.** Cool to the point of being monochrome — the dark ground is obsidian, the accent is steel-blue.
- **No photography** in the current site. No illustrative SVG characters. No hand-drawn textures. No gradients, no noise/grain.
- **Diagrams are the primary visual.** Existing ones (in `assets/diagrams/`) are light-mode SVGs; the brand direction calls for dark-mode re-renderings on `#0a0a0f` or lifted `#12121a` grounds. When producing new diagrams: thin strokes (0.5–1px), Temper Blue for primary connections, secondary palette for semantic roles, JetBrains Mono for labels, Georgia for captions.

### Radii

Essentially zero. `0` on marketing cards, `2–4px` on authed pills and code blocks, `50%` on the "step number" circles in cycle diagrams. That's the whole scale.

### Cards

Flat rectangle. 1px rule border. No shadow. Sometimes left-border accented in blue (blockquote vibe). Hover raises border contrast by ~2× alpha. No other treatment.

---

## Iconography

**There is no icon system in Temper.** This is deliberate.

- **No icon font, no SVG sprite, no Heroicons/Lucide/Tabler.** The production repo ships with exactly three SVGs: the brand mark, the social preview, and a favicon — all variants of the same "threaded t" glyph.
- **No emoji, ever.**
- **No decorative unicode characters** — except `→`, `←`, `↓`, `·`, em-dash `—`, and the middle dot `·` for separators. These are typeset, not iconic.
- **Section labels do the work icons normally do.** Instead of an icon + label pair, Temper uses uppercase JetBrains Mono labels (`WORKFLOW`, `SESSION CONTINUITY`, `THE PROBLEM`) to announce sections.
- **Status & semantic meaning comes from color + typography**, not glyphs. "In-progress" is a `--temper-blue` small-caps mono tag, not an icon badge.

### The one exception: the brand mark

The **threaded t** — a lowercase "t" where the crossbar curves and extends into a continuing line, suggesting a thread weaving onward. Lives at `assets/brand-mark.svg` (wordmark version) and `assets/favicon.svg` (mark-only). Renders in `currentColor` so it inherits from its parent text color — Temper Blue on dark, or dark on light.

Use the mark:
- at nav height (20×20)
- at footer height (16×16)
- on social preview (full with wordmark + tagline)
- as favicon (16×16, 32×32)

### When you need iconography we don't have

If you're designing something that genuinely needs a checkbox, close button, or chevron, use **Lucide** (stroke = 1.5px, color = `currentColor`) as the substitute — it has the right restrained, editorial weight. **This is a substitution; flag it to the Temper team when you use it.**

### Knowledge graph — visual language

The `/vault/<ctx>/graph` view is the most distinctive surface in the product. It deserves its own design record because a generic force-directed graph (dots + edges) under-represents Temper's actual data model. The R11 research pass established two structural classes — **participants** (research / task / session) and **aggregators** (goal / concept / decision) — and the visualization must make that distinction visible *before* anyone reads the legend.

#### Decision: the word is the node; gravity is the grouping

Settled after comparing three approaches (see `preview/kg-scene.html` and `preview/kg-scene-v2.html`):

1. **Participants** — research, task, session — are rendered as their **slug (or title) typeset in Source Serif 4**, 12–14px, with the gradient-pour coloring used throughout the diagram system (saturated top → parchment floor). The word *is* the node. No circle, no pill, no chip. Under each word, in mono-caps at 7.5px rgba(255,255,255,0.32), an edge count and stage (`12 EDGES · IN-PROGRESS`).

2. **Aggregators** — goal, concept, decision — are rendered as **larger, bolder, italic Source Serif 4 at ~19px**, with a **mono-cap overscore** above (`GOAL`, `CONCEPT`, `DECISION`) in the aggregator's color at 0.7 alpha, letter-spacing 0.25em. A faint **radial gravity wash** (the aggregator's color at 0.07 alpha, fading to 0) sits behind them at ~220×150px. The wash is what communicates "this is an attractor" without a frame.

3. **No frames in the default view.** We explicitly rejected the containment metaphor (rectangle-around-children) because it introduces a tabular visual language into what is fundamentally a force-directed graph. Containment and proximity fight for meaning; proximity wins because it matches the underlying physics. Aggregators pull their children close via **higher force-simulation mass**, not via rectangles. See the verdict section in `kg-scene-v2.html`.

4. **Inspection uses one docked panel for every node type.** Clicking any node — participant or aggregator — opens a right-docked resource peek (`420px` wide, obsidian fill, 1px left border in the node's type color at `55` alpha, 240ms slide-in from the right). The panel shows: doctype marker, title, session annotation count if any, *members or neighbors* list, and an excerpt. There is **no center-screen expand overlay.** Aggregator members and participant neighbors use the same grammar — the list header just reads `MEMBERS · N` vs `NEIGHBORS · N` to name the relationship honestly. Clicking any row drills into it: the peek rebinds to that node, camera recenters.

5. **Traversal is explicit via breadcrumb.** Because drilling feels like moving through an association graph, we expose the path as a breadcrumb at the top of the peek. A fresh canvas click starts a trail with that one node (no breadcrumb shown — depth 1 is implicit). Click a member or neighbor → pushes onto the trail. Breadcrumb renders as `llm-wiki › temper-index › throughline`, with aggregators in italic serif and participants in mono caps so the crumb itself announces its type. Any earlier crumb is clickable to slice back to that depth and recenter the graph. Trails of 5+ auto-collapse the middle with `first › … › penult › current`. Closing the peek resets the trail — traversal is current-interaction state, not persistent.

6. **Legend is standalone chrome, stays visible under the peek.** Top-right panel with dark fill + hairline border, `z-index: 16` above the peek. The peek docks *below* it (top offset `168px`) so the legend corner is never eclipsed. Legend holds: the four node-color rows, the `⌊N⌋ SESSIONS · ANNOTATION, NOT EDGE` note, and the contextual hint (`— CLICK ANY NODE TO PEEK —`).

#### Edge vocabulary (stroke encodes type)

All edges `rgba(255,255,255,0.09)` default, 0.75px. Stroke-dash encodes semantic:

| Edge type | Stroke | Meaning |
|---|---|---|
| `depends_on` | solid | structural certainty |
| `extends` | solid | builds upon |
| `preceded_by` | long-dash `4,3` | temporal sequence |
| `relates_to` | medium-dash `2,2` | soft association |
| `references` | dotted `1,3` | citation, weakest |
| *emergent* (meta-doc mode only) | hairline arc, no dash | Jaccard-derived, not declared |

#### Hover — color pours from the source

On node hover: all incident edges lift to 1.1px stroke and take a **linear gradient along their length** — saturated source-color at the source end, target-color at 0.35–0.45 alpha at the target end. All other edges drop to `rgba(255,255,255,0.04)`. A small **inspector pane** appears as right-edge marginalia (see `kg-scene-v2.html` hover mock) with the hovered doc's title, up to five labeled relationships, and the session count.

#### Sessions — marginalia, not participants

Per R11 D4 (sessions deferred), sessions don't crowd the structural graph. Each referenced doc gets a tiny mono-cap session-count halo in session green to its upper right — `⌊3⌋` means "three sessions reference this." Click to open a side list. Sessions only appear as full nodes in their own cluster region (right side of canvas).

#### Context as watermark

The active context name sits at 58–80px, italic Source Serif 4, `rgba(255,255,255,0.035)`, behind everything — `CONTEXT  *temper*`. It answers "where am I?" without competing.

#### Zoom tiers

Three explicit thresholds, not continuous:

| Zoom | What's visible | Purpose |
|---|---|---|
| **Overview** (≤25%) | Gravity washes + mono-cap cluster labels + dots for participants | Cluster-level structure |
| **Neighborhood** (25–75%) | Full typography for nodes within viewport bounds + aggregator treatments | Local reading |
| **Detail** (>75%) | Edge stroke-dash visible + session halos + hover pane | Essay-level density |

Transitions are **220ms threshold fades**, not continuous — consistent with the "settled" motion language.

#### Naming — truncation and date-stripping

Slugs in the vault are often long and date-prefixed (`2026-04-17-doctype-qualified-slugs-design-decision`). Rendered labels:

- Strip ISO date prefixes → move to mono-cap marginalia under the word (`APR 17`)
- Strip doctype prefixes when doctype is already color-encoded (`r11-` stays because it's semantic; `research-` would be redundant)
- Hyphens → spaces (cleaner read in serif)
- Cap at ~22 chars with single-line ellipsis; full title on hover
- Optional frontmatter `display_name:` overrides the auto-derived label

#### Scale target

Beta: ~500 nodes. Enterprise self-hosted: low-thousands. Renderer choice (**cytoscape.js** with Canvas/WebGL backend) is set against this target, not against the current ~50-node state. Viewport culling on; edges drawn only when both endpoints are in bounds.

#### Node-doctype palette (unchanged)

Colors come from `lib/graph/styling.ts`:

| Type | Color | Hex |
|---|---|---|
| research | steel blue | `#7eb8da` |
| task | warm ochre | `#f0a870` |
| session | moss green | `#82c99a` |
| concept | dusty pink | `#d48ac7` |
| goal | warm gold | `#f5d277` |
| decision | *TBD — to be added when decision doctype lands* | — |

These are the *only* place in Temper where the extended palette touches UI instead of illustrations. Treat them as diagram vocabulary.

#### Canonical references

- **`ui_kits/app/graph.html`** — live working prototype (Cytoscape.js + fcose). All the behaviors in this section — unified peek, breadcrumb traversal, standalone legend, structural mode — are implemented here against a 21-node sample vault. Ship target for the first PR.
- **`preview/kg-scene-v2.html`** — static visual spec: compositional study of the gravity-well aggregator treatment vs the rejected frame-based v1, with verdict annotations.
- **`preview/kg-language.html`** — parts catalogue: every participant, aggregator, edge, and annotation type rendered once with naming.
- **`preview/kg-scene.html`** — archived v1 (frame-based), kept so the *why* of v2 stays readable.
- **`docs/kg-handoff.md`** — implementation handoff: PR sequencing, exact file list, data contract notes, and open questions for production.

---

## Components at a glance

| Component | Source | Preview card |
|---|---|---|
| Brand mark (threaded t) | `packages/temper-ui/static/brand-mark.svg` | `preview/mark.html` |
| Wordmark | `packages/temper-ui/src/lib/components/Wordmark.svelte` | `preview/wordmark.html` |
| Nav (marketing) | `packages/temper-ui/src/lib/components/landing/Nav.svelte` | `preview/nav.html` |
| Footer (marketing) | `packages/temper-ui/src/lib/components/landing/Footer.svelte` | `preview/footer.html` |
| Section (left-rail h2) | `packages/temper-ui/src/lib/components/landing/Section.svelte` | `preview/section.html` |
| CLI block | `packages/temper-ui/src/lib/components/landing/CliBlock.svelte` | `preview/cli-block.html` |
| Agent transcript | `packages/temper-ui/src/lib/components/landing/AgentTranscript.svelte` | `preview/agent-transcript.html` |
| Editorial strip + hero | `packages/temper-ui/src/app.css` (`.ed-*`) | `preview/editorial-hero.html` |
| Resource chips (seq/stage/mode/effort) | `packages/temper-ui/src/lib/components/ResourceMetaHeader.svelte` | `preview/resource-chips.html` |
| Facet chips | `packages/temper-ui/src/lib/components/FacetChips.svelte` | `preview/facet-chips.html` |
| Sidebar item | `packages/temper-ui/src/lib/components/Sidebar.svelte` | `preview/sidebar-item.html` |
| Graph node palette | `packages/temper-ui/src/lib/graph/styling.ts` | `preview/graph-nodes.html` |

For full-page recreations, open `ui_kits/landing/index.html` and `ui_kits/app/index.html`.

---

## Caveats & known substitutions

- **Reading face: Source Serif 4** (SIL OFL, Google Fonts, self-hostable). Replaces Georgia — same old-style proportions, wider weight range, real italic. See `preview/type-stack.html`.
- **Icons: Phosphor Thin** (1px stroke, outline) for utility glyphs in the authed app only. Never paired with a text label, never in marketing. See `preview/iconography.html`.
- **Diagrams:** new editorial aesthetic on `#12121a` lifted panels — the word is the node, a thin blue rule is the container, semantic color only where meaning demands it. See `preview/diagram-*.html`.
- **Tailwind preset:** `tailwind.config.preset.js` (v3) and `tailwind.theme.css` (v4) mirror the production `@theme` block so prototypes can drop into the real codebase verbatim.
- **Diagrams are light-mode.** The five SVGs in `assets/diagrams/` come straight from the production repo and were designed for GitHub README rendering. The brand-direction doc calls for dark-mode rerenderings; when you need one on-site, drop a `<rect fill="#0a0a0f">` behind it or build a new one.
- **No real icon set.** When you must use a utility icon (close, chevron, etc.), substitute Lucide at stroke 1.5px and flag it.
- **Authed app uses Tailwind.** The Svelte source uses Tailwind `@theme` for its `bg-quiet-*` and `border-zinc-*` utilities. The UI kit recreations in `ui_kits/app/` use plain CSS with the same color tokens — functionally equivalent, not class-for-class identical.
