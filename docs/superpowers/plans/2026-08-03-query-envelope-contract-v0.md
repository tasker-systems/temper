# Query-Envelope Contract v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the v0 query-envelope contract for Temper's search family — the act declarations, the typed `IdSet` currency, the predicate layer, the composition envelope and trace, and the refusal variants — **as Rust types**, with a committed JSON-Schema snapshot gate.

**The contract is not a document this plan writes.** Its destination is Temper's published, generated API spec, so hand-authoring one here would create the second copy the whole design forbids. The published artifact and its usability review are **T3's** work; this plan ships the types it is generated from.

**Architecture:** Types live in `crates/temper-core/src/types/query/`, one file per responsibility, following the crate's existing feature-gated derive pattern. The act *registry* is data (declarations + a chainability matrix), not code paths — T3 later gates that data against the live router and the SQL signatures. **No schemas are hand-written**: schemars emits them from the Rust types into committed snapshots, which is what makes the artifact a projection of the code rather than a second copy of it.

**Tech Stack:** Rust, serde 1.0.228, schemars 1.2.1, ts-rs, utoipa, cargo-nextest, cargo-make.

**Source of truth for shape:** `docs/superpowers/specs/2026-08-03-query-envelope-contract-v0-design.md`. If this plan and that spec disagree, the spec wins — amend the spec rather than diverging silently.

## Global Constraints

Every task's requirements implicitly include this section.

**Contract invariants — copied verbatim from the spec:**

- *"Only an `IdSet` crosses a stage boundary."* Per-act `meta` is terminal — produced, disclosed, never consumed as a later stage's input.
- *"`kind` is OPEN"* — an unknown kind must **deserialize successfully** so the act layer can render a typed `refused`. A parse error is the wrong failure: it cannot carry a reason.
- *"the `disposition` enum is closed"* — an unknown disposition must **fail** to deserialize. Its closedness is what lets consumers match exhaustively.
- *"Kinds are domain-named, not table-named."* `resource`, never `kb_resources`.
- *"An accumulation cap is disclosed, always."*
- *"No bare `score`"* — every quantity carries its act's name: `lexical_rank`, `vector_affinity`, `region_salience`, `graph_adjacency`.
- *"Nothing in the search family is `served` today. That is the finding, not an omission."*

**Repo constraints:**

- **Typed structs over inline JSON.** Never `serde_json::json!()` for data with a known structure.
- **Derive pattern**, copied from `crates/temper-core/src/types/ids.rs` — written here in the form
  **rustfmt actually produces**, since the single-line `schemars` variant is 101 chars and every
  sketch in this plan reproduces it. Write it wrapped and skip the `cargo make fix` round-trip:
  ```rust
  #[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
  #[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
  #[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
  #[cfg_attr(
      any(feature = "mcp", feature = "scenario-schema"),
      derive(schemars::JsonSchema)
  )]
  ```
- **`Option<T>` + `skip_serializing_if` emits a ts-rs warning, and that is the crate's convention.**
  ts-rs cannot parse the combined attribute and ignores it, so the TS field is `T | null` rather
  than optional. 71 such warnings pre-exist in temper-core and `ts(optional)` appears nowhere in
  the crate `[verified — 2026-08-03]`. Conform; do not "fix" it into an outlier.
- **Do NOT register these types in `crates/temper-api/src/openapi.rs` components.** T2 ships no route. The audit already found `SearchResultRow` shipped as *"a published wire type with no producer"* into `openapi.json`, both TS trees and the Ruby gem — do not manufacture a second one. T3 registers them when routes exist.
- **After any task that adds a ts-rs derive**: run `cargo make generate-ts-types`, commit the regenerated tree, then run `cd packages/temper-ui && bun run check`. `cargo make check` does **not** cover temper-ui.
- **Schema snapshots are package-scoped, never `--workspace`.** Feature unification changes the emitted schema: under `--workspace` temper-core's `mcp` feature unifies in and id newtypes emit **inline**; package-scoped they emit as `$ref`s. See the comment block at `tools/cargo-make/main.toml:91`.
- `cargo make check` before claiming any task complete.
- Branch: `jct/query-envelope-contract-v0-spec` (continues from the design commits `f302d047`, `a0ee3fef`).

