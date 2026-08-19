# temper — Brand Direction & Creative Strategy

## Part 1: What You Have (Audit)

### The Site

The current temperkb.io is a single-page landing with a strong editorial sensibility. Dark ground (#0a0a0f), warm off-white text (#e8e4df), Georgia serif for body copy, JetBrains Mono for code and labels. A single accent — muted steel-blue (#7eb8da) — carries every interactive signal: nav logo, section labels, CLI commands, emphasized words.

The layout is disciplined. Single column at 800px max-width. Sections divide with a thin rule and announce themselves with uppercase mono labels, then develop behind a blue left-border. The hero centers on a tagline — *"Clarify your intention"* — that reads more like the opening of an essay than a SaaS landing page.

This is good. The tone is literate, considered, and quiet. It communicates that this tool was built by someone who thinks carefully — which is exactly the sensibility temper should project. The restraint is the brand.

**What's working:**

- The serif + mono pairing creates a distinctive voice — editorial meets terminal. Few developer tools look like this; most reach for Inter or some geometric sans. Georgia gives temper a sense of *writing* rather than *designing*, which maps directly to the markdown-native philosophy.
- The muted blue accent avoids the saturated-blue-on-dark cliché of most DevTools sites. It reads as steel, water, or twilight — all of which carry the right connotations (resilience, depth, calm focus).
- The section structure (label → heading with italic emphasis → body prose) creates rhythm without visual clutter. No hero images, no gradients, no illustrations fighting for attention.
- The CLI block and agent transcript are real demonstrations, not mockups. They show the product doing its actual thing.

**What needs attention:**

- The SVG diagrams (context-rot.svg, throughline-layers.svg, etc.) live in a completely different visual universe — light backgrounds (#fafafa), system-ui fonts, Tailwind-derived pastels (purple-blue-green). They look like they belong to a different product. These need to be brought into the temper aesthetic.
- The favicon is a blue rounded square with a white "t" in system-ui bold. It's functional but generic — it doesn't carry any of the brand's editorial character.
- The site tells one story to one audience. It's implicitly speaking to a solo developer who already understands context rot. The personas (builders, teams, agents) aren't yet differentiated.
- The tagline "Clarify your intention" is evocative but slightly abstract. It doesn't immediately tell you what the tool does. This works for the main landing (it's a mood-setter), but persona pages need sharper framing.
- "knowledge base with structure" (the meta title) is too generic. It sounds like Notion or Confluence.

### The README & Vision

The written voice in README.md and VISION.md is the product's strongest brand asset. It's precise, opinionated, and literate without being pretentious. Phrases like "the connective tissue that turns a pile of specs and tickets into a navigable development history" and "context compounds instead of decaying" are genuinely good writing — they carry both the technical claim and the emotional truth.

The vision doc in particular reads like a design manifesto. The "Not a Ticketing System" framing and "the knowledge base — not the tool — is the unit of value" are the kind of positioning statements that stick.

---

## Part 2: The Brand (Definition)

### Name Etymology

**temper** /ˈtempər/ — three meanings, all earned:

1. **Metallurgical.** To heat and cool metal in a controlled process, making it harder and more resilient without making it brittle. This is the core metaphor: your knowledge base isn't just collected, it's *refined through use*. Each session tempers the context.

2. **Dispositional.** The temperament of a project — its character, its tendencies, its rhythm. Temper holds the project's disposition: what it cares about, what it's decided, what it's deferred.

3. **Musical.** To temper an instrument is to adjust intervals so that it plays well in all keys. Temper adjusts the intervals between sessions so that any starting point — any agent, any machine, any day — can pick up the melody.

The verb sense is primary. *Temper your context.* Not "store" it, not "manage" it. Temper it — refine it, strengthen it, make it resilient.

### Core Concepts

Three ideas anchor every piece of temper communication:

**The Throughline.** The connective thread across sessions, decisions, and evolving understanding. Not a log. Not a changelog. A *narrative* that the project tells about itself — what it was, what it's becoming, and why. The throughline is what an agent reads at the start of a session to understand not just *what* to do, but *why this work matters right now*. It's the difference between a pile of documents and a living history.

**Markdown as Medium.** Markdown is the native language of both human thought and machine comprehension. It's portable, version-controllable, readable in any editor, and natively understood by every language model. Temper doesn't invent a new format — it gives structure to the format people already think in. Frontmatter carries the throughline. Content carries the thinking. Git carries the history. Everything resolves to markdown.

**Session Over Session.** The unit of work isn't the ticket — it's the session. Each session reads what came before, does its work within a scope, and writes back what it learned. Context compounds. Decisions persist. The vault grows richer with use, not staler with age. This is the antidote to context rot: not better search, but better *continuity*.

### Voice & Tone

**Register:** Literate technical. Write as if explaining to a sharp colleague over coffee — precise but not formal, opinionated but not aggressive, confident but not salesy. Assume intelligence. Don't oversimplify.

**Rhythm:** Short declarative sentences for claims. Longer sentences with subordinate clauses for elaboration. The em-dash is our friend — it lets us layer context without losing momentum. Paragraphs of 2-4 sentences. Never bullet points in marketing copy (the site should feel like reading, not scanning).

**Vocabulary preferences:**

| Prefer | Over |
|--------|------|
| throughline | context window, memory |
| vault | workspace, repository |
| session | conversation, chat |
| temper (verb) | manage, organize, maintain |
| compounds | grows, scales |
| resilient | robust, reliable |
| intention | configuration, setup |
| narrative | data, information |
| resolves to markdown | outputs markdown |

**Specific avoidances:**
- Never "AI-powered" or "intelligent" as selling points. Temper is agent-aware, not agent-branded.
- Never "never lose context again" or similar absolute promises. The framing is *compounding*, not *perfection*.
- Never "simple" or "easy." The tool respects complexity. The word is *considered* or *deliberate*.
- Never "revolutionize" or "transform." The word is *temper* — gradual, intentional improvement.
- Avoid "knowledge management" (sounds like enterprise middleware). This is a *knowledge base* with *structure*.

**The italic emphasis pattern.** The site uses italicized words in headings to draw the eye to the key concept: "Clarify your *intention*", "Knowledge work deserves *structure*". This is a strong brand pattern. Use it consistently — one italicized word per heading, always the conceptual anchor.

### Color System

The current accent (#7eb8da, muted steel-blue) is the right starting point, but needs to be developed into a system.

**Primary palette (dark ground):**

| Role | Hex | Name | Usage |
|------|-----|------|-------|
| Ground | #0a0a0f | Obsidian | Page background |
| Text | #e8e4df | Parchment | Body text |
| Text mid | rgba(255,255,255,0.65) | Chalk | Secondary text |
| Text dim | rgba(255,255,255,0.45) | Graphite | Tertiary text, labels |
| Accent | #7eb8da | Temper Blue | Links, emphasis, borders |
| Accent dim | rgba(126,184,218,0.4) | Temper Blue (muted) | Subtle accents |
| Rule | rgba(255,255,255,0.06) | — | Dividers |

**Extended palette (for diagrams, illustrations, persona differentiation):**

| Name | Hex | Semantic | Usage |
|------|-----|----------|-------|
| Session Green | #86efac / #166534 | Active, growing | Sessions, continuity |
| Decision Amber | #fcd34d / #92400e | Choices, inflection | Decisions, scope routing |
| Deferred Slate | #94a3b8 / #475569 | Paused, future | Deferred work, next sessions |
| Rot Red | #fca5a5 / #991b1b | Decay, loss | Context rot (problem state) |

These secondary colors appear ONLY in diagrams and illustrations, never in the site chrome. The site itself stays monochrome-plus-blue.

**Diagram visual language (replacing current SVGs):**

The current SVGs use light backgrounds and Tailwind pastels. The new standard:

- Dark ground matching the site (#0a0a0f or slightly lifted #12121a)
- Same serif + mono typography as the site
- Temper Blue for primary connections, with green/amber/slate for semantic roles
- Thin strokes (0.5-1px), no heavy borders
- No rounded-rect-with-drop-shadow aesthetic — flat, editorial, structural

### Typography

**Current (keep):**

- Body: Georgia, serif — the literary register
- Code/labels: JetBrains Mono — the technical register
- Emphasis: Italic Georgia for the one key word in each heading

**Hierarchy:**

- H1 (hero): clamp(2.4rem, 5vw, 3.8rem), weight 300, serif
- H2 (section): 1.6rem, weight 300, serif, with italic emphasis word
- Section label: 0.65rem, mono, uppercase, 0.2em tracking, Temper Blue
- Body: 1rem, serif, 1.8 line-height
- CLI/code: 0.8rem mono, with blue highlighting for commands
- Scores/metadata: 0.65-0.7rem mono

### Spatial Philosophy

- Single column, 800px max-width
- Generous vertical rhythm (5rem section padding)
- Left-border accent (2px blue) on content sections
- Horizontal rules as section separators
- No cards, no grid layouts, no multi-column. The reading experience is linear, like a well-typeset essay.

---

## Part 3: The Icon (Favicon & Brand Mark)

### Concept: The Temper Mark

The current favicon — a blue rounded square with a white "t" — is placeholder-level. The brand mark should carry the throughline concept.

**Direction: The Threaded T**

A lowercase "t" where the crossbar extends and curves into a continuous line — suggesting the throughline weaving through the vertical stroke. The line doesn't close; it trails off, implying continuation. The "t" reads as both the letter and a simplified needle-and-thread.

At favicon scale (16x16, 32x32), this simplifies to: a vertical stroke with a crossbar that has a slight curve or trail at one end — enough to suggest movement and continuity without becoming illegible.

**Rendering:**

- Monochrome. Works in Temper Blue on dark, or dark on light. No filled backgrounds at small sizes.
- At 32x32: the vertical stroke + curved crossbar
- At 64x64+: the crossbar's trailing thread becomes more visible, curving gently downward
- At display size: the full threaded-t with the throughline visible as a continuous path

**Secondary mark: The Throughline Glyph**

For use in diagrams, decorative contexts, and as a section ornament: three short horizontal lines at slightly different vertical positions, connected by a single curved vertical line. Abstract representation of "layers connected by a thread." Think musical staff meets connecting line.

### Where the marks appear

| Context | Mark | Treatment |
|---------|------|-----------|
| Browser favicon | Threaded T | Temper Blue on transparent |
| GitHub repo avatar | Threaded T | Temper Blue on #0a0a0f |
| GitHub social preview | Full wordmark + tagline | Dark ground |
| Site nav logo | "temper" in mono (current) | Keep as-is |
| README header | Threaded T + "temper" wordmark | Inline SVG |
| Diagram corner | Throughline glyph (small) | Subtle, decorative |

---

## Part 4: Persona Pages

### Page Architecture

Each persona page follows the same structure as the landing, with adapted content:

1. **Hero** — persona-specific tagline + a tailored CLI/agent demonstration
2. **Problem framing** — the specific pain this persona feels
3. **Throughline promise** — how temper addresses it
4. **Feature showcase** — 2-3 features most relevant to this persona
5. **Getting started** — persona-appropriate onboarding path
6. **Cross-sell** — subtle nod to the other personas ("temper also works for...")

All pages share the same layout system (single column, serif + mono, blue accent, left-border sections). The differentiation is in copy, demonstrations, and which SVG diagrams appear.

### Temper for Builders

**Audience:** Solo developers, technically inclined PMs, indie hackers, side-project builders. People who work alone or in very small groups, often with AI agents as their primary collaborator.

**Tagline:** *"Remember what you *decided*"*

**Core narrative:** You're three sessions into a feature. You made a decision about the auth strategy on Tuesday, explored two caching approaches on Wednesday, and now it's Friday and the agent has no idea any of that happened. You re-explain. Again. Temper holds the thread so you don't have to.

**Key demonstrations:**
- `temper warmup` injecting last session's context — the "oh, it remembers" moment
- `temper search "caching strategy"` returning the decision doc with 0.94 relevance
- The session continuity cycle diagram (dark-mode version)

**Feature emphasis:**
- Session continuity (warmup → work → save cycle)
- Semantic search across your vault
- Scope-based workflow (patch/feature/epic)
- Markdown vault as institutional memory

**Tone:** Direct, practical, slightly conspiratorial ("you know this feeling"). Speaks to the frustration of re-explaining context to agents.

**Hero CLI block:**
```
$ temper warmup --project myapp

Last session: Mar 28 — Chose JWT rotation over session tokens
In-progress: api-auth-middleware (feature, 60% complete)
Deferred: rate limiting (blocked on load testing)
3 sessions of context loaded. Ready.
```

### Temper for Teams

**Audience:** Small teams (2-8) working on shared projects, and larger engineering organizations evaluating self-hosted knowledge management. Teams already using Claude Code, Cursor, or similar tools.

**Tagline:** *"Your team's throughline, *everywhere*"*

**Core narrative:** When one person's Tuesday decision lives in their chat history and another person's Wednesday implementation lives in a different IDE, the project's narrative fragments. Temper Cloud gives every session — human or agent, laptop or CI runner — the same ground truth. The vault syncs. The throughline holds.

**Key demonstrations:**
- Team sync scenario: Developer A saves a session on their laptop, Developer B pulls context on their desktop
- Agent-to-agent handoff: a CI agent reads what the development agent decided
- The dual-authority model diagram (git for content, Postgres for metadata)

**Feature emphasis:**
- Temper Cloud sync with conflict resolution
- Team contexts with access control
- Self-hosting option (same protocol, your infrastructure)
- pgvector semantic search at scale
- Knowledge graph connecting resources across projects

**Tone:** Still literate and considered, but with more emphasis on collaboration dynamics and organizational concerns. Speaks to the "we tried shared docs and it didn't stick" experience.

**Hero CLI block:**
```
$ temper sync --team engineering

Pulling 3 new sessions from cloud...
  → alex: session/2026-03-29 — migrated payment service
  → ci-agent: session/2026-03-29 — ran integration suite, 2 failures
  → dana: decision/retry-backoff-strategy — exponential with jitter

Your vault is current. 847 resources indexed.
```

### Temper for Agents

**Audience:** Developers building agent workflows, AI tooling enthusiasts, people evaluating MCP integrations. This page speaks to the "how do I make my agents smarter" curiosity.

**Tagline:** *"Context that's always *ready to hand*"*

**Core narrative:** Your agent is powerful. It can write code, analyze architecture, plan implementations. But every session, it starts from zero — no memory of what it built yesterday, no awareness of the decisions that shaped today's constraints. Temper gives agents what they lack: a persistent, structured, searchable context layer. Through the CLI, agents read and write markdown. Through the MCP server, they query the vault directly. The throughline isn't just for humans anymore.

**Key demonstrations:**
- The agent transcript (already on the landing page, but expanded)
- MCP server integration: an agent querying `temper.search("authentication decisions")` through MCP
- Skill generation: `temper skill install` producing a Claude Code skill file

**Feature emphasis:**
- CLI as agent interface (temper warmup, search, session save)
- MCP server for direct agent-to-vault communication
- Generated skill files that teach agents the vault's structure
- Session pre-warming via hooks
- The vault as shared memory between human and agent sessions

**Tone:** Technically precise, slightly forward-looking. Speaks to the excitement of agents that actually understand project context. Less emotional than the Builders page, more architecturally curious.

**Hero interaction:**
```
agent (via MCP) → temper.search("payment retry strategy")

  decision/retry-backoff-strategy.md     0.96
  session/2026-03-29-payment-service.md  0.91
  research/idempotency-patterns.md       0.84

agent: I see we decided on exponential backoff with jitter (Mar 29).
       The research doc notes a P99 concern above 5 retries.
       I'll implement with a configurable max_retries defaulting to 4.
```

---

## Part 5: SVG Evolution Strategy

### The Problem

The current diagrams are in the "light-mode documentation illustration" genre — white backgrounds, system-ui fonts, Tailwind colors. They were built for README.md rendering on GitHub (which has a white background) and look perfectly fine there. But they clash with the site's dark, editorial aesthetic.

### The Solution: Two Renderings, One Source of Truth

Rather than maintaining two entirely separate SVG sets, the diagrams should be designed to work in both contexts:

**For the site (dark ground):**
- Background: transparent (inherits #0a0a0f from the page)
- Text: #e8e4df (parchment) for primary, rgba(255,255,255,0.45) for secondary
- Accents: Temper Blue (#7eb8da) for primary connections
- Semantic colors: Session Green, Decision Amber, Deferred Slate, Rot Red (muted versions)
- Typography: Georgia for labels, JetBrains Mono for code/technical text
- Strokes: 0.5-1px, subtle

**For GitHub/README (light ground):**
- Background: transparent (inherits white from GitHub)
- Text: #1a1a1a for primary, #888 for secondary
- Accents: #2563eb (more saturated blue for legibility on white)
- Semantic colors: the current Tailwind-derived palette is fine here
- Typography: system-ui (GitHub doesn't load custom fonts)

The practical approach: maintain the current SVGs for README/GitHub use, and create new dark-ground versions for the site. Over time, these can converge into a single SVG using CSS custom properties and `prefers-color-scheme`, but for now, two sets is simpler and looks better.

### Priority Diagrams for the Site

These are the diagrams that should appear on persona pages:

1. **Context Rot (revised)** → Builders page. The before/after comparison. Simplify: less busy, more dramatic. The "without" side should feel uncomfortable (thin, fading lines). The "with" side should feel warm (growing, connected lines).

2. **Throughline Layers (revised)** → Main landing + Builders page. The vision → milestones → tickets → sessions cascade with feedback loops. Make the feedback arrows more prominent — they're the key insight.

3. **Session Continuity Cycle (revised)** → Builders page. The warmup → work → save loop. Emphasize the vault at center — it's the persistent layer.

4. **Sync & Collaboration (new)** → Teams page. Multiple machines/agents connecting to a shared vault. Show the dual-authority model (git + Postgres) in a way that's visually intuitive.

5. **Agent Integration (new)** → Agents page. An agent reading from and writing to the vault. Show the MCP pathway alongside the CLI pathway.

6. **Scope Routing (revised)** → Builders page. The patch/feature/epic branching. This one works conceptually; just needs the visual treatment updated.

### Design Language for Site SVGs

- **No boxes for boxes' sake.** The current diagrams are box-heavy. For the site versions, use more open compositions — text with connecting lines, spatial relationships conveyed through position rather than containment.
- **The throughline as a literal visual element.** A continuous line or thread that weaves through diagrams, connecting concepts. This becomes a recognizable motif across all diagrams.
- **Animation potential.** The site runs SvelteKit — the SVGs can be animated. The throughline thread could draw itself on scroll. Session nodes could pulse as they're "warmed up." The context rot diagram could animate the degradation vs. compounding.

---

## Part 6: Immediate Next Steps

### Deliverables (prioritized)

1. **Brand mark / favicon** — The threaded-t icon in SVG, at 32x32 and 64x64 sizes. Replaces the current placeholder.

2. **GitHub social preview image** — 1280x640 with the brand mark, wordmark, and tagline on dark ground. This is what people see when the repo is shared.

3. **Revised landing page copy** — Tighten the hero, sharpen the tagline for meta/social, refine section headings. The copy is already good; this is polish.

4. **Builders page** — Full page with tailored hero, problem framing, feature showcase, and revised dark-mode diagrams.

5. **Agents page** — Full page with MCP-focused narrative and agent integration diagrams.

6. **Teams page** — Full page with sync/collaboration narrative and new diagrams.

7. **Diagram refresh** — Dark-mode versions of the key SVGs for use on the site.

8. **README refresh** — Updated to reflect the current state of the product, with the brand mark header and sharpened copy.
