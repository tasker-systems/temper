# `/api/query` door — PRs A0 and A implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `temper search` page, then split query validation into a pass that cannot go stale and a pass that can, promote twelve string refusals to variants before anything publishes them, and stop `follow-from`/`survey` from validating clean and failing at execution.

**Architecture:** Two PRs. **A0** adds one clap flag and follows the tripwire test that fires when it appears. **A** restructures `crates/temper-core/src/types/query/validate.rs` (816 production lines, 1,227 test lines) into a `shape` module that may not consult act declarations and a `capability` module that may, guards that seam twice, promotes the twelve `RefusalReason::Other(_)` strings, and removes the two placeholder rows from `CALLABLE_FRAGMENTS`.

**Tech Stack:** Rust, clap 4 (derive), serde, cargo-nextest, cargo-make.

**Spec:** [`docs/superpowers/specs/2026-08-12-api-query-door-design.md`](../specs/2026-08-12-api-query-door-design.md). Read ⟨3⟩, ⟨4⟩ and ⟨5⟩ before Task 2.

## Global Constraints

- **Never scope a test with `--workspace`** — it hangs on bin-target enumeration. Always `-p <crate>`, and prefer `--test <target>` too.
- **`cargo make check` before claiming any task complete.** It gates the generated artifacts.
- **`cmd | tee log` reports tee's exit code** — a red gate notifies as green. Use `set -o pipefail` or do not pipe.
- **This plan touches no SQL and no migration**, so no `.sqlx` regeneration is required in either PR.
- **`validate()` must keep returning EVERY refusal, never the first.** That promise is why `ReturnSpec.with` uses the shared `ResourceSection` vocabulary instead of a narrow enum — a serde failure short-circuits before validation runs (`validate.rs:769-772`).
- **A refusal must never become an existence oracle.** No refusal detail may disclose whether a resource, context or cogmap exists.
- Type regeneration: Task 3 changes serialized values, so it must run `cargo make test-schema-core` (snapshot fixtures) and `cargo make generate-ts-types`, both committed in the same commit.

---

# PR A0 — `temper search --offset`

Independent of the door and pre-existing. `/api/search` the **route** has always accepted offset (`SearchParams.offset`, `crates/temper-core/src/types/api.rs:63-64`), which is why `Door::Api` and `Door::Mcp` already declare `terms_unreachable: []`. The gap is exactly one missing clap flag.

### Task 1: `temper search` gains `--offset`, and the declarations follow the tripwire

