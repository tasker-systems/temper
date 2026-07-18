# Admin-Event Sink Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make administration event-sourced — a queryable, firewalled admin ledger on `kb_events`, proven end-to-end by the grant chokepoint.

**Architecture:** Admin events ride `kb_events` with a **both-NULL producing anchor** (the cognition firewall — every region producer, the steward delta, and materialize attribution scope by anchor, so NULL-anchored events are structurally invisible to them). Because that firewall also hides admin events from every *reader*, the read path is `kb_events."references"` — a GIN-indexed, never-written column of typed provenance pointers, orthogonal to the anchor. Writers are SQL-resident: each admin act becomes a plpgsql function doing event-append + projection in one transaction, mirroring `facet_set`/`relationship_assert`.

**Tech Stack:** Rust (temper-substrate, temper-services, temper-api, temper-cli, temper-mcp), PostgreSQL 18 + plpgsql, sqlx, axum, rmcp, clap.

**Spec:** [`docs/superpowers/specs/2026-07-16-admin-event-sink-design.md`](../specs/2026-07-16-admin-event-sink-design.md)

**Scope:** Spec §9 steps 1–4. Step 5 (the remaining ~18 acts) is a **separate plan**, written once this plan's pattern exists. This plan delivers a queryable ledger and one proven writer pair.

## Global Constraints

- **Additive-only on `main`.** Every schema change is a forward migration. `main` auto-deploys; a big-bang change is never acceptable.
- **Never edit a shipped migration** — sqlx checksum-locks applied migrations.
- **Migrations use `uuid_generate_v7()`**, never native `uuidv7()` (breaks Neon PG17).
- **`CREATE OR REPLACE FUNCTION` cannot add a parameter.** A new param needs `DROP FUNCTION` + `CREATE`, which is a **write outage across deploy skew**. Get signatures right the first time.
- **The anchor rule:** authority acts are NULL-anchored, always. Never pass a producing anchor to an admin `_event_append`.
- **NULL anchor means "no cognition home", NOT "admin"** — `lens_created` is already in that bucket. Never write a reader that infers admin-ness from anchor nullity. Discriminate by event type or by `references`.
- **Admin payload key ban:** no admin payload may spell a key `resource_id`, `block_id`, `edge_id`, or `owner:{table,id}`. `element_trail_node`/`element_trail_edge` match on payload key shape with no type filter and are gated only by `resources_visible_to` — a violation leaks authority records to any reader of the resource. Use `subject_table`/`subject_id` and carry identity in `references`.
- **Typed structs over `serde_json::json!()`** for anything with a known shape.
- **SQL macros** (`sqlx::query!`) for production queries; regenerate caches after SQL changes (see Task 5).
- **Run `cargo make check` before every commit.**
- **Do not run migrations against prod and do not merge PRs.** Stop at "PR up + CI green + summary".

## Dependencies (not tasks in this plan)

- **`019f6b06-c48f-7a81-a238-cdd6b131f3dc`** — *"Legacy profiles have no emitter entities"* — ✅ **LANDED** (PR #465, merged `fca2ef01`; migration applied and verified in prod 2026-07-16). `gm-anirudh`, `lohjishan` and `anonymous` each carry all four surface emitters; `system` untouched. Task 5's first event has its emitters.
- **`019f6b1b-59ea-7660-b631-3b811aea378d`** — *"`payload_schema` is RED on main and runs in no CI job"* — ✅ **LANDED** (PR #464). Baseline verified green 2026-07-16: `cargo make test-schema` → 110/110. Task 1 is unblocked.

> **Task 1 implementer, read this first.** Both blockers are landed and the baseline is verified
> green. Your regen should touch **only the 4 snapshots carrying `AnchorTable` in their `$defs`**:
> `resource_created`, `relationship_asserted`, `property_asserted`, `region_materialized`. The only
> change in each is **two new enum values** (`kb_connections`, `kb_machine_clients`), plus a
> one-line trailing-comma reflow on `kb_profiles` because it is no longer last — i.e. `4 files
> changed, 12 insertions(+), 4 deletions(-)`. If your regen churns other files or reorders anything,
> the baseline was not actually green — stop and report.
>
> **This tripwire previously said 5 snapshots including `cogmap_seeded`. That was wrong**, and is
> corrected here: `CogmapSeeded` carries no `AnchorRef` field — its only `kb_profiles` mention is a
> *description string* on `ProfileId`, which is not an `AnchorTable` reference. Verified 2026-07-16.
>
> **`cargo nextest run -p temper-substrate` does NOT catch this drift.** `tests/payload_schema.rs` is
> `#![cfg(feature = "scenario-schema")]`, so it is compiled out of an unfeatured run: the snapshots go
> stale silently and red only in CI's Unit job. Task 1 Step 5b regenerates them explicitly. And run it
> **package-scoped** — `cargo make test-schema` does; `--workspace` emits a different schema (see
> CLAUDE.md).

## File Structure

| File | Responsibility |
|---|---|
| `crates/temper-substrate/src/payloads.rs` (modify) | `AnchorTable` gains `Connections`/`MachineClients`; new `EventRef`/`RefRel`/`AdminGrantPayload` types |
| `crates/temper-substrate/src/events.rs` (modify) | `EventKind` variants + `SeedAction` arms for admin acts |
| `crates/temper-substrate/src/replay.rs` (modify) | `kb_access_grants` joins `INPUT_TABLES` |
| `crates/temper-services/src/services/admin_ledger_service.rs` (create) | The read surface: query by subject, query by actor, authz gate |
| `crates/temper-services/src/services/access_service.rs` (modify) | `insert_grant`/`delete_grant` become wrappers over the SQL fns; `delete_grant` gains an actor |
| `crates/temper-api/src/handlers/admin_ledger.rs` (create) | HTTP transport for the read surface |
| `crates/temper-cli/src/commands/admin_ledger.rs` (create) | `temper admin ledger` |
| `crates/temper-mcp/src/tools/admin_ledger.rs` (create) | MCP parity |
| `migrations/20260717000010_admin_event_types.sql` (create) | Event-type seeds + payload schemas; **also NULLs the two stale registry rows** (folds in `019f6b48-a562-7871-a48d-87945a796c7e`) |
| `migrations/20260717000020_admin_ledger_epoch.sql` (create) | The epoch marker |
| `migrations/20260717000030_admin_grant_fns.sql` (create) | `_admin_grant_created` / `_admin_grant_revoked` + projectors |

> **The migration numbers above were RENUMBERED from `20260716…` to `20260717…` (2026-07-16).** All
> three original numbers are now taken on `main` and **applied to prod**: `20260716000010` is
> `steward_ingest_delta_max_event_id`, and `20260716000020` is `backfill_legacy_profile_emitters`
> (PR #465 — this plan's own dependency, which landed into the slot the plan had reserved).
> A version collision is not a merge conflict — it surfaces as **only the DB-backed CI jobs failing
> early on a `_sqlx_migrations_pkey` duplicate**, which reads like a flake and is not; re-running
> does not help. **Re-check the highest applied version before creating any migration here** — more
> may have landed since. `ls migrations/ | tail -3`.

---

### Task 1: The `references` contract — typed shape + `AnchorTable` extension

`kb_events."references"` is `JSONB NOT NULL DEFAULT '[]'`, GIN-indexed (`idx_kb_events_references … USING GIN ("references" jsonb_path_ops)`), documented as `[{rel, target:{kind,id}}]`, and **never written — 0 rows carry it, re-verified against live prod 2026-07-16 at 13,405 events** (the spec quoted 9,835; the ledger grows, the invariant has not moved). The `rel` vocabulary (`supersedes|derived_from|touches`) lives in a **comment, not a CHECK** — extending it needs no migration.

`AnchorTable` already has `Teams` and `Profiles` but lacks the admin subjects.

**Files:**
- Modify: `crates/temper-substrate/src/payloads.rs:31-46` (AnchorTable), and append new types
- Test: `crates/temper-substrate/src/payloads.rs` (inline `#[cfg(test)]` module — follow the file's existing convention)

**Interfaces:**
- Produces: `AnchorTable::{Connections, MachineClients}`; `RefRel::{Supersedes, DerivedFrom, Touches, Subject, Principal}`; `EventRef { rel: RefRel, target: AnchorRef }`; `EventRef::subject(AnchorRef) -> EventRef`; `EventRef::principal(AnchorRef) -> EventRef`.
- Consumes: existing `AnchorRef { table: AnchorTable, id: Uuid }` (`payloads.rs:59-62`).

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)]` module in `crates/temper-substrate/src/payloads.rs`:

```rust
#[test]
fn event_ref_serializes_to_the_documented_references_shape() {
    let team = Uuid::parse_str("019f6055-6aea-7aa2-a133-61552dd3d7e4").unwrap();
    let refs = vec![
        EventRef::subject(AnchorRef { table: AnchorTable::Connections, id: team }),
        EventRef::principal(AnchorRef { table: AnchorTable::Teams, id: team }),
    ];
    let json = serde_json::to_value(&refs).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {"rel": "subject",   "target": {"kind": "kb_connections", "id": team}},
            {"rel": "principal", "target": {"kind": "kb_teams",       "id": team}},
        ]),
        "references must match the column's documented [{{rel, target:{{kind,id}}}}] shape"
    );
    let back: Vec<EventRef> = serde_json::from_value(json).unwrap();
    assert_eq!(back, refs, "references must round-trip");
}

#[test]
fn machine_clients_anchor_serializes_as_the_ddl_spells_it() {
    let j = serde_json::to_value(AnchorTable::MachineClients).unwrap();
    assert_eq!(j, serde_json::json!("kb_machine_clients"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-substrate -E 'test(event_ref_serializes)'`
Expected: FAIL — `cannot find type EventRef`, `no variant Connections`.

- [ ] **Step 3: Write minimal implementation**

Add the two variants to `AnchorTable` (`payloads.rs:31`), keeping the DDL-exact rename convention:

```rust
    #[serde(rename = "kb_connections")]
    Connections,
    #[serde(rename = "kb_machine_clients")]
    MachineClients,
```

Then append the reference types:

```rust
/// The `rel` vocabulary of `kb_events."references"`. The first three are the column's
/// original documented set; `Subject`/`Principal` are the admin-ledger extension (spec
/// 2026-07-16 §5). The vocabulary lives in a column COMMENT, not a CHECK, so this enum is
/// the only enforcement — keep it exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefRel {
    #[serde(rename = "supersedes")]
    Supersedes,
    #[serde(rename = "derived_from")]
    DerivedFrom,
    #[serde(rename = "touches")]
    Touches,
    /// What the act was performed ON (the grant's subject, the machine provisioned).
    #[serde(rename = "subject")]
    Subject,
    /// WHO the act was performed FOR (the team granted, the profile promoted).
    #[serde(rename = "principal")]
    Principal,
}

/// One typed provenance pointer in `kb_events."references"`.
///
/// This is the admin ledger's ONLY read path. Admin events are NULL-anchored (the cognition
/// firewall), which makes them invisible to every anchor-scoped reader — so identity must live
/// here, where the GIN index (`idx_kb_events_references`) can find it and no cognition reader
/// looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    pub rel: RefRel,
    /// Spelled `target` with a `kind` field to match the column's documented shape; `AnchorRef`
    /// serializes its table as `kind` via the wrapper below.
    pub target: RefTarget,
}

/// `AnchorRef`'s wire shape inside `references`: `{kind, id}` rather than `{table, id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefTarget {
    pub kind: AnchorTable,
    pub id: Uuid,
}

impl From<AnchorRef> for RefTarget {
    fn from(a: AnchorRef) -> Self {
        RefTarget { kind: a.table, id: a.id }
    }
}

