---
name: temper-vault
description: >
  Work with temper-compatible knowledge vaults — markdown files with YAML frontmatter organized
  into sessions, tickets, milestones, concepts, and research notes by project. Use this skill
  whenever the user mentions a knowledge vault, knowledge base, temper vault, session notes,
  ticket management, milestone roadmapping, epic planning, or wants to organize development
  work using markdown-based project tracking. Also trigger when the user references vault
  primitives (tickets, milestones, sessions, concepts, sources) or asks about scope-based
  workflow routing (patch/feature/epic). This skill does NOT require the temper CLI — it works
  directly with the vault's file structure and frontmatter conventions.
---

# Temper Vault — Knowledge Base Workflow

This skill teaches you how to work with a temper-compatible knowledge vault: a directory of
markdown files with YAML frontmatter, organized by project, that tracks development work
through sessions, tickets, milestones, concepts, and research notes.

The vault is the source of truth for strategic context. You read and write markdown files
directly — no CLI or database required. Everything is human-readable, version-controllable,
and portable.

**Important distinction**: The vault holds the *strategic layer* — milestones, tickets,
sessions, concepts. Design specs, implementation plans, and code live in the *project
repositories* themselves (often under `docs/superpowers/specs/` and `docs/superpowers/plans/`).
The vault tracks what work exists, why it matters, and what happened. The project repo
tracks how it gets built.

## Finding the Vault

The vault root contains a `temper.toml` configuration file. To locate it:

1. If the user tells you where it is, use that path
2. Look for `temper.toml` in the current working directory or its parents
3. Check common locations: `~/projects/knowledge`, `~/knowledge`, `~/vault`

Read `temper.toml` to understand which projects are configured, what directories to use,
and how the vault is organized. The `[projects.*]` sections define the project names and
paths you'll work with.

**Default directory layout** (these are configurable in `temper.toml` under `[vault]`):

```
vault-root/
├── temper.toml        — vault configuration
├── sessions/          — session notes, organized by project
│   ├── {project}/
│   └── general/       — cross-project sessions
├── tickets/           — tickets, organized by project
│   └── {project}/
├── milestones/        — milestone definitions, organized by project
│   └── {project}/
├── concepts/          — concept notes (cross-project ideas and threads)
├── sources/           — source index stubs pointing to documents elsewhere
├── templates/         — note templates
├── docs/              — design documents and plans
└── .temper/           — index artifacts (managed by temper CLI, don't edit)
```

## Vault Primitives

Each primitive is a markdown file with YAML frontmatter. The `type` field identifies
what kind of note it is. Read `references/frontmatter.md` for the complete field reference.

### Tickets

Tickets are units of work. They live in `tickets/{project}/` and track through four stages:
`backlog` → `in-progress` → `done` (or `cancelled`).

**Filename convention**: `{date}-{slug}.md` where date is `YYYY-MM-DD` and slug is the
kebab-cased title. Example: `2026-03-24-ground-state-data-quality.md`

**Scope** controls the workflow applied to a ticket:

| Scope | Nature | What happens |
|-------|--------|--------------|
| `patch` | Tactical fix | Implement directly, no ceremony |
| `feature` | Deliberate build | Design → plan → implement |
| `epic` | Strategic planning | Map problem space → milestone roadmap → first ticket |

When scope is missing, ask the user: "Does this feel like a patch, feature, or epic?"

### Milestones

Milestones group tickets into strategic chunks. They live in `milestones/{project}/` and
have a `status` field: `active`, `completed`, or `paused`.

A good milestone explains *why it exists*, what it enables, and how it sequences work.
See the Epic Workflow section for how milestones are created.

### Sessions

Session notes capture what happened during a working session. They live in
`sessions/{project}/` (or `sessions/general/` for cross-project work).

**Filename convention**: `{date} — {title}.md`