**Files:**
- Modify: `crates/temper-cli/src/actions/search.rs:25-36` (`CliSearchArgs`), `:68-78` (`SearchParams` construction)
- Modify: `crates/temper-cli/src/cli.rs:300-306` (the `Search` variant)
- Modify: `crates/temper-cli/src/main.rs:1304-1331` (`Commands::Search` arm)
- Modify: `crates/temper-cli/src/commands/search_cmd.rs:41-51` (`run`'s re-bundle)
- Modify: `crates/temper-core/src/types/query/registry.rs:175, :209, :234` (three `unified_doors` calls), `:737` (`the_cli_cannot_page_the_find_acts_and_that_is_declared`)
- Modify: `crates/temper-cli/tests/act_door_coverage_cli_terms.rs` (`temper_search_still_cannot_page`)

**Interfaces:**
- Produces: `CliSearchArgs { …, pub offset: Option<i64> }` — used by nothing outside `temper-cli`.
- **No `temper-client` change.** It takes `SearchParams` whole (`client.search().search_with_params(&params)`), so a new field needs no plumbing there.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block in `crates/temper-cli/src/actions/search.rs` (if the file has no test module, create one at the end with `#[cfg(test)] mod tests { use super::*;`):

```rust
#[test]
fn the_cli_offset_reaches_search_params() {
    // `temper search` could only ever read page 1. `SearchParams.offset` existed the whole
    // time — this asserts the CLI now fills it, which is the only half that was missing.
    let params = build_search_params(CliSearchArgs {
        query: "anything",
        embedding: None,
        context: None,
        cogmap: &[],
        doc_type: None,
        limit: Some(10),
        offset: Some(20),
    })
    .expect("a query with no anchor conflict builds");
    assert_eq!(params.offset, Some(20));
    assert_eq!(params.limit, Some(10), "limit must not be displaced by the new field");
}
```

- [ ] **Step 2: Run it and confirm it fails**

```bash
cargo nextest run -p temper-cli --lib the_cli_offset_reaches_search_params
```

Expected: **compile error** — `CliSearchArgs` has no field `offset`. A compile error is the correct failure here; the field genuinely does not exist.

- [ ] **Step 3: Add the field and fill it**

In `crates/temper-cli/src/actions/search.rs`, add to `CliSearchArgs` after `limit`:

```rust
    pub limit: Option<i64>,
    /// Page offset. `/api/search` has always accepted one (`SearchParams.offset`); this door
    /// simply had no flag for it, which is what `door_coverage`'s CLI term axis recorded.
    pub offset: Option<i64>,
```

and in `build_search_params`'s returned struct, beside `limit`:

```rust
        limit: args.limit,
        offset: args.offset,
        ..SearchParams::default()
```

- [ ] **Step 4: Run the test and confirm it passes**

```bash
cargo nextest run -p temper-cli --lib the_cli_offset_reaches_search_params
```

Expected: PASS.

- [ ] **Step 5: Add the flag and thread it**

`crates/temper-cli/src/cli.rs`, in the `Search` variant after `limit`:

```rust
        /// Maximum results (default 10)
        #[arg(long)]
        limit: Option<i64>,
        /// Skip this many results. Applied per arm — the exact and wide arms page
        /// independently, because their quantities are incommensurable.
        #[arg(long)]
        offset: Option<i64>,
```

`crates/temper-cli/src/main.rs`, `Commands::Search` — add `offset,` to the destructuring pattern (after `limit,`) and `offset,` to the `CliSearchArgs` literal (after `limit,`).

`crates/temper-cli/src/commands/search_cmd.rs`, inside `run`'s re-bundle — add `offset: args.offset,` after `limit: args.limit,`.

- [ ] **Step 6: Run tier 2 and watch the tripwire fire — this red is the point**

```bash
cargo nextest run -p temper-cli --test act_door_coverage_cli_terms
```

Expected: `temper_search_still_cannot_page` **FAILS** with its own instructions:

> `temper search` has gained --offset. Every find act's CLI `terms_unreachable` must drop `Offset`, and `the_cli_cannot_page_the_find_acts_and_that_is_declared` in registry.rs must be updated with it — that test compares the declaration to a literal and will not notice on its own.

`the_cli_term_shortfall_is_what_clap_actually_lacks` should fail too, since the derived set no longer matches the declared one. **If neither fails, stop** — the flag is not reaching clap's tree and nothing downstream is trustworthy.

- [ ] **Step 7: Drop `Offset` from the three declarations**

`crates/temper-core/src/types/query/registry.rs` — at lines 175, 209 and 234:

```rust
door_coverage: unified_doors(vec![BoundTerm::Offset], vec![IdKind::Resource]),
// becomes
door_coverage: unified_doors(vec![], vec![IdKind::Resource]),
```

(line 209, `find-about-anywhere`, has `vec![]` as its second argument — leave that alone; it accepts no bounds.)

- [ ] **Step 8: Update the literal-comparison test in `registry.rs`**

At `registry.rs:737`, `the_cli_cannot_page_the_find_acts_and_that_is_declared` asserts `terms_unreachable == [Offset]`. Rename and invert it, keeping its explanation of *why* the axis exists:

```rust
    #[test]
    fn the_cli_can_now_page_the_find_acts_and_that_is_declared() {
        // The concrete parity gap that forced door coverage to be its own axis rather than a
        // `BuildState` variant — door-partiality is orthogonal to build state, so no
        // `BuildState` variant could ever have carried it. The gap is now CLOSED: `temper search`
        // gained `--offset`, so the axis records full term reach rather than a shortfall.
        //
        // Kept rather than deleted. The axis's value was never the non-empty entry; it is that
        // the declaration and the parser are held to each other, which
        // `the_cli_term_shortfall_is_what_clap_actually_lacks` now checks in the direction that
        // has content — every admitted term must have a flag.
        for name in [
            ActName::FindExact,
            ActName::FindAboutAnywhere,
            ActName::FindAboutWithin,
        ] {
            let a = declaration(&name).unwrap();
            assert_eq!(a.build_state, BuildState::Served);
            // Read the term axis alone: `..` keeps this from silently becoming a second
            // assertion about the bound axis, which
            // `no_door_can_supply_the_resource_bound_the_find_acts_accept` owns.
            let Some(DoorReach::Serves {
                terms_unreachable, ..
            }) = a.door_coverage.get(&Door::Cli)
            else {
                panic!("{name:?} must serve the CLI door");
            };
            assert!(
                terms_unreachable.is_empty(),
                "{name:?} declares {terms_unreachable:?} unreachable at the CLI, but \
                 `temper search` now accepts every term it admits"
            );
        }
    }
```

- [ ] **Step 9: Replace the tripwire with its inverse, so the axis keeps a named live claim**

In `crates/temper-cli/tests/act_door_coverage_cli_terms.rs`, replace `temper_search_still_cannot_page` with:

```rust
/// The live instance, asserted from the parser rather than from the declaration.
///
/// Kept as its own test beside the general gate for the reason its predecessor was: a general
/// gate over an empty set passes silently, and this one names the flag. The claim has inverted —
/// `temper search` CAN page — so the way it goes wrong has inverted too. It now fails if the flag
/// is removed while the declarations still claim full reach.
#[test]
fn temper_search_can_page() {
    let flags = search_flags();
    assert!(
        flags.contains("offset"),
        "`temper search` has lost --offset. Every find act's CLI `terms_unreachable` must \
         regain `Offset`, and `the_cli_can_now_page_the_find_acts_and_that_is_declared` in \
         registry.rs compares the declaration to a literal and will not notice on its own."
    );
}
```

- [ ] **Step 10: Run both gates and confirm green**

```bash
cargo nextest run -p temper-cli --test act_door_coverage_cli_terms
cargo nextest run -p temper-core --lib types::query::registry
```

Expected: PASS for both. `the_cli_term_shortfall_is_what_clap_actually_lacks` now requires clap to carry **both** `--limit` and `--offset`, so the axis has more live content than before, not less.

- [ ] **Step 11: Full gate and commit**

```bash
cargo make check
git add crates/temper-cli crates/temper-core/src/types/query/registry.rs
git commit -m "temper search can page, and the axis that said it could not now says so"
```

---

# PR A — the vocabulary and the flip

### Task 2: Split `validate.rs` into a pass that may consult declarations and one that may not

**Files:**
- Create: `crates/temper-core/src/types/query/validate/shape.rs`
- Create: `crates/temper-core/src/types/query/validate/capability.rs`
- Move: `crates/temper-core/src/types/query/validate.rs` → `crates/temper-core/src/types/query/validate/mod.rs`

**Interfaces:**
- Produces:
  - `pub fn validate_shape(c: &Composition) -> Vec<PlanRefusal>` — expressibility only. Consumed by PR C's `temper query --check`.
  - `pub(crate) fn validate_shape_indexed(c: &Composition) -> (Vec<PlanRefusal>, Option<Vec<StageNode>>)` — the same refusals plus the topological order when the DAG is acyclic.
  - `pub(crate) fn validate_capability(c: &Composition, by_name: &BTreeMap<&str, &StageNode>, errs: &mut Vec<PlanRefusal>)`
  - `pub fn validate(c: &Composition) -> Result<ValidatedComposition, Vec<PlanRefusal>>` — unchanged signature and unchanged behaviour.
- Consumes: `super::registry::declaration`, `super::registry::search_family` — **only in `capability.rs`**.

**The classification is per site, not per variant.** Spec ⟨3⟩ carries the table; it is the authority and was derived by reading every site. Two variants straddle the seam (`BoundTermNotApplicable`, `FilterNotApplicable`), which is why placement is by line, not by match arm.

**One behaviour must not change.** Today `check_act` runs only inside `topo_order`'s `Some` arm (`validate.rs:799-804`), so a cyclic plan returns the cycle *alone*. A test pins this (`validate.rs:982-984`: *"reachable acts keep the cycle the sole finding"*). The split must preserve it: `validate()` runs `capability` only when the shape pass produced an order.

- [ ] **Step 1: Write the failing test — the seam's behaviour, before any code moves**

Add to the existing `mod tests` in `validate.rs`:

```rust
#[test]
fn the_shape_pass_answers_without_consulting_any_declaration() {
    // A plan whose ONLY problem is that its act is not reachable from this surface. The
    // capability pass refuses it; the shape pass must not, because a client one release behind
    // would then decline a plan the server would run.
    let c = a_legal_single_stage_plan_over(ActName::Survey);

    let shape = validate_shape(&c);
    assert!(
        shape.is_empty(),
        "the shape pass raised {shape:?} for a well-formed plan; only expressibility belongs here"
    );

    let full = validate(&c).expect_err("the capability pass refuses an unreachable mechanic");
    assert!(
        full.iter().any(|e| e.reason == RefusalReason::NotSeparablyReachable),
        "expected the capability pass to supply the refusal the shape pass withheld; got {full:?}"
    );
}

#[test]
fn the_shape_pass_still_refuses_a_malformed_plan() {
    // The other direction: a shape refusal must not have migrated into the capability pass,
    // where `--check` would never see it.
    let mut c = a_legal_single_stage_plan_over(ActName::Survey);
    c.outcome.returns.clear();

    let shape = validate_shape(&c);
    assert!(
        shape
            .iter()
            .any(|e| e.reason == RefusalReason::Other("no-returns".to_string())),
        "a composition that answers nothing is malformed regardless of what is built; got {shape:?}"
    );
}
```

`a_legal_single_stage_plan_over` does not exist yet — write it beside these tests, modelled on the existing legal-plan builders in the same `mod tests` (search for `StageNode::Act(ActInvocation {` in the test module and copy the nearest one). It must return a composition with one stage, a threaded `Intention`, and one `returns` entry naming that stage.

> ⚠️ **Use `ActName::Substantiate`, not `Survey`.** Survey is still *in* `CALLABLE_FRAGMENTS` when this task runs — Task 5 is what removes it — so `expect_err` would find nothing and the test would fail for the wrong reason. `substantiate` is the fixture that works today and keeps working after Task 5, verified in `registry.rs:376-386`: `build_state: Served` with `served_by: "resource_standing_shape"`, which is absent from `CALLABLE_FRAGMENTS`, so it refuses as `NotSeparablyReachable` rather than `NotImplemented`. Its `accepts_bounds`, `accepts_seeds`, `accepts_bound_terms` and `accepts_filters` are all empty, so a minimal single-stage plan over it raises nothing else — the shape pass sees a clean plan and the capability pass sees exactly one refusal. Replace `ActName::Survey` with `ActName::Substantiate` in both tests above.

- [ ] **Step 2: Run and confirm failure**

```bash
cargo nextest run -p temper-core --lib the_shape_pass_answers_without_consulting_any_declaration
```

Expected: **compile error** — `validate_shape` does not exist.

- [ ] **Step 3: Create the module directory and move the file**

```bash
cd crates/temper-core/src/types/query
mkdir validate
git mv validate.rs validate/mod.rs
```

- [ ] **Step 4: Move the shape checks into `shape.rs`**

Create `crates/temper-core/src/types/query/validate/shape.rs`. Its header states the rule the module exists to hold:

```rust
//! Expressibility — is this a well-formed composition?
//!
//! **This module may not consult act declarations.** A refusal raised here must be true of the
//! plan and the published wire contract alone, so that a client running it against a NEWER server
//! cannot refuse a plan that server would run. Version skew is structural in this project: a
//! released CLI binary carries a `search_family()` older than the server's, and
//! `CALLABLE_FRAGMENTS` — which decides `NotSeparablyReachable` — is exactly what later beats
//! widen.
//!
//! Two guards hold that, because one is not enough. `the_shape_module_reaches_no_declaration`
//! scans this file's source for any route to the registry. But "reads no declaration" and "cannot
//! go stale" are NOT the same predicate: five sites in the capability pass read no declaration and
//! are nonetheless pure door capability (`this door does not YET apply property predicates`). So
//! `the_shape_pass_emits_exactly_these_reasons` pins the emitted set as well. The classification
//! is a judgment, and a judgment needs a pin rather than an inference.
```

Move these, verbatim except for import paths, from `mod.rs`:

| From `mod.rs` | What |
|---|---|
| `:654-660` | `no-stages` |
| `:666-672` | `no-returns` |
| `:674-688` | name indexing + `duplicate-stage-name` |
| `:690-714` | `combinator-arity`, `dangling-reference` |
| `:724-737` | `duplicate-return-stage` |
| `:739-768` | `combinator-not-returnable`, `unknown-return-stage` (**not** the `SectionNotAvailable` loop at `:773-790`) |
| `:793-798` | the `cycle` arm |
| `:162-215` | `produced_kind_of`, `topo_order` |
| `:224-231` | `unknown-act` — **rewritten**, see Step 5 |
| `:267-275` | `MissingProvenance` |
| `:303-320` | `AnchorTakesOneId` |
| `:404-414` | `BoundTermNotApplicable`, the negative-value arm **only** |
| `:471-489` | `MissingIntention` |
| `:493-500` | `UnknownFilterValue` |
| `:501-515` | `empty-property-key`, `empty-contains` |

Everything else in `check_act` moves to `capability.rs` in Step 6.

- [ ] **Step 5: Rewrite `unknown-act` as a type check**

`ActName` is **open** — `#[serde(untagged)] Other(String)` at `act.rs:45-46` — so a caller can send `"act": "made-up"` and it deserializes. Today that is detected by `declaration()` returning `None`, which needs the registry. It is answerable from the type alone:

```rust
    // `ActName` is open (`act.rs:45-46`), so an unrecognized act name deserializes into
    // `Other` rather than failing serde. That makes this caller-reachable — and answerable
    // without the registry, which is what lets it live in the shape pass. `declaration()`
    // returning `None` for a KNOWN variant would be an internal inconsistency, not a caller
    // error; the capability pass owns that case.
    if let ActName::Other(raw) = &inv.act {
        errs.push(refusal(
            Some(name),
            RefusalReason::Other("unknown-act".to_string()),
            format!("`{raw}` is not a known act"),
        ));
        return;
    }
```

- [ ] **Step 6: Move the capability checks into `capability.rs`**

Create `crates/temper-core/src/types/query/validate/capability.rs` with a header stating the converse rule:

```rust
//! Capability — the shape is fine, and this server has not built it yet.
//!
//! Everything here may move as beats land, which is why none of it may be raised by a client
//! against a server it does not share a binary with. Note that reading a declaration is NOT what
//! makes a check belong here: `:355`, `:370`, `:381`, `:389` and the `SectionNotAvailable` loop
//! read no declaration and are pure door capability — their own detail strings say "does not YET
//! apply". Task 10b and any widening of `ReturnSpec::ADMITTED_SECTIONS` retire them.
```

Move: `:235-251` (`NotImplemented`, `NotSeparablyReachable`), `:276-301` (`UnsupportedSeedKind`, `UnsupportedBoundKind`), `:339-393` (all four "this door does not apply" `FilterNotApplicable` sites), `:415-441` (`BoundTermNotApplicable`, the 32-bit and not-admitted arms), `:443-457` (`FilterNotApplicable`, act does not admit), and `:773-790` (`SectionNotAvailable`).

The `declaration()` lookup lives here:

```rust
    let Some(decl) = declaration(&inv.act) else {
        // The shape pass has already refused `ActName::Other`. Reaching here means a KNOWN
        // variant has no declaration — an internal inconsistency, not a caller error. Every
        // non-`Other` variant is covered by `search_family()`, which `registry.rs`'s own
        // exhaustiveness test holds.
        return;
    };
```

- [ ] **Step 7: Rewrite `validate` in `mod.rs`**

```rust
pub fn validate(c: &Composition) -> Result<ValidatedComposition, Vec<PlanRefusal>> {
    let (mut errs, ordered) = shape::validate_shape_indexed(c);

    // Capability runs only when the DAG is acyclic, preserving the incumbent behaviour that a
    // cyclic plan returns the cycle ALONE. A test pins it (`reachable acts keep the cycle the
    // sole finding`), and it is the right shape: per-stage findings over a graph that is not a
    // graph would be findings about a plan that cannot be read.
    if let Some(ordered) = ordered {
        let by_name = index_by_name(c);
        capability::validate_capability(c, &by_name, &mut errs);
        if errs.is_empty() {
            return Ok(ValidatedComposition {
                composition: c.clone(),
                ordered,
            });
        }
    }

    Err(errs)
}

