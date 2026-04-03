# Public Landing Page & Deploy Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **CRITICAL: Read the mockup files before writing any code.**
> - `docs/superpowers/specs/mockups/2026-04-03-landing-page-full.html` — full page mockup, source of truth for all CSS values
> - `docs/superpowers/specs/mockups/2026-04-03-landing-page-agent-section.html` — agent section (Option A conversation transcript)
> - `docs/superpowers/specs/2026-04-03-public-landing-page-design.md` — design spec

**Goal:** Deliver a polished public landing page for temperkb.io with the "Quiet Instrument" dark visual identity, a docs placeholder page, and validated Vercel deployment.

**Architecture:** SvelteKit with Svelte 5 runes, CSS custom properties for the dark design system alongside existing Tailwind, extracted landing components for Nav/Hero/Section/CliBlock. The landing page is a single `+page.svelte` composing these components. The `(app)` layout group for authenticated pages is untouched.

**Tech Stack:** SvelteKit 2, Svelte 5 (runes mode), Tailwind CSS v4, @sveltejs/adapter-vercel, JetBrains Mono via Google Fonts

---

## File Structure

```
packages/temper-ui/src/
├── app.html                              # Modify: add JetBrains Mono font
├── app.css                               # Modify: add dark palette CSS custom properties
├── routes/
│   ├── +layout.svelte                    # Modify: replace with dark bg + Nav component
│   ├── +page.svelte                      # Replace: full landing page using Section components
│   ├── docs/
│   │   └── +page.svelte                  # Create: docs placeholder
│   └── (app)/
│       └── +layout.svelte                # Unchanged
├── lib/
│   └── components/
│       └── landing/
│           ├── Nav.svelte                # Create: scroll-aware navigation
│           ├── Hero.svelte               # Create: hero section with CLI preview
│           ├── Section.svelte            # Create: reusable section wrapper (left border, label, heading)
│           ├── CliBlock.svelte           # Create: styled terminal block
│           └── AgentTranscript.svelte    # Create: conversation transcript mock
```

---

### Task 1: CSS Design System — Dark Palette Custom Properties

**Files:**
- Modify: `packages/temper-ui/src/app.css`
- Modify: `packages/temper-ui/src/app.html`

**Context:** Read the mockup at `docs/superpowers/specs/mockups/2026-04-03-landing-page-full.html` first. Extract exact CSS values from the `:root` block in the mockup's `<style>` tag. The existing `app.css` has a Tailwind `@theme` block with temper blue scale and chalk/ink colors — keep those (they're needed for the `(app)` authenticated layout), but add the dark palette variables.

- [ ] **Step 1: Add JetBrains Mono to app.html**

Add a Google Fonts link to `app.html` `<head>` before `%sveltekit.head%`:

```html
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@300;400;500&display=swap" rel="stylesheet" />
```

- [ ] **Step 2: Add dark palette CSS custom properties to app.css**

Append after the existing `@theme` block:

```css
:root {
  --bg: #0a0a0f;
  --text: #e8e4df;
  --text-mid: rgba(255, 255, 255, 0.65);
  --text-dim: rgba(255, 255, 255, 0.45);
  --blue: #7eb8da;
  --blue-dim: rgba(126, 184, 218, 0.4);
  --blue-border: rgba(126, 184, 218, 0.5);
  --blue-border-dim: rgba(126, 184, 218, 0.25);
  --rule: rgba(255, 255, 255, 0.06);
  --mono: 'JetBrains Mono', 'Fira Code', monospace;
  --serif: 'Georgia', 'Times New Roman', serif;
}
```

- [ ] **Step 3: Verify build passes**

