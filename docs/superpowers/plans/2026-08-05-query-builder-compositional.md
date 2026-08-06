# TemperQueryBuilder — Implementation Plan (beats A–C)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A declared composition of acts — a named DAG — is validated statically and compiled into
one SQL statement that executes over `survey` and `follow-from`, returning per-arm results with a
per-stage trace.

**Architecture:** Types and the pure validator live in `temper-core/src/types/query/`. The SQL
compiler lives in `temper-substrate/src/readback/`, following that module's established runtime-`sqlx`
pattern. Orchestration — validate, compile, execute, assemble — lives in `temper-services`. No door is
built in this plan.

**Tech Stack:** Rust, sqlx 0.8 (runtime `query_as`, not the macros — see Global Constraints),
PostgreSQL 17/18, schemars + ts-rs + utoipa for generated artifacts, cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-08-05-query-builder-compositional-design.md`. Every task cites
the section it implements. **Read the cited section — this plan is an index over the spec, not a
replacement for it.**

**Out of scope, by design:** beat D (binding `find-exact` / `find-about-*` to phase 1's
`search_exact` / `search_wide`), beat E (the API/MCP/CLI doors), beat F (act-declaration
reconciliation). D and F are gated on the sibling session's phase-1 work; E is a follow-on plan.

---

## Global Constraints

- **Rust standards:** all public types derive `Debug`; `#[expect(lint, reason = "...")]` never
  `#[allow]`; params structs beyond 5 domain-related parameters.
- **No `serde_json::json!()` for structured data.** Define a struct. (`CLAUDE.md`, Code Quality Rules.)
- **Every wire type carries the four cfg-gated derives** used throughout `types/query/`:
  `utoipa::ToSchema` (`web-api`), `ts_rs::TS` with `ts(export, export_to = "query.ts")`
  (`typescript`), `schemars::JsonSchema` (`mcp`). Copy the attribute block from a neighbouring type
  verbatim; a missing derive fails artifact generation, not compilation.
- **Regenerate artifacts in the same commit as the type change.** `cargo make check` gates all of
  them. Query-schema snapshots:
  `UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema`
  `[verified — crates/temper-core/tests/query_schema.rs:5]`. TypeScript: `cargo make generate-ts-types`.
  OpenAPI: `cargo make openapi`. Read the `generated-artifacts` skill before the first one.
- **Runtime `sqlx` is an allow-listed exception, and this plan widens the allow-list.**
  `readback/mod.rs`'s module note names exactly three runtime reads, all for a `::vector` bind, and
  records that sixteen others were clawed back on 2026-07-30 for imitating them without a reason.
  Task 10 amends that note. Do not add a runtime read without it.
- **Never interpolate a caller-supplied value into SQL.** Bind it. The only identifiers the builder
  emits are stage names, and Task 9 holds them to an allowlist.
- **Test database:** `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`,
  started with `cargo make docker-up`. temper-services DB tests are gated `#![cfg(feature = "test-db")]`
  `[verified — crates/temper-services/tests/reachable_teams_one_definition_test.rs:1]`.

### On code blocks in this plan

This repo's grounding discipline forbids authoring invented implementation bodies into a plan —
*"implementers build the code block, not the correct prose beside it."* So every code block here is
one of exactly two things:

- **A test.** The failing test *is* the specification. Write it as given.
- **A type or signature declaration.** This is the interface neighbouring tasks depend on and must
  not be guessed.

**No task contains an implementation body.** Where a step says "make it pass," the signature above it
and the test beside it are the whole contract; write the body against the real code you find on disk.
If disk contradicts this plan, disk wins — report the gap.

---

## File Structure

**Create:**
- `crates/temper-core/src/types/query/stage.rs` — stage identity, inputs, outputs. One responsibility:
  what a node is called and what flows between nodes.
- `crates/temper-core/src/types/query/validate.rs` — the pure validator and `ValidatedComposition`.
- `crates/temper-substrate/src/readback/query_plan.rs` — SQL emission. The only file that writes SQL text.
- `crates/temper-services/src/services/query_service.rs` — orchestration.
- `crates/temper-substrate/tests/query_plan_compile.rs` — pure emission tests, no database.
- `crates/temper-services/tests/query_service_test.rs` — execution against a real database.

**Modify:**
- `crates/temper-core/src/types/query/envelope.rs` — `ActInvocation` gains identity and input;
  `ActResult.produced` becomes a tagged union.
- `crates/temper-core/src/types/query/composition.rs` — `stages` becomes nodes; `OutcomeDeclaration`
  gains `returns` and loses `produces`.
- `crates/temper-core/src/types/query/disposition.rs` — two `RefusalReason` changes.
- `crates/temper-core/src/types/query/trace.rs` — `BoundsSource::Expression` documented as reserved.
- `crates/temper-core/src/types/query/mod.rs` — re-exports.
- `crates/temper-substrate/src/readback/mod.rs:26-34` — the runtime-`sqlx` exception note.

---

# Beat A — the types

## Task 1: Stage identity and inputs

**GD-3: AMEND.** Amends `ActInvocation`, whose `bounds: Option<IdSet>` is the caller-supplied case and
survives as one variant. Spec §1, §2. Authorized by spec §2: *"`Composition.stages` becomes a set of
named nodes, each declaring its inputs as references rather than literal ids."*

**Files:**
- Create: `crates/temper-core/src/types/query/stage.rs`
- Modify: `crates/temper-core/src/types/query/envelope.rs:21-39`, `mod.rs`

**Interfaces:**
- Consumes: `IdSet` (`id_set.rs`), `BoundsMode` (`scalars.rs`).
- Produces:
  ```rust
  pub struct StageName(String);
  impl StageName {
      /// Rejects anything outside `[a-z][a-z0-9_]{0,62}`. Task 9 relies on this for SQL identifier
      /// safety, so the constructor is the only way to build one.
      pub fn parse(raw: &str) -> Option<StageName>;
      pub fn as_str(&self) -> &str;
  }

  #[serde(rename_all = "snake_case", tag = "from")]
  pub enum StageInput {
      Caller { ids: IdSet },
      Upstream { stage: StageName },
  }
  ```
  and `ActInvocation` gains `pub name: StageName` and `pub input: Option<StageInput>`, losing
  `pub bounds: Option<IdSet>`.

- [ ] **Step 1: Write the failing tests** in `stage.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::id_set::{IdKind, IdSet};

    #[test]
    fn a_stage_name_is_a_safe_sql_identifier_or_it_does_not_exist() {
        // Task 9 emits stage names as CTE identifiers. The type is the gate: if a name cannot be
        // constructed, it cannot reach SQL. This is parse-don't-validate, and it is the reason
        // there is no `StageName::new_unchecked`.
        assert!(StageName::parse("hits").is_some());
        assert!(StageName::parse("wide_arm_2").is_some());
        assert!(StageName::parse("Hits").is_none(), "uppercase rejected");
        assert!(StageName::parse("2hits").is_none(), "must start with a letter");
        assert!(StageName::parse("hits-2").is_none(), "hyphen rejected");
        assert!(StageName::parse("hits\"; DROP TABLE kb_resources; --").is_none());
        assert!(StageName::parse("").is_none());
        assert!(StageName::parse(&"a".repeat(64)).is_none(), "63 is the ceiling");
    }

    #[test]
    fn an_input_distinguishes_caller_ids_from_an_upstream_reference() {
        // THE gap this whole phase exists to close: the invocation side can finally declare what
        // BoundsSource has always been able to report.
        let caller = StageInput::Caller {
            ids: IdSet { kind: IdKind::Resource, provenance: None, ids: vec![] },
        };
        let upstream = StageInput::Upstream { stage: StageName::parse("hits").unwrap() };
        assert_ne!(
            serde_json::to_string(&caller).unwrap(),
            serde_json::to_string(&upstream).unwrap()
        );
        for v in [caller, upstream] {
            assert_eq!(
                serde_json::from_str::<StageInput>(&serde_json::to_string(&v).unwrap()).unwrap(),
                v
            );
        }
    }

    #[test]
    fn a_stage_name_round_trips_through_the_wire_as_a_bare_string() {
        // Transparent on the wire so a plan reads as JSON a human wrote, not as a tagged wrapper.
        let n = StageName::parse("near").unwrap();
        assert_eq!(serde_json::to_string(&n).unwrap(), "\"near\"");
        assert_eq!(serde_json::from_str::<StageName>("\"near\"").unwrap(), n);
        assert!(serde_json::from_str::<StageName>("\"Near\"").is_err(), "validation applies on deserialize");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-core --features mcp stage::tests`
Expected: FAIL — `stage.rs` does not exist / `StageName` not found.

- [ ] **Step 3: Implement `stage.rs`**

