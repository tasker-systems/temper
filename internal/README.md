# internal/

Engineering material that is **not** public documentation: process artifacts
(plans, specs, reviews), contributor guides, audits, and generated internal
projections.

## Why this directory exists

`docs/` is synced to the public documentation site. The invariant that keeps
that safe is structural rather than configured:

> **Everything in `docs/` is public. Nothing else lives there.**

An allowlist is configuration that has to be got right every time, and it was
got wrong: internal security audits were published because the tree contained
them, not because anyone chose to publish them. Moving non-public material out
of `docs/` means no configuration mistake can expose it.

See [the design spec](./superpowers/specs/2026-08-19-docs-surface-rebuild-design.md).

## Writing plans and specs

**Specs go in `internal/superpowers/specs/`. Plans go in `internal/superpowers/plans/`.**

The superpowers skills default to `docs/superpowers/...`; that default is wrong
for this repo. It is overridden here and by the two in-repo agent entry points,
`CLAUDE.md` and `AGENTS.md`, which every session reads and which are tracked, so
the override is reviewable and travels with a clone.

The temper skill's own `fundamentals.md` carries the same override, but it is a
machine-local file (`~/.claude/skills/temper/guidance/fundamentals.md`) — it is
not in this repository, no gate protects it, and it has to be re-applied per
machine. Treat the two files above as the durable statement.

## Stale paths in applied migrations

**47** files under `migrations/` cite a `docs/` path that has since moved:
35 cite `docs/superpowers/...`, 9 cite `docs/code-reviews/...`, 1 cites
`docs/research/...`, and 2 cite `docs/search-open-meta-indexing.md` (measured, not
estimated: `grep -rlE 'docs/(superpowers|code-reviews|research)/|docs/search-open-meta-indexing\.md' migrations/`).

Those citations are **stale and will not be repaired.** An applied migration is
immutable — editing one, even a comment, changes its checksum and fails
`db-migrate`. To resolve such a citation, replace the `docs/` prefix with
`internal/` — which happens to work for all 47, since every cited path moved to
the same-named location under `internal/` — or read it out of git history.