Run: `cd packages/temper-ui && npx svelte-kit sync && npx svelte-check --tsconfig ./tsconfig.json`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/app.html packages/temper-ui/src/app.css
git commit -m "feat(temper-ui): add dark palette CSS custom properties and JetBrains Mono font"
```

---

### Task 2: Nav Component — Scroll-Aware Navigation

**Files:**
- Create: `packages/temper-ui/src/lib/components/landing/Nav.svelte`

**Context:** Read the mockup at `docs/superpowers/specs/mockups/2026-04-03-landing-page-full.html`. Find the `<nav class="nav">` element and its CSS. The nav is transparent at top, gains `background: rgba(10, 10, 15, 0.95)` and `backdrop-filter: blur(12px)` on scroll past 40px. Contains logo left ("temper" in JetBrains Mono, `--blue`), "GitHub" link and "Get Started" CTA right.

- [ ] **Step 1: Create the Nav component**

```svelte
<!-- packages/temper-ui/src/lib/components/landing/Nav.svelte -->
<script lang="ts">
  let scrolled = $state(false);

  function handleScroll() {
    scrolled = window.scrollY > 40;
  }
</script>

<svelte:window onscroll={handleScroll} />

<nav class="nav" class:scrolled>
  <a href="/" class="nav-logo">temper</a>
  <div class="nav-links">
    <a href="https://github.com/tasker-systems/temper">GitHub</a>
    <a href="/docs" class="cta">Get Started</a>
  </div>
</nav>

<style>
  .nav {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 100;
    padding: 1.2rem 2.5rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    transition: background 0.3s, border-color 0.3s;
    border-bottom: 1px solid transparent;
  }

  .nav.scrolled {
    background: rgba(10, 10, 15, 0.95);
    border-bottom-color: var(--rule);
    backdrop-filter: blur(12px);
  }

  .nav-logo {
    font-family: var(--mono);
    font-size: 0.75rem;
    font-weight: 500;
    letter-spacing: 0.15em;
    color: var(--blue);
    text-decoration: none;
  }

  .nav-links {
    display: flex;
    gap: 1.5rem;
    align-items: center;
  }

  .nav-links a {
    font-family: var(--mono);
    font-size: 0.7rem;
    color: var(--text-dim);
    text-decoration: none;
    letter-spacing: 0.05em;
    transition: color 0.2s;
  }

  .nav-links a:hover {
    color: var(--text);
  }

  .nav-links .cta {
    padding: 0.4rem 1rem;
    border: 1px solid var(--blue-border-dim);
    color: var(--blue);
    transition: border-color 0.2s, color 0.2s;
  }

  .nav-links .cta:hover {
    border-color: var(--blue);
    color: var(--text);
  }
</style>
```

- [ ] **Step 2: Verify build passes**

Run: `cd packages/temper-ui && npx svelte-check --tsconfig ./tsconfig.json`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/lib/components/landing/Nav.svelte
git commit -m "feat(temper-ui): add scroll-aware Nav component"
```

---

### Task 3: Section and CliBlock Components — Reusable Landing Primitives

**Files:**
- Create: `packages/temper-ui/src/lib/components/landing/Section.svelte`
- Create: `packages/temper-ui/src/lib/components/landing/CliBlock.svelte`

**Context:** Read the mockup at `docs/superpowers/specs/mockups/2026-04-03-landing-page-full.html`. The `Section` component wraps every content section (except hero and footer) with the left-border accent pattern: a 2px blue left border, section label in uppercase monospace, heading with blue italic emphasis. The `CliBlock` component renders a styled terminal block used in the hero and agent sections.

- [ ] **Step 1: Create the Section component**