Declare `StageName` and `StageInput` with the signatures in **Interfaces** above, plus the standard
four cfg-gated derives copied from a neighbouring type. `StageName` needs a manual `Deserialize` (or
`#[serde(try_from = "String")]`) so the third test's rejection-on-deserialize holds. Add
`pub mod stage;` and the re-exports to `mod.rs`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-core --features mcp stage::tests`
Expected: PASS.

- [ ] **Step 5: Amend `ActInvocation` and fix its tests**

In `envelope.rs:21-39` replace `bounds: Option<IdSet>` with `name: StageName` and
`input: Option<StageInput>`. Keep `bounds_mode`, `terms`, `resource_filter`, `edge_filter` unchanged.
`envelope.rs`'s existing test `an_invocation_without_bounds_or_terms_omits_them` and
`composition.rs`'s `stage()` helper both construct `ActInvocation` and will not compile — update them
to the new shape rather than deleting them.

- [ ] **Step 6: Run the crate's tests**

Run: `cargo nextest run -p temper-core --features mcp`
Expected: PASS. The `query_schema` snapshot test will FAIL — that is correct and Task 5 regenerates it.
Note the failure and continue.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/types/query/
git commit -m "query: a stage has a name and an input, so a pipe can finally be declared"
```

---

## Task 2: Node kinds and the DAG

**GD-3: AMEND.** Amends `Composition.stages`. Spec §2 — *"Set combinators are their own node kind.
`union` and `intersect` take two inputs; no act does, so they cannot be act invocations without lying
about what an act is."*

**Files:**
- Modify: `crates/temper-core/src/types/query/composition.rs:49-76`, `stage.rs`, `mod.rs`

**Interfaces:**
- Consumes: `StageName`, `StageInput`, `ActInvocation` (Task 1).
- Produces:
  ```rust
  #[serde(rename_all = "snake_case")]
  pub enum CombineOp { Union, Intersect }

  pub struct CombineNode {
      pub name: StageName,
      pub op: CombineOp,
      /// Two or more. One input is not a combination; validation refuses it (Task 6).
      pub inputs: Vec<StageName>,
  }

  #[serde(untagged)]
  pub enum StageNode { Act(ActInvocation), Combine(CombineNode) }

  impl StageNode {
      pub fn name(&self) -> &StageName;
      /// Every upstream name this node reads. Empty for a caller-fed act.
      pub fn upstream_names(&self) -> Vec<&StageName>;
  }
  ```
  and `Composition.stages: Vec<StageNode>`, replacing `Vec<ActInvocation>`. `Composition::act_sequence`
  is **removed** — a DAG has no single sequence, and returning one would be a false claim. Task 6's
  topological order replaces it.

- [ ] **Step 1: Write the failing tests** in `composition.rs`'s test module

```rust
#[test]
fn a_combinator_is_its_own_node_kind_because_no_act_takes_two_inputs() {
    let c = CombineNode {
        name: StageName::parse("both").unwrap(),
        op: CombineOp::Union,
        inputs: vec![
            StageName::parse("quoted").unwrap(),
            StageName::parse("wide").unwrap(),
        ],
    };
    let node = StageNode::Combine(c.clone());
    assert_eq!(node.name(), &c.name);
    assert_eq!(node.upstream_names().len(), 2);
    assert_eq!(
        serde_json::from_str::<StageNode>(&serde_json::to_string(&node).unwrap()).unwrap(),
        node
    );
}

#[test]
fn an_act_node_reports_its_single_upstream_and_a_caller_fed_one_reports_none() {
    let seeded = StageNode::Act(ActInvocation {
        name: StageName::parse("near").unwrap(),
        input: Some(StageInput::Upstream { stage: StageName::parse("hits").unwrap() }),
        bounds_mode: Some(BoundsMode::Seed),
        terms: BTreeMap::new(),
        resource_filter: None,
        edge_filter: None,
        act: ActName::FollowFrom,
    });
    assert_eq!(seeded.upstream_names().len(), 1);

    let rooted = StageNode::Act(ActInvocation {
        name: StageName::parse("hits").unwrap(),
        input: None,
        bounds_mode: None,
        terms: BTreeMap::new(),
        resource_filter: None,
        edge_filter: None,
        act: ActName::FindExact,
    });
    assert!(rooted.upstream_names().is_empty());
}

#[test]
fn a_composition_carries_nodes_and_no_longer_claims_a_single_sequence() {
    // `act_sequence()` is gone on purpose: a DAG has no one order, and a method returning one
    // would be a false claim that reads as true. Task 6's topological order replaces it.
    let c = Composition {
        outcome: OutcomeDeclaration {
            description: "exact hits and their neighbours".to_string(),
            returns: vec![],
        },
        intention: None,
        on_stage_refusal: RefusalDisposition::Halt,
        meta_detail: Default::default(),
        bounds: BTreeMap::new(),
        stages: vec![],
    };
    assert!(c.stages.is_empty());
}
```

> **Note:** the third test references `OutcomeDeclaration { returns }`, which Task 3 introduces. Write
> it now with `produces: None` instead and change it in Task 3 — or do Tasks 2 and 3 in one sitting.
> Do **not** leave it referencing a field that does not exist.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-core --features mcp composition::tests`
Expected: FAIL — `CombineNode` not found.

- [ ] **Step 3: Implement the node types**

Declare `CombineOp`, `CombineNode`, `StageNode` per **Interfaces**, with the four derives. Change
`Composition.stages` to `Vec<StageNode>` and delete `act_sequence`. `StageNode` is `#[serde(untagged)]`
so a plan's JSON reads naturally; the two variants are distinguishable because `CombineNode` has `op`
and `ActInvocation` has `act`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-core --features mcp composition::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/query/
git commit -m "query: a composition is a DAG of nodes, and combinators are not acts"
```

---

## Task 3: Returned stages are declared

**GD-3: AMEND.** Spec §3, *"Returned stages are declared, not inferred"* — including the reason
inference was rejected: *"adding a downstream stage silently stops returning what you used to get
back."*

**Files:**
- Modify: `crates/temper-core/src/types/query/composition.rs:40-46`

**Interfaces:**
- Produces:
  ```rust
  pub struct ReturnSpec {
      pub stage: StageName,
      /// Empty means the kind's default projection. Named fields subselect it.
      #[serde(default, skip_serializing_if = "Vec::is_empty")]
      pub fields: Vec<String>,
  }
  ```
  and `OutcomeDeclaration { description: String, returns: Vec<ReturnSpec> }` — `produces: Option<IdKind>`
  is **removed**.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn an_outcome_declares_which_stages_come_back() {
    let o = OutcomeDeclaration {
        description: "neighbours of my exact hits".to_string(),
        returns: vec![ReturnSpec {
            stage: StageName::parse("near").unwrap(),
            fields: vec!["title".to_string(), "home".to_string()],
        }],
    };
    assert_eq!(o.returns.len(), 1);
    assert_eq!(
        serde_json::from_str::<OutcomeDeclaration>(&serde_json::to_string(&o).unwrap()).unwrap(),
        o
    );
}

#[test]
fn an_empty_field_list_means_the_default_projection_and_serializes_to_nothing() {
    let r = ReturnSpec { stage: StageName::parse("near").unwrap(), fields: vec![] };
    assert!(!serde_json::to_string(&r).unwrap().contains("fields"));
}

#[test]
fn a_composition_no_longer_declares_one_produced_kind() {
    // A resource arm beside a region arm has no single answer. `produces` was a field that could
    // only ever be right for a single-arm plan — it is derived from `returns` now, not declared.
    let json = serde_json::to_string(&OutcomeDeclaration {
        description: "x".to_string(),
        returns: vec![],
    })
    .unwrap();
    assert!(!json.contains("produces"));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-core --features mcp composition::tests`
Expected: FAIL — `ReturnSpec` not found.

- [ ] **Step 3: Implement**

