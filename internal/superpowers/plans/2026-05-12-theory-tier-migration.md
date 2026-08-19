# `/theory` Tier Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the 9 hand-drafted `theory-*-draft.md` files at the repo root into a new `(public)/theory/*` route tier in the temper-ui SvelteKit site, with light shared chrome (← Theory backlink + prev/next) and matching the existing visual register.

**Architecture:** Hand-translate each draft into a `+page.svelte` file. A new `(public)/theory/+layout.svelte` adds tier chrome conditionally based on `$page.route.id`. A new `TheoryNav.svelte` component renders the prev/next pair. Drafts are deleted after migration; `docs-ia-proposal.md` moves to `docs/theory-ia-proposal.md`. The existing site (`/`, `/agents`, `/builders`, `/how-it-works`, `/docs`) is untouched.

**Tech Stack:** SvelteKit 2, Svelte 5 (runes mode), Tailwind CSS v4. Type-check via `bun run check` (svelte-kit sync + svelte-check). Visual verification via `bun run dev` and a browser.

**Spec:** `docs/superpowers/specs/2026-05-12-theory-tier-migration-design.md`

---

## Working assumptions for the implementer

- **The 9 draft files at the repo root are the source of truth for prose during this work.** Do not paraphrase. Translate each draft's "Page copy (draft)" section faithfully into HTML. The editorial notes ("Editorial notes", "Things considered and rejected") at the bottom of each draft are **not** page content — they are decisions log. Do not migrate them.
- **Visual register:** match `/how-it-works/+page.svelte` and `/docs/+page.svelte`. Serif body, parchment heading, mono section label, max-width 800px. CSS variables from `src/app.css` (`--parchment`, `--chalk`, `--graphite`, `--temper-blue`, `--rule`, `--font-serif`, `--font-mono`).
- **Reuse `Section.svelte`** from `$lib/components/landing/` for major sub-sections within each page (it provides `:global(h2)` and `:global(p)` styles plus the left-border + mono label).
- **Reuse `Footer.svelte`** rendered from `theory/+layout.svelte` once (the outer `(public)/+layout.svelte` does *not* render a Footer; `/how-it-works` renders it per-page).
- **Cross-links in drafts** use markdown `[label](/theory/...)` syntax. Translate them to `<a href="/theory/...">label</a>`. SvelteKit handles client-side nav automatically.
- **Run `bun run dev` in a separate terminal** at the start. Keep it running throughout. Visual verification means actually loading the page in a browser, not just trusting svelte-check.
- **All paths in this plan are relative to** `packages/temper-ui/` **unless they start with** `docs/` **or are repo-root draft files.**

---

## Canonical reading order (used by prev/next nav)

This sequence is load-bearing — it drives `TheoryNav` prev/next computation in Task 3. Do not change without updating the spec.

```
/theory                       (entry, no prev/next)
/theory/ontology              (prev: /theory,            next: /theory/manifold)
/theory/manifold              (prev: /theory/ontology,   next: /theory/time)
/theory/time                  (prev: /theory/manifold,   next: /theory/deformation)
/theory/deformation           (prev: /theory/time,       next: /theory/perspectives)
/theory/perspectives          (prev: /theory/deformation, next: /theory/translation)
/theory/translation           (prev: /theory/perspectives, next: /theory/schema)
/theory/schema                (prev: /theory/translation, next: /theory/open-questions)
/theory/open-questions        (prev: /theory/schema,     next: none)
```

---

## Task 1: Scaffold tier layout, route stubs, and Footer wiring

**Files:**
- Create: `src/routes/(public)/theory/+layout.svelte`
- Create: `src/routes/(public)/theory/+page.svelte`
- Create: `src/routes/(public)/theory/ontology/+page.svelte`
- Create: `src/routes/(public)/theory/manifold/+page.svelte`
- Create: `src/routes/(public)/theory/time/+page.svelte`
- Create: `src/routes/(public)/theory/deformation/+page.svelte`
- Create: `src/routes/(public)/theory/perspectives/+page.svelte`
- Create: `src/routes/(public)/theory/translation/+page.svelte`
- Create: `src/routes/(public)/theory/schema/+page.svelte`
- Create: `src/routes/(public)/theory/open-questions/+page.svelte`

- [ ] **Step 1: Create the tier layout with Footer**

Write `src/routes/(public)/theory/+layout.svelte`:

```svelte
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Footer from '$lib/components/landing/Footer.svelte';

  let { children }: { children: Snippet } = $props();
</script>

{@render children()}

<Footer />
```

No chrome yet — Task 3 adds the backlink + prev/next.

- [ ] **Step 2: Create 9 route stubs**

Each route gets a minimal `+page.svelte` that proves the route resolves. Use this exact content for every stub:

For `src/routes/(public)/theory/+page.svelte`:

```svelte
<svelte:head>
  <title>What Temper is building toward — temper</title>
</svelte:head>

<h1>/theory entry — placeholder</h1>
```