```svelte
<!-- packages/temper-ui/src/lib/components/landing/Section.svelte -->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let { label, children }: { label: string; children: Snippet } = $props();
</script>

<div class="section-divider"><hr /></div>
<section class="section">
  <div class="section-inner">
    <div class="section-label">{label}</div>
    {@render children()}
  </div>
</section>

<style>
  .section-divider {
    max-width: 800px;
    margin: 0 auto;
    padding: 0 2.5rem;
  }

  .section-divider hr {
    border: none;
    border-top: 1px solid var(--rule);
  }

  .section {
    max-width: 800px;
    margin: 0 auto;
    padding: 5rem 2.5rem;
  }

  .section-inner {
    border-left: 2px solid var(--blue-border);
    padding-left: 2rem;
  }

  .section-label {
    font-family: var(--mono);
    font-size: 0.65rem;
    letter-spacing: 0.2em;
    text-transform: uppercase;
    color: var(--blue);
    margin-bottom: 1.2rem;
  }

  .section :global(h2) {
    font-family: var(--serif);
    font-size: 1.6rem;
    font-weight: 300;
    margin-bottom: 1rem;
    line-height: 1.3;
    color: var(--text);
  }

  .section :global(h2 em) {
    color: var(--blue);
    font-style: italic;
  }

  .section :global(p) {
    font-family: var(--serif);
    font-size: 1rem;
    color: var(--text-mid);
    line-height: 1.8;
    margin-bottom: 1rem;
  }

  .section :global(p strong) {
    color: var(--text);
    font-weight: 400;
  }
</style>
```

- [ ] **Step 2: Create the CliBlock component**

```svelte
<!-- packages/temper-ui/src/lib/components/landing/CliBlock.svelte -->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
</script>

<div class="cli-block">
  {@render children()}
</div>

<style>
  .cli-block {
    width: 100%;
    background: rgba(255, 255, 255, 0.02);
    border: 1px solid rgba(255, 255, 255, 0.06);
    padding: 1.2rem 1.5rem;
    font-family: var(--mono);
    font-size: 0.8rem;
    text-align: left;
  }

  .cli-block :global(.cli-prompt) {
    color: var(--text-mid);
    margin-bottom: 0.8rem;
  }

  .cli-block :global(.cmd) {
    color: var(--blue);
  }

  .cli-block :global(.flag) {
    color: rgba(255, 255, 255, 0.3);
  }

  .cli-block :global(.cli-results) {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .cli-block :global(.cli-result) {
    display: flex;
    justify-content: space-between;
    padding: 0.3rem 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }

  .cli-block :global(.cli-result:last-child) {
    border-bottom: none;
  }

  .cli-block :global(.cli-score) {
    color: var(--blue-dim);
  }
</style>
```

- [ ] **Step 3: Verify build passes**

Run: `cd packages/temper-ui && npx svelte-check --tsconfig ./tsconfig.json`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/lib/components/landing/Section.svelte packages/temper-ui/src/lib/components/landing/CliBlock.svelte
git commit -m "feat(temper-ui): add Section and CliBlock reusable landing components"
```

---

### Task 4: Hero Component

**Files:**
- Create: `packages/temper-ui/src/lib/components/landing/Hero.svelte`

**Context:** Read the mockup at `docs/superpowers/specs/mockups/2026-04-03-landing-page-full.html`. Find the `<section class="hero">` element. The hero is full viewport height, centered, with the headline "Clarify your *intention*", tagline, two CTAs, and a CLI preview block showing `temper search` with results.

- [ ] **Step 1: Create the Hero component**

```svelte
<!-- packages/temper-ui/src/lib/components/landing/Hero.svelte -->
<script lang="ts">
  import CliBlock from './CliBlock.svelte';
</script>

<section class="hero">
  <h1>Clarify your <em>intention</em></h1>
  <p class="tagline">
    Everything resolves to markdown. The throughline is always visible.
    The system gets out of the way.
  </p>
  <div class="hero-ctas">
    <a href="/docs" class="primary">Get Started</a>
    <a href="https://github.com/tasker-systems/temper" class="secondary">View on GitHub</a>
  </div>
  <div class="cli-wrapper">
    <CliBlock>
      <div class="cli-prompt">
        <span class="flag">$</span> <span class="cmd">temper search</span> "authentication decisions" <span class="flag">--context backend</span>
      </div>
      <div class="cli-results">
        <div class="cli-result"><span>decision/api-auth-strategy.md</span><span class="cli-score">0.94</span></div>
        <div class="cli-result"><span>research/oauth-provider-comparison.md</span><span class="cli-score">0.87</span></div>
        <div class="cli-result"><span>session/2026-03-28-auth-implementation.md</span><span class="cli-score">0.82</span></div>
      </div>
    </CliBlock>
  </div>