/// Expressibility alone — every refusal that is true of the plan and the published contract
/// without consulting what this server has built. This is what `temper query --check` runs.
pub fn validate_shape(c: &Composition) -> Vec<PlanRefusal> {
    shape::validate_shape_indexed(c).0
}
```

`index_by_name` is the first-wins `BTreeMap` build currently inline at `:675-688`; extract it to `mod.rs` and have `shape` call it too, so the two passes cannot index differently.

- [ ] **Step 8: Run the new tests, then the whole file's suite**

```bash
cargo nextest run -p temper-core --lib the_shape_pass_answers_without_consulting_any_declaration
cargo nextest run -p temper-core --lib the_shape_pass_still_refuses_a_malformed_plan
cargo nextest run -p temper-core --lib types::query::validate
```

Expected: PASS for all. Every pre-existing test in the file must pass **unchanged** — this task moves code and changes no behaviour. If one needs editing, stop and work out why; a behaviour change here is a defect, not a rebase.

- [ ] **Step 9: Commit**

```bash
cargo make check
git add crates/temper-core/src/types/query/validate
git commit -m "Validation splits in two: what a stale client may say, and what only the server may"
```

### Task 3: Promote the twelve string refusals to variants

Free now, breaking after the door ships — spec ⟨4⟩. Nine of the twelve change spelling: `RefusalReason` carries `#[serde(rename_all = "snake_case")]`, so `Other("dangling-reference")` goes on the wire as `"dangling-reference"` while `DanglingReference` goes as `"dangling_reference"`.

