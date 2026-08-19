# `/theory` Tier Migration — Design

**Date:** 2026-05-12
**Status:** Approved
**Implements:** `docs-ia-proposal.md` (Tier 1 + Tier 2 + Tier 2b + Tier 2c sections)
**Out of scope:** Tier 4 (`/docs` → `/using-temper` rename), separate session

## Goal

Migrate the 9 hand-drafted `/theory` page sources at the repo root into the temper-ui SvelteKit site as a new `(public)/theory/*` route tier. The existing temperkb.io surface (`/`, `/agents`, `/builders`, `/how-it-works`, `/docs`) is not touched.

Source drafts to be migrated:

| Source file (repo root) | Target route |
|---|---|
| `theory-entry-draft.md` | `/theory` |
| `theory-ontology-draft.md` | `/theory/ontology` |
| `theory-manifold-draft.md` | `/theory/manifold` |
| `theory-time-draft.md` | `/theory/time` |
| `theory-deformation-draft.md` | `/theory/deformation` |
| `theory-perspectives-draft.md` | `/theory/perspectives` |
| `theory-translation-draft.md` | `/theory/translation` |
| `theory-schema-draft.md` | `/theory/schema` |
| `theory-open-questions-draft.md` | `/theory/open-questions` |

## Resolved architectural choices

These were settled during brainstorming (2026-05-12 session). The implementation plan should not revisit them.

1. **Rendering mechanism — hand-translate to Svelte HTML.** Each page is a hand-written `+page.svelte` matching the `/docs` and `/how-it-works` pattern. No mdsvex; no runtime markdown rendering; no `?raw` markdown imports. The drafts are not retained as canonical source — the `+page.svelte` becomes canonical going forward.
2. **Tier chrome — light footer nav via shared `+layout.svelte`.** Top: `← Theory` backlink on sub-pages. Bottom: prev/next pair. No persistent sidebar.
3. **Source-draft disposition — delete after migration.** Git history is the audit trail.

## Architecture

### Routing and file layout

```
packages/temper-ui/src/routes/(public)/theory/
├── +layout.svelte                 ← tier-shared chrome (backlink + prev/next)
├── TheoryNav.svelte               ← prev/next component (tier-scoped, not in $lib)
├── +page.svelte                   ← /theory entry
├── ontology/+page.svelte
├── manifold/+page.svelte
├── time/+page.svelte
├── deformation/+page.svelte
├── perspectives/+page.svelte
├── translation/+page.svelte
├── schema/+page.svelte
└── open-questions/+page.svelte
```

The existing `(public)/+layout.svelte` provides outer-site chrome. The new `theory/+layout.svelte` nests inside it.

Inline cross-links in the drafts (`[Ontology](/theory/ontology)` etc.) become `<a href="/theory/ontology">` in the rendered pages — SvelteKit handles client-side navigation automatically.

### Shared tier chrome

`(public)/theory/+layout.svelte` conditionally renders chrome based on `$page.route.id`:

- **Entry page (`/theory`):** layout adds no chrome. The entry page already contains the sub-page index and reads as a hub. The outer-site layout still wraps it.
- **Sub-pages (every other `/theory/*`):** a small `← Theory` backlink at the top; a `<TheoryNav prev next />` pair at the bottom.

**Canonical reading order** (drives prev/next):

```
/theory
  → /theory/ontology
  → /theory/manifold
  → /theory/time
  → /theory/deformation
  → /theory/perspectives
  → /theory/translation
  → /theory/schema
  → /theory/open-questions
```

The first sub-page (`/theory/ontology`) has no `prev`; the last (`/theory/open-questions`) has no `next`. The layout component owns the ordering as a static array; sub-pages do not pass their own neighbors.

### Page-level components and visual register

Each `+page.svelte` follows the `/how-it-works` register:

- A hero block at the top — `<div class="hero-label t-label">`, `<h1 class="t-hero-title">`, `<p class="tagline t-tagline">` — sourced from the draft's framing sentences.
- Subsequent sections use the existing `$lib/components/landing/Section.svelte` with a `label` prop. Section provides serif `h2`, mono section label, 2px parchment left border, max-width 800px.
- Prose lists (`ul`/`ol`), blockquotes, inline `code`, and emphasis (`em`, `strong`) use plain HTML elements. Section's `:global()` styles cover `h2`, `p`, `p strong`. Anything Section does not cover (`ul`, `ol`, `blockquote`, inline `code`, links inside prose) needs styling. The implementation should:
  1. Add per-page scoped `:global()` rules for elements that need styling.
  2. If a pattern repeats across all 9 pages (likely: `ul`, `a` underline color, inline `code`), promote it to the `theory/+layout.svelte` global block.
- `Footer.svelte` (`$lib/components/landing/Footer.svelte`) — confirm during implementation whether it is already rendered by the outer `(public)/+layout.svelte` or needs to be added per-page. Match the existing pages' behavior, do not double-render.

#### Schema page special case

The schema page (`/theory/schema`) carries a WIP marker per the IA proposal: a one-paragraph framing note at the top of the page, not a banner. The schema draft contains many tables (entity types, event structure, topic taxonomy, accountability vectors, chain-link-kinds). These render as semantic HTML tables. A single shared `.theory-table` style is added — either scoped to the schema page or promoted to `theory/+layout.svelte` if no other page uses tables.

#### Open-questions page special case

The open-questions page uses two top-level `<section>` blocks with stable IDs `model` and `schema`. The IA proposal anchors cross-links from `/theory/schema` to `/theory/open-questions#schema`, so these anchors are load-bearing — do not change them.

### Page metadata

Each page sets `<svelte:head>`:

- `<title>` — `"<page title> — temper"` (matches the existing `/docs` pattern: `Docs — temper`).
- `<meta name="description">` — one sentence drawn from each draft's framing line.

The entry page's title is `"What Temper is building toward — temper"` (the heading the draft proposes).

### New components introduced

| Component | Path | Props | Purpose |
|---|---|---|---|
| `TheoryNav.svelte` | `routes/(public)/theory/TheoryNav.svelte` | `prev?: {href, title}; next?: {href, title}` | Prev/next pair at the bottom of each sub-page. |

Co-located with the routes (not under `$lib/components/`) because it is not reusable outside this tier.

No other new components. The implementation reuses `Section.svelte` and (if needed) `Footer.svelte` from `$lib/components/landing/`.

## Files touched

### Created (11 files)

- `packages/temper-ui/src/routes/(public)/theory/+layout.svelte`
- `packages/temper-ui/src/routes/(public)/theory/TheoryNav.svelte`
- `packages/temper-ui/src/routes/(public)/theory/+page.svelte`
- `packages/temper-ui/src/routes/(public)/theory/ontology/+page.svelte`
- `packages/temper-ui/src/routes/(public)/theory/manifold/+page.svelte`
- `packages/temper-ui/src/routes/(public)/theory/time/+page.svelte`
- `packages/temper-ui/src/routes/(public)/theory/deformation/+page.svelte`
- `packages/temper-ui/src/routes/(public)/theory/perspectives/+page.svelte`
- `packages/temper-ui/src/routes/(public)/theory/translation/+page.svelte`
- `packages/temper-ui/src/routes/(public)/theory/schema/+page.svelte`
- `packages/temper-ui/src/routes/(public)/theory/open-questions/+page.svelte`

### Deleted (9 files)

- `theory-entry-draft.md`
- `theory-ontology-draft.md`
- `theory-manifold-draft.md`
- `theory-time-draft.md`
- `theory-deformation-draft.md`
- `theory-perspectives-draft.md`
- `theory-translation-draft.md`
- `theory-schema-draft.md`
- `theory-open-questions-draft.md`

### Moved (1 file)

- `docs-ia-proposal.md` → `docs/theory-ia-proposal.md`

This relocates the finished IA design artifact to a permanent home. It is referenced as `Implements:` in this spec's front matter.

### Untouched

- Every existing route under `(public)/`: `+page.svelte`, `agents/`, `builders/`, `docs/`, `how-it-works/`.
- The outer `(public)/+layout.svelte`.
- Any other crate, package, or doc surface.

## Verification

1. **Type-check / svelte-check passes**

   ```bash
   cd packages/temper-ui && bun run check
   ```

2. **Dev server walk-through**

   ```bash
   cd packages/temper-ui && bun run dev
   ```

   With the dev server running, manually verify:
   - `http://localhost:5173/theory` renders the entry page; section index lists 6 model pages + 2 reference surfaces; no `← Theory` backlink at top; no prev/next at bottom (entry is the hub).
   - Each `/theory/<slug>` route renders with: hero block, sectioned prose, `← Theory` backlink at top, prev/next at bottom.
   - Prev/next round-trips the canonical order. First sub-page has no prev; last has no next.
   - Inline cross-links in prose navigate without full-page reload.
   - Visual register matches `/how-it-works` and `/docs` (serif body, parchment headings, mono section labels, max-width ~800px).
   - `/theory/schema` carries the WIP framing note and renders its tables legibly.
   - `/theory/open-questions` has working `#model` and `#schema` anchors.

3. **No regressions on existing pages**

   - `/`, `/agents`, `/builders`, `/how-it-works`, `/docs` render unchanged.
   - Outer-site navigation, header, footer unchanged.

4. **Repo-root cleanup**

   - The 9 `theory-*-draft.md` files are gone.
   - `docs-ia-proposal.md` is gone from root; `docs/theory-ia-proposal.md` exists with identical content.

No new automated tests are added. The package's existing test pattern (`sanity.test.ts`) does not cover route rendering, and adding test scaffolding for static content pages is overkill for this feature.

## Out of scope (explicit)

- **`/docs` → `/using-temper` rename** (IA proposal Tier 4). Separate session. No redirect stubs added in this work.
- **Linking `/theory` from the landing.** The IA proposal explicitly defers landing-side integration; `/theory` is reached via external links in this pass.
- **Visual redesign or new design tokens.** New pages inhabit the existing register.
- **Internal link audits across other pages.** No existing page currently links to `/theory`; nothing to update.
- **MarkdownRenderer reuse.** `$lib/components/MarkdownRenderer.svelte` exists but is not used — the chosen mechanism is hand-translation to HTML.
- **Reusing drafts as runtime/build-time content sources.** They are deleted; the Svelte pages are canonical.

## Implementation notes for the plan-writing step

- Translation is **content-faithful, not source-faithful.** The drafts contain editorial annotations at the bottom (e.g., "Editorial notes", "Things considered and rejected"). These do not migrate — they were brainstorming output, not page copy.
- The drafts use inline italics and bold liberally. Preserve them as `<em>` and `<strong>`; do not lose emphasis in the translation.
- The entry page has a voice-shift mid-page (manifesto first-person → semantic-model third-person → manifesto first-person closing). The draft's editorial notes call this out; the rendered page must preserve the shift without inserting hard breaks the draft does not have.
- The schema page is dense. Plan it as its own implementation task with extra verification, not as one-of-nine.
- Verify whether the outer `(public)/+layout.svelte` already renders `Footer` before deciding to add it per-page; the existing `/how-it-works` page imports and renders `Footer` itself, suggesting the outer layout does not — but confirm.
