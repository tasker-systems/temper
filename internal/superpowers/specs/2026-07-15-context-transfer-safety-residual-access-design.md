# Context transfer safety + residual-access

- **Date:** 2026-07-15
- **Status:** Design — draft for review
- **Goal:** Teams in Temper: usable multi-user collaboration surface (temper resource `019f25d6-e1a9-7360-8a35-6bdf8ef53940`)
- **Task:** Transfer safety + residual-access design (`019f6399-3c96-7273-97a7-53397682c881`)
- **Fast-follow to:** [Context ownership transfer (T-A)](2026-07-15-context-ownership-transfer-design.md) — shipped in v0.2.5. T-A moved the container and documented these items as non-goals; this spec decides whether/how to close them.

## Motivation

T-A shipped the container move: a personal context can be transferred to a team, after
which the team's authoring members can write into its resources. It deliberately deferred
the *safety and completeness* story — what happens to the individual pieces (resource
ownership, prior shares, provenance) when the container changes hands. This spec resolves
those questions and, in doing so, corrects a conflation in the original framing.

### Grounding correction (verified against the live schema, 2026-07-15)

The task's research snapshot described an earlier generation of the access model. Verified
current reality:

| Task's prose | Verified reality |
|---|---|
| access via `kb_resources.originator_profile_id` | access flows through **`kb_resource_homes`** (`owner_profile_id` / `originator_profile_id`, both `NOT NULL`) + `contexts_readable_by` + `context_authorable_by_profile` |
| "reuse `reassign_team_resources`" (Q3) | **no such function exists** — only single-resource `resource_reassign`; a bulk owner move would be new build |
| context write-grant "backdoor" is a live risk | the predicate arm exists in `context_authorable_by_profile`, but **no surface mints one** — a guardrail against future wiring, not an open hole |

The concrete residual-access surface after a transfer, verified live:

- **Per-resource homes persist untouched.** `context_reassign` flips only
  `kb_contexts.(owner_table, owner_id)`; it never touches `kb_resource_homes`. Every
  resource keeps `owner_profile_id = ` the original author.
- **Pre-existing context reach survives the ownership flip:** `kb_team_contexts` shares
  (`contexts_readable_by` arm 3) and explicit context read-grants (`kb_access_grants`, arm
  4) both carry over to the new owning team.

## The spine: two levers

The residual-access worry dissolves once two distinct actions are named separately, rather
than collapsed into "transfer":

- **Lever A — Context homing transfer** *(T-A, shipped)*. Moves
  `kb_contexts.(owner_table, owner_id)` to a team. It does **not** touch resource
  `owner_profile_id`. It is **owner-safe by construction**: the gate (`can_share` and the
  `context_reassign` invariant both require **owner/maintainer on the target team AND
  administering the context** — verified in `context_service::can_share`,
  `context_service.rs:369`, and the SQL `context_reassign` guard) guarantees the actor is a
  *continuing, authorized* team member. Resources stay owned by profiles who retain
  legitimate access, and the team gains access through the container-write cascade
  (`context_authorable_by_profile` team arm).
- **Lever B — Ownership handoff to another profile** *(`resource_reassign`, exists)*. The
  distinct action that moves `owner_profile_id`. Offboarding a *departed* member's owned
  resources is a Lever-B concern — **not transfer's job** — and is spun out as its own task.

Because the actor performing a transfer is always a continuing team member, no legitimate
access is lost and no "departed author" is created by the act of transferring. The
departed-author case arises only *later*, on offboarding, and is addressed there.

## Decisions

### D1 — Demote `originator_profile_id` from access (Q1)

**Decision.** `CREATE OR REPLACE` `resources_visible_to` and `can_modify_resource`,
removing the `OR h.originator_profile_id = p_profile` arm from each. Access becomes
`owner_profile_id` + explicit grants + container cascade. `originator_profile_id` remains a
recorded, returned provenance fact — it simply stops conferring access.

**Rationale.** One responsibility per key: `owner` = access, `originator` = provenance. The
current conflation makes `resource_reassign` a *half*-handoff — it moves `owner_profile_id`
but the original creator keeps access as originator, which is a latent bug. Demoting
originator makes reassign a true handoff.

**Safety.** Verified live: `owner_profile_id` is `NOT NULL` (dropping the originator arm can
never orphan a resource — the owner arm is always populated), and `owner_profile_id` ↔
`originator_profile_id` **diverge in 0 rows today** (they split only after a
`resource_reassign`). So the change is **behavior-preserving on all current data** and
skew-safe (no deployed binary observes a behavior change). Global blast radius, pinned by
the differential test below.

**Signal preserved.** This is not zeroing a signal that "nobody uses today." Originator
attribution is still recorded and returned; we un-overload it, we don't discard it.

### D2 — Transfer performs no owner reassign (Q3)

**Decision.** Transfer stays a pure container move; it does **not** bulk-reassign resource
`owner_profile_id`. This is the direct consequence of the two-lever model: a resource owner
is a `NOT NULL` FK to `kb_profiles` (a team can never be a resource owner), and the
transfer is owner-safe by construction. Documented boundary, not built.

