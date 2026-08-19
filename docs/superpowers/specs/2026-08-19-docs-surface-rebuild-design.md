# Docs surface rebuild — design

`[drafted — 2026-08-19, Pete + session]`

**Status:** approved in design. Governs the `docs/` rebuild on `jct/temper-docs-rebuild`.

This document records *what the documentation surface is, why it is shaped that way, and what
the rebuild requires* — decisions-and-rationale, not page copy. Page copy is downstream; each
page is its own drafting pass. It is a sibling to [`site-ia.md`](../../site-ia.md), which
governs the public **temperkb.io** site; this one governs **`docs/` and what it publishes**.

---

## The problem, measured

Measured on `jct/temper-docs-rebuild` at 2026-08-19:

| | |
|---|---|
| Markdown files under `docs/` | 551 |
| Of those, `docs/superpowers/` | 472 (239 plans, 226 specs, 5 reviews, 1 handoff, 1 spike) |
| Actual documentation surface | ~79 files (26 guides, 12 cognitive-maps, 6 agents, 6 auth, 5 development, 11 loose at root, rest) |
| Index or entry point | none — there is no `docs/README.md` |
| Pages published at `g07jkdagwt.apidog.io` | 550 doc pages, plus ~108 API endpoints and ~316 schemas |

The published site is a faithful mirror of the tree, which is the whole problem: 462 of its
550 doc pages are internal plans and specs, against 26 guides. A reader currently lands on
*Code Review Audit — 2026-03-31*.

### The exposure that makes the ordering non-negotiable

The site is public and unauthenticated (verified by credential-free fetch, HTTP 200, with a
`sitemap.xml` and an `llms.txt` enumerating every page). It serves, among others, the
surfaces-in AuthN/AuthZ/credential-flow audit, the pre-deployment security audit, the security
audit playbook, and the brand strategy.

**The live vector is closed.** The AuthN/AuthZ audit describes an F-0 self-escalation vector;
PR #482 merged 2026-07-18 and closes it, confirmed via `gh`. The pre-deployment audit carries
its own historical-artifact banner. So what is exposed is internal methodology, architecture
reasoning, and an unreleased-work trail — not an open hole. That is why unpublishing is first
in the work order and not an emergency.

---

## Settled decisions

| Dial | Decision |
|---|---|
| **Topology** | Apidog owns docs **and** API reference. temperkb.io stays narrative/marketing; `/using-temper` becomes a link out. |
| **Source of truth** | `docs/` in the repo, synced to Apidog. Docs change through the same review gate as code. |
| **Audiences** | Individual user / agent operator · deployer / self-hoster · API consumer / integrator. **Contributors are not a public audience.** |
| **Register** | Self-contained, with a deliberately thin concepts tier that links up to temperkb.io for depth. |
| **Process docs** | `docs/superpowers/` moves to a sibling directory now; per-file triage is a later session. |
| **Derivation** | Anything describing the *how* of a tool is generated where it can be, audited where it cannot. |

### The invariant

> **Everything in `docs/` is public. Nothing else lives there.**

This is the load-bearing decision. It makes the sync safe *by construction* rather than by
allowlist — no configuration can leak what is not in the tree. An allowlist is what failed
here: nobody chose to publish the security audit, the tree simply contained it.

The invariant's cost is that contributor material must leave `docs/`, which is why that is its
own step in the work order rather than a side effect of the `superpowers/` move.

---

## Structure

The tree is **kind-shaped**; navigation is **audience-shaped**; they are separate dials. This
follows `site-ia.md`'s own model, which separates navigation surface, URL hierarchy, and
entry-point status and insists they are independent — a page can be a top-level URL without
being a front door.

```
docs/
├── index.md              the three doors
├── doors/                thin routers — sequence, not content
│   ├── for-users.md
│   ├── for-operators.md
│   └── for-integrators.md
├── concepts/             authored, thin; links up to temperkb.io for depth
├── playbooks/            authored; cross-persona task guides
└── reference/            GENERATED — never hand-edited
    ├── api/              from openapi.json
    ├── cli/              from the --help tree
    └── config/           from TemperConfig
```

**Why doors are pages and not directories.** A door carries reading *order* — the sequence a
persona should walk. Encoding that as hierarchy would bake one reader's path into the URL of
material that serves three, and would force `reference/cli` to be duplicated or arbitrarily
assigned to one persona. The cognitive-maps set already holds this discipline: cross-reference
by concept, never by ordinal, *because a reader can arrive anywhere*.

**Why `reference/` is one subtree.** It is the only arrangement where the entire generated
surface has a single drift boundary. Scattering generation across audience directories means
one gate per location and no way to assert the set is complete.

### What moves where

| Current | Destination | Why |
|---|---|---|
| `docs/superpowers/` (472) | sibling dir outside `docs/` | process artifacts, never documentation |
| `docs/development/` (5) | outside `docs/` | contributor audience — not public |
| `docs/agents/` (6) | outside `docs/` | contributor audience; reachable via `AGENTS.md` |
| `docs/code-reviews/` (5), `docs/security/` (1), `docs/decisions/` (1), `docs/research/` (3) | outside `docs/` | internal engineering record |
| `docs/guides/` (26) | `docs/playbooks/` + `docs/reference/` | split by kind; several are operator playbooks already |
| `docs/auth/` (6) | `docs/concepts/` + `docs/reference/api/` | contract docs serve integrators |
| 11 loose root files | triage — most are spent design docs | see Deferred |

---

## The derivation layer

Temper already runs both rails this needs, gated through `cargo make check` and CI. The rebuild
rides them; it does not invent a third.