impl EventRef {
    /// What the act was performed on.
    pub fn subject(target: impl Into<RefTarget>) -> Self {
        EventRef { rel: RefRel::Subject, target: target.into() }
    }
    /// Who the act was performed for.
    pub fn principal(target: impl Into<RefTarget>) -> Self {
        EventRef { rel: RefRel::Principal, target: target.into() }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-substrate -E 'test(event_ref_serializes) or test(machine_clients_anchor)'`
Expected: PASS (2 tests).

- [ ] **Step 5: Guard the exhaustiveness of the extended AnchorTable**

`AnchorTable` is matched elsewhere. Run the full crate to catch non-exhaustive matches:

Run: `cargo nextest run -p temper-substrate`
Expected: PASS. If a `match` on `AnchorTable` fails to compile, add the two arms — do **not** add a `_ =>` catch-all (a new variant must stay a compile error).

- [ ] **Step 5b: Regenerate the payload-schema snapshots — Step 5 CANNOT catch this**

`AnchorTable` derives `schemars::JsonSchema` under `scenario-schema`, so two new variants restale
every snapshot carrying it. **Step 5's unfeatured run does not see this**: `tests/payload_schema.rs`
is `#![cfg(feature = "scenario-schema")]` and compiles out, so the drift is invisible locally and
reds CI's Unit job.

Run: `cargo make test-schema` → expect **FAIL** (`resource_created payload schema drifted`). That
failure is the proof the drift is real.

Then: `UPDATE_SCHEMA=1 cargo make test-schema`, and **inspect the diff against the tripwire above**:

```bash
git diff --stat -- crates/temper-substrate/tests/fixtures/payloads/
# expect exactly: 4 files changed, 12 insertions(+), 4 deletions(-)
```

Re-run `cargo make test-schema` → expect PASS. Stage the regenerated snapshots **with** the code:
they are one change, and the drift gate compares against git, so an unstaged-but-correct regen still
fails `cargo make check`.

- [ ] **Step 6: Commit**

```bash
cargo make check
git add crates/temper-substrate/src/payloads.rs crates/temper-substrate/tests/fixtures/payloads/
git commit -m "feat(admin-ledger): typed references shape + admin AnchorTable variants

kb_events.references has never been written -- 0 of 13,405 events carry it. It is the admin ledger's
read path: NULL-anchored admin events are invisible to every anchor-scoped
reader by design, so identity lives in references where the GIN index finds it
and no cognition reader looks.

The rel vocabulary is a column comment, not a CHECK, so extending it with
subject/principal needs no migration."
```

---

### Task 2: The read service — query by subject, query by actor

The spec's central inversion: **the read path ships before any writer**, so the writers are built against a known query shape rather than a hypothetical one.

**Files:**
- Create: `crates/temper-services/src/services/admin_ledger_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs` (register the module)
- Test: `crates/temper-services/tests/admin_ledger_test.rs`

> **Verified plan/reality notes (2026-07-16):**
> - `ApiError::NotFound` is a **unit variant** (`crates/temper-services/src/error.rs:9`), not
>   `NotFound(String)`. Write `ApiError::NotFound` and `matches!(err, ApiError::NotFound)`.
> - **`crates/temper-services/tests/common/` does not exist.** Every existing test in that
>   directory defines its fixture **inline in its own file** (see `context_read_predicate_test.rs`,
>   which declares `struct Org` at the top). Follow that convention — do **not** invent a shared
>   `common` module.
> - `steward_ingest_delta(p_cogmap uuid, p_watermark uuid)` takes a **cogmap**, not a team
>   (`migrations/20260701000005_steward_ingest_watermark.sql:40`).
>
> **Second pre-dispatch pass (2026-07-16, on branch `jct/admin-ledger-read-service`).** The pass
> above verified the *names* this task calls. It did not verify the *predicate* this task's own
> code proposed, and that is where the defects were. Three blockers and four corrections, all
> folded into the steps below. Recorded because the shape repeats: **the gap was never in the APIs
> the plan borrowed — it was in the SQL the plan wrote itself.**
>
> - **`system_access = 'admin'` is NOT this codebase's admin predicate.** Live
>   `is_system_admin(p_profile_id)` is *owner of the gating team*
>   (`kb_team_members.role='owner'` on `kb_system_settings.gating_team_slug`) — it never reads
>   `kb_profiles.system_access`. Probed on dev, they diverge on the only row present:
>   `handle=system, system_access=admin, (system_access='admin')=t, is_system_admin()=f`.
>   `access_service::is_system_admin` is already **`pub`** (`access_service.rs:43`) — call it.
>   Restating it in SQL was both a second copy of the policy *and* a copy of the wrong policy.
> - **`can_administer_grant` is `access_service.rs:62`**, signature
>   `(pool, caller, subject_table: &str, subject_id: Uuid) -> ApiResult<bool>`; it gates
>   `grant_capability` (`:189`) and `revoke` (`:218`). `pub(crate)` suffices — `admin_ledger_service`
>   is in the same crate.
> - **`AnchorTable` has no `as_str`/`Display`** — its only string form is the serde rename
>   (`payloads.rs:31-50`). `can_administer_grant` takes `&str`. Add `AnchorTable::as_str()` beside
>   the enum; do not round-trip through `serde_json::to_string` (stringly-typed, and it quotes).
> - **The GIN probe shape is CORRECT — verified, and Step 6 as originally written would have
>   denied it.** See Step 6.
> - `grant_created`/`grant_revoked` are **already seeded** (`20260624000003_canonical_seed.sql:51-52`),
>   so the test's `WHERE et.name = 'grant_created'` resolves today. `admin_ledger_opened` does not
>   exist until Task 4 — harmless, `= ANY($1)` simply matches nothing and `ledger_epoch` returns
>   `None`.
> - `kb_events` allows the both-NULL anchor (`CHECK ((producing_anchor_table IS NULL) =
>   (producing_anchor_id IS NULL))`), and `kb_events_append_only` fires `BEFORE DELETE OR UPDATE`
>   only — the seed INSERT is fine.
>
> **And the reason none of that would have been caught: the specced tests could not fail on it.**
> The original three tests are *all* satisfied by an `is_system_admin`-only gate — admin reads,
> outsider denied, firewall holds. None exercises the middle, and the middle is §5's whole
> correction. So the original Step 4's gate would have gone green, been reviewed against a plan
> whose prose said something else, and shipped. A fourth test (`the_grant_writer_can_read_their_own
> _grant_record`) now covers it, with a probe step that proves it fails on the old gate. **Where a
> plan's prose and its code disagree, the tests decide which one ships — so check what the tests
> can actually distinguish.**

**Interfaces:**
- Consumes: `temper_substrate::payloads::{EventRef, RefRel, RefTarget, AnchorTable}` (Task 1).
- Produces:
  - `AdminLedgerEntry { event_id: Uuid, event_type: String, actor_profile_id: Uuid, actor_handle: String, occurred_at: DateTime<Utc>, payload: serde_json::Value, references: Vec<EventRef>, correlation_id: Option<Uuid> }`
  - `list_by_subject(pool: &PgPool, caller: ProfileId, subject: RefTarget, limit: i64, offset: i64) -> ApiResult<Vec<AdminLedgerEntry>>`
  - `list_by_actor(pool: &PgPool, caller: ProfileId, actor: ProfileId, limit: i64, offset: i64) -> ApiResult<Vec<AdminLedgerEntry>>`
  - `ledger_epoch(pool: &PgPool) -> ApiResult<Option<DateTime<Utc>>>`

**Authorization** (spec §5 — **challenged 2026-07-16; the principle held, the implementation was refuted and corrected. Read §5 before writing `gate()`.**):

The read gate **mirrors the write gate** — but the *actual* write gate, **dispatched per event type**, never one uniform team-shaped predicate:

| record | read gate | mirrors |
|---|---|---|
| `grant_created` / `grant_revoked` | `is_system_admin` OR `can(caller,'grant',subject_table,subject_id)` | `access_service::can_administer_grant` |
| machine / connection acts | `machine_authz::authorize(owner_team)` | itself |
| `promote_admin`, `update_system_settings` | `is_system_admin` | itself |

Default arm: **`is_system_admin`** — fail closed. An act not in the table is admin-only to read until someone adds it deliberately.

> ⚠️ **This line previously read "flagged for review and confirmed" and specified `machine_authz::authorize` for the whole ledger. It was never confirmed, and it was wrong.** That predicate is `is_system_admin OR role_on_team(team)=Owner`, failing closed on `team = None` — it gates *machine registration*, not grants. The grant path gates on `can_administer_grant` (`access_service.rs:189`/`:218`): `is_system_admin OR can(caller,'grant',subject)`. A capability on the subject is not a role on a team. Because `derived_access_profile` lets a **resource owner** grant on their own resource, and a profile-homed resource has **no owning team**, the old gate would have Forbidden the actor from reading the record of the act they had just performed — on the mainline path. Probed live: `is_sysadmin = f`, `can_write_the_grant = t`. Do not restore it.

**Implementation note:** `can_administer_grant` is a **private** fn in `access_service` today. Expose it (`pub(crate)`) and call it — do **not** restate its body, or the read gate becomes a second copy of the policy and drifts from the write gate it exists to mirror.

#### The gate cannot be a prelude — it must select the type set (amended 2026-07-16)

> This plan's original Step 4 shipped `gate(pool, caller)` called **before** `fetch()`, whose body
> was `is_admin → Ok, else NotFound`. That is not the gate this section describes, and the gap is
> **structural, not editorial**: a gate that runs before the query cannot dispatch on event type,
> because no event type is known until rows come back. Fixing the body in place is impossible;
> the shape has to change.

**Invert it.** Do not fetch rows and then filter them — **compute what the caller may read, then
ask only for that.** The subject is a *parameter* of `list_by_subject`, so the whole §5 table can be
evaluated **once, before the query**, yielding the set of event types this caller may read *for this
subject*. That set becomes the `t.name = ANY($1)` bind the query already had.

```rust
/// The §5 table, evaluated for one subject. Returns the event types `caller` may read about
/// `subject` — empty means "nothing", which the caller turns into 404.
///
/// One gate call per act family, NOT one per row: the subject is fixed, so the answer is too.
/// Per-row gating would be an N+1 (two queries per row) AND would silently break LIMIT/OFFSET —
/// filtering after the window means page 2 is not the second 50 readable rows, it is whatever
/// survived of the second 50 raw rows.
async fn readable_event_types(
    pool: &PgPool,
    caller: ProfileId,
    subject: RefTarget,
) -> ApiResult<Vec<&'static str>> {
    // Admin reads everything; one query, and the common admin path stops here.
    if access_service::is_system_admin(pool, caller).await? {
        return Ok(ADMIN_EVENT_TYPES.to_vec());
    }

    let mut readable = Vec::new();

    // grant_created / grant_revoked → mirrors access_service::can_administer_grant.
    // (is_system_admin is already OR-ed inside it; we short-circuited above, so this is the
    // can_grant arm doing the work.)
    if access_service::can_administer_grant(pool, caller, subject.kind.as_str(), subject.id).await? {
        readable.push("grant_created");
        readable.push("grant_revoked");
    }

    // admin_ledger_opened → the epoch marker. is_system_admin only; handled by the arm above.
    // Machine/connection acts → machine_authz::authorize(owner_team). NOT REACHED IN THIS TASK:
    //   no such event type exists until step 5 of the spec's §9, and ADMIN_EVENT_TYPES does not
    //   list one. When one is added, it gets an arm HERE, and the default below keeps it
    //   admin-only until someone does.
    //
    // Default: absent from this fn ⇒ admin-only ⇒ fail closed.
    Ok(readable)
}
```

This keeps every property §5 asked for — dispatch per event type, no new predicate, no second copy
of the policy, fail-closed default — and it costs **two queries regardless of page size**, both
already indexed. An act type nobody added an arm for is unreadable by non-admins: that is the
default arm, expressed as *absence from the returned set* rather than as a match arm nobody wrote.

**The actor axis (`list_by_actor`) does not have this shape**, and cannot until §11.1b is decided —
see the decision step below. Its rows span many subjects and many types, so there is no single
subject to evaluate the table against.

- [x] **Step 0: Close §11.1b's first sub-question — is the actor axis self-gating? (ADDED 2026-07-16)**

**CLOSED 2026-07-16: SELF-GATING.** Decision + the live probe behind it are recorded in **spec
§11.1b** — read it before Step 4; do not reconstruct the reasoning from this summary.

- `caller == actor` ⇒ **all** the caller's own admin acts, **no per-subject gate**, conditioned only
  on `access_service::has_system_access(caller)`. Keep your history unless you lose the front door.
- `caller != actor` ⇒ `is_system_admin` or 404.

Call `has_system_access`, do not assume it. Both surfaces gate it upstream already
(`temper-api/src/middleware/system_access.rs:38`, `temper-mcp/src/service.rs:85`), so this is
defense in depth against a route wired without the layer — and it is the same function, not a
restatement. It is vacuous under `access_mode='open'` (short-circuits true); that is intended.

The other two consequences (the vanished subject; fail-closed on a producer bug) are **recorded, not
resolved, by this task** — they need no code here, and §11.1b now carries both to Task 5, where the
first writer is actually built.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-services/tests/admin_ledger_test.rs`:

```rust
#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_substrate::payloads::{AnchorTable, RefTarget};
use uuid::Uuid;

// No `mod common;` — crates/temper-services/tests/ has no shared harness. The fixture is
// inline, below (Step 2), matching context_read_predicate_test.rs.

/// Insert a NULL-anchored admin event by hand. Task 5 replaces this with a real fire arm;
/// until then the read surface must be provable against a crafted row.
async fn seed_admin_event(pool: &PgPool, emitter: Uuid, subject: Uuid, principal: Uuid) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO kb_events
               (event_type_id, emitter_entity_id, payload, "references")
           SELECT et.id, $1,
                  jsonb_build_object('subject_table','kb_contexts','subject_id',$2::text),
                  jsonb_build_array(
                    jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind','kb_contexts','id',$2)),
                    jsonb_build_object('rel','principal','target', jsonb_build_object('kind','kb_teams',   'id',$3))
                  )
             FROM kb_event_types et WHERE et.name = 'grant_created'
           RETURNING id"#,
    )
    .bind(emitter).bind(subject).bind(principal)
    .fetch_one(pool).await.expect("seed admin event")
}

#[sqlx::test]
async fn list_by_subject_finds_the_admin_event(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    let ev = seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    let got = temper_services::services::admin_ledger_service::list_by_subject(
        &pool,
        f.admin_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id },
        50,
        0,
    )
    .await
    .expect("list_by_subject");

    assert_eq!(got.len(), 1, "the seeded grant_created must be found by its subject reference");
    assert_eq!(got[0].event_id, ev);
    assert_eq!(got[0].event_type, "grant_created");
    assert_eq!(got[0].actor_profile_id, f.admin_profile.uuid());
}

