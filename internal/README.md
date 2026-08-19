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
for this repo and is overridden here, in `CLAUDE.md`, and in the temper skill's
`fundamentals.md`.

## Stale paths in applied migrations

35 files under `migrations/` cite `docs/superpowers/...` in header comments.
Those citations are **stale and will not be repaired.** An applied migration is
immutable — editing one, even a comment, changes its checksum and fails
`db-migrate`. To resolve such a citation, replace the `docs/` prefix with
`internal/`, or read it out of git history.
