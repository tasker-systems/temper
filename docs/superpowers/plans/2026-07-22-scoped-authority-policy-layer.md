# `ScopedAuthority` Policy Layer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.
>
> **This plan is an index, not a substitute for the spec.** Per
> `implementation-grounding.md` GD-4, it deliberately contains **no invented code bodies**. Every task
> cites the spec section you must read and the `file:line` you must open. Where a signature appears, it
> is either quoted from disk (CONFORM) or explicitly tagged as new (EXTEND/AMEND). If you find
> yourself writing code this plan "described" without having opened the cited file, stop — that is the
> failure mode the tagging exists to prevent.

**Goal:** Name and finish the scoped-authorization pattern temper already grew three times — one
`ScopedAuthority` trait, one sealed `Authorized<A>` proof that carries its subject, and write
primitives that cannot be reached without a warrant.

**Architecture:** A trait in `temper-services/src/authz/` that each domain's existing authority enum
implements. `resolve` keeps doing its own sequenced SQL probes (SQL predicates stay authoritative —
this routes to them, it does not restate them). One `authorize` function seals `(authority, subject)`
into a proof; acts read the subject *from the proof*, so there is no second spelling to transpose.

**Tech Stack:** Rust 2021, `#[async_trait]`, sqlx/Postgres, cargo-make + cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-07-22-scoped-authority-policy-layer-design.md` — read §1–§3
before Task 1. Read §6 before Task 6. Read §2.3–§2.4 before Task 9.

---

## Global Constraints

- **No DB migration, no `.sqlx` regen from production SQL.** The predicates are *called*, not
  rewritten. If a task adds a **test-target** `query!` macro, run `cargo make prepare-services`
  (per-crate, `--all-targets`) — never `cargo sqlx prepare --workspace` for that.
- **No OpenAPI / temper-rb gem / `schema.ts` regen.** Everything added is `pub(crate)`; no DTO moves.
- **Behavior-preserving.** Every existing test must stay green *unchanged*. A test you had to edit to
  make pass is a behavior change — stop and escalate rather than editing the assertion.
- **No `_ =>` catchall** in any match over an authority enum or over `GrantWarrant`. Adding a variant
  must be a compile error at every decision site. This is the property the layer exists to buy.
- **`#[async_trait]`, not native AFIT** — matches the incumbent `Backend` trait
  (`temper-workflow/src/operations/backend.rs:54`); AFIT forces `Send`-bound gymnastics at the axum
  handlers for no gain.
- **Do not add `trybuild` fixtures for this layer.** Spec §8 explains why they would pass for the
  wrong reason. This is a conclusion, not an oversight.
- **Auth before writes**, always. Never write-then-check.
- Run `cargo make check` before every commit.

## A note on TDD in a behavior-preserving refactor

Most tasks here change *structure*, not *outcomes*, so a conventional red-green cycle would be
theatre — the existing suites already assert the behavior and already pass. The discipline that
actually applies:

1. **Before**: run the task's named suites and record they are green. That is your baseline.
2. **After**: the same suites, still green, **with no test edits**.
3. **Plus**: a new test only where the task creates an *observable new property* (Task 6's decision,
   Task 9's call-site count).

~~Task 6 is the one genuine red-green task — it changes behavior — and it is written that way.~~
**Superseded 2026-07-23: there is no behaviour-changing task in this plan.** Task 6 was written
expecting to resolve the gating-team asymmetry in one direction; grounding found three sites with
three different and individually correct relationships to the gating team, and the resolution was to
record them (spec §6.1). Its new test pins a deliberate *absence* and is green on first run — which
is the correct shape for pinning existing behaviour, not a failed red-green cycle.

---

## File Structure

**Created:**
- `crates/temper-services/src/authz/mod.rs` — the trait, `Authorized<A>`, `authorize()`. Nothing else.
- `crates/temper-services/src/authz/grant.rs` — `impl ScopedAuthority for GrantAuthority`; later
  `GrantWarrant`, `BornSubject`.
- `crates/temper-services/src/authz/machine.rs` — `impl ScopedAuthority for MachineAuthority`.
- `crates/temper-services/src/authz/two_sided.rs` — `TwoSidedAuthority` (PR 2).
- `crates/temper-services/src/authz/connection.rs` — `ConnectionAuthority` (PR 2). Its own file
  rather than a `two_sided.rs` arm: it shares the *shape* but none of the resolver, and the
  one-policy-per-file rule below is what makes that legible.
- `crates/temper-services/src/authz/read_gates.rs` — `TeamReadAuthority`, `ActorHistoryAuthority`.

**Modified (exact sites in each task):** `lib.rs:10` (module decl), `services/access_service.rs`,
`services/machine_authz.rs`, `services/cogmap_service.rs`, `services/context_service.rs`,
`services/connection_service.rs`, `services/team_service.rs`, `services/admin_ledger_service.rs`,
`services/machine_registration_service.rs`, `backend/db_backend.rs`.

**Why split by domain rather than one `authz.rs`:** each file holds one domain's arms + its resolve
probes, so a reviewer reads one policy at a time, and `mod.rs` stays small enough that the contract is
readable in one screen.

---

# PR 1 — Mechanism + incumbents

**Coherence:** zero new policy. The shape is proven on code that already worked. Independently
shippable; behavior identical.

> **Execution log (2026-07-22).** Landed as `6cde7bda`, `b5f5eec6`, `898d4149`, `49025a00`.
> Four deviations from the plan as written, each deliberate:
>
> 1. **Tasks 1 and 2 landed in one commit.** Step 4 predicted the trait would be dead code alone and
>    prescribed a temporary `#![allow(dead_code)]`. Creating an attribute the plan itself warns must
>    be removed is worse than folding the boundary; a trait plus its first impl is one coherent unit.
> 2. **`authorize_capability_grant` routes through `authorize`, not bare `resolve`.** The leftover
>    dead-code warnings on `Authorized`/`authorize` were the tell that calling `resolve` directly was
>    the wrong shape — the function denies on `None` and then branches on the arm, which *is*
>    `authorize` plus a match. It now reads its scope from `proof.subject()`.
> 3. **`RefTarget` reached further than Task 2 assumed, in the useful direction.** The admin ledger
>    already *held* one and was flattening it to `(&str, Uuid)` only because the gate took strings, so
>    typing the gate simplified that call site. `AnchorTable::as_str()`'s doc comment names
>    `can_administer_grant` as its reason for existing; that reason is now gone.
> 4. **`MachineAuthority::None` is unreachable at both consumer matches**, since the `authorize`
>    wrapper refuses denials before returning. Enumerated anyway (no `_ =>`), per Global Constraints.
>
> Two gates the plan did not anticipate: rustdoc's `-D broken-intra-doc-links` rejects `[`links`]` to
> `pub(crate)` items in a private module (they became plain code spans), and `mod authz` is private
> rather than `pub` — every item in it is `pub(crate)`, so `pub mod` would advertise a contract
> nothing outside the crate can use.

---

### Task 1: The `authz` module — trait, proof, gate

**Files:**
- Create: `crates/temper-services/src/authz/mod.rs`
- Modify: `crates/temper-services/src/lib.rs:10` (add `pub mod authz;` — alphabetical, after `auth_config`)

**Read first:** spec §2.1 and §2.2 in full. Then open `crates/temper-services/src/auth/mod.rs` — the
sealed-proof idiom you are mirroring (`SystemAdmin`, its private field, its accessor, and the gate
co-located with it) lives there and is the pattern of record.

**Interfaces — Produces** (later tasks depend on these exact names):
- `pub(crate) trait ScopedAuthority: Sized + Copy + Debug` with `type Subject: Copy + Debug`,
  `async fn resolve(pool: &PgPool, caller: ProfileId, subject: Self::Subject) -> ApiResult<Self>`,
  `fn is_denial(&self) -> bool`, `fn denial() -> ApiError`
- `pub(crate) struct Authorized<A: ScopedAuthority>` — **private** fields `authority: A`,
  `subject: A::Subject`; accessors `authority(&self) -> A`, `subject(&self) -> A::Subject`
- `pub(crate) async fn authorize<A: ScopedAuthority>(pool, caller, subject) -> ApiResult<Authorized<A>>`

- [x] **Step 1 — CONFORM: read the incumbent seal.** Open `crates/temper-services/src/auth/mod.rs`
      and read `SystemAdmin` + `require_system_admin`. Note three things you will copy: the private
      field, the doc comment that says *why* it is private, and the gate living in the same module as
      the type (it must — the private field means only that module can construct it).

- [x] **Step 2 — EXTEND (spec §2.1): write the trait.** Four items, exactly as in the Interfaces block
      above. Each carries a doc comment stating its obligation. `denial()` must carry the reason it
      exists — quote spec §2.1's justification (`team_service.rs:277–279` hides team existence because
      *"team slugs are globally unique and used in share flows"*), so a future reader does not
      "simplify" it to a hardcoded `Forbidden`.

- [x] **Step 3 — EXTEND (spec §2.2): write `Authorized<A>` and `authorize`.** Fields private. The
      doc comment on `subject()` must say it is *the only* subject an act may touch — that sentence is
      the whole point of the type. `authorize` resolves, returns `A::denial()` when `is_denial()`, and
      seals otherwise.