#[sqlx::test]
async fn the_admin_event_is_invisible_to_cognition(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    // The firewall: a NULL-anchored event must not be counted by the steward's ingest delta.
    // NOTE steward_ingest_delta(p_cogmap, p_watermark) takes a COGMAP, not a team
    // (migrations/20260701000005_steward_ingest_watermark.sql:40).
    let new_events: i64 = sqlx::query_scalar(
        "SELECT new_events FROM steward_ingest_delta($1, NULL)",
    )
    .bind(f.cogmap_id)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    assert_eq!(new_events, 0, "NULL-anchored admin events must not reach the steward delta");
}

#[sqlx::test]
async fn a_non_admin_cannot_read_the_ledger(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    let err = temper_services::services::admin_ledger_service::list_by_subject(
        &pool,
        f.outsider_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id },
        50,
        0,
    )
    .await
    .expect_err("an outsider must not read the admin ledger");

    assert!(
        matches!(err, temper_services::ApiError::NotFound),
        "reads deny with 404, not 403 (the deny-split invariant); got {err:?}"
    );
}

/// THE TEST THIS SUITE WAS MISSING (added 2026-07-16). The three tests above are all satisfied by
/// an `is_system_admin`-only gate — admin reads, outsider is denied, neither exercises the middle.
/// But the middle **is** §5's entire correction: a non-admin who could WRITE the grant must be able
/// to READ the record of it. Without this test the refuted gate passes green, and so does the
/// original Step 4 body. A suite that cannot fail on the bug the spec was rewritten to prevent is
/// not a suite, it is decoration.
///
/// `derived_access_profile` gives a resource's owner `can(…,'grant',…)` on it — derived, no
/// explicit grant, no team, no admin. Probed live in §5: `is_sysadmin=f, can_write_the_grant=t`.
#[sqlx::test]
async fn the_grant_writer_can_read_their_own_grant_record(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    // The owner of the subject: not an admin, but `can_administer_grant` is true for them.
    // Assert BOTH halves first — if the fixture silently made them an admin, this test would
    // pass while proving nothing.
    assert!(
        !access_service::is_system_admin(&pool, f.owner_profile).await.unwrap(),
        "the grant writer must NOT be an admin, or this test proves nothing"
    );

    seed_admin_event(&pool, f.admin_emitter, f.owned_resource_id, f.team_id).await;

    let got = temper_services::services::admin_ledger_service::list_by_subject(
        &pool,
        f.owner_profile,
        RefTarget { kind: AnchorTable::Resources, id: f.owned_resource_id },
        50,
        0,
    )
    .await
    .expect("the actor who could write this grant must be able to read its record");

    assert_eq!(got.len(), 1, "the grant writer sees the record of the act they could perform");
}

/// §11.1b, decided 2026-07-16: the actor axis is SELF-GATING. This is the test that distinguishes
/// it from a subject-gated axis — the actor authored the act but cannot administer its subject, so
/// under subject-gating they would lose sight of their own authorship. Which is the exact defect
/// this whole spec exists to undo.
#[sqlx::test]
async fn the_actor_keeps_their_own_history_without_the_capability(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    // An act authored BY the owner, ON a subject the owner cannot administer (the admin's
    // context, not theirs). Authorship and capability deliberately pulled apart.
    seed_admin_event(&pool, f.owner_emitter, f.context_id, f.team_id).await;

    // The subject axis denies them — they have no can_grant on this subject. This half must hold
    // or the test is not exercising the distinction.
    let subject_err = temper_services::services::admin_ledger_service::list_by_subject(
        &pool,
        f.owner_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id },
        50,
        0,
    )
    .await
    .expect_err("no capability on this subject ⇒ the subject axis denies");
    assert!(matches!(subject_err, temper_services::ApiError::NotFound));

    // The actor axis returns it anyway. That is the decision.
    let mine = temper_services::services::admin_ledger_service::list_by_actor(
        &pool,
        f.owner_profile,
        f.owner_profile,
        50,
        0,
    )
    .await
    .expect("an actor always reads their own acts");

    assert_eq!(mine.len(), 1, "authorship survives the loss of capability over the subject");
    assert_eq!(mine[0].actor_profile_id, f.owner_profile.uuid());
}

/// The other half of the decision: reading SOMEONE ELSE's history is an audit, and audits are
/// admin-only. Self-gating widens the actor's own view; it must not widen anyone else's.
#[sqlx::test]
async fn reading_another_actors_history_is_admin_only(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    let err = temper_services::services::admin_ledger_service::list_by_actor(
        &pool,
        f.outsider_profile,
        f.admin_profile,
        50,
        0,
    )
    .await
    .expect_err("a non-admin must not audit another profile's acts");
    assert!(matches!(err, temper_services::ApiError::NotFound));

    // ...and the admin may.
    let audit = temper_services::services::admin_ledger_service::list_by_actor(
        &pool,
        f.admin_profile,
        f.admin_profile,
        50,
        0,
    )
    .await
    .expect("an admin audits");
    assert_eq!(audit.len(), 1);
}
```

> The fixture therefore also needs `owner_profile`, `owner_emitter` and `owned_resource_id`: a
> non-admin profile, its emitter (it authors events in the actor-axis tests), and a resource it owns
> (homed on its own context, `owner_table='kb_profiles'` — which is exactly the shape that has **no
> owning team** and refuted the original gate). Add all three to `AdminFixture`.
> Note the seeds above anchor on the **resource** in one test and the **context** in another, so
> `seed_admin_event`'s hardcoded `'kb_contexts'` subject kind must become a parameter.

- [ ] **Step 2: Build the fixture — inline, in this file**

`crates/temper-services/tests/` has **no `common/` module**. Every test there declares its fixture
inline (see `context_read_predicate_test.rs`, which opens with `struct Org`). Follow that.

Replace the `mod common;` line with an inline fixture at the top of `admin_ledger_test.rs`:

```rust
struct AdminFixture {
    admin_profile: temper_core::types::ids::ProfileId,
    admin_emitter: Uuid,
    outsider_profile: temper_core::types::ids::ProfileId,
    team_id: Uuid,
    cogmap_id: Uuid,
    context_id: Uuid,
}

/// A system-admin with emitters, an outsider, a team, a cogmap, and a context to grant on.
///
/// **Build profiles through `profile_service`, never with raw INSERTs.**
/// `provision_profile_entities` is what creates the `<handle>@<surface>` emitter that
/// `resolve_emitter` (a `fetch_one`, no lazy creation) needs and that this ledger reads back.
/// A fixture that hand-INSERTs a profile passes while production 500s — which is exactly the
/// live bug recorded in task `019f6b06-c48f-7a81-a238-cdd6b131f3dc`.
///
/// **`admin_profile` is not an admin because a column says so.** `is_system_admin(p)` is
/// *owner of the gating team* — it reads `kb_team_members`, joined to the team whose slug is
/// `kb_system_settings.gating_team_slug`. It never looks at `kb_profiles.system_access`. So this
/// fixture MUST, or every admin assertion below is vacuous:
///   1. create the team,
///   2. `UPDATE kb_system_settings SET gating_team_slug = <that team's slug>` — it is **empty**
///      out of the box, and an empty slug means *nobody is admin* (the deliberate bootstrap:
///      `is_system_admin` returns false for everyone until an operator names the team),
///   3. add `admin_profile` to it with `role = 'owner'` — `member` is not enough.
/// Setting `system_access = 'admin'` does NOT do this. It fires `trg_sync_system_membership`,
/// which auto-joins teams carrying an `auto_join_role` (`temper-system` carries `watcher`) — a
/// *watcher*, not an owner of the gating team. Verified on dev: the `system` profile has
/// `system_access = 'admin'` and `is_system_admin() = f`.
///
/// Assert this in the fixture rather than trusting it — a fixture whose admin is not an admin
/// makes `a_non_admin_cannot_read_the_ledger` pass for the wrong reason (everyone is a non-admin).
async fn admin_fixture(pool: &PgPool) -> AdminFixture {
    // Read `context_read_predicate_test.rs` and `saml_provisioning_test.rs` first and reuse
    // whatever profile/team construction they already do rather than inventing a third shape.
    todo!(
        "construct via profile_service (provisioning the emitters) + team_service::create_team; \
         then point gating_team_slug at that team and make admin_profile its OWNER; \
         then assert access_service::is_system_admin(pool, admin_profile) is true, and that \
         is_system_admin(pool, outsider_profile) is false"
    )
}
```

**This `todo!()` is the one place this plan defers to the implementer.** Read the two named test
files first; match their construction. The one non-negotiable is that profiles come from
`profile_service` so their emitters exist.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test`
Expected: FAIL — `admin_ledger_service` does not exist.

Export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` and ensure `cargo make docker-up` has run.

- [ ] **Step 4: Write the implementation**

Create `crates/temper-services/src/services/admin_ledger_service.rs`:

```rust
//! The admin ledger's read surface.
//!
//! Admin events are NULL-anchored (spec 2026-07-16 §4) — the cognition firewall. That firewall
//! is structural: every region producer, `steward_ingest_delta`, materialize attribution, and
//! `latest_event_id_for_context` scope by `producing_anchor_table`, so a both-NULL event is
//! invisible to all of them. It is equally invisible to every *reader*, which is why identity
//! lives in `kb_events."references"` (GIN-indexed, and consulted by no cognition reader).
//!
//! Two axes, both index-backed:
//!   - by subject  → `references @> …`      (idx_kb_events_references, jsonb_path_ops)
//!   - by actor    → `emitter_entity_id = …` (idx_kb_events_emitter, (emitter, occurred_at DESC))

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_substrate::payloads::{EventRef, RefTarget};
use uuid::Uuid;

use crate::services::access_service;
use crate::{ApiError, ApiResult};

#[derive(Debug, Clone, serde::Serialize)]
pub struct AdminLedgerEntry {
    pub event_id: Uuid,
    pub event_type: String,
    pub actor_profile_id: Uuid,
    pub actor_handle: String,
    pub occurred_at: DateTime<Utc>,
    pub payload: serde_json::Value,
    pub references: Vec<EventRef>,
    pub correlation_id: Option<Uuid>,
}

/// Admin event types. The ledger read surface returns ONLY these — never cognition events that
/// happen to share the NULL-anchor bucket (`lens_created` is already in it). Discriminating by
/// anchor nullity would silently absorb system-config events; discriminate by type.
const ADMIN_EVENT_TYPES: &[&str] = &["admin_ledger_opened", "grant_created", "grant_revoked"];

// `readable_event_types` (above, in the Authorization section) IS the gate. There is no
// `gate(pool, caller)` prelude — a prelude cannot dispatch on event type, which is the whole of §5.

pub async fn list_by_subject(
    pool: &PgPool,
    caller: ProfileId,
    subject: RefTarget,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    let types = readable_event_types(pool, caller, subject).await?;
    if types.is_empty() {
        // Reads deny with 404, not 403 — the deny-split invariant. A 403 would confirm the
        // ledger has something to hide about this subject.
        return Err(ApiError::NotFound);
    }

    // The `rel` is pinned to `subject` deliberately. `[{"target": …}]` alone would also match a
    // `principal` or `touches` reference to the same id — "every act performed FOR this team"
    // silently answering a query that says "performed ON it". jsonb_path_ops containment indexes
    // the fuller object just as well (verified: Bitmap Index Scan, Step 6).
    let probe = serde_json::json!([{ "rel": "subject", "target": subject }]);
    fetch(pool, &types, Some(probe), None, limit, offset).await
}

/// The actor axis is **self-gating** (spec §11.1b, decided 2026-07-16): you may always read the
/// record of acts you performed. Losing a capability, a role, or ownership of a subject does not
/// take your own history from you — only losing system access does, because then you are not a
/// reader at all.
///
/// Deliberately NOT subject-gated. The defect that motivated this whole spec is
/// `kb_access_grants` destroying `granted_by_profile_id` on upsert; a ledger that restores
/// authorship and then hides it from its author would be a poor trade. Probed live: §5's
/// `can_grant` arm carries ZERO of prod's 5 real grants, so a subject-gate here would today mean
/// "admins only" — and ownership is mutable (`rehome`/`reassign` ship), so the demoted actor is
/// reachable by ordinary usage, not just by demotion.
pub async fn list_by_actor(
    pool: &PgPool,
    caller: ProfileId,
    actor: ProfileId,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    // The front door, called rather than assumed. Both surfaces gate this upstream already
    // (temper-api middleware, temper-mcp service) — this is defense in depth against a future
    // route wired without the layer, and it is the same predicate, not a second copy of it.
    // Vacuous under access_mode='open', where has_system_access short-circuits true. Intended.
    if !access_service::has_system_access(pool, caller).await? {
        return Err(ApiError::NotFound);
    }

    // Reading someone else's history is an audit, and audits are admin-only.
    if caller != actor && !access_service::is_system_admin(pool, caller).await? {
        return Err(ApiError::NotFound);
    }

    // No per-subject gate: that is the decision. The full catalogue is correct here precisely
    // because the axis is the caller's own authorship (or an admin's audit).
    fetch(pool, ADMIN_EVENT_TYPES, None, Some(actor.uuid()), limit, offset).await
}

