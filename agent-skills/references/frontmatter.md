# Frontmatter Reference

Complete YAML frontmatter schemas for each vault primitive type.

## Ticket

```yaml
---
id: "019d1d24-2000-738c-bf4e-834cc33ec611"   # UUIDv7 (time-ordered)
type: ticket
title: "Ground-State Data Quality"             # Human-readable, quoted
slug: "2026-03-24-ground-state-data-quality"   # date-prefixed kebab-case
project: "storyteller"                         # matches [projects.*] key in temper.toml
milestone: "tier-betwixt-grammar-to-vocabulary" # milestone slug, or null
stage: backlog                                  # backlog | in-progress | done | cancelled
scope: epic                                     # patch | feature | epic | null
seq: 20                                         # ordering integer
created: 2026-03-24T07:08:27.300279-04:00      # ISO 8601 with timezone
updated: 2026-03-24T08:09:44.989237-04:00      # ISO 8601 with timezone
branch: null                                    # git branch name, or null
pr: null                                        # PR URL, or null
---
```

**File location**: `tickets/{project}/{slug}.md`

**Stage transitions**: `backlog` → `in-progress` → `done` or `cancelled`

**Body structure**: Free-form markdown. Typically includes Summary, scope description,
context links, and acceptance criteria for feature/epic tickets. Patch tickets can be
as brief as a single paragraph.

## Milestone

```yaml
---
id: "019d20f9-6a92-7532-bc67-deb3924ece06"
type: milestone
title: "Tier Betwixt: Grammar to Vocabulary"
slug: "tier-betwixt-grammar-to-vocabulary"      # kebab-case, no date prefix
project: "storyteller"
seq: 290                                         # ordering integer
status: active                                   # active | completed | paused
created: 2026-03-23                              # date only (YYYY-MM-DD)
---
```

**File location**: `milestones/{project}/{slug}.md`

**Body structure**: Should include:
- Why this milestone exists (motivation, what gap it fills)
- Scope (what's in, what's out, prerequisites)
- Sequencing (ordered list of work chunks)
- Relationship to other milestones/tiers
- Status narrative (where things stand)

## Session

```yaml
---
id: "019d1d24-2000-7379-8f26-ae4ae87bc5c6"
type: session
date: 2026-03-24                                # date only
project: storyteller                            # unquoted is fine for simple strings
cluster: ""                                     # optional grouping tag
---
```

**File location**: `sessions/{project}/{date} — {title}.md`

Note the em dash (—) in the filename, not a hyphen. If multiple sessions occur on the
same date, they each get distinct titles.

**Body structure** (standard template):

```markdown
# Session: {title}

## Goal
What this session set out to accomplish.

## What happened
What was attempted, what worked, what didn't.

## Decisions
Significant choices made and why (alternatives considered).

## What connected
Concepts, patterns, or cross-project links noticed.

## To pick up
Next steps, open threads, things to investigate.
```

## Concept

```yaml
---
type: concept
description: ""                                 # one-line summary
aliases: []                                     # alternative names
tags: []                                        # categorization tags
sources: []                                     # source document references
related: []                                     # related concept slugs
created: 2026-03-24
updated: 2026-03-24
---
```

**File location**: `concepts/{slug}.md`

**Body structure**:

```markdown
# {title}

Brief description of the concept and why it matters across the work.

## Threads
Where this concept appears and how it manifests in each context.

## Open Questions
Things we've wondered about or haven't resolved yet.
```

## Research

```yaml
---
id: "019d1d24-2000-7379-8f26-ae4ae87bc5c6"
type: research
date: 2026-03-24
project: "storyteller"
title: "Knowledge Graph Query Patterns"
slug: "knowledge-graph-query-patterns"
---
```

**File location**: `sessions/{project}/` or a dedicated research directory

**Body structure**:

```markdown
# {title}

## Topic
What question or area is being investigated.

## Findings
Key discoveries, data points, and conclusions.

## Sources
References, links, documentation consulted.

## Implications
How this affects current or planned work.

## Open Questions
What remains unknown or needs further investigation.
```

## Source

```yaml
---
type: source
path:                                           # filesystem path to the source document
project:                                        # which project owns this source
tags: []
---
```

**File location**: `sources/{slug}.md`

Lightweight stubs that point to documents in other repositories. The body is typically
a one-line description plus a list of key concepts found in the source.

## Slug Derivation Rules

- **Tickets**: `{YYYY-MM-DD}-{kebab-cased-title}` — date prefix from creation date
- **Milestones**: `{kebab-cased-title}` — no date prefix
- **Sessions**: filename uses `{YYYY-MM-DD} — {title}.md` (em dash, spaces around it)
- **Concepts**: `{kebab-cased-title}` — no date prefix
- **Research**: `{kebab-cased-title}` — no date prefix

Kebab-case rules: lowercase, replace spaces with hyphens, strip special characters
(colons, quotes, parentheses), collapse multiple hyphens.

## ID Generation

IDs use UUIDv7 format (RFC 9562), which embeds a timestamp for natural ordering. When
generating IDs programmatically isn't possible, use a placeholder or omit the `id` field —
it's used for deduplication and ordering but isn't required for the vault to function.