- [x] **Step 4 — Wire the module.** Add `pub mod authz;` to `lib.rs`. Nothing implements the trait
      yet, so expect dead-code warnings; add `#![allow(dead_code)]` **only if** clippy fails, and
      remove it in Task 2 when the first impl lands. Do not leave it behind.

- [x] **Step 5 — Verify.**
      ```bash
      cargo make check
      ```
      Expected: clean. No test run needed — nothing calls this yet.

- [x] **Step 6 — Commit.**
      ```bash
      git add crates/temper-services/src/authz/mod.rs crates/temper-services/src/lib.rs
      git commit -m "authz: the ScopedAuthority trait and the sealed Authorized proof"
      ```

---

### Task 2: `GrantAuthority` onto the trait

**Files:**
- Create: `crates/temper-services/src/authz/grant.rs`
- Modify: `crates/temper-services/src/services/access_service.rs:70` (the enum), `:103`
  (`grant_authority` — its body **moves**, unchanged, into the impl)

**Read first:** spec §3 row 1. Then read `access_service.rs:64–130` in full — the enum's doc comment
explains why the arms carry *why* and not merely *whether*, and `grant_authority`'s body carries the
L0/gating-map escalation guard (`:118`) that a careless move would drop.

**Interfaces — Consumes:** `ScopedAuthority`, `Authorized`, `authorize` (Task 1).
**Interfaces — Produces:** `impl ScopedAuthority for GrantAuthority { type Subject = RefTarget; .. }`

`RefTarget` is **verified on disk** — `temper-substrate/src/payloads.rs:117–121`, already
`#[derive(Debug, Clone, Copy, PartialEq, Eq, ..)]` with `kind: AnchorTable` and `id: Uuid`. It is the
subject type `admin_ledger_service` already threads (`:89` calls
`can_administer_grant(pool, caller, subject.kind.as_str(), subject.id)`). Reuse it; do **not** mint a
parallel pair type.

- [x] **Step 1 — Baseline.** Record green:
      ```bash
      cargo nextest run -p temper-services --features test-db --test admin_ledger_test
      cargo nextest run -p temper-api --features test-db --test access_grants_test
      ```

- [x] **Step 2 — AMEND: `RefTarget` retires the stringly-typed subject.** `grant_authority` currently
      takes `subject_table: &str` and **string-compares** it (`access_service.rs:118`:
      `subject_table == "kb_cogmaps"`). `RefTarget.kind` is the `AnchorTable` enum
      (`payloads.rs:32–51`), whose variants cover every grant subject in use (`Cogmaps`, `Connections`,
      `Resources`, `Contexts`, `Teams`, `Profiles`, …) — verified against every `"kb_*"` subject literal
      in `access_service.rs`, `connection_service.rs`, and `db_backend.rs`. So that comparison becomes a
      typed match, satisfying the repo's no-stringly-typed-matches rule. Keep
      `can_administer_grant`'s existing `&str` seam working by converting at its boundary, so the
      ledger caller (`admin_ledger_service.rs:89`) is untouched by this task.

- [x] **Step 3 — CONFORM: move, do not rewrite.** Move `grant_authority`'s body (`:103–130`) into
      `resolve` **verbatim**, changing only the signature. The three branches — admin short-circuit,
      L0/gating guard, `profile_can_grant` — must survive in order. The short-circuit is load-bearing
      (spec D3: 1 query for an admin, 3 for a denied delegate); do not reorder it.

- [x] **Step 4 — AMEND: `is_denial` / `denial`.** `is_denial` is `matches!(self, GrantAuthority::None)`.
      `denial()` is `ApiError::Forbidden` — verified at `access_service.rs:391` and the two sinks.

- [x] **Step 5 — Keep `can_administer_grant` working.** It is the seam `admin_ledger_service` calls
      (`:89`) precisely so the read gate cannot drift from the write gate. Re-point it at the trait;
      **do not delete it and do not inline it into the ledger** — spec §1 quotes why.

- [x] **Step 6 — Verify.** Both suites from Step 1, still green, **no test edits**. Plus `cargo make check`.

- [x] **Step 7 — Commit.** `git commit -m "authz: GrantAuthority implements ScopedAuthority"`

---

### Task 3: `MachineAuthority` onto the trait

**Files:**
- Create: `crates/temper-services/src/authz/machine.rs`
- Modify: `crates/temper-services/src/services/machine_authz.rs:57` (enum), `:68` (`authorize`)

**Read first:** spec §3 row 2. Then `machine_authz.rs:55–86`. Note the fail-closed rule stated there:
*"a teamless machine (`team_id IS NULL`) is admin-only… 'No team to check' must never mean 'nothing to
deny'."* That behavior must survive exactly.