**If bulk owner-reassign is ever wanted** (e.g. to normalize ownership onto the team's
principal), the natural target is the transferring actor — already an owner/maintainer on
the target team — so no arbitrary choice arises. Out of scope here; it is Lever B applied in
bulk, and belongs with the offboarding spin-out if it lands at all.

### D3 — Surface residual reach, don't sweep (Q2)

**Decision.** Transfer succeeds and leaves prior shares/grants intact, but **reports** the
reach the new owner just inherited so they can prune deliberately. A visibility affordance,
not a policy change — transfer does not become a compound action, and no existing consumer
is silently broken.

**Shape.** Extend `ReassignContextOutcome` (`temper-core/src/types/context.rs:124`) with two
**additive** fields:

- `inherited_shares: Vec<TeamRef>` — teams the context is shared to via `kb_team_contexts`.
- `inherited_read_grants: Vec<ContextReadGrantRef>` — subjects holding an explicit
  `kb_access_grants` context read-grant (profile- or team-anchored).

A new `context_service` read gathers both for the transferred `context_id`. Threaded through
API handler → `temper-client` → CLI (`temper context transfer` output) → MCP
(`transfer_context` result). Additive-only, so skew-safe (no hard-fail across version skew).

**Wire-contract regen (per CLAUDE.md).** New DTO fields restale three committed artifacts —
regenerate all via `cargo make openapi` (`openapi.json` + temper-rb gem +
temper-ts `schema.ts`) and `cargo make generate-ts-types` (ts-rs). Stage the regenerated
output; the drift gates compare against git.

### D4 — Context write-grant backdoor stays a guardrail (locked decision, T-A)

**Decision.** Retain, do not wire. `context_authorable_by_profile` includes a
`profile_explicit_grant(p_profile, 'write', 'kb_contexts', ...)` arm — verified live. It is
kept because a grant is a deliberate act of delegation. But **no CLI/MCP/API surface may
mint a write-grant on a *personal* context**; verified, none does today. Exposing one would
be a backdoor around "shared authorship requires team ownership." Documented prominently so a
future contributor does not add it as a "convenience." Shared authorship flows only through
team ownership (Lever A).

### D5 — Cogmap boundary out of scope (Q4)

`kb_team_cogmaps` is a separate store; cogmap-homed resources do not reassign through the
context path. Confirmed a boundary, documented. No work.

### D6 — No ingest/transfer transactional coupling (Q5)

Transfer is a single-row `UPDATE` on `kb_contexts`. A concurrent in-progress ingest writes
`kb_resources` + `kb_resource_homes` anchored to the *unchanged* `context_id`, so its
resources land in the now-team-owned context and inherit team access through the cascade;
`owner_profile_id` is the ingesting author (a continuing team member). `ingest_state`
(`complete`/`in_progress`) never interacts with ownership. No coupling needed — documented
with this reasoning.

## Spin-out task

**Offboarding: departed-member owned-resource handoff** — Lever B applied when a member is
removed from a team. This is the genuine residual-access case transfer legitimately does not
cover: after D1, a resource's access is `owner_profile_id`, so a departed member retains
access to resources they still *own* in a team context until ownership is handed off. New
backlog task; not in this spec's scope.

## Testing

- **Differential** (assert new-path == old-path): the D1 predicates over non-divergent data
  produce the identical visible/modifiable sets as the current predicates. Assert on the
  live corpus shape, not hand-written expectations.
- **Divergence fixture:** `resource_reassign` a resource to a new owner, then assert the
  *originator* loses both read (`resources_visible_to`) and write (`can_modify_resource`),
  and the new owner holds both. This is the behavior D1 changes; it must be pinned.
- **Adversarial authz** in `BEGIN/ROLLBACK`: probe the live predicates directly (not the
  migration text), covering the personal-context, team-owned, and grant arms.
- **E2e:** a transfer with a pre-existing share and a pre-existing context read-grant
  returns both in `inherited_shares` / `inherited_read_grants`; a transfer with neither
  returns empty vectors.

## Implementation scope (PR breakdown)

The task notes it "may spawn build follow-ups." Two PRs:

- **PR1 — D1 originator demotion.** Additive migration (`CREATE OR REPLACE` both
  predicates), differential + divergence tests, sqlx cache regen. Foundational, standalone,
  behavior-preserving on current data.
  - *Plan-time check:* grep for any existing unit test asserting *originator-grants-access*
    before landing — D1's blast radius is global.
- **PR2 — D3 surface residual reach.** `ReassignContextOutcome` extension + the residual-reach
  read + surface threading (API/client/CLI/MCP) + wire-contract regen + e2e.
- **Docs.** The two-lever model and the D2/D4/D5/D6 boundaries fold into this spec and the
  content field guide; the D4 guardrail gets a prominent note.
- **Spin-out.** Create the offboarding / owned-resource-handoff backlog task.

## Non-goals

- Bulk resource-owner reassignment on transfer (D2).
- Sweeping/revoking prior shares or grants on transfer (D3 — surface only).
- Any context write-grant mint surface on a personal context (D4).
- Cogmap-homed resource reassignment (D5).
- Departed-member offboarding handoff (spun out).