**Files:**
- Modify: `crates/temper-core/src/types/query/disposition.rs:57-176` (the enum and its `Other` doc block)
- Modify: `crates/temper-core/src/types/query/validate/shape.rs` (all twelve construction sites)
- Modify: `crates/temper-core/src/types/query/validate/mod.rs` (test assertions naming `Other(...)`)
- Modify: `crates/temper-core/tests/fixtures/query/*.schema.json` (regenerated)
- Modify: `packages/temper-ui/src/lib/types/generated/query.ts` (regenerated)

**Interfaces:**
- Produces twelve new `RefusalReason` variants: `NoStages`, `NoReturns`, `DuplicateStageName`, `CombinatorArity`, `DanglingReference`, `DuplicateReturnStage`, `CombinatorNotReturnable`, `UnknownReturnStage`, `Cycle`, `UnknownAct`, `EmptyPropertyKey`, `EmptyContains`.

- [ ] **Step 1: Write the failing test**

In `disposition.rs`'s `mod tests`:

```rust
#[test]
fn every_refusal_this_crate_raises_is_a_known_reason() {
    // `is_known` answered `false` for twelve reasons the server itself emitted, and those twelve
    // were kebab-case while every declared variant was snake_case — so a client's vocabulary was
    // two conventions. Recorded in review and deferred to the door, which is now.
    for reason in [
        RefusalReason::NoStages,
        RefusalReason::NoReturns,
        RefusalReason::DuplicateStageName,
        RefusalReason::CombinatorArity,
        RefusalReason::DanglingReference,
        RefusalReason::DuplicateReturnStage,
        RefusalReason::CombinatorNotReturnable,
        RefusalReason::UnknownReturnStage,
        RefusalReason::Cycle,
        RefusalReason::UnknownAct,
        RefusalReason::EmptyPropertyKey,
        RefusalReason::EmptyContains,
    ] {
        assert!(reason.is_known(), "{reason:?} must not deserialize as `Other`");
    }
}

#[test]
fn a_promoted_reason_round_trips_in_snake_case() {
    let json = serde_json::to_string(&RefusalReason::DanglingReference).unwrap();
    assert_eq!(json, "\"dangling_reference\"");
    assert_eq!(
        serde_json::from_str::<RefusalReason>(&json).unwrap(),
        RefusalReason::DanglingReference
    );
}

#[test]
fn other_still_carries_a_reason_from_a_newer_producer() {
    // `Other` is not vestigial after the promotion — this is its actual purpose.
    let reason: RefusalReason = serde_json::from_str("\"some_future_reason\"").unwrap();
    assert!(!reason.is_known());
}
```