Declare `ReturnSpec` with the four derives; replace `OutcomeDeclaration.produces` with `returns`.
Update every construction site in `composition.rs`'s existing tests.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-core --features mcp`
Expected: PASS except the `query_schema` snapshot (Task 5 regenerates it).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/query/composition.rs
git commit -m "query: an outcome declares what comes back, so topology cannot silently change it"
```

---

## Task 4: A stage output is a tagged union

**GD-3: AMEND.** Spec §2 and §10. Two motivations, one change: `substantiate` annotates rather than
selects and so *"can be declared and cannot return"*; and the currency must be able to admit a second
member additively rather than as a breaking change.

**Files:**
- Modify: `crates/temper-core/src/types/query/envelope.rs:64-95`, `stage.rs`

**Interfaces:**
- Produces:
  ```rust
  /// What a stage produced. A tagged union with exactly ONE member today.
  ///
  /// Tagged from the first line so that admitting a second currency later is additive rather than
  /// breaking. It is NOT a claim that a second currency is coming — spec §10 refuses one for v0 and
  /// states the reason (a derived intention cannot be embedded inside a single statement).
  #[serde(rename_all = "snake_case", tag = "produced")]
  pub enum StageOutput { Ids { set: IdSet } }

  impl StageOutput {
      pub fn kind(&self) -> IdKind;
      pub fn len(&self) -> usize;
      pub fn is_empty(&self) -> bool;
  }
  ```
  and `ActResult.produced: StageOutput`, replacing `produced: IdSet`.

- [ ] **Step 1: Write the failing tests** in `envelope.rs`'s test module

```rust
#[test]
fn a_stage_output_is_tagged_so_a_second_currency_would_be_additive() {
    // The one-variant union is the whole point: an untagged IdSet could not grow without a
    // breaking change, and `substantiate` — which annotates rather than selects — has no shape to
    // return at all under the old field type.
    let o = StageOutput::Ids {
        set: IdSet { kind: IdKind::Region, provenance: None, ids: vec![] },
    };
    let json = serde_json::to_string(&o).unwrap();
    assert!(json.contains("\"produced\""), "the discriminator is present from day one");
    assert_eq!(serde_json::from_str::<StageOutput>(&json).unwrap(), o);
    assert_eq!(o.kind(), IdKind::Region);
    assert!(o.is_empty());
}