/// The epoch: admin history begins here. NOT a backfill marker — everything before this is
/// genuinely unrecorded (spec §8), and the surface must say so rather than imply absence.
pub async fn ledger_epoch(pool: &PgPool) -> ApiResult<Option<DateTime<Utc>>> {
    Ok(sqlx::query_scalar!(
        "SELECT e.occurred_at FROM kb_events e
           JOIN kb_event_types t ON t.id = e.event_type_id
          WHERE t.name = 'admin_ledger_opened'
          ORDER BY e.occurred_at ASC LIMIT 1"
    )
    .fetch_optional(pool)
    .await?)
}

async fn fetch(
    pool: &PgPool,
    // The gate's output, not the whole catalogue: `readable_event_types` decided this.
    types: &[&str],
    subject_probe: Option<serde_json::Value>,
    actor: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> ApiResult<Vec<AdminLedgerEntry>> {
    // Runtime query_as: the two axes select different predicates. Follows the search_service
    // precedent for dynamic predicates.
    let rows = sqlx::query_as::<_, (Uuid, String, Uuid, String, DateTime<Utc>, serde_json::Value, serde_json::Value, Option<Uuid>)>(
        r#"SELECT e.id, t.name, p.id, p.handle, e.occurred_at, e.payload, e."references", e.correlation_id
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
             JOIN kb_entities   en ON en.id = e.emitter_entity_id
             JOIN kb_profiles    p ON p.id = en.profile_id
            WHERE t.name = ANY($1)
              AND ($2::jsonb IS NULL OR e."references" @> $2::jsonb)
              AND ($3::uuid  IS NULL OR p.id = $3::uuid)
            ORDER BY e.occurred_at DESC, e.id DESC
            LIMIT $4 OFFSET $5"#,
    )
    // The authorized set, NOT ADMIN_EVENT_TYPES. Binding the catalogue here would make the gate
    // decorative — it would compute a type set and then query for every type anyway.
    // If the `Encode` bound on `&[&str]` does not resolve, collect to `Vec<String>` — do not
    // reach for string interpolation.
    .bind(types)
    .bind(subject_probe)
    .bind(actor)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|(event_id, event_type, actor_profile_id, actor_handle, occurred_at, payload, refs, correlation_id)| {
            Ok(AdminLedgerEntry {
                event_id,
                event_type,
                actor_profile_id,
                actor_handle,
                occurred_at,
                payload,
                references: serde_json::from_value(refs)
                    .map_err(|e| ApiError::Internal(format!("malformed references on {event_id}: {e}")))?,
                correlation_id,
            })
        })
        .collect()
}
```

Register it in `crates/temper-services/src/services/mod.rs`:

```rust
pub mod admin_ledger_service;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test`
Expected: PASS (6 tests — three added 2026-07-16: the grant-writer test, and the two actor-axis
tests carrying §11.1b's decision).

If `the_admin_event_is_invisible_to_cognition` fails, **stop** — the firewall is the design's load-bearing claim. Do not adjust the test to pass; report it.

If `the_grant_writer_can_read_their_own_grant_record` fails, **stop** — that is the refuted gate
resurrecting, and the fix is the gate, never the test. Softening it to `expect_err` would restore
exactly the defect §5 was rewritten to prevent, and it would look like a passing suite.

**Probe the suite before believing it (`feedback_differential_testing_over_handwritten_expectations`,
`feedback_ground_in_data_not_ast`).** Temporarily replace `readable_event_types`' body with
`Ok(if access_service::is_system_admin(pool, caller).await? { ADMIN_EVENT_TYPES.to_vec() } else { vec![] })`
— i.e. this plan's original Step 4 gate, modulo its wrong admin predicate. Confirm
`the_grant_writer_can_read_their_own_grant_record` goes **red** and the other three stay green. If
it does not go red, the test is not testing the gate. Revert the probe.

- [ ] **Step 6: Prove the GIN index is used, not a seq scan**

The whole read path rests on `idx_kb_events_references`. A containment query that seq-scans 13k rows is a design failure hiding as a passing test.

> **AMENDED 2026-07-16 — this step, as originally written, returns a false negative and instructs
> you to "fix" a probe that is already correct.** Dev's `kb_events` holds **4 rows**. At that size
> the planner picks `Seq Scan` for *any* predicate, index-compatible or not — it is measuring the
> table, not the probe. The original step said "If it shows `Seq Scan`, the probe shape does not
> match `jsonb_path_ops` containment — fix the probe, not the index," which on this database is
> advice to break a working query. **Already run, both ways, on the amended branch:** plain
> `EXPLAIN` → `Seq Scan`; `SET enable_seqscan=off` → `Bitmap Index Scan on
> idx_kb_events_references`. **The probe shape is correct.** What this step actually checks is
> that the probe *can* use the index — so take the planner's choice off the table:

```bash
psql "$DATABASE_URL" -c "SET enable_seqscan=off; EXPLAIN SELECT id FROM kb_events WHERE \"references\" @> '[{\"rel\":\"subject\",\"target\":{\"kind\":\"kb_contexts\",\"id\":\"019f6055-6aea-7aa2-a133-61552dd3d7e4\"}}]'::jsonb;"
```

Expected: `Bitmap Index Scan on idx_kb_events_references`. If it *still* shows `Seq Scan` with
seqscan disabled, the probe genuinely cannot use `jsonb_path_ops` — then, and only then, fix the
probe.

Note the probe now pins `"rel":"subject"`, matching the amended `list_by_subject`. Containment
indexes the fuller object identically — a narrower probe is not a slower one.

Both indexes this task's two axes rest on are confirmed present on dev:
`idx_kb_events_references gin ("references" jsonb_path_ops)` and
`idx_kb_events_emitter btree (emitter_entity_id, occurred_at DESC)`. Neither needs creating.

- [ ] **Step 7: Regenerate the sqlx cache and commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make check
# NOTE: `crates/temper-services/tests/common/mod.rs` was staged here in the original plan and
# does NOT exist — Steps 1-2 say so themselves, and `git add` fails the whole command on an
# unmatched pathspec. The fixture is inline. Removed 2026-07-16.
# `payloads.rs` is staged because this task adds `AnchorTable::as_str()` (see the notes).
git add crates/temper-services/src/services/admin_ledger_service.rs \
        crates/temper-services/src/services/mod.rs \
        crates/temper-services/src/services/access_service.rs \
        crates/temper-substrate/src/payloads.rs \
        crates/temper-services/tests/admin_ledger_test.rs \
        docs/superpowers/plans/2026-07-16-admin-event-sink.md \
        .sqlx crates/temper-services/.sqlx
git commit -m "feat(admin-ledger): read surface on kb_events.references

Ships before any writer, deliberately. The NULL anchor that firewalls admin
events from cognition also hides them from every reader, so the read path had
to be designed first or the writers would target a query shape nobody proved.

Two axes, both index-backed: by subject via references @> (GIN), by actor via
(emitter_entity_id, occurred_at DESC). Filters by admin event type rather than
by anchor nullity — lens_created already lives in the NULL bucket."
```

---

### Task 3: The `element_trail` payload-key invariant