- [ ] **Step 2: Run and confirm failure**

```bash
cargo nextest run -p temper-core --lib types::query::disposition
```

Expected: compile error — the variants do not exist.

- [ ] **Step 3: Add the twelve variants**

In `disposition.rs`, before `Other(String)`. Each gets a one-line doc comment saying what it means; group them under a comment recording where they came from:

```rust
    // ── Expressibility. Promoted from `Other(_)` strings with the door `[2026-08-12]` ───────────
    //
    // These were kebab-case strings the server emitted and its own `is_known` answered `false`
    // to. Nothing consumed them, because there was no door — so the spelling change from
    // `dangling-reference` to `dangling_reference` cost nothing here and would have been a
    // breaking change to a published 400 body a week later.
    /// The composition declares no stages, so it asks nothing.
    NoStages,
    /// The composition returns no stages, so it answers nothing.
    NoReturns,
    /// Two stages share a name.
    DuplicateStageName,
    /// A set combination needs two or more inputs.
    CombinatorArity,
    /// A stage references a stage that was never declared.
    DanglingReference,
    /// A stage is named more than once in `returns`.
    DuplicateReturnStage,
    /// A combinator's rows come from more than one act, so they have no single act to score them.
    CombinatorNotReturnable,
    /// `returns` names a stage that was never declared.
    UnknownReturnStage,
    /// The composition contains a cycle; a query DAG must be acyclic.
    Cycle,
    /// The `act` name is not one this server declares. `ActName` is open, so this is reachable.
    UnknownAct,
    /// A property predicate was supplied with no key.
    EmptyPropertyKey,
    /// A `contains` predicate was supplied with no values, so it narrows nothing.
    EmptyContains,
```

