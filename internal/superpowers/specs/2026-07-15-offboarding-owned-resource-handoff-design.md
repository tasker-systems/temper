# Offboarding: departed-member owned-resource handoff (surface residual reach on removal)

- **Date:** 2026-07-15
- **Status:** Design — draft for review
- **Goal:** Teams in Temper: usable multi-user collaboration surface (temper resource `019f25d6-e1a9-7360-8a35-6bdf8ef53940`)
- **Task:** Offboarding: departed-member owned-resource handoff — Lever B on team removal (`019f6786-196c-7441-9bbb-3a08c9eb3e63`)
- **Fast-follow to:** [Context transfer safety + residual-access (T-D)](2026-07-15-context-transfer-safety-residual-access-design.md). That spec named this as the one genuine residual-access case a container transfer legitimately does not cover, and spun it out.

## Motivation

The transfer-safety spec closed the container-move story. It also named a distinct
residual-access case it deliberately did not address: when a member *leaves* a team, the
resources they still **own** in that team's contexts keep `owner_profile_id` pointing at
them. After **D1** (originator demoted from the access predicates, shipped today in
migration `20260715000040`), `owner_profile_id` is the single access-bearing key — so a
departed member retains genuine read **and** write on every resource they still own in a
team context, until ownership is handed off. That handoff is **Lever B** (ownership
reassignment), applied on offboarding.

### Grounding correction (verified against `main`, 2026-07-15)

The task's framing — and the transfer-safety spec's grounding note — assumed the bulk
handoff was unbuilt (*"no such function exists — a bulk owner move would be new build"*).
**That is stale.** Verified on `main` (commits `4052785a` + `424130b0`, 2026-07-03):
Lever B already exists end-to-end, in both single and bulk form.

| Layer | Single handoff | Bulk offboarding handoff |
|---|---|---|
| Service (`reassign_service.rs`) | `reassign_resource` — auth: current owner **or** team-admin reach; cogmap-homed rejected; idempotent no-op to current owner | `reassign_team_resources` — scope: *owned by `from` ∩ homed in a context shared to the team*; auth: caller manages the team **and** `to` is a member; soft-deleted team is inert; one transaction; per-resource reassign events |
| API | `POST /api/resources/{id}/reassign` (`reassign_resource`) | `POST /api/teams/{id}/reassign` (`reassign_team`) |
| Client (`temper-client`) | `resources.reassign` | `teams.reassign` |
| CLI | `temper resource reassign <ref> --to <uuid>` | `temper team reassign <team> --from <uuid> --to <uuid>` (help text: "offboarding") |
| Tests | present | `bulk_reassigns_only_owned_and_scoped`, `bulk_non_manager_forbidden`, `bulk_into_non_member_forbidden`, `bulk_forbidden_on_soft_deleted_team`, `bulk_empty_match_is_ok` |

So the **mechanism** the task asks us to design is already built and tested. Combined with
D1, the departed-owner residual-access worry is *already resolved* — **provided an admin
actually runs the handoff.**

### The real gap

`remove_member` (`team_service.rs:386`) does nothing about owned resources. It performs the
auth + last-owner-guarded `DELETE` and returns `()` (API: `204 No Content`). There is **no
detection, no count, no warning.** The removal path and the handoff path are entirely
disjoint. An admin who removes a member without separately remembering to run
`team reassign` silently leaves that member as an *external owner* holding read + write on
their resources — the exact residual-access hole, now invisible.

The gap is not a missing mechanism. It is a missing **signal**: nothing tells the admin the
handoff is needed.

## Decision: surface residual reach on removal, do not sweep

`remove_member` reports the residual owned-resource reach the just-removed member retains,
so the admin can run the existing handoff deliberately. It does **not** auto-reassign.

This is the direct sibling of transfer-safety's **D3** ("surface residual reach, don't
sweep") and follows the same principle that governs this whole area: *who inherits a
departed member's work is a reach judgment the admin must make deliberately* — friction that
declares is a feature, not a step to automate away. Auto-reassign was considered and
rejected on exactly that ground; a hard pre-check (block removal until reassigned) was
rejected as over-constraining a previously-simple operation.

## Design

### Component 1 — extract the scope read (DRY)

The scope query is currently inline in `reassign_team_resources` (`reassign_service.rs:174`):
resources owned by a profile **and** homed in a context shared to the team. Extract it into
one read in `reassign_service`:

```rust
/// Resources owned by `profile` and homed in a context shared to `team`.
/// The single definition of "what a departing member still owns in this team",
/// shared by the bulk handoff (the move set) and remove_member's surfacing (the count).
async fn team_scoped_owned(pool, team_id, profile_id) -> ApiResult<Vec<ScopedOwnedRow>>
```

`ScopedOwnedRow` carries `resource_id` and `context_id` (+ enough to render a context ref).
`reassign_team_resources` consumes the ids for its move set; `remove_member`'s surfacing
groups by context for the count + breakdown. **One query**, so the definition of the
residual set can never drift between the warning and the handoff it recommends. (Fundamentals:
extract shared predicate sets; never two copies that will drift.)

### Component 2 — `remove_member` returns the residual reach