**Interfaces — Produces:** `impl ScopedAuthority for MachineAuthority { type Subject = Option<Uuid>; .. }`
plus a **new `None` arm** on the enum.

- [x] **Step 1 — Baseline.**
      ```bash
      cargo nextest run -p temper-services --features test-db --lib machine_authz
      ```

- [x] **Step 2 — AMEND (spec §2.2): add the `None` arm.** Today denial is `Err(ApiError::Forbidden)`
      returned from inside `authorize` (`:79`, `:85`). That bypasses `denial()`. Add `MachineAuthority::None`
      and have `resolve` return it instead of erroring.

- [x] **Step 3 — CONFORM: find every match on `MachineAuthority`.** `rg -n "MachineAuthority::" --type rust crates/`
      — each match must gain a `None` arm, with **no `_ =>` catchall** (Global Constraints).
      `contain_target_team` (`:180`) is one; it currently matches two arms exhaustively.

- [x] **Step 4 — Preserve fail-closed.** The `Option<Uuid>` subject with `None` ⇒ deny must be a branch
      in `resolve`, not an accident of a missing arm. Read `:78–81` before you move it.

- [x] **Step 5 — Verify.** Step 1's suite green, no edits. `cargo make check`.

- [x] **Step 6 — Commit.** `git commit -m "authz: MachineAuthority implements ScopedAuthority, denial becomes an arm"`

---

### Task 4: `TeamReadAuthority` — the first `NotFound` domain

**Files:**
- Create: `crates/temper-services/src/authz/read_gates.rs`
- Modify: `crates/temper-services/src/services/team_service.rs:280–286`

**Read first:** spec §3 row 5 and §2.1's `denial()` justification. Then `team_service.rs:276–286` —
the doc comment states the information-hiding intent; carry it onto `denial()` so it cannot be
"simplified" later.

**Interfaces — Produces:** `impl ScopedAuthority for TeamReadAuthority { type Subject = Uuid; .. }`,
arms `Member | SystemAdmin | None`, `denial() -> ApiError::NotFound`.

- [x] **Step 1 — Baseline.** `cargo nextest run -p temper-api --features test-db --test team_lifecycle_test`

- [x] **Step 2 — CONFORM: read the existing gate.** `team_service.rs:282–285` — `is_member` (any role)
      OR `is_system_admin`, else `NotFound`. Two probes, and `role_on_team` is called for the member
      check; reuse it, do not write new SQL.

- [x] **Step 3 — EXTEND: the impl.** Three arms so the *reason* survives (member vs admin), matching
      how `GrantAuthority` carries why-not-just-whether (`access_service.rs:64`).

- [x] **Step 4 — Re-point `team_detail`** to `authorize::<TeamReadAuthority>`. The `NotFound` must come
      from `denial()`, not from a hand-written error at the call site.

- [x] **Step 5 — Verify.** Step 1's suite green, no edits. Confirm a non-member still gets **404, not
      403** — that is the whole point of this domain. `cargo make check`.

- [x] **Step 6 — Commit.** `git commit -m "authz: team read gate onto ScopedAuthority (NotFound domain)"`

---

### Task 5: `ActorHistoryAuthority` — the ledger actor axis

**Files:**
- Modify: `crates/temper-services/src/authz/read_gates.rs`,
  `crates/temper-services/src/services/admin_ledger_service.rs:181`

**Read first:** spec §3, including the blockquote **"`list_by_actor` carries two checks, and only the
second one migrates."** Read it before touching anything — folding the standing check in is the exact
mistake the note exists to prevent.

**Interfaces — Produces:** `impl ScopedAuthority for ActorHistoryAuthority { type Subject = ProfileId; .. }`,
arms `SelfActor | SystemAdmin | None`, `denial() -> ApiError::NotFound`.

- [x] **Step 1 — Baseline.** `cargo nextest run -p temper-services --features test-db --test admin_ledger_test`

- [x] **Step 2 — CONFORM: read both checks.** `admin_ledger_service.rs:176` is `has_system_access` — a
      **standing** question. `:181` is `caller != actor && !is_system_admin` — the scoped one.
      **Only `:181` migrates.** Leave `:176` exactly where it is and do not fold it into the resolve.

- [x] **Step 3 — EXTEND: the impl.** `SelfActor` when `caller == subject`; `SystemAdmin` when the
      governance probe holds; else `None`. Note the ordering: `caller == actor` is free (no query), so
      it goes first — a self-read must not cost a DB round-trip.

- [x] **Step 4 — Re-point `list_by_actor`'s second check** to `authorize::<ActorHistoryAuthority>`.

