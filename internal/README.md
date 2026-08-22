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

## Attributing a decision to a person

**Do not stamp a claim with someone's name unless you can point at the record.**

`[provisional — <date>, judgement call]` is the default and correct tag for a design choice an
agent made while writing. It says the choice is real, dated, and **re-litigable** — which is what
almost every choice in these documents actually is.

`[ruled — <date>, <name>]` is a much stronger claim: *this person decided this, and it is not yours
to reopen.* Use it only when a durable record exists and the tag cites it — a decision resource, a
session note quoting them, a linked conversation. A verbatim quotation is its own citation and needs
no verb: `` `[Pete — 2026-08-21]` *"…their words…"* ``.

### Why this is written down

`[provisional — 2026-08-22, judgement call]` On 2026-08-22, 125 tags across 26 files in this
directory attributed decisions to Pete that he had not made. They were agent-authored constraints,
stamped with his name in the same commit that wrote them, with no record behind any of them.

The cost is not tidiness. One of them — spec §8.2, *"never a percentage, bar, meter, ratio or 0–100
scale"* — was **quoted back at him as his own ruling** when he asked for a surface to be changed,
and used to argue against a change he had requested twice. An agent citing it had no way to tell it
from a real ruling, and neither did he.

So the whole set was converted to `provisional`. That loses nothing: a genuine ruling among them can
be restored **with its record attached**, and anything else can now be argued on its merits instead
of on someone's authority. Restoring a bare name-stamp without a citation is the failure this note
exists to prevent.

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