> **SHIPPED 2026-07-17.** Two tests appended to `crates/temper-services/tests/admin_ledger_test.rs`:
> `no_admin_payload_spells_a_trail_matched_key` and `an_admin_event_never_appears_in_an_element_trail`.
> Pre-dispatch against `main` (which had moved since 2026-07-16 — Task 2 shipped as PR #475) found
> two API-drift corrections to the sketch below, plus one design correction. The sketch below is
> left as the original design; **read the shipped file for the exact tests** — they differ per the
> three corrections here:
>
> - **Seed arity.** Task 2's shipped `seed_admin_event` is **5-arg** —
>   `(pool, emitter, subject_kind: AnchorTable, subject, principal)` — not the 4-arg form sketched
>   here. Every call passes an `AnchorTable`.
> - **Test attribute.** Every test in the file uses `#[sqlx::test(migrator = "temper_services::MIGRATOR")]`,
>   not bare `#[sqlx::test]` (the workspace migrations are not on `./migrations` relative to the crate).
> - **Non-vacuity (the design correction).** The original sketch seeded a **context**-subject event
>   and asserted it stays out of a **node** trail. But `element_trail_node` matches only
>   `resource_id`/`owner`/`block_id` — a context-subject payload can *never* match it, so that test
>   passes whether or not the ban exists: it **cannot fail**. That is the precise defect §5 itself was
>   born from ("the specced tests could not fail on it"). Both tests are now seeded through the
>   canonical writer with **positive controls** so they genuinely can go red — proven by injecting
>   `resource_id` into the writer and watching both fail (corpus scan reports the offender; trail scan
>   reports `leaked > 0`).

`element_trail_node`/`element_trail_edge` (`migrations/20260706000002_element_trail_payload_actor.sql:7-52`) have **no event-type filter**. They match purely on payload key shape and are gated only by `resources_visible_to(p_profile)` (`:47-49`). An admin payload spelling `resource_id` — natural, since a grant with `subject_table='kb_resources'` *is about* a resource — would surface **who was granted access to it** to any reader of that resource.

This lands **before any admin payload exists**, so the invariant is never retrofitted.

**Files:**
- Test: `crates/temper-services/tests/admin_ledger_test.rs` (append)

**Interfaces:**
- Consumes: Task 2's fixture and `seed_admin_event`.

- [ ] **Step 1: Write the failing test**

```rust
/// The banned keys. element_trail_* match on payload KEY SHAPE with no type filter, so an admin
/// payload using any of these leaks an authority record into a cognition read gated only by
/// resources_visible_to. Spec 2026-07-16 §5 makes this a tested invariant, not a convention.
const BANNED_ADMIN_PAYLOAD_KEYS: &[&str] = &["resource_id", "block_id", "edge_id", "owner"];

#[sqlx::test]
async fn no_admin_payload_spells_a_trail_matched_key(pool: PgPool) {
    let bad: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT t.name, k.key
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
             CROSS JOIN LATERAL jsonb_object_keys(e.payload) AS k(key)
            WHERE t.name = ANY($1) AND k.key = ANY($2)"#,
    )
    .bind(ADMIN_EVENT_TYPES_FOR_TEST)
    .bind(BANNED_ADMIN_PAYLOAD_KEYS)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        bad.is_empty(),
        "admin payloads must not spell element_trail-matched keys — these leak authority \
         records to any reader of the resource. Use subject_table/subject_id + references. \
         Offenders: {bad:?}"
    );
}

#[sqlx::test]
async fn an_admin_event_never_appears_in_an_element_trail(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    seed_admin_event(&pool, f.admin_emitter, f.context_id, f.team_id).await;

    // element_trail_node over every resource the admin can see must return no admin event.
    // NOTE its RETURNS TABLE spells the type column `kind`, not `event_type`
    // (migrations/20260706000002_element_trail_payload_actor.sql:27).
    let leaked: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
             FROM kb_resources r
             CROSS JOIN LATERAL element_trail_node($1, r.id) AS tr
            WHERE tr.kind = ANY($2)"#,
    )
    .bind(f.admin_profile.uuid())
    .bind(ADMIN_EVENT_TYPES_FOR_TEST)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    assert_eq!(leaked, 0, "no admin event may surface in a cognition element trail");
}
```

Add near the top of the test file:

```rust
const ADMIN_EVENT_TYPES_FOR_TEST: &[&str] = &["admin_ledger_opened", "grant_created", "grant_revoked"];
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test -E 'test(payload) or test(element_trail)'`
Expected: PASS immediately — Task 2's `seed_admin_event` already uses `subject_table`/`subject_id`. **A test that passes on first write is correct here**: it is a regression guard against Task 5 and the follow-on plan, not a red-green cycle. If `an_admin_event_never_appears_in_an_element_trail` fails, `element_trail_node`'s signature differs from the assumption — read `migrations/20260706000002_element_trail_payload_actor.sql:7-52` and fix the call, not the assertion.

- [ ] **Step 3: Commit**

```bash
cargo make check
git add crates/temper-services/tests/admin_ledger_test.rs
git commit -m "test(admin-ledger): element_trail payload-key invariant

element_trail_node/_edge match on payload key shape with NO type filter and are
gated only by resources_visible_to. An admin payload spelling resource_id would
surface who was granted access to a resource, to anyone who can read it.

Lands before any admin payload exists so the invariant is never retrofitted."
```

---

### Task 4: Event types + payload schemas + the epoch marker

> **SHIPPED 2026-07-17, reshaped by grounding. Two decisions with Pete changed the approach below;
> read this before the original steps.**
>
> **1. The admin payloads are now TYPED, not hand-written migration JSON.** Pre-dispatch found the
> gate the plan didn't know about: `bootseed_publishes_payload_schemas` (`bootseed.rs:84`) asserts
> the stamped registry set **equals** `TYPED_EVENT_NAMES` (`payloads.rs`). Stamping 3 non-typed admin
> schemas broke it (16 ≠ 15). So `GrantCreated`/`GrantRevoked`/`AdminLedgerOpened` are now real
> structs in `payloads.rs` (reusing `AnchorTable`/`ProfileId`), added to `TYPED_EVENT_NAMES` (15→18),
> with committed fixtures (`UPDATE_SCHEMA=1 cargo make test-schema`). Migration `…010` stamps the
> three **verbatim from those fixtures** (canonical-seed pattern), so repo==registry==Rust-types, and
> `verify_ledger_roundtrip` now validates every admin payload — including the SQL-built grant payloads
> Task 5 writes — against its struct. The grant struct shapes match Task 5's `_admin_grant_*` SQL
> exactly (subject/principal table+id, four caps, `granted_by`, optional `previous`).
>
> **2. The 019f6b48 fold (NULL region_materialized/lens_created) is DROPPED and re-filed.** Same gate:
> NULLing a *typed* name breaks stamped==typed. Their staleness is real, but the invariant-consistent
> fix is a **re-stamp from current fixtures**, not a NULL — which is the deliberate registration pass
> the plan itself said was out of scope. Re-filed as its own task. (Verified against prod: it has
> exactly 2 non-NULL schemas, `region_materialized` 235 events + `lens_created` 3 — the plan's premise
> held; the July-12 migrations that re-stamp them are themselves stale, so no chain state is current.)
>
> **3. The epoch payload is `{ note }` only** — `ledger_epoch` reads the event's `occurred_at`, and
> the module's rule keeps timestamps out of payloads, so there is no `opened_at`.
>
> **4. The EventKind variants + replay no-op arms (Task 5 Step 4) are folded in here** — the epoch
> event's mere existence would otherwise make `replay::from_canonical_name` hard-fail on the unknown
> type. Task 5 Step 4 is therefore already done; see its note.
>
> The migration/step bodies below are the original design; the shipped migrations differ per the
> above — read `migrations/20260717000010_admin_event_types.sql`, `…020`, `payloads.rs`, and
> `crates/temper-substrate/src/{events,replay}.rs` for the exact form.

**Files:**
- Create: `migrations/20260717000010_admin_event_types.sql`
- Create: `migrations/20260717000020_admin_ledger_epoch.sql`
- Modify: `crates/temper-substrate/src/payloads.rs` (the 3 typed structs + `TYPED_EVENT_NAMES`)
- Modify: `crates/temper-substrate/src/{events,replay}.rs` (EventKind variants + replay arms — Task 5 Step 4, folded)
- Create: `crates/temper-substrate/tests/fixtures/payloads/{admin_ledger_opened,grant_created,grant_revoked}.v1.schema.json`

**Interfaces:**
- Produces: `kb_event_types` rows `admin_ledger_opened`, and payload schemas on the pre-existing `grant_created`/`grant_revoked` rows. One `admin_ledger_opened` event. **Also NULLs the two stale registry rows** — see below.
- Consumes: nothing.

> **This task now also closes `019f6b48-a562-7871-a48d-87945a796c7e`** (*"Prod `kb_event_types` carries stale payload schemas — repo is fixed, the registry is not"*), folded in here by decision (2026-07-16, with Pete) rather than shipped as its own migration. This is the first thing to touch `payload_schema` since the boot-seed, so it is the natural and cheapest place: a separate migration would touch the same column twice for no benefit. **Decision: NULL `region_materialized` and `lens_created`, do not re-stamp** — a reader can handle "unregistered"; a reader cannot detect "confidently wrong". Reasoning in Step 1's migration comment. **The task's recurrence question ("what keeps registry == repo after the next payload change?") must be answered in this PR** — see the callout after Step 1.

`grant_created`/`grant_revoked` **already exist** in `kb_event_types` (seeded 2026-06-24, `migrations/20260624000003_canonical_seed.sql:51-52`) with NULL `payload_schema` and zero events. They are not dropped — they are this task's types. Their schemas get filled.

- [ ] **Step 1: Write the event-types migration**

Create `migrations/20260717000010_admin_event_types.sql`:

```sql
-- Admin-ledger event types (spec 2026-07-16 §9 step 3).
--
-- `grant_created`/`grant_revoked` were seeded 2026-06-24 and have carried NULL payload_schema and
-- zero events ever since. They are NOT orphans to be dropped: they are this arc's types, and this
-- migration gives them the schemas they never got. NULL payload_schema is legitimate per
-- 20260624000001_canonical_schema.sql:445-447 ("NULL = unregistered/permissive") and 31 of 33 types
-- have one -- so their emptiness never evidenced anything.
--
-- `admin_ledger_opened` is the epoch marker's type. Administration has ~14 acts in prod history and
-- ZERO events; those acts are not reconstructable (kb_teams has no creator column, and the grant
-- upsert overwrites granted_by_profile_id + granted_at). So the ledger declares where it begins
-- rather than synthesizing a past it cannot know.

INSERT INTO kb_event_types (name, payload_schema, schema_version)
SELECT 'admin_ledger_opened', $js${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "AdminLedgerOpened",
  "type": "object",
  "additionalProperties": false,
  "required": ["opened_at", "note"],
  "properties": {
    "opened_at": {"type": "string", "format": "date-time"},
    "note": {"type": "string"}
  }
}$js$::jsonb, 1
WHERE NOT EXISTS (SELECT 1 FROM kb_event_types WHERE name = 'admin_ledger_opened');

-- Fill the two pre-existing types' schemas. Note the payload deliberately spells the subject
-- `subject_table`/`subject_id` and NOT `resource_id`/`owner` -- element_trail_node/_edge match on
-- payload key shape with no type filter, so those keys would leak the grant into any reader's
-- element trail (spec §5, and there is a test).
UPDATE kb_event_types SET payload_schema = $js${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "GrantCreated",
  "type": "object",
  "additionalProperties": false,
  "required": ["subject_table","subject_id","principal_table","principal_id",
               "can_read","can_write","can_delete","can_grant","granted_by"],
  "properties": {
    "subject_table":   {"enum": ["kb_resources","kb_contexts","kb_cogmaps","kb_connections"]},
    "subject_id":      {"type": "string", "format": "uuid"},
    "principal_table": {"enum": ["kb_teams","kb_profiles"]},
    "principal_id":    {"type": "string", "format": "uuid"},
    "can_read":        {"type": "boolean"},
    "can_write":       {"type": "boolean"},
    "can_delete":      {"type": "boolean"},
    "can_grant":       {"type": "boolean"},
    "granted_by":      {"type": "string", "format": "uuid"},
    "previous":        {
      "type": "object",
      "additionalProperties": false,
      "description": "Capabilities before this act, when it replaced an existing grant. Absent on a fresh grant. An upsert that CHANGES capabilities is a real admin act and must not be silently dropped.",
      "required": ["can_read","can_write","can_delete","can_grant"],
      "properties": {
        "can_read":   {"type": "boolean"},
        "can_write":  {"type": "boolean"},
        "can_delete": {"type": "boolean"},
        "can_grant":  {"type": "boolean"}
      }
    }
  }
}$js$::jsonb
WHERE name = 'grant_created';

UPDATE kb_event_types SET payload_schema = $js${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "GrantRevoked",
  "type": "object",
  "additionalProperties": false,
  "required": ["subject_table","subject_id","principal_table","principal_id","revoked_by"],
  "properties": {
    "subject_table":   {"enum": ["kb_resources","kb_contexts","kb_cogmaps","kb_connections"]},
    "subject_id":      {"type": "string", "format": "uuid"},
    "principal_table": {"enum": ["kb_teams","kb_profiles"]},
    "principal_id":    {"type": "string", "format": "uuid"},
    "revoked_by":      {"type": "string", "format": "uuid"}
  }
}$js$::jsonb
WHERE name = 'grant_revoked';

-- ── NULL the two STALE registry rows (task 019f6b48-a562-7871-a48d-87945a796c7e) ──
--
-- This migration is the first thing to touch kb_event_types.payload_schema since the boot-seed, so
-- it is where the registry's other half gets fixed rather than leaving a second pass to schedule.
--
-- PR #464 fixed the REPO side of the payload_schema rot (the committed fixtures match the Rust types
-- again). It could not fix the REGISTRY: kb_event_types rows are stamped ONCE at boot-seed and never
-- re-stamp themselves. So prod's only two non-NULL schemas are both STALE -- they describe an older
-- payload than the code writes:
--
--   region_materialized -- does not know `telos_centroid`  (208 events written)
--   lens_created        -- does not know `TelosConstants`  (3 events written)
--
-- DECISION (2026-07-16, with Pete): NULL them, do not re-stamp. NULL is explicitly legitimate --
-- 20260624000001_canonical_schema.sql:445-447 blesses it as "unregistered/permissive", and 31 of 33
-- types already are. A reader can handle "unregistered"; a reader CANNOT detect "confidently wrong".
-- The emitters spec publishes this registry as the contract for external writers, so a stale schema
-- actively lies to them -- and that stops being theoretical as kb_connections emitters come online.
-- Re-stamping is the other option and was rejected here only because it must be RIGHT to be worth
-- doing, and getting it right is a deliberate registration pass over the whole catalogue, not a
-- rider on this migration.
--
-- Additive-only: touches a metadata column, no shape. Nothing validates payloads against
-- payload_schema at write time (_event_append just resolves the type id and inserts), so this
-- changes no behavior -- it removes a false claim.
UPDATE kb_event_types
   SET payload_schema = NULL
 WHERE name IN ('region_materialized', 'lens_created');
```

> **The recurrence question — answer it in this PR, do not just NULL and move on.**
> The root cause of `019f6b48-a562-7871-a48d-87945a796c7e` is that **nothing re-stamps**: the
> boot-seed does it once, and every later payload change silently drifts the registry from the repo.
> NULLing the two liars removes today's false claim; it does **not** stop the next one. This
> migration itself proves the point — it hand-writes `grant_created`/`grant_revoked` schemas that will
> drift from `payloads.rs` the moment someone edits those structs, and nothing will notice.
>
> This is the same rot that produced `019f6b1b-59ea-7660-b631-3b811aea378d` (`payload_schema` red in
> no CI job) and `scenario-schema` (a feature gating tests that ran nowhere): **a contract with no
> gate is a contract that drifts.** Say plainly in the PR what keeps registry == repo after the next
> payload change — a CI check, a release-ritual step, or a boot-seed re-stamp — or record that it is
> knowingly unsolved and re-file it. Do not leave it implied.
>
> Note the tension worth surfacing: the committed fixtures are generated from the Rust types by
> `UPDATE_SCHEMA=1 cargo make test-schema`, but these migration schemas are **hand-written JSON**.
> Two sources for one contract is the drift, restated. A follow-on that stamps the registry FROM the
> generated fixtures would close it properly; this task is not scoped to build that, but it is the
> honest recommendation.

- [ ] **Step 2: Write the epoch migration**

Create `migrations/20260717000020_admin_ledger_epoch.sql`:

```sql
-- The admin ledger's epoch (spec 2026-07-16 §8).
--
-- NOT a backfill. ~14 admin acts occurred in prod before any writer existed and 8 of them are
-- permanently unreconstructable: kb_teams has no creator column at all, kb_team_members has no
-- actor, and revoked grants were hard-DELETEd. The 6 that "survive" don't either -- the grant
-- upsert overwrites granted_by_profile_id AND sets granted_at = now(), so those columns are a
-- current snapshot, not history. Synthesizing events from them would mint immortal, append-only
-- rows asserting the wrong actor at a fabricated time.
--
-- A partially-backfilled ledger is WORSE than an honestly-empty one: a reader cannot distinguish
-- "no event" from "predates the writer" from "reconstruction with the wrong actor". An empty
-- ledger with an epoch is unambiguous.
--
-- Emitted by the system actor -- the bare `system` entity, which never resolves through
-- resolve_emitter (20260624000003_canonical_seed.sql). Both-NULL producing anchor: the epoch has
-- no cognition home, and neither will any admin event after it.

INSERT INTO kb_events (event_type_id, emitter_entity_id, payload, "references")
SELECT et.id,
       e.id,
       jsonb_build_object(
         'opened_at', to_jsonb(now()),
         'note', 'Admin ledger opens here. No administrative history exists before this event: '
              || 'the acts happened, but no writer recorded them and their actors are not '
              || 'reconstructable from surviving columns.'
       ),
       '[]'::jsonb
  FROM kb_event_types et
  CROSS JOIN kb_entities e
 WHERE et.name = 'admin_ledger_opened'
   AND e.name  = 'system'
   AND NOT EXISTS (
         SELECT 1 FROM kb_events x JOIN kb_event_types xt ON xt.id = x.event_type_id
          WHERE xt.name = 'admin_ledger_opened'
       );
```

- [ ] **Step 3: Apply and verify locally**

```bash
cargo make docker-up
cargo sqlx migrate run
psql "$DATABASE_URL" -c "SELECT t.name, e.payload->>'opened_at' AS opened_at, e.producing_anchor_table FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id WHERE t.name='admin_ledger_opened';"

# The two stale registry rows are NULLed, and nothing else lost a schema.
psql "$DATABASE_URL" -c "SELECT name, payload_schema IS NOT NULL AS has_schema FROM kb_event_types WHERE payload_schema IS NOT NULL ORDER BY name;"
```

Expected: exactly one `admin_ledger_opened` row; `producing_anchor_table` is **NULL**.

Expected from the second query: **exactly `grant_created` and `grant_revoked`** — and nothing else.
`region_materialized` and `lens_created` must be **absent** (NULLed by Step 1). If either still
appears, the NULL did not apply. If a *third* name appears, someone stamped a schema this plan does
not know about — stop and report rather than NULLing it too.

> ⚠️ **A local run does not prove the prod fix.** A fresh local DB is boot-seeded from the current
> repo, so its `region_materialized`/`lens_created` rows may be absent, correct, or stale depending
> on seed state — the NULL can be a no-op locally and still be the whole point in prod, where those
> two rows carry the stale schemas (208 and 3 events respectively). **Verify against prod after
> Pete applies**, exactly as `019f6b06-c48f-7a81-a238-cdd6b131f3dc` did: the guard's behavior on the
> environment that has the bad data is the only verification that counts.

- [ ] **Step 4: Verify idempotency**

```bash
psql "$DATABASE_URL" -f migrations/20260717000020_admin_ledger_epoch.sql
psql "$DATABASE_URL" -c "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id WHERE t.name='admin_ledger_opened';"
```

Expected: `1`. A second run inserts nothing.

- [ ] **Step 5: Assert the epoch reads back through the service**

Append to `crates/temper-services/tests/admin_ledger_test.rs`:

```rust
#[sqlx::test]
async fn the_epoch_is_readable_and_null_anchored(pool: PgPool) {
    let epoch = temper_services::services::admin_ledger_service::ledger_epoch(&pool)
        .await
        .expect("ledger_epoch");
    assert!(epoch.is_some(), "the epoch marker must exist after migration");

    let anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id
          WHERE t.name='admin_ledger_opened' AND e.producing_anchor_table IS NOT NULL",
    )
    .fetch_one(&pool).await.unwrap();
    assert_eq!(anchored, 0, "the epoch must be NULL-anchored — it has no cognition home");
}
```

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test -E 'test(epoch)'`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo make check
git add migrations/20260717000010_admin_event_types.sql \
        migrations/20260717000020_admin_ledger_epoch.sql \
        crates/temper-services/tests/admin_ledger_test.rs
git commit -m "feat(admin-ledger): event types, payload schemas, and the epoch marker

The epoch is NOT a backfill. 8 of ~14 historical admin acts are permanently
unreconstructable (kb_teams has no creator column; revoked grants were
hard-DELETEd) and the 6 that appear to survive don't -- the grant upsert
overwrites granted_by_profile_id and sets granted_at = now(). Synthesizing from
them would mint immortal append-only rows asserting the wrong actor.

grant_created/grant_revoked are not dropped: they are this arc's types and
finally get the payload schemas they never had."
```

---

### Task 5: The grant chokepoint — SQL fns, projectors, replay ownership

> **SHIPPED 2026-07-18. Grounding against `main` changed five things below; read this first.**
>
> 1. **Migration is `20260718000010_admin_grant_fns.sql`, not `…0717000030`.** A sibling session was
>    landing a `…0717000030`, and the day had rolled over — a new-day stamp both avoids the collision
>    and is correct (the migration-collision-reads-as-a-flake trap).
> 2. **A FIFTH caller the plan's "4 callers" missed:** `machine_registration_service::apply_reach`
>    (`:137`) also calls `insert_grant`. Found by grepping every call site (the signature change breaks
>    any missed one). Wired through `apply_reach`.
> 3. **Correlation is DROPPED from the Rust signatures.** Step 6's rationale — "grant_reach mints one
>    CorrelationId and threads it to the affirmation AND the grant" — rests on a false premise:
>    `connection_service` fires **no events at all** (the reach affirmation is a column `UPDATE`), so
>    the grant is the lone event and self-roots correctly. `insert_grant(conn, p, emitter)` /
>    `delete_grant(conn, …, revoker, emitter)` — no `correlation`, no `ctx: EventContext`. The SQL fns
>    keep `p_correlation DEFAULT NULL`, so the capability survives at the SQL layer for any future
>    correlated caller **without** a deploy-skew signature change.
> 4. **Emitter resolution is LAZY where grants are conditional.** `apply_reach` takes
>    `emitter: Option<EntityId>`, resolved `Some` iff `reach.grants()` is non-empty — a pure
>    team-membership provision fires no grant event and must not require the minter to carry a
>    `<handle>@web` entity (a mere gating-team watcher does not). The always-grants paths
>    (`grant_capability`, `revoke_capability`, `grant_reach`) resolve unconditionally.
> 5. **Step 4 (EventKind variants) + Step 5's mechanics already shipped in Task 4** (the replay no-op
>    arms). Task 5 adds only `"kb_access_grants"` to `INPUT_TABLES`.
>
> Fixture fixes: `seed_admin`/`seed_team_member` in the connection/machine tests now provision emitters
> via the production `provision_profile_entities` — a caller that authors a grant event must carry its
> emitter, exactly as production does (the "fixture without emitters passes while production 500s"
> pattern). Verified: services suite 392/392, artifact/replay 281/281, `cargo make check` green.