Session notes follow a standard structure (Goal, What happened, Decisions, What connected,
To pick up) that creates a narrative thread across working sessions. They are *curated
observations*, not raw logs — write them as if a future version of yourself will read them
to pick up where you left off.

### Concepts

Concept notes capture ideas and threads that span projects. They live in `concepts/` and
track where a concept appears, how it manifests, and what open questions remain.

### Research

Research notes are high-fidelity investigations into a specific question. They capture
findings, sources, implications, and open questions.

### Sources

Source index stubs that point to documents in other repositories. Lightweight pointers
with tags for cross-referencing.

## Epic Workflow — The Primary Focus

Epics are strategic. The output is a **milestone roadmap**, not delivered code. Each
subsequent session works one ticket from the roadmap, learns, evolves the roadmap, and
creates the next ticket.

### Starting an Epic

When a ticket has scope `epic` (or the user describes strategic/exploratory work):

1. **Read the ticket** — understand what problem space to map
2. **Discovery** — read related sessions, milestones, concepts, and source code to build
   a picture of the landscape. Favor vault content over assumptions.
3. **Map the problem space** — identify the key dimensions, constraints, dependencies,
   unknowns, and competing concerns. This is brainstorming, not implementation design.
4. **Produce a milestone roadmap** — create a milestone file that includes:
   - A throughline summary (why this work matters, what it enables)
   - Sequenced deliverable chunks with clear boundaries
   - Validation gates between phases
   - Dependencies and prerequisites
   - Open questions that will resolve through doing the work
5. **Create the FIRST actionable ticket** — scoped as `patch` or `feature`, linked to
   the milestone, concrete enough to start immediately
6. **Save a session note** — capture the discovery, decisions, and what to pick up next

### The Epic Philosophy

The roadmap guides session work, not ticket-spread. Don't create 15 tickets upfront.
Create ONE ticket — the next thing to do. After completing it, learn from the experience,
evolve the roadmap if needed, and create the next ticket. This prevents premature
decomposition and lets the roadmap breathe.

Milestones are **living documents**. As sessions reveal new information, update the
milestone: refine the sequencing, resolve open questions, add newly discovered constraints.
The roadmap should reflect your current understanding, not your initial guess.

### What a Good Milestone Looks Like

A milestone born from an epic workflow has these characteristics:

- **Narrative motivation** — not just "what to build" but "why this gap exists now and what
  makes it legible." Often a milestone becomes necessary because previous work succeeded in
  ways that exposed a new layer of complexity.
- **Scope boundaries** — explicit prerequisites (what must be done first), the work itself
  (sequenced chunks), and what it enables downstream.
- **Design questions** — the hard questions that the work will answer. These aren't
  blockers; they're the intellectual terrain the milestone maps.
- **Relationship context** — what other milestones/tiers this depends on or enables.

For example, a milestone might explain: "Tier B produced a grammar of narrative — archetypes,
dynamics, state variables across 30 genres. But grammar without vocabulary produces
aesthetically coherent yet narratively hollow output. This milestone bridges that gap by
designing world-building data structures and generating test worlds."

That framing tells a future session *why* it's doing this work, not just what files to edit.

### Epic Session Rhythm

Each session working under an epic milestone follows a rhythm:

1. Read the milestone and the current ticket
2. Do the work (the ticket is scoped as `patch` or `feature`)
3. Capture what you learned — especially surprises, revised assumptions, new constraints
4. Update the milestone if what you learned changes the plan
5. Create the next ticket (or mark the milestone complete if you're done)
6. Save the session note with specifics

### Mid-Session Drift Detection

Watch for scope mismatch during a session:

- **Patch drifting up**: needs design decisions, touches 3+ files, considering multiple
  approaches → suggest promoting to `feature`
- **Feature drifting up**: needs decomposition into multiple deliverables, spans multiple
  sessions → suggest promoting to `epic`
- **Epic drifting down**: first ticket is obvious, roadmap has only 1-2 items → suggest
  `feature` or just start working

## Writing Vault Files

When creating or updating vault files, follow these conventions:

### Frontmatter

Read `references/frontmatter.md` for exact field schemas. Key rules:

- **IDs**: Use UUIDv7 format (time-ordered). Generate with appropriate tooling or use a
  placeholder like `{{id}}` if generation isn't available.
- **Dates**: ISO 8601. `created` uses full datetime with timezone for tickets, date-only
  for milestones and sessions. `updated` tracks the last modification.
- **Slugs**: Derived from the title — lowercase, kebab-case, no special characters.
  Ticket slugs are prefixed with the creation date: `2026-03-24-my-ticket-title`.
- **Strings with colons or special chars**: Always quote in YAML frontmatter.

### Content Quality

Session notes, milestone descriptions, and ticket bodies should be written with care.
These aren't throwaway notes — they're the institutional memory of the project.

- **Sessions**: Write as curated narrative, not raw transcript. Future sessions will read
  these to pick up context. Include specific details: file paths, function names, data
  counts, branch names, number of commits, test results. A session that says "Fixed the
  loader" is far less useful than one that says "Added `_is_valid_value()` sentinel check
  to `loader.py`, eliminating 22 entities with `'null'` string slugs. Result: 0 sentinel
  slugs in DB." The specificity is what makes these useful across sessions.
- **Sessions — "What connected"**: This section captures cross-project patterns and
  insights. Use `[[wiki-link]]` syntax to reference concepts. This is where the knowledge
  graph grows — don't skip it even when the session was heads-down implementation.
- **Sessions — "To pick up"**: Make these concrete and actionable, not vague aspirations.
  "LLM trope family classification — use Ollama structured generation to classify the 108
  unclassified tropes into canonical families" is actionable. "Continue working on data
  quality" is not.
- **Milestones**: Explain the *why* before the *what*. A milestone that says "Implement
  auth" is less useful than one that explains why auth is needed now, what it enables,
  and how it fits into the larger arc.
- **Tickets**: Include enough context that someone (or a future AI session) starting cold
  can understand what to do and why. For non-trivial tickets, include: a summary, the
  specific issues or requirements, proposed approaches (with tradeoffs), scope boundaries,
  and links to related sessions and files.

### Linking

Use `[[wiki-link]]` syntax for cross-references between vault notes. These are compatible
with Obsidian and create a navigable knowledge graph.

## Session Lifecycle

### Starting a Session

When beginning work:

1. Read recent session notes for the relevant project (`sessions/{project}/`)
2. Check for in-progress tickets (`tickets/{project}/` where `stage: in-progress`)
3. Review the relevant milestone if one exists
4. Orient before acting — understand where the work left off

### During a Session

- Track decisions and their rationale as you go
- Notice connections to other concepts or projects
- If scope drift occurs, flag it rather than silently absorbing complexity

### Ending a Session

Create a session note following the template structure (Goal, What happened, Decisions,
What connected, To pick up). The template is a floor, not a ceiling — add sections like
**Observations** for unexpected findings or pattern recognition that doesn't fit neatly
into the standard structure.

If the session was working a ticket, update the ticket's frontmatter:
- Set `stage` to the new state (`done`, or keep `in-progress` if continuing)
- Update the `updated` timestamp
- Add `branch` and `pr` if applicable

## Customization

### Vault Location

Set the vault path in your workflow. The skill looks for `temper.toml` to identify the root.

### Adding Projects

Add project entries to `temper.toml`:

```toml
[projects.myapp]
repo = "org/myapp"
path = "~/projects/myapp"
```

### Custom Templates

Place custom templates in the vault's `templates/` directory. The skill uses whatever
templates it finds there. At minimum you want: `ticket.md`, `milestone.md`, `session.md`.

### Adapting the Workflow

The scope routing (patch/feature/epic) is a framework, not a cage. If a project doesn't
need the ceremony of scope-based routing, just use tickets and sessions directly. The
vault structure works regardless of how much process you layer on top.
