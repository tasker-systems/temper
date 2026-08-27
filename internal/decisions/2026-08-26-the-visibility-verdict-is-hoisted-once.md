# The visibility verdict is computed once and carried, and discipline is what holds it

**Date:** 2026-08-26
**Status:** Decided — accepted residue, recorded
**Scope:** the five `__temper_ungated_*` SQL functions and the single emitter that calls them
**Task:** `01a035f2-d37a-7a83-9f6c-b93d58eb5847`

## Decision

Five SQL functions under the `__temper_ungated_` prefix apply **no visibility gate of their own**.
They take the RBAC verdict as `p_visible_ids uuid[]` — the first argument in every one — and trust
their caller absolutely:

```
__temper_ungated_find_exact
__temper_ungated_find_resources_with
__temper_ungated_find_wide
__temper_ungated_follow_from
__temper_ungated_survey
```

The invariant *"every mechanic acts only on resources visible to the principal"* is therefore **not
a property of these function bodies**. It is held by two things and nothing else: a single private
emitter, and a CI tripwire.

This is accepted as a **named residue**, not treated as a defect.

## Why

A multi-stage `/api/query` composition must compute `resources_visible_to` **once** — hoisted into
the `__temper_vis` CTE — rather than once per stage. The planner does not dedupe that function
across call sites, so an N-stage composition would otherwise pay N recursive team closures.

The realistic bug this shape invites is not a rogue call site. It is an *approved* one passing an
upstream stage's ids where the visible set belongs: CI green, RBAC bypassed, and every returned row
still plausible. That failure is closed **structurally rather than by review** — `VISIBLE_IDS` and
`PRINCIPAL_BIND` are module constants written inside `emit_ungated_core_call`, and neither is a
field of `CoreCall`. There is no wrong set to pass because there is no argument for it.

A second emitter was rejected on this ground: two emitters are two places, and the second is the one
nobody audits.

Two details are worth stating because they are confusable:

- `p_visible_ids` NULL **admits nothing** (fail-closed). That is the opposite polarity from
  `p_bound_ids`, which sits beside it in `CoreCall::Walk` and where NULL means *unbounded*.
- `__temper_ungated_survey` breaks the single-authorization-argument shape: it takes
  `p_visible_ids` **then** `p_principal`, because `wayfind_region_scores` runs its own region gate
  by principal. Any claim of the form *"every ungated core takes only `p_visible_ids`"* is false.

The authorization that cannot ride in an id set — anchor readability, one boolean per call and a
property of no row — is kept **inside** the cores via `p_anchor_reader` / `anchor_readable_by_profile`,
so a core cannot be lied to about it.

## What this is not

**It is not a database permission.** There are zero `GRANT`, `REVOKE`, or `CREATE ROLE` statements
anywhere under `migrations/`. Postgres grants `EXECUTE` on functions to `PUBLIC` by default and
nothing here narrows it, so **every role in the database can call these functions with an arbitrary
`uuid[]` and receive ungated rows**. `REVOKE` would buy nothing today because the application
connects as the owning role.

`migrations/20260808000030_composable_find_family.sql` says so in its own header, and
`COMMENT ON FUNCTION __temper_ungated_survey(...)` repeats it at the object level:

> The `__temper_ungated_` prefix is source discipline enforced by audit-ungated-fragments.sh, NOT a
> database permission.

That is the residue. It is owned, not mitigated.

## How it is enforced

Three mechanisms, and the limits of each:

1. **The single private emitter** — `fn emit_ungated_core_call` in
   `crates/temper-substrate/src/readback/query_plan.rs`. Both `const VISIBLE_IDS` and
   `const PRINCIPAL_BIND` are module constants written by the emitter itself; `enum CoreCall` has no
   field for either.
2. **`.github/scripts/audit-ungated-fragments.sh`** — freezes **four** derived sets against inline
   baselines: SQL function names, migration files naming the prefix, migrations redefining a
   relation an ungated body reads, and per-file counts of Rust sites. It fails on an empty scan
   rather than passing vacuously. Run in CI via `code-quality.yml` → `rust-quality`, and again
   inside the ungated `guard-tests` job through its own harness
   (`.github/scripts/test-audit-ungated-fragments.sh`, 14 assertions across 6 probe groups,
   including a deliberate green probe that a comment-only mention must not move the count).
3. **`every_ungated_core_call_takes_its_ids_from_the_hoisted_relation_and_nothing_else`**
   (`crates/temper-substrate/tests/query_plan_compile.rs`) — compiles a 4-stage composition covering
   all four `CoreCall` variants and asserts on a written-out literal rather than the imported
   constant, so the assertion cannot agree with a wrong value.

**What none of them proves:**

- **The guard pins *where* a core is called. It can never pin *what is passed*.** Its own header
  says this.
- **The Rust half is prefix-derived**, so a file referencing a core through its exported constant
  (`EMIT_FIND_WIDE`, `EMIT_SURVEY` — as `crates/temper-services/src/backend/query_read.rs` does) is
  invisible to it. That use is benign, but the guard cannot distinguish benign from not.
- **The SQL relation watch is textual**, so its field of view is narrower than its subject.
- **Test trees are out of scope by design**, so a test calling a core directly is expected and
  invisible here.
- The audit **runs** in CI. Whether it is a *required* status check is branch-protection
  configuration, not a repo fact, and is not established here.

## A stale precision claim, corrected

`emit_ungated_core_call` carries a `[corrected — 2026-08-14]` note enumerating two further uses of
`VISIBLE_IDS`/`PRINCIPAL_BIND` (the `__temper_vis` CTE and `unusable_tally`) and asserting *"neither
is a core-call ARGUMENT POSITION."*

**That enumeration is now incomplete for `PRINCIPAL_BIND`.** The `find-resources-with` narrowing
builder in the same file renders `OwnerSlot::Principal` as `format!("{PRINCIPAL_BIND}::uuid")`, and
that string is joined into `narrowings`, which `CoreCall::Selection` renders **into the core call's
argument list**, filling `p_owner_profile`.

Security impact is nil — `p_owner_profile` is a narrowing whose NULL narrows nothing, and
`VISIBLE_IDS` is untouched by this path. But the note's precision claim is stale, and a guard
credited with coverage it does not have is the exact failure mode this tree elsewhere names.

## Revisiting

Re-review is forced when any of these becomes true:

1. A sixth `__temper_ungated_*` function name, **or a new arity of an existing one** — the
   name-keyed baseline does *not* move on a new arity, and `migrations/20260817000020_follow_from_offset.sql`
   is the standing precedent for that happening.
2. A second emitter, or `VISIBLE_IDS` becoming a `CoreCall` field or an `emit_ungated_core_call`
   parameter.
3. `PRINCIPAL_BIND` gaining a use beyond anchor readability and the `@me` owner narrowing — the
   constant's own doc says it *"must never gain another use."*
4. A migration redefining a relation an ungated body reads.
5. **Deploying to any topology where the runtime role is not the owning role.** At that point
   `REVOKE` starts buying something, and this accepted residue should be re-priced rather than
   re-affirmed.
6. Any core gaining a second id set, or `p_principal` moving out of survey's second position.