The proving pair. It catches the generic grant path **and** `connection_service::grant_reach`'s bypass (`connection_service.rs:467`, `:486`), which calls `insert_grant` directly — a service-layer sink would miss it. It also exercises replay ownership end-to-end.

**Files:**
- Create: `migrations/20260717000030_admin_grant_fns.sql`
- Modify: `crates/temper-substrate/src/events.rs` (EventKind + SeedAction arms)
- Modify: `crates/temper-substrate/src/replay.rs:88` (INPUT_TABLES)
- Modify: `crates/temper-services/src/services/access_service.rs:128,159`
- Test: `crates/temper-services/tests/admin_ledger_test.rs`

> **Verified plan/reality notes (2026-07-16):**
> - `grant_capability(pool, caller, req: &GrantCapabilityRequest) -> ApiResult<GrantOutcome>` takes
>   its request **by reference** (`access_service.rs:184`). Same for `revoke_capability` (`:213`).
> - `GrantOutcome { granted: bool }` lives in `temper_core::types::cognitive_maps` (`:295-298`).
>   `insert_grant`'s `ApiResult<bool>` contract must be preserved so this is unchanged.
> - `grant_req`/`revoke_req` are **local helpers you write in the test file** — there is no
>   `common` module in `crates/temper-services/tests/`.
> - `_event_append`'s named params are confirmed: `p_references`, `p_correlation`
>   (`migrations/20260624000002_canonical_functions.sql:765-774`). It `RAISE`s
>   `'event_type % not seeded'` — so Task 4's migration must land before any fire.

**Interfaces:**
- Consumes: `EventRef`/`RefTarget` (Task 1), the event types (Task 4).
- Produces: `EventKind::{AdminLedgerOpened, GrantCreated, GrantRevoked}`; SQL fns `_admin_grant_created`, `_admin_grant_revoked`; `insert_grant(conn, p: &InsertGrantParams, emitter: EntityId, ctx: EventContext) -> ApiResult<bool>`; `delete_grant(conn, subject_table, subject_id, principal_table, principal_id, revoker: ProfileId, emitter: EntityId, ctx: EventContext) -> ApiResult<bool>`.

> **Signature warning:** `delete_grant` gains three parameters. `CREATE OR REPLACE FUNCTION` cannot add a param, and a mutation-fn signature change is a **write outage across deploy skew** on an auto-deploying `main`. The SQL fns here are **new**, so this is safe — but get them right the first time; widening them later is the expensive case.

- [ ] **Step 1: Write the failing test**

```rust
#[sqlx::test]
async fn granting_writes_an_event_and_the_row(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    let outcome = temper_services::services::access_service::grant_capability(
        &pool,
        f.admin_profile,
        &grant_req(f.context_id, f.team_id),
    )
    .await
    .expect("grant_capability");
    assert!(outcome.granted, "a fresh grant reports granted");

    let entries = temper_services::services::admin_ledger_service::list_by_subject(
        &pool, f.admin_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id }, 50, 0,
    ).await.unwrap();

    assert_eq!(entries.len(), 1, "the grant must be on the ledger");
    assert_eq!(entries[0].event_type, "grant_created");
    assert_eq!(entries[0].actor_profile_id, f.admin_profile.uuid());
    assert_eq!(entries[0].payload["subject_table"], "kb_contexts");
    assert!(entries[0].payload.get("resource_id").is_none(), "banned key");
}

#[sqlx::test]
async fn revoking_writes_an_event_even_though_the_row_is_deleted(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    temper_services::services::access_service::grant_capability(
        &pool, f.admin_profile, &grant_req(f.context_id, f.team_id)).await.unwrap();
    temper_services::services::access_service::revoke_capability(
        &pool, f.admin_profile, &revoke_req(f.context_id, f.team_id)).await.unwrap();

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_access_grants WHERE subject_id=$1")
        .bind(f.context_id).fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 0, "revoke still hard-DELETEs the row — the row is the projection");

    let entries = temper_services::services::admin_ledger_service::list_by_subject(
        &pool, f.admin_profile,
        RefTarget { kind: AnchorTable::Contexts, id: f.context_id }, 50, 0,
    ).await.unwrap();

    assert_eq!(entries.len(), 2, "the ledger keeps BOTH acts — this is the whole point");
    assert_eq!(entries[0].event_type, "grant_revoked", "newest first");
    assert_eq!(entries[1].event_type, "grant_created");
}

#[sqlx::test]
async fn the_connection_grant_reach_bypass_is_also_on_the_ledger(pool: PgPool) {
    // connection_service::grant_reach calls access_service::insert_grant DIRECTLY, bypassing
    // grant_capability (connection_service.rs:467). A service-layer sink would miss it; the
    // chokepoint must not.
    let f = connection_fixture(&pool).await;

    temper_services::services::connection_service::grant_reach(
        &pool, f.admin_profile, f.connection_id, f.team_id, None,
    ).await.expect("grant_reach");

    let entries = temper_services::services::admin_ledger_service::list_by_subject(
        &pool, f.admin_profile,
        RefTarget { kind: AnchorTable::Connections, id: f.connection_id }, 50, 0,
    ).await.unwrap();

    assert_eq!(entries.len(), 1, "grant_reach's bypass must still reach the ledger");
    assert_eq!(entries[0].payload["subject_table"], "kb_connections");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test -E 'test(granting) or test(revoking) or test(bypass)'`
Expected: FAIL — no events written.

- [ ] **Step 3: Write the SQL functions**

Create `migrations/20260717000030_admin_grant_fns.sql`:

```sql
-- The grant chokepoint, SQL-resident (spec 2026-07-16 §7).
--
-- WHY SQL AND NOT RUST: cognition events are not fired from Rust alongside a Rust write --
-- fire() dispatches a SeedAction to a SQL function that appends the event AND projects, in one
-- txn (_event_append, canonical_functions.sql:765). Admin acts follow the same shape. A
-- Rust service-layer sink would also MISS connection_service::grant_reach, which bypasses
-- grant_capability and calls insert_grant directly (connection_service.rs:467).
--
-- BOTH-NULL PRODUCING ANCHOR, always. A grant is an authority act; it has no cognition home even
-- when its subject IS a context. Anchoring it would put it in front of every region producer and
-- break the "governance is traceable, but it isn't knowledge" boundary.
--
-- The payload spells the subject `subject_table`/`subject_id`, NEVER `resource_id`/`owner`:
-- element_trail_node/_edge match on payload key shape with no type filter and are gated only by
-- resources_visible_to, so those keys would leak the grant to any reader of the resource.

CREATE FUNCTION _admin_grant_created(
    p_emitter         uuid,
    p_subject_table   text,
    p_subject_id      uuid,
    p_principal_table text,
    p_principal_id    uuid,
    p_can_read        boolean,
    p_can_write       boolean,
    p_can_delete      boolean,
    p_can_grant       boolean,
    p_granted_by      uuid,
    p_correlation     uuid DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_prev     jsonb := NULL;
    v_inserted boolean;
    v_payload  jsonb;
BEGIN
    -- Capture the prior capabilities BEFORE the upsert overwrites them. An upsert that changes
    -- capabilities returns inserted = false, so keying emission on that bool alone would silently
    -- drop a real authority change. The event carries before/after instead.
    SELECT jsonb_build_object('can_read', can_read, 'can_write', can_write,
                              'can_delete', can_delete, 'can_grant', can_grant)
      INTO v_prev
      FROM kb_access_grants
     WHERE subject_table = p_subject_table AND subject_id = p_subject_id
       AND principal_table = p_principal_table AND principal_id = p_principal_id;

    INSERT INTO kb_access_grants
        (subject_table, subject_id, principal_table, principal_id,
         can_read, can_write, can_delete, can_grant, granted_by_profile_id)
    VALUES (p_subject_table, p_subject_id, p_principal_table, p_principal_id,
            p_can_read, p_can_write, p_can_delete, p_can_grant, p_granted_by)
    ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
    DO UPDATE SET can_read = EXCLUDED.can_read, can_write = EXCLUDED.can_write,
                  can_delete = EXCLUDED.can_delete, can_grant = EXCLUDED.can_grant,
                  granted_by_profile_id = EXCLUDED.granted_by_profile_id, granted_at = now()
    RETURNING (xmax = 0) INTO v_inserted;

    v_payload := jsonb_build_object(
        'subject_table', p_subject_table, 'subject_id', p_subject_id,
        'principal_table', p_principal_table, 'principal_id', p_principal_id,
        'can_read', p_can_read, 'can_write', p_can_write,
        'can_delete', p_can_delete, 'can_grant', p_can_grant,
        'granted_by', p_granted_by);
    IF v_prev IS NOT NULL THEN
        v_payload := v_payload || jsonb_build_object('previous', v_prev);
    END IF;

    PERFORM _event_append(
        'grant_created', p_emitter, NULL, NULL, v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind', p_subject_table,   'id', p_subject_id)),
            jsonb_build_object('rel','principal','target', jsonb_build_object('kind', p_principal_table, 'id', p_principal_id))),
        p_correlation => p_correlation);

    RETURN v_inserted;
END;
$$;

CREATE FUNCTION _admin_grant_revoked(
    p_emitter         uuid,
    p_subject_table   text,
    p_subject_id      uuid,
    p_principal_table text,
    p_principal_id    uuid,
    p_revoked_by      uuid,
    p_correlation     uuid DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_deleted boolean := false;
BEGIN
    DELETE FROM kb_access_grants
     WHERE subject_table = p_subject_table AND subject_id = p_subject_id
       AND principal_table = p_principal_table AND principal_id = p_principal_id;
    GET DIAGNOSTICS v_deleted = ROW_COUNT;
    v_deleted := (v_deleted::int > 0);

    -- Emit only when something was actually revoked: a no-op revoke is not an admin act, and the
    -- ledger is append-only -- a spurious row can never be corrected, only quarantined.
    IF v_deleted THEN
        PERFORM _event_append(
            'grant_revoked', p_emitter, NULL, NULL,
            jsonb_build_object(
                'subject_table', p_subject_table, 'subject_id', p_subject_id,
                'principal_table', p_principal_table, 'principal_id', p_principal_id,
                'revoked_by', p_revoked_by),
            p_references => jsonb_build_array(
                jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind', p_subject_table,   'id', p_subject_id)),
                jsonb_build_object('rel','principal','target', jsonb_build_object('kind', p_principal_table, 'id', p_principal_id))),
            p_correlation => p_correlation);
    END IF;

    RETURN v_deleted;
END;
$$;

COMMENT ON FUNCTION _admin_grant_created IS
  'Grant upsert + grant_created event, one txn. Both-NULL producing anchor: a grant is an authority act with no cognition home, even when its subject is a context. Carries `previous` when it replaced an existing grant -- an upsert that changes capabilities returns inserted=false, so the bool alone would drop a real authority change.';

COMMENT ON FUNCTION _admin_grant_revoked IS
  'Grant DELETE + grant_revoked event, one txn. The DELETE stays: the row is the current-state projection, the ledger is the temporal record (access spec §3.7). Emits only when a row was actually deleted -- kb_events is append-only and a spurious event is immortal.';
```

