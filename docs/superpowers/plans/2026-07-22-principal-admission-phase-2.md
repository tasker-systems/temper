# Principal Admission — Phase 2 (the destructive drops)

**Task:** `019f8a93-f179-7782-9cad-25df66ebdb69` (build/large, plan-first).
**Goal:** `019f7cdb-a1b6-7e80-b19a-349a3d427671`.
**Spec:** [`docs/superpowers/specs/2026-07-20-principal-admission-state-machine-design.md`](../specs/2026-07-20-principal-admission-state-machine-design.md) — §8, §9, §11, §14/D18, §6.
**Phase 1 plan (executed):** [`2026-07-20-principal-admission-phase-1.md`](./2026-07-20-principal-admission-phase-1.md).
**Grounded against:** `main` @ `fa504238` (PR #512 merge), live dev DB, 2026-07-22.

Phase 1 (all 17 tasks / Beats A–H) is shipped and deployed; every Phase-1 PR was additive-on-schema.
The columns Phase 1 stopped *controlling* survive read-only as projections. Phase 2 removes the code
that still touches them, then drops them. This plan was written **after** Phase 1 was real, per the
goal's condition ("its shape depends on what the real implementation leaves behind") — and the
re-grounding below finds **three places where spec §11 or the task prose diverges from shipped
code**. Those are called out as AMEND, with the evidence.

---

## 0. Decisions recorded (both were preconditions to planning)

### D-A · `Revoke` stops Slack minting — `state = 'approved'` (satisfies task `019f80a9`)

Task `019f80a9-89d4-78f2-a9f4-e0a267d37616` requires this answered *before* Phase 2 is planned, with
reasoning. When `kb_profiles.is_active` drops, the Slack mint kill-switch at
`slack_grant_vault_service.rs:265` is rewritten to **`standing = 'approved'`** (only an approved
principal mints), **not** the behaviour-preserving `standing <> 'deactivated'`.

Reasoning: `<> 'deactivated'` is a mechanical translation of today's `is_active` gate, and under D11
it would let a **born-`Denied`** Slack-linked human mint a token — reopening the exact gap the design
exists to close (§8: being born `Denied` *is* the access-control mechanism). `= 'approved'` makes a
`Revoke` stop minting immediately, which is what "revoked" means (the Slack security audit named the
mint-after-disconnect property as the integration's weakest). Caveat recorded: this is a *partial*
close — an **already-issued** access token still authenticates to its own `exp`, which temper does
not control. A test pins the chosen behaviour (a `Revoked` principal cannot mint — asserted).

### D-B · Keep `kb_join_requests.status`; drop only the wire field (AMENDs spec §11)

The task and spec §11 list `kb_join_requests.status` as a **column** drop. Grounding shows this is a
**spec-vs-shipped divergence** (see §1 evidence E4): spec §11 justified the drop by "`Requested`
standing is the duplicate guard now (D12)", and that half is true — `state='requested'` exists and
guards duplicates. But the *column* also carries the **request-outcome audit** (pending → approved /
rejected / withdrawn) with **live readers** (`get_own_request` → the CLI and the 403), which
`standing='requested'` does not replace; the header of `access_service.rs` states the admin-event
sink that would replace it is "a future deliverable." So **Phase 2 keeps the column** (and its
indexes and the `JoinRequestStatus` PG enum) and drops **only** the 403's `join_request_status` wire
field. Dropping the column would require building the admin-event sink first — out of Phase 2's
destructive-drops scope.

---

## 1. Grounding evidence (printed from disk / live DB — GD-1/GD-2)

Every citation below was re-verified against `main` @ `fa504238`. Where the task/spec said one thing
and the code says another, both are shown.

### E1 · `access_mode` — no SQL reader; two Rust read sites; wire field spans 4 artifacts
- **No SQL function reads the column.** The only `pg_proc` match for `access_mode` was
  `has_system_access`, and printing it shows the match is a **comment** — its body reads
  `kb_principal_standing.state='approved'`. (`\sf has_system_access`.)
- **Prod Rust reads:** `access_service.rs:479` (settings SELECT projection) and `:546` (update
  RETURNING). Both feed `PublicSystemSettings` / `SystemSettings`.
- **Wire field:** `PublicSystemSettings.access_mode: String` at `temper-core/src/types/access_gate.rs:76`
  (comment at `:65-67` already records it as "retired as a control … survives as a read-only String
  until Phase 2"). Propagates to `openapi.json` → `clients/temper-ts/src/generated/schema.ts:3057`,
  `clients/temper-rb/lib/temper/generated/models/public_system_settings.rb`, and
  `packages/temper-ui/src/lib/types/generated/access.ts:27`.
- **Test-seed surface:** ~8 test files seed `access_mode='open'|'invite_only'` (`admin_settings_test`,
  `relationship_handler_test`, `facet_handler_test`, `common/fixtures.rs`, `auto_join_team`,
  `standing_backfill_test`, `identity_graft_test`, plus `#[cfg(test)]` helpers in `connection_service`
  and `machine_registration_service`).

### E2 · `is_active` — breaks 0 SQL; two Rust readers; **no production writer**
- **Breaks zero SQL functions** (confirms spec §11 correction and goal C2). All 30 functions whose
  body mentions `is_active` bind the alias to `kb_resources` or `kb_teams`; the two profile-suspicious
  ones are `context_authorable_by_profile` (`t.is_active` → `kb_teams`) and `profile_effective_teams`
  (`t.is_active` → `kb_teams`). No function reads `kb_profiles.is_active`; no index/constraint on it.
- **Two Rust readers.** `auth/mod.rs:246` (`if !profile.is_active {`) — the Level-1 deactivation gate
  in `gate_resolved_profile` (spec §11 cites `auth/mod.rs:242-246`). And
  `slack_grant_vault_service.rs:`**`265`** (`if row.revoked_at.is_some() || !row.is_active {`). ⚠️
  **The spec and task both cite `:214`; it has moved to `:265`.** (Task explicitly said "re-verify.")
- **No production writer.** All four `UPDATE kb_profiles SET is_active = false` are inside
  `#[cfg(test)]` modules (`auth/mod.rs:588,758`; `slack_grant_vault_service.rs:700`;
  `slack_link_service.rs:503`). Production deactivation is **only** `admin_deactivate`
  (`access_service.rs:735`) → `standing_service::apply(Act::Deactivate)` → `standing='deactivated'`,
  which the two `is_active` readers do **not** observe today. **So the `is_active` drop is a
  reader-repoint that also closes a latent gap** (a standing-`Deactivated` principal can currently
  still mint a Slack token, because `:265` only checks `is_active`).
- **Target predicate exists:** `Standing::Deactivated` (`temper-principal/src/standing.rs:20`);
  `admit()` maps it to `Refusal::Deactivated` (`admission.rs:56`). The DB CHECK allows `deactivated`.
- **Reasoning that must survive the move:** `slack_link_service.rs:85` (`lookup_linked_handle`)
  deliberately does **not** filter on `is_active` (reporting "unlinked" to a deactivated user sends
  them into a link loop the callback then refuses). Per task `019f80a9`, that argument now extends to
  `Denied`/`Revoked` — carry the reasoning, not just a bare `file:line`.

### E3 · `system_access` — no runtime reader; writers dual-write already; 2 SQL fns read the column
- **No production runtime reader of the column.** `access_service.rs:1165` (`get_entitlements`) and
  `cli/commands/auth.rs:276` are **local variables** from `has_system_access(...)` / `resolve_system_access`,
  not column reads. The public `temper-core` `Profile` struct (`profile.rs:20`) does **not** carry
  `system_access`.
- **Writers already dual-write** (Phase 1 landed this — spec §11 "the three system_access writers").
  `scenario/loader.rs:53` and `scenario/access/loader.rs:143` INSERT `system_access` **and** call
  `principal_standing_apply` from the scenario's declared tier; their own comment says *"Keep writing
  system_access (a projection that survives Phase 1) and ALSO mint the standing row that is now
  authoritative."* `bootseed.rs:32` INSERTs `system_access='admin'` for the genesis `system` actor.
- **Two SQL functions read the column** (spec §11 right; a naive grep is wrong). `\sf` shows both use
  it only for the admin→owner team-role coupling:
  - `ensure_auto_join_memberships`: `CASE WHEN (SELECT system_access FROM kb_profiles WHERE id=p_profile)='admin' THEN 'owner' ELSE t.auto_join_role END`
  - `backfill_auto_join_team`: `CASE WHEN p.system_access='admin' THEN 'owner' ELSE v_role END`
  Both have **live Rust callers**: `access_service.rs:1126` and `team_service.rs:205` respectively —
  so they are rewritten, **not** dropped.
- **The trigger** `trg_sync_system_membership` fires `AFTER INSERT OR UPDATE OF system_access ON kb_profiles`
  (`pg_get_triggerdef`), so its definition names the column: the column drop and trigger drop are
  dependency-linked (CASCADE / trigger-first). Its body (`sync_system_membership`) calls
  `has_system_access` + an `ELSE DELETE` from auto-join teams — the body does **not** name the column.
- **Test-seed sweep:** ~25 files `UPDATE/INSERT … system_access` to make a profile "approved"/"admin".
  This is the dominant mechanical cost.

### E4 · `kb_join_requests.status` — live audit trail; the "drop" is a spec-vs-shipped divergence
- `create_join_request` (`access_service.rs:809`) both transitions standing (`Act::Request` → the DB
  `requested` state) **and** INSERTs `kb_join_requests … status='pending'`. The review flow UPDATEs
  `status`; `vw_join_requests` and `get_own_request` READ it; the CLI (`temper auth`) renders it.
- The DB CHECK on `kb_principal_standing.state` **includes `'requested'`** — so `standing='requested'`
  is the duplicate guard (spec §11's justification is real). But the column carries the **outcome
  audit** beyond the guard, with live readers standing does not replace. → **D-B: keep the column.**
- `join_request_status` is also a **PG enum type** and the 403 wire field (`error.rs:15`,
  `access_gate.rs:12,116,153`, `openapi.json:7563/8515`). Keeping the column keeps the enum; only the
  403 **field** drops.

### E5 · The admission gate and demotion ownership (drop-order safety)
- `middleware/system_access.rs` (`require_system_access`, gated router only) builds the 403 from
  `standing_service::admit` (typed `refusal`) **and** `join_request_status: own_request.map(|r| r.status)`
  — the exact site the wire-field drop touches. `require_auth` runs first and already gates
  `Deactivated` ("require_auth already gated deactivation before this layer runs").
- **Governance owns demotion** (`standing_service.rs:119-132`: `Revoke`/`Deactivate` demote), so the
  trigger's `ELSE DELETE` is no longer the only automatic demotion path — spec §11's drop-order
  precondition is met. Residual: dropping the trigger stops the auto-join *membership* cleanup, but
  membership is decorative under D18 (confers no access), so stale memberships are harmless.

---

## 2. Structure — two PRs, split on deployment character (spec §11)

Spec §11: *"Phasing follows deployment character, not size."* Phase 2 itself splits the same way, and
the split is **code-first, then schema-only**, so the destructive migration lands against a codebase
that already touches none of the doomed columns.

- **PR-A (additive; rides auto-deploy under additive-only-on-`main`).** Stop reading/writing the
  doomed columns *everywhere* (prod + tests) and drop the two wire fields. **No schema change, no
  DROP.** After PR-A is deployed to every target, `access_mode`, `is_active`, and `system_access` are
  code-orphaned.
- **PR-B (destructive; operator-run per target via the cutover runbook).** The DROP migrations +
  sqlx cache regen. Pure schema. Nothing in code references the dropped objects, so there is no
  code/schema straddle.

`kb_join_requests.status` is dropped in **neither** PR (D-B).

> **Why the test sweep rides PR-A, not PR-B.** Making fixtures mint `standing` instead of setting
> `system_access` is behaviour-preserving *while the column still exists* (standing is already
> authoritative). Doing it in PR-A keeps PR-B a clean migrations-only change and removes every macro
> reference to the doomed columns before the DROP, so the sqlx cache regen in PR-B has nothing to
> straddle.

---

## 3. PR-A — retire the readers and the wire fields (additive)

Each step tagged CONFORM / EXTEND / AMEND (GD-3). Implementers: the plan's citations are the only
pre-grounded facts; verify anything else on disk (GD-1). **No code bodies are authored here by
intent (GD-4)** — read the cited sites and the cited spec sections.

### A1 · Repoint the two `is_active` readers to standing — AMEND
- **Change:** `auth/mod.rs:246` and `slack_grant_vault_service.rs:265` (E2 — note the moved line).
  `:246` → refuse a `standing='deactivated'` principal (the Level-1 deactivation gate; align with
  `admit()`/`Refusal::Deactivated`). `:265` → **D-A**: mint only when `standing='approved'`.
- **Cite:** spec §6 (acts/guards), §7 (fail-closed), and task `019f80a9` (Slack analysis + the
  `lookup_linked_handle` reasoning to carry, E2). AMEND authorization: spec §11 names these two sites
  as where deactivation is enforced in Rust and directs their move ahead of the column drop.
- **Also:** remove `is_active` from the `temper-core` `Profile` struct (`profile.rs:28`) and the
  `profile_service.rs:592` SELECT, so nothing selects the column. Confirm no other reader of
  `Profile.is_active` remains (grep before removing — GD-1).
- **Tests (pin, don't infer):** a `Revoked`/`Deactivated` principal cannot mint (D-A); a
  `standing='deactivated'` principal is refused at `:246`; and — closing the accidental safety task
  `019f80a9` names — the Slack **link callback fires no `Provision`** (a linked-Slack human keeps one
  standing row). E2E at the production caller's level.

### A2 · Remove the `access_mode` read + wire field; regen SDKs + UI — AMEND (D18)
- **Change:** drop `access_mode` from the settings projection (`access_service.rs:479`, `:546`) and
  from `PublicSystemSettings`/`SystemSettings` (`access_gate.rs:76,90,99`). The column still exists
  (dropped in PR-B) but nothing reads it.
- **Regen chain (tri-artifact + UI):** `cargo make generate-ts-types` (→ `temper-ui access.ts:27`),
  `cargo make openapi` (→ `openapi.json` + `temper-rb` + `temper-ts schema.ts:3057`), then **stage
  the regenerated artifacts** (the drift gate compares against git, not a fresh build), and run
  `cd packages/temper-ui && bun run check` (not covered by `cargo make check`).
- **Cite:** spec §14 / D18 (access_mode retired). AMEND authorization: D18 + the `access_gate.rs:65-67`
  comment that already schedules this field's removal "until Phase 2."

### A3 · Drop the `join_request_status` wire field (keep the column + enum) — AMEND (partial, per D-B)
- **Change:** remove the `join_request_status` field from `SystemAccessDetails` (`access_gate.rs:153`)
  and its population in `middleware/system_access.rs` (E5) and `error.rs` round-trip (`:164-218`).
  **Keep** `kb_join_requests.status`, its indexes, `vw_join_requests`, `get_own_request`, the CLI
  rendering, and the `JoinRequestStatus` PG enum (D-B).
- **Regen chain:** same tri-artifact as A2 (the field is in `openapi.json:7563`).
- **CLI fallback:** Phase 1 kept this field "one release for the deployed CLI"
  (`client/src/http.rs:404`, `cli/access_gate.rs:99`). Removing it is safe once CLIs have rolled; the
  CLI's typed-`Refusal` branch already covers the primary path. Confirm the legacy
  `render_from_join_request_status` fallback is either retained harmlessly or removed with the field.
- **Cite:** D-B above; task 019f8a93 ("retire the wire field once CLIs have rolled").

### A4 · Stop writing `system_access` from the three writers; rewrite the two SQL functions — AMEND (D18)
- **Writers (E3):** remove the `system_access` column write from `scenario/loader.rs:53`,
  `scenario/access/loader.rs:143`, and `bootseed.rs:32`; **keep** the `principal_standing_apply` mint
  (loaders) and mint `standing='approved'` for the genesis `system` actor if it needs one (verify —
  the system actor is an event emitter; grounding task).
- **Two SQL functions (E3):** rewrite `ensure_auto_join_memberships` and `backfill_auto_join_team` to
  drop the `system_access='admin' → 'owner'` branch (use `auto_join_role` uniformly). Spec §11
  already blessed the resulting "cosmetic team-role churn" (`owner → watcher`) as safe under D10/D18,
  because admin-ness lives in `kb_principal_governance` now, not the team row. New additive migration
  (`CREATE OR REPLACE FUNCTION`), not an edit to the birth migration.
- **Live callers unchanged:** `access_service.rs:1126`, `team_service.rs:205` keep calling them.
- **Out of scope:** `enroll_in_gating_team` (called by `machine_registration_service.rs:251,333`)
  does **not** read `system_access`; its retirement is goal-deferred to the machine-principal work.
- **Cite:** spec §9, §11 (the `system_access` writers + the trigger note), D18.

### A5 · The test-seed sweep — CONFORM
- Switch the ~25 fixtures that `UPDATE/INSERT … system_access` (E3) and the ~8 that seed
  `access_mode` (E1) to mint standing / omit the retired column. Prefer **one shared test helper**
  (mint `standing='approved'`/`admin` via `principal_standing_apply` + governance) over 25 bespoke
  edits — the incumbent to conform to is the loaders' existing standing-mint (E3).
- **Cite:** spec §12 (verification) for what the fixtures must still guarantee.

**PR-A verification:** `cargo make check` (offline sqlx honesty), `cargo make test-all`,
`cargo make test-e2e-embed`, `cd packages/temper-ui && bun run check`. The columns still exist, so
nothing should reference them — grep `access_mode`/`is_active`/`system_access` in non-test prod code
and confirm only the PR-B drop targets remain.

---

## 4. PR-B — the drops (destructive, operator-run)

New migrations dated `20260722…` (naming: `YYYYMMDDHHMMSS_name.sql`, per `migrations/`), **order
load-bearing**:

1. `DROP TRIGGER trg_sync_system_membership ON kb_profiles;` — first (governance owns demotion, E5;
   membership is decorative under D18).
2. `ALTER TABLE kb_profiles DROP COLUMN system_access;` — after the trigger (the trigger definition
   names the column). Then drop the `system_access` enum type if unreferenced (verify).
3. `ALTER TABLE kb_profiles DROP COLUMN is_active;` — breaks **no** SQL (E2).
4. `ALTER TABLE kb_system_settings DROP COLUMN access_mode;`.
5. **Not dropped:** `kb_join_requests.status`, its indexes, the `join_request_status` enum (D-B) —
   state this explicitly in the migration so a future reader knows it was a decision, and that spec
   §11's "drop it + `idx_join_requests_one_pending`" was superseded by the shipped lifecycle.

Then: `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` →
`cargo make prepare-e2e` (per-crate last; §"SQL Query Checking"), and commit the `.sqlx` churn.
Each migration should **assert** its precondition where cheap (e.g. no live reader) rather than trust
it (spec §11's "assert the count rather than trust it" discipline).

**CI green, then stop.** Pete runs PR-B's migrations per target (temperkb.io + the one enterprise
instance, ~12 alpha testers — §11 deployment surface).

---

## 5. Operator runbook (PR-B, per target)

Per the cutover doc; one target at a time.

1. **Precondition:** confirm PR-A is deployed on the target (the running binary reads none of the
   doomed columns). This is the additive-then-destructive safety — do not run PR-B against a binary
   that predates PR-A.
2. **Snapshot** `kb_profiles` / `kb_system_settings` (rollback evidence).
3. **Apply** the PR-B migrations in order (1→4 above).
4. **Verify:** `has_system_access` / `is_system_admin` still answer from standing/governance; a
   sample approved principal still resolves; the gating flows (request/approve, Slack mint under
   D-A) behave. Read prod via the neonctl→psql path (foregrounded, joint), not a subagent.
5. **temperkb.io specifics:** all six principals are `is_active=true` (§15), zero join requests —
   the deactivated-flip cell is empty there. The enterprise instance is unverified; treat its
   deactivated/pending populations as real and check them before dropping.

---

## 6. Out of scope (named, not forgotten)

- **MCP admission parity** — held lower-tier (goal + task). The shared gate (`has_system_access`/
  `admit`) is repointed by A1–A4; the MCP request/withdraw/review *surface* is untouched.
- **`kb_join_requests.status` column drop** — deferred with the admin-event sink (D-B).
- **`enroll_in_gating_team` / gating-team enrollment retirement** — goal-deferred to machine-principal
  work.
- **Sealing `AuthenticatedProfile`** (`019f8a94`) and **precise TS/RB `Refusal`** (`019f8a95`) —
  separate filed tasks.

---

## 7. Acceptance criteria (task 019f8a93) — mapping

- *Written Phase 2 plan grounded against `main`* → this document; §1 is the evidence.
- *Each drop additive-safe to sequence (readers moved before the column goes)* → §2 PR-A/PR-B split;
  §4 migration order.
- *Operator runbook for temperkb.io + enterprise* → §5.
- *`019f80a9` Slack-token decision folded in* → §0 D-A (answered with reasoning; a test pins it).
- *CI green; Pete runs the migrations per target* → §4 (stop at PR-B green) / §5.
