# Frontmatter Reference

> **Generated from `temper_workflow`'s embedded JSON Schemas — the same ones the server validates
> against.** Do not hand-edit: run the emit command and commit the result. It exists in generated
> form for one reason. Its hand-written predecessor described a set of primitives temper had
> already retired, and nothing could notice, because a document describing types is not connected
> to the types. Derived from the validator's own source, this one cannot name a doc type that does
> not exist, nor omit one that does.

A resource's frontmatter has **two tiers**, and the distinction is enforced on write:

| Tier | Vocabulary | Rejects unknown keys? |
|------|-----------|----------------------|
| `managed_meta` | **Closed** — the `temper-*` workflow/provenance keys below | **Yes.** An unknown key is an error |
| `open_meta` | **Open** — anything you like | No. Some keys are *recognized* (below); the rest are stored as-is |

Identity is **not** metadata. `title`, the doc type, and the resource's home (a context or a
cognitive map) are first-class fields on the write call, never `managed_meta` keys. The **slug is
derived from the title** on every surface — there is no way to set one, and a slug placed in
frontmatter is inert.

## Doc types

These 14 are the complete set. A `doc_type_name` outside it is rejected.

- `commitment`
- `concept`
- `concern`
- `decision`
- `domain`
- `fact`
- `goal`
- `memory`
- `principle`
- `question`
- `research`
- `session`
- `task`
- `theme`

## `managed_meta` by doc type

The type-specific half of the closed vocabulary. A key listed against one type is accepted on that type; sending it on another is a validation error, not a silent no-op.

| Doc type | Keys | Must the caller send one? |
|---|---|---|
| `commitment` | *(none beyond the universal set)* | no |
| `concept` | *(none beyond the universal set)* | no |
| `concern` | *(none beyond the universal set)* | no |
| `decision` | *(none beyond the universal set)* | no |
| `domain` | *(none beyond the universal set)* | no |
| `fact` | *(none beyond the universal set)* | no |
| `goal` | `temper-seq` — Ordering within context (integer)<br>`temper-status` — Goal lifecycle status (one of: `active`, `completed`, `paused`, `cancelled`) | no |
| `memory` | *(none beyond the universal set)* | no |
| `principle` | *(none beyond the universal set)* | no |
| `question` | *(none beyond the universal set)* | no |
| `research` | *(none beyond the universal set)* | no |
| `session` | *(none beyond the universal set)* | no |
| `task` | `temper-branch` — Git branch name<br>`temper-effort` — Work size estimate (one of: `small`, `medium`, `large`)<br>`temper-mode` — Work type (one of: `plan`, `build`)<br>`temper-pr` — Pull request URL or identifier<br>`temper-seq` — Ordering within goal (integer)<br>`temper-stage` — Task workflow stage (one of: `backlog`, `in-progress`, `done`, `cancelled`) | `temper-stage` — optional, defaults to `backlog` |
| `theme` | *(none beyond the universal set)* | no |

## Universal `managed_meta` keys

Accepted on every doc type.

- `temper-llm-model` — Model that produced this resource
- `temper-llm-run` — UUIDv7 of the graph-index run that created this resource
- `temper-provenance` — How this resource was created (one of: `llm-discovered`, `user-created`)

## Server-managed fields — never send these

Stamped by the server. They appear on every resource you read and are rejected (or ignored) on write.

- `temper-id`
- `temper-provisional-id`
- `temper-type`
- `temper-context`
- `temper-owner`
- `temper-created`
- `temper-updated`
- `temper-source`
- `temper-legacy-id`
- `temper-slug`

## `open_meta` — the open tier

Any key is accepted and stored. The keys below are *recognized*: they carry a declared shape, and some are indexed for search. An unrecognized key is neither an error nor indexed.

| Key | Shape | Notes |
|---|---|---|
| `keywords` | string or array | FTS-indexed at weight C (convention v1). Deliberately-attached topical tags that boost search ranking. A JSON array of strings (space-joined into the vector) or a bare JSON string. Synonymous with `tags` for ranking. |
| `tags` | string or array | FTS-indexed at weight C (convention v2). The everyday topical-tag key; ranks identically to `keywords`, and accepts the same shapes: a JSON array of strings (space-joined into the vector) or a bare JSON string. A bare string is ONE tag and is stored as a one-element array: `"concept design"` becomes `["concept design"]`, filterable as `concept design` and NOT as `concept` (ruled 2026-08-15, migration 20260815000030). Write the array form when you mean several tags. |
| `descriptor` | string | FTS-indexed at weight D (convention v1). The full section descriptor, for corpora where importers truncate it out of the title under length pressure — keeps the discriminating words searchable. A JSON string. |
| `date` | string | Shape-convention (not FTS-indexed). ISO-8601 calendar date, YYYY-MM-DD. The most common open_meta key in production. |
| `relates_to` | array | Shape-convention (not indexed). Soft relationship to other resources (UUIDs, slugs, or refs). Parallel to the hard edge model. |
| `derived_from` | string or array | Shape-convention (not indexed). Source resources this was derived from. |
| `preceded_by` | string or array | Shape-convention (not indexed). Resources that precede this in sequence. |
| `references` | array | Shape-convention (not indexed). Referenced resources or external URIs. |
| `depends_on` | array | Shape-convention (not indexed). Dependencies (UUIDs, slugs, or refs). |

**Discouraged keys.** Legal, but they shadow a managed field and drift away from it silently:

- `slug` — the canonical value lives in `temper-slug`
- `title` — the canonical value lives in `temper-title`