- [x] **Step 5 — Verify.** Step 1's suite green, no edits. `cargo make check`.

- [x] **Step 6 — Commit + open PR 1.**
      ```bash
      git commit -m "authz: ledger actor axis onto ScopedAuthority"
      cargo make check && cargo make test-db
      gh pr create --title "ScopedAuthority: the mechanism, on the incumbents" --base main
      ```
      **STOP.** PR 1 must merge before PR 2 branches (spec §9 — sequential off `main`, not stacked).

---

# PR 2 — Collapse the two-sided gates

**Branch fresh off `main` after PR 1 merges.**
**Coherence:** one policy where there were three, **in Rust** (spec §6.2 — `context_reassign`'s plpgsql
copy is a fourth, deliberately out of reach). This is where spec §6's asymmetry becomes a decision.

> **Execution log (2026-07-23).** Branch `jct/scoped-authority-pr2`, four commits: the spec/plan
> amendment, then Tasks 6, 7, 8. Behavior-preserving throughout; **zero test edits** across
> temper-services (37 connection + 11 context), temper-api, and the e2e bind/share/transfer suites.
>
> 1. **Task 6 stopped being a behaviour change**, and that was the substantive finding of this PR.
>    Its premise — one asymmetry, pick a direction — did not survive grounding: `can_bind`'s guard is
>    structural (and its load-bearing direction is *unbind*, which its comment never said),
>    `can_share`'s guard outlived the D11 change that invalidated its stated reason, and
>    `contain_target_team` correctly has none. Pete chose no behaviour change; the deliverable became
>    the three reasons plus a test pinning the deliberate absence. **The plan now contains no
>    behaviour-changing task at all.** Spec §6.1 records it.
> 2. **Spec §6's table missed a fourth instance, and it is in plpgsql.** `context_reassign` carries
>    the whole policy independently. It bounds what the collapse may claim (§6.2) and is why
>    `can_share`'s gating-team exclusion still has a reason to exist at all.
> 3. **`TwoSidedObject`, not `RefTarget`.** Task 7's interface line specified
>    `type Subject = (RefTarget, Uuid)`. `RefTarget` has four variants and two of them cannot occur
>    here, so the impl would have owed an answer for resources and connections whose only honest form
>    is a silent denial. A closed two-arm enum makes the match exhaustive over what is real.
> 4. **`ConnectionScope`/`TwoSidedScope` are named structs, not tuples.** Task 8 specified
>    `(Uuid, Uuid)`. Two same-typed ids in positional order is precisely the transposition this layer
>    exists to remove — the tuple type-checks with the arguments swapped.
> 5. **`ConnectionAuthority` maps one error variant back to a denial arm.** `contain_target_team`
>    refuses with `Forbidden` but reports an absent target team with `NotFound`. Only the refusal
>    becomes an arm; `NotFound` propagates. Folding both would turn "that team does not exist" into
>    "you may not" — a behaviour change pinned by an existing test.
> 6. **Task 8 touches `grant_reach` only.** The plan's `:480,503` line refs point at the two
>    `insert_grant` sites, which are PR 3's business. `revoke_reach` deliberately asks only the first
>    of the two questions and is not this gate's shape.
>
> The PR-1 rustdoc gate recurred exactly as logged (`-D broken-intra-doc-links` on `pub(crate)`
> items), costing one `cargo make check` cycle. The note was right; reading it earlier would have
> saved the cycle.

---

### Task 6: Record the three reasons behind the gating-team asymmetry

> **AMENDED 2026-07-23 — this was written as "the one genuine red-green task."** It is not one. PR 2
> grounding (spec §6.1) found three sites whose gating-team relationship genuinely differs: `can_bind`'s
> guard is structurally load-bearing via `cogmap_write_requires_admin` and its *unbind* direction is the
> one nobody wrote down; `can_share`'s guard survives but its stated rationale was invalidated by D11
> and the real footing is the `reassign` path's plpgsql twin; `contain_target_team` correctly has none.
> Pete's call (2026-07-23): **no behaviour change at any of the three.** The deliverable is the reasons,
> recorded where each gate lives, plus a test pinning the deliberate absence.
>
> **The plan now contains no behaviour-changing task.** The whole three-PR arc is behaviour-preserving.

**Files:**
- Modify: `crates/temper-services/src/services/cogmap_service.rs:9-13,58-61` (module header + `can_bind` doc)
- Modify: `crates/temper-services/src/services/context_service.rs:364-369` (`can_share` doc)
- Modify: `crates/temper-services/src/services/machine_authz.rs:172-181` (`contain_target_team` doc)
- Test: the connection-reach suite (locate with `rg -n "contain_target_team|grant_reach" -g '*.rs' crates/*/tests/ tests/e2e/tests/`)

