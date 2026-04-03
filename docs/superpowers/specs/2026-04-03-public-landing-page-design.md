# Public Landing Page & Deploy Pipeline — Design Spec

**Date**: 2026-04-03
**Branch**: `jcoletaylor/sveltekit-ui-for-temperkb-io-foundations`
**Session**: 2 of SvelteKit UI Foundations
**Scope**: Landing page, docs placeholder, Vercel deploy pipeline to production

---

## 1. Scope

This session delivers:

1. A polished public landing page at `/` for temperkb.io
2. A `/docs` route with a "coming soon" page pointing to the GitHub repo
3. Updated global layout, nav, and CSS for the "Quiet Instrument" visual identity
4. Vercel deployment validated through to production

Out of scope: Auth0 integration, authenticated app pages, real documentation content (docs will get dedicated treatment later, potentially as an mdbook-style static site with temper styling).

---

## 2. Visual Identity: The Quiet Instrument

### 2.1 Direction

Dark, editorial, restrained. The content is the design. Temper is a tool for clarifying intention — the visual language should reflect that: precise, confident, quiet.

### 2.2 Color Palette

| Token | Value | Usage |
|-------|-------|-------|
| `--bg` | `#0a0a0f` | Page background — near-black with a faint blue undertone |
| `--text` | `#e8e4df` | Primary text — warm off-white |
| `--text-mid` | `rgba(255,255,255,0.65)` | Body text, descriptions |
| `--text-dim` | `rgba(255,255,255,0.45)` | Secondary text, taglines |
| `--blue` | `#7eb8da` | Accent — section labels, emphasized text, CTAs |
| `--blue-dim` | `rgba(126,184,218,0.4)` | Subtle blue — scores, secondary accents |
| `--blue-border` | `rgba(126,184,218,0.5)` | Section left borders (primary) |
| `--blue-border-dim` | `rgba(126,184,218,0.25)` | Borders on hover, secondary elements |
| `--rule` | `rgba(255,255,255,0.06)` | Section dividers |

### 2.3 Typography

| Role | Font | Weight | Notes |
|------|------|--------|-------|
| Headings | Georgia (serif) | 300 | Light weight, generous letter-spacing |
| Body | Georgia (serif) | 400 | Line-height 1.7-1.8 |
| Labels, code, CLI | JetBrains Mono | 400-500 | Uppercase with letter-spacing for labels |
| Emphasis in headings | — | italic | Colored `--blue` |

### 2.4 Key Visual Patterns

- **Left border accents**: Every content section gets a 2px left border in `--blue-border`, with content indented. Evokes markdown blockquote markers — reinforces the "everything is markdown" identity.
- **Section dividers**: Faint horizontal rules (`--rule`) between sections. No background color changes — the entire page is one continuous dark surface.
- **Generous whitespace**: 5rem vertical padding between sections. The page breathes.
- **CLI as product shot**: Terminal-styled blocks showing real temper commands and output. No screenshots, no GUI mockups.

---

## 3. Navigation

### 3.1 Behavior

Scroll-aware fixed nav:
- **At top**: Transparent background, no border. Logo and links float over the hero.
- **On scroll (>40px)**: Background fades in (`rgba(10, 10, 15, 0.95)`), bottom border appears, `backdrop-filter: blur(12px)`.

### 3.2 Content

- **Left**: `temper` logo in JetBrains Mono, colored `--blue`
- **Right**: "GitHub" link (dim text) + "Get Started" CTA (bordered, blue)
- No Docs link until real docs exist. No Dashboard link on the public page.

---

## 4. Landing Page Sections

Seven sections in order. All share the left-border accent pattern except the hero and footer.

### 4.1 Hero

- **Headline**: "Clarify your *intention*" — "intention" in blue italic
- **Tagline**: "Everything resolves to markdown. The throughline is always visible. The system gets out of the way."
- **CTAs**: "Get Started" (primary, blue border, links to `/docs`) + "View on GitHub" (secondary, dim border, links to GitHub repo)
- **CLI preview**: A styled terminal block showing `temper search "authentication decisions" --context backend` with three results (file paths + similarity scores)
- Full viewport height, centered content

### 4.2 The Premise

- **Label**: `THE PREMISE`
- **Heading**: "Knowledge work deserves *structure*"
- **Content**: Two paragraphs establishing the problem (context scatters) and the solution (markdown with frontmatter in your vault, frontmatter carries the throughline, content carries the thinking)
- This is the philosophical statement, not a feature list

### 4.3 How It Works

- **Label**: `HOW IT WORKS`
- **Heading**: "Write markdown. Let temper do the *rest*."
- **Content**: Four workflow steps displayed as command + description pairs:
  1. `temper init` — Create a vault
  2. `temper add` — Write markdown with frontmatter
  3. `temper search` — Semantic search across your vault
  4. `temper sync` — Push to cloud, pull to another machine
- Commands are kept high-level. No `temper auth login`, `temper import`, or advanced flags — those belong in documentation.

### 4.4 What Temper Tracks

- **Label**: `WHAT TEMPER TRACKS`
- **Heading**: "The vocabulary of *structured* knowledge work"
- **Content**: Brief intro paragraph, then a 2-3 column grid of concept cards:
  - **Goals** — The outcome you're working toward
  - **Tasks** — Discrete units with mode and effort
  - **Sessions** — What happened, decisions made, next steps
  - **Research** — Investigation, analysis, design explorations
  - **Decisions** — The choice, alternatives, constraints
  - **Concepts** — Domain knowledge humans and agents share
- Cards have monospace labels in blue, description text in dim