For each sub-page stub (replace `<title>` and `<h1>` with the route's slug), e.g. `src/routes/(public)/theory/ontology/+page.svelte`:

```svelte
<svelte:head>
  <title>Ontology — temper</title>
</svelte:head>

<h1>/theory/ontology — placeholder</h1>
```

Repeat for the other 7 sub-routes. Titles: `Ontology`, `Manifold`, `Time`, `Deformation`, `Perspectives`, `Translation`, `Schema`, `Open questions`. Heading text just identifies which placeholder you're looking at.

- [ ] **Step 3: Type-check passes**

```bash
cd packages/temper-ui && bun run check
```

Expected: 0 errors, 0 warnings. (Tailwind/Svelte-check may emit a hint about the layout's `$props()` type if Svelte 5 isn't set up — should not happen in this repo since `/how-it-works` uses the same shape.)

- [ ] **Step 4: Visual verification**

With `bun run dev` running, load these URLs and confirm each renders:

- `http://localhost:5173/theory` — `/theory entry — placeholder` heading + outer Nav + Footer.
- `http://localhost:5173/theory/ontology` — `/theory/ontology — placeholder` + Footer.
- Spot-check 2 more sub-routes from the list above.

No console errors. Existing pages (`/`, `/how-it-works`) still render normally.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/routes/\(public\)/theory/
git commit -m "feat(ui): scaffold /theory tier layout + route stubs

Adds (public)/theory/+layout.svelte (rendering Footer) and 9 placeholder
+page.svelte files for the new theory tier. No content yet — subsequent
tasks translate the draft .md files at the repo root into each route."
```

---

## Task 2: Add `TheoryNav` component

**Files:**
- Create: `src/routes/(public)/theory/TheoryNav.svelte`

- [ ] **Step 1: Implement the component**

Write `src/routes/(public)/theory/TheoryNav.svelte`:

```svelte
<script lang="ts">
  type NavLink = { href: string; title: string };
  let { prev, next }: { prev?: NavLink; next?: NavLink } = $props();
</script>

<nav class="theory-nav">
  {#if prev}
    <a class="nav-link prev" href={prev.href}>
      <span class="nav-direction">← Previous</span>
      <span class="nav-title">{prev.title}</span>
    </a>
  {:else}
    <span class="nav-spacer"></span>
  {/if}

  {#if next}
    <a class="nav-link next" href={next.href}>
      <span class="nav-direction">Next →</span>
      <span class="nav-title">{next.title}</span>
    </a>
  {:else}
    <span class="nav-spacer"></span>
  {/if}
</nav>

<style>
  .theory-nav {
    max-width: 800px;
    margin: 0 auto;
    padding: 3rem 2.5rem 1rem;
    display: flex;
    justify-content: space-between;
    gap: 2rem;
    border-top: 1px solid var(--rule);
  }
  .nav-spacer { flex: 1; }
  .nav-link {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    text-decoration: none;
    color: var(--graphite);
    transition: color 0.2s;
  }
  .nav-link:hover { color: var(--temper-blue); }
  .nav-link.next { text-align: right; align-items: flex-end; }
  .nav-direction {
    font-family: var(--font-mono);
    font-size: 0.65rem;
    letter-spacing: 0.15em;
    text-transform: uppercase;
  }
  .nav-title {
    font-family: var(--font-serif);
    font-size: 1.05rem;
    color: var(--parchment);
  }
  @media (max-width: 640px) {
    .theory-nav { flex-direction: column; padding: 2rem 1.5rem 0.5rem; gap: 1.5rem; }
    .nav-link.next { text-align: left; align-items: flex-start; }
  }
</style>
```

The empty-spacer trick keeps prev and next visually anchored to opposite ends even when only one is present (first/last sub-page).

- [ ] **Step 2: Type-check passes**

```bash
cd packages/temper-ui && bun run check
```

Expected: 0 errors. The component has no consumers yet — this verifies it compiles standalone.

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/routes/\(public\)/theory/TheoryNav.svelte
git commit -m "feat(ui): add TheoryNav prev/next component

Co-located with the routes (not in \$lib) because it is tier-scoped —
nothing outside (public)/theory uses it."
```

---

## Task 3: Wire shared chrome into the tier layout

**Files:**
- Modify: `src/routes/(public)/theory/+layout.svelte`

- [ ] **Step 1: Replace the layout with the full chrome version**

Overwrite `src/routes/(public)/theory/+layout.svelte`:

```svelte
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { page } from '$app/state';
  import Footer from '$lib/components/landing/Footer.svelte';
  import TheoryNav from './TheoryNav.svelte';

  let { children }: { children: Snippet } = $props();

  type NavLink = { href: string; title: string };

  const ORDER: NavLink[] = [
    { href: '/theory',                title: 'What Temper is building toward' },
    { href: '/theory/ontology',       title: 'Ontology' },
    { href: '/theory/manifold',       title: 'The manifold' },
    { href: '/theory/time',           title: 'Time' },
    { href: '/theory/deformation',    title: 'Deformation' },
    { href: '/theory/perspectives',   title: 'Perspectives' },
    { href: '/theory/translation',    title: 'Translation' },
    { href: '/theory/schema',         title: 'Schema' },
    { href: '/theory/open-questions', title: 'Open questions' },
  ];

  // Derive current index from the route. SvelteKit route ids look like
  // "/(public)/theory" or "/(public)/theory/ontology"; strip the group.
  const currentHref = $derived.by(() => {
    const id = page.route.id ?? '';
    return id.replace('/(public)', '') || '/theory';
  });

  const currentIndex = $derived(ORDER.findIndex((link) => link.href === currentHref));
  const isEntry = $derived(currentHref === '/theory');
  const prev = $derived(currentIndex > 0 ? ORDER[currentIndex - 1] : undefined);
  const next = $derived(
    currentIndex >= 0 && currentIndex < ORDER.length - 1 ? ORDER[currentIndex + 1] : undefined
  );
</script>

{#if !isEntry}
  <div class="theory-backlink">
    <a href="/theory">← Theory</a>
  </div>
{/if}

{@render children()}

{#if !isEntry}
  <TheoryNav {prev} {next} />
{/if}

<Footer />

<style>
  .theory-backlink {
    max-width: 800px;
    margin: 0 auto;
    padding: 2.5rem 2.5rem 0;
  }
  .theory-backlink a {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    letter-spacing: 0.12em;
    color: var(--graphite);
    text-decoration: none;
    text-transform: uppercase;
    transition: color 0.2s;
  }
  .theory-backlink a:hover { color: var(--temper-blue); }

  /* Shared prose styles for theory pages.
     Pages wrap their main prose region in `<div class="theory-page">` so
     these globals don't bleed onto unrelated routes. */
  :global(.theory-page ul),
  :global(.theory-page ol) {
    font-family: var(--font-serif);
    font-size: 1rem;
    color: var(--chalk);
    line-height: 1.8;
    margin: 0 0 1rem 1.5rem;
    padding: 0;
  }
  :global(.theory-page li) { margin-bottom: 0.4rem; }
  :global(.theory-page li strong) { color: var(--parchment); font-weight: 400; }

  :global(.theory-page blockquote) {
    margin: 1.25rem 0;
    padding: 0.5rem 0 0.5rem 1.25rem;
    border-left: 2px solid var(--temper-blue-border);
    font-family: var(--font-serif);
    color: var(--parchment);
    font-style: italic;
  }
  :global(.theory-page blockquote strong) { font-style: normal; color: var(--parchment); }

  :global(.theory-page code) {
    font-family: var(--font-mono);
    font-size: 0.85rem;
    color: var(--temper-blue);
  }

  :global(.theory-page a) {
    color: var(--temper-blue);
    text-decoration: none;
    border-bottom: 1px solid var(--temper-blue-border-dim);
    transition: border-color 0.2s;
  }
  :global(.theory-page a:hover) { border-bottom-color: var(--temper-blue); }

  :global(.theory-page h3) {
    font-family: var(--font-serif);
    font-size: 1.15rem;
    font-weight: 400;
    color: var(--parchment);
    margin: 1.5rem 0 0.75rem;
  }

  :global(.theory-page table.theory-table) {
    width: 100%;
    border-collapse: collapse;
    margin: 1.5rem 0;
    font-family: var(--font-serif);
    font-size: 0.95rem;
    color: var(--chalk);
  }
  :global(.theory-page table.theory-table th),
  :global(.theory-page table.theory-table td) {
    text-align: left;
    padding: 0.6rem 0.75rem;
    border-bottom: 1px solid var(--rule);
    vertical-align: top;
  }
  :global(.theory-page table.theory-table th) {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    font-weight: 400;
    color: var(--temper-blue);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border-bottom-color: var(--temper-blue-border-dim);
  }
</style>
```

Notes for the implementer:

- The `page` import from `$app/state` is the Svelte 5 idiomatic form (replacing `$app/stores`). If `bun run check` complains, fall back to `import { page } from '$app/stores'` and use `$page.route.id` (with the leading `$`).
- `$derived` and `$derived.by` are Svelte 5 runes. The existing codebase already uses runes (`$props()` in `(public)/+layout.svelte`), so they should resolve.
- `:global()` selectors here apply to every page (Svelte does NOT scope `:global()` rules to the layout). They are gated by `.theory-page` so they only match content inside that wrapper. **All theory pages must wrap their main prose region in `<div class="theory-page">...</div>`.**

- [ ] **Step 2: Type-check passes**

```bash
cd packages/temper-ui && bun run check
```

Expected: 0 errors. If you get `Cannot find module '$app/state'`, swap to `$app/stores` per the note above and re-run.

- [ ] **Step 3: Visual verification**

With `bun run dev` running:

- `http://localhost:5173/theory` — placeholder content, NO `← Theory` backlink at top, NO prev/next at bottom (entry is the hub).
- `http://localhost:5173/theory/ontology` — `← Theory` at top, TheoryNav at bottom with `Previous: What Temper is building toward` on the left and `Next: The manifold` on the right.
- `http://localhost:5173/theory/open-questions` — `← Theory` at top, TheoryNav at bottom with `Previous: Schema` on the left and the right side empty (no next).
- Click `← Theory` from a sub-page → lands on `/theory`.
- Click Next on `/theory/ontology` → lands on `/theory/manifold`.
- Click Previous on `/theory/manifold` → lands on `/theory/ontology`.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/routes/\(public\)/theory/+layout.svelte
git commit -m "feat(ui): wire backlink + prev/next chrome into /theory layout

Layout derives prev/next from a static ORDER array based on the current
route. Entry page (/theory) suppresses chrome since it is the hub.
Shared prose styles live in this file scoped under .theory-page so they
don't bleed onto unrelated routes."
```

---

## Task 4: Translate `/theory` entry page

**This is the worked example for the rest of the translations.** Tasks 5–12 reference its patterns.

**Files:**
- Modify: `src/routes/(public)/theory/+page.svelte`

**Source:** `theory-entry-draft.md` (at the repo root). Read the file's "Page copy (draft)" section — the prose between the `---` markers — and translate it. Do not include the file's "Editorial notes" or "Things considered and rejected" sections.

- [ ] **Step 1: Translate the draft into the page**

Overwrite `src/routes/(public)/theory/+page.svelte`:

```svelte
<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>What Temper is building toward — temper</title>
  <meta
    name="description"
    content="Attention is the teleological anchor. Temper is built as a commitment to respecting it. An introduction to the theory tier."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory</div>
  <h1 class="t-hero-title">What Temper is building <em>toward</em></h1>
  <p class="tagline t-tagline">
    Attention is our most precious resource. I am building Temper as a
    commitment to respecting it.
  </p>
</section>

<div class="theory-page">

<Section label="The anchor">
  <h2>Attention is the <em>teleological anchor</em></h2>
  <p>
    Attention is how I experience myself, time, and the world. It is how
    I am present to my own life, how I direct my agency, how I make any
    of the choices I think of as mine. It is the medium of intention —
    what I do with my attention is what I do at all. When I direct it
    well, I am the agent of my own work; when it is fractured or
    hijacked or spent without my consent, I lose ground in the most
    fundamental sense.
  </p>
  <p>
    This is true of every perspective capable of attention, not just
    mine. When I demand attention from a colleague, a friend, or an
    agent working on my behalf, I am asking for their capacity to be
    present and to act. Every interrupt is a demand on that capacity.
    Every poorly-justified ping, every system that requires
    re-discovering what it could have made present, every interface
    that demands construction-from-scratch when reasonable defaults
    exist, is a demand on something irreplaceable. The cost compounds,
    because every low-leverage demand on attention is attention not
    available for what actually matters — and attention, unlike most
    resources, does not regenerate. When it is spent, it is spent.
  </p>
  <p>
    I believe this gives the design of information systems an ethical
    character it usually lacks. If attention is the medium of agency,
    then a system that wastes attention isn't just inefficient — it is
    treating something precious as fungible. Efficiency is not, in this
    frame, a productivity virtue. It is an obligation —
    <em>efficiency-as-ethic</em> — that follows from taking attention
    seriously as the thing it actually is.
  </p>
  <p>
    Temper is built around what falls out of that obligation. Building
    it in this frame of reference, I am committed to a few key
    principles.
  </p>
</Section>

<Section label="Commitment 1">
  <h2>Common queries should not require <em>fresh attention</em> each time</h2>
  <p>
    Most people in a given role ask roughly the same kinds of questions,
    with the same kinds of intent behind them. Pre-paying those queries —
    making them cheap by default — is not paternalism. It is the system
    doing the work that does not need to be done freshly each time, so
    attention can land on what is actually new.
  </p>
</Section>

<Section label="Commitment 2">
  <h2>Perspective-differences are real and should be made <em>visible</em></h2>
  <p>
    Different people working on the same thing produce genuinely
    different information from the same data, because they engage it
    from different positions with different concerns. A system that
    pretends otherwise — that produces a single canonical view —
    forces attention to be spent re-discovering those differences in
    every conversation. Surfacing them is how a system spends attention
    once and saves it forever.
  </p>
</Section>

<Section label="Commitment 3">
  <h2>Information past its time should <em>fade</em>, not crowd the present</h2>
  <p>
    Information that is no longer relevant should grow harder to find;
    information that is no longer true should not surface as if it
    were; information that needs to be preserved for audit should not
    pollute default retrieval. None of this is unusual to want. What
    is unusual is taking it seriously enough to design for, rather
    than letting deprecation tags pile up while everything stays
    equally findable.
  </p>
</Section>

<Section label="Commitment 4">
  <h2>Where the system has been wrong, future engagement should <em>know</em></h2>
  <p>
    Confidence and accuracy are not the same thing. Without a trace of
    where errors have lived before, attention cannot land on what
    needs scrutiny — and a system that hides its own past errors is
    asking attention to do work the system should have done.
  </p>
</Section>

<Section label="One commitment up front">
  <h2>The system does not store <em>knowledge</em></h2>
  <p>
    These are not features I want to ship. They are commitments I want
    to keep. There is a separate semantic model where the architectural
    detail lives — what the manifold is, what fields are, how
    forgetting works geometrically, how perspectives are
    characterized. The pages under <code>/theory</code> introduce that
    model.
  </p>
  <p>
    One commitment in the model runs through everything that follows,
    and deserves naming up front: the system stores data and traces of
    past intentional acts — recorded questions, notes, decisions,
    which themselves become further data. It does not store
    <em>knowledge</em>. Knowledge is the relationship between a
    perspective and the information that perspective produces through
    engagement with data.
  </p>
  <p>
    The label "knowledge base" is a misnomer in light of this.
    Knowledge is always potential, never actual, until activated by a
    perspective. The system's job is never to <em>be right about what
    something means</em> — meaning is not in the system. Its job is to
    faithfully represent data, faithfully record intentions, and
    faithfully compute projections such that perspectives engaging
    with those projections are well-equipped to produce knowledge.
  </p>
</Section>

<Section label="The model">
  <h2>The shape of the <em>model</em></h2>
  <p>
    The pages here introduce the model in the order the source document
    does. Each is short; each can be read alone; the sequence is the
    most coherent path through.
  </p>
  <ul>
    <li>
      <strong><a href="/theory/ontology">Ontology</a></strong> — Data,
      intention, information, knowledge. The stratified layers.
    </li>
    <li>
      <strong><a href="/theory/manifold">Manifold</a></strong> —
      Positions, fields, streams. The geometry.
    </li>
    <li>
      <strong><a href="/theory/time">Time</a></strong> — Time as a
      primary axis. Events-as-primary. Why this is a substrate
      commitment.
    </li>
    <li>
      <strong><a href="/theory/deformation">Deformation</a></strong> —
      Forming and forgetting. Strong vs. weak. Scarification.
      Self-cohesion and relaxation.
    </li>
    <li>
      <strong><a href="/theory/perspectives">Perspectives</a></strong>
      — Trajectories, not points. Role-perspective vs. individual.
      Access vs. expertise.
    </li>
    <li>
      <strong><a href="/theory/translation">Translation</a></strong> —
      Why translation is irreducible. Bridges. Knowledge as
      relationship.
    </li>
  </ul>
  <p>Two reference surfaces sit alongside:</p>
  <ul>
    <li>
      <strong><a href="/theory/schema">Schema</a></strong> — The
      structural codification: entity types, event structure, topic
      taxonomy, resolved stances. Work in progress.
    </li>
    <li>
      <strong><a href="/theory/open-questions">Open questions</a></strong>
      — What is not yet settled. Updates over time as items resolve.
    </li>
  </ul>
  <p>The model is provisional. It captures a mental picture clearly enough to be argued with.</p>
  <p>
    One symmetry is worth flagging before the model proper. The
    perspective-side has roughly the same shape as the data-side:
    discrete deformations, continuous trajectories, characteristic
    decay rates, prior-and-likelihood structure. Resources and
    perspectives are both on the manifold; both have positions,
    trajectories, and decay; both can be deformed strongly or weakly;
    both have spatial profiles for their effects. This is some
    evidence the primitives are well-chosen rather than ad-hoc — a
    model that requires fewer primitives to describe more phenomena
    is more likely to be sound.
  </p>
</Section>

<Section label="Closing">
  <p>
    I am not trying to win an academic argument or start a movement. I
    am writing this so that when I am six months into implementation
    and tempted to take a shortcut that costs the user some attention
    they will not get back, I have something to read that reminds me
    why I started. I am publishing it because the commitment is more
    honest if it is shared, and because anyone considering this tool
    deserves to know what its author thinks the tool is <em>for</em>
    before they decide whether to spend their attention on it.
  </p>
</Section>

</div>

<style>
  .hero {
    min-height: 50vh;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    text-align: center;
    padding: 5rem 2.5rem 2rem;
  }
  .hero-label { margin-bottom: 1.5rem; }
  .hero h1 { margin-bottom: 1.5rem; }
  .tagline { max-width: 36em; }
</style>
```

Patterns to notice (you will reuse them for every subsequent page):

1. **Hero block.** `<section class="hero">` with `.hero-label.t-label`, `<h1 class="t-hero-title">`, `<p class="tagline t-tagline">`. The h1 uses `<em>` for the accent word — italic + temper-blue.
2. **`<div class="theory-page">` wrapper** around the main content, enabling the shared prose styles from the layout.
3. **`<Section label="...">` blocks.** Each major section of the draft becomes a `<Section>`. The `label` prop is a short mono-uppercase descriptor; the inner `<h2>` is the prose heading. Choose labels that fit the section's role (`The anchor`, `Commitment 1`, etc.). Not all source headings are direct labels — invent terse ones where the source heading is too long.
4. **`<em>` in h2s** highlights the key noun/verb in temper-blue italics, matching `/how-it-works` style.
5. **`<svelte:head>`** sets `<title>` and `<meta description>`.
6. **Scoped `<style>`** at the bottom handles only the hero layout. Everything else inherits from the layout's `.theory-page` global rules and the `Section` component's own globals.
7. **No `<Footer />`** in the page — the layout renders it.

- [ ] **Step 2: Type-check passes**

```bash
cd packages/temper-ui && bun run check
```

Expected: 0 errors.

- [ ] **Step 3: Visual verification**

Load `http://localhost:5173/theory`:

- Hero block reads correctly, with "toward" italicized in temper-blue.
- Each `<Section>` has its mono label in temper-blue, parchment h2, serif body.
- The sub-page index renders as a bulleted list with bold link labels followed by em-dash descriptions. Each link works (click through to confirm — they land on the corresponding stub).
- The symmetry paragraph appears after the index, before the closing.
- The closing paragraph reads correctly.
- No `← Theory` backlink at top, no prev/next at bottom.
- Footer renders at the very bottom.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/routes/\(public\)/theory/+page.svelte
git commit -m "feat(ui): translate /theory entry page from theory-entry-draft.md"
```

---

## Task 5: Translate `/theory/ontology`

**Files:**
- Modify: `src/routes/(public)/theory/ontology/+page.svelte`

**Source:** `theory-ontology-draft.md`

**Structure notes for this page:**

- One short page (~500 words), four ontological layers + the misnomer commitment + closing line.
- **Hero:** `Ontology — temper` (title). Label `/theory/ontology`. H1 `Ontology: data, intention, information, <em>knowledge</em>`. Tagline: the first sentence ("The model has four ontological layers...").
- **Sections to create:**
  1. `<Section label="Data">` — h2 `Data`, one paragraph.
  2. `<Section label="Intention">` — h2 `Intention`, one paragraph.
  3. `<Section label="Information">` — h2 `Information`, one paragraph.
  4. `<Section label="Knowledge">` — h2 `Knowledge`, the two-paragraph treatment ending with the formula. **Render the formula in a `<blockquote>`** — the draft uses markdown `>` syntax: `> **K = experience(P, I)** where **I = project(Q, F | P)**`. Translate to `<blockquote><strong>K = experience(P, I)</strong> where <strong>I = project(Q, F | P)</strong></blockquote>`. Follow with the paragraph that begins "A perspective P (itself a function..." that explains the formula.
  5. `<Section label="Misnomer">` — h2 `Knowledge bases are <em>misnomers</em>`. Three paragraphs from the "Knowledge bases are misnomers" section. The opening sentence — "A consequence worth stating up front: <strong>knowledge bases are misnomers.</strong>" — keeps the bold on the claim. Then "The system's job..." paragraph. Then the closing line "None of the pages that follow can claim the system <em>knows</em>. They describe the conditions under which knowledge is produced."
- **No prose-level cross-links** in this draft. Just the closing line.

- [ ] **Step 1: Translate** following the Task 4 worked example. Use the same `<script>`, hero `<style>`, `.theory-page` wrapper pattern.
- [ ] **Step 2:** `cd packages/temper-ui && bun run check` — expect 0 errors.
- [ ] **Step 3:** Visual verify at `http://localhost:5173/theory/ontology`. Confirm backlink + prev/next render. Formula renders as a styled blockquote with bold variables. Prev: `What Temper is building toward`. Next: `The manifold`.
- [ ] **Step 4:** Commit:
  ```bash
  git add packages/temper-ui/src/routes/\(public\)/theory/ontology/+page.svelte
  git commit -m "feat(ui): translate /theory/ontology from theory-ontology-draft.md"
  ```

---

## Task 6: Translate `/theory/manifold`

**Files:**
- Modify: `src/routes/(public)/theory/manifold/+page.svelte`

**Source:** `theory-manifold-draft.md`

**Structure notes:**

- ~700 words. The geometric vocabulary page.
- **Hero:** title `The manifold — temper`. Label `/theory/manifold`. H1 `The <em>manifold</em>`. Tagline: "The model's geometric vocabulary for <em>aboutness</em>."
- **Sections:**
  1. `<Section label="Positions">` (or `<Section label="The manifold">` if you prefer to mirror the source heading) — h2 `Positions on the manifold` (rename slightly to disambiguate from the page title), two paragraphs from the "The manifold" section.
  2. `<Section label="Streams">` — h2 `Streams and <em>particles</em>`, three paragraphs. The third paragraph has a cross-link `<a href="/theory/schema">schema</a>`.
  3. `<Section label="Fields">` — h2 `Fields`, intro paragraph + bulleted list (4 items: Spatial profile, Weight, Temporal character, Characteristic decay distance) + closing paragraph. Each bullet starts with `<strong>{name}</strong> — {description}`.
  4. `<Section label="Coupling">` — h2 `Bidirectional <em>coupling</em>`, three paragraphs. The third has a cross-link to `/theory/deformation`.
  5. `<Section label="Projection">` — h2 `Projection`, two paragraphs.
  6. `<Section label="Context">` — h2 `Context is a <em>verb-state</em>`, three paragraphs.
- **Inline emphasis:** The draft uses `*aboutness*` (italics, render `<em>aboutness</em>`), `*projection*`, `*witnessed*`, etc. Preserve every italic and bold.

- [ ] **Step 1:** Translate. Same hero/script/style pattern as Task 4.
- [ ] **Step 2:** `bun run check` — expect 0 errors.
- [ ] **Step 3:** Visual at `/theory/manifold`. Prev: `Ontology`. Next: `Time`. Bulleted Fields list renders with bold leading terms. Cross-links to `/theory/schema` and `/theory/deformation` work.
- [ ] **Step 4:** Commit `feat(ui): translate /theory/manifold from theory-manifold-draft.md`.

---

## Task 7: Translate `/theory/time`

**Files:**
- Modify: `src/routes/(public)/theory/time/+page.svelte`

**Source:** `theory-time-draft.md`

**Structure notes:**

- ~400 words. The shortest model page — appropriately short, do not pad.
- **Hero:** title `Time — temper`. Label `/theory/time`. H1 `<em>Time</em>`. Tagline: "Time is not an afterthought; it is co-equal with position."
- **Sections:**
  1. `<Section label="Temporal extension">` — h2 `Every element of the model is <em>temporally extended</em>`. Then a `<ul>` with 6 bullets (Field profile drifts, Field weight rises/falls, Stream constitutively temporal, Projection has temporal lens, Resources have temporal validity, Perspectives as trajectories). The last bullet contains a cross-link `(<a href="/theory/perspectives">perspectives</a> returns to this)`.
  2. `<Section label="Events as substrate">` — h2 `Why this forces <em>events as the substrate</em>`, three paragraphs. The middle paragraph contains the bolded claim: `<strong>Event-sourcing is not one substrate option among many; it is the only substrate consistent with the model.</strong>` The third paragraph contains a cross-link `<a href="/theory/open-questions#schema">open</a>` (note the fragment).
- **No closing section** beyond the events-as-substrate finish.

- [ ] **Step 1:** Translate.
- [ ] **Step 2:** `bun run check`.
- [ ] **Step 3:** Visual at `/theory/time`. Prev: `The manifold`. Next: `Deformation`. The cross-link to `/theory/open-questions#schema` works — clicking it lands on the open-questions page (currently the stub; in Task 12 it gains the `#schema` anchor).
- [ ] **Step 4:** Commit `feat(ui): translate /theory/time from theory-time-draft.md`.

---

## Task 8: Translate `/theory/deformation`

**Files:**
- Modify: `src/routes/(public)/theory/deformation/+page.svelte`

**Source:** `theory-deformation-draft.md`

**Structure notes:**

- ~750 words.
- **Hero:** title `Deformation — temper`. Label `/theory/deformation`. H1 `Deformation: forming and <em>forgetting</em>`. Tagline: "Deformations are the events that shape the manifold's geometry."
- **Sections:**
  1. `<Section label="Forming">` — h2 `Forming deformations`, intro paragraph + two paragraphs labeled with bolded leading phrase: `<strong>Strong / authoritative deformations</strong> are...` and `<strong>Weak / cumulative deformations</strong> are...`. Then a third paragraph naming the "recording threshold" in bold.
  2. `<Section label="By topic-class">` — h2 `Deformation by <em>topic-class</em>`, intro + three paragraphs each leading with a bolded phrase (Emission-adds-geometry, Observation-reinforces-geometry, Correction-scars-geometry). Closing paragraph cross-links `<a href="/theory/schema">schema's topic taxonomy</a>`.
  3. `<Section label="Forgetting">` — h2 `Forgetting <em>mechanics</em>`, intro paragraph + three paragraphs each leading with a bolded mechanic (Decay, Deformation, Folding). Each ends with a "answers..." italic phrase: `Decay answers <em>no longer relevant</em>.` etc.
  4. `<Section label="Scarification">` — h2 `Scarification`, three paragraphs. The "scar is not the corrected information" claim stays. Closing paragraph cross-links `<a href="/theory/time">time</a>`.
  5. `<Section label="Self-cohesion">` — h2 `Self-cohesion and <em>background relaxation</em>`, intro + two paragraphs each leading with bolded primitive (Self-cohesion, Background relaxation), then a closing paragraph distinguishing the two questions, ending with a cross-link `<a href="/theory/open-questions#model">an open question</a>`.

- [ ] **Step 1:** Translate.
- [ ] **Step 2:** `bun run check`.
- [ ] **Step 3:** Visual at `/theory/deformation`. Prev: `Time`. Next: `Perspectives`. Bolded leading phrases render correctly inline (not as headings). Cross-links resolve.
- [ ] **Step 4:** Commit `feat(ui): translate /theory/deformation from theory-deformation-draft.md`.

---

## Task 9: Translate `/theory/perspectives`

**Files:**
- Modify: `src/routes/(public)/theory/perspectives/+page.svelte`

**Source:** `theory-perspectives-draft.md`

**Structure notes:**

- ~800 words. The longest model page.
- **Hero:** title `Perspectives — temper`. Label `/theory/perspectives`. H1 `<em>Perspectives</em>`. Tagline: "The model's account of <em>who is asking</em> mirrors its account of what is asked about."
- **Sections (8):**
  1. `<Section label="On the manifold">` — h2 `Perspectives on the <em>manifold</em>`, two paragraphs.
  2. `<Section label="Trajectories">` — h2 `Perspectives are trajectories, not <em>points</em>`, two paragraphs.
  3. `<Section label="Characterization">` — h2 `Perspective <em>characterization</em>`, intro paragraph + `<ul>` with 4 bullets (Identity, Reliability profile, Characteristic intention-vectors, Domain-specificity), then closing paragraph about domain-specificity.
  4. `<Section label="Role vs individual">` — h2 `Role-perspective and <em>individual-perspective</em>`, six paragraphs. Bolded leading phrases: `<strong>role-perspective</strong>`, `<strong>individual-perspective</strong>`. Bayesian framing in third paragraph: explicit `<em>prior</em>`, `<em>likelihoods</em>`, `<em>posterior</em>` italics. Cross-link to `<a href="/theory/deformation">deformation</a>` in the role-changes-as-strong-deformations paragraph.
  5. `<Section label="Aggregates">` — h2 `Aggregate-perspectives`, two paragraphs. Bolded `<strong>on-behalf-of</strong>`.
  6. `<Section label="Personas">` — h2 `Personas as <em>field-sets</em>`, one paragraph.
  7. `<Section label="Visibility">` — h2 `Visibility: access and <em>expertise</em>`, intro paragraph + four paragraphs. Leading bolded `<strong>Access</strong>` and `<strong>Expertise</strong>`.
  8. `<Section label="Observer-relativity">` — h2 `Observer-relativity (weak version)`, three paragraphs. **Last sentence of the second paragraph is bolded**: `<strong>Shared understanding is therefore an emergent property of convergent projection histories, not something guaranteed by shared substrate.</strong>` The third paragraph cross-links `<a href="/theory/translation">the translation page</a>`.

- [ ] **Step 1:** Translate.
- [ ] **Step 2:** `bun run check`.
- [ ] **Step 3:** Visual at `/theory/perspectives`. Prev: `Deformation`. Next: `Translation`. All bolded leading phrases inline. Page is long — scroll-test that section dividers (the `<Section>` component's `<hr>`) render at correct spacing.
- [ ] **Step 4:** Commit `feat(ui): translate /theory/perspectives from theory-perspectives-draft.md`.

---

## Task 10: Translate `/theory/translation`

**Files:**
- Modify: `src/routes/(public)/theory/translation/+page.svelte`

**Source:** `theory-translation-draft.md`

**Structure notes:**

- ~600 words. Closes the theory model sequence.
- **Hero:** title `Translation — temper`. Label `/theory/translation`. H1 `<em>Translation</em>`. Tagline: "Translation between perspectives is not a resolution problem; it is a first-class activity the substrate has to support." (Paraphrase tightly — the source's opening paragraph is long; the tagline should be one sentence.)
- **Sections:**
  1. `<Section label="Composition">` — h2 `Intention-vectors <em>compose and conflict</em>`, one paragraph.
  2. `<Section label="The problem">` — h2 `The translation <em>problem</em>`, five paragraphs. Includes the **bolded** claim `<strong>make perspective-differences visible without claiming to have resolved them</strong>` in the third paragraph.
  3. `<Section label="Bridges">` — h2 `Translation events, bridges, and <em>translation-scars</em>`, one paragraph. Bold `<strong>bridging structure</strong>`.
  4. `<Section label="Knowledge as relationship">` — h2 `Knowledge as <em>relationship</em>`, two paragraphs. Cross-link to `<a href="/theory/ontology">data / information / knowledge stratification</a>`.

- [ ] **Step 1:** Translate.
- [ ] **Step 2:** `bun run check`.
- [ ] **Step 3:** Visual at `/theory/translation`. Prev: `Perspectives`. Next: `Schema`.
- [ ] **Step 4:** Commit `feat(ui): translate /theory/translation from theory-translation-draft.md`.

---

## Task 11: Translate `/theory/schema` (table-heavy reference page)

**Files:**
- Modify: `src/routes/(public)/theory/schema/+page.svelte`

**Source:** `theory-schema-draft.md`

**Special considerations:**

- This is the densest page. It is reference-style, not tutorial.
- **WIP framing as opening paragraph**, NOT as a banner. The first paragraph IS the WIP note.
- **Eight tables** total. Each gets `<table class="theory-table">` — the `.theory-table` class is styled in the layout's `:global()` block.
- **One bulleted list** at the end ("Resolved stances") with ~25 items.

**Structure notes:**

- **Hero:** title `Schema — temper`. Label `/theory/schema`. H1 `<em>Schema</em>`. Tagline: "The structural codification of the model. A snapshot, not a final specification."
- **Sections:**
  1. `<Section label="What this is">` — no h2 (or h2 `A working reference`), opening two paragraphs (the WIP framing). Cross-link `<a href="/theory/open-questions#schema">/theory/open-questions#schema</a>`.
  2. `<Section label="Entity types">` — h2 `Entity types`, then `<table class="theory-table">` with headers `Type / Description / Notes` and 6 rows (Discrete entity, Aggregate-perspective, Resource, Resource-aggregate, Role-class, Event). Cell content from draft.
  3. `<Section label="Event structure">` — h2 `Event structure (universal)`, intro sentence, then two `<ul>` lists with bolded leading phrases `<strong>Core (always present):</strong>` and `<strong>Notably not in core:</strong>` containing the bullet items.
  4. `<Section label="Topic taxonomy">` — h2 `Topic taxonomy`, intro line ("Categories below are illustrative, not closed."), then `<table class="theory-table">` with headers `Topic Class / Examples / Notes` and 8 rows.
  5. `<Section label="Roles within events">` — h2 `Roles within events`, `<table>` with 3 rows (Emitter / On-behalf-of / Subject), plus a clarifying paragraph after about observation-topic events.
  6. `<Section label="Derived structures">` — h2 `Derived structures`, intro line, `<table>` with 9 rows.
  7. `<Section label="Mechanics">` — h2 `Mechanics`, `<table>` with 13 rows (Strong deformation through Projection).
  8. `<Section label="Stratification">` — h2 `Stratification`, `<table>` with 3 rows (Substrate / Information / Knowledge).
  9. `<Section label="Accountability">` — h2 `Accountability vectors`, intro line, `<table>` with 4 rows.
  10. `<Section label="Chain links">` — h2 `Chain link kinds (within on-behalf-of chains)`, `<table>` with 4 rows.
  11. `<Section label="Resolved stances">` — h2 `Resolved <em>stances</em>`, intro line, then a long `<ul>` of ~25 single-line items (the schema's load-bearing commitments).
  12. `<Section label="What is moving">` — h2 `What's still <em>moving</em>`, three paragraphs (the "Intentionally open / Pending opinionated stance" framing) including a `<ul>` with the two bullets `<strong>Intentionally open</strong>` and `<strong>Pending opinionated stance</strong>` each followed by a one-line example summary. Two `<a href="/theory/open-questions#schema">` cross-links.

**Table render pattern (worked example for the entity types table):**

```svelte
<Section label="Entity types">
  <h2>Entity types</h2>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Type</th>
        <th>Description</th>
        <th>Notes</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Discrete entity</td>
        <td>Perspective-bearing, non-aggregate. Has capacity-for-observation and a position in perspective-space. Subtypes: human, agent, deterministic-system.</td>
        <td>Plays the Emitter role within events.</td>
      </tr>
      <!-- 5 more rows -->
    </tbody>
  </table>
</Section>
```

- [ ] **Step 1:** Translate. Allow more time than the other pages — there are 8 tables to transcribe carefully from the draft.
- [ ] **Step 2:** `bun run check`.
- [ ] **Step 3:** Visual at `/theory/schema`. Prev: `Translation`. Next: `Open questions`. **Every table** is legible — mono blue uppercase header row, serif body cells, hairline rule between rows. Test on a narrow window (640px) — tables can overflow; if they look broken, add `overflow-x: auto` to a wrapper around each `<table>` (you can do this with a `<div class="table-wrap">` and a scoped style in the schema page). Cross-links to `/theory/open-questions#schema` resolve.
- [ ] **Step 4:** Commit `feat(ui): translate /theory/schema from theory-schema-draft.md`.

---

## Task 12: Translate `/theory/open-questions` (with anchors)

**Files:**
- Modify: `src/routes/(public)/theory/open-questions/+page.svelte`

**Source:** `theory-open-questions-draft.md`

**Special consideration:** The `#model` and `#schema` anchors are load-bearing — cross-links from `/theory/time` and `/theory/schema` resolve to them. Use `<section id="model">` and `<section id="schema">` so the URL fragments work.

**Structure notes:**

- ~700 words. Two top-level sections under stable IDs.
- **Hero:** title `Open questions — temper`. Label `/theory/open-questions`. H1 `Open <em>questions</em>`. Tagline: "The questions the model and its schema have deliberately not resolved."
- **Sections:**
  1. `<Section label="About this page">` — no h2 (or h2 `A working list`), the two-paragraph intro about the page's purpose and the mirrored-items convention.
  2. **`<section id="model">`** wrapping a `<Section label="Model">` — h2 `Open questions about the <em>model</em> itself`, intro paragraph + `<ul>` of 7 items, each with a `<strong>` leading question (e.g., `<strong>Is <em>field</em> too undifferentiated?</strong>`) followed by an explanatory sentence or two.
  3. **`<section id="schema">`** wrapping a `<Section label="Schema">` — h2 `Schema-level <em>unsettled material</em>`, intro paragraph. Then `<h3>Intentionally open (downstream design)</h3>` + `<ul>` of ~11 items. Then `<h3>Pending opinionated stance</h3>` + `<ul>` of ~10 items, each ending with `<em>(mirrors #model.)</em>` where applicable. Use `<a href="#model">#model</a>` for the mirrored references.
  4. `<Section label="A note">` — h2 `A note on this page`, three paragraphs.

**Anchor render pattern:**

```svelte
<section id="model">
  <Section label="Model">
    <h2>Open questions about the <em>model</em> itself</h2>
    <!-- intro + list -->
  </Section>
</section>
```

The outer `<section id="model">` provides the fragment target. The inner `<Section>` provides the visual chrome. The nesting is fine — `Section.svelte` is just a div wrapper.

- [ ] **Step 1:** Translate.
- [ ] **Step 2:** `bun run check`.
- [ ] **Step 3:** Visual at `/theory/open-questions`. Prev: `Schema`. Next: none (the right side of TheoryNav is empty). **Test the anchors directly**: load `/theory/open-questions#model` and `/theory/open-questions#schema` — the page should scroll to the corresponding section. From `/theory/schema`, click the WIP-framing cross-link to `/theory/open-questions#schema` and confirm it lands at the schema section.
- [ ] **Step 4:** Commit `feat(ui): translate /theory/open-questions from theory-open-questions-draft.md`.

---

## Task 13: Repo-root cleanup, IA proposal relocation, end-to-end verification

**Files:**
- Delete: 9 `theory-*-draft.md` files at the repo root
- Move: `docs-ia-proposal.md` → `docs/theory-ia-proposal.md`

- [ ] **Step 1: Move the IA proposal**

```bash
git mv docs-ia-proposal.md docs/theory-ia-proposal.md
```

- [ ] **Step 2: Delete the 9 draft files**

```bash
git rm theory-entry-draft.md \
       theory-ontology-draft.md \
       theory-manifold-draft.md \
       theory-time-draft.md \
       theory-deformation-draft.md \
       theory-perspectives-draft.md \
       theory-translation-draft.md \
       theory-schema-draft.md \
       theory-open-questions-draft.md
```

- [ ] **Step 3: End-to-end verification**

Type-check the whole UI package and Rust workspace (since `cargo make check` is the project's umbrella lint):

```bash
cd packages/temper-ui && bun run check
```

Expected: 0 errors.

Then, with `bun run dev` running, walk the canonical reading order in the browser:

1. Open `/theory` — verify hero, all 6 model links and 2 reference-surface links, symmetry paragraph, closing.
2. Click `Ontology` link → on `/theory/ontology`, verify backlink + prev (entry) + next (manifold).
3. Click Next repeatedly through ontology → manifold → time → deformation → perspectives → translation → schema → open-questions.
4. On open-questions, verify Next is absent.
5. Click Previous all the way back to `/theory/ontology`.
6. From `/theory/ontology` click `← Theory` — lands on entry, no chrome.
7. Test deep anchor: load `/theory/open-questions#schema` directly, page scrolls to schema section.
8. Test inline cross-link: from `/theory/schema` (WIP paragraph), click the open-questions link → lands on `/theory/open-questions#schema`.

Then confirm existing pages still work:

- `/` — landing renders unchanged.
- `/how-it-works` — renders unchanged.
- `/docs` — renders unchanged.
- `/agents`, `/builders` — render unchanged.

- [ ] **Step 4: Commit the cleanup**

```bash
git add -A
git status   # confirm: 9 deletions at root, 1 rename (docs-ia-proposal.md → docs/theory-ia-proposal.md)
git commit -m "chore: remove theory drafts and move IA proposal into docs/

Draft files theory-*-draft.md served their purpose as source material for
the /theory tier translation. The +page.svelte files in
packages/temper-ui/src/routes/(public)/theory/ are canonical going
forward; git history preserves the drafts for audit.

docs-ia-proposal.md moves to docs/theory-ia-proposal.md — it is the
finished IA design artifact for the migration and deserves a permanent
home in docs/."
```

- [ ] **Step 5: Final sanity check**

```bash
git log --oneline -15
ls theory-*.md 2>/dev/null   # expected: no output
ls docs/theory-ia-proposal.md   # expected: exists
```

Expected log shape (most recent first): cleanup commit, 9 translation commits, chrome wiring commit, TheoryNav commit, scaffold commit. The 13 commits tell a coherent story when reviewed.

---

## Things that can go wrong (read before starting)

- **Svelte 5 `$app/state` vs `$app/stores`.** This codebase is Svelte 5 + SvelteKit 2 and *should* support `import { page } from '$app/state'`. If `bun run check` errors there, fall back to the legacy form `import { page } from '$app/stores'` and replace `page.route.id` with `$page.route.id` (the dollar prefix unwraps the store). Note: in the layout, the `$derived` runes work with either form — `page` is reactive in both.
- **`(public)` group in route id.** SvelteKit route ids include the layout group: `/(public)/theory/ontology`. The layout strips `(public)` via `.replace('/(public)', '')` to get the URL-shaped id. Verify the regex works during Task 3 visual check; if a sub-page has no prev/next, the matching failed.
- **`:global()` rules under `.theory-page` only match wrapped content.** If a theory page's `<ul>` renders unstyled, you forgot the `<div class="theory-page">` wrapper. Add it.
- **`<Section>` component already provides `:global(h2)` and `:global(p)` styles.** Do not duplicate those in the layout's global block.
- **Table overflow on narrow screens.** The schema page's tables can blow out a 640px window. If they do, wrap each `<table>` in `<div class="table-wrap" style="overflow-x: auto;">` (or add a `.table-wrap` scoped style block on the schema page).
- **Anchors in SvelteKit.** A page-internal fragment link like `#schema` works out of the box if you put `id="schema"` on a real element. Cross-page fragment links like `/theory/open-questions#schema` also work; SvelteKit navigates to the page, then scrolls to the anchor. If scrolling doesn't happen, the anchor element doesn't exist — check the `<section id="...">` wrapper.
- **Don't run `cargo make check`** at the end. It runs the full Rust + TS check pipeline, which is slow. `bun run check` in `packages/temper-ui` is the relevant gate for this work. The pre-commit hook will run the full check on each commit anyway.
- **Drafts must still exist during translation.** Don't run the Task 13 deletion early — Tasks 4-12 read the draft files for prose content.

---

## Self-review against spec — checklist

Before declaring the plan complete, the implementer should:

- [ ] 11 files created (1 layout, 1 nav component, 9 pages) — Tasks 1, 2, 3, 4-12.
- [ ] 9 draft files deleted at the repo root — Task 13.
- [ ] 1 file moved (`docs-ia-proposal.md` → `docs/theory-ia-proposal.md`) — Task 13.
- [ ] Outer site routes untouched — every commit's `git status` confirms no changes outside `packages/temper-ui/src/routes/(public)/theory/` and the repo-root cleanup.
- [ ] Canonical reading order matches the spec's Section 2.
- [ ] Entry page suppresses chrome; sub-pages have backlink + prev/next.
- [ ] Cross-links use the exact `/theory/...` paths the drafts reference; `#model` and `#schema` anchors on the open-questions page work.
- [ ] No new automated tests were added (per spec out-of-scope).