Then correct the `Other(String)` doc block at `:158-172` in place — it currently enumerates the twelve and says the promotion *"belongs with the door, not ahead of it."* Replace that with a note that the promotion happened and what `Other` is for now. **Do not delete the history**; say it was done and when.

- [ ] **Step 4: Replace the twelve construction sites**

In `validate/shape.rs`, each `RefusalReason::Other("x".to_string())` becomes its variant. The details stay byte-identical — only the reason changes.

- [ ] **Step 5: Update test assertions naming the old strings**

`validate/mod.rs`'s test module compares against `RefusalReason::Other("duplicate-return-stage".to_string())` and similar at `:1256`, `:1273`, `:1292`, `:1969`. Point them at the variants.

- [ ] **Step 6: Run and confirm green**

```bash
cargo nextest run -p temper-core --lib types::query
```

Expected: PASS.

- [ ] **Step 7: Regenerate the artifacts and commit them in the same commit**

```bash
UPDATE_SCHEMA=1 cargo make test-schema-core
cargo make generate-ts-types
git diff --stat crates/temper-core/tests/fixtures/query packages/temper-ui/src/lib/types/generated
```

Expected: the query fixtures and `query.ts` show the twelve new enum values. **If `resource_view.ts` also changes, read it** — ts-rs rewrites a dependency's file with only the types reachable from the graph being exported, and `ResourceSection` has silently vanished this way before.

```bash
cargo make check
git add crates/temper-core packages/temper-ui/src/lib/types/generated
git commit -m "Twelve refusals stop being strings, while that is still free"
```

### Task 4: Guard the seam twice

**Files:**
- Create: `crates/temper-core/tests/query_validate_seam.rs`

**Interfaces:**
- Consumes: `validate_shape` (Task 2), the twelve variants (Task 3).

- [ ] **Step 1: Write both guards**