### 4.5 For Humans and Agents

- **Label**: `FOR HUMANS AND AGENTS`
- **Heading**: "Context that's always *ready to hand*"
- **Content**: Paragraph framing the problem (agents are powerful but need context) and the solution (temper gives agents the throughline)
- **Conversation transcript mock**: A styled mock of a coding agent session (generic — not branded to any specific tool) showing the actual workflow:
  1. **User**: `/temper task start api-v2-migration`
  2. **Agent**: Loads task context, displays a blockquote-styled summary (goal progress, prior sessions, key decisions, deferred work), then states readiness with awareness of what's been done and what remains
  3. **User**: `let's start with the client SDK`
  - The transcript shows the *dialogue* — how a single command gives the agent the full throughline. This is not a hypothetical; it's what temper sessions actually look like.
- Follow-up paragraph about vault subscriptions and no vendor lock-in — if it can read files, it can use temper

### 4.6 Temper Cloud

- **Label**: `TEMPER CLOUD`
- **Heading**: "Your vault, *everywhere*"
- **Content**: Brief paragraph about cross-machine, cross-team sync
- **Feature list**: Bullet points with blue dots:
  - Cross-machine sync with conflict resolution
  - Semantic search powered by pgvector embeddings
  - Team contexts with granular access control
  - Knowledge graph connecting your resources
  - Self-host or use temperkb.io — same protocol, your choice

### 4.7 Footer

- Minimal: `temper` logo left, links right (GitHub, Docs, MIT License)
- Separated by a horizontal rule
- Same dark background, lowest contrast text

---

## 5. Docs Placeholder

### 5.1 Route: `/docs`

A single page at `/docs` with the same dark aesthetic:

- Heading: "Documentation"
- Body: "Temper is under active development. Documentation is coming soon."
- Link to GitHub repo README as the current reference
- Link back to the landing page
- No sidebar, no sub-routes, no docs layout infrastructure yet

### 5.2 Future Docs Strategy

Documentation will be treated as a dedicated workstream. Options under consideration include an mdbook-style static site built and deployed alongside the SvelteKit app, using the temper visual identity. This is explicitly deferred.

---

## 6. Technical Implementation

### 6.1 CSS Architecture

Replace the current Tailwind-based `app.css` with a CSS custom properties system matching the Quiet Instrument palette. Tailwind remains available for utility classes, but the design system is expressed through CSS variables.

The `@theme` block in `app.css` will be updated to include the new dark palette tokens alongside the existing temper blue scale (which remains useful for the authenticated app pages later).

### 6.2 Route Structure Changes

```
src/routes/
├── +layout.svelte          # Updated: dark bg, scroll-aware nav
├── +page.svelte            # Replaced: full landing page
├── docs/
│   └── +page.svelte        # New: docs placeholder
├── (app)/                   # Unchanged: authenticated layout group
│   ├── +layout.svelte
│   └── dashboard/
│       └── +page.svelte
```

### 6.3 Component Extraction

The landing page sections are substantial enough to warrant extracting into components under `src/lib/components/landing/`:

- `Nav.svelte` — Scroll-aware navigation
- `Hero.svelte` — Hero section with CLI preview
- `Section.svelte` — Reusable section wrapper (left border, label, heading pattern)
- `CliBlock.svelte` — Styled terminal/CLI preview block

The individual section content (premise, how-it-works, etc.) stays inline in `+page.svelte` using `Section.svelte` as a wrapper, keeping the landing page readable as a single document.

### 6.4 Fonts

- **Georgia**: System font, no loading required
- **JetBrains Mono**: Load via Google Fonts or self-host. Add to `app.html` `<head>`.

### 6.5 Vercel Configuration

The existing `packages/temper-ui/vercel.json` with the API rewrite is correct:

```json
{
  "rewrites": [
    { "source": "/api/:path*", "destination": "https://temper-cloud.vercel.app/api/:path*" }
  ]
}
```

No changes needed. The Vercel project (`temper-ui`, `prj_UFUosi5qWyG7Vz830I0pOUkXyynK`) is already created. Deployment validation requires:

1. Push the branch to GitHub
2. Verify Vercel picks up the build (root directory set to `packages/temper-ui`)
3. Confirm preview deployment renders correctly
4. Promote to production on the temperkb.io domain

### 6.6 Environment Variables

For this session (public pages only), no environment variables are required beyond what Vercel provides by default. `DATABASE_URL` and `API_BASE_URL` are only needed once authenticated pages are built.

---

## 7. Reference Mockups

**Implementation MUST read these files before writing any code.** They are the source of truth for visual decisions — colors, typography, spacing, layout, and component structure.

| File | Description |
|------|-------------|
| `docs/superpowers/specs/mockups/2026-04-03-landing-page-full.html` | Complete landing page mockup — all 7 sections, nav, footer. Open in a browser or read the HTML source for exact CSS values, spacing, and structure. |
| `docs/superpowers/specs/mockups/2026-04-03-landing-page-agent-section.html` | Agent workflow section options — Option A (conversation transcript) is the chosen design for §4.5. |

### How to use these mockups

- The full-page mockup is a self-contained HTML file with inline CSS. Every color, font size, spacing value, and layout decision is specified in the source.
- Subagents implementing components should `Read` the relevant mockup file and extract the CSS values directly rather than interpreting descriptions from this spec.
- The mockup uses inline styles for prototyping speed. Implementation should extract these into CSS custom properties and Svelte component styles.
- The agent section mockup (Option A — conversation transcript) should replace the `temper warmup` terminal block that appears in the full-page mockup's "For Humans and Agents" section.