- **generate → commit → drift-gate** — `openapi-check`, `openapi-rb-drift`, `openapi-ts-drift`,
  `ts-rs-drift`, `skills-drift`.
- **audit-with-baseline → fail-on-growth** — `audit-unattributed-decisions.sh`, which is already
  prose hygiene and already asserts *its scan found something* rather than passing vacuously.

| Layer | Source | Kept honest by |
|---|---|---|
| `reference/api` | `openapi.json`, already router-derived | existing `openapi-check` |
| `reference/cli` | `--help` tree, emitted from the real binary | new drift gate, same shape |
| `reference/config` | `TemperConfig` via rustdoc/schema | new drift gate, same shape |
| `playbooks/`, `concepts/`, `doors/` | hand-written | `docs-coverage` (below) |

**The CLI reference must be emitted by running the built binary**, not by parsing source. A
generator that reads clap definitions can agree with itself while disagreeing with what ships.

### `scripts/docs-coverage.py`

Modelled on `scripts/register-coverage.py`, and inheriting its discipline verbatim:

> **It detects; it does not decide.** **Coverage is never inferred from absence.** Every place
> it cannot see is reported as unknown rather than as clean.

Four checks:

1. **Reach** — for each page, which door routes to it. Orphans are *reported and never
   adjudicated*: a page nothing routes to may be perfectly correct (a deep reference page
   reached by search).
2. **Dangling door links** — a door naming a page that does not exist. A factual defect, and
   the **only** thing `--strict` fails on.
3. **Generated-tree integrity** — any hand-edit inside `reference/`.
4. **Parse refusal** — if the tree or the doors will not parse, it says so and refuses to report
   zero, because reporting zero would infer coverage from absence.

An uncovered page is never an error, for the same reason an uncovered clause is not: the page
may be new, or deliberately unrouted. Growth in orphans is the signal; a count is not a verdict.

---

## Work order

1. **Unpublish** — Apidog project settings, `g07jkdagwt`. Pete's action; the only step currently
   leaking. Everything below is reversible.
2. **Move `docs/superpowers/` out** — one mechanical commit. Drops 462 pages from the published
   set on its own.
3. **Move contributor material out** — establishes the invariant.
4. **Enrich `site-ia.md`** — extend it to govern all three surfaces. It currently governs one,
   and the docs surface has never had a governing document.
5. **Build `reference/` generation** — CLI and config join the API on the existing rails, each
   with its drift gate.
6. **Author `doors/`, `concepts/`, `playbooks/`** — the prose pass, once the frame holds still.
7. **Add `docs-coverage`** — once there are doors for it to check.

Steps 2 and 3 are mechanical and fast. Steps 4 and 6 carry the writing and the judgement.

---

## Out of scope

### Rejected

- **A publish allowlist.** Considered and rejected in favour of the invariant. An allowlist is
  configuration that must be right; the invariant is a property of the tree that cannot be got
  wrong by editing a config file. This is the direct lesson of the exposure.
- **Retiring Apidog for a SvelteKit-rendered docs surface.** Weighed during design; rejected
  because it discards a working 108-endpoint / 316-schema reference render for build work that
  serves no reader.
- **Per-file triage of `docs/superpowers/` in this pass.** Explicitly rejected as scope for this
  session — see Deferred.

### Deferred

- **Triage of the 472 process artifacts.** They move first and get triaged later, as their own
  session. Most are spent; a minority are still cited (`site-ia.md` names two 2026-06-18 plans
  as grounding for its middle tier).
- **Triage of the 11 loose root files.** Same pass.
- **The temperkb.io side of the topology change** — `/using-temper` becoming a link out is a
  temper-ui change, not a `docs/` change.

---

## Open questions

Declared, not filed — each needs a decision before the step that depends on it.

1. **Does `docs/cognitive-maps/*.md` retire?** Twelve files, hand-duplicated as twelve Svelte
   pages under `packages/temper-ui/src/routes/(public)/cognitive-maps/`. The UI pages are
   hand-authored Svelte with no build-time pipeline from `docs/` — verified: nothing in the
   UI's config reads `docs/`. Since temperkb.io now owns the explanatory register, keeping both
   means maintaining the same prose in two places by hand, indefinitely. **Blocks step 6.**
2. **How does Apidog ingest?** Unverified. No Apidog configuration exists anywhere in the repo,
   so publishing is configured cloud-side. Published plan entries match the repo's top-level
   plan count exactly (234); published spec entries exceed the repo's by two (228 vs 226), which
   is suggestive of a repo-derived import rather than a live sync — but the slug forms differ
   between the two sides and a reliable comparison was not achieved. **If no live sync exists,
   building it is part of step 5** and this spec's "source of truth" decision is an intent, not
   a description. Resolvable via the Apidog MCP tools once a session can call them.
3. **Where do the moved directories land?** Step 2 says "a sibling directory"; the name is
   unchosen. Candidates: `process/`, `work/`, `internal/`. Note this spec is itself written to
   `docs/superpowers/specs/` and will move with the rest.

---

## Provenance

- **Governing sibling:** [`site-ia.md`](../../site-ia.md) — the temperkb.io public-site IA, whose
  four-trunks/three-doors model and three-independent-dials discipline this document follows.
- **Discipline model:** `scripts/register-coverage.py` — detects-does-not-decide; never infers
  coverage from absence.
- **Generation rails:** `.claude/skills/generated-artifacts/SKILL.md`, and the drift gates in
  `.github/scripts/check-*-drift.sh`.
- **Prose-audit precedent:** `.github/scripts/audit-unattributed-decisions.sh`.
- **Visual companion:** the approach comparison this design was chosen from,
  <https://claude.ai/code/artifact/19e76a90-825c-4a2f-b80e-866b1c6d85f0>.