After the existing auth + `DELETE` succeed, run `team_scoped_owned` for the removed profile
(read-only — auth-before-writes ordering is untouched; the query is membership-independent,
so it computes correctly *after* the row is deleted). Return a typed outcome:

```rust
pub struct RemoveMemberOutcome {
    pub residual_owned: ResidualOwnedReach,
}
pub struct ResidualOwnedReach {
    pub count: usize,                    // total resources still owned in team contexts
    pub contexts: Vec<ResidualContext>,  // blast-radius breakdown
}
pub struct ResidualContext {
    pub context_ref: String,             // decorated context ref, reusing existing rendering
    pub count: usize,
}
```

Wire types live in `temper-core` (`web-api` + `typescript` derives), per the boundary rule.
`count: 0` is the common, clean case (member owned nothing in the team's contexts).

The last-owner guard already makes a blocked removal a no-op `Err(Conflict)`; the surfacing
read only runs on a removal that actually happened.

### Component 3 — thread through the surfaces (CLI + API only)

- **API** `DELETE /api/teams/{id}/members/{profile_id}`: `204 No Content` → `200 + RemoveMemberOutcome`.
  Additive/skew-safe: the existing `temper-client::teams.remove_member` discards the body
  (returns `Result<()>` today) — it will be widened to return the outcome, and an older
  client that ignores the body is unaffected. No hard-fail across version skew.
- **CLI** `team remove-member`: on `count > 0`, print a nudge —
  *"⚠ profile `<uuid>` still owns N resource(s) in `<contexts>`. Hand them off with:
  `temper team reassign +<team> --from <uuid> --to <member-uuid>`."* On `count == 0`, the
  existing quiet success.
- **MCP**: untouched. Team-member management (`add_member` / `remove_member` / `change_role`)
  is entirely absent from MCP by construction — offboarding is a human-admin CLI+API surface.
  Adding reassign to MCP while member management is MCP-absent would be an inconsistent
  boundary; explicitly out of scope (confirmed).

### Component 4 — wire-contract regen + tests

- **Regen** (per CLAUDE.md): a new response DTO restales the router products. Run
  `cargo make openapi` (openapi.json + temper-rb gem + temper-ts `schema.ts`) and
  `cargo make generate-ts-types` (ts-rs). Stage the output — the drift gates compare against
  git, so a freshly-regenerated-but-unstaged artifact still reds `cargo make check`.
- **Tests** (extend `reassign_service` / `team_service` suites, `#[sqlx::test]`):
  - **Surfacing correctness** — after removing a member who owns 3 resources across 2
    team-shared contexts, `count == 3` and `contexts` reflects the per-context split;
    resources owned by *other* members and resources in *unshared* contexts are excluded
    (mirrors the existing `bulk_reassigns_only_owned_and_scoped` shape).
  - **Empty case** — removing a member who owns nothing scoped returns `count == 0`.
  - **Differential** — `team_scoped_owned` returns exactly the id set that
    `reassign_team_resources` moves for the same `(team, from)`, so the warning and the
    handoff can never disagree. Assert on the shared read, not hand-written expectations.
  - **Self-leave** — a self-leaving member gets the same outcome shape (surfacing is
    uniform; the residual info is useful signal to a departing owner even though they may
    not be able to reassign).

## Non-goals

- **Auto-reassign on removal** — rejected (surface, don't sweep).
- **Pre-check that blocks removal** — rejected (over-constrains a simple op).
- **Member handle/ref input for `team reassign --from/--to`** — stays raw profile UUIDs;
  handle/ref resolution is a separate ergonomics task, not this one (confirmed).
- **Any MCP surface for reassign or member removal** — consistent with team-management being
  MCP-absent (confirmed).
- **New bulk-owner-normalization semantics** — the existing `reassign_team_resources` scope
  (owned-by-`from` ∩ team-shared-context) is the handoff; unchanged.

## Implementation scope (single PR)

One cohesive PR — the change is small and the narrative is one beat:

1. Extract `team_scoped_owned` in `reassign_service`; refactor `reassign_team_resources` to
   consume it (behavior-preserving — pin with the differential test).
2. Add `RemoveMemberOutcome` / `ResidualOwnedReach` / `ResidualContext` to `temper-core`.
3. `remove_member` returns the outcome; API handler returns `200 + body`; client widens to
   return it; CLI prints the nudge.
4. Wire-contract regen (openapi + gem + ts schema + ts-rs), staged.
5. Tests per Component 4; `cargo make check` + the temper-api integration target
   (backend-command-adjacent change) + sqlx cache regen as needed.

## Open questions / risks

- **Context-ref rendering post-transfer.** A team-shared context may be team-owned or
  still personally-owned-but-shared; the decorated `context_ref` should reuse existing
  context-ref rendering rather than reconstruct it. Confirm the rendering helper at
  implementation time (plan-verification: grep the real API before writing the prompt).
- **`200` vs `204` skew.** Verify `temper-client::teams.remove_member` (and any other
  caller) tolerates `200 + body` where it previously saw `204` — expected fine (status is
  checked for success, body discarded), but pin it.