</section>

<style>
  .hero {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    text-align: center;
    padding: 6rem 2.5rem 4rem;
  }

  h1 {
    font-family: var(--serif);
    font-size: clamp(2.4rem, 5vw, 3.8rem);
    font-weight: 300;
    line-height: 1.2;
    margin-bottom: 1.5rem;
    letter-spacing: 0.02em;
    color: var(--text);
  }

  h1 em {
    color: var(--blue);
    font-style: italic;
  }

  .tagline {
    font-family: var(--serif);
    font-size: 1.1rem;
    color: var(--text-dim);
    font-style: italic;
    max-width: 36em;
    margin-bottom: 3rem;
    line-height: 1.7;
  }

  .hero-ctas {
    display: flex;
    gap: 1rem;
    margin-bottom: 4rem;
  }

  .hero-ctas a {
    font-family: var(--mono);
    font-size: 0.8rem;
    padding: 0.6rem 1.5rem;
    text-decoration: none;
    letter-spacing: 0.05em;
    transition: all 0.2s;
  }

  .hero-ctas .primary {
    border: 1px solid var(--blue-border);
    color: var(--blue);
  }

  .hero-ctas .primary:hover {
    background: rgba(126, 184, 218, 0.1);
  }

  .hero-ctas .secondary {
    border: 1px solid rgba(255, 255, 255, 0.12);
    color: var(--text-dim);
  }

  .hero-ctas .secondary:hover {
    border-color: rgba(255, 255, 255, 0.25);
    color: var(--text-mid);
  }

  .cli-wrapper {
    width: 100%;
    max-width: 620px;
  }
</style>
```

- [ ] **Step 2: Verify build passes**

Run: `cd packages/temper-ui && npx svelte-check --tsconfig ./tsconfig.json`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/lib/components/landing/Hero.svelte
git commit -m "feat(temper-ui): add Hero component with CLI preview"
```

---

### Task 5: AgentTranscript Component

**Files:**
- Create: `packages/temper-ui/src/lib/components/landing/AgentTranscript.svelte`

**Context:** Read the mockup at `docs/superpowers/specs/mockups/2026-04-03-landing-page-agent-section.html`. Option A (conversation transcript) is the chosen design. This shows a mock coding agent session: user invokes `/temper task start`, agent responds with loaded context in a blockquote-styled summary, user gives next direction. The component is generic — not branded to any specific agent tool.

- [ ] **Step 1: Create the AgentTranscript component**