**Read first:** spec §6 **in full** — §6.1 for the three reasons verbatim, §6.2 for the plpgsql fourth
instance. Do not proceed to Task 7 until the comments are written; collapsing first is how the reasons
get lost in the move.

- [x] **Step 1 — ESCALATED and decided.** Presented 2026-07-23 with the live predicate bodies as
      evidence. Outcome: option "no behaviour change; record three reasons." Recorded in spec §6.1.

- [x] **Step 2 — CONFORM: `can_bind`'s comment gains the unbind direction.** The existing text names
      only binding ("binding there flips the map into the `require_cogmap_write_admin` regime"). Add
      that `can_bind` also gates `unbind_team` (`:121`), so the guard's sharper purpose is preventing a
      non-admin from **un**binding a protected map out of the admin-write regime. Both the module
      header (`:9-13`) and the fn doc (`:58-61`) carry the claim; update both or they drift.

- [x] **Step 3 — AMEND: `can_share`'s rationale is replaced, not deleted.** Strike *"sharing into the
      root team is an instance-level escalation"* — quote the live `has_system_access` /
      `is_system_admin` bodies from spec §6.1 as the reason it is stale. Replace with the footing that
      does hold: `can_share` also gates `reassign` (`:505`), a transfer of ownership into the root team
      that `context_reassign`'s plpgsql independently forbids, so relaxing the Rust half would split the
      two copies into different error paths. Cite spec §6.1 and §6.2.

- [x] **Step 4 — EXTEND: pin `contain_target_team`'s deliberate absence.** Its doc gains a paragraph
      stating the absence is a decision (a reach grant confers READ on the granter's own connection
      data; it flips no regime), and a test asserts it: a non-admin team owner grants connection reach
      to the gating team and **succeeds**. This test must be **GREEN on first run** — it pins current
      behaviour. A red here means the behaviour is not what §6.1 grounded; stop and escalate.

- [x] **Step 5 — Verify.** New test green. The three tests pinning the live exclusions
      (`bind_cogmap_e2e.rs:424`, `context_share_e2e.rs:437`, `context_service.rs:946`) green,
      **no edits** — needing to edit one is the signal the direction was wrong.
      ```bash
      cargo nextest run -p temper-services --features test-db --lib context_service
      cargo make test-e2e   # bind_cogmap_e2e + context_share_e2e
      ```

- [x] **Step 6 — If the test added a `query!` macro:** `cargo make prepare-services`.

- [x] **Step 7 — Commit.** `git commit -m "authz: the three reasons behind the gating-team asymmetry"`

---

### Task 7: `TwoSidedAuthority` — collapse `can_bind` + `can_share`

**Files:**
- Create: `crates/temper-services/src/authz/two_sided.rs`
- Modify: `services/cogmap_service.rs:62` (`can_bind`), `services/context_service.rs:370` (`can_share`)

**Read first:** spec §3 row 3, §6's table, and **§6.2**. Then read **both** gates side by side —
`cogmap_service.rs:58–81` and `context_service.rs:364–390`. They differ in exactly one probe.

> **§6.2 bounds this task's claim.** `context_reassign`
> (`migrations/20260715000010_context_reassign_fns.sql:77-93`) is a **fourth** copy of this policy, in
> plpgsql, that the collapse cannot reach — deliberately, since it is the atomic enforcement behind the
> Rust pre-check (`context_service.rs:629`). The commit message and PR body say **"one policy where
> there were three, in Rust"**. Do not write "one copy".

**Interfaces — Produces:** `impl ScopedAuthority for TwoSidedAuthority { type Subject = (RefTarget, Uuid); .. }`
— the subject is the **pair**, which is what closes the transposition hazard (spec §2.2).

- [x] **Step 1 — Baseline.**
      ```bash
      cargo nextest run -p temper-api --features test-db --test cogmap_authz_test
      cargo make test-e2e   # covers context_share_e2e + bind_cogmap_e2e
      ```

- [x] **Step 2 — CONFORM: identify the single difference.** Subject-administration is
      `profile_can_grant(pool, caller, "kb_cogmaps", id)` for bind, and
      `caller_administers_context(pool, caller, id)` for share. Everything else — admin short-circuit,
      gating-team exclusion, `can_manage` on the target team — is the same policy twice. Parameterize
      **only** that probe.

- [x] **Step 3 — EXTEND: the shared resolver.** Carry both gates' doc-comment rationale onto it; those
      comments are the record of *why* the gating-team exclusion exists and must not be lost in the move.