```rust
//! The two guards on the shape/capability seam — spec ⟨3⟩.
//!
//! Guard one asks whether the shape module can reach a declaration. Guard two asks which reasons
//! it actually emits. Both are needed, and the second is the load-bearing one: five capability
//! sites read no declaration at all, so an import scan alone would happily let them sit in the
//! shape pass, where a stale client would raise them against a newer server.

const SHAPE_SRC: &str = include_str!("../src/types/query/validate/shape.rs");

/// The shape module's source with comment lines removed.
///
/// **Both guards scan CODE, not prose**, and this is not a convenience. The rule is about what
/// `shape.rs` can call, and its own header necessarily *names* the things it may not reach — it
/// explains the seam. Scanning raw source made guard one fail on the doc comment that documents
/// it, which is a test failing on its own subject matter. Found while writing this file.
///
/// Line-oriented and deliberately crude: it strips `//` line comments only. A `/* */` block
/// comment would defeat it, which is why `no_block_comments_in_the_shape_module` exists below.
fn shape_code() -> String {
    SHAPE_SRC
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The stripper above only understands `//`. Say so, and hold it.
#[test]
fn no_block_comments_in_the_shape_module() {
    assert!(
        !SHAPE_SRC.contains("/*"),
        "`validate/shape.rs` gained a block comment. `shape_code()` strips only `//` lines, so \
         a forbidden name inside `/* */` would slip past both guards. Use `//`, or teach the \
         stripper."
    );
}

/// Guard one — the shape module has no route to the act declarations.
#[test]
fn the_shape_module_reaches_no_declaration() {
    let code = shape_code();
    for forbidden in ["registry", "declaration(", "search_family", "CALLABLE_FRAGMENTS"] {
        assert!(
            !code.contains(forbidden),
            "`validate/shape.rs` calls `{forbidden}` in code. A refusal that consults what this \
             server has built cannot be raised by a client that does not share its binary."
        );
    }
}

/// Guard two — the reasons the shape pass emits are exactly this set.
///
/// Pinned rather than derived, because the classification is a JUDGMENT. `FilterNotApplicable`
/// at `capability.rs`'s "this door does not yet apply" sites reads no declaration and would pass
/// guard one; it belongs to capability because Task 10b retires it. Nothing but this pin records
/// that.
#[test]
fn the_shape_pass_emits_exactly_these_reasons() {
    let code = shape_code();
    let mut found: Vec<&str> = code
        .match_indices("RefusalReason::")
        .map(|(i, _)| {
            let tail = &code[i + "RefusalReason::".len()..];
            let end = tail
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(tail.len());
            &tail[..end]
        })
        .collect();
    found.sort_unstable();
    found.dedup();

    let expected = [
        "AnchorTakesOneId",
        "BoundTermNotApplicable",
        "CombinatorArity",
        "CombinatorNotReturnable",
        "Cycle",
        "DanglingReference",
        "DuplicateReturnStage",
        "DuplicateStageName",
        "EmptyContains",
        "EmptyPropertyKey",
        "MissingIntention",
        "MissingProvenance",
        "NoReturns",
        "NoStages",
        "UnknownAct",
        "UnknownFilterValue",
        "UnknownReturnStage",
    ];

    assert_eq!(
        found, expected,
        "the shape pass's emitted reasons moved. Adding one means asserting it cannot change \
         without a wire-contract change; removing one means it moved to capability and \
         `temper query --check` stopped reporting it."
    );
}
```

> **`BoundTermNotApplicable` appears in both passes on purpose** — the negative-value arm is shape, the 32-bit and not-admitted arms are capability. That is why this pin is over emitted reasons rather than over variants, and why it cannot be a disjointness assertion.

- [ ] **Step 2: Run and confirm both pass**

```bash
cargo nextest run -p temper-core --test query_validate_seam
```

Expected: PASS. If guard two fails, the listed diff tells you which side a check landed on.

- [ ] **Step 3: Bite-probe both guards — a guard nobody has watched fail is not evidence**

Temporarily add `use super::super::registry::declaration;` to `shape.rs` and re-run: guard one must fail. Revert. Then temporarily move the `properties` refusal (`capability.rs`, the `does not yet apply property predicates` site) into `shape.rs` and re-run: guard **two** must fail while guard **one stays green** — that is the whole reason guard two exists. Revert both.

Record what you observed in the commit message. If guard one fails on the second probe, the probe is wrong or `shape.rs` gained an import — find out which before proceeding.

- [ ] **Step 4: Commit**

```bash
cargo make check
git add crates/temper-core/tests/query_validate_seam.rs
git commit -m "Two guards on the seam, because reading no declaration is not the same as cannot go stale"
```

### Task 5: Flip the placeholder

`CALLABLE_FRAGMENTS` maps `search_graph_expand` and `wayfind_region_scores` to `__temper_unbound_act`, a function that deliberately does not exist — so `follow-from` and `survey` validate clean and fail at execution. Spec ⟨5⟩: the flip is what keeps `registry.rs:42-46`'s `DoorReach::Absent` true once the door opens, not merely what avoids a 500.

**Files:**
- Modify: `crates/temper-core/src/types/query/validate/mod.rs:60-65` (`CALLABLE_FRAGMENTS` and its doc comment)
- Modify: `crates/temper-core/src/types/query/validate/mod.rs` (tests using `follow-from`/`survey` as reachable — 32 references)
- Modify: `crates/temper-substrate/tests/query_plan_compile.rs` (2 references)

- [ ] **Step 1: Write the failing test**

In `validate/mod.rs`'s test module:

```rust
#[test]
fn an_act_whose_fragment_takes_arguments_no_slot_supplies_refuses_statically() {
    // `follow-from` and `survey` mapped to a placeholder function that does not exist, so they
    // validated clean and failed at EXECUTION. Invisible while nothing executed a composition
    // outside its own tests; a 500 the moment a door opened.
    //
    // The flip is not primarily about the 500. `registry.rs` declares both acts
    // `DoorReach::Absent` at all three doors and promises they restore to `Serves` when this door
    // lands. Had the placeholder survived, BOTH would have been false: reachable through the
    // door, and unable to answer.
    for act in [ActName::FollowFrom, ActName::Survey] {
        let c = a_legal_single_stage_plan_over(act.clone());
        let errs = validate(&c).expect_err("{act:?} has no fragment this surface can emit");
        assert!(
            errs.iter()
                .any(|e| e.reason == RefusalReason::NotSeparablyReachable),
            "{act:?} must refuse as unreachable rather than compile to an absent function; \
             got {errs:?}"
        );
    }
}
```

- [ ] **Step 2: Run and confirm failure**

```bash
cargo nextest run -p temper-core --lib an_act_whose_fragment_takes_arguments_no_slot_supplies_refuses_statically
```

Expected: FAIL — `validate` returns `Ok` for both acts today.

- [ ] **Step 3: Drop the two rows**

```rust
const CALLABLE_FRAGMENTS: &[(&str, &str)] = &[
    ("search_exact", "__temper_ungated_find_exact"),
    ("search_wide", "__temper_ungated_find_wide"),
];
```

Then rewrite the doc comment's last paragraph. It currently reads *"`follow-from` and `survey` map to the deliberately-absent placeholder… Keeping them here — rather than dropping them, which would make them refuse statically — preserves the beat-C behaviour their tests pin."* Replace with why they are now absent: their fragments take arguments no slot supplies (`p_depth`/`p_gamma`, `p_lens`), Task 11's edge-provenance spike is what unblocks `follow-from`, and absence here is what keeps `DoorReach::Absent` honest.

- [ ] **Step 4: Move the legal-plan examples back onto the find acts**

Beat B grounded its legal-plan examples on `follow-from`/`survey` **because the find acts were then unreachable** — recorded in the task body. Beat D made the find acts reachable and nobody moved them back. This step is that move.

Run the suite and work through the failures:

```bash
cargo nextest run -p temper-core --lib types::query::validate
```

For each failure, the test's *subject* is unchanged — only the act it is expressed over. A cycle test needs a reachable act so the cycle stays the sole finding; a chaining test needs two reachable acts. `find-exact` → `find-about-within` is a legal chain (`find-about-within` accepts a resource bound). **Do not weaken an assertion to make it pass** — if a test cannot be re-expressed over a reachable act, stop and say so.

The comment at `:982-984` already anticipates this: *"(now refuses as unreachable; reachable acts keep the cycle the sole finding.)"*

- [ ] **Step 5: Fix the substrate compile tests**

```bash
cargo nextest run -p temper-substrate --features artifact-tests,test-db --test query_plan_compile
```

> **`readback/` tests are gated `artifact-tests`.** A run scoped `--features test-db` compiles `query_plan_execute.rs` to nothing — this has cost a broken suite before. Use both features.

`query_plan.rs:380` carries a comment stating that `follow-from` and `survey` reach the placeholder emission. After the flip they cannot: `validate` refuses them first, so that branch is unreachable through `compile`. Update the comment to say so rather than deleting the branch — the codebase's idiom is *"unreachable through `validate`, and asserted anyway."*

- [ ] **Step 6: Confirm the whole query surface is green**

```bash
cargo nextest run -p temper-core --lib types::query
cargo nextest run -p temper-core --test query_validate_seam
cargo nextest run -p temper-core --test query_schema
cargo nextest run -p temper-substrate --features artifact-tests,test-db --test query_plan_compile
cargo nextest run -p temper-cli --test act_door_coverage_cli_terms
```

- [ ] **Step 7: Commit**

```bash
cargo make check
git add crates/temper-core crates/temper-substrate
git commit -m "An act that cannot answer refuses instead of compiling to a function that does not exist"
```

---

## What this plan does NOT do

Named so a reviewer does not read their absence as an oversight. All are spec non-goals.

- **No door.** `POST /api/query`, `temper query`, and the MCP tool are PRs B and later. Nothing here is reachable by a caller.
- **No `--check`.** PR C. Task 2 builds `validate_shape` *for* it; this plan ships no consumer.
- **No `bounds_unreachable` change.** `/api/query` is the first door whose params carry a resource-id list, which makes `find-exact` and `find-about-within`'s `[IdKind::Resource]` false at Api and Cli — but only once B opens. `no_door_can_supply_the_resource_bound_the_find_acts_accept` in `registry.rs` stays true through A0 and A, and PR B is where `unified_doors` gains a per-door third argument.
- **No `FilterNotApplicable` split** on the permanent/not-yet axis.
- **No property predicates, no edge provenance, no `EXPLAIN` harness, no survey redesign.**
- **No answer-quality witness.** Every clause of the frame register stays `declared-uncovered`.

`promote_admin`'s gate (`access_service.rs:592`) is ruled and unrelated to these files. It rides in whichever PR touches `access_service.rs` first, or its own commit.

## Declared risk

**Task 5's test fallout is measured but not read.** 32 references to `ActName::FollowFrom`/`ActName::Survey` in `validate/mod.rs` and 2 in `query_plan_compile.rs` — counted, not individually classified. The claim that *"each test's actual subject is unchanged"* is inherited from Beat B's own note about why the examples moved, and is reasoning rather than a per-test check. If a test resists re-expression over a reachable act, that is a real finding about the flip, not a rebase problem.