```svelte
<!-- packages/temper-ui/src/lib/components/landing/AgentTranscript.svelte -->
<div class="transcript">
  <div class="message">
    <div class="role">you</div>
    <div class="content user">/temper task start api-v2-migration</div>
  </div>

  <div class="message">
    <div class="role">agent</div>
    <div class="content agent">
      <div class="agent-text">Loading task context...</div>
      <div class="context-summary">
        Goal: api-v2-migration (3 tasks, 2 complete)<br />
        Prior sessions: 4 (last: Mar 28 — auth middleware)<br />
        Key decisions: REST over GraphQL, JWT rotation<br />
        Deferred: rate limiting, webhook signatures
      </div>
      <div class="agent-text">
        This is a <span class="highlight">build/medium</span> task. Based on prior sessions,
        you've completed the auth middleware and route migration. The remaining work is the
        client SDK update.
      </div>
      <div class="agent-text">
        I've read the research doc and the two deferred decisions. Ready to plan the implementation.
      </div>
    </div>
  </div>

  <div class="message">
    <div class="role">you</div>
    <div class="content user">let's start with the client SDK</div>
  </div>
</div>

<style>
  .transcript {
    border: 1px solid rgba(255, 255, 255, 0.06);
    padding: 1.2rem;
    margin-top: 1.5rem;
    font-family: var(--mono);
    font-size: 0.7rem;
    line-height: 1.8;
  }

  .message {
    margin-bottom: 1.2rem;
  }

  .message:last-child {
    margin-bottom: 0;
  }

  .message + .message {
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    padding-top: 1rem;
  }

  .role {
    color: var(--blue-dim);
    font-size: 0.6rem;
    margin-bottom: 0.4rem;
  }

  .content.user {
    color: rgba(255, 255, 255, 0.7);
  }

  .content.agent {
    font-family: var(--serif);
    font-size: 0.8rem;
    line-height: 1.7;
  }

  .agent-text {
    color: rgba(255, 255, 255, 0.55);
    margin-bottom: 0.6rem;
  }

  .agent-text:last-child {
    margin-bottom: 0;
  }

  .context-summary {
    font-family: var(--mono);
    font-size: 0.65rem;
    color: rgba(255, 255, 255, 0.35);
    margin-bottom: 0.8rem;
    padding: 0.6rem;
    background: rgba(255, 255, 255, 0.02);
    border-left: 2px solid rgba(126, 184, 218, 0.3);
    line-height: 1.8;
  }

  .highlight {
    color: var(--blue);
  }
</style>
```

- [ ] **Step 2: Verify build passes**

Run: `cd packages/temper-ui && npx svelte-check --tsconfig ./tsconfig.json`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/lib/components/landing/AgentTranscript.svelte
git commit -m "feat(temper-ui): add AgentTranscript component for landing page"
```

---

### Task 6: Root Layout — Dark Background with Nav

**Files:**
- Modify: `packages/temper-ui/src/routes/+layout.svelte`

**Context:** The current root layout has a light background (`bg-chalk`) with a header containing "temper | Docs | Dashboard" using Tailwind classes. Replace it with the dark background and the Nav component. The `(app)` layout group has its own layout for authenticated pages — it will need its own styling later, but for now it inherits the dark root.

- [ ] **Step 1: Update the root layout**

Replace the entire content of `+layout.svelte`:

```svelte
<script>
  import '../app.css';
  import Nav from '$lib/components/landing/Nav.svelte';

  let { children } = $props();
</script>

<svelte:head>
  <title>temper — clarify your intention</title>
  <meta name="description" content="CLI-first knowledge base with semantic search, frontmatter-driven structure, and cloud sync." />
</svelte:head>

<div class="app">
  <Nav />
  <main>
    {@render children()}
  </main>
</div>

<style>
  .app {
    min-height: 100vh;
    background: var(--bg);
    color: var(--text);
    font-family: var(--serif);
    line-height: 1.7;
    -webkit-font-smoothing: antialiased;
  }
</style>
```

- [ ] **Step 2: Verify build passes**

Run: `cd packages/temper-ui && npx svelte-check --tsconfig ./tsconfig.json`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/routes/+layout.svelte
git commit -m "feat(temper-ui): update root layout with dark theme and Nav component"
```

---

### Task 7: Landing Page — All Seven Sections

**Files:**
- Modify: `packages/temper-ui/src/routes/+page.svelte`

**Context:** Read both mockup files:
- `docs/superpowers/specs/mockups/2026-04-03-landing-page-full.html` — full page structure
- `docs/superpowers/specs/mockups/2026-04-03-landing-page-agent-section.html` — Option A conversation transcript for §4.5

Replace the existing placeholder page with the full landing page. Uses Hero, Section, CliBlock, and AgentTranscript components. Section content stays inline using Section as a wrapper.

- [ ] **Step 1: Replace the landing page**

Replace the entire content of `+page.svelte`:

```svelte
<script>
  import Hero from '$lib/components/landing/Hero.svelte';
  import Section from '$lib/components/landing/Section.svelte';
  import CliBlock from '$lib/components/landing/CliBlock.svelte';
  import AgentTranscript from '$lib/components/landing/AgentTranscript.svelte';
</script>

<Hero />

<!-- The Premise -->
<Section label="The premise">
  <h2>Knowledge work deserves <em>structure</em></h2>
  <p>
    Code is an expression of intent. So are specifications, plans, and decisions.
    But the context behind them — the <strong>why</strong>, the alternatives considered,
    the constraints that shaped the choice — scatters across conversations, documents,
    and memory.
  </p>
  <p>
    Temper gives that context a home. Every goal, task, session, research thread,
    and decision lives as <strong>markdown with frontmatter</strong> in your vault.
    The frontmatter carries the throughline. The content carries the thinking.
  </p>
</Section>

<!-- How It Works -->
<Section label="How it works">
  <h2>Write markdown. Let temper do the <em>rest</em>.</h2>
  <div class="workflow">
    <div class="workflow-step">
      <span class="workflow-cmd">temper init</span>
      <span class="workflow-desc">Create a vault — a directory of markdown files with a temper.toml config</span>
    </div>
    <div class="workflow-step">
      <span class="workflow-cmd">temper add</span>
      <span class="workflow-desc">Write markdown with frontmatter. Temper infers context, doc type, and relationships.</span>
    </div>
    <div class="workflow-step">
      <span class="workflow-cmd">temper search</span>
      <span class="workflow-desc">Semantic search across your vault. Find decisions by meaning, not just keywords.</span>
    </div>
    <div class="workflow-step">
      <span class="workflow-cmd">temper sync</span>
      <span class="workflow-desc">Push to the cloud. Pull to another machine. Your vault follows you.</span>
    </div>
  </div>
</Section>

<!-- What Temper Tracks -->
<Section label="What temper tracks">
  <h2>The vocabulary of <em>structured</em> knowledge work</h2>
  <p>
    Every file in your vault has a doc type that temper understands.
    These aren't arbitrary tags — they're the building blocks of how
    work actually progresses.
  </p>
  <div class="concepts">
    <div class="concept">
      <div class="concept-name">Goals</div>
      <p>The outcome you're working toward. Tasks and sessions roll up to goals.</p>
    </div>
    <div class="concept">
      <div class="concept-name">Tasks</div>
      <p>Discrete units of work with mode (plan/build) and effort (small/medium/large).</p>
    </div>
    <div class="concept">
      <div class="concept-name">Sessions</div>
      <p>What happened in a working session — decisions made, context discovered, next steps.</p>
    </div>
    <div class="concept">
      <div class="concept-name">Research</div>
      <p>Investigation and analysis. Design explorations, comparisons, architectural options.</p>
    </div>
    <div class="concept">
      <div class="concept-name">Decisions</div>
      <p>The choice, the alternatives, the constraints. Captured so you never re-litigate.</p>
    </div>
    <div class="concept">
      <div class="concept-name">Concepts</div>
      <p>Domain knowledge. The vocabulary of your project that humans and agents share.</p>
    </div>
  </div>
</Section>

<!-- For Humans and Agents -->
<Section label="For humans and agents">
  <h2>Context that's always <em>ready to hand</em></h2>
  <p>
    Agentic tools like Claude Code and Cursor are powerful — but only when
    they have context. Temper gives agents the same throughline that humans
    carry in their heads: what we're building, why, what we've decided, and
    what's deferred.
  </p>
  <AgentTranscript />
  <p class="after-transcript">
    Subscribe to contexts across projects. Everything arrives as markdown
    in your vault — no special tooling, no vendor lock-in. If it can read
    files, it can use temper.
  </p>
</Section>

<!-- Temper Cloud -->
<Section label="Temper Cloud">
  <h2>Your vault, <em>everywhere</em></h2>
  <p>
    Work on your laptop. Pick up on your desktop. Let a cloud agent
    contribute while you sleep. Temper Cloud syncs your vault across
    machines and team members with semantic search built in.
  </p>
  <div class="cloud-features">
    <div class="cloud-feature"><div class="dot"></div><span>Cross-machine sync with conflict resolution</span></div>
    <div class="cloud-feature"><div class="dot"></div><span>Semantic search powered by pgvector embeddings</span></div>
    <div class="cloud-feature"><div class="dot"></div><span>Team contexts with granular access control</span></div>
    <div class="cloud-feature"><div class="dot"></div><span>Knowledge graph connecting your resources</span></div>
    <div class="cloud-feature"><div class="dot"></div><span>Self-host or use temperkb.io — same protocol, your choice</span></div>
  </div>
</Section>

<!-- Footer -->
<footer class="footer">
  <div class="footer-logo">temper</div>
  <div class="footer-links">
    <a href="https://github.com/tasker-systems/temper">GitHub</a>
    <a href="/docs">Docs</a>
    <a href="https://github.com/tasker-systems/temper/blob/main/LICENSE">MIT License</a>
  </div>
</footer>

<style>
  /* Workflow steps */
  .workflow {
    display: flex;
    flex-direction: column;
    gap: 1.2rem;
    margin-top: 1.5rem;
  }

  .workflow-step {
    display: flex;
    align-items: flex-start;
    gap: 1.2rem;
  }

  .workflow-cmd {
    font-family: var(--mono);
    font-size: 0.75rem;
    padding: 0.4rem 0.8rem;
    border: 1px solid rgba(255, 255, 255, 0.1);
    color: var(--blue);
    white-space: nowrap;
    min-width: 140px;
  }

  .workflow-desc {
    font-family: var(--serif);
    font-size: 0.9rem;
    color: var(--text-dim);
    padding-top: 0.3rem;
    line-height: 1.7;
  }

  /* Concept cards */
  .concepts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 1rem;
    margin-top: 1.5rem;
  }

  .concept {
    border: 1px solid rgba(255, 255, 255, 0.06);
    padding: 1.2rem;
    transition: border-color 0.2s;
  }

  .concept:hover {
    border-color: var(--blue-border-dim);
  }

  .concept-name {
    font-family: var(--mono);
    font-size: 0.7rem;
    color: var(--blue);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    margin-bottom: 0.5rem;
  }

  .concept :global(p) {
    font-size: 0.85rem;
    color: var(--text-dim);
    line-height: 1.6;
    margin-bottom: 0;
  }

  /* After transcript spacing */
  .after-transcript {
    margin-top: 1.5rem;
  }

  /* Cloud features */
  .cloud-features {
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
    margin-top: 1.5rem;
  }

  .cloud-feature {
    display: flex;
    gap: 1rem;
    align-items: baseline;
  }

  .dot {
    width: 4px;
    height: 4px;
    background: var(--blue);
    border-radius: 50%;
    flex-shrink: 0;
    margin-top: 0.5rem;
  }

  .cloud-feature span {
    font-family: var(--serif);
    font-size: 0.95rem;
    color: var(--text-mid);
  }

  /* Footer */
  .footer {
    max-width: 800px;
    margin: 0 auto;
    padding: 4rem 2.5rem;
    border-top: 1px solid var(--rule);
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .footer-logo {
    font-family: var(--mono);
    font-size: 0.7rem;
    color: var(--blue-dim);
    letter-spacing: 0.1em;
  }

  .footer-links {
    display: flex;
    gap: 1.5rem;
  }

  .footer-links a {
    font-family: var(--mono);
    font-size: 0.65rem;
    color: rgba(255, 255, 255, 0.25);
    text-decoration: none;
    letter-spacing: 0.05em;
    transition: color 0.2s;
  }

  .footer-links a:hover {
    color: var(--text-dim);
  }
</style>
```