#[test]
fn a_result_still_declares_the_kind_it_produced_through_the_union() {
    // Contract chaining compares KINDS. Wrapping the set must not cost that.
    let r = ActResult {
        act: ActName::Survey,
        produced: StageOutput::Ids {
            set: IdSet { kind: IdKind::Region, provenance: None, ids: vec![] },
        },
        extent: Extent::Complete,
        total: None,
        terms_effective: BTreeMap::from([(BoundTerm::Regions, 3)]),
        narrowed_by: vec![],
        bounds_in: 0,
        bounds_honored: 0,
        bounds_dropped: 0,
    };
    assert_eq!(r.produced.kind(), IdKind::Region);
    assert_eq!(
        serde_json::from_str::<ActResult>(&serde_json::to_string(&r).unwrap()).unwrap(),
        r
    );
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-core --features mcp envelope::tests`
Expected: FAIL — `StageOutput` not found.

- [ ] **Step 3: Implement**

Declare `StageOutput` in `stage.rs` with the four derives; change `ActResult.produced`. Update
`envelope.rs`'s existing tests (`a_result_declares_the_kind_it_produced`,
`a_result_can_report_partial_without_paying_for_a_total`,
`a_traversal_result_reports_indeterminate_rather_than_guessing`,
`the_dropped_count_cannot_be_read_as_an_existence_oracle`) — all four construct `ActResult`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-core --features mcp envelope::tests`
Expected: PASS. **`the_dropped_count_cannot_be_read_as_an_existence_oracle` must still pass** — it is
the anti-oracle regression boundary and the wrapper must not change what the wire renders.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/query/
git commit -m "query: a stage output is a tagged union, so an annotating act has somewhere to land"
```

---

## Task 5: The two contract amendments, and artifact regeneration

**GD-3: AMEND.** Spec §9.1, *"Two shipped types are left with no producer, and they are treated
differently on a real distinction."*

**Files:**
- Modify: `crates/temper-core/src/types/query/disposition.rs:82-87`, `trace.rs:20-29`
- Modify (generated): `crates/temper-core/tests/fixtures/query/*.json`,
  `packages/temper-ui/src/lib/types/generated/query.ts`, `openapi.json`

- [ ] **Step 1: Write the failing test** in `disposition.rs`

```rust
#[test]
fn the_removed_expression_reason_is_no_longer_a_known_variant() {
    // There is no expression language, so nothing can raise this. `RefusalReason` is OPEN, so
    // removing it now costs nothing and re-adding it later is additive. Keeping a reason nothing
    // can raise is a claim about the system with no referent.
    let r: RefusalReason = serde_json::from_str("\"expression_not_pushdownable\"").unwrap();
    assert_eq!(r, RefusalReason::Other("expression_not_pushdownable".to_string()));
    assert!(!r.is_known(), "an old producer's value degrades to Other, it does not fail");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core --features mcp disposition::tests`
Expected: FAIL — parses as the known `ExpressionNotPushdownable` variant, so `is_known()` is true.

- [ ] **Step 3: Remove the variant and document the reserved one**

Delete `RefusalReason::ExpressionNotPushdownable`. In `trace.rs`, rewrite `BoundsSource::Expression`'s
doc comment to state that it is **reserved and currently unreachable**: `BoundsSource` is a closed
tagged enum, so removing it would make re-adding it breaking; Task 13 adds the test asserting no
compiled plan emits it. Reference spec §9.1.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core --features mcp disposition::tests`
Expected: PASS.

- [ ] **Step 5: Regenerate every artifact Tasks 1–5 restaled**

```bash
UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema
cargo make generate-ts-types
cargo make openapi
cargo make check
```

Expected: `cargo make check` PASSES. If `openapi-check`, `ts-rs-drift` or a schema snapshot reds,
read the `generated-artifacts` skill — do not hand-edit a generated file.

- [ ] **Step 6: Commit — one commit, all regenerated artifacts together**

```bash
git add -A
git commit -m "query: no expression language, so one reason goes and one variant is reserved"
```

---

# Beat B — validation

## Task 6: `ValidatedComposition` and topology

**GD-3: EXTEND.** Spec §5 authorizes the static layer, extending contract §4.1.1's static-refusal rule
from bound terms to topology.

**Files:**
- Create: `crates/temper-core/src/types/query/validate.rs`
- Modify: `mod.rs`

**Interfaces:**
- Consumes: `Composition`, `StageNode`, `StageName` (Tasks 1–3).
- Produces:
  ```rust
  /// One reason a plan is not executable. Static — no database was consulted.
  pub struct PlanRefusal {
      /// The stage it attaches to, when it attaches to one.
      pub stage: Option<StageName>,
      pub reason: RefusalReason,
      pub detail: String,
  }

  /// A composition that has passed every static check, in topological order.
  ///
  /// Parse-don't-validate: the field is private and `validate` is the only constructor, so a
  /// compiler that accepts this type cannot be handed an unvalidated plan.
  pub struct ValidatedComposition { /* private */ }

  impl ValidatedComposition {
      /// Nodes in dependency order — every node appears after all of its upstreams.
      pub fn ordered(&self) -> &[StageNode];
      pub fn composition(&self) -> &Composition;
      pub fn returns(&self) -> &[ReturnSpec];
  }

  /// ALL refusals, never the first — a caller fixing a plan should see every problem at once.
  pub fn validate(c: &Composition) -> Result<ValidatedComposition, Vec<PlanRefusal>>;
  ```

- [ ] **Step 1: Write the failing tests** in `validate.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Helpers keep the tests readable; each builds a minimal legal node.
    fn act(name: &str, a: ActName, input: Option<StageInput>) -> StageNode { /* build ActInvocation */ }
    fn plan(stages: Vec<StageNode>, returns: Vec<&str>) -> Composition { /* build Composition */ }

    #[test]
    fn a_cycle_is_refused_rather_than_compiled() {
        // A query over a graph must itself be acyclic. This is the check that makes topological
        // ordering total rather than best-effort.
        let c = plan(
            vec![
                act("a", ActName::FollowFrom, Some(StageInput::Upstream { stage: StageName::parse("b").unwrap() })),
                act("b", ActName::FollowFrom, Some(StageInput::Upstream { stage: StageName::parse("a").unwrap() })),
            ],
            vec!["a"],
        );
        let errs = validate(&c).unwrap_err();
        assert!(errs.iter().any(|e| e.detail.contains("cycle")), "got: {errs:?}");
    }

    #[test]
    fn a_reference_to_an_undeclared_stage_is_refused() {
        let c = plan(
            vec![act("near", ActName::FollowFrom, Some(StageInput::Upstream { stage: StageName::parse("ghost").unwrap() }))],
            vec!["near"],
        );
        let errs = validate(&c).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].stage.as_ref().unwrap().as_str(), "near");
    }

    #[test]
    fn two_stages_may_not_share_a_name() {
        let c = plan(
            vec![
                act("hits", ActName::FindExact, None),
                act("hits", ActName::FindAboutAnywhere, None),
            ],
            vec!["hits"],
        );
        assert!(validate(&c).is_err(), "a duplicate name makes every reference ambiguous");
    }

    #[test]
    fn a_returns_entry_naming_no_stage_is_refused() {
        let c = plan(vec![act("hits", ActName::FindExact, None)], vec!["ghost"]);
        assert!(validate(&c).is_err());
    }

    #[test]
    fn a_combinator_with_one_input_is_refused() {
        // One input is not a combination. Admitting it would let a plan express a no-op node that
        // reads as a merge.
        let c = plan(
            vec![
                act("hits", ActName::FindExact, None),
                StageNode::Combine(CombineNode {
                    name: StageName::parse("both").unwrap(),
                    op: CombineOp::Union,
                    inputs: vec![StageName::parse("hits").unwrap()],
                }),
            ],
            vec!["both"],
        );
        assert!(validate(&c).is_err());
    }

    #[test]
    fn every_refusal_is_reported_not_just_the_first() {
        // A caller repairing a plan should see all of it. Returning the first turns one round trip
        // into N.
        let c = plan(
            vec![act("near", ActName::FollowFrom, Some(StageInput::Upstream { stage: StageName::parse("ghost").unwrap() }))],
            vec!["also_missing"],
        );
        let errs = validate(&c).unwrap_err();
        assert!(errs.len() >= 2, "expected the dangling ref AND the bad return; got: {errs:?}");
    }

    #[test]
    fn a_valid_plan_comes_back_in_dependency_order() {
        let c = plan(
            vec![
                act("near", ActName::FollowFrom, Some(StageInput::Upstream { stage: StageName::parse("hits").unwrap() })),
                act("hits", ActName::FindExact, None),
            ],
            vec!["near"],
        );
        let v = validate(&c).expect("plan is legal");
        let names: Vec<&str> = v.ordered().iter().map(|n| n.name().as_str()).collect();
        assert_eq!(names, vec!["hits", "near"], "declaration order is not execution order");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-core --features mcp validate::tests`
Expected: FAIL — `validate.rs` does not exist.

- [ ] **Step 3: Implement the topology half**

Write the helpers, `PlanRefusal`, `ValidatedComposition` (private field, no public constructor), and
`validate` covering: duplicate names, dangling references, cycles, combinator arity, `returns`
resolution, and the topological sort. Declaration-driven checks are Task 7 — leave them out and do
not stub them.

Use `RefusalReason::Other(..)` for topology reasons that have no variant yet; Task 8 decides whether
any deserve one.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-core --features mcp validate::tests`
Expected: PASS, all seven.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/query/
git commit -m "query: a plan is validated before it is compiled, and every refusal is reported"
```

---

## Task 7: Declaration-driven refusals

**GD-3: CONFORM.** Conforms to `search_family()` as the single source of chainability — `registry.rs`'s
header: *"The chainability matrix is not a separate structure: it is the relation induced by each
declaration's `produces` against every other's `accepts_bounds` / `accepts_seeds`. Encoding it twice
would be the `ADMIN_EVENT_TYPES` failure."* **Read every declaration in `registry.rs` before writing
this task** — the values are the specification.

**Files:**
- Modify: `crates/temper-core/src/types/query/validate.rs`

**Interfaces:**
- Consumes: `search_family()` (`registry.rs:76`), `ActDeclaration`.
- Produces: no new public signature — `validate` gains checks.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_kind_the_act_does_not_accept_is_refused_against_the_registry() {
    // `find-exact` accepts bounds of kind `resource` only. Piping `survey`'s regions into it is a
    // category error the DECLARATIONS already know about — this check reads them, it does not
    // restate them.
    let c = plan(
        vec![
            act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap))),
            act("hits", ActName::FindExact, Some(StageInput::Upstream { stage: StageName::parse("shape").unwrap() })),
        ],
        vec!["hits"],
    );
    let errs = validate(&c).unwrap_err();
    assert!(errs.iter().any(|e| e.reason == RefusalReason::UnsupportedBoundKind), "got: {errs:?}");
}

#[test]
fn survey_declines_limit_because_its_bound_is_a_funnel_width() {
    // The worked case from contract §4.1.1: a term is never reinterpreted to fit. `survey` admits
    // `regions` and not `limit`, because `wayfind_region_scores` takes p_regions_n and has no rows
    // to limit.
    let mut node = act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap)));
    if let StageNode::Act(a) = &mut node { a.terms.insert(BoundTerm::Limit, 10); }
    let errs = validate(&plan(vec![node], vec!["shape"])).unwrap_err();
    assert!(errs.iter().any(|e| e.reason == RefusalReason::BoundTermNotApplicable));
}

#[test]
fn an_edge_filter_on_a_resource_only_act_is_declined_not_ignored() {
    let mut node = act("hits", ActName::FindExact, None);
    if let StageNode::Act(a) = &mut node {
        a.edge_filter = Some(EdgeFilter { edge_kinds: vec![EdgeKind::LeadsTo], labels: vec![] });
    }
    let errs = validate(&plan(vec![node], vec!["hits"])).unwrap_err();
    assert!(errs.iter().any(|e| e.reason == RefusalReason::FilterNotApplicable));
}

#[test]
fn a_find_about_stage_without_a_threaded_intention_refuses_rather_than_substituting() {
    // "I chose not to embed" and "I cannot embed" stay distinguishable. The server never quietly
    // embeds on the caller's behalf.
    let mut c = plan(vec![act("wide", ActName::FindAboutAnywhere, None)], vec!["wide"]);
    c.intention = None;
    let errs = validate(&c).unwrap_err();
    assert!(errs.iter().any(|e| e.reason == RefusalReason::MissingIntention));

    c.intention = Some(Intention { query: "salience".to_string(), embedded: true });
    assert!(validate(&c).is_ok(), "with an intention the same plan is legal");
}

#[test]
fn an_unbuilt_act_is_refused_as_not_implemented() {
    // `substantiate` is declared and unbuilt. A plan naming it is refused statically, never
    // attempted.
    let errs = validate(&plan(vec![act("ev", ActName::Substantiate, None)], vec!["ev"])).unwrap_err();
    assert!(errs.iter().any(|e| e.reason == RefusalReason::NotImplemented));
}

#[test]
fn a_region_set_without_provenance_is_refused() {
    // Context regions and cogmap regions are both RegionId and are NOT interchangeable — a context
    // region's id 404s at the sole consumer of region ids. Checked here, that is a declined plan;
    // unchecked, it is a rediscovered 404.
    let c = plan(
        vec![act("r", ActName::FollowFrom, Some(caller_ids_no_provenance(IdKind::Region)))],
        vec!["r"],
    );
    let errs = validate(&c).unwrap_err();
    assert!(errs.iter().any(|e| e.reason == RefusalReason::MissingProvenance));
}

#[test]
fn a_kind_changing_hop_is_expressible_so_the_region_phase_is_not_foreclosed() {
    // Spec §4 requirement 3. No v1 act changes kind, so without this test a resource-shaped
    // assumption could pass everything and quietly make region-mediated composition unbuildable.
    // `cogmap_list -> survey` is resource-free and legal TODAY with no SQL change.
    let c = plan(
        vec![act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap)))],
        vec!["shape"],
    );
    let v = validate(&c).expect("cogmap in, region out is a legal hop");
    assert_eq!(v.ordered().len(), 1);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-core --features mcp validate::tests`
Expected: FAIL — no declaration checks exist yet.

- [ ] **Step 3: Implement the declaration-driven checks**

Look each act up in `search_family()` and check the plan against its declared
`accepts_bounds` / `accepts_seeds` / `accepts_bound_terms` / `accepts_filters` / `build_state`.
`bound_ceilings` is **not** a refusal — a ceiling clamps and is disclosed through `terms_effective`
at execution (contract §4.1).

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-core --features mcp`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/query/validate.rs
git commit -m "query: the declarations decide what is chainable, and nothing restates them"
```

---

## Task 7b: The property predicate — open keys, closed operators

**GD-3: EXTEND.** Spec §12, authorized as a scope addition that *"grows the surface and does not
change its shape."* **Read §12 before starting** — the design turns on three measured facts (the
existing key-agnostic indexes, the open subject vocabulary, and the type-unstable keys), and none of
them is inferable from the code.

**Files:**
- Modify: `crates/temper-core/src/types/query/filter.rs`, `validate.rs`, `envelope.rs`