**Out of scope for every task in this plan:** the executor, any SQL, any act implementation, any route or MCP tool, converging the six incumbent tagged-id patterns (design §3.1.2), and the four non-search families.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/temper-core/src/types/query/mod.rs` | module wiring + re-exports |
| `crates/temper-core/src/types/query/id_set.rs` | `IdKind`, `IdProvenance`, `IdSet` — the currency |
| `crates/temper-core/src/types/query/scalars.rs` | `Extent`, `BoundTerm`, `BoundsMode`, `MetaDetail` |
| `crates/temper-core/src/types/query/filter.rs` | `EdgeFilter`, `ResourceFilter`, `FacetPredicate`, `FilterField` — **re-uses** `types::graph::EdgeKind` |
| `crates/temper-core/src/types/query/disposition.rs` | `StageDisposition`, `ActRefusal`, `RefusalDisposition` |
| `crates/temper-core/src/types/query/act.rs` | `ActName`, `BuildState`, `VisibilityProfile`, `ActDeclaration` |
| `crates/temper-core/src/types/query/registry.rs` | the seven declarations + chainability matrix, as data |
| `crates/temper-core/src/types/query/envelope.rs` | `ActInvocation`, `ActResult`, `NarrowedBy` |
| `crates/temper-core/src/types/query/trace.rs` | `StageTrace`, `BoundsSource`, `MetaTruncated`, `CompositionTrace` |
| `crates/temper-core/src/types/query/composition.rs` | `Composition`, `Intention`, `OutcomeDeclaration` |
| `crates/temper-core/tests/query_schema.rs` | committed JSON-Schema snapshot gate |
| `crates/temper-core/tests/fixtures/query/*.schema.json` | the snapshots |

---

### Task 1: `IdSet` — the typed currency

**Files:**
- Create: `crates/temper-core/src/types/query/mod.rs`, `crates/temper-core/src/types/query/id_set.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (add `pub mod query;`)

**Interfaces:**
- Consumes: `uuid::Uuid`; `crate::types::ids::{CogmapId, ContextId}`.
- Produces: `IdKind`, `IdProvenance`, `IdSet`. Every later task uses `IdSet` as the only cross-stage value.

- [ ] **Step 1: Write the failing tests**

Create `crates/temper-core/src/types/query/id_set.rs` with only this test module at the bottom (implementation comes in step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_serializes_domain_named_not_table_named() {
        assert_eq!(serde_json::to_string(&IdKind::Resource).unwrap(), "\"resource\"");
        assert_eq!(serde_json::to_string(&IdKind::Region).unwrap(), "\"region\"");
        assert_eq!(serde_json::to_string(&IdKind::Cogmap).unwrap(), "\"cogmap\"");
        assert_eq!(serde_json::to_string(&IdKind::Context).unwrap(), "\"context\"");
    }

    #[test]
    fn unknown_kind_deserializes_rather_than_erroring() {
        // The vocabulary is OPEN: a newer producer must not break an older consumer, and the
        // refusal must be renderable with a reason rather than surfacing as a parse error.
        let k: IdKind = serde_json::from_str("\"block\"").expect("unknown kind must parse");
        assert_eq!(k, IdKind::Other("block".to_string()));
        assert!(!k.is_known());
    }

    #[test]
    fn id_set_round_trips_without_provenance() {
        let u = uuid::Uuid::now_v7();
        let s = IdSet { kind: IdKind::Resource, provenance: None, ids: vec![u] };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("provenance"), "absent provenance must not serialize");
        assert_eq!(serde_json::from_str::<IdSet>(&json).unwrap(), s);
    }

    #[test]
    fn region_set_carries_its_anchor_provenance() {
        // Context regions and cogmap regions are both `region` and are NOT interchangeable:
        // graph_region_composition gates on cogmap_readable_by_profile and a context region's
        // cogmap_id is NULL. The kind tag alone would admit a chain that always 404s.
        let m = CogmapId::new();
        let s = IdSet {
            kind: IdKind::Region,
            provenance: Some(IdProvenance::Cogmap(m)),
            ids: vec![uuid::Uuid::now_v7()],
        };
        let back: IdSet = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert!(!back.provenance.unwrap().is_context());
    }

    #[test]
    fn provenance_distinguishes_the_two_region_anchors() {
        let c = IdProvenance::Context(ContextId::new());
        let m = IdProvenance::Cogmap(CogmapId::new());
        assert!(c.is_context());
        assert!(!m.is_context());
        assert_ne!(serde_json::to_string(&c).unwrap(), serde_json::to_string(&m).unwrap());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core id_set`
Expected: FAIL — compile error, `IdKind` / `IdProvenance` / `IdSet` not found.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/temper-core/src/types/query/id_set.rs`:

```rust
//! The query-envelope currency: a typed, tagged set of ids.
//!
//! The tag is carried as DATA, not as a Rust newtype. `crates/temper-core/src/types/ids.rs`
//! already defines 17 typed ids, but the `define_id!` macro applies `#[serde(transparent)]`, so
//! every one of them serializes as a bare uuid string. This contract is a wire contract and jaq
//! operates on JSON — a newtype would give the chaining check nothing to check.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::ids::{CogmapId, ContextId};

/// What an [`IdSet`]'s ids name.
///
/// OPEN vocabulary: an unrecognized kind parses into [`IdKind::Other`] so the act layer can
/// refuse it with a reason. Domain-named, never table-named — this deliberately diverges from
/// `LedgerRefKind`, which renames every variant to its SQL table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum IdKind {
    Resource,
    Region,
    Cogmap,
    Context,
    /// An unrecognized kind. Never constructed by this crate — only by deserializing a producer
    /// newer than this consumer.
    #[serde(untagged)]
    Other(String),
}

impl IdKind {
    /// Whether this is a kind v0 closed the vocabulary at (design §3.1.1).
    pub fn is_known(&self) -> bool {
        !matches!(self, IdKind::Other(_))
    }
}

/// Which anchor produced a set of region ids.
///
/// Load-bearing for exactly one kind today. Mirrors the shape of
/// [`crate::types::home::HomeAnchor`] without reusing it: that type has no wire derives and is
/// deliberately internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "anchor", content = "id")]
pub enum IdProvenance {
    Cogmap(CogmapId),
    Context(ContextId),
}

impl IdProvenance {
    pub fn is_context(&self) -> bool {
        matches!(self, IdProvenance::Context(_))
    }
}

/// The one value that crosses a stage boundary. Membership, never rank.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct IdSet {
    pub kind: IdKind,
    /// Required for `region`; absent for every other kind today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<IdProvenance>,
    pub ids: Vec<Uuid>,
}
```

Create `crates/temper-core/src/types/query/mod.rs`:

```rust
//! The v0 query-envelope contract. These types ARE the contract: the published spec is generated
//! from them in T3 and ships with the other generated artifacts, so there is no hand-written
//! second copy. See `docs/superpowers/specs/2026-08-03-query-envelope-contract-v0-design.md` for
//! the design reasoning.

pub mod id_set;

pub use id_set::{IdKind, IdProvenance, IdSet};
```

Add `pub mod query;` to `crates/temper-core/src/types/mod.rs`, keeping the existing alphabetical placement (between `pub mod profile;` and `pub mod relationship_events;` — confirm the neighbours by reading the file).

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core id_set`
Expected: PASS, 5 tests.

If `#[serde(untagged)]` on a single variant fails to compile, the serde version is older than 1.0.181 — verify with `rg -n '^name = "serde"' -A 2 Cargo.lock` (expected 1.0.228) before considering a hand-written `Deserialize`.

- [ ] **Step 5: Regenerate TypeScript and check the UI**

```bash
cargo make generate-ts-types
cd packages/temper-ui && bun install && bun run check && cd ../..
```
Expected: both clean. A `d3-*` "implicit any" failure is a stale `node_modules`, not your change.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/query/ crates/temper-core/src/types/mod.rs packages/temper-ui/src/lib/types/generated/
git commit -m "feat(query): IdSet — the typed, tagged currency"
```

---

### Task 2: Envelope scalars — `Extent`, `BoundTerm`, `BoundsMode`, `MetaDetail`

**Files:**
- Create: `crates/temper-core/src/types/query/scalars.rs`
- Modify: `crates/temper-core/src/types/query/mod.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `Extent`, `BoundTerm`, `BoundsMode`, `MetaDetail`. Task 5 puts `BoundTerm` on `ActDeclaration`; Task 7 puts `Extent` and `BoundTerm` on the envelopes; Task 8 uses `MetaDetail` and `BoundTerm`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extent_distinguishes_complete_from_partial_from_uncountable() {
        // The clause needs "this is everything" vs "this is some of it" — NOT a total. `Partial`
        // is answerable with a limit+1 probe; a total would cost a second query per stage.
        let all = [
            Extent::Complete,
            Extent::Partial,
            Extent::Indeterminate { reason: "candidate set is produced by the bound".to_string() },
        ];
        let rendered: Vec<String> = all.iter().map(|e| serde_json::to_string(e).unwrap()).collect();
        // All three are mutually distinguishable.
        assert_eq!(rendered.len(), 3);
        assert_ne!(rendered[0], rendered[1]);
        assert_ne!(rendered[1], rendered[2]);
        for (e, j) in all.iter().zip(rendered.iter()) {
            assert_eq!(&serde_json::from_str::<Extent>(j).unwrap(), e);
        }
    }

    #[test]
    fn extent_never_serializes_as_bare_null() {
        // A nullable would collapse "complete" with "could not tell" — the
        // is_stale-on-a-never-materialized-map ambiguity, one family over.
        assert_ne!(serde_json::to_string(&Extent::Complete).unwrap(), "null");
    }

    #[test]
    fn bound_terms_each_have_exactly_one_meaning() {
        // `limit` means rows, always. `regions` means funnel width, always. A term is never
        // reinterpreted per act — an act that cannot serve a term DECLINES it (Task 4/5).
        assert_eq!(serde_json::to_string(&BoundTerm::Limit).unwrap(), "\"limit\"");
        assert_eq!(serde_json::to_string(&BoundTerm::Offset).unwrap(), "\"offset\"");
        assert_eq!(serde_json::to_string(&BoundTerm::Regions).unwrap(), "\"regions\"");
    }

    #[test]
    fn bound_term_is_closed_unknown_terms_fail_to_parse() {
        // Closed on purpose: a term whose meaning is not fixed by the contract is exactly the
        // thing `the-same-bound-term-means-the-same-thing-on-every-read` forbids.
        assert!(serde_json::from_str::<BoundTerm>("\"page_size\"").is_err());
    }

    #[test]
    fn bounds_mode_round_trips_both_directions() {
        assert_eq!(serde_json::to_string(&BoundsMode::Bound).unwrap(), "\"bound\"");
        assert_eq!(serde_json::to_string(&BoundsMode::Seed).unwrap(), "\"seed\"");
    }

    #[test]
    fn meta_detail_defaults_to_surviving() {
        assert_eq!(MetaDetail::default(), MetaDetail::Surviving);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core scalars`
Expected: FAIL — `Extent` not found.

- [ ] **Step 3: Write the implementation**

```rust
//! Base-envelope scalars shared by every act.

use serde::{Deserialize, Serialize};

/// Whether the caller received everything that matched.
///
/// NOT a total. A total costs a second query — the standing tax of pagination — and across a chain
/// that tax is paid per stage; for a whole composition it is not even well-defined, because each
/// stage's output is the next stage's domain. `Partial` is answerable with a `limit + 1` probe.
///
/// This is what `every-bound-a-read-applies-is-visible-in-its-answer` actually asks for: the
/// ability to distinguish "this is everything" from "this is some of it".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "extent")]
pub enum Extent {
    /// Everything that matched is here.
    Complete,
    /// More exists beyond what was returned.
    Partial,
    /// Neither is determinable: the candidate set is *produced by* the bound rather than selected
    /// under it. `survey` is the worked case — a region-salience traversal has no size prior to
    /// its own funnel width.
    Indeterminate { reason: String },
}

/// A bound term. CLOSED, and each term has exactly one meaning on every read: `limit` is rows,
/// `offset` is rows skipped, `regions` is funnel width.
///
/// An act that cannot serve a term does NOT reinterpret it — it declines it
/// (`RefusalReason::BoundTermNotApplicable`), decided statically against the schema before
/// execution. That is `the-same-bound-term-means-the-same-thing-on-every-read` by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BoundTerm {
    /// Rows returned.
    Limit,
    /// Rows skipped.
    Offset,
    /// Funnel width, in regions. `survey`'s only bound term — it has no rows to limit.
    Regions,
}

/// How a receiving act consumes the `IdSet` it was handed.
///
/// Declared at the CONSUMING stage, never the producing one — the producer emits membership and
/// has no opinion about what the next act does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BoundsMode {
    /// Narrow to within this set.
    Bound,
    /// Grow from this set.
    Seed,
}

/// How much per-resource meta the trace retains (design §4.4, tier 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum MetaDetail {
    /// Per-resource meta only for ids in the final result set. Bounded by the caller's own limit.
    #[default]
    Surviving,
    /// Every id at every stage, including ids dropped mid-composition. The diagnostic mode.
    Full,
    /// Tier 1 only.
    None,
}
```

Add `pub mod scalars;` and re-export `BoundTerm, BoundsMode, Extent, MetaDetail` from `query/mod.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core scalars`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
cargo make generate-ts-types
git add crates/temper-core/src/types/query/ packages/temper-ui/src/lib/types/generated/
git commit -m "feat(query): Extent, BoundTerm, BoundsMode, MetaDetail"
```

---

### Task 3: Filters — the predicate layer

The task that makes the audit's #1 finding untypeable. `--edge-type advances` today returns a corpus byte-identical to `--no-graph`, with `reason: ok` and exit 0, because the filter compares `edge_kind::text` while every edge the caller has ever seen displays a `label`.

**Files:**
- Create: `crates/temper-core/src/types/query/filter.rs`
- Modify: `crates/temper-core/src/types/query/mod.rs`

**Interfaces:**
- Consumes: **`crate::types::graph::EdgeKind`** — the incumbent, NOT a new definition. See the gap note below.
- Produces: `EdgeFilter`, `ResourceFilter`, `FacetPredicate`, `FilterField`. Task 6 puts `FilterField` on `ActDeclaration`; Task 8 puts the filters on `ActInvocation`.

> **⚠️ Plan/reality gap — corrected `[2026-08-03, T2 build]`.** This task was written as though it
> introduced `EdgeKind`. It does not. `crates/temper-core/src/types/graph.rs:33` already defines it
> with the same four variants, `#[sqlx(type_name = "edge_kind", rename_all = "snake_case")]` bound to
> `migrations/20260624000001_canonical_schema.sql:95`, `#[serde(rename_all = "snake_case")]`, ts-rs
> export to `graph.ts`, and a re-export at `crates/temper-core/src/types/mod.rs`
> (`pub use graph::{EdgeKind, Polarity};`) `[verified — 2026-08-03]`.
>
> **Import it; do not redefine it.** Two Rust mirrors on one DDL enum, only one `sqlx::Type`-checked,
> is a drift site; a second definition also collides at the `types/mod.rs` re-export and emits two
> same-named TS types. Design §4.1.2 carries the amendment.
>
> **The tests below are kept and still earn their place** — they become regression cover asserting
> that the incumbent has the properties the contract depends on (closedness, snake_case wire form,
> and that `"advances"` cannot deserialize). They pass against the incumbent unchanged.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_kind_is_closed_at_the_four_the_ddl_declares() {
        // migrations/20260624000001_canonical_schema.sql:95
        //   CREATE TYPE edge_kind AS ENUM ('express', 'contains', 'leads_to', 'near');
        for (k, j) in [
            (EdgeKind::Express, "\"express\""),
            (EdgeKind::Contains, "\"contains\""),
            (EdgeKind::LeadsTo, "\"leads_to\""),
            (EdgeKind::Near, "\"near\""),
        ] {
            assert_eq!(serde_json::to_string(&k).unwrap(), j);
        }
    }

    #[test]
    fn a_label_cannot_be_passed_as_an_edge_kind() {
        // THE audit's #1 finding, fixed at the type level. `advances` is a real LABEL that appears
        // on real edges; it is not an edge_kind. Today `--edge-type advances` silently narrows to
        // nothing with reason: ok. Here it cannot be constructed at all.
        assert!(serde_json::from_str::<EdgeKind>("\"advances\"").is_err());
        assert!(serde_json::from_str::<EdgeKind>("\"derived_from\"").is_err());
    }

    #[test]
    fn labels_and_edge_kinds_are_separate_fields_on_the_filter() {
        // Separate slots, different types — so the caller who means "advances" has exactly one
        // place to put it, and it is the right one.
        let f = EdgeFilter {
            edge_kinds: vec![EdgeKind::LeadsTo],
            labels: vec!["advances".to_string()],
        };
        let back: EdgeFilter =
            serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.edge_kinds, vec![EdgeKind::LeadsTo]);
        assert_eq!(back.labels, vec!["advances".to_string()]);
    }

    #[test]
    fn an_empty_filter_serializes_to_nothing() {
        let f = ResourceFilter::default();
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "{}", "an unset filter must not emit empty arrays");
        assert_eq!(serde_json::from_str::<ResourceFilter>("{}").unwrap(), f);
    }

    #[test]
    fn resource_filters_compose_and_round_trip() {
        // filters-compose-to-narrow: several predicates on one request, AND semantics.
        let f = ResourceFilter {
            doc_type: vec!["task".to_string()],
            tags: vec!["search".to_string(), "ci".to_string()],
            facets: vec![FacetPredicate { key: "domain".to_string(), value: "search".to_string() }],
            stage: Some("in-progress".to_string()),
            ..Default::default()
        };
        assert_eq!(serde_json::from_str::<ResourceFilter>(&serde_json::to_string(&f).unwrap()).unwrap(), f);
    }

    #[test]
    fn filter_fields_name_the_two_slots_an_act_may_admit() {
        assert_eq!(serde_json::to_string(&FilterField::Resource).unwrap(), "\"resource\"");
        assert_eq!(serde_json::to_string(&FilterField::Edge).unwrap(), "\"edge\"");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core filter`
Expected: FAIL — `EdgeFilter` / `ResourceFilter` / `FacetPredicate` / `FilterField` not found.
**Not** `EdgeKind`: it resolves already, via the `use` above. If a failure names `EdgeKind` as
missing, the import is wrong — do not "fix" it by defining one.

- [ ] **Step 3: Write the implementation**

```rust
//! The predicate layer. Bounds are membership and terms are magnitude; neither can narrow by what
//! a thing IS. Everything carries `kb_properties` (`doc_type`, `tags`, `facet`) and every edge
//! carries BOTH an `edge_kind` and a `label`.
//!
//! Typed slots, deliberately NOT a generic `{field, op, value}` grammar: a general predicate
//! language would be more expressive and would immediately re-open every conflation this contract
//! exists to close.

use serde::{Deserialize, Serialize};

// The four members of the DDL's `edge_kind` enum
// (`migrations/20260624000001_canonical_schema.sql:95`) are ALREADY modelled, and re-used here
// rather than restated. `types::graph::EdgeKind` is `sqlx::Type`-bound to that DDL, so it is the
// copy a schema change breaks — which is exactly why the contract must not carry a second one.
// Its closedness is the fix for the audit's #1 finding: an edge `label` such as `advances` cannot
// be passed here.
use crate::types::graph::EdgeKind;

/// Narrowing over edges. `edge_kinds` and `labels` are DIFFERENT AXES and are never merged: the
/// kind is a closed DDL enum, the label is free text the caller actually sees on every edge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct EdgeFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_kinds: Vec<EdgeKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// One `kb_properties` facet predicate, at the inner-key grain the facet model uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct FacetPredicate {
    pub key: String,
    pub value: String,
}

/// Narrowing over resources. Every field is AND-composed; an unset field narrows nothing.
///
/// An unknown value on a closed vocabulary (`doc_type`, `stage`, `status`) is a REFUSAL
/// (`RefusalReason::UnknownFilterValue`), never a confident empty page — the audit found four
/// filters that accept nonsense and return one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct ResourceFilter {
    /// `kb_properties` where `property_key = 'doc_type'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_type: Vec<String>,
    /// `kb_properties` where `property_key = 'tags'`. AND-containment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `kb_properties` where `property_key = 'facet'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<FacetPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

/// Which filter slot an act admits. An unadmitted filter is DECLINED, never ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FilterField {
    Resource,
    Edge,
}
```

Add `pub mod filter;` and re-export `EdgeFilter, FacetPredicate, FilterField, ResourceFilter` from `query/mod.rs`.

**Do NOT re-export `EdgeKind` from `query/mod.rs`.** It is not query's type: `crate::types` already
re-exports it from `graph`, and a second public path to one type invites the very ambiguity this
correction removed. `EdgeFilter` referencing it is enough.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core filter`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
cargo make generate-ts-types
git add crates/temper-core/src/types/query/ packages/temper-ui/src/lib/types/generated/
git commit -m "feat(query): the predicate layer, with edge_kind and label as separate axes"
```

---

### Task 4: Dispositions and refusal

**Files:**
- Create: `crates/temper-core/src/types/query/disposition.rs`
- Modify: `crates/temper-core/src/types/query/mod.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `StageDisposition`, `ActRefusal`, `RefusalReason`, `RefusalDisposition`. Task 7 puts `StageDisposition` on `StageTrace`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_exactly_four_dispositions() {
        let all = [
            StageDisposition::Answered,
            StageDisposition::Empty,
            StageDisposition::Withheld,
            StageDisposition::Refused,
        ];
        let rendered: Vec<String> =
            all.iter().map(|d| serde_json::to_string(d).unwrap()).collect();
        assert_eq!(rendered, ["\"answered\"", "\"empty\"", "\"withheld\"", "\"refused\""]);
    }

    #[test]
    fn disposition_is_closed_unknown_values_fail_to_parse() {
        // Closedness is the property that lets consumers match exhaustively. Adding a fifth
        // variant is a BREAKING change (design §6.1) precisely because of this test.
        assert!(serde_json::from_str::<StageDisposition>("\"partially_answered\"").is_err());
    }

    #[test]
    fn empty_and_refused_are_distinguishable() {
        // An honest zero and a declined question are different answers; collapsing them is the
        // refusal-dialect divergence this contract exists to end.
        assert_ne!(
            serde_json::to_string(&StageDisposition::Empty).unwrap(),
            serde_json::to_string(&StageDisposition::Refused).unwrap()
        );
    }

    #[test]
    fn refusal_carries_a_reason_and_round_trips() {
        let r = ActRefusal {
            reason: RefusalReason::UnsupportedBoundKind,
            detail: "act `find-exact` does not accept bounds of kind `region`".to_string(),
        };
        assert_eq!(serde_json::from_str::<ActRefusal>(&serde_json::to_string(&r).unwrap()).unwrap(), r);
    }

    #[test]
    fn composition_refusal_disposition_has_two_v0_values() {
        assert_eq!(serde_json::to_string(&RefusalDisposition::Halt).unwrap(), "\"halt\"");
        assert_eq!(
            serde_json::to_string(&RefusalDisposition::DegradeAndDisclose).unwrap(),
            "\"degrade_and_disclose\""
        );
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core disposition`
Expected: FAIL — `StageDisposition` not found.

- [ ] **Step 3: Write the implementation**

```rust
//! How a stage resolved, and what a refusal says.

use serde::{Deserialize, Serialize};

/// How a single stage resolved. CLOSED — adding a variant is a breaking change (design §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum StageDisposition {
    /// Rows returned.
    Answered,
    /// Honest zero — the question was asked and nothing matched.
    Empty,
    /// Material exists; the asker's standing does not admit disclosure at this depth.
    Withheld,
    /// The act declined a well-formed question.
    Refused,
}

/// Why an act refused. A typed variant so every door renders the same value; how a door
/// TRANSPORTS it (HTTP status, MCP error code) stays a door concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// The act does not accept bounds of the supplied `IdKind`.
    UnsupportedBoundKind,
    /// The act does not accept seeds of the supplied `IdKind`.
    UnsupportedSeedKind,
    /// A region set arrived without the `provenance` its kind requires.
    MissingProvenance,
    /// The act is declared but not built (`build_state` is not `served` or `fused`).
    NotImplemented,
    /// A required input the composition never supplied — e.g. a `find-about-*` stage with no
    /// threaded intention. Explicitly NOT a silent substitution.
    MissingIntention,
    /// A filter value outside a closed vocabulary — an unknown `doc_type`, `stage` or `status`.
    /// Refused rather than returned as an empty page: a typo must never be reportable as an
    /// absence, which is what four filters do today.
    UnknownFilterValue,
    /// A filter slot the act does not admit (`ResourceFilter` on an edge-only act, or the
    /// reverse). Declined, never ignored.
    FilterNotApplicable,
    /// A bound term was supplied to an act for which that frame of reference does not exist —
    /// e.g. `limit` (rows) handed to `survey`, whose bound is a funnel width. The term is never
    /// reinterpreted to fit; it is declined. Raised STATICALLY, at plan validation against the
    /// generated schemas, so an inapplicable bound is a property of the plan rather than a
    /// runtime surprise.
    BoundTermNotApplicable,
}

/// A refusal, distinct from a failure and from an honest empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct ActRefusal {
    pub reason: RefusalReason,
    /// Human-readable, disclosed at the depth the asker's standing allows.
    pub detail: String,
}

/// What a composition does when a stage refuses. Declared BEFORE execution; the executor never
/// improvises it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RefusalDisposition {
    Halt,
    DegradeAndDisclose,
}
```

Add `pub mod disposition;` and re-export all four types from `query/mod.rs`.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core disposition`
Expected: PASS, 5 tests.

- [ ] **Step 5: Commit**

```bash
cargo make generate-ts-types
git add crates/temper-core/src/types/query/ packages/temper-ui/src/lib/types/generated/
git commit -m "feat(query): four dispositions and the typed refusal variant"
```

---

### Task 5: Act identity and build-state

**Files:**
- Create: `crates/temper-core/src/types/query/act.rs`
- Modify: `crates/temper-core/src/types/query/mod.rs`

**Interfaces:**
- Consumes: `IdKind` (Task 1).
- Produces: `ActName`, `BuildState`, `VisibilityProfile`, `ActDeclaration`. Task 6 populates the registry with `ActDeclaration` values.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn act_names_are_asker_shaped_on_the_wire() {
        assert_eq!(serde_json::to_string(&ActName::FindExact).unwrap(), "\"find-exact\"");
        assert_eq!(
            serde_json::to_string(&ActName::FindAboutAnywhere).unwrap(),
            "\"find-about-anywhere\""
        );
        assert_eq!(
            serde_json::to_string(&ActName::FindAboutWithin).unwrap(),
            "\"find-about-within\""
        );
        assert_eq!(serde_json::to_string(&ActName::FollowFrom).unwrap(), "\"follow-from\"");
        assert_eq!(serde_json::to_string(&ActName::Survey).unwrap(), "\"survey\"");
        assert_eq!(serde_json::to_string(&ActName::Substantiate).unwrap(), "\"substantiate\"");
    }

    #[test]
    fn act_discriminator_is_open_unknown_acts_parse() {
        // Adding an act is ADDITIVE (design §6.1) — an older consumer must survive a newer one.
        let a: ActName = serde_json::from_str("\"enumerate\"").expect("unknown act must parse");
        assert_eq!(a, ActName::Other("enumerate".to_string()));
    }

    #[test]
    fn fused_build_state_names_its_host() {
        // `fused` is a fact, not a euphemism: the host is what T3's gate checks has a door while
        // the act itself does not.
        let b = BuildState::Fused { host: "unified_search".to_string() };
        let back: BuildState = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.host(), Some("unified_search"));
        assert_eq!(BuildState::Unbuilt.host(), None);
        assert_eq!(BuildState::Served.host(), None);
    }

    #[test]
    fn a_declaration_cannot_omit_what_the_asker_holds() {
        // `every-act-is-situated` enforced at the type layer: asker_holds is not Option.
        let d = ActDeclaration {
            name: ActName::FindExact,
            asker_holds: "I can quote the exact words".to_string(),
            served_by: Some("search_fts_candidates".to_string()),
            build_state: BuildState::Fused { host: "unified_search".to_string() },
            accepts_bounds: vec![IdKind::Resource],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 1,
        };
        assert!(!d.asker_holds.is_empty());
        assert_eq!(serde_json::from_str::<ActDeclaration>(&serde_json::to_string(&d).unwrap()).unwrap(), d);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core act::`
Expected: FAIL — `ActName` not found.

- [ ] **Step 3: Write the implementation**

```rust
//! Act identity, build-state, and the declaration shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::id_set::IdKind;
use super::filter::FilterField;
use super::scalars::BoundTerm;

/// The act vocabulary. Asker-shaped, not mechanism-shaped: an act names what the asker holds, and
/// the mechanic currently serving it is evidence rather than identity.
///
/// OPEN discriminator — adding an act is additive.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub enum ActName {
    /// *I can quote the exact words.*
    #[serde(rename = "find-exact")]
    FindExact,
    /// *A concept, no exact words; search everything I can see.*
    #[serde(rename = "find-about-anywhere")]
    FindAboutAnywhere,
    /// *A concept, plus a set to search inside.*
    #[serde(rename = "find-about-within")]
    FindAboutWithin,
    /// *A found thing; I want its neighbours.*
    #[serde(rename = "follow-from")]
    FollowFrom,
    /// *A question about what a scope knows.*
    #[serde(rename = "survey")]
    Survey,
    /// *A claim; I want its defensibility.*
    #[serde(rename = "substantiate")]
    Substantiate,
    /// The anti-act: visibility-shaped admission wearing relevance's costume. Declared so that
    /// promoting it to a real act requires DELETING an explicit refusal.
    #[serde(rename = "admit")]
    Admit,
    #[serde(untagged)]
    Other(String),
}

/// Whether an act is reachable, and how. Every value is mechanically checkable by T3's gate —
/// which is the whole point, because a hand-maintained build-state is the `ADMIN_EVENT_TYPES`
/// failure: a const beside a registry, with a test holding its own second copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum BuildState {
    /// Exactly one door invokes this act alone.
    Served,
    /// The mechanic runs only inside a named composite; the act has no door, the host has one.
    Fused { host: String },
    /// No mechanic exists.
    Unbuilt,
}

impl BuildState {
    pub fn host(&self) -> Option<&str> {
        match self {
            BuildState::Fused { host } => Some(host.as_str()),
            _ => None,
        }
    }
}

/// Where the principal constraint applies to an act's mechanic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum VisibilityProfile {
    PrincipalAgnostic,
    /// Every input and operation is principal-free, but the fragment is a window or aggregate
    /// whose frame is the principal's read-set. `survey`'s `sal_norm` is the worked example.
    AgnosticInValueRelativeInDomain,
    PrincipalRelative,
}

/// One act, declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct ActDeclaration {
    pub name: ActName,
    /// What the asker holds. NOT optional — `every-act-is-situated` enforced by the signature.
    pub asker_holds: String,
    /// The SQL function serving this act; `None` when `build_state` is `unbuilt`. T3 fingerprints
    /// this function's body against `scoring_revision`.
    pub served_by: Option<String>,
    pub build_state: BuildState,
    pub accepts_bounds: Vec<IdKind>,
    pub accepts_seeds: Vec<IdKind>,
    /// Which bound terms this act admits. A term absent here is DECLINED, never reinterpreted:
    /// `survey` admits `[Regions]` and not `Limit`, because `wayfind_region_scores` takes a funnel
    /// width and has no rows to limit.
    pub accepts_bound_terms: Vec<BoundTerm>,
    /// Which filter slots this act admits. An unadmitted filter is declined
    /// (`RefusalReason::FilterNotApplicable`), never silently ignored.
    pub accepts_filters: Vec<FilterField>,
    /// Published ceilings, per admitted term. A ceiling the caller could have read is disclosed by
    /// `terms_effective` and owes no separate warning; an UNPUBLISHED ceiling is the defect, not
    /// the clamping. A term with no entry here has no ceiling.
    pub bound_ceilings: BTreeMap<BoundTerm, i64>,
    pub produces: Option<IdKind>,
    pub visibility_profile: VisibilityProfile,
    /// Bumped whenever the served-by body changes the scale or meaning of a quantity. T3 gate 4
    /// reds when the body hash moves and this does not.
    pub scoring_revision: u32,
}
```

Add `pub mod act;` and re-export the four types.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core act::`
Expected: PASS, 4 tests.

- [ ] **Step 5: Commit**

```bash
cargo make generate-ts-types
git add crates/temper-core/src/types/query/ packages/temper-ui/src/lib/types/generated/
git commit -m "feat(query): ActName, BuildState, ActDeclaration"
```

---

### Task 6: The registry — seven declarations and the chainability matrix

The task where the contract makes checkable claims about reality. Every value here is a claim T3's gates verify.

**Files:**
- Create: `crates/temper-core/src/types/query/registry.rs`
- Modify: `crates/temper-core/src/types/query/mod.rs`

**Interfaces:**
- Consumes: `ActDeclaration`, `ActName`, `BuildState`, `VisibilityProfile` (Task 5); `IdKind` (Task 1).
- Produces: `pub fn search_family() -> Vec<ActDeclaration>` and `pub fn declaration(name: &ActName) -> Option<ActDeclaration>`. T3 gates iterate `search_family()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_family_declares_seven_acts_including_the_anti_act() {
        let acts = search_family();
        assert_eq!(acts.len(), 7);
        let names: Vec<&ActName> = acts.iter().map(|a| &a.name).collect();
        for expected in [
            ActName::FindExact,
            ActName::FindAboutAnywhere,
            ActName::FindAboutWithin,
            ActName::FollowFrom,
            ActName::Survey,
            ActName::Substantiate,
            ActName::Admit,
        ] {
            assert!(names.contains(&&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn nothing_in_the_search_family_is_served() {
        // This is the FINDING, not an omission: every mechanic is reachable only through
        // unified_search. When a real door lands, this test changes deliberately.
        for a in search_family() {
            assert_ne!(a.build_state, BuildState::Served, "{:?} claims served", a.name);
        }
    }

    #[test]
    fn every_declaration_states_what_the_asker_holds() {
        for a in search_family() {
            assert!(!a.asker_holds.trim().is_empty(), "{:?} omits asker_holds", a.name);
        }
    }

    #[test]
    fn unbuilt_acts_name_no_serving_function_and_built_ones_do() {
        for a in search_family() {
            match a.build_state {
                BuildState::Unbuilt => assert!(
                    a.served_by.is_none(),
                    "{:?} is unbuilt but names a function", a.name
                ),
                _ => assert!(
                    a.served_by.is_some(),
                    "{:?} is built but names no function", a.name
                ),
            }
        }
    }

    #[test]
    fn follow_from_takes_resource_seeds_and_cannot_be_bounded() {
        // The one genuine foreclosure: search_graph_expand has no scope parameter.
        let a = declaration(&ActName::FollowFrom).unwrap();
        assert_eq!(a.accepts_seeds, vec![IdKind::Resource]);
        assert!(a.accepts_bounds.is_empty(), "follow-from bounded is unbuilt");
    }

    #[test]
    fn survey_accepts_anchor_kinds_as_bounds() {
        // Dissolved by the typed currency: wayfind_region_scores takes (p_anchor_table,
        // p_anchor_id), so cogmap_list -> survey needs no SQL change.
        let a = declaration(&ActName::Survey).unwrap();
        assert!(a.accepts_bounds.contains(&IdKind::Cogmap));
        assert!(a.accepts_bounds.contains(&IdKind::Context));
        assert_eq!(a.produces, Some(IdKind::Region));
    }

    #[test]
    fn find_about_anywhere_accepts_no_bounds_by_definition() {
        // A bound would make it find-about-within. This is a definitional exclusion, not a hole.
        let a = declaration(&ActName::FindAboutAnywhere).unwrap();
        assert!(a.accepts_bounds.is_empty());
        let w = declaration(&ActName::FindAboutWithin).unwrap();
        assert!(!w.accepts_bounds.is_empty());
    }

    #[test]
    fn survey_admits_regions_and_refuses_limit() {
        // `limit` means ROWS on every read. wayfind_region_scores takes p_regions_n — a funnel
        // width — and has no rows to limit, so survey DECLINES `limit` rather than quietly
        // reinterpreting it as a region count. That is the-same-bound-term-means-the-same-thing
        // holding by construction.
        let a = declaration(&ActName::Survey).unwrap();
        assert!(a.accepts_bound_terms.contains(&BoundTerm::Regions));
        assert!(!a.accepts_bound_terms.contains(&BoundTerm::Limit));

        let e = declaration(&ActName::FindExact).unwrap();
        assert!(e.accepts_bound_terms.contains(&BoundTerm::Limit));
        assert!(!e.accepts_bound_terms.contains(&BoundTerm::Regions));
    }

    #[test]
    fn every_ceiling_is_published_for_a_term_the_act_admits() {
        // An unpublished ceiling is the defect, not the clamping: a caller who could have read the
        // ceiling is owed no separate warning, but one who could not is owed the refusal.
        for a in search_family() {
            for term in a.bound_ceilings.keys() {
                assert!(
                    a.accepts_bound_terms.contains(term),
                    "{:?} publishes a ceiling for a term it does not admit: {term:?}", a.name
                );
            }
        }
    }

    #[test]
    fn follow_from_filters_edges_and_the_find_acts_filter_resources() {
        // The two slots never cross: an edge-walking act has no resource predicate to apply, and
        // a lexical/vector act has no edge to filter.
        assert_eq!(
            declaration(&ActName::FollowFrom).unwrap().accepts_filters,
            vec![FilterField::Edge]
        );
        assert_eq!(
            declaration(&ActName::FindExact).unwrap().accepts_filters,
            vec![FilterField::Resource]
        );
    }

    #[test]
    fn the_anti_act_is_declared_unbuilt_and_produces_nothing() {
        let a = declaration(&ActName::Admit).unwrap();
        assert_eq!(a.build_state, BuildState::Unbuilt);
        assert_eq!(a.produces, None);
        assert!(a.accepts_bounds.is_empty() && a.accepts_seeds.is_empty());
    }

    #[test]
    fn survey_is_the_only_act_relative_in_domain() {
        // sal_norm is a percent_rank whose window frame is the asker's visible set — measured,
        // 382 of 385 regions score differently across two visible-anchor sets. No other act in
        // the family has that property, and claiming one did would be a false declaration.
        let relative: Vec<ActName> = search_family()
            .into_iter()
            .filter(|a| a.visibility_profile == VisibilityProfile::AgnosticInValueRelativeInDomain)
            .map(|a| a.name)
            .collect();
        assert_eq!(relative, vec![ActName::Survey]);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core registry`
Expected: FAIL — `search_family` not found.

- [ ] **Step 3: Write the implementation**

```rust
//! The search family's act declarations, as data.
//!
//! Every value here is a claim about the deployed system that T3's gates verify:
//! `build_state` against the live router, `accepts_*` against the SQL signatures, and
//! `scoring_revision` against a fingerprint of `served_by`'s body.

use std::collections::BTreeMap;

use super::act::{ActDeclaration, ActName, BuildState, VisibilityProfile};
use super::filter::FilterField;
use super::id_set::IdKind;
use super::scalars::BoundTerm;

fn fused() -> BuildState {
    BuildState::Fused { host: "unified_search".to_string() }
}

/// The seven declarations. Order is stable — the generated contract renders them in this order.
pub fn search_family() -> Vec<ActDeclaration> {
    vec![
        ActDeclaration {
            name: ActName::FindExact,
            asker_holds: "I can quote the exact words".to_string(),
            served_by: Some("search_fts_candidates".to_string()),
            build_state: fused(),
            // Post-filter in unified_search's `corpus` CTE. Membership-equivalent to a pre-filter
            // because the FTS arm carries no top-k, so nothing can be crowded out of it.
            accepts_bounds: vec![IdKind::Resource],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 2, // ts_rank flag 32 -> 33, migration 20260801000010
        },
        ActDeclaration {
            name: ActName::FindAboutAnywhere,
            asker_holds: "a concept, no exact words; search everything I can see".to_string(),
            served_by: Some("search_vector_candidates".to_string()),
            build_state: fused(),
            // A bound would make this find-about-within. Definitional exclusion, not a hole.
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 2, // best-of-N shrunk toward the chunk mean, 20260801000010
        },
        ActDeclaration {
            name: ActName::FindAboutWithin,
            asker_holds: "a concept, plus a set to search inside".to_string(),
            served_by: Some("search_vector_candidates".to_string()),
            build_state: fused(),
            accepts_bounds: vec![IdKind::Resource, IdKind::Context, IdKind::Cogmap],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 2,
        },
        ActDeclaration {
            name: ActName::FollowFrom,
            asker_holds: "a found thing; I want its neighbours".to_string(),
            served_by: Some("search_graph_expand".to_string()),
            build_state: fused(),
            // UNBUILT: search_graph_expand has no scope parameter. "Walk from these seeds but
            // stay inside this set" is unstatable. The one genuine foreclosure.
            accepts_bounds: vec![],
            accepts_seeds: vec![IdKind::Resource],
            accepts_bound_terms: vec![BoundTerm::Limit],
            accepts_filters: vec![FilterField::Edge],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 1,
        },
        ActDeclaration {
            name: ActName::Survey,
            asker_holds: "a question about what a scope knows".to_string(),
            served_by: Some("wayfind_region_scores".to_string()),
            build_state: fused(),
            // Takes (p_anchor_table, p_anchor_id) — an anchor, which a typed IdSet can name.
            accepts_bounds: vec![IdKind::Cogmap, IdKind::Context],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Regions],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::from([(BoundTerm::Regions, 20)]),
            produces: Some(IdKind::Region),
            visibility_profile: VisibilityProfile::AgnosticInValueRelativeInDomain,
            scoring_revision: 1,
        },
        ActDeclaration {
            name: ActName::Substantiate,
            asker_holds: "a claim; I want its defensibility".to_string(),
            served_by: None,
            build_state: BuildState::Unbuilt,
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::new(),
            produces: None,
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 0,
        },
        ActDeclaration {
            name: ActName::Admit,
            asker_holds: "nothing — this is an anti-act, declared so that promoting \
                          cold-start admission to a real act must delete an explicit refusal"
                .to_string(),
            served_by: None,
            build_state: BuildState::Unbuilt,
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::new(),
            produces: None,
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 0,
        },
    ]
}

/// Look up one declaration by name.
pub fn declaration(name: &ActName) -> Option<ActDeclaration> {
    search_family().into_iter().find(|a| &a.name == name)
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core registry`
Expected: PASS, 12 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/query/
git commit -m "feat(query): the search family's seven act declarations"
```

---

### Task 7: Invocation and result envelopes

**Files:**
- Create: `crates/temper-core/src/types/query/envelope.rs`
- Modify: `crates/temper-core/src/types/query/mod.rs`

**Interfaces:**
- Consumes: `ActName` (Task 5), `IdSet`/`IdKind` (Task 1), `BoundsMode`/`Extent`/`BoundTerm` (Task 2).
- Produces: `ActInvocation`, `ActResult`, `NarrowedBy`. Task 8's `StageTrace` embeds `NarrowedBy`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::id_set::{IdKind, IdSet};
    use std::collections::BTreeMap;

    #[test]
    fn an_invocation_without_bounds_or_terms_omits_them() {
        let inv = ActInvocation {
            act: ActName::FindAboutAnywhere,
            bounds: None,
            bounds_mode: None,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
        };
        let json = serde_json::to_string(&inv).unwrap();
        assert!(!json.contains("bounds"));
        assert!(!json.contains("terms"));
        assert!(!json.contains("filter"));
        assert_eq!(serde_json::from_str::<ActInvocation>(&json).unwrap(), inv);
    }

    #[test]
    fn a_result_declares_the_kind_it_produced() {
        // `produced` is an IdSet, so an act's output kind is machine-checkable rather than
        // inferred from which act ran. This is what makes contract chaining compare kinds.
        let r = ActResult {
            act: ActName::Survey,
            produced: IdSet { kind: IdKind::Region, provenance: None, ids: vec![] },
            extent: Extent::Complete,
            total: None,
            terms_effective: BTreeMap::from([(BoundTerm::Regions, 3)]),
            narrowed_by: vec![],
            bounds_in: 0,
            bounds_honored: 0,
            bounds_withheld: 0,
        };
        assert_eq!(r.produced.kind, IdKind::Region);
        assert_eq!(serde_json::from_str::<ActResult>(&serde_json::to_string(&r).unwrap()).unwrap(), r);
    }

    #[test]
    fn a_result_can_report_partial_without_paying_for_a_total() {
        // The whole point of Extent: "there is more" is answerable with a limit+1 probe, where a
        // total would cost a second query on every stage of every chain.
        let r = ActResult {
            act: ActName::FindExact,
            produced: IdSet { kind: IdKind::Resource, provenance: None, ids: vec![] },
            extent: Extent::Partial,
            total: None,
            terms_effective: BTreeMap::from([(BoundTerm::Limit, 50)]),
            narrowed_by: vec![],
            bounds_in: 0,
            bounds_honored: 0,
            bounds_withheld: 0,
        };
        assert_eq!(r.extent, Extent::Partial);
        assert!(r.total.is_none(), "a partial answer owes no total");
        // The applied ceiling is visible beside what was asked, so the clamp is not silent.
        assert_eq!(r.terms_effective.get(&BoundTerm::Limit), Some(&50));
    }

    #[test]
    fn a_traversal_result_reports_indeterminate_rather_than_guessing() {
        let r = ActResult {
            act: ActName::Survey,
            produced: IdSet { kind: IdKind::Region, provenance: None, ids: vec![] },
            extent: Extent::Indeterminate {
                reason: "region-salience traversal has no size prior to its funnel width".to_string(),
            },
            total: None,
            terms_effective: BTreeMap::from([(BoundTerm::Regions, 3)]),
            narrowed_by: vec![],
            bounds_in: 0,
            bounds_honored: 0,
            bounds_withheld: 0,
        };
        assert!(matches!(r.extent, Extent::Indeterminate { .. }));
    }

    #[test]
    fn narrowed_by_records_what_a_threshold_excluded() {
        let n = NarrowedBy {
            key: "min_lexical_rank".to_string(),
            value: "0.4".to_string(),
            admitted: Some(12),
            excluded: Some(88),
        };
        // A filter may be disclosed without paying to count what it excluded.
        let cheap = NarrowedBy {
            key: "doc_type".to_string(),
            value: "task".to_string(),
            admitted: None,
            excluded: None,
        };
        assert!(!serde_json::to_string(&cheap).unwrap().contains("admitted"));
        assert_eq!(serde_json::from_str::<NarrowedBy>(&serde_json::to_string(&n).unwrap()).unwrap(), n);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core envelope`
Expected: FAIL — `ActInvocation` not found.

- [ ] **Step 3: Write the implementation**

```rust
//! The per-act invocation and result envelopes.
//!
//! Base ⊕ per-act extension: `params<act>` and `meta<act>` are added by the act implementations
//! (out of scope for v0's contract task) via `#[serde(flatten)]` on a discriminated extension.

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use super::act::ActName;
use super::filter::{EdgeFilter, ResourceFilter};
use super::id_set::IdSet;
use super::scalars::{BoundTerm, BoundsMode, Extent};

/// One act, invoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct ActInvocation {
    pub act: ActName,
    /// The only value that crosses a stage boundary. Membership, never rank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<IdSet>,
    /// How this act consumes `bounds`. Required whenever `bounds` is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_mode: Option<BoundsMode>,
    /// Act-level bound terms. A term this act does not admit is refused STATICALLY
    /// (`RefusalReason::BoundTermNotApplicable`), never reinterpreted to fit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub terms: BTreeMap<BoundTerm, i64>,
    /// Narrowing by what a thing IS. At most one slot applies per act; supplying the other is
    /// `RefusalReason::FilterNotApplicable`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_filter: Option<ResourceFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_filter: Option<EdgeFilter>,
}

/// One act-specific threshold, and what applying it did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct NarrowedBy {
    pub key: String,
    pub value: String,
    /// Counts are carried ONLY where the act computes them for free. Requiring them would
    /// re-introduce the second query `Extent` exists to avoid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admitted: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluded: Option<i64>,
}

/// One act's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct ActResult {
    pub act: ActName,
    /// Declared kind, so contract chaining compares kinds rather than inferring them.
    pub produced: IdSet,
    /// Complete / partial / indeterminate. NOT a total — see `Extent`.
    pub extent: Extent,
    /// Carried only by acts that can produce one WITHOUT a second query. Never by a composition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
    /// The APPLIED value of every admitted term, beside what was asked. Generalizes the
    /// `regions_effective` pattern the audit calls "a model of an honest knob" — which existed
    /// for exactly one term and was never extended to `limit` or `depth`.
    pub terms_effective: BTreeMap<BoundTerm, i64>,
    pub narrowed_by: Vec<NarrowedBy>,
    pub bounds_in: i64,
    pub bounds_honored: i64,
    pub bounds_withheld: i64,
}
```

Add `pub mod envelope;` and re-export.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core envelope`
Expected: PASS, 5 tests.

- [ ] **Step 5: Commit**

```bash
cargo make generate-ts-types
git add crates/temper-core/src/types/query/ packages/temper-ui/src/lib/types/generated/
git commit -m "feat(query): ActInvocation and ActResult envelopes"
```

---

### Task 8: The composition envelope and its trace

Both halves in one task: the trace is the composition envelope's disclosure half, and a reviewer cannot meaningfully accept one while rejecting the other.

**Files:**
- Create: `crates/temper-core/src/types/query/trace.rs`, `crates/temper-core/src/types/query/composition.rs`
- Modify: `crates/temper-core/src/types/query/mod.rs`

**Interfaces:**
- Consumes: `ActName` (Task 5), `StageDisposition`/`RefusalDisposition` (Task 4), `NarrowedBy`/`ActInvocation` (Task 7), `MetaDetail` (Task 2), `IdKind` (Task 1).
- Produces: `StageTrace`, `BoundsSource`, `MetaTruncated`, `CompositionTrace`, `Intention`, `OutcomeDeclaration`, `Composition`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::disposition::StageDisposition;

    #[test]
    fn a_refused_stage_still_has_a_trace_entry() {
        // The reason disclosure lives in the envelope rather than in each act's response: a
        // refused stage has no result to attach disclosure to.
        let t = StageTrace {
            stage: 2,
            act: ActName::FindAboutWithin,
            disposition: StageDisposition::Refused,
            bounds_source: Some(BoundsSource::Upstream { stage: 1 }),
            bounds_in: 40,
            bounds_honored: 0,
            bounds_withheld: 0,
            narrowed_by: vec![],
            meta_truncated: None,
        };
        assert_eq!(t.disposition, StageDisposition::Refused);
        assert_eq!(serde_json::from_str::<StageTrace>(&serde_json::to_string(&t).unwrap()).unwrap(), t);
    }

    #[test]
    fn bounds_source_distinguishes_upstream_from_an_expression() {
        // When jaq post-filters between stages, the next stage's bounds no longer equal the
        // upstream act's produced set. Not forbidden — DISCLOSED.
        let up = BoundsSource::Upstream { stage: 1 };
        let ex = BoundsSource::Expression;
        let ca = BoundsSource::Caller;
        assert_ne!(serde_json::to_string(&up).unwrap(), serde_json::to_string(&ex).unwrap());
        assert_ne!(serde_json::to_string(&ex).unwrap(), serde_json::to_string(&ca).unwrap());
    }

    #[test]
    fn a_truncated_meta_budget_is_always_disclosed() {
        // ORPHAN_LIMIT = 50 truncates with no response flag and no server log. The contract may
        // decline to carry detail; it may never do so silently.
        let m = MetaTruncated { stage: 3, retained: 50, dropped: 412 };
        let t = StageTrace {
            stage: 3,
            act: ActName::FollowFrom,
            disposition: StageDisposition::Answered,
            bounds_source: None,
            bounds_in: 0,
            bounds_honored: 0,
            bounds_withheld: 0,
            narrowed_by: vec![],
            meta_truncated: Some(m),
        };
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("meta_truncated"));
        assert_eq!(serde_json::from_str::<StageTrace>(&json).unwrap(), t);
    }

    #[test]
    fn a_composition_trace_is_ordered_and_carries_its_detail_level() {
        let c = CompositionTrace {
            meta_detail: MetaDetail::Surviving,
            stages: vec![],
        };
        assert_eq!(c.meta_detail, MetaDetail::Surviving);
        assert_eq!(
            serde_json::from_str::<CompositionTrace>(&serde_json::to_string(&c).unwrap()).unwrap(),
            c
        );
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-core trace`
Expected: FAIL — `StageTrace` not found.

- [ ] **Step 3: Write the implementation**

```rust
//! Per-stage disclosure. Tier 1 of design §4.4 — mandatory, O(stages), never truncated.

use serde::{Deserialize, Serialize};

use super::act::ActName;
use super::disposition::StageDisposition;
use super::envelope::NarrowedBy;
use super::scalars::MetaDetail;

/// Where a stage's bounds came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "source")]
pub enum BoundsSource {
    /// Verbatim from an earlier stage's `produced` set.
    Upstream { stage: u32 },
    /// Produced by a jaq expression between stages — i.e. the caller sub-selected, and the
    /// bounds no longer equal any act's output.
    Expression,
    /// Supplied directly by the caller.
    Caller,
}

/// A per-resource meta budget that bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct MetaTruncated {
    pub stage: u32,
    pub retained: i64,
    pub dropped: i64,
}

/// One stage's mandatory disclosure. Exists whether or not the stage produced a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct StageTrace {
    pub stage: u32,
    pub act: ActName,
    pub disposition: StageDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_source: Option<BoundsSource>,
    pub bounds_in: i64,
    pub bounds_honored: i64,
    pub bounds_withheld: i64,
    pub narrowed_by: Vec<NarrowedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_truncated: Option<MetaTruncated>,
}

/// The whole composition's disclosure: an ordered per-stage record array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct CompositionTrace {
    pub meta_detail: MetaDetail,
    pub stages: Vec<StageTrace>,
}
```

Add `pub mod trace;` and re-export.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-core trace`
Expected: PASS, 4 tests.

- [ ] **Step 5: Write the failing tests for the composition envelope**

Create `crates/temper-core/src/types/query/composition.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::act::ActName;
    use crate::types::query::disposition::RefusalDisposition;
    use crate::types::query::envelope::ActInvocation;

    fn stage(act: ActName) -> ActInvocation {
        ActInvocation {
            act,
            bounds: None,
            bounds_mode: None,
            terms: std::collections::BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
        }
    }

    #[test]
    fn a_composition_declares_its_refusal_disposition_up_front() {
        // Declared BEFORE execution; the executor never improvises it.
        let c = Composition {
            outcome: OutcomeDeclaration {
                description: "bias-review over a curated corpus".to_string(),
                produces: None,
            },
            intention: None,
            on_stage_refusal: RefusalDisposition::Halt,
            meta_detail: Default::default(),
            bounds: std::collections::BTreeMap::new(),
            stages: vec![stage(ActName::FindExact)],
        };
        assert_eq!(c.on_stage_refusal, RefusalDisposition::Halt);
        assert_eq!(serde_json::from_str::<Composition>(&serde_json::to_string(&c).unwrap()).unwrap(), c);
    }

    #[test]
    fn the_intention_is_a_composition_level_field_not_a_per_stage_one() {
        // Computed ONCE at composition start and threaded, so every find-about-* stage provably
        // interrogates the same intention rather than re-embedding a mutated string.
        let c = Composition {
            outcome: OutcomeDeclaration { description: "x".to_string(), produces: None },
            intention: Some(Intention { query: "wayfind salience".to_string(), embedded: true }),
            on_stage_refusal: RefusalDisposition::DegradeAndDisclose,
            meta_detail: Default::default(),
            bounds: std::collections::BTreeMap::new(),
            stages: vec![stage(ActName::FindAboutAnywhere)],
        };
        let json = serde_json::to_string(&c).unwrap();
        // One intention on the envelope; the stage carries none.
        assert_eq!(json.matches("\"intention\"").count(), 1);
        assert_eq!(serde_json::from_str::<Composition>(&json).unwrap(), c);
    }

    #[test]
    fn an_absent_intention_is_representable_so_a_stage_can_refuse_rather_than_substitute() {
        // "I chose not to embed" and "I cannot embed" become distinguishable: with no intention
        // on the envelope, a find-about-* stage refuses (RefusalReason::MissingIntention) rather
        // than the server quietly embedding on the caller's behalf.
        let c = Composition {
            outcome: OutcomeDeclaration { description: "lexical only".to_string(), produces: None },
            intention: None,
            on_stage_refusal: RefusalDisposition::Halt,
            meta_detail: Default::default(),
            bounds: std::collections::BTreeMap::new(),
            stages: vec![stage(ActName::FindExact)],
        };
        assert!(c.intention.is_none());
        assert!(!serde_json::to_string(&c).unwrap().contains("intention"));
    }

    #[test]
    fn an_outcome_declaration_cannot_omit_its_description() {
        // The pocket outcome register: a named plan states its served-by. Not Option.
        let o = OutcomeDeclaration { description: "what being served looks like".to_string(), produces: None };
        assert!(!o.description.is_empty());
        assert_eq!(serde_json::from_str::<OutcomeDeclaration>(&serde_json::to_string(&o).unwrap()).unwrap(), o);
    }
}
```

- [ ] **Step 6: Run to verify it fails**

Run: `cargo nextest run -p temper-core composition`
Expected: FAIL — `Composition` not found.

- [ ] **Step 7: Write the composition envelope**

Prepend to `crates/temper-core/src/types/query/composition.rs`:

```rust
//! The composition envelope: an ordered list of stages plus the things that ride alongside them.
//!
//! The PRINCIPAL is deliberately absent. Visibility applies inside each act's execution — one
//! known application point per stage — and jaq reshapes what visibility admitted without ever
//! seeing the credential. There is no field here for it, by construction.

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use super::act::ActName;
use super::disposition::RefusalDisposition;
use super::envelope::ActInvocation;
use super::id_set::IdKind;
use super::scalars::{BoundTerm, MetaDetail};

/// The question, computed once at composition start and threaded to every stage.
///
/// Its ABSENCE is meaningful: a `find-about-*` stage with no intention refuses, rather than the
/// server embedding on the caller's behalf. That is what makes "I chose not to embed" and
/// "I cannot embed" different states instead of one ambiguous one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct Intention {
    pub query: String,
    /// Whether an embedding was computed for it. Inspectable in the trace, which is what makes
    /// paraphrase-stability measurable from outside.
    pub embedded: bool,
}

/// A composition's pocket outcome register: what it is for, in the act schemas' own terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct OutcomeDeclaration {
    /// What being served looks like. NOT optional.
    pub description: String,
    /// The kind the whole composition yields, when it is fixed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub produces: Option<IdKind>,
}

/// A composition, declared before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
pub struct Composition {
    pub outcome: OutcomeDeclaration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intention: Option<Intention>,
    /// What happens when a stage refuses. Declared, never improvised.
    pub on_stage_refusal: RefusalDisposition,
    #[serde(default)]
    pub meta_detail: MetaDetail,
    /// The SECOND bound layer: over the composition's own output, distinct from the act-level
    /// terms on each stage. A composition never carries a total — with each stage's output the
    /// next stage's domain, a full-composition total is not well-defined.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bounds: BTreeMap<BoundTerm, i64>,
    /// Ordered. Stages reference their inputs explicitly — there is no prev-else-fallback.
    pub stages: Vec<ActInvocation>,
}

impl Composition {
    /// The acts this composition names, in order. Used by the contract-chaining check.
    pub fn act_sequence(&self) -> Vec<&ActName> {
        self.stages.iter().map(|s| &s.act).collect()
    }
}
```

Add `pub mod composition;` and re-export `Composition, Intention, OutcomeDeclaration` from `query/mod.rs`.

- [ ] **Step 8: Run to verify it passes**

Run: `cargo nextest run -p temper-core composition`
Expected: PASS, 4 tests.

- [ ] **Step 9: Commit**

```bash
cargo make generate-ts-types
git add crates/temper-core/src/types/query/ packages/temper-ui/src/lib/types/generated/
git commit -m "feat(query): the composition envelope and its per-stage trace"
```

---

### Task 9: The JSON-Schema snapshot gate

Makes the schema artifact a projection of the code. **No schema is hand-written.**

**Files:**
- Create: `crates/temper-core/tests/query_schema.rs`
- Create: `crates/temper-core/tests/fixtures/query/*.schema.json` (generated, then committed)
- Modify: `tools/cargo-make/main.toml`

**Interfaces:**
- Consumes: every type from Tasks 1–8.
- Produces: `cargo make test-schema` covering temper-core as well as temper-substrate.

- [ ] **Step 1: Confirm `query/mod.rs` re-exports everything the harness names**

The harness below addresses types as `q::<Name>`, so `crates/temper-core/src/types/query/mod.rs` must re-export every type it names. Its final state:

```rust
pub mod act;
pub mod composition;
pub mod disposition;
pub mod envelope;
pub mod filter;
pub mod id_set;
pub mod registry;
pub mod scalars;
pub mod trace;

pub use act::{ActDeclaration, ActName, BuildState, VisibilityProfile};
pub use composition::{Composition, Intention, OutcomeDeclaration};
pub use disposition::{StageDisposition, ActRefusal, RefusalDisposition, RefusalReason};
pub use envelope::{ActInvocation, ActResult, NarrowedBy};
pub use filter::{EdgeFilter, FacetPredicate, FilterField, ResourceFilter};
pub use id_set::{IdKind, IdProvenance, IdSet};
pub use registry::{declaration, search_family};
pub use scalars::{BoundTerm, BoundsMode, Extent, MetaDetail};
pub use trace::{BoundsSource, CompositionTrace, MetaTruncated, StageTrace};
```

Five of these types (`RefusalReason`, `VisibilityProfile`, `NarrowedBy`, `BoundsSource`, `MetaTruncated`) get no `check::<>` call of their own — they are nested inside checked types and land in `$defs`, so they are covered transitively. That is deliberate, not an omission.

- [ ] **Step 2: Write the snapshot harness**

Create `crates/temper-core/tests/query_schema.rs`, modelled directly on `crates/temper-substrate/tests/payload_schema.rs`:

```rust
#![cfg(feature = "mcp")]
//! Query-contract JSON-Schemas are emitted from the SAME structs the wire uses, so the artifact
//! and the code cannot drift. One committed snapshot per type.
//!
//! Regenerate: UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema
//!
//! PACKAGE-SCOPED AND FEATURE-PINNED ON PURPOSE. The emitted schema depends on feature
//! unification: with `mcp` on, the id newtypes emit INLINE (their `schemars(inline)` attribute);
//! under a different feature set they emit as `$ref`s into `$defs`. `mcp` is the authoritative
//! shape here because it is what an MCP tool schema actually carries. See the comment block at
//! tools/cargo-make/main.toml:91.

use temper_core::types::query as q;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/query");

fn check<T: schemars::JsonSchema>(name: &str) {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    let path = format!("{DIR}/{name}.schema.json");
    if std::env::var("UPDATE_SCHEMA").is_ok() {
        std::fs::create_dir_all(DIR).unwrap();
        std::fs::write(&path, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        rendered, committed,
        "{name} query schema drifted — re-run with UPDATE_SCHEMA=1"
    );
}

#[test]
fn query_contract_schemas_match_snapshots() {
    check::<q::IdSet>("id_set");
    check::<q::IdKind>("id_kind");
    check::<q::IdProvenance>("id_provenance");
    check::<q::Extent>("extent");
    check::<q::BoundTerm>("bound_term");
    // EdgeKind is deliberately absent: it belongs to `types::graph`, is `sqlx::Type`-bound to the
    // DDL, and is snapshotted through `EdgeFilter` rather than as a query-owned type.
    check::<q::EdgeFilter>("edge_filter");
    check::<q::ResourceFilter>("resource_filter");
    check::<q::FilterField>("filter_field");
    check::<q::BoundsMode>("bounds_mode");
    check::<q::MetaDetail>("meta_detail");
    check::<q::StageDisposition>("disposition");
    check::<q::ActRefusal>("refusal");
    check::<q::RefusalDisposition>("refusal_disposition");
    check::<q::ActName>("act_name");
    check::<q::BuildState>("build_state");
    check::<q::ActDeclaration>("act_declaration");
    check::<q::ActInvocation>("act_invocation");
    check::<q::ActResult>("act_result");
    check::<q::StageTrace>("stage_trace");
    check::<q::CompositionTrace>("composition_trace");
    check::<q::Intention>("intention");
    check::<q::OutcomeDeclaration>("outcome_declaration");
    check::<q::Composition>("composition");
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo nextest run -p temper-core --features mcp --test query_schema`
Expected: FAIL — every `check` asserts against an empty string, because no snapshot is committed yet.

- [ ] **Step 4: Generate the snapshots**

Run: `UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema`
Then inspect at least `id_kind.schema.json` and confirm the domain names appear and no `kb_` prefix does:

```bash
rg -n "kb_" crates/temper-core/tests/fixtures/query/*.schema.json && echo "FAIL: table names leaked" || echo "OK: no table names"
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo nextest run -p temper-core --features mcp --test query_schema`
Expected: PASS.

- [ ] **Step 6: Wire it into cargo-make**

In `tools/cargo-make/main.toml`, rename the existing `[tasks.test-schema]` to `[tasks.test-schema-substrate]` (keeping its full comment block verbatim — it explains the package-scoping rule), add:

```toml
[tasks.test-schema-core]
description = "Run the query-contract JSON-Schema snapshot gate. Regenerate: UPDATE_SCHEMA=1 cargo make test-schema-core"
# PACKAGE-SCOPED AND FEATURE-PINNED for the same reason as test-schema-substrate: the emitted
# schema depends on feature unification. `mcp` is the authoritative shape because it is what an
# MCP tool schema carries.
command = "cargo"
args = ["nextest", "run", "-p", "temper-core", "--features", "mcp", "--test", "query_schema", "--no-fail-fast"]

[tasks.test-schema]
description = "All JSON-Schema snapshot gates"
dependencies = ["test-schema-substrate", "test-schema-core"]
```

`[tasks.test]` already depends on `test-schema`, so both gates now run under `cargo make test` with no further edit — confirm by reading the `[tasks.test]` block.

- [ ] **Step 7: Verify the whole gate runs**

Run: `cargo make test-schema`
Expected: both suites pass.

Run: `cargo make test`
Expected: passes, and the output shows both schema suites ran.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/tests/ tools/cargo-make/main.toml
git commit -m "test(query): committed JSON-Schema snapshot gate for the v0 contract"
```

---

### Task 10: Full check

**Files:**
- None. This task changes nothing; it verifies.

**Interfaces:**
- Consumes: everything.
- Produces: a green `cargo make check` and a green `cargo make test`.

> There is deliberately **no contract-document reconciliation step**. The published contract is
> *generated* from these types in T3 and lives with the other generated artifacts — so there is no
> hand-written second copy to reconcile, and nothing here to remember to run.

- [ ] **Step 1: Run the full quality gate**

Run: `cargo make check`
Expected: clean — fmt, clippy, docs, machete, TS typecheck, biome, and every drift gate (`openapi-check`, `ts-rs-drift`, `skills-drift`).

If `ts-rs-drift` reds, a previous task skipped `cargo make generate-ts-types`. Run it and commit the regenerated tree.

- [ ] **Step 2: Run the full test suite**

Run: `cargo make test`
Expected: passes, including both schema gates.

- [ ] **Step 3: Check the UI**

Run: `cd packages/temper-ui && bun run check && cd ../..`
Expected: clean. `cargo make check` does not cover this.

- [ ] **Step 4: Confirm the snapshots are committed and current**

Run: `git status --short crates/temper-core/tests/fixtures/query/`
Expected: no output — every emitted snapshot is committed. An uncommitted snapshot means the gate would pass locally and red in CI.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore(query): full check green for the v0 contract types"
```

---

## Deliberately not in this plan

Named so their absence reads as a decision rather than an oversight:

- **The executor.** No jaq evaluation, no stage sequencing, no composition running. v0 is a contract.
- **Any act implementation, route, or MCP tool.** Which is why nothing is registered in `openapi.rs` components.
- **The five gates.** T3 builds them; this plan produces the *data* they check (`build_state`, `accepts_*`, `scoring_revision`, `served_by`).
- **The published contract artifact**, and the deliberate usability review of the schema once it exists. Both are T3's, and both depend on these types being in place first.
- **`params<act>` and `meta<act>` extension payloads.** The base envelope declares the seam; the per-act extensions arrive with the acts.
- **`unified_search` expressed as the first named plan** (design §7). It is expressible under this contract, and writing it down is what makes its `blend0` sum legible as a defect — but *running* it needs the executor, so authoring it here would produce a plan nothing can validate. It is the natural first entry once an executor exists.
- **Converging the six incumbent tagged-id patterns** (design §3.1.2).
- **The four out-of-scope families.** A separate pass, deliberately scheduled before any of this is built.