- [ ] **Step 2: Run dev server and visually verify**

Run: `cd packages/temper-ui && npx vite dev --port 5173`
Open: `http://localhost:5173`
Expected: Full landing page renders with all 7 sections matching the mockup aesthetic

- [ ] **Step 3: Verify build passes**

Run: `cd packages/temper-ui && npx vite build`
Expected: Build succeeds with no errors

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/routes/+page.svelte
git commit -m "feat(temper-ui): implement full landing page with all seven sections"
```

---

### Task 8: Docs Placeholder Page

**Files:**
- Create: `packages/temper-ui/src/routes/docs/+page.svelte`

**Context:** A simple placeholder page at `/docs` with the same dark aesthetic. No sidebar, no sub-routes. Points to GitHub repo README as the current reference.

- [ ] **Step 1: Create the docs placeholder**

```svelte
<!-- packages/temper-ui/src/routes/docs/+page.svelte -->
<svelte:head>
  <title>Documentation — temper</title>
</svelte:head>

<div class="docs-placeholder">
  <h1>Documentation</h1>
  <p>
    Temper is under active development. Documentation is coming soon.
  </p>
  <p>
    In the meantime, the
    <a href="https://github.com/tasker-systems/temper">GitHub repository README</a>
    is the best reference for getting started.
  </p>
  <a href="/" class="back-link">&larr; Back to home</a>