- [x] **Step 4 — Re-point both call sites.** `cogmap_service.rs:34` and `:121`; `context_service.rs:436`,
      `:469`, `:505`. Each currently maps `false → Err(Forbidden)`; that mapping now comes from
      `denial()`.

- [x] **Step 5 — Verify.** Both suites green, no edits. `cargo make check`.

- [x] **Step 6 — Commit.** `git commit -m "authz: one TwoSidedAuthority replaces can_bind and can_share"`

---

### Task 8: `ConnectionAuthority`

**Files:**
- Modify: `crates/temper-services/src/authz/two_sided.rs` (or a sibling if it reads better),
  `services/connection_service.rs:480,503`

**Read first:** spec §3 row 4, and `connection_service.rs:409–430` — the doc comment explains why this
path **must not** route through grant authority: *"the `can_grant` seam has no bootstrap holder for a
connection subject."* Preserve that; do not "unify" it into `GrantAuthority`.

**Interfaces — Produces:** `impl ScopedAuthority for ConnectionAuthority { type Subject = (Uuid, Uuid); .. }`
— `(connection_id, target_team_id)`. Needed by Task 11's `GrantWarrant::ConnectionReach`.

- [x] **Step 1 — Baseline.** `cargo nextest run -p temper-services --features test-db --lib connection`
      plus `cargo nextest run -p temper-api --features test-db --test connection_reach_grant_equivalence_test`

- [x] **Step 2 — CONFORM: the two questions.** The doc comment names them explicitly — *"May you act on
      this connection?"* (`machine_authz::authorize` on the connection's owning team) and *"May you
      hand read-reach to THAT team?"* (`contain_target_team`). Both, in that order.

- [x] **Step 3 — EXTEND: the impl**, composing the two existing calls. Call them; do not restate them.

- [x] **Step 4 — Verify.** Suite green, no edits. `cargo make check`.

- [x] **Step 5 — Commit + open PR 2.**
      ```bash
      cargo make check && cargo make test-db
      gh pr create --title "ScopedAuthority: collapse the two-sided gates" --base main
      ```
      **STOP.** PR 2 merges before PR 3 branches.

---

# PR 3 — Seal the write primitives

**Branch fresh off `main` after PR 2 merges.**
**Coherence:** the enclosure spec's explicitly deferred write-primitive deepening, collected.

---

### Task 9: `BornSubject` and its call-site-count test

**Files:**
- Modify: `crates/temper-services/src/authz/grant.rs`
- Test: same file, `#[cfg(test)] mod tests`

**Read first:** spec §2.4 **including its honest limit** — `BornSubject` cannot prove freshness. Do not
write a doc comment claiming it can.

**Interfaces — Produces:** `pub(crate) struct BornSubject<S: Copy>` with private `subject: S`,
accessor `subject(&self) -> S`, and one `pub(crate) fn` constructor whose name reads as a claim.