**Interfaces:**
- Produces:
  ```rust
  /// What a predicate addresses. OPEN, deliberately — `kb_properties.owner_table` is a varchar
  /// mirroring no DDL enum, so a closed set would be a claim the schema does not make. Contrast
  /// `EdgeKind`, which is closed BECAUSE it mirrors one.
  #[serde(rename_all = "snake_case")]
  pub enum PropertySubject {
      Resource,
      /// Empty in this deployment's data and NOT empty in others — the polymorphic owner is design
      /// intent, not accident. Spec §12.
      Edge,
      #[serde(untagged)]
      Other(String),
  }

  #[serde(rename_all = "snake_case", tag = "op")]
  pub enum PropertyOp {
      /// The key is present at all. A row-existence check on the `property_key` btree — NOT a jsonb
      /// operator, because `jsonb_path_ops` does not index key-existence and the btree already
      /// answers it.
      HasKey,
      /// `property_value @> $v` for any listed value. OR within the predicate, matching the
      /// established within-field OR of `doc_type` and `EdgeFilter.labels`.
      ///
      /// The caller supplies the JSON shape they mean. Containment does not coerce:
      /// `'["x"]'::jsonb @> '"x"'::jsonb` is FALSE, so a type-unstable key needs both shapes listed.
      Contains { values: Vec<serde_json::Value> },
  }

  pub struct PropertyPredicate {
      pub subject: PropertySubject,
      pub key: String,
      pub op: PropertyOp,
  }
  ```
  and `ActInvocation` gains `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub properties: Vec<PropertyPredicate>`.

> **`serde_json::Value` here is not a violation of the typed-structs rule.** The rule forbids
> `json!()` for data with a *known* structure. A property value's structure is the vault's, not
> ours — 71 keys spanning five JSON types, user-defined by design. Typing it would be the invention.

- [ ] **Step 1: Write the failing tests** in `filter.rs`

```rust
#[test]
fn a_property_subject_is_open_because_owner_table_is_a_varchar() {
    // The opposite call from EdgeKind, and principled rather than inconsistent: EdgeKind mirrors a
    // DDL enum so closedness is a FACT; owner_table mirrors nothing, so closedness would be a
    // claim the schema does not make.
    assert_eq!(serde_json::to_string(&PropertySubject::Edge).unwrap(), "\"edge\"");
    let unknown: PropertySubject = serde_json::from_str("\"block\"").expect("open, so it parses");
    assert_eq!(unknown, PropertySubject::Other("block".to_string()));
}

#[test]
fn has_key_and_contains_are_the_whole_v1_vocabulary() {
    // No operator takes a fragment of a query language. Both bind.
    let hk = PropertyPredicate {
        subject: PropertySubject::Resource,
        key: "keywords".to_string(),
        op: PropertyOp::HasKey,
    };
    let ct = PropertyPredicate {
        subject: PropertySubject::Edge,
        key: "confidence".to_string(),
        op: PropertyOp::Contains { values: vec![serde_json::json!("high")] },
    };
    for p in [hk, ct] {
        assert_eq!(
            serde_json::from_str::<PropertyPredicate>(&serde_json::to_string(&p).unwrap()).unwrap(),
            p
        );
    }
}

#[test]
fn contains_carries_a_list_so_one_predicate_spans_a_type_unstable_key() {
    // Measured: `derived_from` is an array on 112 resources and a string on 21. Containment does
    // not coerce, so a single-shape predicate silently answers for one population and not the
    // other. The list is what lets a caller ask for both.
    let p = PropertyPredicate {
        subject: PropertySubject::Resource,
        key: "derived_from".to_string(),
        op: PropertyOp::Contains {
            values: vec![serde_json::json!("abc"), serde_json::json!(["abc"])],
        },
    };
    let PropertyOp::Contains { values } = &p.op else { panic!("wrong op") };
    assert_eq!(values.len(), 2);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-core --features mcp filter::tests`
Expected: FAIL — `PropertyPredicate` not found.

- [ ] **Step 3: Implement the types**

Declare the three types in `filter.rs` with the four cfg-gated derives; add `properties` to
`ActInvocation`.

- [ ] **Step 4: Write the failing validation tests** in `validate.rs`

```rust
#[test]
fn a_content_block_subject_is_refused_because_blocks_are_addressable_not_queryable() {
    // Spec §12: block properties exist so provenance can attach to PART of a resource. That is
    // addressability, a different affordance from being a queryable subject.
    let c = plan_with_property(PropertySubject::Other("content_block".to_string()), "block_role", PropertyOp::HasKey);
    let errs = validate(&c).unwrap_err();
    assert!(errs.iter().any(|e| e.reason == RefusalReason::UnknownFilterValue));
}

#[test]
fn an_empty_property_key_is_refused_rather_than_matching_everything() {
    let c = plan_with_property(PropertySubject::Resource, "", PropertyOp::HasKey);
    assert!(validate(&c).is_err());
}

#[test]
fn contains_with_no_values_is_refused_because_it_narrows_nothing() {
    // An empty list is not "match all" and is not "match none" — it is a caller mistake, and
    // silently treating it as either is the confident-empty failure this contract exists to end.
    let c = plan_with_property(
        PropertySubject::Resource,
        "tags",
        PropertyOp::Contains { values: vec![] },
    );
    assert!(validate(&c).is_err());
}
```

- [ ] **Step 5: Run, implement the validation, run again**

Run: `cargo nextest run -p temper-core --features mcp validate::tests`
Expected: FAIL, then PASS after implementing.

- [ ] **Step 6: Regenerate artifacts and commit**

```bash
UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema
cargo make generate-ts-types && cargo make openapi && cargo make check
git add -A
git commit -m "query: properties are queryable — open keys, two bound operators"
```

---

## Task 8: The `BuildState` gap — a fused act the caller cannot reach

**GD-3: EXTEND.** Spec §7, *"The state between C and D, and the `BuildState` gap it lands on."*
Authorized because `RefusalReason` is open and the honest variant does not exist.

**Coordination:** the sibling's phase-1 work hits this same gap from the other side (an act whose
mechanic exists with no door). **Whoever lands first settles it; check before implementing** whether
phase 1 has already added a variant, and reuse it if so.

**Files:**
- Modify: `crates/temper-core/src/types/query/disposition.rs`, `validate.rs`

**Interfaces:**
- Produces: `RefusalReason::NotSeparablyReachable` — *the act is declared and its mechanic exists, but
  it is fused into a host this surface cannot invoke.*

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_fused_act_the_builder_cannot_invoke_refuses_honestly() {
    // Between beats C and D the three `find` acts are Fused{unified_search}: the mechanic exists
    // and the builder cannot reach into a composite to call it. `NotImplemented` is documented as
    // "build_state is not served or fused" and is FALSE here — the distinction BuildState cannot
    // currently draw is existence versus reachability-from-this-surface.
    let errs = validate(&plan(vec![act("hits", ActName::FindExact, None)], vec!["hits"])).unwrap_err();
    assert!(
        errs.iter().any(|e| e.reason == RefusalReason::NotSeparablyReachable),
        "a fused act must not be reported as unbuilt; got: {errs:?}"
    );
}

#[test]
fn survey_and_follow_from_are_not_refused_by_that_rule() {
    // Their mechanics are standalone SQL functions the builder calls directly. This is the test
    // that must go RED at beat D, when find-* become separately callable and the rule narrows.
    assert!(validate(&plan(
        vec![act("shape", ActName::Survey, Some(caller_ids(IdKind::Cogmap)))],
        vec!["shape"],
    )).is_ok());
}
```

- [ ] **Step 2: Run to verify the first fails**

Run: `cargo nextest run -p temper-core --features mcp validate::tests::a_fused_act`
Expected: FAIL — variant does not exist.

- [ ] **Step 3: Add the variant and the rule**

Add `NotSeparablyReachable` to `RefusalReason` with a doc comment stating the distinction and citing
spec §7. In `validate`, refuse an act whose `build_state` is `Fused { .. }` **unless** its `served_by`
function is one the builder can call directly. Maintain that set as a `const` in `validate.rs` with a
comment naming beat D as what shrinks it — **not** as a hardcoded act list, which would drift from the
declarations.

- [ ] **Step 4: Run to verify both pass**

Run: `cargo nextest run -p temper-core --features mcp`
Expected: PASS.

- [ ] **Step 5: Regenerate artifacts and commit**

```bash
UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema
cargo make generate-ts-types && cargo make openapi && cargo make check
git add -A
git commit -m "query: a fused act the builder cannot invoke refuses honestly, not as unbuilt"
```

---

# Beat C — the compiler

## Task 9: Identifier safety and CTE skeleton emission

**GD-3: EXTEND.** Spec §6. Pure string emission — **no database in this task**, which is what makes
the security property testable in isolation.

**Files:**
- Create: `crates/temper-substrate/src/readback/query_plan.rs`
- Create: `crates/temper-substrate/tests/query_plan_compile.rs`
- Modify: `crates/temper-substrate/src/readback/mod.rs` (add `pub mod query_plan;`)

**Interfaces:**
- Consumes: `ValidatedComposition` (Task 6).
- Produces:
  ```rust
  /// A compiled statement and its ordered binds. The SQL text is never concatenated with a caller
  /// value — every value is a positional bind.
  pub struct CompiledQuery {
      pub sql: String,
      pub binds: Vec<QueryBind>,
      /// Stage name -> CTE name, in emission order. Task 12 uses it to attribute rows to arms.
      pub cte_names: Vec<(String, String)>,
  }

  #[derive(Debug, Clone)]
  pub enum QueryBind {
      Profile(ProfileId),
      Uuids(Vec<Uuid>),
      Text(String),
      Int(i64),
      /// Rendered through `format_pgvector` and bound as `$n::vector`, the same treatment
      /// `unified_search` gives its embedding.
      Embedding(Vec<f32>),
  }

  pub fn compile(v: &ValidatedComposition, principal: ProfileId) -> CompiledQuery;
  ```

- [ ] **Step 1: Write the failing tests** in `tests/query_plan_compile.rs`

```rust
#[test]
fn every_caller_value_is_bound_and_none_is_interpolated() {
    // The security property, tested where it can be tested exhaustively: no database, no fixtures,
    // just the emitted text. A uuid appearing literally in the SQL is the failure.
    let (v, ids) = plan_with_caller_ids();
    let c = compile(&v, test_profile());
    for id in ids {
        assert!(!c.sql.contains(&id.to_string()), "id {id} was interpolated, not bound");
    }
    assert!(c.binds.iter().any(|b| matches!(b, QueryBind::Uuids(_))));
}