</div>

<style>
  .docs-placeholder {
    max-width: 600px;
    margin: 0 auto;
    padding: 12rem 2.5rem 6rem;
  }

  h1 {
    font-family: var(--serif);
    font-size: 2rem;
    font-weight: 300;
    color: var(--text);
    margin-bottom: 1.5rem;
  }

  p {
    font-family: var(--serif);
    font-size: 1rem;
    color: var(--text-mid);
    line-height: 1.8;
    margin-bottom: 1rem;
  }

  a {
    color: var(--blue);
    text-decoration: none;
    transition: color 0.2s;
  }

  a:hover {
    color: var(--text);
  }

  .back-link {
    display: inline-block;
    margin-top: 2rem;
    font-family: var(--mono);
    font-size: 0.75rem;
    color: var(--text-dim);
    letter-spacing: 0.05em;
  }

  .back-link:hover {
    color: var(--blue);
  }
</style>
```

- [ ] **Step 2: Verify build passes**

Run: `cd packages/temper-ui && npx svelte-check --tsconfig ./tsconfig.json`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/routes/docs/+page.svelte
git commit -m "feat(temper-ui): add docs placeholder page"
```

---

### Task 9: Vercel Deployment — Preview and Production

**Files:**
- No file changes — this is deployment validation

**Context:** The Vercel project `temper-ui` (`prj_UFUosi5qWyG7Vz830I0pOUkXyynK`) is already created and connected to the GitHub repo. Root directory is set to `packages/temper-ui`. The `vercel.json` in that directory has the API rewrite configured.

- [ ] **Step 1: Push the branch to GitHub**

```bash
git push origin jcoletaylor/sveltekit-ui-for-temperkb-io-foundations
```

- [ ] **Step 2: Verify preview deployment**

Run: `vercel ls --project temper-ui` or check the Vercel dashboard.
Expected: A preview deployment should be triggered automatically from the push. Wait for it to complete and verify the landing page renders correctly at the preview URL.

- [ ] **Step 3: Check the preview URL**

Open the preview URL in a browser. Verify:
- Nav is transparent at top, solidifies on scroll
- All 7 sections render with correct styling
- `/docs` shows the placeholder page
- No console errors
- Fonts load correctly (JetBrains Mono visible in labels and CLI blocks)

- [ ] **Step 4: Deploy to production**

Once preview looks good:

```bash
vercel promote <deployment-url> --scope <team>
```

Or use the Vercel dashboard to promote the preview to production on the temperkb.io domain.

- [ ] **Step 5: Verify production**

Open `https://temperkb.io` and confirm the landing page renders identically to the preview.
