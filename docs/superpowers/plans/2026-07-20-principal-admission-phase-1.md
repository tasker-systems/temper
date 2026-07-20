# Principal Admission — Phase 1 (additive) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the question *"may this principal use this instance?"* exactly one owner — a persisted `Standing` lifecycle plus a pure, per-request `Admission` machine — and cut every existing access path over to it without changing observable auth behaviour for active principals.

**Architecture:** Two machines (spec D1). A new **`temper-principal`** crate holds the pure decision logic with no `sqlx` and no database types — purity enforced by the compiler. Persistence is a new `kb_principal_standing` row table plus an append-only `kb_principal_standing_events` log, mutated only through one plpgsql function per transition that writes row + log + `kb_events` in a single transaction. `has_system_access` and `is_system_admin` are repointed to read standing and governance state respectively; every existing caller is unchanged because both are SQL functions and the repoint happens in their bodies.

**Tech Stack:** Rust (new crate `temper-principal`; changes in temper-services, temper-substrate, temper-api, temper-cli), PostgreSQL 18 local / 17 Neon (sqlx migrations), cargo-make + cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-07-20-principal-admission-state-machine-design.md`. Read §16 (status) → §3 (D1–D17) → §6 (the transition table) before starting. **§6's transition table is authoritative over §5's diagram** (spec §5).

**Task:** `019f7f61-c9d0-75d1-94dc-c0644f47a6a7`. Plan/large. Branch: cut a fresh branch off `main` (see Global Constraints).

---

## Global Constraints

- **`main` @ `8a77bf46`.** Every symbol, line number, and DDL excerpt in this plan was grepped or queried against that commit and against local dev Postgres on 2026-07-20 before the plan was written. Treat the citations as pre-grounded (per `implementation-grounding.md` GD-1); treat anything *not* cited as unverified and check it yourself.
- **Phase 1 is additive on schema.** No `DROP COLUMN`, no `DROP TRIGGER`, no `DROP INDEX` in this PR. `kb_profiles.system_access`, `kb_profiles.is_active`, `kb_join_requests.status`, `kb_system_settings.access_mode`, and `trg_sync_system_membership` all survive Phase 1 and are dropped by the separate Phase 2 plan. This is what lets Phase 1 ride auto-deploy under the additive-only-on-`main` invariant (`DEPLOYING.md`).
- **New migrations are numbered `20260720000010`, `20260720000020`, …** The highest on disk is `20260719000020_slack_disconnect_event.sql`. Never edit a shipped migration — they are sqlx checksum-locked.
- **Migrations use `uuid_generate_v7()`, never native `uuidv7()`.** Native breaks Neon PG17.
- **Every new event type must spell `category` explicitly.** The `DEFAULT` was dropped in `20260719000010`; an unstamped registration fails `23502` at apply time, and `crates/temper-services/tests/admin_ledger_test.rs:1002` pins that.
- **All new SQL predicates must be total.** `EXISTS(SELECT 1 … WHERE …)`, never `SELECT state = '…' FROM …`. Measured on local dev 2026-07-20: with no matching row the scalar form returns `NULL`, and `IF NOT <null>` **does not enter the branch** — it falls through, fail-**open**. Full measurement in *Grounding* below.
- **No `_ =>` arm in any `match` over `Standing` or `Act`.** Spec §7 obligation 3: adding a state must become a compile error at every decision site. That is the entire reason the crate is separate.
- **Regenerate the sqlx cache after any SQL change:** `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services` and `cargo make prepare-e2e` if test-target queries changed. Per-crate last.
- **`cargo make check` must pass before every commit.** It forces `SQLX_OFFLINE=true`, so it is the honest local probe of the committed caches.
- **Payload schema regen is package-scoped:** `UPDATE_SCHEMA=1 cargo make test-schema`. The task is already `-p temper-substrate` scoped (`tools/cargo-make/main.toml:38-62`) — do **not** add `--workspace`; the emitted shape differs under feature unification and the workspace shape is one nothing ever stamps.

---

## Grounding — read this before Task 1

Per `implementation-grounding.md` GD-1, the evidence comes first and the tasks cite it. Six spec claims did not survive verification; four are corrections you must not "fix back", and two are constraints the spec did not anticipate.

### C1 — §7's type-state premise is false (affects Task 3)

§7 says `AdmittedPrincipal` will preserve "the type-state guarantee `SystemAuthorized` has today." There is no such guarantee today.

```rust
// crates/temper-services/src/auth/mod.rs:258-263
/// Proof that a profile passed **both** levels: authenticated *and*
/// system-authorized. Only obtainable from [`require_system_access`], which
/// only accepts an [`AuthenticatedProfile`] — so the type makes it impossible
/// to run Level 2 without having passed Level 1.
#[derive(Debug)]
pub struct SystemAuthorized(pub AuthenticatedProfile);
```

```rust
// crates/temper-core/src/types/auth.rs:63-66
pub struct AuthenticatedProfile {
    pub profile: Profile,
    pub claims: AuthClaims,
}
```

`grep -rn 'impl AuthenticatedProfile\|impl SystemAuthorized' crates/` → **exit 1, no matches.** Both have fully public fields and no constructor, so any crate can build either by struct literal. The doc comment asserts a property the type does not enforce.

**Consequence:** Task 3 must *add* the guarantee to `AdmittedPrincipal` (private field + crate-only constructor), not inherit it. **Decision (Pete, 2026-07-20): enforce on the new type only.** `SystemAuthorized` and `AuthenticatedProfile` keep their current shape; the gap is filed as follow-up work (see *Follow-ups*), not fixed here.

### C2 — §11 is wrong about `is_active`: dropping it breaks **zero** SQL functions (affects Phase 2, and closes §13 Q2)

§11 says dropping `kb_profiles.is_active` breaks four functions: `can_modify_resource`, `context_authorable_by_profile`, `graph_home_contexts`, `resources_visible_to`. All four reference a **different table's** column. Measured against local dev:

```
can_modify_resource            ::  r.is_active   → kb_resources  (canonical_functions, "FROM kb_resources r")
context_authorable_by_profile  ::  t.is_active   → kb_teams      ("JOIN kb_teams t ON t.id = c.owner_id AND t.is_active")
graph_home_contexts            ::  rr.is_active  → kb_resources  ("JOIN kb_resources rr ON rr.id = h.resource_id AND rr.is_active")
resources_visible_to           ::  r.is_active   → kb_resources
```

Derived two independent ways that agree: (a) `pg_proc` introspection restricted to functions whose body mentions **both** `kb_profiles` and `is_active` returns exactly those four, and inspecting each shows the alias binds elsewhere; (b) an independent grep sweep over `migrations/` found **no** SQL function or view anywhere reading `kb_profiles.is_active`. There is also no index or constraint on it (`pg_indexes` for `kb_profiles` returns only `_pkey` and `_handle_key`).

Enforcement of profile deactivation is **entirely Rust-side**, at two sites:

```rust
// crates/temper-services/src/auth/mod.rs:246
if !profile.is_active {
    return Err(AuthzError::Deactivated { profile_id: profile.id });
}
```
```rust
// crates/temper-services/src/services/slack_grant_vault_service.rs:214
if row.revoked_at.is_some() || !row.is_active {
```

**Consequence:** §13's open question 2 ("which non-auth `is_active` readers move versus consume a projection") has an empirical answer — there are only two readers and both are Rust. Phase 2's `is_active` drop is far smaller than the spec budgets for. Do not write migration rewrites for four functions that do not need them.

### C3 — §2's chokepoint claim is refuted, but its conclusion survives via a better object (affects Task 7, Task 17)

§2 claims `is_system_admin` has "~12 Rust callers, ALL routed through `access_service::is_system_admin` (:44)". Both halves are wrong:

- **21 production call sites**, not ~12 (across `db_backend.rs:2030`, `connection_service.rs:75`, `admin_ledger_service.rs:80,181`, `cogmap_service.rs:68`, `machine_registration_service.rs:379`, `slack_disconnect_service.rs:267`, `machine_authz.rs:51`, `context_service.rs:376`, `team_service.rs:150,283`, `machine_client_service.rs:93`, `access_service.rs:104,432,915`, `handlers/access.rs:145,163,189,205,223`, `handlers/embed.rs:195`).
- **Not all routed through Rust.** `migrations/20260715000010_context_reassign_fns.sql:76` calls it *in-database*: `IF NOT is_system_admin(v_actor) THEN`.

**But the conclusion holds for a better reason.** The Rust wrapper is a pure passthrough:

```rust
// crates/temper-services/src/services/access_service.rs:44-50
pub async fn is_system_admin(pool: &PgPool, profile_id: ProfileId) -> ApiResult<bool> {
    let result = sqlx::query_scalar!("SELECT is_system_admin($1)", *profile_id,)
        .fetch_one(pool)
        .await?;
    Ok(result.unwrap_or(false))
}
```

So the real chokepoint is the **SQL function body**, which is strictly better than the Rust wrapper: it covers the in-database caller too. D10's economics are intact — one body to repoint, all 21 Rust sites and the one SQL site follow. **Repoint the SQL, not the Rust.**

Note also the `.unwrap_or(false)`: the Rust wrapper already fail-closes on `NULL`. The fail-open hazard is confined to the plpgsql `IF NOT` sites, exactly as §7 targets.

### C4 — §10's "row + log + event" three-part pattern does not exist in this repo (affects Task 6)

Every atomic function in `migrations/` is **two**-part: mutate the projection row, then `PERFORM _event_append(...)`. There is no precedent anywhere for a separate transition-log table alongside a `kb_events` emission; the only log-shaped table is `kb_resource_audits`, which no `_event_append` function writes.

**Consequence:** D4 still requires the dedicated log (`kb_principal_standing_events`), and Task 6 builds it — but tag it **EXTEND (new construction, spec D4 / §10)**, not "follow the existing pattern." The template to copy for the *event* half is real and cited in Task 6; the log half has no template.

### C5 — `temper-principal` cannot depend on `temper-core` (affects Task 1)

D3 requires no `sqlx` dependency, "purity enforced by the compiler, not by convention." But `temper-core`'s sqlx dep is **non-optional**:

```toml
# crates/temper-core/Cargo.toml:22-30
sqlx = { version = "0.8", features = [
  "chrono", "json", "macros", "postgres", "runtime-tokio-rustls", "uuid"
] }
```

and `define_id!` emits sqlx `Type`/`Encode`/`Decode`/`PgHasArrayType` impls **ungated** (`crates/temper-core/src/types/ids.rs:6-108`), so `ProfileId` (`:133`) is sqlx-coupled by construction.

**Consequence:** `temper-principal` depends on **nothing from this workspace**. It takes no ids at all — the machine judges assembled evidence, and ids are the seam's business (spec §4: *"`temper-principal` never resolves a credential. It judges assembled evidence."*). This satisfies D3 more strongly than D3 asked for. Task 1 pins it with a dependency test.

### C6 — `access_mode`'s fate was unspecified, and it blocks `Request` (affects Task 18)

```rust
// crates/temper-services/src/services/access_service.rs:671-678
match access_mode {
    AccessMode::Open => {
        return Err(ApiError::BadRequest(
            "System is in open mode — no access request needed".to_string(),
        ));
    }
    AccessMode::InviteOnly => {}
}
```

Under D11 every door births `Denied` regardless of mode, so a principal on an `open` instance must be able to `Request`. Leaving this rejection in place makes `open` instances a dead end.

**Decision (Pete, 2026-07-20): retire `access_mode` fully.** Implemented as: **Phase 1 removes every reader and the concept** (this rejection, `AccessMode` in the gate path, the settings-write acceptance); **Phase 2 drops the column** with the other drops, so Phase 1 stays additive-on-schema. If the column must go in this PR instead, move Task 18b into Phase 2's drop list and accept that Phase 1 becomes operator-run.

### C7 — spec claims that DID hold (do not re-verify these)

- **§7's NULL table reproduces exactly.** Measured in `BEGIN/ROLLBACK` on local dev with `kb_system_settings` emptied:
  ```
  empty-settings has_system_access = <NULL> (is null: t)
  empty-settings is_system_admin  = <NULL> (is null: t)
  IF NOT has_system_access(...) => GUARD DID NOT FIRE      ← fail-OPEN
  WHERE-shape rows returned = 0                            ← fail-CLOSED
  ```
- **Exactly two `IF NOT <predicate>` sites**, `auto_join_team_generalization.sql:44` and `context_reassign_fns.sql:76`. A naive `grep 'IF NOT '` returns 14; twelve are `IF NOT FOUND`, a plpgsql row-count diagnostic, not a predicate. Filter with `grep -v 'IF NOT EXISTS' | grep -v 'IF NOT FOUND'`.
- **Exactly three SQL callers of `has_system_access`**, none of them Level 3: `ensure_auto_join_memberships`, `backfill_auto_join_team`, `sync_system_membership`. Confirmed by `pg_get_functiondef` scan.
- **Exactly three non-test-gated `system_access` writers**, at the cited lines: `scenario/bootseed.rs:32`, `scenario/loader.rs:53`, `scenario/access/loader.rs:143`. None of those three files contains any `cfg(test)` marker.
- **Rule 0's discriminator is genuinely FK-only.** `kb_profiles` columns are `id, handle, display_name, system_access, email, preferences, created, is_active` — no kind column.
- **D9 holds.** `create_join_request` resolves `gating_team_slug` and errors if none (`access_service.rs:680-688`); every row that exists targets the gating team.
- **The `"join_request"` sentinel is test-fixture-only.** It appears at `error.rs:299` and `error.rs:377` and nowhere in production; the live domain is `open`/`invite_only`, populated from `get_public_settings` at `middleware/system_access.rs:49`.

### C8 — surface facts that differ from how the spec describes them

- **There is no separate approve function and no separate reject function.** Both are `access_service::review_request` (`access_service.rs:836`), branching on `params.decision`; the approve branch is `:877-895`. Plan accordingly.
- **The revoke path is not in `machine_registration_service.rs`.** It is `machine_client_service::revoke` (`machine_client_service.rs:124`). D17's hook goes there.
- **The admin check is copy-pasted inline in five handlers** (`handlers/access.rs:143,161,188,203,221`), not shared. Five sites to touch, not one.
- **No MCP tool touches join requests or `/admin/access`.** All 28 tools enumerated; only `admin_ledger` is admin-adjacent and it is read-only. §14 already notes this; it stays out of scope here (see *Follow-ups*).
- **`existing self-service CLI lives under `temper auth`**, not `temper admin`: `temper auth request-access` / `temper auth withdraw-request` (`cli.rs:890-897`).

---

## File Structure

**Created**

| Path | Responsibility |
|---|---|
| `crates/temper-principal/Cargo.toml` | The pure crate's manifest. Dependencies: `serde` only. No workspace deps, no `sqlx`. |
| `crates/temper-principal/src/lib.rs` | Re-exports; the crate's module map and the purity doc. |
| `crates/temper-principal/src/standing.rs` | `Standing` enum + total `parse` (`&str -> Option<Standing>`) + `as_str`. |
| `crates/temper-principal/src/act.rs` | `Act` enum, `ActorAuthority` enum, `Provisioner` enum. |
| `crates/temper-principal/src/transition.rs` | `transition(...) -> Result<Standing, Refusal>` — §6's table, exhaustive. |
| `crates/temper-principal/src/admission.rs` | `admit(...) -> Result<AdmittedPrincipal, Refusal>` — the per-request pure decision. |
| `crates/temper-principal/src/refusal.rs` | `Refusal` typed enum (replaces the stringly `access_mode` 403 payload). |
| `crates/temper-principal/tests/matrix.rs` | The exhaustive state × act × authority table test (spec §12). |
| `migrations/20260720000010_principal_standing.sql` | The two standing tables, the governance table, indexes. |
| `migrations/20260720000020_principal_standing_events.sql` | Event-type registrations (`category = 'admin'`). |
| `migrations/20260720000030_principal_standing_fns.sql` | One plpgsql function per transition; row + log + event, one txn. |
| `migrations/20260720000040_repoint_predicates.sql` | `has_system_access` / `is_system_admin` bodies repointed, `EXISTS`-total. |
| `migrations/20260720000050_backfill_standing.sql` | The 4-rule backfill + pending-request pass + genesis-log pass. |
| `crates/temper-services/src/services/standing_service.rs` | The seam: gather evidence → call `temper-principal` → dispatch to one SQL fn. |
| `crates/temper-services/tests/standing_backfill_test.rs` | The differential backfill test (spec §12). |
| `crates/temper-services/tests/standing_totality_test.rs` | SQL totality: non-`NULL` for absent row, deactivated, unknown state. |

**Modified**

| Path | Change |
|---|---|
| `crates/temper-substrate/src/payloads.rs` | Payload structs + `TYPED_EVENT_NAMES` + `ADMIN_EVENT_NAMES` arity bumps + `verify_ledger_roundtrip` arms. |
| `crates/temper-substrate/tests/payload_schema.rs` | One `check::<>` line per new payload. |
| `crates/temper-substrate/src/scenario/bootseed.rs` | Genesis door mints standing `Approved` + admin (D11's deliberate exception). |
| `crates/temper-substrate/src/scenario/loader.rs` | Mint a standing row alongside the tier. |
| `crates/temper-substrate/src/scenario/access/loader.rs` | Same. |
| `crates/temper-services/src/auth/mod.rs` | `require_system_access` routes through the pure machine; `AdmittedPrincipal` added. |
| `crates/temper-services/src/services/access_service.rs` | `review_request`, `create_join_request`, `withdraw_request`, `promote_admin` route through the machines; `access_mode` readers removed. |
| `crates/temper-services/src/services/machine_client_service.rs` | `revoke` fires `Revoke` on standing in the same txn (D17). |
| `crates/temper-services/src/error.rs` | `SystemAccessRequired` carries the typed `Refusal`. |
| `crates/temper-api/src/handlers/access.rs` | Five admin handlers; new act endpoints. |
| `crates/temper-cli/src/cli.rs` + `commands/admin.rs` | CLI verbs for the new acts. |

---

## Beat A — the pure machine

No database, no async, no workspace dependencies. This beat is fully testable with `cargo nextest run -p temper-principal` and needs nothing running.

---

### Task 1: The `temper-principal` crate skeleton and `Standing`

**Files:**
- Create: `crates/temper-principal/Cargo.toml`
- Create: `crates/temper-principal/src/lib.rs`
- Create: `crates/temper-principal/src/standing.rs`
- Test: `crates/temper-principal/src/standing.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: `temper_principal::Standing` (enum: `Denied`, `Requested`, `Approved`, `Revoked`, `Deactivated`), `Standing::parse(&str) -> Option<Standing>`, `Standing::as_str(&self) -> &'static str`.

**GD-3 tag: EXTEND.** Authorized by spec D3 ("a new crate, `temper-principal`, with no `sqlx` dependency — purity enforced by the compiler, not by convention"). The *shape* — no workspace deps at all — is a CONFORM to the constraint documented in **C5**: `temper-core` pulls `sqlx` non-optionally, so depending on it would silently defeat D3.

**Invariant, carried verbatim from spec §7:**
> "Parsing is `&str -> Option<Standing>`; `None` refuses. Never a panic, never a default. The column can hold a value a given binary does not know during a rolling deploy or after a rollback. **This is the obligation most likely to be got wrong, because it only bites inside a deploy window.**"

No manifest edit is needed to register the crate: the workspace is `members = ["crates/*", "tests/e2e"]` (`Cargo.toml:3`).

- [ ] **Step 1: Write the failing test**

Create `crates/temper-principal/src/standing.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_every_known_state() {
        for s in [
            Standing::Denied,
            Standing::Requested,
            Standing::Approved,
            Standing::Revoked,
            Standing::Deactivated,
        ] {
            assert_eq!(
                Standing::parse(s.as_str()),
                Some(s),
                "as_str/parse must round-trip; the column literal and the enum are one contract"
            );
        }
    }

    #[test]
    fn an_unrecognized_state_is_none_never_a_default() {
        // Spec §7 obligation 2: the column can hold a value this binary does not know
        // during a rolling deploy or after a rollback. `None` refuses; it never defaults.
        for unknown in ["", "APPROVED", "approved ", "admin", "pending", "🙂"] {
            assert_eq!(
                Standing::parse(unknown),
                None,
                "{unknown:?} must not parse — a default here is a silent grant inside a deploy window"
            );
        }
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-principal`
Expected: FAIL — the package does not exist yet (`error: package ID specification 'temper-principal' did not match any packages`).

- [ ] **Step 3: Write the minimal implementation**

Create `crates/temper-principal/Cargo.toml`:

```toml
[package]
name = "temper-principal"
version = "0.1.0"
edition = "2021"
description = "The pure principal-admission and standing-transition machines. No I/O, no database."

# DELIBERATELY EMPTY OF WORKSPACE DEPENDENCIES.
#
# Spec D3 requires no `sqlx` here, "purity enforced by the compiler, not by convention". That
# rules out `temper-core`: its sqlx dependency is NON-OPTIONAL (temper-core/Cargo.toml:22) and
# `define_id!` emits sqlx Type/Encode/Decode impls ungated (temper-core/src/types/ids.rs:6-108),
# so taking `ProfileId` would pull sqlx straight back in and D3 would hold only by wishful
# thinking. This crate therefore takes NO ids at all — it judges assembled evidence and the
# seam in temper-services owns every identifier (spec §4).
[dependencies]
serde = { version = "1", features = ["derive"] }
```

Create `crates/temper-principal/src/lib.rs`:

```rust
//! The principal-admission machines (spec 2026-07-20 §4).
//!
//! Two machines, deliberately separated (D1): a **persisted** `Standing` lifecycle whose
//! transitions this crate validates, and a **pure, per-request** `Admission` decision that reads
//! standing as evidence.
//!
//! # Why this is its own crate
//!
//! Every `match` over [`Standing`] here is exhaustive with no `_ =>` arm, so adding a state
//! becomes a compile error at every decision site (spec §7 obligation 3). That property is what
//! the crate boundary buys; it cannot be bought by discipline inside a larger crate.
//!
//! This crate performs no I/O, holds no identifiers, and never resolves a credential. It judges
//! assembled evidence — which is what makes it safe to share across surfaces (spec §4).

mod act;
mod admission;
mod refusal;
mod standing;
mod transition;

pub use act::{Act, ActorAuthority, Provisioner};
pub use admission::{admit, AdmittedPrincipal};
pub use refusal::Refusal;
pub use standing::Standing;
pub use transition::transition;
```

> **Note for the implementer:** `lib.rs` above names all five modules. Create empty placeholder files for `act.rs`, `admission.rs`, `refusal.rs`, and `transition.rs` now (a single `//! placeholder — Task 2/3` line each) so the crate compiles, and comment out the `pub use` lines for types that do not exist yet. Tasks 2 and 3 fill them and restore the re-exports.

Now write the implementation half of `crates/temper-principal/src/standing.rs`, **above** the test module you created in Step 1:

```rust
use serde::{Deserialize, Serialize};

/// The one authoritative standing state for a principal (spec D2).
///
/// Five states plus absence. **Absence is not a variant** — a principal with no standing row is
/// denied structurally (spec §7 obligation 1), which is what makes D7's connection-profile safety
/// hold by construction rather than by a check someone can forget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Provisioned, never granted. Where every door lands (D11).
    Denied,
    /// Has asked for system access. Still denied, but the refusal can say so. Human-only (D14).
    Requested,
    /// May use the instance.
    Approved,
    /// Was granted and lost it. Only an admin act leaves this state (D15).
    Revoked,
    /// The principal itself is disabled. Prior standing is recoverable from the log.
    Deactivated,
}

impl Standing {
    /// The database literal for this state. Paired with [`Standing::parse`]; the two are one
    /// contract and the round-trip is tested.
    pub fn as_str(&self) -> &'static str {
        match self {
            Standing::Denied => "denied",
            Standing::Requested => "requested",
            Standing::Approved => "approved",
            Standing::Revoked => "revoked",
            Standing::Deactivated => "deactivated",
        }
    }

    /// Total parse. **`None` refuses; it never defaults** (spec §7 obligation 2).
    ///
    /// The column can hold a value this binary does not know during a rolling deploy or after a
    /// rollback. Returning a default here would admit an unknown state, and it would only bite
    /// inside a deploy window — which is why this is the obligation most likely to be got wrong.
    pub fn parse(raw: &str) -> Option<Standing> {
        match raw {
            "denied" => Some(Standing::Denied),
            "requested" => Some(Standing::Requested),
            "approved" => Some(Standing::Approved),
            "revoked" => Some(Standing::Revoked),
            "deactivated" => Some(Standing::Deactivated),
            _ => None,
        }
    }
}
```

> The `_ => None` here is the **one** permitted catchall in the crate, and it is permitted because it is on the *input* side (`&str`, an unbounded set) and its result is a refusal. The prohibition in spec §7 obligation 3 is on catchalls in matches over `Standing` itself, where a new variant must force a compile error.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p temper-principal`
Expected: PASS — 2 tests.

- [ ] **Step 5: Prove the purity constraint holds (D3)**

Run: `cargo tree -p temper-principal | grep -c sqlx`
Expected: `0`.

Run: `cargo tree -p temper-principal`
Expected: `temper-principal v0.1.0` with `serde` and its proc-macro as the only entries. If `sqlx`, `temper-core`, `uuid`, or `tokio` appear, a dependency crept in and D3 is broken — stop and remove it rather than proceeding.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-principal/
git commit -m "feat(principal): the pure crate skeleton and a total Standing parse

Spec D3 — no sqlx, enforced by the compiler. Deliberately depends on nothing
in this workspace: temper-core's sqlx dep is non-optional and define_id! emits
sqlx impls ungated, so taking ProfileId would defeat D3 silently."
```

---

### Task 2: `Act`, `ActorAuthority`, and the transition table

**Files:**
- Create: `crates/temper-principal/src/act.rs`
- Create: `crates/temper-principal/src/refusal.rs`
- Create: `crates/temper-principal/src/transition.rs`
- Create: `crates/temper-principal/tests/matrix.rs`
- Modify: `crates/temper-principal/src/lib.rs` (restore the `pub use` lines)

**Interfaces:**
- Consumes: `Standing` (Task 1).
- Produces:
  - `Act` — `Provision { path: Provisioner }`, `Request`, `Withdraw`, `Approve`, `Reject`, `Revoke { reason: String }`, `Deactivate`, `Reactivate { prior: Option<Standing> }`, `RequestReview`.
  - `ActorAuthority` — `Credential`, `SelfPrincipal`, `Admin`.
  - `Provisioner` — `Saml`, `OauthFirstLogin`, `MachineRegistration`, `BootSeed`.
  - `Refusal` — see Task 2 Step 3.
  - `transition(current: Option<Standing>, act: &Act, authority: ActorAuthority) -> Result<Standing, Refusal>`.

**GD-3 tag: EXTEND.** Authorized by spec §6, which supplies the complete transition table (D14–D16 filled it; F3 is RESOLVED).

**Invariants, carried verbatim from spec §6:**
> "Every cell not listed here is **illegal and refused with a reason** — there is no catchall."

> "**`Reactivate` is the only data-dependent target in the machine.** That is a deliberate property and worth protecting — D15 exists partly to keep it at one, and a future act whose target depends on history should be treated as a design smell until argued for."

> "**Every self act is illegal from `Deactivated`, and is specified so explicitly** — even though `gate_resolved_profile` (`auth/mod.rs:242-246`) already makes it unreachable by refusing an `AuthenticatedProfile` to a deactivated principal. Leaning on another layer to make a cell unreachable is the cross-layer reasoning that produced this design's original bugs; the table stays total on its own terms."

The authoritative table, transcribed from spec §6:

| Act | Actor | Legal from | → Resulting standing |
|---|---|---|---|
| `Provision { path }` | admin / credential | *absence only* | `Denied` (boot-seed: `Approved`) |
| `Request` | self | `Denied` | `Requested` |
| `Withdraw` | self | `Requested` | `Denied` |
| `Approve` | admin | `Requested`, `Denied` (D14), `Revoked` (D16) | `Approved` |
| `Reject` | admin | `Requested` | `Denied` |
| `Revoke { reason }` | admin | `Approved` | `Revoked` |
| `Deactivate` | admin | any live state | `Deactivated` |
| `Reactivate` | admin | `Deactivated` | **prior state, from the log** |
| `RequestReview` | self | `Revoked` | **unchanged — `Revoked`** (D15) |

- [ ] **Step 1: Write the failing test**

Create `crates/temper-principal/tests/matrix.rs`:

```rust
//! The exhaustive state × act × authority matrix (spec §12).
//!
//! "The state × act matrix is exhaustively enumerable — five states × eight acts ×
//! actor-authority variants, as a table test with no database. Adding a state fails compilation
//! until every cell is filled."
//!
//! Every illegal cell asserts a *reason*, not merely a refusal: "The point of refusing at the act
//! is that the actor learns why; a test that only checks 'not admitted' would pass on a silent
//! denial." (spec §12)

use temper_principal::{Act, ActorAuthority, Refusal, Standing};

/// Every state, including absence. Adding a `Standing` variant without extending this array is
/// caught by `every_standing_variant_is_in_the_matrix` below.
const STATES: [Option<Standing>; 6] = [
    None,
    Some(Standing::Denied),
    Some(Standing::Requested),
    Some(Standing::Approved),
    Some(Standing::Revoked),
    Some(Standing::Deactivated),
];

fn all_acts() -> Vec<Act> {
    vec![
        Act::Provision { path: temper_principal::Provisioner::OauthFirstLogin },
        Act::Request,
        Act::Withdraw,
        Act::Approve,
        Act::Reject,
        Act::Revoke { reason: "test".to_string() },
        Act::Deactivate,
        Act::Reactivate { prior: Some(Standing::Approved) },
        Act::RequestReview,
    ]
}

const AUTHORITIES: [ActorAuthority; 3] = [
    ActorAuthority::Credential,
    ActorAuthority::SelfPrincipal,
    ActorAuthority::Admin,
];

#[test]
fn every_cell_is_decided_and_every_refusal_carries_a_reason() {
    for state in STATES {
        for act in all_acts() {
            for authority in AUTHORITIES {
                let outcome = temper_principal::transition(state, &act, authority);
                if let Err(refusal) = outcome {
                    assert!(
                        !refusal.reason().is_empty(),
                        "cell ({state:?}, {act:?}, {authority:?}) refused with an empty reason — \
                         a silent denial is the failure this test exists to catch"
                    );
                }
            }
        }
    }
}

#[test]
fn the_legal_cells_are_exactly_the_spec_six_table() {
    use ActorAuthority::{Admin, SelfPrincipal};
    use Standing::*;

    let legal: Vec<(Option<Standing>, Act, ActorAuthority, Standing)> = vec![
        (Some(Denied), Act::Request, SelfPrincipal, Requested),
        (Some(Requested), Act::Withdraw, SelfPrincipal, Denied),
        (Some(Requested), Act::Approve, Admin, Approved),
        (Some(Denied), Act::Approve, Admin, Approved), // D14 — machines never Request
        (Some(Revoked), Act::Approve, Admin, Approved), // D16 — no separate Reinstate
        (Some(Requested), Act::Reject, Admin, Denied),
        (Some(Approved), Act::Revoke { reason: "r".into() }, Admin, Revoked),
        (Some(Denied), Act::Deactivate, Admin, Deactivated),
        (Some(Requested), Act::Deactivate, Admin, Deactivated),
        (Some(Approved), Act::Deactivate, Admin, Deactivated),
        (Some(Revoked), Act::Deactivate, Admin, Deactivated),
        (Some(Revoked), Act::RequestReview, SelfPrincipal, Revoked), // D15 — moves nothing
    ];

    for (from, act, authority, expected) in legal {
        assert_eq!(
            temper_principal::transition(from, &act, authority),
            Ok(expected),
            "spec §6 says ({from:?}, {act:?}, {authority:?}) → {expected:?}"
        );
    }
}

#[test]
fn revoke_is_illegal_from_denied_and_requested() {
    // Spec §6: "you cannot revoke what was never granted." §5's diagram shows an arrow into
    // Revoked originating at Denied; no act produces that edge. §6 is authoritative.
    for from in [Standing::Denied, Standing::Requested] {
        let out = temper_principal::transition(
            Some(from),
            &Act::Revoke { reason: "r".into() },
            ActorAuthority::Admin,
        );
        assert!(
            matches!(out, Err(Refusal::IllegalTransition { .. })),
            "Revoke from {from:?} must be refused — nothing was ever granted"
        );
    }
}

#[test]
fn a_revoked_principal_cannot_re_request() {
    // D15: "there is no path out of Revoked except an admin act, so there is nothing to launder."
    let out = temper_principal::transition(
        Some(Standing::Revoked),
        &Act::Request,
        ActorAuthority::SelfPrincipal,
    );
    assert!(
        matches!(out, Err(Refusal::IllegalTransition { .. })),
        "Revoked → Request must be refused; RequestReview is the only self act from Revoked"
    );
}

#[test]
fn request_review_leaves_standing_unchanged() {
    // D15 obligation: the marker moves nothing.
    assert_eq!(
        temper_principal::transition(
            Some(Standing::Revoked),
            &Act::RequestReview,
            ActorAuthority::SelfPrincipal
        ),
        Ok(Standing::Revoked)
    );
}

#[test]
fn every_self_act_is_illegal_from_deactivated() {
    // Spec §6 requires this be specified on the table's own terms, NOT left to
    // gate_resolved_profile (auth/mod.rs:246) making it unreachable. Leaning on another layer is
    // the cross-layer reasoning that produced this design's original bugs.
    for act in [Act::Request, Act::Withdraw, Act::RequestReview] {
        let out = temper_principal::transition(
            Some(Standing::Deactivated),
            &act,
            ActorAuthority::SelfPrincipal,
        );
        assert!(
            matches!(out, Err(Refusal::IllegalTransition { .. })),
            "{act:?} from Deactivated must be refused by this table, independently of Level 1"
        );
    }
}

#[test]
fn provision_is_legal_only_from_absence_and_never_grants() {
    use temper_principal::Provisioner;
    // D11: "Every provision path births Denied. No door grants access."
    for path in [
        Provisioner::Saml,
        Provisioner::OauthFirstLogin,
        Provisioner::MachineRegistration,
    ] {
        assert_eq!(
            temper_principal::transition(
                None,
                &Act::Provision { path },
                ActorAuthority::Credential
            ),
            Ok(Standing::Denied),
            "{path:?} must birth Denied — no door grants access (D11)"
        );
    }

    // The genesis exception, deliberate and load-bearing (D11, F6).
    assert_eq!(
        temper_principal::transition(
            None,
            &Act::Provision { path: Provisioner::BootSeed },
            ActorAuthority::Credential
        ),
        Ok(Standing::Approved),
        "the boot-seed mints the first admin — the one deliberate exception"
    );

    // "Provision fires only on profile mint, never on a returning principal." A revoked SAML
    // principal re-asserting must not be re-provisioned back to Denied (F4).
    let out = temper_principal::transition(
        Some(Standing::Revoked),
        &Act::Provision { path: Provisioner::Saml },
        ActorAuthority::Credential,
    );
    assert!(
        matches!(out, Err(Refusal::IllegalTransition { .. })),
        "Provision from an existing standing must be refused — this is what closes F4 structurally"
    );
}

#[test]
fn reactivate_restores_the_prior_state_and_refuses_without_one() {
    for prior in [Standing::Denied, Standing::Requested, Standing::Approved, Standing::Revoked] {
        assert_eq!(
            temper_principal::transition(
                Some(Standing::Deactivated),
                &Act::Reactivate { prior: Some(prior) },
                ActorAuthority::Admin
            ),
            Ok(prior),
            "Reactivate restores rather than guesses (spec §5)"
        );
    }

    // Backfilled rows are the exception §5 names: the log begins at migration time. The backfill
    // writes a genesis entry (§11) precisely so this arm is unreachable in practice — but the
    // machine must still refuse rather than guess.
    assert!(
        matches!(
            temper_principal::transition(
                Some(Standing::Deactivated),
                &Act::Reactivate { prior: None },
                ActorAuthority::Admin
            ),
            Err(Refusal::NoPriorStanding)
        ),
        "Reactivate with no prior state must refuse, never default to Approved"
    );
}

#[test]
fn authority_is_enforced_not_advisory() {
    // A self principal cannot approve itself; a credential cannot approve anything.
    for authority in [ActorAuthority::SelfPrincipal, ActorAuthority::Credential] {
        let out = temper_principal::transition(
            Some(Standing::Requested),
            &Act::Approve,
            authority,
        );
        assert!(
            matches!(out, Err(Refusal::InsufficientAuthority { .. })),
            "Approve by {authority:?} must be refused — approval is always an admin act (D11)"
        );
    }
}

#[test]
fn every_standing_variant_is_in_the_matrix() {
    // Guard: adding a Standing variant without adding it to STATES would silently shrink the
    // matrix, and every other test here would still pass.
    let covered: Vec<Standing> = STATES.iter().flatten().copied().collect();
    for s in [
        Standing::Denied,
        Standing::Requested,
        Standing::Approved,
        Standing::Revoked,
        Standing::Deactivated,
    ] {
        assert!(covered.contains(&s), "{s:?} is missing from STATES");
    }
    assert_eq!(covered.len(), 5, "STATES gained or lost a variant");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-principal --test matrix`
Expected: FAIL to compile — `Act`, `ActorAuthority`, `Refusal`, `Provisioner`, and `transition` are not defined.

- [ ] **Step 3: Write the minimal implementation**

Create `crates/temper-principal/src/act.rs`:

```rust
use crate::standing::Standing;
use serde::{Deserialize, Serialize};

/// Which door minted the principal (spec §6's provision table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provisioner {
    Saml,
    OauthFirstLogin,
    MachineRegistration,
    /// Genesis. The one deliberate exception that births `Approved` (D11, F6): on a fresh
    /// instance no admin exists, so nobody could ever be approved. Bootstrapping temper already
    /// requires database write access, and the bootstrap SoP foregrounds that.
    BootSeed,
}

/// Who is acting. Three authorities (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorAuthority {
    /// The credential itself is the authority — provision paths only.
    Credential,
    /// The principal acting on its own standing.
    SelfPrincipal,
    /// An actor for whom `is_system_admin` holds.
    Admin,
}

/// The eight acts (D16 dropped `Reinstate`), plus `RequestReview` which moves nothing (D15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "act")]
pub enum Act {
    /// Fires **only on profile mint**, never on a returning principal. An existing auth link
    /// returns at step 1 of `resolve_human_from_claims` and never reaches the mint; a returning
    /// principal's standing is *loaded, not set*. This is what closes F4 structurally.
    Provision { path: Provisioner },
    /// The consent-capturing act (D12) — human-only. Machines have no self and can never Request.
    Request,
    Withdraw,
    Approve,
    Reject,
    Revoke { reason: String },
    Deactivate,
    /// The only data-dependent target in the machine (spec §6). `prior` is read from the standing
    /// log by the caller; `None` refuses rather than guesses.
    Reactivate { prior: Option<Standing> },
    /// Sets a marker and moves nothing (D15). **Never an admission input** — see `admission.rs`.
    RequestReview,
}
```

Create `crates/temper-principal/src/refusal.rs`:

```rust
use crate::{act::ActorAuthority, standing::Standing};
use serde::{Deserialize, Serialize};

/// Why a principal was refused, typed.
///
/// This replaces the stringly-typed enriched 403, which carried `access_mode: String` and whose
/// tests asserted a sentinel `"join_request"` that was never a real mode
/// (`temper-services/src/error.rs:299,377` — verified: the live domain is `open`/`invite_only`).
///
/// Spec §12: "Every illegal cell asserts a *reason*, not just a refusal. The point of refusing at
/// the act is that the actor learns why; a test that only checks 'not admitted' would pass on a
/// silent denial."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "refusal")]
pub enum Refusal {
    /// No standing row. Absence denies (spec §7 obligation 1) — this is what makes D7 structural.
    NoStanding,
    /// The column held a value this binary does not know (spec §7 obligation 2).
    UnrecognizedStanding { raw: String },
    /// Provisioned but never granted. The refusal says *"you may request access."*
    Denied,
    /// Asked, not yet decided. The refusal says *"your request is pending."*
    Requested,
    /// Was granted and lost it. A different sentence, and a different audit signal, than `Denied`.
    Revoked,
    /// The principal itself is disabled.
    Deactivated,
    /// The act is not legal from this state (spec §6 — every unlisted cell).
    IllegalTransition {
        from: Option<Standing>,
        act: &'static str,
    },
    /// The actor lacks the authority this act requires.
    InsufficientAuthority {
        required: ActorAuthority,
        actual: ActorAuthority,
    },
    /// `Reactivate` with no recoverable prior state. The backfill's genesis pass (spec §11) exists
    /// so this is unreachable for pre-existing rows; the machine still refuses rather than guesses.
    NoPriorStanding,
}

impl Refusal {
    /// A non-empty human-facing reason for every variant. The matrix test asserts non-emptiness
    /// across the whole cell space, so a new variant cannot ship silent.
    pub fn reason(&self) -> String {
        match self {
            Refusal::NoStanding => "no standing on this instance".to_string(),
            Refusal::UnrecognizedStanding { raw } => {
                format!("standing {raw:?} is not recognized by this build")
            }
            Refusal::Denied => "access has not been granted; you may request access".to_string(),
            Refusal::Requested => "your access request is pending review".to_string(),
            Refusal::Revoked => "access was revoked; you may request a review".to_string(),
            Refusal::Deactivated => "this principal is deactivated".to_string(),
            Refusal::IllegalTransition { from, act } => match from {
                Some(s) => format!("{act} is not legal from {}", s.as_str()),
                None => format!("{act} is not legal for a principal with no standing"),
            },
            Refusal::InsufficientAuthority { required, actual } => {
                format!("this act requires {required:?} authority; caller has {actual:?}")
            }
            Refusal::NoPriorStanding => {
                "no prior standing is recorded, so reactivation has nothing to restore".to_string()
            }
        }
    }
}
```

Create `crates/temper-principal/src/transition.rs`:

```rust
use crate::{
    act::{Act, ActorAuthority, Provisioner},
    refusal::Refusal,
    standing::Standing,
};

/// Validate one transition against spec §6's table.
///
/// **§6's table is authoritative over §5's diagram.** The earlier diagram showed an edge from
/// `Denied` into `Revoked` that no act produces; in this repo a disagreement between prose and
/// sketch resolves in the sketch's favour, so the table is stated as code here and the diagram is
/// an aid.
///
/// Every cell not listed is illegal and refused **with a reason** — there is no catchall over
/// `Standing`, so adding a state is a compile error here (spec §7 obligation 3).
pub fn transition(
    current: Option<Standing>,
    act: &Act,
    authority: ActorAuthority,
) -> Result<Standing, Refusal> {
    // Authority first. Auth before writes (and before deciding a target state).
    require_authority(act, authority)?;

    match act {
        // Absence only. A returning principal's standing is LOADED, never SET (F4).
        Act::Provision { path } => match current {
            None => Ok(match path {
                Provisioner::BootSeed => Standing::Approved,
                Provisioner::Saml
                | Provisioner::OauthFirstLogin
                | Provisioner::MachineRegistration => Standing::Denied,
            }),
            Some(_) => Err(illegal(current, "provision")),
        },

        Act::Request => match current {
            Some(Standing::Denied) => Ok(Standing::Requested),
            _ => Err(illegal(current, "request")),
        },

        Act::Withdraw => match current {
            Some(Standing::Requested) => Ok(Standing::Denied),
            _ => Err(illegal(current, "withdraw")),
        },

        // D14 — legal from Denied too: machines have no self and can never Request, so without
        // this the entire machine surface is a dead end.
        // D16 — legal from Revoked: `Reinstate` was identical to this, and the log's prior_state
        // already makes a reinstatement legible.
        Act::Approve => match current {
            Some(Standing::Requested) | Some(Standing::Denied) | Some(Standing::Revoked) => {
                Ok(Standing::Approved)
            }
            _ => Err(illegal(current, "approve")),
        },

        Act::Reject => match current {
            Some(Standing::Requested) => Ok(Standing::Denied),
            _ => Err(illegal(current, "reject")),
        },

        // You cannot revoke what was never granted.
        Act::Revoke { .. } => match current {
            Some(Standing::Approved) => Ok(Standing::Revoked),
            _ => Err(illegal(current, "revoke")),
        },

        // Any LIVE state. Not from absence, and not from Deactivated (already there).
        Act::Deactivate => match current {
            Some(Standing::Denied)
            | Some(Standing::Requested)
            | Some(Standing::Approved)
            | Some(Standing::Revoked) => Ok(Standing::Deactivated),
            _ => Err(illegal(current, "deactivate")),
        },

        // The only data-dependent target in the machine (spec §6). Refuses rather than guesses.
        Act::Reactivate { prior } => match current {
            Some(Standing::Deactivated) => match prior {
                Some(p) => Ok(*p),
                None => Err(Refusal::NoPriorStanding),
            },
            _ => Err(illegal(current, "reactivate")),
        },

        // D15 — sets a marker and moves nothing, so a revocation cannot be laundered back to
        // Denied. The no-laundering property is structural rather than bookkept.
        Act::RequestReview => match current {
            Some(Standing::Revoked) => Ok(Standing::Revoked),
            _ => Err(illegal(current, "request_review")),
        },
    }
}

fn illegal(from: Option<Standing>, act: &'static str) -> Refusal {
    Refusal::IllegalTransition { from, act }
}

/// Spec §6's actor column, enforced.
fn require_authority(act: &Act, actual: ActorAuthority) -> Result<(), Refusal> {
    let required = match act {
        // Provision's actor is NOT "none" universally: `temper admin machine provision` is
        // admin-run. Under D11 this grants nothing either way, but the table must not imply an
        // unauthenticated mint path exists — so both Credential and Admin are accepted here.
        Act::Provision { .. } => {
            return match actual {
                ActorAuthority::Credential | ActorAuthority::Admin => Ok(()),
                ActorAuthority::SelfPrincipal => Err(Refusal::InsufficientAuthority {
                    required: ActorAuthority::Credential,
                    actual,
                }),
            }
        }
        Act::Request | Act::Withdraw | Act::RequestReview => ActorAuthority::SelfPrincipal,
        Act::Approve
        | Act::Reject
        | Act::Revoke { .. }
        | Act::Deactivate
        | Act::Reactivate { .. } => ActorAuthority::Admin,
    };

    if actual == required {
        Ok(())
    } else {
        Err(Refusal::InsufficientAuthority { required, actual })
    }
}
```

Restore the `pub use` lines in `crates/temper-principal/src/lib.rs` for `Act`, `ActorAuthority`, `Provisioner`, `Refusal`, and `transition` (leave `admission` commented until Task 3).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p temper-principal`
Expected: PASS — 2 tests from Task 1 plus 10 from `matrix.rs`.

- [ ] **Step 5: Prove the no-catchall property is real**

Temporarily add a sixth variant to `Standing` (e.g. `Suspended`) and run `cargo check -p temper-principal`.
Expected: compile errors at **every** `match` over `Standing` in `transition.rs` and `standing.rs` — not a silent default. Then **revert the variant**; this step is a proof, not a change.

If it compiles clean, a catchall crept in somewhere and spec §7 obligation 3 is broken. Find it before proceeding.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-principal/
git commit -m "feat(principal): the eight acts and spec §6's transition table

Every cell decided, every refusal carries a reason, no catchall over Standing.
D14 (Approve from Denied — machines never Request), D15 (RequestReview moves
nothing), D16 (no separate Reinstate)."
```

---

### Task 3: `admit` and the construction-restricted `AdmittedPrincipal`

**Files:**
- Create: `crates/temper-principal/src/admission.rs`
- Modify: `crates/temper-principal/src/lib.rs` (restore the `admission` re-exports)
- Test: `crates/temper-principal/src/admission.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `Standing`, `Refusal` (Tasks 1–2).
- Produces: `AdmittedPrincipal` (opaque, no public constructor), `admit(raw_standing: Option<&str>) -> Result<AdmittedPrincipal, Refusal>`.

**GD-3 tag: EXTEND + AMEND.** EXTEND for the new type, authorized by spec §7 (*"`AdmittedPrincipal` is constructible only by the machine"*). AMEND on the premise: §7 says this *preserves* a guarantee `SystemAuthorized` has today, and **C5 above shows it does not** — `SystemAuthorized(pub AuthenticatedProfile)` with a public field and no `impl` block. We are **adding** the guarantee, on the new type only (Pete's decision).

**Invariants, carried verbatim from spec §7 and §15:**
> "**Absence denies.** No standing row is not an error and not a default-grant. This is what makes D7 structural."

> "**An unrecognized state denies.** Parsing is `&str -> Option<Standing>`; `None` refuses. Never a panic, never a default."

> D15 obligation 1: "**The marker is never an admission input.** It is an inbox signal only. Admission reads standing and nothing else; `Revoked` denies whether or not a review is pending. ANDing the marker into the decision would restore precisely the conjunction-across-provisional-facts shape D2 forbids. This is stated as an obligation rather than left implied **because it is the tempting change** — a future reader will see a pending review and reach for it."

Note the signature consequence of that last invariant: **`admit` takes the raw standing and nothing else.** It has no parameter for a pending review, no parameter for `is_active`, no parameter for gating-team membership. That is not an oversight to fix — a reviewer who sees a future PR add a second parameter to `admit` should reject it and re-read D2.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-principal/src/admission.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::refusal::Refusal;

    #[test]
    fn only_approved_is_admitted() {
        assert!(admit(Some("approved")).is_ok());
        for denied in ["denied", "requested", "revoked", "deactivated"] {
            assert!(
                admit(Some(denied)).is_err(),
                "{denied} must not be admitted"
            );
        }
    }

    #[test]
    fn absence_denies() {
        // Spec §7 obligation 1 — this is what makes D7's connection-profile safety structural.
        assert_eq!(admit(None), Err(Refusal::NoStanding));
    }

    #[test]
    fn an_unrecognized_state_denies_and_names_itself() {
        // Spec §7 obligation 2 — the rolling-deploy / rollback window.
        match admit(Some("quarantined")) {
            Err(Refusal::UnrecognizedStanding { raw }) => assert_eq!(raw, "quarantined"),
            other => panic!("expected UnrecognizedStanding, got {other:?}"),
        }
    }

    #[test]
    fn each_refusal_is_distinguishable_so_the_403_can_differ() {
        // §12/D12: Denied refuses with "you may request access", Requested with "your request is
        // pending". That messaging distinction is the real justification for Requested existing as
        // a state, and it only works if the two refusals are different values.
        assert_eq!(admit(Some("denied")), Err(Refusal::Denied));
        assert_eq!(admit(Some("requested")), Err(Refusal::Requested));
        assert_eq!(admit(Some("revoked")), Err(Refusal::Revoked));
        assert_eq!(admit(Some("deactivated")), Err(Refusal::Deactivated));
    }

    #[test]
    fn admit_reads_standing_and_nothing_else() {
        // D15 obligation 1, pinned as a signature test. `admit` takes one argument. If a future
        // change gives it a second — a pending-review flag, is_active, gating membership — that
        // is the conjunction-across-provisional-facts shape D2 forbids, and this test's failure
        // is the intended alarm. Do not "fix" it by updating the call; re-read D2.
        let _: fn(Option<&str>) -> Result<AdmittedPrincipal, Refusal> = admit;
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-principal --lib`
Expected: FAIL to compile — `admit` and `AdmittedPrincipal` are not defined.

- [ ] **Step 3: Write the minimal implementation**

Write, **above** that test module in `crates/temper-principal/src/admission.rs`:

```rust
use crate::{refusal::Refusal, standing::Standing};

/// Proof that a principal is admitted (spec §7).
///
/// **Constructible only by [`admit`].** The single field is private and there is no public
/// constructor, no `Default`, and no `From`, so a value of this type cannot exist without a
/// standing of `Approved` having been read and parsed.
///
/// This is a genuine enforcement, and it is new. The design doc describes it as "preserving the
/// type-state guarantee `SystemAuthorized` has today" — but `SystemAuthorized` is
/// `pub struct SystemAuthorized(pub AuthenticatedProfile)` with a public field and no `impl`
/// block (temper-services/src/auth/mod.rs:263), so any crate can build one by struct literal.
/// That gap is filed separately; it is not inherited here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedPrincipal {
    /// Private. This is the whole point of the type.
    standing: Standing,
}

impl AdmittedPrincipal {
    /// The standing that admitted this principal. Always `Approved` — exposed so a caller can log
    /// or assert it without being able to forge one.
    pub fn standing(&self) -> Standing {
        self.standing
    }
}

/// The pure, per-request admission decision (spec D1).
///
/// Takes the raw column value so that parsing — and therefore spec §7 obligation 2 — happens
/// inside the machine rather than at a call site that might default.
///
/// **One argument, deliberately.** Admission reads standing and nothing else (D15 obligation 1):
/// a `Revoked` principal is refused whether or not a review is pending, and ANDing any second
/// provisional fact into this decision restores exactly the bug shape D2 forbids. A future change
/// that adds a parameter here should be rejected at review.
pub fn admit(raw_standing: Option<&str>) -> Result<AdmittedPrincipal, Refusal> {
    // Absence denies — not an error, not a default-grant (spec §7 obligation 1).
    let Some(raw) = raw_standing else {
        return Err(Refusal::NoStanding);
    };

    // An unrecognized state denies. Never a panic, never a default (spec §7 obligation 2).
    let Some(standing) = Standing::parse(raw) else {
        return Err(Refusal::UnrecognizedStanding {
            raw: raw.to_string(),
        });
    };

    // No catchall — adding a state is a compile error here (spec §7 obligation 3).
    match standing {
        Standing::Approved => Ok(AdmittedPrincipal { standing }),
        Standing::Denied => Err(Refusal::Denied),
        Standing::Requested => Err(Refusal::Requested),
        Standing::Revoked => Err(Refusal::Revoked),
        Standing::Deactivated => Err(Refusal::Deactivated),
    }
}
```

Restore the `admission` re-export line in `crates/temper-principal/src/lib.rs`:

```rust
pub use admission::{admit, AdmittedPrincipal};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p temper-principal`
Expected: PASS — all tests from Tasks 1–3.

- [ ] **Step 5: Prove `AdmittedPrincipal` is genuinely un-forgeable**

Add this to `crates/temper-principal/tests/matrix.rs` and confirm it does **not** compile, then delete it (a proof, not a change):

```rust
#[test]
fn forging_must_not_compile() {
    let _ = temper_principal::AdmittedPrincipal { standing: temper_principal::Standing::Approved };
}
```

Run: `cargo check -p temper-principal --tests`
Expected: FAIL with `error[E0451]: field 'standing' of struct 'AdmittedPrincipal' is private`.

If it compiles, the field is public and the guarantee is the same paper one `SystemAuthorized` has. Fix it before proceeding. **Then delete the test.**

- [ ] **Step 6: Run the full quality gate and commit**

Run: `cargo make check`
Expected: PASS.

```bash
git add crates/temper-principal/
git commit -m "feat(principal): admit() and a genuinely un-forgeable AdmittedPrincipal

Absence denies, an unrecognized state denies, no catchall. admit() takes ONE
argument by design (D15 obligation 1) — a second provisional fact would be the
conjunction shape D2 forbids.

NB the type-state guarantee is ADDED here, not inherited: SystemAuthorized has
a pub field and no impl block, so it never enforced what its doc claims."
```

---

## Beat B — persistence

Everything here is SQL. Beat A must land first (Beat D wires them together, but Beat B's function signatures are designed against Beat A's act names).

---

### Task 4: The standing, log, and governance tables

**Files:**
- Create: `migrations/20260720000010_principal_standing.sql`
- Test: `crates/temper-services/tests/standing_totality_test.rs` (created here, extended in Task 7)

**Interfaces:**
- Produces: tables `kb_principal_standing`, `kb_principal_standing_events`, `kb_principal_governance`; the `principal_standing` and `principal_act` enum-shaped CHECK domains.

**GD-3 tag: EXTEND.** Authorized by spec §10 ("Persistence"). The dedicated log is **new construction, not pattern-following** — see **C4**: the repo's atomic functions are two-part (mutate row, `PERFORM _event_append`) and no separate transition-log table exists anywhere.

**Invariants, carried verbatim:**
> D2: "Standing is **one authoritative state in one table**. Not a conjunction across tables."

> D9: "The practical consequence for implementation: **do not carry `team_id` into the standing tables**, and do not treat the existing unique index as evidence that standing needs a per-team key."

**Design decisions this task locks in, with their reasons:**

- **`state` is `text` with a `CHECK`, not a Postgres `ENUM`.** `system_access` is an enum and that is precisely what makes it painful: adding a value needs `ALTER TYPE`, which cannot run in the same transaction as its use on older PG and complicates rollback. A `CHECK` constraint on `text` is alterable additively, and spec §7 obligation 2 already requires the *reader* to tolerate an unknown value — an enum would make an unknown value unrepresentable at write time but does nothing for a binary reading a value added after it shipped.
- **No `team_id` anywhere in these tables** (D9).
- **Governance is its own table, not a column on standing.** D10 puts admin-ness somewhere that is not `kb_team_members`; making it a column on `kb_principal_standing` would re-couple the two questions the spec spent §2 separating.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-services/tests/standing_totality_test.rs`:

```rust
//! Structural guarantees of the standing tables (spec §10, D2, D7, D9).

use sqlx::PgPool;

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn standing_is_one_row_per_principal(pool: PgPool) {
    let profile: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('t1','T1') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'denied')")
        .bind(profile)
        .execute(&pool)
        .await
        .expect("first insert");

    // D2: ONE authoritative state. A second row for the same principal must be impossible.
    let err = sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'approved')")
        .bind(profile)
        .execute(&pool)
        .await
        .expect_err("a second standing row must be refused");

    let db = err.as_database_error().expect("a database error");
    assert_eq!(
        db.code().as_deref(),
        Some("23505"),
        "must fail as a unique violation — standing is one row per principal (D2). Got: {db}"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unknown_state_literal_is_refused_at_write_time(pool: PgPool) {
    let profile: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('t2','T2') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let err = sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'admin')")
        .bind(profile)
        .execute(&pool)
        .await
        .expect_err("an unknown state must be refused");

    assert_eq!(
        err.as_database_error().unwrap().code().as_deref(),
        Some("23514"),
        "must fail the CHECK constraint"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_standing_tables_carry_no_team_dimension(pool: PgPool) {
    // D9: "do not carry team_id into the standing tables". Asking to join a TEAM is orthogonal to
    // standing in the SYSTEM; conflating them is what put a team_id on a system-access request.
    for table in ["kb_principal_standing", "kb_principal_standing_events", "kb_principal_governance"] {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.columns
              WHERE table_name = $1 AND column_name LIKE '%team%'",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "{table} must carry no team dimension (D9)");
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_standing_log_is_append_only_in_shape(pool: PgPool) {
    // The log has no UPDATE path in the design; assert it at least records both endpoints of a
    // transition, so `Reactivate` has something to restore from (spec §5, §11).
    for col in ["prior_state", "resulting_state", "act", "actor_profile_id", "occurred_at"] {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.columns
              WHERE table_name = 'kb_principal_standing_events' AND column_name = $1",
        )
        .bind(col)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "the standing log must record {col}");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo make docker-up` then
`DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test standing_totality_test`
Expected: FAIL — `relation "kb_principal_standing" does not exist`.

- [ ] **Step 3: Write the migration**

Create `migrations/20260720000010_principal_standing.sql`:

```sql
-- Principal admission — the persisted half (spec 2026-07-20 §10, D2/D7/D9/D10).
--
-- ONE AUTHORITATIVE STATE IN ONE TABLE (D2). The question "may this principal use this instance?"
-- previously required ANDing conditions across kb_profiles.system_access, kb_profiles.is_active,
-- gating-team membership, and kb_join_requests.status -- written by uncoordinated call sites whose
-- meanings differed by which door the principal entered through. That shape produced two latent
-- bugs in a single morning, neither visible in the diff that would have introduced it. This table
-- gives the question exactly one owner.
--
-- `state` IS text + CHECK, NOT A POSTGRES ENUM, and that is deliberate. `system_access` is an enum
-- and that is exactly what makes it awkward: adding a value needs ALTER TYPE, with transaction and
-- rollback constraints a CHECK does not have. An enum would also buy nothing for the obligation
-- that actually matters -- spec §7 obligation 2 is about a BINARY reading a value added after it
-- shipped, which no write-time constraint can help with. The Rust reader is total by construction
-- (`Standing::parse` returns Option and None refuses); this CHECK guards the write side only.
--
-- NO team_id, ANYWHERE IN THIS FILE (D9). `kb_join_requests` is shaped as though requests were
-- per-team -- team_id is a NOT NULL FK and the uniqueness constraint is
-- (team_id, requesting_profile_id) WHERE status = 'pending' -- but `create_join_request` only ever
-- targets the gating team (access_service.rs:680-688, it resolves gating_team_slug and errors if
-- none). So every row that exists is really "may I use this instance?" wearing a per-team shape.
-- Those are two different questions and this table asks only the first one. Do not read the
-- existing unique index as evidence that standing needs a per-team key.
--
-- CONNECTION PROFILES GET NO ROW AT ALL (D7). Absence denies, so their safety is structural rather
-- than a check someone can forget. The backfill's rule 0 enforces this and there is no
-- discriminator column to key on -- kind is inferable only via NOT EXISTS against kb_connections.

CREATE TABLE kb_principal_standing (
    profile_id  uuid PRIMARY KEY REFERENCES kb_profiles(id) ON DELETE CASCADE,
    state       text NOT NULL CHECK (state IN ('denied','requested','approved','revoked','deactivated')),
    updated     timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_principal_standing IS
  'The one authoritative answer to "may this principal use this instance?" (spec 2026-07-20 D2). '
  'Absence denies. Written ONLY through the transition functions in 20260720000030 -- a direct '
  'UPDATE bypasses the log and the ledger event, which is the exact drift this design removes.';

-- Fast lookup of everyone awaiting a decision, for /admin/access. Partial: the interesting
-- population is tiny and the table is one row per principal.
CREATE INDEX idx_principal_standing_pending
    ON kb_principal_standing (state)
    WHERE state IN ('requested','denied');

-- ---------------------------------------------------------------------------------------------
-- The append-only transition log.
--
-- NEW CONSTRUCTION, not a pattern this repo already has. Every atomic function in migrations/ is
-- TWO-part: mutate the projection row, then PERFORM _event_append. There is no existing separate
-- transition-log table anywhere (the only log-shaped table is kb_resource_audits, which no
-- _event_append function writes). D4 requires both halves here, so this is the first of its kind.
--
-- WHY BOTH, given kb_events already records the act: `Reactivate` must restore the prior state
-- rather than guess it (spec §5), and reading that from kb_events would put the admission machine
-- behind the admin-ledger read gate -- which dispatches per act and is a very different question
-- from "what was this principal's standing before it was deactivated?". One cheap local read
-- beats coupling the gate to the ledger.
-- ---------------------------------------------------------------------------------------------
CREATE TABLE kb_principal_standing_events (
    id                uuid PRIMARY KEY DEFAULT uuid_generate_v7(),
    profile_id        uuid NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    act               text NOT NULL CHECK (act IN (
                        'provision','request','withdraw','approve','reject',
                        'revoke','deactivate','reactivate','request_review')),
    -- NULL exactly once per principal: the `provision` that created them, which has no prior.
    prior_state       text CHECK (prior_state IN ('denied','requested','approved','revoked','deactivated')),
    resulting_state   text NOT NULL CHECK (resulting_state IN ('denied','requested','approved','revoked','deactivated')),
    -- NULL for the boot-seed genesis act and for backfilled rows: there is no actor to name.
    actor_profile_id  uuid REFERENCES kb_profiles(id),
    reason            text,
    occurred_at       timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_principal_standing_events IS
  'Append-only. NEVER UPDATE OR DELETE A ROW HERE. `Reactivate` reads the most recent '
  'resulting_state before the deactivation to restore rather than guess (spec §5).';

-- The read `Reactivate` performs: most recent entry for a principal, walking backwards.
CREATE INDEX idx_principal_standing_events_lookup
    ON kb_principal_standing_events (profile_id, occurred_at DESC);

-- ---------------------------------------------------------------------------------------------
-- Governance (D10) -- shipped at the outset rather than deferred to a second spec.
--
-- WHY THIS TABLE EXISTS AT ALL. The original design deferred governance and defined only a seam.
-- The seam did not hold: `is_system_admin` reads a kb_team_members row, so "maintain 'admin
-- implies Approved' by firing a demotion on transition" means adding a TWENTY-FIRST uncoordinated
-- writer to the system's most-written table -- §1's own diagnosis, prescribed as the cure.
--
-- What makes moving it cheap is narrow and was measured: gating-team ownership has exactly ONE
-- authorization reader, the SQL function `is_system_admin`. Every Rust caller (21 production call
-- sites) goes through access_service::is_system_admin, which is a pure passthrough
-- (`SELECT is_system_admin($1)`), and the one in-database caller
-- (20260715000010_context_reassign_fns.sql:76) calls the SQL function directly. So repointing the
-- SQL BODY -- done in 20260720000040 -- moves all 22 call sites at once, and gating-team ownership
-- stops carrying authorization meaning at all. The ~20 writers to kb_team_members become ordinary
-- team-role churn because there is no longer authority stored there to alter.
--
-- ONE ROW PER ADMIN, not a boolean column on kb_principal_standing. Keeping governance in its own
-- table is what keeps "may you act" and "may you govern" two questions (spec §2); a column would
-- re-couple exactly what §2 separates.
-- ---------------------------------------------------------------------------------------------
CREATE TABLE kb_principal_governance (
    profile_id   uuid PRIMARY KEY REFERENCES kb_profiles(id) ON DELETE CASCADE,
    granted_by   uuid REFERENCES kb_profiles(id),   -- NULL for the boot-seed genesis admin
    granted_at   timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_principal_governance IS
  'Who may change the rules (spec 2026-07-20 D10). The presence of a row IS admin-ness. '
  'INVARIANT: admin implies standing = approved -- you cannot govern an instance you may not use. '
  'Enforced by the promote path and by Revoke/Deactivate demoting (20260720000030), never by an '
  'AND at read time.';
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test standing_totality_test`
Expected: PASS — 4 tests.

- [ ] **Step 5: Verify the migration applies cleanly from scratch**

Run:
```bash
export PGPASSWORD=temper
psql -h localhost -p 5437 -U temper -d temper_development -c '\d kb_principal_standing' \
  -c '\d kb_principal_standing_events' -c '\d kb_principal_governance'
```
Expected: all three tables present, `kb_principal_standing.profile_id` is the PRIMARY KEY, and **no column matching `%team%` in any of them**.

- [ ] **Step 6: Commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make check
git add migrations/20260720000010_principal_standing.sql crates/temper-services/tests/standing_totality_test.rs .sqlx crates/temper-services/.sqlx
git commit -m "feat(standing): the standing, log, and governance tables

D2 one row per principal; D9 no team dimension; D10 governance at the outset in
its own table. state is text+CHECK not an enum — §7 obligation 2 is about a
binary reading a value added after it shipped, which no write constraint helps."
```

---

### Task 5: Event payloads and their registrations

**Files:**
- Modify: `crates/temper-substrate/src/payloads.rs`
- Modify: `crates/temper-substrate/tests/payload_schema.rs`
- Create: `crates/temper-substrate/tests/fixtures/payloads/principal_standing_changed.v1.schema.json` (generated, not hand-written)
- Create: `migrations/20260720000020_principal_standing_events.sql`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `temper_substrate::payloads::PrincipalStandingChanged`, `PrincipalGovernanceChanged`; the two registered event types `principal_standing_changed` and `principal_governance_changed`, both `category = 'admin'`.

**GD-3 tag: CONFORM.** The event-type registration mechanics are entirely existing, load-bearing constraints. The template is `migrations/20260719000020_slack_disconnect_event.sql` — one migration old, and the only file containing both a modern (`category`-spelled) registration and an atomic transition function.

**Two acts, one event type each, deliberately.** Nine acts do **not** get nine event types. The act rides in the payload; the type distinguishes *standing* changes from *governance* changes, which is the distinction §2 spends its length on. Nine types would mean nine schema snapshots, nine registrations, and nine `verify_ledger_roundtrip` arms for a difference already carried in a field.

**The mechanical checklist — every item is load-bearing:**

1. Struct in `payloads.rs` with **both** derives.
2. Name appended to `TYPED_EVENT_NAMES` **and its `[&str; N]` arity bumped**.
3. Name appended to `ADMIN_EVENT_NAMES` **and its arity bumped**. Omitting this loses the `category = 'admin'` classification on any path that rebuilds the registry from scratch (`bootseed::seed_system` after a `reset_schema` truncate).
4. A `verify_ledger_roundtrip` arm. **This is not compiler-forced** — there is a `_ => {}` arm, so omission is silent.
5. `check::<p::X>("x")` line in `payload_schema.rs`.
6. `UPDATE_SCHEMA=1 cargo make test-schema` to generate the fixture.
7. That fixture pasted **verbatim** into the migration's `$JS$…$JS$`.

**Do not add these names to `system.yaml`.** `seed_migration_event_types_match_system_yaml` (`bootseed.rs:117-124`) asserts every `system.yaml` name also appears in `migrations/20260624000003_canonical_seed.sql` — a shipped, applied migration that must not be edited. The precedent is `admin_ledger_opened`: typed, stamped by its own forward migration, absent from `system.yaml`.

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-substrate/tests/payload_schema.rs`, inside `payload_schemas_match_snapshots`:

```rust
    check::<p::PrincipalStandingChanged>("principal_standing_changed");
    check::<p::PrincipalGovernanceChanged>("principal_governance_changed");
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo make test-schema`
Expected: FAIL to compile — `no variant or associated item named PrincipalStandingChanged`.

- [ ] **Step 3: Write the payload structs**

Add to `crates/temper-substrate/src/payloads.rs`, next to the admin-ledger payload block (near `GrantCreated` at `:930`):

```rust
/// `principal_standing_changed` — one principal-admission transition (spec 2026-07-20 §10, D4).
///
/// ONE EVENT TYPE FOR ALL NINE ACTS, with the act in the payload. Nine types would mean nine
/// schema snapshots, nine registrations and nine roundtrip arms for a distinction already carried
/// in a field. The type boundary that IS worth drawing is standing-vs-governance, because that is
/// the boundary spec §2 separates: "may you act" and "may you govern" are two questions.
///
/// `prior` is `None` exactly once per principal — the `provision` that created them.
/// `actor` is `None` for the boot-seed genesis act and for backfilled rows: there is no actor to
/// name, and inventing one would put a fabricated attribution on the ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct PrincipalStandingChanged {
    pub subject_table: AnchorTable,
    pub subject_id: Uuid,
    pub act: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior: Option<String>,
    pub resulting: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `principal_governance_changed` — a principal gained or lost the authority to change the rules
/// (spec 2026-07-20 D10).
///
/// Separate from `PrincipalStandingChanged` because governance is the separate question. A demote
/// fired as a consequence of `Revoke`/`Deactivate` emits BOTH events in the same transaction, and
/// the pair is what makes the causal story legible in the ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct PrincipalGovernanceChanged {
    pub subject_table: AnchorTable,
    pub subject_id: Uuid,
    /// `granted` | `revoked`.
    pub change: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
```

Bump both consts (currently `TYPED_EVENT_NAMES: [&str; 19]` at `:996` and `ADMIN_EVENT_NAMES: [&str; 4]` at `:1023`):

```rust
pub const TYPED_EVENT_NAMES: [&str; 21] = [
    // ... existing 19, unchanged ...
    "principal_standing_changed",
    "principal_governance_changed",
];

pub const ADMIN_EVENT_NAMES: [&str; 6] = [
    "admin_ledger_opened",
    "grant_created",
    "grant_revoked",
    "slack_principal_disconnected",
    "principal_standing_changed",
    "principal_governance_changed",
];
```

Add the two `verify_ledger_roundtrip` arms alongside the existing ones. **The `_ => {}` arm means omitting these is silent, not a compile error** — this is the single highest-risk omission in the task:

```rust
        "principal_standing_changed" => {
            let _: PrincipalStandingChanged = serde_json::from_value(payload.clone())?;
        }
        "principal_governance_changed" => {
            let _: PrincipalGovernanceChanged = serde_json::from_value(payload.clone())?;
        }
```

- [ ] **Step 4: Generate the snapshots and verify**

Run: `UPDATE_SCHEMA=1 cargo make test-schema`
Expected: two new files under `crates/temper-substrate/tests/fixtures/payloads/`.

Run: `cargo make test-schema`
Expected: PASS, including `snapshot_files_cover_exactly_the_typed_names` — which fails if you added a `check::<>` without a fixture, or a `TYPED_EVENT_NAMES` entry without both.

**Do not run this under `--workspace`.** Under workspace feature unification temper-core's `mcp` feature pulls in and the id newtypes emit **inline** rather than as `$ref`s into `$defs` — the same structs, two different schemas, decided by the cargo invocation. `-p temper-substrate` is authoritative because it is what the boot-seed stamps.

- [ ] **Step 5: Write the registration migration**

Create `migrations/20260720000020_principal_standing_events.sql`. Paste each generated fixture **byte-for-byte** into its `$JS$…$JS$` block:

```sql
-- Register the two principal-admission event types (spec 2026-07-20 §10, D4).
--
-- CATEGORY IS SPELLED AT REGISTRATION, not stamped afterwards. 20260719000010 dropped the
-- `category` DEFAULT precisely so an omission fails 23502 at apply time naming the column, rather
-- than silently joining the trail allowlist. `registering_an_event_type_requires_an_explicit_category`
-- (crates/temper-services/tests/admin_ledger_test.rs:1002) pins that behaviour.
--
-- `admin` also buys two runtime guarantees for free: kb_events_admin_is_unanchored (admin implies a
-- NULL producing anchor, which the transition functions satisfy by passing NULL,NULL) and the
-- element-trail allowlist, which admits only `domain`. An admission act is an authority act; it has
-- no cognition home, and anchoring it would put it in front of every region producer and break the
-- "governance is traceable, but it isn't knowledge" boundary.
--
-- NOT ADDED TO system.yaml, deliberately. seed_migration_event_types_match_system_yaml
-- (bootseed.rs:117-124) requires every system.yaml name to also appear in canonical_seed.sql -- a
-- shipped, applied migration that must not be edited. `admin_ledger_opened` set the precedent.
--
-- THE JSON BELOW IS GENERATED, NOT AUTHORED. It is copied byte-for-byte from
-- crates/temper-substrate/tests/fixtures/payloads/*.v1.schema.json, emitted by
-- `UPDATE_SCHEMA=1 cargo make test-schema` (package-scoped -p temper-substrate; the workspace
-- invocation emits a different, unstamped shape). Editing it here desynchronizes repo, registry,
-- and Rust types.
--
-- Template: 20260719000020_slack_disconnect_event.sql.

INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('principal_standing_changed', $JS$
  <<< paste crates/temper-substrate/tests/fixtures/payloads/principal_standing_changed.v1.schema.json verbatim >>>
$JS$::jsonb, 1, 'admin'),
  ('principal_governance_changed', $JS$
  <<< paste crates/temper-substrate/tests/fixtures/payloads/principal_governance_changed.v1.schema.json verbatim >>>
$JS$::jsonb, 1, 'admin')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;
```

> The `<<< paste … >>>` markers are instructions to the implementer, not content. They must not survive into the committed file — the migration will not apply with them present, which is the intended forcing function.

- [ ] **Step 6: Verify the registration applies and is classified**

Run:
```bash
export PGPASSWORD=temper
psql -h localhost -p 5437 -U temper -d temper_development -At \
  -c "SELECT name||' → '||category FROM kb_event_types WHERE name LIKE 'principal_%' ORDER BY name"
```
Expected:
```
principal_governance_changed → admin
principal_standing_changed → admin
```

- [ ] **Step 7: Commit**

```bash
cargo make check
git add crates/temper-substrate/ migrations/20260720000020_principal_standing_events.sql
git commit -m "feat(standing): register the two principal-admission event types

One type per QUESTION (standing vs governance), not one per act — the act rides
in the payload. Both category='admin', spelled explicitly: the DEFAULT was
dropped in 20260719000010 so an omission fails 23502.

Both ADMIN_EVENT_NAMES and the verify_ledger_roundtrip arms updated; the latter
has a _ => {} arm, so omitting it would have been silent."
```

---

### Task 6: The transition functions — row + log + event, one transaction

**Files:**
- Create: `migrations/20260720000030_principal_standing_fns.sql`
- Test: `crates/temper-services/tests/standing_transition_test.rs`

**Interfaces:**
- Produces:
  - `principal_standing_apply(p_profile uuid, p_act text, p_resulting text, p_actor uuid, p_reason text) RETURNS text` — the one writer. Returns the resulting state.
  - `principal_prior_standing(p_profile uuid) RETURNS text` — what `Reactivate` restores from.
  - `principal_governance_set(p_profile uuid, p_granted boolean, p_actor uuid, p_reason text) RETURNS boolean`.

**GD-3 tag: EXTEND (new construction).** Per **C4** the two-part shape (mutate row, `PERFORM _event_append`) is existing and load-bearing; the third part (the standing log) has no template. Template for the existing halves: `migrations/20260718000010_admin_grant_fns.sql` and `migrations/20260719000020_slack_disconnect_event.sql:144-206`.

**Invariant, carried verbatim from spec §10:**
> "**One SQL function per transition** writes the row, appends the log, and emits the `kb_events` record in a single transaction, so a standing change without its audit record is not representable."

**One function, not nine — and why that still satisfies §10.** §10 says "one SQL function per transition." Taken literally that is nine near-identical functions differing only in a string literal, which is the enumerate-don't-compose shape the repo's SQL guidance warns against. What §10 is *buying* is atomicity: a standing change without its audit record must be unrepresentable. A single `principal_standing_apply` that always writes all three in one statement buys exactly that, and buys it in one place rather than nine places that can drift. **The legality decision stays in `temper-principal`** (Task 2) — this function is the committer, not the judge, and it must never re-implement the table.

> **Do not add legality checks here.** If this function starts refusing transitions, there are two transition tables in two languages and they will disagree. The Rust machine decides; SQL commits.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-services/tests/standing_transition_test.rs`:

```rust
//! The transition committer: row + log + ledger event, atomically (spec §10, D4).

use sqlx::PgPool;

async fn a_profile(pool: &PgPool, handle: &str) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn apply_writes_row_log_and_event_together(pool: PgPool) {
    let p = a_profile(&pool, "applies").await;

    let state: String = sqlx::query_scalar(
        "SELECT principal_standing_apply($1,'provision','denied',NULL,NULL)",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, "denied");

    let row: String = sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
        .bind(p)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row, "denied", "the projection row must exist");

    let log: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_principal_standing_events WHERE profile_id=$1 AND act='provision'",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(log, 1, "the log entry must exist");

    let ev: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id
          WHERE t.name = 'principal_standing_changed' AND e.payload->>'subject_id' = $1::text",
    )
    .bind(p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ev, 1, "the ledger event must exist — D4 makes the trio atomic");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_log_records_the_prior_state_so_reactivate_can_restore(pool: PgPool) {
    let p = a_profile(&pool, "restores").await;
    let admin = a_profile(&pool, "restores-admin").await;

    for (act, resulting) in [
        ("provision", "denied"),
        ("request", "requested"),
        ("approve", "approved"),
        ("deactivate", "deactivated"),
    ] {
        sqlx::query_scalar::<_, String>("SELECT principal_standing_apply($1,$2,$3,$4,NULL)")
            .bind(p)
            .bind(act)
            .bind(resulting)
            .bind(admin)
            .fetch_one(&pool)
            .await
            .unwrap();
    }

    // Spec §5: "Prior standing is recoverable from the log, so reactivation restores rather than
    // guesses."
    let prior: Option<String> = sqlx::query_scalar("SELECT principal_prior_standing($1)")
        .bind(p)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        prior.as_deref(),
        Some("approved"),
        "the state immediately before deactivation must be recoverable"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn prior_standing_is_null_when_there_is_nothing_to_restore(pool: PgPool) {
    let p = a_profile(&pool, "no-prior").await;
    let prior: Option<String> = sqlx::query_scalar("SELECT principal_prior_standing($1)")
        .bind(p)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(prior.is_none(), "must be NULL, so the Rust machine refuses rather than guesses");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_standing_change_without_its_audit_record_is_not_representable(pool: PgPool) {
    // D4's whole point. Assert the counts move together across a sequence.
    let p = a_profile(&pool, "atomic").await;
    let admin = a_profile(&pool, "atomic-admin").await;

    for (act, resulting) in [("provision", "denied"), ("approve", "approved"), ("revoke", "revoked")] {
        sqlx::query_scalar::<_, String>("SELECT principal_standing_apply($1,$2,$3,$4,'because')")
            .bind(p).bind(act).bind(resulting).bind(admin)
            .fetch_one(&pool).await.unwrap();
    }

    let logs: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_principal_standing_events WHERE profile_id=$1")
        .bind(p).fetch_one(&pool).await.unwrap();
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id
          WHERE t.name='principal_standing_changed' AND e.payload->>'subject_id' = $1::text",
    ).bind(p).fetch_one(&pool).await.unwrap();

    assert_eq!(logs, 3);
    assert_eq!(events, 3, "one ledger event per transition, always");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn admin_events_are_unanchored(pool: PgPool) {
    // kb_events_admin_is_unanchored: admin category implies a NULL producing anchor. An admission
    // act is an authority act with no cognition home; anchoring it would put it in front of every
    // region producer.
    let p = a_profile(&pool, "unanchored").await;
    sqlx::query_scalar::<_, String>("SELECT principal_standing_apply($1,'provision','denied',NULL,NULL)")
        .bind(p).fetch_one(&pool).await.unwrap();

    let anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id
          WHERE t.name='principal_standing_changed'
            AND (e.producing_anchor_table IS NOT NULL OR e.producing_anchor_id IS NOT NULL)",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(anchored, 0);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn governance_set_is_idempotent_and_emits_only_on_change(pool: PgPool) {
    let p = a_profile(&pool, "gov").await;
    let admin = a_profile(&pool, "gov-admin").await;

    let first: bool = sqlx::query_scalar("SELECT principal_governance_set($1,true,$2,NULL)")
        .bind(p).bind(admin).fetch_one(&pool).await.unwrap();
    let second: bool = sqlx::query_scalar("SELECT principal_governance_set($1,true,$2,NULL)")
        .bind(p).bind(admin).fetch_one(&pool).await.unwrap();

    assert!(first, "the first grant changes something");
    assert!(!second, "the second is a no-op");

    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id
          WHERE t.name='principal_governance_changed' AND e.payload->>'subject_id' = $1::text",
    ).bind(p).fetch_one(&pool).await.unwrap();
    assert_eq!(events, 1, "a no-op is not an admin act; the ledger is append-only and a spurious row can never be corrected, only quarantined");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test standing_transition_test`
Expected: FAIL — `function principal_standing_apply(...) does not exist`.

- [ ] **Step 3: Write the migration**

Create `migrations/20260720000030_principal_standing_fns.sql`:

```sql
-- The principal-admission committer (spec 2026-07-20 §10, D4).
--
-- WHY SQL AND NOT RUST: production event emission is SQL-resident in this repo. There are ZERO
-- production `INSERT INTO kb_events` statements in crates/ -- the four grep hits are all inside
-- #[cfg(all(test, feature = "test-db"))] modules -- and substrate's events.rs describes itself as
-- the firing surface for "seeding, scenario loading, and tests", while "the SQL functions stay the
-- atomic event+materialize+commit mechanism". Admission acts follow that shape.
--
-- ONE COMMITTER, NOT NINE. §10 says "one SQL function per transition". Taken literally that is
-- nine functions differing by a string literal -- the enumerate-don't-compose shape. What §10 is
-- BUYING is atomicity: a standing change without its audit record must not be representable. One
-- function that always writes all three in one statement buys exactly that, in one place rather
-- than nine that can drift.
--
-- THIS FUNCTION DOES NOT DECIDE LEGALITY, AND MUST NEVER START. The transition table lives in
-- temper-principal (Rust, exhaustive, no catchall) and is tested as a pure matrix with no
-- database. If this function grows a legality check there are two transition tables in two
-- languages and they will disagree -- which is the class of bug this entire design removes. The
-- Rust machine judges; SQL commits.
--
-- BOTH-NULL PRODUCING ANCHOR, always. An admission act is an authority act; it has no cognition
-- home. Anchoring it would put it in front of every region producer and break the "governance is
-- traceable, but it isn't knowledge" boundary. kb_events_admin_is_unanchored enforces it.
--
-- Template: 20260718000010_admin_grant_fns.sql and 20260719000020_slack_disconnect_event.sql:144.

CREATE FUNCTION principal_standing_apply(
    p_profile   uuid,
    p_act       text,
    p_resulting text,
    p_actor     uuid    DEFAULT NULL,
    p_reason    text    DEFAULT NULL
) RETURNS text
LANGUAGE plpgsql AS $$
DECLARE
    v_prior   text;
    v_emitter uuid;
BEGIN
    -- The prior state, captured BEFORE the upsert -- it is the log's whole value.
    SELECT state INTO v_prior FROM kb_principal_standing WHERE profile_id = p_profile;

    INSERT INTO kb_principal_standing (profile_id, state, updated)
    VALUES (p_profile, p_resulting, now())
    ON CONFLICT (profile_id) DO UPDATE
      SET state = EXCLUDED.state, updated = EXCLUDED.updated;

    INSERT INTO kb_principal_standing_events
        (profile_id, act, prior_state, resulting_state, actor_profile_id, reason)
    VALUES (p_profile, p_act, v_prior, p_resulting, p_actor, p_reason);

    -- Events need a NOT NULL emitter. Prefer the acting principal's emitter entity; fall back to
    -- the canonical `system` actor, which bootseed.rs:31 guarantees exists.
    SELECT id INTO v_emitter FROM kb_entities
     WHERE profile_id = COALESCE(p_actor, p_profile) LIMIT 1;
    IF v_emitter IS NULL THEN
        SELECT e.id INTO v_emitter
          FROM kb_entities e JOIN kb_profiles pr ON pr.id = e.profile_id
         WHERE pr.handle = 'system' LIMIT 1;
    END IF;
    IF v_emitter IS NULL THEN
        RAISE EXCEPTION 'no emitter entity available for a principal-standing event (profile %)', p_profile;
    END IF;

    PERFORM _event_append(
        'principal_standing_changed', v_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'subject_table', 'kb_profiles',
            'subject_id',    p_profile,
            'act',           p_act,
            'prior',         v_prior,
            'resulting',     p_resulting,
            'actor',         p_actor,
            'reason',        p_reason)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_profiles','id', p_profile))));

    RETURN p_resulting;
END;
$$;

COMMENT ON FUNCTION principal_standing_apply IS
  'The ONE writer of kb_principal_standing. Commits row + log + ledger event in one transaction '
  '(spec §10, D4). Does NOT decide legality -- temper-principal does, and duplicating that here '
  'would create two transition tables in two languages.';

-- ---------------------------------------------------------------------------------------------
-- What `Reactivate` restores from (spec §5: "restores rather than guesses").
--
-- Returns the resulting_state of the most recent entry BEFORE the deactivation, or NULL when
-- there is nothing to restore -- in which case the Rust machine refuses (Refusal::NoPriorStanding)
-- rather than defaulting. NULL here is a decision, not an accident.
--
-- Backfilled rows would otherwise always hit the NULL arm, since the log begins at migration time.
-- 20260720000050's genesis pass writes a synthetic entry for exactly that reason.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION principal_prior_standing(p_profile uuid) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT prior_state
      FROM kb_principal_standing_events
     WHERE profile_id = p_profile
       AND act = 'deactivate'
     ORDER BY occurred_at DESC
     LIMIT 1
$$;

-- ---------------------------------------------------------------------------------------------
-- Governance (D10). Idempotent, and emits ONLY on a real change: a no-op is not an admin act, and
-- the ledger is append-only -- a spurious row can never be corrected, only quarantined.
-- ---------------------------------------------------------------------------------------------
CREATE FUNCTION principal_governance_set(
    p_profile uuid,
    p_granted boolean,
    p_actor   uuid DEFAULT NULL,
    p_reason  text DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_changed boolean := false;
    v_emitter uuid;
BEGIN
    IF p_granted THEN
        INSERT INTO kb_principal_governance (profile_id, granted_by)
        VALUES (p_profile, p_actor)
        ON CONFLICT (profile_id) DO NOTHING;
        GET DIAGNOSTICS v_changed = ROW_COUNT;
        v_changed := (v_changed::int > 0);
    ELSE
        DELETE FROM kb_principal_governance WHERE profile_id = p_profile;
        GET DIAGNOSTICS v_changed = ROW_COUNT;
        v_changed := (v_changed::int > 0);
    END IF;

    IF v_changed THEN
        SELECT id INTO v_emitter FROM kb_entities
         WHERE profile_id = COALESCE(p_actor, p_profile) LIMIT 1;
        IF v_emitter IS NULL THEN
            SELECT e.id INTO v_emitter FROM kb_entities e
              JOIN kb_profiles pr ON pr.id = e.profile_id WHERE pr.handle = 'system' LIMIT 1;
        END IF;

        PERFORM _event_append(
            'principal_governance_changed', v_emitter, NULL, NULL,
            jsonb_strip_nulls(jsonb_build_object(
                'subject_table', 'kb_profiles',
                'subject_id',    p_profile,
                'change',        CASE WHEN p_granted THEN 'granted' ELSE 'revoked' END,
                'actor',         p_actor,
                'reason',        p_reason)),
            p_references => jsonb_build_array(
                jsonb_build_object('rel','subject',
                    'target', jsonb_build_object('kind','kb_profiles','id', p_profile))));
    END IF;

    RETURN v_changed;
END;
$$;

COMMENT ON FUNCTION principal_governance_set IS
  'Grant or revoke the authority to change the rules (spec D10). Idempotent; emits only on a real '
  'change. INVARIANT (enforced by callers): admin implies standing = approved.';
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test standing_transition_test`
Expected: PASS — 6 tests.

- [ ] **Step 5: Commit**

```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make check
git add migrations/20260720000030_principal_standing_fns.sql crates/temper-services/tests/standing_transition_test.rs .sqlx crates/temper-services/.sqlx
git commit -m "feat(standing): the transition committer — row + log + event, one txn

One committer, not nine: §10's atomicity is what matters and one function buys
it in one place. Legality stays in temper-principal — two transition tables in
two languages would be the bug class this design removes."
```

---

### Task 7: Repoint `has_system_access` and `is_system_admin`

**Files:**
- Create: `migrations/20260720000040_repoint_predicates.sql`
- Modify: `crates/temper-services/tests/standing_totality_test.rs` (add the totality cases)

**Interfaces:**
- Consumes: `kb_principal_standing`, `kb_principal_governance` (Task 4).
- Produces: repointed bodies for both SQL functions. **Signatures are unchanged**, so all 22 call sites follow with no code change.

**GD-3 tag: AMEND.** Both functions exist; the spec authorizes changing their basis (§7: *"`has_system_access` stays a SQL function reading the standing table"*; §9: *"`is_system_admin` reads governance state directly"*).

**This is the cutover.** Everything before it is inert; after it, standing is the gate. It is deliberately its own migration so it can be reasoned about and, if necessary, reverted alone.

**Invariant, carried verbatim from spec §7:**
> "It **must** be written as `EXISTS(SELECT 1 … WHERE state = 'approved')`, not `SELECT state = 'approved' FROM …`. With no matching row the latter returns **`NULL`, and `NULL` is not `false`**. `EXISTS` is total."

**The hazard, measured on local dev 2026-07-20** (this table is a measurement, not an argument):

| caller shape | scalar form, no row | `EXISTS` form, no row |
|---|---|---|
| plpgsql `IF NOT <pred> THEN RETURN` | **falls through — proceeds** | guard fires — denies |
| `WHERE <pred>` | row excluded | row excluded |

Reproduced exactly, with `kb_system_settings` emptied in `BEGIN/ROLLBACK`:
```
empty-settings has_system_access = <NULL> (is null: t)
IF NOT has_system_access(...) => GUARD DID NOT FIRE
WHERE-shape rows returned = 0
```

The two `IF NOT` sites that this protects — the only two in the repo — are `20260629000002_auto_join_team_generalization.sql:44` (`IF NOT has_system_access`) and `20260715000010_context_reassign_fns.sql:76` (`IF NOT is_system_admin`, which fails open into **system admin**, the higher blast radius). Both are fixed by making the predicates total; neither file is edited.

**Ordering matters within this migration.** Repoint `is_system_admin` **first**. Both predicates are read during the same deploy, and `is_system_admin` guards the higher-blast-radius `IF NOT`.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-services/tests/standing_totality_test.rs`:

```rust
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn both_predicates_are_total(pool: PgPool) {
    // Spec §7: "SQL totality has its own test — has_system_access and is_system_admin return
    // non-NULL for a profile with no standing row, a deactivated one, and an unknown state value."
    let absent = a_profile(&pool, "absent").await;

    let deactivated = a_profile(&pool, "deactivated").await;
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'deactivated')")
        .bind(deactivated).execute(&pool).await.unwrap();

    // An unknown state cannot be inserted through the CHECK, so reach past it to simulate the
    // rolling-deploy window this obligation exists for.
    let unknown = a_profile(&pool, "unknown").await;
    sqlx::query("ALTER TABLE kb_principal_standing DROP CONSTRAINT kb_principal_standing_state_check")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'quarantined')")
        .bind(unknown).execute(&pool).await.unwrap();

    for (label, id) in [("absent", absent), ("deactivated", deactivated), ("unknown", unknown)] {
        for f in ["has_system_access", "is_system_admin"] {
            let v: Option<bool> = sqlx::query_scalar(&format!("SELECT {f}($1)"))
                .bind(id).fetch_one(&pool).await.unwrap();
            assert_eq!(
                v, Some(false),
                "{f}({label}) must be FALSE, never NULL — a NULL in `IF NOT` falls through, \
                 fail-OPEN, and context_reassign_fns.sql:76 falls open into system admin"
            );
        }
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn only_approved_standing_grants_access(pool: PgPool) {
    for state in ["denied", "requested", "revoked", "deactivated"] {
        let p = a_profile(&pool, &format!("s-{state}")).await;
        sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,$2)")
            .bind(p).bind(state).execute(&pool).await.unwrap();
        let v: Option<bool> = sqlx::query_scalar("SELECT has_system_access($1)")
            .bind(p).fetch_one(&pool).await.unwrap();
        assert_eq!(v, Some(false), "{state} must not grant access");
    }

    let ok = a_profile(&pool, "s-approved").await;
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,'approved')")
        .bind(ok).execute(&pool).await.unwrap();
    let v: Option<bool> = sqlx::query_scalar("SELECT has_system_access($1)")
        .bind(ok).fetch_one(&pool).await.unwrap();
    assert_eq!(v, Some(true));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn admin_ness_no_longer_reads_gating_team_ownership(pool: PgPool) {
    // D10: gating-team ownership stops being an authorization fact. Make someone a gating-team
    // OWNER without a governance row and assert they are NOT admin — this is the property that
    // makes the ~20 kb_team_members writers harmless.
    let p = a_profile(&pool, "gating-owner").await;
    let team: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','System')
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    ).fetch_one(&pool).await.unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'owner')")
        .bind(team).bind(p).execute(&pool).await.unwrap();

    let v: Option<bool> = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(p).fetch_one(&pool).await.unwrap();
    assert_eq!(
        v, Some(false),
        "owning the gating team must confer nothing once governance holds its own state (D10)"
    );

    sqlx::query("INSERT INTO kb_principal_governance (profile_id) VALUES ($1)")
        .bind(p).execute(&pool).await.unwrap();
    let v: Option<bool> = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(p).fetch_one(&pool).await.unwrap();
    assert_eq!(v, Some(true), "the governance row IS admin-ness now");
}
```

Add the `a_profile` helper to the top of the file if Task 4 did not already put one there.

- [ ] **Step 2: Run it to verify it fails**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test standing_totality_test`
Expected: FAIL — `admin_ness_no_longer_reads_gating_team_ownership` fails (the old body still reads team membership), and `only_approved_standing_grants_access` fails.

- [ ] **Step 3: Write the migration**

Create `migrations/20260720000040_repoint_predicates.sql`:

```sql
-- THE CUTOVER (spec 2026-07-20 §7, §9, D2, D10).
--
-- Everything before this migration is inert. After it, standing is the gate and governance is
-- admin-ness. Deliberately its own migration so it can be reasoned about -- and if necessary
-- reverted -- alone.
--
-- SIGNATURES ARE UNCHANGED, so all call sites follow with no code change. That is the whole
-- economics of D10, and the chokepoint is the SQL BODY, not the Rust wrapper: 21 production Rust
-- call sites reach is_system_admin through access_service::is_system_admin, which is a pure
-- passthrough (`SELECT is_system_admin($1)`), and one caller is IN-DATABASE
-- (20260715000010_context_reassign_fns.sql:76) and never touches Rust at all. Repointing the body
-- moves all 22 at once. (An earlier draft of the spec said "~12 Rust callers, ALL routed through
-- access_service" -- both halves are wrong, but the conclusion holds for this better reason.)
--
-- EXISTS, NOT A SCALAR COMPARISON. Measured on local dev 2026-07-20 with kb_system_settings
-- emptied, in BEGIN/ROLLBACK:
--
--     empty-settings has_system_access = <NULL> (is null: t)
--     IF NOT has_system_access(...) => GUARD DID NOT FIRE      <- fail-OPEN
--     WHERE-shape rows returned = 0                            <- fail-CLOSED
--
-- A NULL in a WHERE clause is fail-closed; a NULL in plpgsql `IF NOT` is fail-OPEN. There are
-- exactly two `IF NOT <predicate>` sites in this repo (a naive grep returns 14; twelve are
-- `IF NOT FOUND`, a row-count diagnostic):
--
--   20260629000002_auto_join_team_generalization.sql:44  IF NOT has_system_access(...)
--   20260715000010_context_reassign_fns.sql:76           IF NOT is_system_admin(...)   <- falls
--                                                        open into SYSTEM ADMIN
--
-- Both are fixed by making the predicates total here. Neither file is edited. The old bodies were
-- shaped `SELECT ... FROM settings`, so an empty kb_system_settings made both return NULL today --
-- a LATENT trap, not a live exploit (the row is seeded by 20260624000003_canonical_seed.sql:23,
-- pinned by CHECK (id = 1), and every production writer is an UPDATE; there are no DELETEs). The
-- new table must not inherit it.
--
-- ORDER IS DELIBERATE: is_system_admin first. Both are read during the same deploy and it guards
-- the higher-blast-radius site.

CREATE OR REPLACE FUNCTION is_system_admin(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    -- D10: admin-ness IS a governance row. Gating-team ownership no longer carries authorization
    -- meaning, which is what makes the ~20 uncoordinated writers to kb_team_members harmless --
    -- they became ordinary team-role churn the moment this body stopped reading them.
    --
    -- Note there is no AND against standing here. The invariant "admin implies Approved" is
    -- maintained by the transition (Revoke and Deactivate demote) and guarded at promotion, never
    -- checked at read time -- ANDing across two tables at read time is the exact shape D2 forbids.
    SELECT EXISTS (
        SELECT 1 FROM kb_principal_governance g WHERE g.profile_id = p_profile_id
    )
$$;

CREATE OR REPLACE FUNCTION has_system_access(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    -- D2: one authoritative state in one table. No access_mode, no gating-team membership, no
    -- tier. Absence denies structurally (spec §7 obligation 1), which is what makes D7's
    -- connection-profile safety hold without a check anyone can forget.
    SELECT EXISTS (
        SELECT 1 FROM kb_principal_standing s
         WHERE s.profile_id = p_profile_id
           AND s.state = 'approved'
    )
$$;

COMMENT ON FUNCTION has_system_access IS
  'May this principal use this instance? Reads kb_principal_standing and NOTHING else (spec D2). '
  'EXISTS, never a scalar comparison: a NULL here falls through plpgsql `IF NOT` guards, fail-OPEN.';
COMMENT ON FUNCTION is_system_admin IS
  'May this principal change the rules? Reads kb_principal_governance and NOTHING else (spec D10).';
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test standing_totality_test`
Expected: PASS — 7 tests.

- [ ] **Step 5: Verify the live bodies and re-measure the NULL behaviour**

Run:
```bash
export PGPASSWORD=temper
psql -h localhost -p 5437 -U temper -d temper_development -q -c '\sf has_system_access' -c '\sf is_system_admin'
```
Expected: both bodies are `SELECT EXISTS (...)` with no `FROM settings` tail.

Then re-run the §7 probe from *Grounding* C7 against the new functions:
```bash
psql -h localhost -p 5437 -U temper -d temper_development -q <<'SQL'
BEGIN;
DO $$
DECLARE v_id uuid; v_out text;
BEGIN
    INSERT INTO kb_profiles(handle,display_name) VALUES ('totality-probe','P') RETURNING id INTO v_id;
    DELETE FROM kb_system_settings;
    IF NOT has_system_access(v_id) THEN v_out := 'GUARD FIRED (denied)'; ELSE v_out := 'GUARD DID NOT FIRE'; END IF;
    RAISE NOTICE 'IF NOT has_system_access(...) => %', v_out;
    IF NOT is_system_admin(v_id) THEN v_out := 'GUARD FIRED (denied)'; ELSE v_out := 'GUARD DID NOT FIRE'; END IF;
    RAISE NOTICE 'IF NOT is_system_admin(...)  => %', v_out;
END $$;
ROLLBACK;
SQL
```
Expected: **both** now report `GUARD FIRED (denied)`. Before this migration, both reported `GUARD DID NOT FIRE`. That flip is the latent trap being closed.

- [ ] **Step 6: Commit**

```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make check
git add migrations/20260720000040_repoint_predicates.sql crates/temper-services/tests/standing_totality_test.rs .sqlx crates/temper-services/.sqlx
git commit -m "feat(standing): the cutover — repoint both predicates, EXISTS-total

Signatures unchanged, so all 22 call sites follow: 21 Rust sites via a
passthrough wrapper plus one in-database caller. The SQL body is the real
chokepoint, which is better than the Rust wrapper the spec named.

Both were NULL-returning on empty settings and both fed `IF NOT` guards that
fail OPEN — measured before and after. Latent, not live; now closed."
```

---

## Beat C — the backfill

Task 7 made standing the gate. Until this beat runs, **every existing principal has no standing row and is therefore denied.** Tasks 7 and 8 must land in the same PR and, on a live instance, the same deploy.

> **Ordering note for the implementer:** it is tempting to put the backfill migration *before* the repoint so there is never a denied window. Don't — the backfill must evaluate the **old** predicate (D8), and the old predicate only exists before Task 7's `CREATE OR REPLACE`. Migrations apply in one transaction per file, in filename order, and `20260720000050` runs after `20260720000040` — so the backfill would read the *new* body and produce a table that says nothing but `denied`. The backfill therefore captures the old predicate's verdict **as a materialized snapshot taken in its own migration, computed from first principles rather than by calling the repointed function.** That is why the rules below inline the old logic instead of calling `has_system_access`.

---

### Task 8: The backfill — four rules, two extra passes

**Files:**
- Create: `migrations/20260720000050_backfill_standing.sql`

**Interfaces:**
- Consumes: `kb_principal_standing`, `kb_principal_standing_events`, `kb_principal_governance`.
- Produces: one standing row per non-connection profile; a genesis log entry per row; governance rows for existing admins.

**GD-3 tag: EXTEND**, authorized by spec §11 and D8.

**The rule, transcribed verbatim from spec §11 — evaluated in order, first match wins:**

| # | condition | standing |
|---|---|---|
| 0 | profile is a **connection profile** | **no row at all** (D7) |
| 1 | `is_active = false` | `Deactivated` |
| 2 | `has_system_access(id) IS TRUE` | `Approved` |
| 3 | otherwise — including **`NULL`** | `Denied` |

**Why each clause is not optional:**

- **Rule 0 is load-bearing.** Under `access_mode = 'open'` the old predicate returns `true` for **every** profile, so a literal per-profile backfill would mint connection profiles `Approved` rows — "directly contradicting D7 and **dissolving the structural safety D7 claims**" (§11). There is no discriminator column; kind is inferable only via `NOT EXISTS (SELECT 1 FROM kb_connections …)`.
- **Rule 3 is written `IS TRUE … else Denied`, not `false → Denied`,** so that `NULL` is handled **by decision rather than by omission**. Today's predicate returns `NULL` when `kb_system_settings` is empty (§7), and a rule with only `true`/`false` arms would leave that to whatever the migration happened to do.
- **Rules 1 and 2 both match a deactivated principal whose old predicate is true, and the ordering is the decision.** `Deactivated` wins, because D6 folds `is_active` in and a principal who is disabled is disabled.

**What "behaviour-preserving" does and does not mean — carried verbatim from §11:**
> "**The predicate is not preserved for deactivated principals.** It flips `true → false`, deliberately."
> "**Auth-observable behaviour *is* preserved**, because `gate_resolved_profile` (`auth/mod.rs:242-246`) rejects `!profile.is_active` at **Level 1**, and `require_system_access` only accepts an `AuthenticatedProfile`."
> "**The behaviour that does change** is in the non-auth callers that pass a third party's id. `backfill_auto_join_team` (`WHERE has_system_access(p.id)`) today enrols deactivated profiles and will stop; `ensure_auto_join_memberships` and `access_service.rs:914` likewise. These are **deliberate, named changes**, not incidental fallout."

**On temperkb.io this cell is empty** — all six principals are `is_active = true` (§15, re-verified 2026-07-20). The enterprise instance is unverified, which is why the rule is specified rather than assumed harmless.

**The two passes the single rule cannot express:**

1. **Pending requests.** The old predicate cannot see `status = 'pending'`, so in-flight requests would backfill to `Denied` and silently lose their request-ness — `Requested` would be unreachable by the backfill. Prod has **zero** join requests, so this is correctness-only there; the enterprise instance is unverified.
2. **A synthetic genesis log entry.** §5 promises `Reactivate` "restores rather than guesses", but the log begins at migration time — so every backfilled `Deactivated` row has no prior state to restore, "which is exactly the case §5 says cannot happen." The backfill must write a genesis entry recording the pre-deactivation standing (**rule 2 evaluated *ignoring* rule 1**), or `Reactivate` is undefined for every pre-existing deactivated principal.

   §11 flags why this matters more than it looks: *"the evidence is actively destroyed: `sync_system_membership` **deletes** auto-join memberships whenever the predicate reads false, so once the predicate is repointed, a `Deactivated` principal's gating-team membership — the very thing their access was derived from — is gone and does not come back."*

**A third pass this plan adds: governance.** §11 does not mention backfilling `kb_principal_governance`, because governance came into scope at D10 after §11 was written. Without it, **repointing `is_system_admin` de-admins every existing admin** — the instance would have zero admins and, under D11, no way to make one. Existing admins are gating-team owners under the *old* definition, so the pass is `INSERT … SELECT` over `kb_team_members` where `role = 'owner'` on the gating team. **This is the single most important pass in the migration**; getting it wrong locks the operator out of their own instance.

- [ ] **Step 1: Write the migration**

Create `migrations/20260720000050_backfill_standing.sql`:

```sql
-- Backfill standing, governance, and a genesis log (spec 2026-07-20 §11, D8).
--
-- BACKFILL BY EVALUATING THE OLD PREDICATE, NOT BY READING THE TIER (D8). A tier-based backfill
-- would silently lock out anyone whose access comes entirely from gating-team membership with
-- system_access = 'none' -- confirmed on temperkb.io as exactly the `anonymous` row, at exactly
-- the cardinality §11 predicted.
--
-- THE OLD PREDICATE IS INLINED HERE, NOT CALLED. 20260720000040 already replaced
-- has_system_access's body with the standing read, and migrations apply in filename order -- so
-- calling it here would read the new body and backfill every principal to `denied`. The logic
-- below is the pre-cutover body, transcribed from 20260624000002_canonical_functions.sql:1388.
--
-- RULES ARE ORDERED; FIRST MATCH WINS. Rules 1 and 2 both match a deactivated principal whose old
-- predicate is true, and the ordering IS the decision: Deactivated wins, because D6 folds
-- is_active in and a principal who is disabled is disabled. The cost is stated honestly in §11 --
-- the predicate flips true->false for those principals, deliberately -- and auth-observable
-- behaviour is unaffected because gate_resolved_profile (auth/mod.rs:246) rejects !is_active at
-- Level 1 and the type-state makes reaching Level 2 without Level 1 impossible.

-- The old predicate's verdict, materialized before anything is written.
CREATE TEMP TABLE _old_verdict ON COMMIT DROP AS
WITH settings AS (
    SELECT access_mode, gating_team_slug FROM kb_system_settings LIMIT 1
)
SELECT p.id AS profile_id,
       p.is_active,
       (SELECT CASE
            WHEN s.access_mode = 'open' THEN true
            WHEN s.access_mode = 'invite_only' THEN EXISTS (
                SELECT 1 FROM kb_team_members tm
                  JOIN kb_teams t ON t.id = tm.team_id
                 WHERE tm.profile_id = p.id AND t.slug = s.gating_team_slug)
            ELSE false
        END FROM settings s) AS old_access
  FROM kb_profiles p
 -- RULE 0 (D7): connection profiles get NO ROW AT ALL. Not optional -- under access_mode='open'
 -- the old predicate is true for EVERY profile, so a literal per-profile backfill would mint
 -- connection profiles `approved` rows, contradicting D7 and dissolving the structural safety D7
 -- claims. There is no discriminator column on kb_profiles; kind is FK-inferable only.
 WHERE NOT EXISTS (SELECT 1 FROM kb_connections c WHERE c.profile_id = p.id);

-- Rules 1-3. Rule 3 is written `IS TRUE ... ELSE denied` rather than `false -> denied` so that
-- NULL is handled BY DECISION rather than by omission: the old predicate returns NULL when
-- kb_system_settings is empty, and a rule with only true/false arms would leave that case to
-- whatever this migration happened to do.
INSERT INTO kb_principal_standing (profile_id, state)
SELECT profile_id,
       CASE
           WHEN is_active = false      THEN 'deactivated'   -- rule 1
           WHEN old_access IS TRUE     THEN 'approved'      -- rule 2
           ELSE                             'denied'        -- rule 3, including NULL
       END
  FROM _old_verdict
ON CONFLICT (profile_id) DO NOTHING;

-- ---------------------------------------------------------------------------------------------
-- PASS 2 -- pending requests (§11).
--
-- The old predicate cannot see status = 'pending', so in-flight requests would backfill to
-- `denied` and silently lose their request-ness; `requested` would be unreachable by the backfill
-- as specified. temperkb.io has ZERO join requests so this is correctness-only there, but the
-- enterprise instance is unverified.
--
-- Only promotes rows currently `denied`: a pending request from an already-approved principal is
-- not evidence to downgrade them, and a deactivated principal stays deactivated (rule 1 wins).
-- ---------------------------------------------------------------------------------------------
UPDATE kb_principal_standing s
   SET state = 'requested'
  FROM kb_join_requests jr
 WHERE jr.requesting_profile_id = s.profile_id
   AND jr.status = 'pending'
   AND s.state = 'denied';

-- ---------------------------------------------------------------------------------------------
-- PASS 3 -- GOVERNANCE. The most important pass in this file.
--
-- NOT IN §11, because governance came into scope at D10 after §11 was written. Without it,
-- 20260720000040's repoint of is_system_admin DE-ADMINS EVERY EXISTING ADMIN -- and under D11 no
-- door grants access, so the instance would have zero admins and no way to make one. The operator
-- would be locked out of their own instance by a migration.
--
-- Existing admins are gating-team OWNERS under the old definition (the pre-cutover is_system_admin
-- body, 20260624000002_canonical_functions.sql:1409). granted_by is NULL: there is no actor to
-- name for a schema change, and inventing one would put a fabricated attribution on the ledger.
--
-- The `admin implies approved` invariant is asserted at the end of this file rather than assumed.
-- ---------------------------------------------------------------------------------------------
INSERT INTO kb_principal_governance (profile_id, granted_by)
SELECT tm.profile_id, NULL
  FROM kb_team_members tm
  JOIN kb_teams t ON t.id = tm.team_id
  JOIN kb_system_settings st ON st.gating_team_slug = t.slug
 WHERE tm.role = 'owner'
ON CONFLICT (profile_id) DO NOTHING;

-- ---------------------------------------------------------------------------------------------
-- PASS 4 -- the synthetic genesis log entry (§11).
--
-- §5 promises Reactivate "restores rather than guesses", but the log begins at migration time, so
-- every backfilled `deactivated` row would have nothing to restore -- exactly the case §5 says
-- cannot happen. This writes the pre-deactivation standing, computed as RULE 2 EVALUATED IGNORING
-- RULE 1 (i.e. what the principal's standing would have been had they not been deactivated).
--
-- This matters more than it looks because the evidence is actively destroyed: sync_system_membership
-- DELETEs auto-join memberships whenever the predicate reads false, so once the predicate is
-- repointed a deactivated principal's gating-team membership -- the very thing their access was
-- derived from -- is gone and does not come back. This pass is the last moment that information
-- exists.
--
-- actor_profile_id is NULL for every backfilled row: a migration is not an actor.
-- ---------------------------------------------------------------------------------------------
INSERT INTO kb_principal_standing_events
    (profile_id, act, prior_state, resulting_state, actor_profile_id, reason)
SELECT v.profile_id,
       'provision',
       -- For a deactivated principal this is what Reactivate will restore.
       CASE WHEN v.is_active = false
            THEN CASE WHEN v.old_access IS TRUE THEN 'approved' ELSE 'denied' END
            ELSE NULL
       END,
       s.state,
       NULL,
       'backfilled at cutover (migration 20260720000050); no actor'
  FROM _old_verdict v
  JOIN kb_principal_standing s ON s.profile_id = v.profile_id;

-- A deactivated principal needs a `deactivate` entry too, or principal_prior_standing -- which
-- reads the most recent act='deactivate' row -- finds nothing and Reactivate refuses.
INSERT INTO kb_principal_standing_events
    (profile_id, act, prior_state, resulting_state, actor_profile_id, reason)
SELECT v.profile_id,
       'deactivate',
       CASE WHEN v.old_access IS TRUE THEN 'approved' ELSE 'denied' END,
       'deactivated',
       NULL,
       'backfilled at cutover; prior standing reconstructed from the pre-cutover predicate'
  FROM _old_verdict v
 WHERE v.is_active = false;

-- ---------------------------------------------------------------------------------------------
-- ASSERTIONS. A backfill that silently did nothing is worse than one that fails loudly.
-- ---------------------------------------------------------------------------------------------
DO $$
DECLARE
    v_profiles      bigint;
    v_connections   bigint;
    v_standing      bigint;
    v_bad_admin     bigint;
BEGIN
    SELECT count(*) INTO v_profiles FROM kb_profiles;
    SELECT count(DISTINCT profile_id) INTO v_connections FROM kb_connections;
    SELECT count(*) INTO v_standing FROM kb_principal_standing;

    IF v_standing <> v_profiles - v_connections THEN
        RAISE EXCEPTION
            'backfill covered % of % non-connection profiles; rule 0 or the insert is wrong',
            v_standing, v_profiles - v_connections;
    END IF;

    -- D7, asserted rather than assumed: connection profiles must have NO row.
    IF EXISTS (SELECT 1 FROM kb_principal_standing s
                JOIN kb_connections c ON c.profile_id = s.profile_id) THEN
        RAISE EXCEPTION 'a connection profile received a standing row -- D7 is dissolved';
    END IF;

    -- The §9 invariant: admin implies approved. If this fires, the governance pass admitted
    -- someone whose standing does not permit them to use the instance they would govern.
    SELECT count(*) INTO v_bad_admin
      FROM kb_principal_governance g
      LEFT JOIN kb_principal_standing s ON s.profile_id = g.profile_id
     WHERE s.state IS DISTINCT FROM 'approved';
    IF v_bad_admin > 0 THEN
        RAISE EXCEPTION
            '% governance rows whose standing is not approved -- "admin implies Approved" (§9) is violated',
            v_bad_admin;
    END IF;

    RAISE NOTICE 'principal standing backfilled: % rows (% profiles, % connection profiles excluded)',
        v_standing, v_profiles, v_connections;
END $$;
```

- [ ] **Step 2: Apply and inspect on local dev**

Run: `cargo make docker-up` (if not running), then apply migrations via any `#[sqlx::test]` run or `sqlx migrate run`.

Run:
```bash
export PGPASSWORD=temper
psql -h localhost -p 5437 -U temper -d temper_development -At \
  -c "SELECT state||' = '||count(*) FROM kb_principal_standing GROUP BY state ORDER BY 1" \
  -c "SELECT 'governance = '||count(*) FROM kb_principal_governance"
```
Expected on a fresh local dev: one `approved` row (the `system` bootseed profile) and one governance row. **If governance is 0, stop** — the operator-lockout path is live.

- [ ] **Step 3: Commit**

```bash
cargo make check
git add migrations/20260720000050_backfill_standing.sql
git commit -m "feat(standing): backfill — 4 ordered rules, pending pass, genesis log, governance

The old predicate is INLINED, not called: 20260720000040 already replaced its
body and migrations run in filename order, so calling it would backfill
everything to denied.

Pass 3 (governance) is not in §11 — governance came into scope at D10 after §11
was written — and without it the repoint de-admins the instance with no door
left to make a new admin."
```

---

### Task 9: The differential backfill test

**Files:**
- Create: `crates/temper-services/tests/standing_backfill_test.rs`

**GD-3 tag: CONFORM** to spec §12's revised test specification.

**Carried verbatim from spec §12** — note this replaces an earlier, unsatisfiable version:
> "**The backfill gets a differential test — but not the one originally specified.** `old(p) == new(p) ∀p` is **unsatisfiable** on any population containing a deactivated profile, by the deliberate flip in §11. The test is:
> - `old(p) == new(p)` for every `p` **where `is_active`** — the preservation claim, scoped to where it is true;
> - a **separate** assertion that deactivated profiles flip `true → false`, so the intended change is pinned rather than merely tolerated;
> - connection profiles get **no standing row** (D7 / rule 0);
> - profiles with a pending request land in `Requested`, not `Denied`."

> "**`system_access` is not a dimension of the predicate** — it appears nowhere in `has_system_access`'s body. Fanning the population across all three tiers triples its size and tests nothing about admission. Keep one representative per tier only to exercise `trg_sync_system_membership`, which *does* read it."

**Configurations to run the whole population against**, verbatim from §12: `open`/`gating set`, `invite_only`/`gating set`, `invite_only`/`gating NULL`, `open`/`gating NULL`, and **`kb_system_settings` empty** — "the NULL arm — this one fails against today's function, which is the point."

**Per Pete's standing preference: differential, not hand-written expectations.** The test computes `old(p)` from the pre-cutover logic and compares; it does not hard-code which profile should end up where.

**A structural problem this test must solve, and how.** `#[sqlx::test]` applies *all* migrations including the backfill, so by the time the test body runs the cutover has already happened and the old predicate is gone. The test therefore (a) defines the old predicate itself as a local SQL expression — the transcription is the same one the migration uses — and (b) **re-runs the backfill logic** against a purpose-built population rather than relying on the migration's own pass. Extract the backfill body into a callable SQL function in Task 8 if you prefer; the simpler route, taken here, is to inline the same statements in a test helper and assert on the result.

- [ ] **Step 1: Write the test**

Create `crates/temper-services/tests/standing_backfill_test.rs`:

```rust
//! Differential backfill test (spec §12).
//!
//! The claim under test is NOT `old(p) == new(p) ∀p` — that is unsatisfiable on any population
//! containing a deactivated profile, by §11's deliberate flip. It is:
//!   * `old(p) == new(p)` ∀p WHERE is_active   — preservation, scoped to where it is true
//!   * deactivated profiles flip true → false  — the intended change, pinned not tolerated
//!   * connection profiles get no row          — D7 / rule 0
//!   * pending requests land in `requested`    — the pass the single rule cannot express

use sqlx::PgPool;

/// The PRE-CUTOVER predicate, transcribed from 20260624000002_canonical_functions.sql:1388.
/// Inlined because 20260720000040 has already replaced the real function's body by the time any
/// `#[sqlx::test]` body runs.
const OLD_PREDICATE: &str = r#"
    WITH settings AS (SELECT access_mode, gating_team_slug FROM kb_system_settings LIMIT 1)
    SELECT CASE
        WHEN settings.access_mode = 'open' THEN true
        WHEN settings.access_mode = 'invite_only' THEN EXISTS (
            SELECT 1 FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id
             WHERE tm.profile_id = $1 AND t.slug = settings.gating_team_slug)
        ELSE false
    END FROM settings
"#;

struct Population {
    /// (handle, id, is_active, is_connection, has_pending_request)
    rows: Vec<(String, uuid::Uuid, bool, bool, bool)>,
}

/// One representative per tier ONLY — `system_access` is not a dimension of the predicate (it
/// appears nowhere in has_system_access's body), so fanning across all three tiers would triple
/// the population and test nothing about admission. The tiers are here purely to exercise
/// trg_sync_system_membership, which does read the column.
async fn build_population(pool: &PgPool) -> Population {
    let mut rows = Vec::new();

    let team: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) VALUES ('temper-system','System','watcher')
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    ).fetch_one(pool).await.unwrap();

    for (handle, tier, active, member, connection, pending) in [
        ("p-none-out",      "none",     true,  false, false, false),
        ("p-none-in",       "none",     true,  true,  false, false), // the `anonymous` shape (D8)
        ("p-approved-in",   "approved", true,  true,  false, false),
        ("p-admin-in",      "admin",    true,  true,  false, false),
        ("p-inactive-in",   "approved", false, true,  false, false), // the deliberate flip
        ("p-inactive-out",  "none",     false, false, false, false),
        ("p-pending",       "none",     true,  false, false, true),  // pass 2
        ("p-connection",    "none",     true,  false, true,  false), // rule 0
    ] {
        let id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name, system_access, is_active)
             VALUES ($1,$1,$2::system_access,$3) RETURNING id",
        ).bind(handle).bind(tier).bind(active).fetch_one(pool).await.unwrap();

        if member {
            sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'watcher')
                         ON CONFLICT DO NOTHING")
                .bind(team).bind(id).execute(pool).await.unwrap();
        }
        if connection {
            sqlx::query(
                "INSERT INTO kb_connections
                    (provider, slug, name, registered_by_profile_id, profile_id,
                     emitter_entity_id, home_context_id)
                 SELECT 'test', $1, $1, $2, $2, e.id, c.id
                   FROM kb_entities e, kb_contexts c
                  WHERE e.profile_id = $2 LIMIT 1",
            ).bind(handle).bind(id).execute(pool).await.ok();
        }
        if pending {
            sqlx::query(
                "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, source)
                 VALUES (uuid_generate_v7(), $1, $2, 'pending', 'cli')",
            ).bind(team).bind(id).execute(pool).await.unwrap();
        }

        rows.push((handle.to_string(), id, active, connection, pending));
    }

    Population { rows }
}

async fn old_access(pool: &PgPool, id: uuid::Uuid) -> Option<bool> {
    sqlx::query_scalar(OLD_PREDICATE).bind(id).fetch_one(pool).await.unwrap()
}

async fn new_access(pool: &PgPool, id: uuid::Uuid) -> Option<bool> {
    sqlx::query_scalar("SELECT has_system_access($1)").bind(id).fetch_one(pool).await.unwrap()
}

/// Re-run the backfill's rules against the freshly built population. Mirrors
/// migrations/20260720000050 exactly; if the two drift, this test is testing a fiction.
async fn run_backfill(pool: &PgPool) {
    sqlx::query(r#"
        INSERT INTO kb_principal_standing (profile_id, state)
        SELECT p.id,
               CASE WHEN p.is_active = false THEN 'deactivated'
                    WHEN (WITH settings AS (SELECT access_mode, gating_team_slug FROM kb_system_settings LIMIT 1)
                          SELECT CASE
                              WHEN s.access_mode = 'open' THEN true
                              WHEN s.access_mode = 'invite_only' THEN EXISTS (
                                  SELECT 1 FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id
                                   WHERE tm.profile_id = p.id AND t.slug = s.gating_team_slug)
                              ELSE false END FROM settings s) IS TRUE THEN 'approved'
                    ELSE 'denied' END
          FROM kb_profiles p
         WHERE NOT EXISTS (SELECT 1 FROM kb_connections c WHERE c.profile_id = p.id)
        ON CONFLICT (profile_id) DO NOTHING
    "#).execute(pool).await.unwrap();

    sqlx::query(
        "UPDATE kb_principal_standing s SET state = 'requested'
           FROM kb_join_requests jr
          WHERE jr.requesting_profile_id = s.profile_id AND jr.status = 'pending' AND s.state = 'denied'",
    ).execute(pool).await.unwrap();
}

async fn configure(pool: &PgPool, mode: Option<&str>, gating: Option<&str>) {
    match mode {
        None => { sqlx::query("DELETE FROM kb_system_settings").execute(pool).await.unwrap(); }
        Some(m) => {
            sqlx::query(
                "INSERT INTO kb_system_settings (id, access_mode, gating_team_slug) VALUES (1,$1,$2)
                 ON CONFLICT (id) DO UPDATE SET access_mode=EXCLUDED.access_mode,
                                                gating_team_slug=EXCLUDED.gating_team_slug",
            ).bind(m).bind(gating).execute(pool).await.unwrap();
        }
    }
}

async fn differential(pool: &PgPool, mode: Option<&str>, gating: Option<&str>, label: &str) {
    configure(pool, mode, gating).await;
    let pop = build_population(pool).await;

    // Capture the old verdict BEFORE writing any standing.
    let mut before = Vec::new();
    for (h, id, active, conn, pending) in &pop.rows {
        before.push((h.clone(), *id, *active, *conn, *pending, old_access(pool, *id).await));
    }

    sqlx::query("DELETE FROM kb_principal_standing").execute(pool).await.unwrap();
    run_backfill(pool).await;

    for (h, id, active, conn, pending, old) in before {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
                .bind(id).fetch_optional(pool).await.unwrap().flatten();

        if conn {
            // D7 / rule 0. Under `open` the old predicate is true for EVERY profile, so a literal
            // per-profile backfill would mint this row and dissolve D7's structural safety.
            assert!(state.is_none(), "[{label}] {h}: a connection profile must get NO standing row");
            continue;
        }

        if !active {
            // The DELIBERATE flip. Pinned, not merely tolerated.
            assert_eq!(state.as_deref(), Some("deactivated"), "[{label}] {h}");
            assert_eq!(
                new_access(pool, id).await, Some(false),
                "[{label}] {h}: a deactivated principal's predicate flips true→false by design (§11)"
            );
            continue;
        }

        if pending {
            assert_eq!(
                state.as_deref(), Some("requested"),
                "[{label}] {h}: a pending request must land in `requested`, not `denied` — the old \
                 predicate cannot see status='pending' and the second pass exists for exactly this"
            );
            continue;
        }

        // THE PRESERVATION CLAIM, scoped to where it is true.
        assert_eq!(
            new_access(pool, id).await, Some(old.unwrap_or(false)),
            "[{label}] {h}: old(p) must equal new(p) for every active, non-connection principal"
        );
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_with_gating_set(pool: PgPool) {
    differential(&pool, Some("open"), Some("temper-system"), "open/gating").await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn invite_only_with_gating_set(pool: PgPool) {
    differential(&pool, Some("invite_only"), Some("temper-system"), "invite_only/gating").await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn invite_only_with_gating_null(pool: PgPool) {
    differential(&pool, Some("invite_only"), None, "invite_only/NULL").await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_with_gating_null(pool: PgPool) {
    differential(&pool, Some("open"), None, "open/NULL").await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn settings_empty_is_the_null_arm(pool: PgPool) {
    // §12: "this one fails against today's function, which is the point." The old predicate
    // returns NULL with no settings row; rule 3's `IS TRUE ... ELSE denied` handles it BY DECISION
    // rather than by omission, and the repointed predicate is EXISTS-total so it returns false.
    differential(&pool, None, None, "settings-empty").await;

    let any: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT profile_id FROM kb_principal_standing LIMIT 1")
            .fetch_optional(&pool).await.unwrap().flatten();
    if let Some(id) = any {
        assert_eq!(
            new_access(&pool, id).await, Some(false),
            "with no settings row the predicate must be FALSE, never NULL"
        );
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_anonymous_shape_survives_the_backfill(pool: PgPool) {
    // D8, vindicated empirically on prod at exactly the predicted cardinality of one: `anonymous`
    // has tier `none` and access purely via gating-team membership. A tier-based backfill would
    // have locked it out. `p-none-in` is that shape.
    configure(&pool, Some("invite_only"), Some("temper-system")).await;
    build_population(&pool).await;
    sqlx::query("DELETE FROM kb_principal_standing").execute(&pool).await.unwrap();
    run_backfill(&pool).await;

    let state: String = sqlx::query_scalar(
        "SELECT s.state FROM kb_principal_standing s JOIN kb_profiles p ON p.id = s.profile_id
          WHERE p.handle = 'p-none-in'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(
        state, "approved",
        "a tier-`none` gating-team member must backfill to approved — this is the D8 case"
    );
}
```

- [ ] **Step 2: Run the tests**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test standing_backfill_test`
Expected: PASS — 6 tests.

If `settings_empty_is_the_null_arm` fails on the *old* predicate returning `NULL` where the test expects a comparison, that is the §12 note being literal — the assertion is about the **new** predicate being total. Check you are asserting on `has_system_access`, not on `OLD_PREDICATE`.

- [ ] **Step 3: Commit**

```bash
cargo make prepare-services && cargo make check
git add crates/temper-services/tests/standing_backfill_test.rs crates/temper-services/.sqlx
git commit -m "test(standing): differential backfill across five settings configurations

Scoped to the claim that is true — old(p)==new(p) for ACTIVE principals — with
the deactivated flip pinned separately rather than tolerated. Connection
profiles assert no row (D7); pending requests assert `requested`.

One representative per tier only: system_access is not a dimension of the
predicate, so fanning across tiers would triple the population and test nothing."
```

---

## Beat D — the seam

`temper-principal` is pure and knows no ids; the database knows nothing about legality. This beat is the one place they meet.

---

### Task 10: `standing_service` — gather evidence, decide, commit

**Files:**
- Create: `crates/temper-services/src/services/standing_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs` (register the module)
- Modify: `crates/temper-services/Cargo.toml` (add `temper-principal`)
- Test: inline `#[cfg(all(test, feature = "test-db"))] mod tests`

**Interfaces:**
- Consumes: `temper_principal::{Act, ActorAuthority, Provisioner, Refusal, Standing, transition}`; the SQL functions from Task 6.
- Produces:
  - `pub async fn load(pool, ProfileId) -> ApiResult<Option<Standing>>`
  - `pub async fn apply(pool, ApplyStandingParams) -> Result<Standing, ApiError>`
  - `pub struct ApplyStandingParams { pub subject: ProfileId, pub act: Act, pub actor: Option<ProfileId>, pub authority: ActorAuthority }`
  - `pub async fn admit(pool, ProfileId) -> Result<AdmittedPrincipal, Refusal>`

**GD-3 tag: CONFORM** to the repo's service-layer shape (`temper-services/src/services/`, params struct for >5 domain params, auth before writes), **EXTEND** for the new module (spec §4).

**Invariant, carried verbatim from spec §4:**
> "The claims→profile seam stays `pub(crate)` in `temper-services`… `temper-principal` never resolves a credential. It judges assembled evidence. That is what makes it safe to share."

**The order is not negotiable: decide, then commit.** `apply` must call `temper_principal::transition` *before* it calls `principal_standing_apply`. Auth-before-writes is a repo rule, and here it is also what keeps the SQL committer free of a second transition table.

**`Reactivate` is the one act that needs a read before the decision** — it is the only data-dependent target in the machine (spec §6). `apply` loads `principal_prior_standing` and puts it in the `Act::Reactivate { prior }` value before deciding. Do not let any other act acquire this shape without argument.

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-services/src/services/standing_service.rs`:

```rust
#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn profile(pool: &PgPool, handle: &str) -> ProfileId {
        let id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id",
        ).bind(handle).fetch_one(pool).await.unwrap();
        ProfileId::from(id)
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_illegal_transition_is_refused_and_writes_nothing(pool: PgPool) {
        let p = profile(&pool, "illegal").await;
        let admin = profile(&pool, "illegal-admin").await;
        apply(&pool, ApplyStandingParams {
            subject: p, act: Act::Provision { path: Provisioner::OauthFirstLogin },
            actor: None, authority: ActorAuthority::Credential,
        }).await.unwrap();

        // Revoke from Denied — you cannot revoke what was never granted (spec §6).
        let err = apply(&pool, ApplyStandingParams {
            subject: p, act: Act::Revoke { reason: "no".into() },
            actor: Some(admin), authority: ActorAuthority::Admin,
        }).await.expect_err("must refuse");

        assert!(format!("{err}").contains("not legal"), "the refusal must carry a reason: {err}");

        let state: String = sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
            .bind(*p).fetch_one(&pool).await.unwrap();
        assert_eq!(state, "denied", "a refused act must write nothing");

        let logs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_principal_standing_events WHERE profile_id=$1 AND act='revoke'")
            .bind(*p).fetch_one(&pool).await.unwrap();
        assert_eq!(logs, 0, "a refused act must not appear in the log");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn reactivate_restores_the_prior_state_through_the_seam(pool: PgPool) {
        let p = profile(&pool, "react").await;
        let admin = profile(&pool, "react-admin").await;
        for (act, auth) in [
            (Act::Provision { path: Provisioner::OauthFirstLogin }, ActorAuthority::Credential),
            (Act::Approve, ActorAuthority::Admin),
            (Act::Deactivate, ActorAuthority::Admin),
        ] {
            apply(&pool, ApplyStandingParams { subject: p, act, actor: Some(admin), authority: auth })
                .await.unwrap();
        }

        let restored = apply(&pool, ApplyStandingParams {
            subject: p, act: Act::Reactivate { prior: None }, // the seam fills this in
            actor: Some(admin), authority: ActorAuthority::Admin,
        }).await.unwrap();

        assert_eq!(restored, Standing::Approved, "Reactivate restores rather than guesses (§5)");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn admit_denies_a_principal_with_no_standing_row(pool: PgPool) {
        let p = profile(&pool, "nostanding").await;
        assert_eq!(admit(&pool, p).await, Err(Refusal::NoStanding));
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db standing_service`
Expected: FAIL to compile — the module does not exist.

- [ ] **Step 3: Write the implementation**

Add to `crates/temper-services/Cargo.toml` under `[dependencies]`:

```toml
temper-principal = { path = "../temper-principal" }
```

Write `crates/temper-services/src/services/standing_service.rs`:

```rust
//! The seam between the pure admission machines and the database (spec 2026-07-20 §4).
//!
//! ```text
//! services gathers evidence ─► temper-principal decides ─► ONE SQL function commits
//!                                                          (row + log + event, one txn)
//! ```
//!
//! `temper-principal` never resolves a credential and holds no identifiers — it judges assembled
//! evidence, which is what makes it safe to share across surfaces. Every id in this file stays on
//! this side of the boundary.

use crate::error::{ApiError, ApiResult};
use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_principal::{
    admit as pure_admit, transition, Act, ActorAuthority, AdmittedPrincipal, Provisioner, Refusal,
    Standing,
};

/// Parameters for one standing transition. A params struct because the domain arguments would
/// otherwise exceed the repo's threshold, and because `authority` and `actor` must travel together.
pub struct ApplyStandingParams {
    /// The principal whose standing changes.
    pub subject: ProfileId,
    pub act: Act,
    /// The acting principal. `None` for credential-authority acts and the boot-seed.
    pub actor: Option<ProfileId>,
    pub authority: ActorAuthority,
}

/// Load a principal's current standing. `Ok(None)` means no row — which denies (spec §7).
pub async fn load(pool: &PgPool, profile_id: ProfileId) -> ApiResult<Option<Standing>> {
    let raw: Option<String> =
        sqlx::query_scalar!("SELECT state FROM kb_principal_standing WHERE profile_id = $1", *profile_id)
            .fetch_optional(pool)
            .await?;

    // A row whose value this binary does not recognize is NOT `None` — that would silently
    // downgrade "unknown state" to "no standing" and lose the distinction the refusal needs.
    match raw {
        None => Ok(None),
        Some(r) => Standing::parse(&r).map(Some).ok_or_else(|| {
            ApiError::Internal(format!("unrecognized standing {r:?} for profile {}", *profile_id))
        }),
    }
}

/// The per-request admission decision (Level 2).
///
/// Reads standing and nothing else (D15 obligation 1). A `Revoked` principal is refused whether or
/// not a review is pending; ANDing the marker in would restore the conjunction-across-provisional-
/// facts shape D2 forbids, and it is the tempting change.
pub async fn admit(pool: &PgPool, profile_id: ProfileId) -> Result<AdmittedPrincipal, Refusal> {
    let raw: Option<String> =
        sqlx::query_scalar!("SELECT state FROM kb_principal_standing WHERE profile_id = $1", *profile_id)
            .fetch_optional(pool)
            .await
            .map_err(|_| Refusal::NoStanding)?;

    pure_admit(raw.as_deref())
}

/// Decide, then commit. **The order is not negotiable** — auth before writes, and it is also what
/// keeps the SQL committer free of a second transition table.
pub async fn apply(pool: &PgPool, params: ApplyStandingParams) -> ApiResult<Standing> {
    let current = load(pool, params.subject).await?;

    // `Reactivate` is THE ONLY data-dependent target in the machine (spec §6), so it is the only
    // act that needs a read before the decision. Treat a second such act as a design smell until
    // argued for.
    let act = match params.act {
        Act::Reactivate { prior: None } => {
            let prior: Option<String> =
                sqlx::query_scalar!("SELECT principal_prior_standing($1)", *params.subject)
                    .fetch_one(pool)
                    .await?;
            Act::Reactivate { prior: prior.as_deref().and_then(Standing::parse) }
        }
        other => other,
    };

    let resulting = transition(current, &act, params.authority)
        .map_err(|r| ApiError::Forbidden2(r.reason()))?;

    let act_name = act_name(&act);
    let reason = match &act {
        Act::Revoke { reason } => Some(reason.clone()),
        _ => None,
    };

    let committed: Option<String> = sqlx::query_scalar!(
        "SELECT principal_standing_apply($1,$2,$3,$4,$5)",
        *params.subject,
        act_name,
        resulting.as_str(),
        params.actor.map(|a| *a),
        reason,
    )
    .fetch_one(pool)
    .await?;

    // The committer echoes back what it wrote. A disagreement means the SQL grew an opinion.
    debug_assert_eq!(committed.as_deref(), Some(resulting.as_str()));
    Ok(resulting)
}

/// The database literal for an act. Exhaustive, no catchall — adding an act is a compile error.
fn act_name(act: &Act) -> &'static str {
    match act {
        Act::Provision { .. } => "provision",
        Act::Request => "request",
        Act::Withdraw => "withdraw",
        Act::Approve => "approve",
        Act::Reject => "reject",
        Act::Revoke { .. } => "revoke",
        Act::Deactivate => "deactivate",
        Act::Reactivate { .. } => "reactivate",
        Act::RequestReview => "request_review",
    }
}

/// Convenience for the four mint doors (D11): every one births `Denied`, except genesis.
pub async fn provision(pool: &PgPool, subject: ProfileId, path: Provisioner) -> ApiResult<Standing> {
    apply(pool, ApplyStandingParams {
        subject,
        act: Act::Provision { path },
        actor: None,
        authority: ActorAuthority::Credential,
    })
    .await
}
```

> **`ApiError::Forbidden2` is a placeholder name and must not ship.** Beat H replaces the whole refusal path with a typed `Refusal` carried on `ApiError::SystemAccessRequired`. Until then, use the existing `ApiError::Forbidden` and log the reason — do **not** invent a new variant. Reconcile this in Task 17.

Register the module in `crates/temper-services/src/services/mod.rs` next to the other `pub mod` lines.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db standing_service`
Expected: PASS — 3 tests.

- [ ] **Step 5: Commit**

```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make check
git add crates/temper-services/ .sqlx
git commit -m "feat(standing): the seam — gather evidence, decide, commit

Decide THEN commit, always. Reactivate is the only act that reads before
deciding, because it is the only data-dependent target in the machine; a second
one should be argued for, not added."
```

---

### Task 11: Route `require_system_access` through the machine, and the four doors through `provision`

**Files:**
- Modify: `crates/temper-services/src/auth/mod.rs` (`require_system_access`, `:267-285`)
- Modify: `crates/temper-services/src/services/profile_service.rs` (`create_new_profile_and_link`, `:389-430`)
- Modify: `crates/temper-services/src/services/machine_registration_service.rs` (`provision`)
- Modify: `crates/temper-substrate/src/scenario/bootseed.rs` (`:27-36`)
- Modify: `crates/temper-substrate/src/scenario/loader.rs` (`:45-62`), `crates/temper-substrate/src/scenario/access/loader.rs` (`:135-153`)
- Test: `crates/temper-services/src/auth/mod.rs` inline tests + `tests/e2e/tests/access_gate_test.rs`

**GD-3 tag: AMEND** — all five sites exist; spec §6 and D11 authorize changing what they write.

**Invariants, carried verbatim:**
> D11: "**Every provision path births `Denied`.** No door grants access. Approval is always a separate, admin-authored act."

> §6: "**`Provision` fires only on profile mint, never on a returning principal.** An existing auth link returns at step 1 of `resolve_human_from_claims` and never reaches the mint; a returning principal's standing is **loaded, not set**."

> §12: "**The mint split gets one test per path, never one for the pair** — the two doors share a mint function. Under D11 both must birth `Denied`, so this test now guards *uniformity* rather than *divergence*, but the reason for testing each path separately is unchanged."

**The shared-mint fact that makes this safe.** SAML and OAuth reach the same function: `create_new_profile_and_link` (`profile_service.rs:389`), from both `resolve_federated_human` (`auth/mod.rs:194`) and `authenticate`. Under the *old* design the two doors had to diverge at that shared site, and "a constant at that site would have been permissive and silent, opening the instance to anyone who could sign in, with nothing to notice" (§6). **A uniform birth state removes the divergence entirely**, so the `provision` call goes in `create_new_profile_and_link` itself — one site, no per-door constant to get wrong.

`create_new_profile_and_link` currently writes no `system_access` at all (verified — `grep system_access profile_service.rs` returns zero hits; the tier comes from the column DEFAULT at `canonical_schema.sql:127`). Adding the standing mint here is purely additive.

- [ ] **Step 1: Write the failing tests**

Add to `crates/temper-services/src/auth/mod.rs`'s existing test module (`:287`):

```rust
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn oauth_first_login_is_born_denied(pool: PgPool) {
        // The community edition has no paywall. An OAuth signup being born Denied — requiring an
        // admin to enable it — IS the access-control mechanism, deliberately (spec §8). Do not
        // "fix" this because new users are locked out. That is the feature.
        let profile = authenticate(&pool, &claims("oauth|newcomer", "a@example.com")).await.unwrap();
        let standing = crate::services::standing_service::load(&pool, ProfileId::from(profile.profile.id))
            .await.unwrap();
        assert_eq!(standing, Some(temper_principal::Standing::Denied));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn saml_jit_is_also_born_denied(pool: PgPool) {
        // D13 — the "assertion IS the grant" rationale was WITHDRAWN by its own author. An IdP
        // asserting "our org says this person may use this" and the instance deciding "we agree,
        // and now they have access" are different claims by different parties, and it is across
        // exactly that boundary that interception and escalation happen. Team assignment already
        // respects this; system access was the odd one out.
        //
        // DO NOT RESTORE AUTO-APPROVAL HERE. If auto-provisioning is ever wanted it needs an
        // articulated JTBD spec first; there is none, and its absence is why the shortcut was a
        // mistake.
        let profile = resolve_federated_human(
            &pool, "test-provider", "saml|newcomer", "b@example.com", Some(true),
        ).await.unwrap();
        let standing = crate::services::standing_service::load(&pool, ProfileId::from(profile.id))
            .await.unwrap();
        assert_eq!(standing, Some(temper_principal::Standing::Denied));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_two_human_doors_mint_the_same_standing(pool: PgPool) {
        // ONE TEST PER PATH, NEVER ONE FOR THE PAIR (§12) — the two doors share
        // create_new_profile_and_link. This third test guards UNIFORMITY where it used to guard
        // divergence, but the reason for testing each path separately is unchanged.
        //
        // Distinct subjects AND distinct emails: same-email would reconcile the second door onto
        // the FIRST door's profile (resolve_human_from_claims steps 3-4), and this test would then
        // pass by resolving one profile twice — proving nothing about minting.
        let oauth = authenticate(&pool, &claims("oauth|d1", "d1@example.com")).await.unwrap().profile.id;
        let saml = resolve_federated_human(&pool, "test-provider", "saml|d2", "d2@example.com", Some(true))
            .await.unwrap().id;
        assert_ne!(oauth, saml, "guard: the two doors must have minted two profiles");

        for id in [oauth, saml] {
            assert_eq!(
                crate::services::standing_service::load(&pool, ProfileId::from(id)).await.unwrap(),
                Some(temper_principal::Standing::Denied),
                "both doors must birth Denied — no door grants access (D11)"
            );
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_on_a_returning_principal_does_not_touch_standing(pool: PgPool) {
        // F4, closed structurally. A revoked SAML principal re-asserting through the IdP must stay
        // Revoked — the earlier per-ASSERTION wording made Revoke defeatable on the SAML door.
        let profile = resolve_federated_human(
            &pool, "test-provider", "saml|returning", "r@example.com", Some(true),
        ).await.unwrap();
        let id = ProfileId::from(profile.id);
        let admin = /* seed an admin; see the module's existing seed_admin helper */ id;

        use temper_principal::{Act, ActorAuthority};
        use crate::services::standing_service::{apply, ApplyStandingParams};
        apply(&pool, ApplyStandingParams { subject: id, act: Act::Approve, actor: Some(admin), authority: ActorAuthority::Admin }).await.unwrap();
        apply(&pool, ApplyStandingParams { subject: id, act: Act::Revoke { reason: "test".into() }, actor: Some(admin), authority: ActorAuthority::Admin }).await.unwrap();

        // Re-assert through the IdP.
        resolve_federated_human(&pool, "test-provider", "saml|returning", "r@example.com", Some(true))
            .await.unwrap();

        assert_eq!(
            crate::services::standing_service::load(&pool, id).await.unwrap(),
            Some(temper_principal::Standing::Revoked),
            "a returning principal's standing is LOADED, never SET"
        );
    }
```

> The `admin` binding above is a sketch — use the module's existing admin-seeding helper if one is present, or create a profile and insert a `kb_principal_governance` row directly. Do not invent a helper that does not exist without checking first.

- [ ] **Step 2: Run to verify they fail**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --lib auth`
Expected: FAIL — standing is `None` for freshly minted profiles.

- [ ] **Step 3: Wire the four doors**

**(a) The shared human mint** — in `create_new_profile_and_link` (`profile_service.rs:389-430`), after the existing `INSERT INTO kb_profiles` and before the function returns:

```rust
    // D11 — every door births Denied. THIS IS THE SHARED SITE for both human doors (SAML via
    // resolve_federated_human, OAuth via authenticate), and putting the call here rather than at
    // each caller is what removes the per-door constant that could be got wrong. Under the old
    // design the doors had to DIVERGE here, and a constant at this site would have been permissive
    // and silent — every signup born approved, with nothing to notice.
    crate::services::standing_service::provision(
        pool,
        ProfileId::from(profile_id),
        temper_principal::Provisioner::OauthFirstLogin,
    )
    .await?;
```

> **On the `Provisioner` value at the shared site.** Both human doors pass through here, so a single call cannot distinguish them. That is *fine under D11* — both birth `Denied`, and the variant is recorded only for the ledger. If per-door attribution in the event payload is wanted, thread a `Provisioner` parameter down from `resolve_from_claims`; that is the abandoned branch's `MintedAccess` shape and it compiles cleanly, but it buys attribution only, not behaviour. Prefer the simple call unless the ledger distinction is asked for.

**(b) Machine registration** — in `machine_registration_service::provision`, inside the existing transaction, alongside the `kb_machine_clients` insert:

```rust
    // D11 — containment is RETIRED, not relocated: "a minter who cannot confer access is moot when
    // minting never confers access." enroll_in_gating_team's caller check remains as ordinary team
    // hygiene, but it no longer guards system access, because minting no longer confers any.
    standing_service::provision(pool, ProfileId::from(profile_id), Provisioner::MachineRegistration).await?;
```

**(c) The boot-seed (genesis)** — in `bootseed.rs`, after the `kb_profiles` upsert at `:31-36`:

```rust
    // THE ONE DELIBERATE EXCEPTION (D11, F6). On a fresh instance no admin exists, so nobody could
    // ever be approved and the instance would be permanently unusable. Accepted, not worked around:
    // bootstrapping temper already requires database write access, and the bootstrap SoP and
    // scripts foreground that reality.
    sqlx::query!("SELECT principal_standing_apply($1,'provision','approved',NULL,'boot-seed genesis')", profile)
        .execute(pool).await?;
    sqlx::query!("SELECT principal_governance_set($1,true,NULL,'boot-seed genesis admin')", profile)
        .execute(pool).await?;
```

**(d) and (e) The two scenario loaders** — `loader.rs:52-58` and `access/loader.rs:142-148` are byte-identical inserts. In each, after the profile insert, mint standing from the YAML tier so existing scenarios keep meaning what they meant:

```rust
        // Keep writing system_access (the column survives Phase 1 as a projection), and ALSO mint
        // the standing row that is now authoritative. A scenario that says a profile is `approved`
        // must still produce a profile that can act.
        let standing = match p.system_access {
            SystemAccess::None => "denied",
            SystemAccess::Approved | SystemAccess::Admin => "approved",
        };
        sqlx::query!("SELECT principal_standing_apply($1,'provision',$2,NULL,'scenario load')", id, standing)
            .execute(&mut *tx).await?;
        if matches!(p.system_access, SystemAccess::Admin) {
            sqlx::query!("SELECT principal_governance_set($1,true,NULL,'scenario load')", id)
                .execute(&mut *tx).await?;
        }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db`
Expected: PASS.

Run: `cargo make test-artifacts`
Expected: PASS — the scenario write-path tests exercise both loaders.

- [ ] **Step 5: Prove the whole-surface property (§12)**

Spec §12 replaces the retired containment tests with a stronger, surface-wide assertion:
> "**Containment is retired (D11), so its tests change shape.** There is no longer a minter-standing guard to test. What replaces it is the stronger assertion: **no provision path, under any actor, yields `Approved`** — one test per door, plus a test that a machine minted by an admin is still born `Denied`. That is a property of the whole surface rather than of one guard, and it fails loudly if a future door is added carelessly."

Add to `tests/e2e/tests/access_gate_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_provision_path_under_any_actor_yields_approved(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;

    // A machine minted BY AN ADMIN is still born Denied. Under the old design an admin minter was
    // exactly the case that DID confer access (enroll_in_gating_team's check passes for an admin),
    // so this is the cell where the retired guard used to matter most.
    let machine = common::provision_machine_as(&app, &app.token, "m-born-denied").await;

    for id in [profile_id(&admin_profile), machine] {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
                .bind(id).fetch_optional(&pool).await.unwrap().flatten();
        if id == machine {
            assert_eq!(state.as_deref(), Some("denied"), "no door grants access, not even an admin's (D11)");
        }
    }
}
```

> `common::provision_machine_as` may not exist under that name — check `tests/e2e/tests/common/mod.rs` and `machine_registration_authz_e2e.rs` for the established helper before writing a new one.

- [ ] **Step 6: Commit**

```bash
cargo make check && cargo make test-e2e
git add crates/ tests/
git commit -m "feat(standing): every door births Denied (D11)

One call at the SHARED human mint site rather than one per door — under the old
design the doors had to diverge there, and a constant at a shared site would
have been permissive and silent. The boot-seed is the deliberate exception.

D13: SAML is born Denied too. The 'assertion IS the grant' rationale was
withdrawn by its own author — identity assertion and access grant are different
claims by different parties."
```

---

## Beat E — the acts on the surfaces

---

### Task 12: `Request`, `Withdraw`, and `RequestReview`

**Files:**
- Modify: `crates/temper-services/src/services/access_service.rs` (`create_join_request` `:659`, `withdraw_request` `:762`)
- Create: `migrations/20260720000060_review_requests.sql` (the review marker + its guard)
- Modify: `crates/temper-api/src/handlers/access.rs`, `crates/temper-api/src/routes.rs`
- Modify: `crates/temper-cli/src/cli.rs`, `crates/temper-cli/src/commands/auth.rs`

**GD-3 tag: AMEND** (the two existing self-service acts) + **EXTEND** (the review marker, spec D15).

**Invariants, carried verbatim from D15:**
> 1. "**The marker is never an admission input.** It is an inbox signal only… ANDing the marker into the decision would restore precisely the conjunction-across-provisional-facts shape D2 forbids. This is stated as an obligation rather than left implied **because it is the tempting change** — a future reader will see a pending review and reach for it."
> 2. "**It needs its own duplicate guard.** For join requests, `Requested` standing *is* the duplicate guard (D12). A review does not move standing, so it does not inherit one — it needs its own (e.g. a unique partial index on `(profile_id) WHERE decided_at IS NULL`). This is what `idx_join_requests_one_pending` used to do, reappearing for a different reason."
> 3. "**Its open/decided lifecycle is not a regression on D5.** `kb_join_requests.status` was removed because it **duplicated standing**. A review's open/decided state duplicates nothing — standing stays `Revoked` throughout, whatever the outcome."

**`Request` keeps writing `kb_join_requests`, and that is D5 working as designed.** Standing carries the *state*; the request record carries the *payload* — message, `source`, and the terms-acceptance columns. D12 turns on that split: birthing a principal into `Requested` would produce a standing state whose paired record has no terms acceptance, "forcing either fabricated consent or an empty request row that lies about having been requested."

**Terms are unconfigured today** (`terms_version` and `terms_resource_uri` are empty on both prod and local — verified), so `Request` must handle "no terms configured" without blocking, and the acceptance columns stay nullable.

`kb_join_requests.status` **survives Phase 1** (additive-only) and Phase 1 keeps writing it, so nothing reading it breaks mid-flight. It stops being *authoritative* the moment standing exists; Phase 2 drops it, along with `idx_join_requests_one_pending`.

- [ ] **Step 1: Write the failing tests**

Add to `tests/e2e/tests/access_gate_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn request_moves_standing_and_records_consent_separately(pool: sqlx::PgPool) {
    // D5/D12: standing carries state; the request carries payload. The two must both move.
    let app = common::setup(pool.clone()).await;
    preflight(&app, &app.token).await;
    let token = common::generate_second_user_jwt();
    let me = preflight(&app, &token).await;
    let id = profile_id(&me);

    let resp = app.reqwest_client.post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "source": "cli", "message": "please" }))
        .send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let state: String = sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(state, "requested");

    let msg: Option<String> = sqlx::query_scalar(
        "SELECT message FROM kb_join_requests WHERE requesting_profile_id=$1").bind(id)
        .fetch_one(&pool).await.unwrap();
    assert_eq!(msg.as_deref(), Some("please"), "the payload stays on the request record (D5)");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_request_works_in_open_mode_too(pool: sqlx::PgPool) {
    // C6 / the access_mode retirement: under D11 every door births Denied REGARDLESS of mode, so
    // an `open` instance must not be a dead end. The old code returned 400 here.
    let app = common::setup(pool.clone()).await;
    preflight(&app, &app.token).await; // setup leaves the instance `open`
    let token = common::generate_second_user_jwt();
    preflight(&app, &token).await;

    let resp = app.reqwest_client.post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "source": "cli" })).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "open mode must not refuse a request");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_revoked_principal_cannot_re_request_but_may_ask_for_review(pool: sqlx::PgPool) {
    // D15 — the no-laundering property, structural rather than bookkept.
    let app = common::setup(pool.clone()).await;
    let admin = preflight(&app, &app.token).await;
    let token = common::generate_second_user_jwt();
    let me = preflight(&app, &token).await;
    let id = profile_id(&me);

    common::approve_then_revoke(&app, profile_id(&admin), id).await;

    let resp = app.reqwest_client.post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "source": "cli" })).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "Revoked → Request must be refused");

    let resp = app.reqwest_client.post(app.url("/api/access/reviews"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "message": "reconsider please" })).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let state: String = sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(state, "revoked", "RequestReview sets a marker and moves NOTHING (D15)");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_pending_review_does_not_change_admission(pool: sqlx::PgPool) {
    // D15 obligation 1, and §12 names this as "the D15 obligation that a future reader is most
    // likely to break": a Revoked principal with a pending review must be refused IDENTICALLY to
    // one without. Not merely "also refused" — the same refusal.
    let app = common::setup(pool.clone()).await;
    let admin = preflight(&app, &app.token).await;

    let with_token = common::generate_second_user_jwt();
    let without_token = common::generate_third_user_jwt();
    let with = profile_id(&preflight(&app, &with_token).await);
    let without = profile_id(&preflight(&app, &without_token).await);
    for id in [with, without] {
        common::approve_then_revoke(&app, profile_id(&admin), id).await;
    }

    app.reqwest_client.post(app.url("/api/access/reviews"))
        .header("Authorization", format!("Bearer {with_token}"))
        .json(&serde_json::json!({ "message": "please" })).send().await.unwrap();

    let a = app.reqwest_client.get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {with_token}")).send().await.unwrap();
    let b = app.reqwest_client.get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {without_token}")).send().await.unwrap();

    assert_eq!(a.status(), b.status(), "a pending review must not change the outcome");
    assert_eq!(a.status(), StatusCode::FORBIDDEN);
    assert_eq!(a.json::<serde_json::Value>().await.unwrap()["error"]["details"],
               b.json::<serde_json::Value>().await.unwrap()["error"]["details"],
               "and must not change the refusal either");
}
```

> `common::approve_then_revoke` and `common::generate_third_user_jwt` do not exist yet — add them to `tests/e2e/tests/common/mod.rs` in this task. `common::enable_invite_only` is the file's most load-bearing existing fixture; follow its shape.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo make test-e2e`
Expected: FAIL — no `/api/access/reviews` route; `open` mode still 400s.

- [ ] **Step 3: Write the review-marker migration**

Create `migrations/20260720000060_review_requests.sql`:

```sql
-- Review requests (spec 2026-07-20 D15).
--
-- A Revoked principal must be able to ask for reconsideration WITHOUT that request being able to
-- erase the revocation. The rejected alternative -- allow Revoked → Request → Requested and have
-- Withdraw return to the PRIOR state -- works, but preserves the audit signal by careful
-- bookkeeping. D15 makes it structural instead: there is no path out of Revoked except an admin
-- act, so there is nothing to launder.
--
-- It is also the more honest model. "Please let me in" and "please reconsider your decision" are
-- different speech acts with different admin context -- a reviewer needs the revocation reason,
-- which a plain Request has no slot for.
--
-- THIS TABLE IS AN INBOX SIGNAL, NEVER AN ADMISSION INPUT (D15 obligation 1). Admission reads
-- standing and nothing else; a Revoked principal is refused whether or not a review is pending.
-- ANDing this into the decision would restore precisely the conjunction-across-provisional-facts
-- shape D2 forbids -- and it is THE tempting change, which is why it is stated here and tested in
-- `a_pending_review_does_not_change_admission`.
--
-- ITS OPEN/DECIDED LIFECYCLE IS NOT A REGRESSION ON D5. We remove kb_join_requests.status in Phase
-- 2 because it DUPLICATED standing. A review's open/decided state duplicates nothing -- standing
-- stays `revoked` throughout, whatever the outcome. Different question, different answer.

CREATE TABLE kb_principal_review_requests (
    id           uuid PRIMARY KEY DEFAULT uuid_generate_v7(),
    profile_id   uuid NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    message      text,
    created      timestamptz NOT NULL DEFAULT now(),
    decided_at   timestamptz,
    decided_by   uuid REFERENCES kb_profiles(id),
    decision_note text
);

-- ITS OWN DUPLICATE GUARD (D15 obligation 2). For join requests, `requested` standing IS the
-- duplicate guard (D12) -- but a review does not move standing, so it inherits none. This is what
-- idx_join_requests_one_pending used to do, reappearing for a different reason. Note it is
-- per-PRINCIPAL, with no team dimension, which is more correct under D9 than the index it echoes.
CREATE UNIQUE INDEX idx_principal_review_one_open
    ON kb_principal_review_requests (profile_id)
    WHERE decided_at IS NULL;

COMMENT ON TABLE kb_principal_review_requests IS
  'A revoked principal asking for reconsideration (spec D15). AN INBOX SIGNAL ONLY -- never read '
  'by the admission decision. If you are here to AND this into has_system_access, re-read D2.';
```

- [ ] **Step 4: Rewrite the three service functions**

In `access_service.rs`:

**`create_join_request`** — delete the `AccessMode::Open` rejection block (`:671-678`) entirely, and fire `Request` on standing inside the existing transaction, *before* the insert:

```rust
    // Standing first: an illegal Request (from Revoked, from Approved) must refuse before any
    // request row exists. Auth before writes.
    standing_service::apply(pool, ApplyStandingParams {
        subject: params.profile_id,
        act: Act::Request,
        actor: Some(params.profile_id),
        authority: ActorAuthority::SelfPrincipal,
    }).await?;
```

The gating-team resolution (`:680-688`) stays — the request record keeps its `team_id`, which "stays where it is, describing the request rather than the standing" (D9).

**`withdraw_request`** — fire `Act::Withdraw` before the `UPDATE`, same shape.

**New `create_review_request`** — inserts into `kb_principal_review_requests` after firing `Act::RequestReview`, which validates the principal is `Revoked` and moves nothing.

- [ ] **Step 5: Add the route and the CLI verb**

`routes.rs`, in `auth_only_routes()` (**not** gated — a revoked principal cannot pass the gate, which is the whole point):

```rust
        .routes(routes!(handlers::access::create_review_request))
```

`cli.rs`, alongside `RequestAccess`/`WithdrawRequest` at `:890-897`:

```rust
    /// Ask an admin to reconsider a revocation. Does not restore access by itself.
    RequestReview {
        #[arg(long)]
        message: Option<String>,
    },
```

- [ ] **Step 6: Run and commit**

Run: `cargo make test-e2e && cargo make check`

```bash
git add crates/ migrations/20260720000060_review_requests.sql tests/
git commit -m "feat(standing): Request, Withdraw, and RequestReview

RequestReview sets a marker and moves nothing (D15) — the no-laundering property
is structural, not bookkept. It gets its OWN duplicate guard because it does not
inherit `requested` standing's.

create_join_request no longer rejects in open mode: under D11 every door births
Denied regardless of mode, so an open instance was a dead end."
```

---

### Task 13: The admin acts — Approve, Reject, Revoke, Deactivate, Reactivate

**Files:**
- Modify: `crates/temper-services/src/services/access_service.rs` (`review_request` `:836`)
- Modify: `crates/temper-api/src/handlers/access.rs` (the five inline admin checks at `:143,161,188,203,221`)
- Modify: `crates/temper-cli/src/cli.rs` (`AdminRequestsAction` `:1389`), `commands/admin.rs`

**GD-3 tag: AMEND.**

**There is no separate approve and no separate reject** (C8) — both are `review_request`, branching on `params.decision`. Route both branches through the machine:

```rust
    let act = match params.decision {
        JoinRequestStatus::Approved => Act::Approve,
        JoinRequestStatus::Rejected => Act::Reject,
        // The existing guard at :838-844 already rejects anything else with BadRequest; keep it.
        _ => return Err(ApiError::BadRequest("Decision must be 'approved' or 'rejected'".into())),
    };
    standing_service::apply(pool, ApplyStandingParams {
        subject: ProfileId::from(row.requesting_profile_id),
        act,
        actor: Some(params.reviewer_profile_id),
        authority: ActorAuthority::Admin,
    }).await?;
```

**Rejection is deliberately not a state.** Spec §5: "a rejected request returns standing to `Denied` so the principal may re-request — `join_request_rejection_allows_resubmit` (`access_gate_test.rs:403`) already expects this — while the request record keeps the `decision_note`." That existing e2e test is the regression guard and must keep passing unchanged.

**Approve gets three new source states** (D14 `Denied`, D16 `Revoked`, plus the existing `Requested`), and `/admin/access` therefore shows two kinds of row: "people who asked, and machines awaiting a direct grant. That is intended, not an inconsistency to smooth" (§6).

**New endpoints and verbs** — `Revoke`, `Deactivate`, `Reactivate` have no surface today:

| Route | Handler | CLI |
|---|---|---|
| `POST /api/access/admin/principals/{id}/revoke` | `access::revoke_principal` | `temper admin access revoke <profile> --reason <r>` |
| `POST /api/access/admin/principals/{id}/deactivate` | `access::deactivate_principal` | `temper admin access deactivate <profile>` |
| `POST /api/access/admin/principals/{id}/reactivate` | `access::reactivate_principal` | `temper admin access reactivate <profile>` |
| `POST /api/access/admin/principals/{id}/approve` | `access::approve_principal` | `temper admin access approve <profile>` |

All four mount in `gated_routes()` with plain `.route()` (no `#[utoipa::path]`), matching the operator-only convention at `routes.rs:151-169`. **Add their paths to `.github/scripts/check-openapi-routes.sh`'s allowlist or CI fails** — `routes.rs:177` and `:210` both point at that script.

**`--reason` is required on revoke, and that friction is a feature.** `Revoke { reason }` carries it into the log and the ledger, and a reviewer of a later `RequestReview` needs it — "a reviewer needs the revocation reason, which a plain `Request` has no slot for" (D15).

- [ ] **Step 1: Write the failing tests**

Add to `tests/e2e/tests/admin_surface_e2e.rs` — one test per act, plus these two which are the ones most likely to regress:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn approve_works_from_denied_so_machines_are_not_a_dead_end(pool: sqlx::PgPool) {
    // D14 — machines have no self and can never Request. Without Approve-from-Denied the entire
    // machine surface is a dead end. This makes it two pipelines; `requested` is human-only.
    let app = common::setup(pool.clone()).await;
    preflight(&app, &app.token).await;
    let machine = common::provision_machine_as(&app, &app.token, "m-approve").await;

    let resp = app.reqwest_client
        .post(app.url(&format!("/api/access/admin/principals/{machine}/approve")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let state: String = sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
        .bind(machine).fetch_one(&pool).await.unwrap();
    assert_eq!(state, "approved");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reactivate_restores_rather_than_guesses(pool: sqlx::PgPool) {
    // Spec §5. A principal deactivated while Revoked must come back Revoked, not Approved — the
    // failure mode here is silently upgrading someone during a deactivation round-trip.
    let app = common::setup(pool.clone()).await;
    let admin = profile_id(&preflight(&app, &app.token).await);
    let token = common::generate_second_user_jwt();
    let id = profile_id(&preflight(&app, &token).await);

    common::approve_then_revoke(&app, admin, id).await;
    common::admin_act(&app, id, "deactivate").await;
    common::admin_act(&app, id, "reactivate").await;

    let state: String = sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(state, "revoked", "reactivation restores the prior state, it does not guess");
}
```

- [ ] **Step 2: Run to verify they fail, implement, and re-run**

Run: `cargo make test-e2e` → FAIL (no routes) → implement → PASS.

- [ ] **Step 3: Commit**

```bash
cargo make check && cargo make test-e2e
git add crates/ tests/ .github/scripts/check-openapi-routes.sh
git commit -m "feat(standing): the five admin acts on API and CLI

Approve gains Denied (D14 — machines never Request) and Revoked (D16 — Reinstate
was identical to it and prior_state already makes a reinstatement legible).
Rejection stays NOT a state: it returns standing to Denied so the principal may
re-request, which join_request_rejection_allows_resubmit already pins."
```

---

### Task 14: D17 — machine credential revocation fires `Revoke` on standing

**Files:**
- Modify: `crates/temper-services/src/services/machine_client_service.rs` (`revoke` `:124`)

**GD-3 tag: AMEND**, authorized by D17.

**The finding this implements was overstated by the pressure test, and the correction matters.** Machine revocation was reported as a live cross-table AND. It is not: credential revocation is rejected at **authentication**, not authorization —

```rust
// profile_service.rs:243-249
if let Some(revoked_at) = client.revoked_at {
    tracing::warn!(client_id, %revoked_at, "machine gate: rejected (revoked client)");
    return Err(ApiError::Unauthorized(...))
}
```

`Unauthorized` is Level 1. A revoked machine never reaches admission. **What is real is thinner:** the two facts can *disagree* — a machine can sit credential-revoked with standing `Approved` and its grants intact. That state is inert today (nothing can authenticate as it, `rebind` refuses a revoked source, a fresh `provision` mints a new profile) so the exposure is **drift, not a present hole** — but it reads badly in audit: the operator believes the machine is cut off while the ledger says the principal is admitted.

**One prior intent to preserve, and D17 reaches into it — confirm, do not assume.** Revocation deliberately *leaves* grants and memberships so a `rebind` cannot silently resurrect them:

```rust
// machine_client_service.rs:121-123
/// Grants and memberships are deliberately untouched (D11).
```
```rust
// machine_registration_service.rs:383-386
// A revoked credential is dead; it must be re-created by a fresh `provision`, never
// resurrected under a new `client_id`. Rebinding one would revive its surviving grants and
// memberships (revoke leaves them, D11), silently undoing a deliberate revocation.
```

Firing `Revoke` on standing denies *admission*, which sits above grants and does not touch them — so the intent survives. **Task 14 must prove that with a test, not assert it.**

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-services/src/services/machine_client_service.rs`'s test module (near `:505`):

```rust
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoking_a_credential_also_revokes_standing_but_leaves_grants(pool: PgPool) {
        // D17 — one revocation fact, one place. `revoked_at` becomes purely an authentication
        // detail; standing tells the whole story; the two cannot drift because there is only one
        // decision.
        let f = seed_machine_with_reach(&pool).await;
        approve_standing(&pool, f.machine_profile).await;

        let grants_before: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_access_grants WHERE principal_id = $1", *f.machine_profile)
            .fetch_one(&pool).await.unwrap().unwrap_or(0);
        let members_before: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE profile_id = $1", *f.machine_profile)
            .fetch_one(&pool).await.unwrap().unwrap_or(0);
        assert!(grants_before > 0 && members_before > 0, "fixture must have reach to preserve");

        revoke(&pool, f.client_id, f.admin).await.unwrap();

        let state: String = sqlx::query_scalar!(
            "SELECT state FROM kb_principal_standing WHERE profile_id = $1", *f.machine_profile)
            .fetch_one(&pool).await.unwrap();
        assert_eq!(state, "revoked", "one revocation fact, not two that can drift");

        // THE PRIOR INTENT, PROVEN NOT ASSUMED. Revocation deliberately leaves grants and
        // memberships so a rebind cannot silently resurrect them. Revoke on standing denies
        // admission, which sits ABOVE grants and does not touch them.
        let grants_after: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_access_grants WHERE principal_id = $1", *f.machine_profile)
            .fetch_one(&pool).await.unwrap().unwrap_or(0);
        let members_after: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_team_members WHERE profile_id = $1", *f.machine_profile)
            .fetch_one(&pool).await.unwrap().unwrap_or(0);
        assert_eq!(grants_after, grants_before, "D11's intent survives D17");
        assert_eq!(members_after, members_before, "D11's intent survives D17");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoking_an_unapproved_machine_is_not_an_error(pool: PgPool) {
        // Revoke is illegal from Denied (§6 — you cannot revoke what was never granted), but a
        // credential revocation must still succeed. The standing fire is best-effort in exactly
        // this cell and MUST NOT fail the operation.
        let f = seed_machine_with_reach(&pool).await; // born Denied under D11
        revoke(&pool, f.client_id, f.admin).await.expect("credential revocation must succeed");
    }
```

- [ ] **Step 2: Implement**

In `machine_client_service::revoke`, wrap the existing body in a transaction and fire the standing act alongside the `UPDATE`:

```rust
    // D17 — the same transaction. `revoked_at` is an authentication detail; standing is the
    // admission fact. Two facts in two places is how they drift, and drift here reads badly in
    // audit: the operator believes the machine is cut off while the ledger says it is admitted.
    //
    // Best-effort ON THE ILLEGAL CELL ONLY: a machine that was never approved is `Denied`, and
    // Revoke is illegal from Denied (§6). That must not fail the credential revocation — the
    // credential is what the operator asked to kill.
    match standing_service::apply(pool, ApplyStandingParams {
        subject: ProfileId::from(existing.profile_id),
        act: Act::Revoke { reason: format!("machine client {} revoked", existing.client_id) },
        actor: Some(revoker),
        authority: ActorAuthority::Admin,
    }).await {
        Ok(_) => {}
        Err(ApiError::Forbidden) => {
            tracing::debug!(client_id = %existing.client_id,
                "standing was not `approved`; nothing to revoke on the admission axis");
        }
        Err(e) => return Err(e),
    }
```

- [ ] **Step 3: Run and commit**

Run: `DATABASE_URL=… cargo nextest run -p temper-services --features test-db machine_client`

```bash
cargo make check
git add crates/temper-services/
git commit -m "feat(standing): D17 — credential revocation fires Revoke on standing

The pressure test called this a live cross-table AND; it was overstated —
revocation is rejected at AUTHENTICATION (profile_service.rs:243), so a revoked
machine never reaches admission. What is real is DRIFT: the two facts can
disagree and it reads badly in audit.

D11's prior intent is PROVEN, not assumed: grants and memberships survive, so a
rebind still cannot silently resurrect them."
```

---

## Beat F — governance

---

### Task 15: `promote_admin` and demotion-by-transition

**Files:**
- Modify: `crates/temper-services/src/services/access_service.rs` (`promote_admin` `:577-638`)
- Modify: `crates/temper-services/src/services/standing_service.rs` (demote on `Revoke`/`Deactivate`)
- Modify: `crates/temper-cli/src/cli.rs` (add `demote`)

**GD-3 tag: AMEND**, authorized by D10 and §9.

**Invariants, carried verbatim from §9:**
> "**`admin` implies `Approved`.** Promotion guards on standing being `Approved` — you cannot govern an instance you may not use."
> "**Revoke and Deactivate demote**, so 'admin, but admission revoked' is never representable."
> "`is_system_admin` reads governance state directly. It never consults admission at read time, and it never ANDs across tables — the property the old seam was trying to buy, obtained by construction instead of by discipline."

**`promote_admin`'s raw INSERT is retired in the same change** (§9). Note the change in *why*: "Under the old design this was a *requirement* for the invariant to hold (one writer maintaining it against nineteen breaking it); under D10 it is merely cleanup, because the row it writes no longer confers anything."

**The pre-existing demotion bug dies here without being fixed.** Today `promote_admin` writes the membership but deliberately not the tier (`access_service.rs:574-576`'s decoupling comment), while `ensure_auto_join_memberships` derives the role *from* the tier — so they disagree, the tier wins, and a promoted admin is silently demoted `owner → watcher` by any join-request approval. Verified in `BEGIN/ROLLBACK`. **Superseded by D10 rather than fixed:** once admin-ness is not a team role, the two writers have nothing to disagree about. Do not write a separate fix for it in this PR.

**Keep writing the `kb_team_members` row through Phase 1.** It confers nothing after Task 7, but dropping it now would change team rosters mid-flight for no benefit. Phase 2 retires it.

- [ ] **Step 1: Write the failing tests**

Add to `crates/temper-api/tests/admin_settings_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promotion_requires_approved_standing(pool: PgPool) {
    // §9: "you cannot govern an instance you may not use."
    let f = seed(&pool).await;
    let denied = make_profile(&pool, "not-approved").await; // born Denied under D11

    let err = access_service::promote_admin(&pool, denied, None).await
        .expect_err("promoting a non-approved principal must be refused");
    assert!(format!("{err}").to_lowercase().contains("approved"), "the refusal must say why: {err}");

    assert!(!access_service::is_system_admin(&pool, ProfileId::from(denied)).await.unwrap());
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn revoke_demotes_so_admin_but_revoked_is_never_representable(pool: PgPool) {
    // §9. The invariant is maintained BY TRANSITION, never checked at read time — is_system_admin
    // reads governance and nothing else, so if the demotion does not fire, a revoked admin stays
    // admin. That is the whole risk this test covers.
    let f = seed(&pool).await;
    let p = approved_profile(&pool, "soon-revoked").await;
    access_service::promote_admin(&pool, p, None).await.unwrap();
    assert!(access_service::is_system_admin(&pool, ProfileId::from(p)).await.unwrap());

    standing_service::apply(&pool, ApplyStandingParams {
        subject: ProfileId::from(p), act: Act::Revoke { reason: "test".into() },
        actor: Some(f.admin), authority: ActorAuthority::Admin,
    }).await.unwrap();

    assert!(
        !access_service::is_system_admin(&pool, ProfileId::from(p)).await.unwrap(),
        "'admin, but admission revoked' must never be representable (§9)"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deactivate_demotes_too(pool: PgPool) {
    let f = seed(&pool).await;
    let p = approved_profile(&pool, "soon-deactivated").await;
    access_service::promote_admin(&pool, p, None).await.unwrap();

    standing_service::apply(&pool, ApplyStandingParams {
        subject: ProfileId::from(p), act: Act::Deactivate,
        actor: Some(f.admin), authority: ActorAuthority::Admin,
    }).await.unwrap();

    assert!(!access_service::is_system_admin(&pool, ProfileId::from(p)).await.unwrap());
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reactivating_a_demoted_admin_does_not_restore_governance(pool: PgPool) {
    // Reactivate restores STANDING (§5). It says nothing about governance, and silently
    // re-admining someone on reactivation would make a deactivation a round-trip that quietly
    // returns authority. Re-promotion is a separate, deliberate, audited act.
    let f = seed(&pool).await;
    let p = approved_profile(&pool, "round-trip").await;
    access_service::promote_admin(&pool, p, None).await.unwrap();

    for act in [Act::Deactivate, Act::Reactivate { prior: None }] {
        standing_service::apply(&pool, ApplyStandingParams {
            subject: ProfileId::from(p), act, actor: Some(f.admin), authority: ActorAuthority::Admin,
        }).await.unwrap();
    }

    assert_eq!(
        standing_service::load(&pool, ProfileId::from(p)).await.unwrap(),
        Some(Standing::Approved), "standing is restored"
    );
    assert!(
        !access_service::is_system_admin(&pool, ProfileId::from(p)).await.unwrap(),
        "governance is NOT restored — re-promotion is its own act"
    );
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test admin_settings_test`
Expected: FAIL — promotion succeeds from `Denied`; revoke does not demote.

- [ ] **Step 3: Implement**

**`promote_admin`** — add the standing guard before any write, and replace the raw `kb_team_members` INSERT's authority role with a governance grant:

```rust
    // §9: admin implies Approved. Guarded at promotion, maintained by transition — never ANDed at
    // read time, which is the shape D2 forbids and the old seam could not avoid.
    let standing = standing_service::load(pool, ProfileId::from(profile_id)).await?;
    if standing != Some(Standing::Approved) {
        return Err(ApiError::BadRequest(format!(
            "profile '{profile_id}' has standing {:?}; only an approved principal may be promoted \
             — you cannot govern an instance you may not use",
            standing.map(|s| s.as_str()).unwrap_or("none")
        )));
    }

    sqlx::query_scalar!("SELECT principal_governance_set($1,true,$2,NULL)", profile_id, *actor)
        .fetch_one(pool).await?;
```

Keep the existing `kb_team_members` upsert (it confers nothing now; Phase 2 removes it) and keep returning `TeamMemberRow` so the handler and CLI signatures are unchanged.

**Demotion** — in `standing_service::apply`, after a successful commit:

```rust
    // §9 — Revoke and Deactivate demote, so "admin, but admission revoked" is never representable.
    // Maintained BY TRANSITION. This is the one place the two machines touch, and it is a
    // one-directional write (admission → governance), not a read-time AND.
    if matches!(resulting, Standing::Revoked | Standing::Deactivated) {
        sqlx::query_scalar!(
            "SELECT principal_governance_set($1,false,$2,$3)",
            *params.subject,
            params.actor.map(|a| *a),
            Some(format!("demoted by {act_name}")),
        ).fetch_one(pool).await?;
    }
```

Add `temper admin access demote <profile>` mirroring `promote`, calling `principal_governance_set(_, false, _, _)`.

- [ ] **Step 4: Run and commit**

Run: `cargo make check && cargo make test-db && cargo make test-e2e`

```bash
git add crates/
git commit -m "feat(governance): promotion guards on standing; Revoke/Deactivate demote

D10/§9 — the invariant is maintained by TRANSITION, never checked at read time.
is_system_admin reads governance and nothing else, so a missed demotion would
leave a revoked admin admin; four tests cover that cell.

promote_admin's raw kb_team_members INSERT is retired as the authority write.
Under the old design retiring it was a REQUIREMENT for the invariant; under D10
it is merely cleanup, because the row no longer confers anything.

The pre-existing promote/tier demotion bug is SUPERSEDED here, not fixed: once
admin-ness is not a team role, the two disagreeing writers have nothing to
disagree about."
```

---

## Beat G — retire `access_mode`

---

### Task 16: Remove every `access_mode` reader

**Files:**
- Modify: `crates/temper-services/src/services/access_service.rs` (`create_join_request` — already done in Task 12; `get_public_settings`, `update_system_settings`)
- Modify: `crates/temper-api/src/middleware/system_access.rs` (`:41-58`)
- Modify: `crates/temper-services/src/services/machine_registration_service.rs` (`enroll_in_gating_team`'s doc, `:26-50`)
- Modify: `crates/temper-cli/src/cli.rs` (`admin settings --access-mode`, `:1026-1032`)

**GD-3 tag: AMEND.** Authorized by spec §14 ("The `access_mode` retirement is still real work") and Pete's 2026-07-20 decision.

**Scope, restated because it is the one place this plan deviates from a literal reading of the decision.** "Retire fully" is implemented as: **Phase 1 removes the concept — every reader, the settings flag, the gate-path usage. Phase 2 drops the column.** This keeps Phase 1 additive-on-schema and preserves the auto-deploy invariant D10 just recovered. If the column must go in this PR, move the `ALTER TABLE kb_system_settings DROP COLUMN access_mode` here and accept that Phase 1 becomes operator-run.

**What `access_mode` meant, and why nothing is lost.** It selected between "everyone has access" and "gating-team members have access." Standing now answers that question per-principal, which is strictly more expressive: `open` is the state where every principal happens to be `Approved`, and there is no longer a global switch that can silently flip a whole instance's access in one `UPDATE`. **That loss of a global switch is the point**, not a regression to compensate for.

**`enroll_in_gating_team`'s long doc comment is now substantially wrong** and must be rewritten, not deleted. It reasons at length about `access_mode` being `open` making the trigger auto-join everyone, and about the caller check binding "the day that stops." Under D11 minting confers nothing regardless, so the containment it describes is retired (§3 D11: *"minter containment becomes unnecessary, not relocated"*). Replace the rationale; keep the function, which is still doing ordinary team hygiene.

- [ ] **Step 1: Enumerate every reader**

Run:
```bash
cd /Users/petetaylor/projects/tasker-systems/temper
grep -rn 'access_mode\|AccessMode' crates/ --include='*.rs' | grep -v '^crates/[^/]*/tests/'
grep -rn 'access_mode' migrations/
```
Expected: the settings read/write pair in `access_service.rs`, the 403-detail population at `middleware/system_access.rs:49`, the CLI flag at `cli.rs:1028`, the `SystemAccessDetails` field, and the two error.rs test fixtures. **Work from the command's output, not from this list** — the plan's list is a hypothesis and the grep is ground truth.

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn no_production_code_reads_access_mode() {
    // A guard, not a unit test. access_mode is retired as a concept in Phase 1; the column
    // survives only until Phase 2's drop. A new reader would silently re-couple admission to a
    // global switch, which is exactly what standing replaced.
    let hits = std::process::Command::new("grep")
        .args(["-rn", "access_mode", "--include=*.rs", "crates/"])
        .output().expect("grep");
    let text = String::from_utf8_lossy(&hits.stdout);
    let offenders: Vec<&str> = text.lines()
        .filter(|l| !l.contains("/tests/"))
        .filter(|l| !l.contains("// RETIRED"))
        .collect();
    assert!(offenders.is_empty(), "access_mode readers remain:\n{}", offenders.join("\n"));
}
```

> Put this in `crates/temper-services/tests/` and mark it clearly as a lint-shaped test. If the team would rather this live in `.github/scripts/` alongside the other repo-shape guards, that is a reasonable alternative — pick one, do not do both.

- [ ] **Step 3: Implement, run, commit**

Run: `cargo make check && cargo make test-all`

```bash
git add crates/
git commit -m "refactor(access): retire access_mode as a concept

Standing answers per-principal what access_mode answered globally, which is
strictly more expressive — and there is no longer a single UPDATE that can flip
a whole instance's access. That loss of a global switch is the point.

The column survives until Phase 2's drop so Phase 1 stays additive on schema.
enroll_in_gating_team's rationale is rewritten rather than deleted: minter
containment is retired (D11), but the function is still team hygiene."
```

---

## Beat H — the typed refusal

---

### Task 17: `Refusal` replaces the stringly 403

**Files:**
- Modify: `crates/temper-services/src/error.rs` (`:14-17`, `:66-70`, `:110-115`, `:161-176`, `:195-219`, and the two tests at `:293`, `:371`)
- Modify: `crates/temper-core/src/types/access_gate.rs` (`SystemAccessDetails`)
- Modify: `crates/temper-api/src/middleware/system_access.rs` (`:41-58`)
- Modify: `crates/temper-cli/src/access_gate.rs`
- Modify: `crates/temper-client/src/http.rs:587` (the wire fixture)

**GD-3 tag: AMEND**, authorized by spec §7: *"`Refusal` is a typed enum, which retires a wart — the enriched 403 currently carries `access_mode: String`, and its tests assert a sentinel `"join_request"` that is not a real mode."*

**Verified: the sentinel really is fiction.** `"join_request"` appears only at `error.rs:299` and `error.rs:377`, both test fixtures. Production populates the field from `get_public_settings(...).access_mode`, whose live domain is `open`/`invite_only` (`middleware/system_access.rs:49`), and the client-side fixture at `http.rs:587` correctly uses `"invite_only"`. So the tests have been asserting a value the system never emits.

**This task also resolves Task 10's placeholder.** `ApiError::Forbidden2` must not exist by the end of this beat — `standing_service::apply` returns a `Refusal`, and the surfaces render it.

**Wire-contract change — this is a `SystemAccessDetails` shape change.** Per repo convention (`feedback_wire_contracts_need_semver_now`), prefer additive: **add** a `refusal` field carrying the typed value and **keep** `access_mode` in the payload for one release, populated with a compatibility value, rather than removing it in the same PR that adds the replacement. Deployed CLIs parse this body (`temper-client/src/http.rs:587`), and an old CLI hitting a new server should degrade to a generic message, not fail to parse.

**Regenerating the contract artifacts is mandatory and gates CI.** A changed response DTO restales three committed artifacts:
```bash
cargo make openapi   # regenerates openapi.json, the temper-rb gem, and temper-ts's schema.ts
git add openapi.json clients/temper-rb/lib/temper/generated clients/temper-ts/src/generated/schema.ts
```
The drift gates compare against **git**, not against a fresh build — so a correctly regenerated artifact still fails `cargo make check` while it sits unstaged, with an error that reads like you forgot to regenerate. Stage first, then re-run `check`.

- [ ] **Step 1: Update the two lying tests first**

`error.rs:293` and `error.rs:371` assert `access_mode == "join_request"`. Change them to assert the **typed refusal** round-trips, and use a real mode value in any surviving `access_mode` field. Their intent — that the field set survives both conversion directions — is worth keeping; only the fixture value was fiction.

- [ ] **Step 2: Carry the typed refusal**

Add to `SystemAccessDetails`:

```rust
    /// Why this principal was refused, typed (spec §7). Replaces the inference callers previously
    /// had to make from `access_mode` + `join_request_status`, which could not distinguish
    /// "never granted" from "granted and revoked" — a distinction that matters both to the user
    /// and in an audit.
    pub refusal: temper_principal::Refusal,
```

`middleware/system_access.rs` populates it from `standing_service::admit`'s `Err` rather than assembling settings + own-request. **That removes two database round-trips from the refusal path** (`get_public_settings` and `get_own_request`, `:41-45`) — the typed refusal already carries everything the message needs.

- [ ] **Step 3: Render it on the CLI**

`crates/temper-cli/src/access_gate.rs` currently branches on `join_request_status` strings. Branch on `Refusal` instead — exhaustively, no catchall, so a new refusal variant forces a message rather than falling through to a generic one.

The four user-facing messages, from D12 and §5:
- `Denied` → *"Access has not been granted. Run `temper auth request-access` to ask."*
- `Requested` → *"Your access request is pending review."*
- `Revoked` → *"Your access was revoked. Run `temper auth request-review` to ask for reconsideration."*
- `Deactivated` → *"This account is deactivated. Contact an administrator."*

> That messaging distinction "is the real justification for `Requested` existing as a state, and it only works if the two states mean different things" (D12).

- [ ] **Step 4: Run everything and commit**

```bash
cargo make openapi
git add openapi.json clients/
cargo make check && cargo make test-all && cargo make test-e2e
git add crates/
git commit -m "feat(access): a typed Refusal replaces the stringly 403

The retired wart: the enriched 403 carried access_mode: String and its tests
asserted a sentinel 'join_request' that production never emits — the live domain
is open/invite_only. The tests had been pinning fiction.

Additive on the wire: `refusal` is added, `access_mode` stays one release, so a
deployed CLI degrades to a generic message rather than failing to parse.

Also drops two DB round-trips from the refusal path — the typed value carries
what get_public_settings + get_own_request were being consulted for."
```

---

## Self-Review

Run against the spec after the last task, before opening the PR.

**Spec coverage.** Every section mapped to a task:

| Spec | Where |
|---|---|
| D1, D3 (two machines, pure crate) | Tasks 1–3 |
| D2 (one state, one table) | Task 4 + its uniqueness test |
| D4 (log + event, atomic) | Task 6 |
| D5, D12 (payload vs state; born Denied) | Tasks 11, 12 |
| D6 (`is_active` folded in) | Task 8 rule 1 — **see the open item below** |
| D7 (connection profiles, no row) | Task 8 rule 0 + Task 9's assertion |
| D8 (backfill by the old predicate) | Tasks 8–9 |
| D9 (per-principal, no team dimension) | Task 4's `%team%` test |
| D10, §9 (governance at the outset) | Tasks 4, 7, 15 |
| D11 (every door births Denied) | Task 11 + the whole-surface §12 test |
| D13 (SAML rationale withdrawn) | Task 11's `saml_jit_is_also_born_denied` |
| D14, D16 (Approve's three sources) | Tasks 2, 13 |
| D15 (review is a marker) | Task 12 + its three obligations |
| D17 (one revocation fact) | Task 14 |
| §7 (three fail-closed obligations) | Tasks 1, 3, 7 |
| §11 (phasing, backfill, extra passes) | Tasks 8–9 |
| §12 (verification) | Tasks 2, 9, 11, 12, 14 |
| §14 (`access_mode` retirement) | Task 16 |

**Gaps found and carried honestly:**

1. **D6 is only partly delivered.** §11's rule 1 folds `is_active` into `Deactivated`, and Task 8 implements that. But the two Rust readers of `kb_profiles.is_active` (`auth/mod.rs:246`, `slack_grant_vault_service.rs:214`) still read the column, not standing. Phase 1 leaves them — the column survives as a projection, so they keep working, and moving them is Phase 2's job alongside the drop. **§13's open question 2 is now answered empirically** (C2: two Rust readers, zero SQL readers), which is what makes that deferral safe rather than hopeful.
2. **§13 open question 1 stays open.** "Where the `has_system_access` call sites belong" across Level 3's SQL is deliberately deferred by the spec and this plan does not touch it.
3. **No MCP surface for any act.** Confirmed absent today and out of scope here; §14 already files it. This is a **parity gap against the repo's full-surface-parity norm** and should be a follow-up task, not a silent omission.

**Placeholder scan.** One deliberate placeholder is flagged in-plan and must not survive: `ApiError::Forbidden2` in Task 10, resolved in Task 17. Two in-plan markers are forcing functions rather than content: the `<<< paste … >>>` blocks in Task 5 (the migration will not apply with them present) and the helper names in Tasks 11–13 flagged as "check before writing."

**Type consistency.** `Standing`, `Act`, `ActorAuthority`, `Provisioner`, `Refusal`, `AdmittedPrincipal`, `ApplyStandingParams`, `standing_service::{load, apply, admit, provision}`, `principal_standing_apply`, `principal_prior_standing`, `principal_governance_set` are spelled identically in every task that names them. `Standing::as_str` and the SQL `CHECK` literals agree (`denied`/`requested`/`approved`/`revoked`/`deactivated`), and `act_name` agrees with the log's `act` CHECK.

---

## Follow-ups — file these, do not fold them in

1. **`SystemAuthorized` and `AuthenticatedProfile` do not enforce their documented type-state** (C1). Both have fully public fields and no `impl` block, so any crate can forge one. Their doc comments claim otherwise. Fixing it touches every construction site across surfaces, which is why Pete scoped it out of this PR — but the docs currently assert a guarantee that does not exist, and that is its own hazard.
2. **MCP has no join-request or admission surface at all.** Filed in §14; a parity gap against the repo norm.
3. **The pre-existing `promote_admin` / tier demotion bug** (spec §16). Superseded by D10 rather than fixed. Prod is not exposed today (both admins carry tier `admin`); it bites the next admin promoted this way, in the window before this ships. File it only if the fix is wanted before Phase 1 lands.
4. **The five inline `is_system_admin` checks in `handlers/access.rs`** (`:143,161,188,203,221`) are copy-pasted rather than shared. Not this PR's story, but it is now five sites plus four new ones.
5. **Phase 2's plan**, which C2 makes materially smaller than §11 budgets for: dropping `kb_profiles.is_active` breaks zero SQL functions, not four.

---

## Before implementing

**Re-verify production, foregrounded.** §15: *"Production state moves and must be re-verified foregrounded before implementation. `access_mode` has now changed twice across the sessions that produced this design."* It was last checked on 2026-07-20 and read: `invite_only`, gating `temper-system`, 6 principals (2 admin / 3 approved / 1 none), all `is_active = true`, old predicate true for all six, zero join requests. **Task 8's governance pass depends on the admin count** — if it is not 2, the pass is still correct but the post-deploy check differs.

**Cut a fresh branch off `main`.** The spec lives on `jct/principal-admission-state-machine-spec` @ `13cc3520`, which is unmerged. Per the repo's default-to-independent-PRs rule, do not stack: merge the spec branch first, or cut from `main` and let the spec ride in its own PR.

**Do not build on `jct/system-access-regate` @ `8938a251`.** It does not compile (five `query!` sites deliberately missing from `.sqlx`). Its three `auth/mod.rs` tests encode assertions this design still owes — Task 11 carries two of them forward with inverted expectations (SAML is now born `Denied`, D13) and the third unchanged in intent (test each door separately, never the pair).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-20-principal-admission-phase-1.md`. Two execution options:

**1. Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration. Beat A (Tasks 1–3) is genuinely independent and self-contained; Beats B–H are sequential and each needs the previous beat's schema.

**2. Inline Execution** — execute in this session using `superpowers:executing-plans`, batching with checkpoints.

Whichever is chosen: **inject `implementation-grounding.md` verbatim into every implementer prompt.** This plan's citations are the only pre-grounded facts; anything it does not cite must be verified on disk before use.