#[test]
fn the_only_identifiers_emitted_are_validated_stage_names() {
    // StageName::parse is the gate (Task 1). This asserts the emitter honours it rather than
    // formatting arbitrary strings into identifier position.
    let v = plan_two_stages("hits", "near");
    let c = compile(&v, test_profile());
    assert!(c.sql.contains("hits AS ("));
    assert!(c.sql.contains("near AS ("));
    assert_eq!(c.cte_names.len(), 2);
}

#[test]
fn the_visibility_relation_is_materialized_once_no_matter_how_many_stages() {
    // Decision 019fcd13: one query time, one visibility computation. A per-stage recomputation is
    // the thing the single statement exists to collapse.
    let one = compile(&plan_one_stage(), test_profile());
    let three = compile(&plan_three_stages(), test_profile());
    assert_eq!(one.sql.matches("vis AS MATERIALIZED").count(), 1);
    assert_eq!(three.sql.matches("vis AS MATERIALIZED").count(), 1);
}

#[test]
fn a_downstream_stage_selects_ids_only_and_never_a_quantity() {
    // THE rule that keeps no-cross-act-ranking structural (spec §4). If a quantity can cross a
    // stage boundary, cross-act arithmetic becomes mechanically easy and nothing prevents it.
    let v = plan_two_stages("hits", "near");
    let c = compile(&v, test_profile());
    let downstream = c.sql.split("near AS (").nth(1).expect("near CTE present");
    assert!(downstream.contains("SELECT id FROM hits"), "got: {downstream}");
    assert!(!downstream.contains("quantity FROM hits"), "a quantity crossed a stage boundary");
}