- [ ] **Step 1 — EXTEND: the type.** Private field. The doc comment states the claim (*"the subject is
      being born in this transaction; no prior authority over it can exist because it did not exist"*)
      **and** the limit (it cannot verify that claim).

- [ ] **Step 2 — Write the call-site-count test.** Model it on
      `temper-principal/src/admission.rs:102–109` (`admit_reads_standing_and_nothing_else`) — read that
      test first; its value is entirely in the comment explaining why the number matters and that the
      fix is never "bump the number." Count constructions with
      `rg -c "BornSubject::<constructor-name>" crates/temper-services/src/`.

- [ ] **Step 3 — Run it.** Expected: PASS at the count you just measured (1, after Task 11 wires
      genesis). Until Task 11 lands the count is 0 — that is correct, not a failure; the test moves
      with the code.

- [ ] **Step 4 — Commit.** `git commit -m "authz: BornSubject confines the genesis exception"`

---

### Task 10: Per-item sealing — `AuthorizedGrant`

**Files:**
- Modify: `crates/temper-services/src/services/machine_authz.rs:93` (`AuthorizedReach`),
  `services/machine_registration_service.rs:133`

**Read first:** spec §2.3's final paragraph — *why* the warrant takes `&AuthorizedGrant` and not
`&AuthorizedReach`. Then read `AuthorizedReach` (`machine_authz.rs:93–110`) — it is the incumbent
sealed proof and the model for everything in this PR.

**Interfaces — Produces:** `AuthorizedReach::grants() -> &[AuthorizedGrant]` where `AuthorizedGrant`
is sealed and carries its own `cogmap_id`.

- [ ] **Step 1 — Baseline.** `cargo nextest run -p temper-services --features test-db --lib machine_registration`

- [ ] **Step 2 — CONFORM: read the loop.** `machine_registration_service.rs:133` — `for grant in
      reach.grants()`. The per-row subject comes from the item, which is why sealing the item (rather
      than checking membership at runtime) makes it structural.

- [ ] **Step 3 — AMEND: seal the item.** `AuthorizedGrant` with a private field, constructible only
      where `AuthorizedReach` is. `grants()` returns `&[AuthorizedGrant]`.

- [ ] **Step 4 — Verify.** Suite green, no edits. `cargo make check`.

- [ ] **Step 5 — Commit.** `git commit -m "authz: seal each authorized grant, not just the reach"`

---

### Task 11: `GrantWarrant` and the primitive signature change

**Files:**
- Modify: `crates/temper-services/src/authz/grant.rs` (the enum),
  `services/access_service.rs:289` (`insert_grant`), `:320` (`delete_grant`),
  and all five call sites: `access_service.rs:365`, `machine_registration_service.rs:134`,
  `connection_service.rs:480`, `:503`, `backend/db_backend.rs:2225`

**Read first:** spec §2.3 in full, including the four-row caller table and *why* the four gates
differing is correct rather than sloppy.

**Interfaces — Consumes:** `Authorized<GrantAuthority>` (Task 2), `Authorized<ConnectionAuthority>`
(Task 8), `AuthorizedGrant` (Task 10), `BornSubject` (Task 9).
**Interfaces — Produces:** `pub(crate) enum GrantWarrant<'a>` with exactly four arms (spec §2.3) and
`fn subject(&self) -> RefTarget`.

- [ ] **Step 1 — Baseline.** `cargo make test-db` (full workspace — this task touches five call sites
      across four services; a targeted run would miss a regression).

- [ ] **Step 2 — EXTEND: the enum.** Four arms, each doc-commented with *which gate mints it*. The
      enum's own doc comment is the enumerable policy — quote spec §2.3's framing: *"The COMPLETE set of
      ways a `kb_access_grants` row may be born."*

- [ ] **Step 3 — AMEND: `subject()` with no catchall.** Match all four arms. A `_ =>` here would defeat
      the compile-error-on-a-fifth-way property (Global Constraints).

- [ ] **Step 4 — AMEND: the primitives.** `insert_grant` takes the warrant and **drops
      `subject_table`/`subject_id` from `InsertGrantParams`** — reading them from `warrant.subject()`.
      Same for `delete_grant`. If you find yourself passing a subject *alongside* a warrant, stop: that
      reintroduces the second spelling this whole PR exists to remove.

- [ ] **Step 5 — Migrate the five call sites**, each to its own arm per spec §2.3's table. The
      `db_backend.rs:2225` genesis seed is the `Birth` arm — read `:2210–2224` first; the comment there
      explains the emitter and capability choices, which do not change.

- [ ] **Step 6 — Verify.** `cargo make test-db` green, **no test edits**. Task 9's count test now
      expects 1. `cargo make check`.

- [ ] **Step 7 — Commit.** `git commit -m "authz: kb_access_grants writes require a warrant"`

---

### Task 12: Narrow the surface, and sweep

**Files:** `services/access_service.rs:289,320`; whole-crate sweep.

- [ ] **Step 1 — Narrow `pub` → `pub(crate)`** on `insert_grant` and `delete_grant`. Verify no external
      caller first: `rg -n "insert_grant|delete_grant" --type rust crates/ | grep -v temper-services`
      — expected: no hits.

- [ ] **Step 2 — Sweep for restated policy.** `rg -n "is_system_admin" crates/temper-services/src/services/`
      and confirm every remaining production hit is one of the sites spec §3 lists as deliberately out
      of scope (filter-shaped, projection-shaped, conditional, `get_entitlements`). **Any hit not on
      that list is a site this plan missed** — report it, do not quietly migrate it.

- [ ] **Step 3 — Full verification.**
      ```bash
      cargo make check
      cargo make test-db
      cargo make test-e2e
      ```

- [ ] **Step 4 — Commit + open PR 3.**
      ```bash
      gh pr create --title "ScopedAuthority: seal the grant write primitives" --base main
      ```

---

## Out of scope — do not do these while in here

Spec §3 lists them with reasons; repeated so an implementer does not "helpfully" pick one up:

- `connection_service::list` / `machine_client_service::list` (filter-shaped — file separately)
- `admin_ledger_service::readable_event_types` (projection-shaped, already a good citizen)
- `team_service::create`'s `auto_join_role` (conditional/parametric — a required proof breaks the base op)
- `slack_disconnect_service::admin_disconnect_slack_principal` (spec §7 — belongs to the **enclosure's**
  `&SystemAdmin` pattern, not this one; needs its own task)
- Team-membership writes (spec §10 — real follow-up, not this PR)