- [x] **Step 4: Add the EventKind variants and projectors** — **DONE IN TASK 4** (folded forward so
  Task 4's epoch event has a known type; the variants + replay no-op arms shipped in that PR). The
  original instructions are retained below for the record.

In `crates/temper-substrate/src/events.rs`, add to `EventKind`, `as_canonical_name`, and `from_canonical_name` (all three — `from_canonical_name` is documented as the exact inverse):

```rust
    AdminLedgerOpened,
    GrantCreated,
    GrantRevoked,
```

```rust
            EventKind::AdminLedgerOpened => "admin_ledger_opened",
            EventKind::GrantCreated => "grant_created",
            EventKind::GrantRevoked => "grant_revoked",
```

```rust
            "admin_ledger_opened" => EventKind::AdminLedgerOpened,
            "grant_created" => EventKind::GrantCreated,
            "grant_revoked" => EventKind::GrantRevoked,
```

**This is mandatory, not optional.** `replay::replay` (`replay.rs:332-345`) walks every `kb_events` row and does `EventKind::from_canonical_name(&name)` with a `?` — an unknown type is a **hard replay failure**. The moment Task 4's epoch event exists, replay breaks without these variants.

- [ ] **Step 5: Give `kb_access_grants` to replay as an input table**

In `crates/temper-substrate/src/replay.rs`, add to `INPUT_TABLES` (the list at `:88`), after `"kb_team_contexts"`:

```rust
    "kb_access_grants",
```

`INPUT_TABLES` is copied verbatim into the replay namespace, and the projectors are **idempotent re-apply** — the shape `context_reassigned` already uses. Replay walks `ORDER BY e.id` (UUIDv7, time-sortable), so `grant_created` re-applies and a later `grant_revoked` deletes: net state correct.

**Not `PROJECTION_DUMPS`** — that would make replay rebuild grants from events and diff against live, and the 5 pre-epoch grants have no events, so it would report them as spurious forever.

- [ ] **Step 6: Wire the service layer to the SQL fns**

In `crates/temper-services/src/services/access_service.rs`, replace `insert_grant`'s body (`:128`) and `delete_grant`'s (`:159`) with calls to the SQL fns. `insert_grant` keeps its `ApiResult<bool>` contract, so `grant_capability`'s `GrantOutcome { granted }` is unchanged:

```rust
/// Raw grant upsert + `grant_created` event, one txn. **No authorization** — every caller must
/// gate first (unchanged from before; the event records what happened, and it only happens after
/// the caller's gate).
pub async fn insert_grant(
    conn: &mut sqlx::PgConnection,
    p: &InsertGrantParams,
    emitter: EntityId,
    correlation: Option<CorrelationId>,
) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT _admin_grant_created($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) AS "inserted!""#,
        emitter.uuid(),
        p.subject_table,
        p.subject_id,
        p.principal_table,
        p.principal_id,
        p.can_read,
        p.can_write,
        p.can_delete,
        p.can_grant,
        p.granted_by_profile_id,
        correlation.map(CorrelationId::uuid),
    )
    .fetch_one(&mut *conn)
    .await?)
}
```

`delete_grant` gains `revoker`, `emitter`, and `correlation` and returns whether a row was removed:

```rust
pub async fn delete_grant(
    conn: &mut sqlx::PgConnection,
    subject_table: &str,
    subject_id: Uuid,
    principal_table: &str,
    principal_id: Uuid,
    revoker: ProfileId,
    emitter: EntityId,
    correlation: Option<CorrelationId>,
) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT _admin_grant_revoked($1,$2,$3,$4,$5,$6,$7) AS "deleted!""#,
        emitter.uuid(),
        subject_table,
        subject_id,
        principal_table,
        principal_id,
        revoker.uuid(),
        correlation.map(CorrelationId::uuid),
    )
    .fetch_one(&mut *conn)
    .await?)
}
```

Update the four callers. `grant_capability`/`revoke_capability` (`:184`, `:213`) resolve an emitter from their `caller` via `temper_substrate::writes::resolve_emitter(pool, caller, "web")` — the shape `context_service::reassign` uses (`context_service.rs:549`) — and pass `None` for correlation.

`connection_service::grant_reach` (`:467`, `:486`) passes `Some(correlation)`: **it mints one `CorrelationId` and threads it to both** the affirmation and the grant, because the two are one act in one txn (`:449-450` — "never affirmation-without-grant or grant-without-affirmation"). `_event_append` defaults `p_correlation` to `COALESCE(p_correlation, v_ev)`, so each event would otherwise **self-root** and the fusion would be lost.

`connection_service::revoke_reach` (`:542`) passes the caller as `revoker`.

- [ ] **Step 7: Run the tests**

Run: `cargo nextest run -p temper-services --features test-db --test admin_ledger_test`
Expected: PASS (all 8 tests).

- [ ] **Step 8: Prove replay still works**

The single highest-risk regression: admin events now exist and `replay` walks every row.

Run: `cargo make test-artifacts`
Expected: PASS. If replay fails with `no projector for event type admin_ledger_opened`, Step 4 was skipped or incomplete.

- [ ] **Step 9: Full DB-backed suite**

Run: `cargo make test-db && cargo make test-e2e`
Expected: PASS. Trust the **exit code**, not any per-binary Summary line.

- [ ] **Step 10: Regenerate sqlx caches and commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make prepare-e2e
cargo make check
git add migrations/20260717000030_admin_grant_fns.sql \
        crates/temper-substrate/src/events.rs \
        crates/temper-substrate/src/replay.rs \
        crates/temper-services/src/services/access_service.rs \
        crates/temper-services/src/services/connection_service.rs \
        crates/temper-services/tests/admin_ledger_test.rs \
        .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx tests/e2e/.sqlx
git commit -m "feat(admin-ledger): grant chokepoint — the first event-sourced admin act

The proving pair. Installed at insert_grant/delete_grant rather than
grant_capability because connection_service::grant_reach bypasses the latter
and calls insert_grant directly — a service-layer sink would have missed it on
day one, which is precisely how five surfaces came to decline admin-as-events.

kb_access_grants joins INPUT_TABLES with idempotent re-apply projectors (not
PROJECTION_DUMPS: the 5 pre-epoch grants have no events and would diff as
spurious forever). The hard DELETE stays — the row is the projection, the
ledger is the temporal record, which is what access spec §3.7 said in June.

grant_created carries `previous` capabilities: an upsert that CHANGES
capabilities returns inserted=false, so the bool alone would drop a real
authority change."
```

---

### Task 5b: Authorization hardening — the gaps the Task 5 access trace surfaced

> **ADDED 2026-07-18, before merging Task 5's PR (#482, moved to DRAFT).** Pete asked for a full
> identity → authorization → level-of-access trace across every surface this arc touches, on fresh
> context. Three parallel read-only traces (access-grant surface; machine + connection families;
> ledger read gate) plus an empirical probe against the live DB. **None of these is a Task 5
> regression** — all five pre-exist on `main`, verified by `git show main:…`. But four of them are
> merge-blocking by Pete's call: the arc asserts a *chokepoint*, and a chokepoint with known holes is
> a claim, not an invariant.

**Grounding established by the trace (cite these, don't re-derive):**

- The SQL chokepoints are **`SECURITY INVOKER` and perform zero authorization** — verified live
  (`prosecdef = f` for `_admin_grant_created` / `_admin_grant_revoked`). All authz is Rust-side. The
  `insert_grant` doc comment ("**Performs no authorization** — every caller must gate first") is an
  accurate contract, not aspiration.
- **Two authorization regimes write `kb_access_grants`**: `can_administer_grant` (admin OR explicit
  `can_grant` OR resource-home owner) for the resource/cogmap surfaces, and
  `machine_authz::authorize` (admin OR owner of the relevant team) for machine + connection reach.
- `is_system_admin` requires `tm.role = 'owner'` on the gating team — a gating-team *member* is not
  an admin. `has_system_access` is **vacuous under `access_mode = 'open'`**, which is prod's setting,
  so the router's `require_system_access` layer contributes nothing; the service gate is the only
  real one.
- Auth is strictly before write on all five `insert_grant`/`delete_grant` callers — confirmed by
  reading each, not by trusting the comments.

**5b.1 — Genesis bypasses the chokepoint (CONFORM to Task 5's own premise).**
`db_backend.rs:2157` writes a `can_grant`-bearing bootstrap row with a raw `INSERT INTO
kb_access_grants`, so a grant conferring grant-authority lands with **no `grant_created` event**.
Scope-limited (self-grant, same txn, `ON CONFLICT DO NOTHING`) so it is not an escalation — it is an
audit-completeness hole against "the event now lives INSIDE insert_grant". Route it through
`insert_grant`; `tx` and `emitter` are both already in scope at that call site.

**5b.2 — `grant_reach` never validates the GRANTEE team (AMEND).**
`connection_service.rs:442` keys authorization solely on `connection.owner_team_id`; the caller-
supplied `team_id` flows unchecked into `reach_grant_params` (`:472`, `:495`) and into the grant.
`principal_id` carries **no FK**, so a bogus UUID writes a dangling row. The machine path already
bounds its target teams (`machine_authz.rs:140-143`); this one does not.
**DECISION (Pete, 2026-07-18): mirror `contain_reach`** — the caller needs a manage-capable role on
the target team, admins exempt. Same treatment on `revoke_reach` (`:547` → `:553`).

**5b.3 — No capability attenuation (AMEND — this is a deliberate policy change).**
Proven empirically, not inferred: a principal holding `read+grant` and nothing else called the
chokepoint and minted itself `write+delete+grant` (before: `b_grant=t, b_write=f`; after:
`b_write=t, b_delete=t`). `grant_capability` copies the request verbatim (`access_service.rs:214-217`)
after a gate that only asks `'grant'`.
**DECISION (Pete, 2026-07-18): strict attenuation with admin-only amplification** — a non-admin may
confer only capabilities they themselves hold on that subject; gating-team owners stay unrestricted
so bootstrap and repair remain operable.
⚠️ **This rewrites an existing intentional test.** `access_grants_test.rs:236-262` currently asserts
delegate-mints-write-for-a-third-party as *desired* behavior. Under strict attenuation that case
becomes `Forbidden`. Rewrite the test to assert the new policy and keep a case proving admins still
amplify — do not delete the coverage.

**5b.4 — `require_cogmap_write_admin` is never consulted by the grant path (CONFORM).**
It exists to force `is_system_admin` for the reserved L0 kernel and gating-team-joined maps, and is
documented fail-CLOSED (`access_service.rs:266-270`). `grant_capability`/`revoke_capability` call
only `can_administer_grant`, so a non-admin `can_grant` holder on L0 can mint `can_write` on the
kernel. That this state is reachable is shown by `machine_authz.rs:386-392`, which seeds exactly such
a row. Consult it on both grant and revoke when the subject is a cogmap.

**5b.5 — The element-trail firewall is convention + tests, not a runtime filter (EXTEND).**
`element_trail_node`/`_edge` match on payload key shape with **no** event-type filter and are gated
only by `resources_visible_to`. Today the invariant holds solely because admin payloads don't spell
`resource_id`/`owner`/`block_id`/`edge_id` — a naming convention guarded by two (good, non-vacuous)
tests that cannot see a writer added outside the test corpus.
**DECISION (Pete, 2026-07-18): do BOTH, with anchor nullity as the primary signal** — it was the
original design signal, and it is semantically exact: both-NULL anchor means "has no cognition home",
which is the trail's inclusion criterion inverted. Registry classification is the belt-and-braces a
reviewer can actually see, since nullity alone is invisible at the call site.
- Exclude both-NULL-anchor events from `element_trail_node`/`_edge`.
- Add a category/`is_admin` classification to `kb_event_types`, stamp the three admin types, exclude
  by join.

> **Precondition for the nullity filter — DISCHARGED empirically against the full prod corpus
> (read-only, 2026-07-18):** only two event types are both-NULL anchored in prod — `lens_created` (3)
> and `admin_ledger_opened` (1) — and **neither spells a trail-matched key**, so the filter drops
> zero legitimate trail entries. This is an existence proof over *current* data, not over all future
> events; that limit is precisely why the paired registry classification is not redundant.

> **NOTE — the `kb_event_types` change intersects open work.** Prod's registry is already mid-
> reconciliation (13 of 18 typed names have a NULL `payload_schema`; task
> `019f7509-7511-7d90-9bdb-2e08631208e7`). Adding a column is additive and safe on `main`, but
> sequence the *stamping* against that task rather than assuming a fresh-migrate shape.

**Not merge-blocking — carried to a follow-on task:** two stale comments that actively mislead an
auditor. `routes.rs:171-173` claims machine-client routes are gated by `is_system_admin` in the
*handler*; the real gate is `admin OR team owner`, in the *service* (only `rebind` matches the
comment). `admin_ledger_service.rs:135-137` claims upstream temper-api/temper-mcp gates that do not
exist for that service — its `has_system_access` check is the only gate, and it is vacuous under
`access_mode = 'open'`.

---

### Task 6: API + CLI + MCP parity

Full surface parity is always intended. The read surface is useless if only Rust can reach it.

**Files:**
- Create: `crates/temper-api/src/handlers/admin_ledger.rs`
- Modify: `crates/temper-api/src/routes.rs`, `crates/temper-api/src/handlers/mod.rs`
- Create: `crates/temper-cli/src/commands/admin_ledger.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`, the `admin` subcommand tree
- Create: `crates/temper-mcp/src/tools/admin_ledger.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`
- Test: `tests/e2e/tests/admin_ledger_e2e.rs`

**Interfaces:**
- Consumes: `admin_ledger_service::{list_by_subject, list_by_actor, ledger_epoch, AdminLedgerEntry}` (Task 2).
- Produces: `GET /api/admin/ledger?subject_kind=&subject_id=&actor=&limit=&offset=`; `temper admin ledger --subject <kind>:<uuid> | --actor <ref>`; MCP tool `admin_ledger`.

- [ ] **Step 1: Write the failing e2e test**

Create `tests/e2e/tests/admin_ledger_e2e.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

/// At the production caller's level: a real server, a real grant over HTTP, read back over HTTP.
#[tokio::test]
async fn a_grant_made_over_http_is_readable_on_the_ledger_over_http() {
    let h = common::harness().await;
    let admin = h.admin_client().await;

    let ctx = admin.create_context("audit-me").await.expect("create context");
    admin.grant_context_to_team(ctx.id, h.team_id, "read").await.expect("grant");

    let ledger = admin
        .get_admin_ledger_by_subject("kb_contexts", ctx.id)
        .await
        .expect("read ledger");

    assert_eq!(ledger.entries.len(), 1, "the grant must be on the ledger");
    assert_eq!(ledger.entries[0].event_type, "grant_created");
    assert!(ledger.epoch.is_some(), "the response must carry the epoch");
}

#[tokio::test]
async fn a_non_admin_gets_404_from_the_ledger() {
    let h = common::harness().await;
    let outsider = h.outsider_client().await;
    let err = outsider
        .get_admin_ledger_by_subject("kb_contexts", h.some_context_id)
        .await
        .expect_err("outsider must not read the ledger");
    assert_eq!(err.status(), Some(404), "reads deny with 404, not 403");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo make test-e2e`
Expected: FAIL — no such route.

- [ ] **Step 3: Implement the handler**

`crates/temper-api/src/handlers/admin_ledger.rs` — transport only; all logic is in the service (read paths stay service-direct by design). The response DTO carries the epoch alongside the entries, so a reader **never** mistakes an empty list for "nothing happened":

```rust
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct AdminLedgerResponse {
    pub entries: Vec<AdminLedgerEntry>,
    /// When the admin ledger opened. Entries before this do not exist — the acts happened, but
    /// no writer recorded them (spec §8). An empty `entries` with an `epoch` means "nothing since
    /// T", never "nothing ever".
    pub epoch: Option<chrono::DateTime<chrono::Utc>>,
}
```

Register the route in `routes.rs` under `/api/admin/ledger` behind the existing auth middleware.

- [ ] **Step 4: Implement CLI + MCP**

CLI `temper admin ledger --subject kb_contexts:<uuid>` / `--actor <ref>`, routed through `temper-client` over HTTP (the CLI never calls services directly). MCP tool `admin_ledger` with the same two axes, delegating to `admin_ledger_service`.

- [ ] **Step 5: Regenerate the router's artifacts**

OpenAPI, the temper-rb gem, and temper-ts's `schema.ts` are **all products of the router**. A new response DTO stales all three:

```bash
cargo make openapi
git add openapi.json clients/temper-rb/lib/temper/generated clients/temper-ts/src/generated/schema.ts
```

The drift gates compare against **git**, not a fresh build — a correctly regenerated artifact still fails `cargo make check` while unstaged. Stage first, then check.

- [ ] **Step 6: Run everything**

```bash
cargo make check
cargo make test-e2e-embed
```

Expected: PASS. Use `test-e2e-embed`, not `test-e2e` — the latter silently compiles out every `test-embed`-gated test, and CI enables it.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/handlers/admin_ledger.rs crates/temper-api/src/handlers/mod.rs \
        crates/temper-api/src/routes.rs \
        crates/temper-cli/src/commands/admin_ledger.rs crates/temper-cli/src/commands/mod.rs \
        crates/temper-mcp/src/tools/admin_ledger.rs crates/temper-mcp/src/tools/mod.rs \
        tests/e2e/tests/admin_ledger_e2e.rs \
        openapi.json clients/temper-rb/lib/temper/generated clients/temper-ts/src/generated/schema.ts
git commit -m "feat(admin-ledger): API + CLI + MCP read parity

The response carries the epoch alongside the entries so an empty list reads as
'nothing since T', never 'nothing ever' — the distinction a partially-honest
audit log cannot make, and the reason the backfill was withdrawn."
```

---

### Task 7: Amend the published docs

The claim becomes true here, and **not before**. The docs are wrong in a way RETIRE would not have fixed either: they name the wrong *mechanism*.

**Files:**
- Modify: `docs/cognitive-maps/07-operating-temper.md:95-96`
- Modify: `docs/cognitive-maps/07b-governance-and-administration.md` (frontmatter, `:17-18`, `:58-59`, `:70`)
- Modify: `docs/superpowers/specs/2026-07-13-external-systems-as-subscribed-emitters-design.md:467`

- [ ] **Step 1: Fix the mechanism error**

`07-operating-temper.md:96` says admin acts are *"events, with an emitter and **a producing anchor**"*. The anchor is exactly what admin events must **not** have — and the same paragraph's next sentence promises they *"do not participate in cognitive maps"*, which the **NULL** anchor is what delivers. Anyone implementing from this line would have built the leak. Replace with wording that names the real mechanism: an emitter, and **no** producing anchor, which is what keeps them out of maps.

Apply the same fix at `07b:58-59`.

- [ ] **Step 2: Upgrade "firewalled by intent"**

`07b:70` reads *"The two live on the same ledger, firewalled by intent."* That was honest when written. It is now **firewalled by construction** — the NULL anchor is a structural property every region producer, `steward_ingest_delta`, and materialize attribution respect, and Task 2 has a test for it.

- [ ] **Step 3: Scope the "settled" claim honestly**

`07-operating-temper.md:95` and `07b:17-18` claim *every* administrative act is an event. After this plan, **one** is (the grant pair). Scope the claim to what ships, and say the rest is in flight. Do not restore the overclaim — that is what produced this task.

Note `07b`'s lead example is *"creating a team"* — which is **not** in this plan and whose history is unreconstructable (`kb_teams` has no creator column). Pick an example that is true.

- [ ] **Step 4: Fix the emitters spec's false premise**

`2026-07-13-external-systems-as-subscribed-emitters-design.md:467` says *"consistent with the existing admin-event-sourcing shape."* There was no existing shape. Point it at the 2026-07-16 spec.

The `07b` visualization placeholder — admin events flowing into *"a separate channel that does not feed the cognitive maps"* — **survives as-is**. NULL-anchoring implements it faithfully.

- [ ] **Step 5: Commit**

```bash
cargo make check
git add docs/cognitive-maps/07-operating-temper.md \
        docs/cognitive-maps/07b-governance-and-administration.md \
        docs/superpowers/specs/2026-07-13-external-systems-as-subscribed-emitters-design.md
git commit -m "docs: admin-as-events — name the real mechanism, scope the claim

The docs said admin acts carry 'an emitter and a producing anchor'. The anchor
is exactly what they must NOT have: the NULL anchor is what delivers the
'do not participate in cognitive maps' boundary the same paragraph promises.
A literal implementation of that line would have built the leak.

'Firewalled by intent' becomes 'by construction' — it is now a structural
property with a test. The 'settled' claim is scoped to the grant pair that
actually ships."
```

---

## Follow-on tasks (create in temper, do not build here)

- **The remaining authority tier** (§6): machine provision/rebind/revoke/rotate, connection provision/revoke/attach_credential/grant-reach/affirm, `change_role`, `promote_admin`, `update_system_settings`, cogmap bind/unbind, context share/unshare, join-request review. Its own plan, written against this one's proven pattern. **Thread the actor into `promote_admin` and `update_system_settings`** — they take no `caller` today, which is a plumbing gap, not an auth hole.
- **The principal-lifecycle tier** (§6): team create/delete/add_member/remove_member, invitations, SAML reconcile (whose actor is a system reconciler, not a profile — the actor model must handle it).
- **`kb_teams.created_by`** — additive column so future teams record a creator independent of the sink (§11).
- ~~**Does a live profile-creation path still skip `provision_profile_entities`?**~~ **ANSWERED 2026-07-16 — no, and no task is needed.** Traced during `019f6b06`: the human path provisions (`profile_service.rs:155`), both machine paths provision (`machine_registration_service.rs:237`/`:295`), and `connection_service.rs:136` deliberately does *not* — it mints `<handle>@webhook` inline for a genuinely different shape, which is correct, not a gap. Every other `INSERT INTO kb_profiles` is test-only. The backfill is not a treadmill. **Related and worth knowing for Task 5:** sign-in cannot heal a missing emitter either — `resolve_or_create_from_claims` provisions *only* on its brand-new-profile branch; an existing profile returns early from the auth-link lookup or email reconciliation.

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| §4 anchor rule (NULL for authority acts) | Task 5 SQL fns; Task 4 epoch; asserted in Tasks 2, 4 |
| §5 firewall is structural | Task 2 Step 5 (`the_admin_event_is_invisible_to_cognition`) |
| §5 read path on `references`, two axes | Tasks 1, 2; index proven in Task 2 Step 6 |
| §5 `element_trail` invariant + test | Task 3 |
| §5 read authorization | Task 2 `gate()` — **corrected 2026-07-16**: dispatches per event type on the *actual* write gate; the original single `machine_authz::authorize` was refuted |
| §6 catalogue | Task 5 (grant pair only); rest is follow-on, per scope |
| §7 SQL-resident writers | Task 5 |
| §7 replay ownership (`INPUT_TABLES`) | Task 5 Step 5; verified Step 8 |
| §7 correlation threading | Task 5 Step 6 (`grant_reach`) |
| §7 `EventKind` + projectors or replay breaks | Task 5 Step 4; verified Step 8 |
| §7 emitter prerequisite | Extracted → `019f6b06-c48f-7a81-a238-cdd6b131f3dc`; noted as a dependency |
| §8 epoch, no backfill | Task 4 |
| §9 sequencing | Task order (read → invariant → epoch → writer → surfaces → docs) |
| §10 doc amendments | Task 7 |
| §11 open questions | Follow-on list |

**Placeholder scan:** one deliberate `todo!()` in Task 2 Step 2's fixture, explicitly flagged with what to read and why hand-INSERTing profiles would produce a lying fixture. Every other step carries real code or a real command.

**Type consistency:** `RefTarget` is used consistently in Tasks 1, 2, 5. `AdminLedgerEntry` fields match between Task 2's definition and Tasks 5/6's uses. `insert_grant`'s `ApiResult<bool>` contract is preserved so `GrantOutcome { granted }` is unchanged. `ADMIN_EVENT_TYPES` (service) and `ADMIN_EVENT_TYPES_FOR_TEST` (tests) are duplicated deliberately — the test must not import the constant it is guarding, or a wrong constant would pass its own test.