#[test]
fn stages_are_emitted_in_dependency_order() {
    // The compiler consumes ValidatedComposition::ordered(); a CTE referencing one declared later
    // would not parse.
    let v = plan_two_stages_declared_backwards("hits", "near");
    let c = compile(&v, test_profile());
    let hits_at = c.sql.find("hits AS (").unwrap();
    let near_at = c.sql.find("near AS (").unwrap();
    assert!(hits_at < near_at);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-substrate --test query_plan_compile`
Expected: FAIL — `query_plan` module does not exist.

- [ ] **Step 3: Implement `compile`**

Emit `WITH vis AS MATERIALIZED (...)` followed by one CTE per node in `ordered()` order, then the
final select. Bind every value positionally. Reuse `format_pgvector` from `readback/mod.rs` for
embeddings — do not write a second formatter.

For this task, act CTEs may emit a placeholder body that satisfies the shape assertions; Task 10 binds
them to real functions. **Do not leave a placeholder that could reach production** — gate it behind a
`todo!()` on execution, so the code cannot run while it compiles.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-substrate --test query_plan_compile`
Expected: PASS, all five.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-substrate/
git commit -m "query: compile a validated DAG to one statement, with every value bound"
```

---

## Task 10: Bind the two reachable acts, and amend the exception note

**GD-3: AMEND.** Amends `readback/mod.rs`'s runtime-`sqlx` exception note. Spec §6 — *"Amending that
module note to name the second class is a required step of the build, not documentation hygiene.
Left unamended, the note instructs the next cleanup sweep to claw the builder back exactly as it
clawed back the sixteen, and it would be right to."*

**Files:**
- Modify: `crates/temper-substrate/src/readback/query_plan.rs`
- Modify: `crates/temper-substrate/src/readback/mod.rs:26-34`

**Interfaces:**
- Consumes: the deployed signatures, which **must be re-read from the migrations before use**:
  `search_graph_expand(p_principal uuid, p_seed_ids uuid[], p_depth int, p_edge_types text[], p_gamma double precision)`
  and
  `wayfind_region_scores(p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int, p_anchor_table varchar, p_anchor_id uuid)`.
  These are carried from contract §3.3 `[verified — 2026-08-03]` and are **eight days old** — confirm
  against `\df` or the migration before binding.

- [ ] **Step 1: Verify the signatures against the live database**

```bash
psql "$DATABASE_URL" -c '\df search_graph_expand' -c '\df wayfind_region_scores'
```

Record what you find. If either differs from the plan, **the database wins** — bind what is there and
report the gap.

- [ ] **Step 2: Write the failing test** in `tests/query_plan_compile.rs`

```rust
#[test]
fn a_follow_from_stage_calls_the_deployed_graph_function_with_its_filter() {
    // The act's EdgeFilter narrows the WALK, inside the act, narrow-first — not a post-filter on
    // what came back. `edge_kinds` and `labels` are separate axes and must reach separate binds:
    // on live data `derived_from` spans two kinds and `relates_to` spans two, so merging them
    // would silently change the question.
    let v = plan_follow_from_with_edge_filter();
    let c = compile(&v, test_profile());
    assert!(c.sql.contains("search_graph_expand("));
    assert!(c.binds.iter().filter(|b| matches!(b, QueryBind::Text(_))).count() >= 1);
}

#[test]
fn a_survey_stage_binds_its_anchor_pair_and_its_funnel_width() {
    // survey consumes an anchor — (p_anchor_table, p_anchor_id) — which is exactly what an IdSet of
    // kind cogmap or context names. `regions` is its ONLY bound term.
    let v = plan_survey_over_cogmap();
    let c = compile(&v, test_profile());
    assert!(c.sql.contains("wayfind_region_scores("));
    assert!(c.binds.iter().any(|b| matches!(b, QueryBind::Int(_))), "p_regions_n is bound");
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo nextest run -p temper-substrate --test query_plan_compile`
Expected: FAIL — the placeholder body does not name either function.

- [ ] **Step 4: Bind the two act fragments**

Replace the placeholders with real calls, projecting each function's output into the
`(id, kind, quantity)` shape and aliasing the quantity to the act's declared `ActQuantity.field` in
the final select only.

- [ ] **Step 5: Amend the module note**

In `readback/mod.rs:26-34`, extend the exception paragraph to name **two** classes: the incumbent
`::vector` bind (three reads, one of which phase 1 retires), and dynamic composition
(`query_plan::compile`). State that the second class has exactly one member and that adding to it
needs the same deliberate justification the first required. Keep the 2026-07-30 claw-back history —
it is the reason the note works.

- [ ] **Step 6: Run to verify they pass**

Run: `cargo nextest run -p temper-substrate --test query_plan_compile`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-substrate/
git commit -m "query: bind survey and follow-from, and name the second runtime-sqlx class"
```

---

## Task 10b: Emit property predicates, and measure whether the OR uses the index

**GD-3: CONFORM.** Conforms to the three existing `kb_properties` indexes rather than adding any.
Spec §12 — *"the indexes are already there, and they are key-agnostic… a declared list would buy
nothing."*

**Files:**
- Modify: `crates/temper-substrate/src/readback/query_plan.rs`
- Modify: `crates/temper-substrate/tests/query_plan_compile.rs`
- Modify: `docs/superpowers/specs/2026-08-05-query-builder-compositional-design.md` (§12's open measurement)

- [ ] **Step 1: Measure the OR emission before choosing it**

Spec §12 flags this as unverified and a build step. Against a real database:

```bash
psql "$DATABASE_URL" -c "EXPLAIN SELECT owner_id FROM kb_properties WHERE NOT is_folded AND property_key = 'tags' AND property_value @> ANY(ARRAY['\"search\"','\"ci\"']::jsonb[])"
psql "$DATABASE_URL" -c "EXPLAIN SELECT owner_id FROM kb_properties WHERE NOT is_folded AND property_key = 'tags' AND (property_value @> '\"search\"'::jsonb OR property_value @> '\"ci\"'::jsonb)"
```

Record which form uses `idx_kb_properties_value_gin` and which falls back to a scan. **The measured
form is the one to emit.** Write the result into §12's blockquote with a `[verified — <date>]` tag,
replacing the open question.

> A local dev database has far less data than prod, so the planner may choose a seq scan for reasons
> of size rather than of index eligibility. Read the plan for **whether the index is considered**, not
> only for whether it is chosen — and say which you observed.

- [ ] **Step 2: Write the failing tests**

```rust
#[test]
fn a_has_key_predicate_binds_the_key_and_touches_no_jsonb_operator() {
    // has_key is a row-existence check on the property_key btree. Reaching for `?` would need a
    // second GIN index that jsonb_path_ops deliberately does not provide.
    let c = compile(&plan_with_has_key("keywords"), test_profile());
    assert!(!c.sql.contains("keywords"), "the key is bound, not interpolated");
    assert!(c.binds.iter().any(|b| matches!(b, QueryBind::Text(t) if t == "keywords")));
    assert!(!c.sql.contains(" ? "), "no key-existence operator");
}

#[test]
fn a_contains_predicate_binds_every_value_as_jsonb() {
    let c = compile(&plan_with_contains("tags", vec!["search", "ci"]), test_profile());
    assert!(c.sql.contains("@>"));
    assert!(!c.sql.contains("search"), "values are bound, not interpolated");
}

#[test]
fn a_property_predicate_on_an_edge_subject_targets_the_edge_owner() {
    // Edge-owned properties are empty in the community dataset and not in others. The emitted SQL
    // must scope by owner_table, or a resource property would satisfy an edge predicate.
    let c = compile(&plan_with_edge_property("confidence"), test_profile());
    assert!(c.sql.contains("owner_table"));
    assert!(c.binds.iter().any(|b| matches!(b, QueryBind::Text(t) if t == "kb_edges")));
}

#[test]
fn folded_properties_never_satisfy_a_predicate() {
    // Every kb_properties index is partial on `NOT is_folded`. Omitting the predicate both returns
    // retracted data and forfeits the index.
    let c = compile(&plan_with_has_key("keywords"), test_profile());
    assert!(c.sql.contains("is_folded"));
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo nextest run -p temper-substrate --test query_plan_compile`
Expected: FAIL.

- [ ] **Step 4: Implement emission in the measured form**

Emit predicates as `EXISTS` subqueries against `kb_properties`, AND-composed across predicates,
OR-composed within a `Contains` list per Step 1's measurement. Every predicate carries
`NOT is_folded` and an `owner_table` bind.

- [ ] **Step 5: Run to verify they pass, then commit**

```bash
cargo nextest run -p temper-substrate --test query_plan_compile
git add crates/temper-substrate/ docs/superpowers/specs/
git commit -m "query: emit property predicates against the indexes that already exist"
```

---

## Task 11: Spike — can `search_graph_expand` carry edge provenance?

**GD-3: This task decides between CONFORM and AMEND; it does not assume.** Spec §3 — the open check,
stated as unverified. **This is an investigation with a written outcome, not a feature.**

**The question:** `follow-from`'s mechanic is a `MAX(score) GROUP BY node` over the walk — it collapses
paths by construction. Can it emit `from_id` / `edge_kind` / `label` for the surviving path, or for
all paths, without changing its body?

**Files:**
- Modify: the spec's §3 open-check paragraph, with the answer.
- Possibly create: a migration adding a provenance-carrying variant.

- [ ] **Step 1: Read the deployed body**

```bash
psql "$DATABASE_URL" -c "SELECT pg_get_functiondef('search_graph_expand'::regproc)"
```

- [ ] **Step 2: Determine which of three states holds**

1. **Provenance is already available** — the walk retains the parent and the function can project it
   with no body change. CONFORM: bind it in Task 12.
2. **Available with an additive change** — a new function or an added output column, leaving the
   existing one untouched. AMEND: write the migration, additive-only (`main` auto-deploys and the
   build enforces additivity).
3. **Not available without restructuring the walk.** Report and **defer `via` to a later task** —
   ship arms without provenance rather than shipping first-wins, which is a silent lossy pick and is
   named in the spec as the wrong answer.

- [ ] **Step 3: Write the answer into the spec**

Replace §3's *"is not verified here"* paragraph with what you found, tagged
`[verified — <date>]`, including the state number and the evidence. If state 3, add it to §10's
declared holes.

- [ ] **Step 4: Commit the finding**

```bash
git add docs/superpowers/specs/2026-08-05-query-builder-compositional-design.md migrations/ 2>/dev/null
git commit -m "query: answer the edge-provenance check against the deployed walk"
```

---

## Task 12: Execute, project per kind, and assemble per-arm results

**GD-3: EXTEND.** Spec §3. Follows `substrate_read.rs`'s service-direct read pattern
`[verified — search_select at substrate_read.rs:881]`.

**Files:**
- Create: `crates/temper-services/src/services/query_service.rs`
- Create: `crates/temper-services/tests/query_service_test.rs`
- Modify: `crates/temper-services/src/services/mod.rs`

**Interfaces:**
- Consumes: `validate` (Task 6), `compile` (Task 9).
- Produces:
  ```rust
  /// One returned stage's rows, ordered by that stage's own quantity and nothing else.
  pub struct ResultArm {
      pub stage: String,
      pub kind: IdKind,
      /// The act's declared quantity — its deployed column name and range — or None for a
      /// combinator, which has no order to claim.
      pub ordered_by: Option<ActQuantity>,
      pub extent: Extent,
      pub rows: Vec<serde_json::Value>,
  }

  pub struct QueryResponse {
      pub arms: Vec<ResultArm>,
      pub trace: CompositionTrace,
  }

  pub async fn query_execute(
      pool: &PgPool,
      profile_id: ProfileId,
      composition: Composition,
  ) -> ApiResult<QueryResponse>;
  ```

> `rows` is `Vec<serde_json::Value>` because the shape is per-kind and caller-subselected. This is
> the one place the repo's typed-structs rule yields, and it yields to a *projection*, not to a known
> structure. The **kind** and its default field set are typed; the subselected row is not.

- [ ] **Step 1: Write the failing tests** in `tests/query_service_test.rs`

```rust
#![cfg(feature = "test-db")]

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_two_arm_composition_returns_both_arms_with_no_shared_number(pool: PgPool) {
    // THE demonstration: a resource arm and a region arm side by side, each ordered by its own
    // declared quantity on its own scale, and no field anywhere ranking one against the other.
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    let resp = query_execute(&pool, profile, two_arm_plan(cogmap)).await.unwrap();

    assert_eq!(resp.arms.len(), 2);
    let region_arm = resp.arms.iter().find(|a| a.kind == IdKind::Region).unwrap();
    let resource_arm = resp.arms.iter().find(|a| a.kind == IdKind::Resource).unwrap();

    // Each names its own quantity, and the two are NOT the same field.
    let rq = region_arm.ordered_by.as_ref().unwrap();
    let sq = resource_arm.ordered_by.as_ref().unwrap();
    assert_ne!(rq.field, sq.field);

    // No row in either arm carries a field that could be compared across arms.
    for arm in &resp.arms {
        for row in &arm.rows {
            let o = row.as_object().unwrap();
            assert!(!o.contains_key("combined_score"), "the summed field must not reappear");
            assert!(!o.contains_key("score"), "a bare score invites arithmetic across arms");
        }
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_interstitial_stage_is_traced_but_not_hydrated(pool: PgPool) {
    // returns decides what is HYDRATED; the trace decides what is DISCLOSED. A stage that feeds
    // another and is not returned must still appear, with its disposition and counts.
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    let resp = query_execute(&pool, profile, two_stage_plan_returning_only_the_last(cogmap))
        .await
        .unwrap();
    assert_eq!(resp.arms.len(), 1, "only the declared stage is hydrated");
    assert_eq!(resp.trace.stages.len(), 2, "both stages are disclosed");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_region_row_carries_region_fields_and_not_resource_fields(pool: PgPool) {
    // Per-kind projection: a region has no doc_type, and pretending otherwise would put a
    // mostly-null wide row on the wire.
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    let resp = query_execute(&pool, profile, survey_only_plan(cogmap)).await.unwrap();
    let row = resp.arms[0].rows.first().expect("the seeded cogmap has a live region");
    let o = row.as_object().unwrap();
    assert!(o.contains_key("member_count"));
    assert!(!o.contains_key("doc_type"));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_field_subselect_narrows_what_comes_back(pool: PgPool) {
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    let resp = query_execute(&pool, profile, survey_plan_selecting_only_label(cogmap)).await.unwrap();
    let o = resp.arms[0].rows[0].as_object().unwrap();
    assert!(o.contains_key("label"));
    assert!(!o.contains_key("member_count"), "an unselected field is not sent");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_invisible_caller_supplied_id_contributes_an_honest_absence(pool: PgPool) {
    // Caller ids are fully distrusted and SILENTLY excluded — a loud refusal would make the
    // surface a probe for existence. The caller learns the count that did not contribute, and
    // nothing about why.
    let (profile, _) = seed_principal_with_cogmap(&pool).await;
    let other = seed_unrelated_principals_resource(&pool).await;
    let resp = query_execute(&pool, profile, follow_from_plan_seeded_with(vec![other])).await.unwrap();
    let t = &resp.trace.stages[0];
    assert_eq!(t.bounds_in, 1);
    assert_eq!(t.bounds_honored, 0);
    assert_eq!(t.bounds_dropped, 1);
    assert_eq!(t.disposition, StageDisposition::Empty, "empty, never withheld or refused");
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo make docker-up
cargo nextest run -p temper-services --features test-db --test query_service_test
```
Expected: FAIL — `query_service` does not exist.

- [ ] **Step 3: Implement `query_execute` and the seed helpers**

Orchestrate: validate → compile → execute → project per kind → assemble arms + trace. Errors from
`validate` become a `400` carrying **every** `PlanRefusal`. Reuse `reject_degenerate_embedding` from
`substrate_read.rs:863` rather than writing a second check.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db --test query_service_test`
Expected: PASS, all five.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/
git commit -m "query: execute a composition and return arms that share no number"
```

---

## Task 13: The trace, and the reserved variant that must stay unreachable

**GD-3: CONFORM.** Conforms to contract §4.4 tier 1 — *"mandatory, never truncated, no knob turns it
off"*. Spec §3, §9.1.

**Files:**
- Modify: `crates/temper-services/src/services/query_service.rs`
- Modify: `crates/temper-services/tests/query_service_test.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn no_compiled_plan_ever_emits_the_reserved_bounds_source(pool: PgPool) {
    // BoundsSource::Expression is RESERVED and unreachable — there is no expression language. The
    // enum is closed, so removing it would make re-adding it breaking; this test is what turns
    // "reserved" from an unfalsifiable claim into a checked one.
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    for plan in every_shape_we_can_build(cogmap) {
        let resp = query_execute(&pool, profile, plan).await.unwrap();
        for s in &resp.trace.stages {
            assert!(
                !matches!(s.bounds_source, Some(BoundsSource::Expression)),
                "stage {} claimed a compiled predicate that cannot exist",
                s.stage
            );
        }
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_stage_fed_by_another_records_which_one(pool: PgPool) {
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    let resp = query_execute(&pool, profile, two_stage_plan_returning_only_the_last(cogmap))
        .await
        .unwrap();
    let downstream = resp.trace.stages.iter().find(|s| s.stage == 1).unwrap();
    assert!(matches!(downstream.bounds_source, Some(BoundsSource::Upstream { stage: 0 })));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_survey_stage_reports_indeterminate_rather_than_guessing_a_total(pool: PgPool) {
    // A region-salience traversal has no size prior to its own funnel width. Reporting Complete
    // would be a claim the mechanic cannot support.
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    let resp = query_execute(&pool, profile, survey_only_plan(cogmap)).await.unwrap();
    assert!(matches!(resp.arms[0].extent, Extent::Indeterminate { .. }));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_trace_is_present_at_every_meta_detail_including_none(pool: PgPool) {
    // Tier 1 has no knob. `MetaDetail::None` suppresses per-id participation, never the per-stage
    // record — a composition that could hide which stages ran would fail composition-is-legible.
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    for detail in [MetaDetail::None, MetaDetail::Surviving, MetaDetail::Full] {
        let resp = query_execute(&pool, profile, plan_with_detail(cogmap, detail)).await.unwrap();
        assert_eq!(resp.trace.stages.len(), 2, "detail {detail:?} suppressed a stage record");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db --test query_service_test`
Expected: FAIL.

- [ ] **Step 3: Implement trace assembly**

One `StageTrace` per node in `ordered()`, whatever its disposition. `bounds_source` is
`Upstream { stage }` for a stage-referenced input and `Caller` for caller ids — **never** `Expression`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db --test query_service_test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/
git commit -m "query: every stage is disclosed, and the reserved bounds-source stays unreachable"
```

---

## Task 14: The generative `EXPLAIN` harness

**GD-3: EXTEND.** Spec §6 obligation 3 — the substitute for compile-time checking. Decision
`019fcd13`: *"a single compiled statement concentrates all risk in the query plan."*

**Files:**
- Create: `crates/temper-services/tests/query_plan_explain_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
#![cfg(feature = "test-db")]

/// Every DAG shape the declarations permit, up to a bounded size, must compile to a statement
/// Postgres can plan. This is what replaces `query!`'s compile-time check — and it catches
/// something the macro never could, since a statement that type-checks can still plan badly.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_legal_shape_compiles_to_a_plannable_statement(pool: PgPool) {
    let (profile, cogmap) = seed_principal_with_cogmap(&pool).await;
    let shapes = enumerate_legal_shapes(cogmap); // acts x inputs x combinators, depth <= 3
    assert!(shapes.len() >= 12, "the enumerator is not exercising the space: {}", shapes.len());

    for (i, plan) in shapes.into_iter().enumerate() {
        let v = validate(&plan).unwrap_or_else(|e| panic!("shape {i} was enumerated as legal: {e:?}"));
        let c = compile(&v, profile);
        let mut q = sqlx::query(&format!("EXPLAIN {}", c.sql));
        q = bind_all(q, &c.binds);
        q.fetch_all(&pool)
            .await
            .unwrap_or_else(|e| panic!("shape {i} did not plan: {e}\n{}", c.sql));
    }
}

/// A shape the declarations REFUSE must never reach the compiler. This is the negative half —
/// without it the harness only proves that legal things work.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn no_illegal_shape_survives_validation(pool: PgPool) {
    let (_, cogmap) = seed_principal_with_cogmap(&pool).await;
    for (i, plan) in enumerate_illegal_shapes(cogmap).into_iter().enumerate() {
        assert!(validate(&plan).is_err(), "illegal shape {i} validated");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db --test query_plan_explain_test`
Expected: FAIL — the enumerators do not exist.

- [ ] **Step 3: Implement the enumerators**

Derive legal shapes from `search_family()` — do not hand-list them, or the harness stops covering new
acts the moment one is declared. Illegal shapes come from deliberately violating one declared
constraint at a time.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db --test query_plan_explain_test`
Expected: PASS.

- [ ] **Step 5: Full check and commit**

```bash
cargo make check && cargo make test-db
git add crates/temper-services/
git commit -m "query: every legal shape plans, and every illegal one is refused before it compiles"
```

---

## Self-review notes

**Spec coverage.** §1 → Task 1. §2 → Tasks 1, 2, 4. §3 → Tasks 3, 11, 12, 13. §4 → Tasks 9, 12 (and
Task 7's kind-changing-hop test covers requirement 3; requirement 2 is Task 7's provenance test).
§5 → Tasks 6, 7, 8. §6 → Tasks 9, 10, 14. §7 → Task 8, plus the beat structure. §8 → the measured
numbers inform Task 12's fixtures. §9 → no task; it is a recorded decision with nothing to build.
§9.1 → Tasks 5, 13. §10 → Task 4 (tagged union), Task 11 (the open check). §12 → Tasks 7b, 10b.

**Known gaps, stated rather than left to be discovered:**

1. **The validate-only path** (spec §5, *"a validate-only path is published"*) has **no task here** —
   it is a door, and doors are beat E. `validate` is public and pure, so beat E's plan wires it with
   no new logic. Named so it is not mistaken for covered.
2. **`RefusalDisposition` (`Halt` / `DegradeAndDisclose`) is not exercised** by any task. With only
   `survey` and `follow-from` reachable, no runtime stage refusal is reachable to test it against —
   a refusal today is static, and static refusals fail the whole plan. It becomes testable at beat D.
   **Declared uncovered, not silently skipped.**
3. **Tier-2 `MetaDetail::Full` per-id participation** is asserted only for trace presence (Task 13),
   not for its retained content. Full coverage needs a plan where ids drop mid-composition, which
   needs a bounding act — beat D.
4. **`PropertyOp::WeightAtLeast` is designed-for and not built.** Spec §12 records the decision to
   build it as a later phase; `PropertyOp` is an enum with room for it, and no index serves it today.
   Whether one is needed is a measurement that belongs with its build.
5. **Edge-owned property predicates compile and are untested against real edge properties**, because
   the community dataset has none (spec §12). Task 10b asserts the emitted SQL scopes by
   `owner_table`; it cannot assert a match. **Declared uncovered** — an integration test needs a
   fixture that writes an edge property, which is worth adding when the first such deployment is
   exercised, not simulated here.
