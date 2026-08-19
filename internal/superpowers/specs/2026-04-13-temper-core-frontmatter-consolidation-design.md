# Temper-Core Frontmatter Consolidation — Design Spec

**Date:** 2026-04-13
**Task:** `2026-04-13-consolidate-frontmatter-handling-in-temper-core`
**Mode:** build
**Effort:** large (3 sessions)
**Branch:** `jct/frontmatter-consolidation`
**Related work:**
- Supersedes Part A (Known Open Fields Registry) of `2026-04-11-open-meta-intentionality-and-graph-build-design.md`, which is deferred until this work lands.
- Prerequisite for `2026-04-11-knowledge-graph-ui-and-seeding-the-vault-for-relationships` (parent task, in-progress).
- Builds on unified hash work shipped in PR #40 (`fix: unified hash computation to eliminate sync push/pull cycle`).

---

## Problem

Frontmatter parsing, validation, tier splitting, and write-back logic is currently scattered across at least four modules in `temper-core` and called directly from ~15 sites in `temper-cli` and `temper-api`:

| Module | Responsibility | Key exports |
|--------|----------------|-------------|
| `temper-core/src/hash.rs` | Tier routing, hashing | `split_frontmatter_tiers`, `compute_frontmatter_hashes_from_yaml`, `compute_managed_hash`, `compute_open_hash`, `IDENTITY_FIELDS`, `TIER1_SYSTEM_FIELDS` |
| `temper-core/src/normalize.rs` | Text-level block split, apply defaults, rewrite on disk | `split_frontmatter_block`, `normalize_file` |
| `temper-core/src/schema.rs` | JSON Schema validation, introspection, legacy/unknown detection | `validate_frontmatter`, `validate_allowing_provisional`, `schema_value`, `KNOWN_TEMPER_FIELDS`, `LEGACY_FIELDS`, `SYSTEM_MANAGED_FIELDS` |
| `temper-core/src/types/{vault,managed_meta,graph,resource}.rs` | Typed projections | `ResourceFrontmatter`, `ManagedMeta`, `ResourceRelationships`, `Resource` |

Consumer call sites include `temper-cli/src/actions/sync.rs` (four `split_frontmatter_tiers` call sites: two production at lines 801 and 918, two test at lines 2249 and 3186), `actions/doctor_fix.rs` (ad-hoc YAML write-back), `actions/ingest.rs`, `actions/doctor.rs`, `vault.rs`, and `temper-api/src/services/meta_service.rs` (JSON-only, not migrated — see Scope Boundaries).

This functional spread has introduced subtle bugs more than once. Two concrete examples currently in the tree:

1. **Accidental tier routing.** `split_frontmatter_tiers` calls `schema::schema_value(doc_type)` which only returns the doc-type's *own* `properties` object — not the fields merged from `base.schema.json` via `$ref`. Base-schema fields like `relates_to`, `depends_on`, `tags` fall through to `open_meta` because they're not in `schema_keys`. The observable behavior is correct (known open fields stay in open_meta), but it is correct *by accident of `$ref` not being followed*, not by design. A future refactor that makes schema introspection follow `$ref` would silently change tier routing and break sync hashes.

2. **`tags` phantom edges.** `ResourceRelationships::to_edge_declarations` at `graph.rs:131` maps the `tags` field to `EdgeType::TaggedWith` and runs `TargetRef::parse` on each value. Plain-string tags like `"auth"` or `"observability"` parse successfully as slugs, producing edges to nonexistent resources. `tags` is an Obsidian-compatible string vector, not a resource relationship.

Beyond these, the architectural smell matters on its own: "which of four modules do I use to parse a vault file?" is not a question a developer should have to answer. The scattered state also blocks the `temper graph build` work in Spec 2, which needs a single authoritative parse/write path it can safely mutate frontmatter through.

This task consolidates all frontmatter handling into a single `temper-core::frontmatter` module, retires the scattered public APIs, establishes robust `From`/`TryFrom` projections to the existing typed structs, and fixes the `tags` phantom-edge bug as a drive-by.

## Approach

**New module:** `crates/temper-core/src/frontmatter/` becomes the single authoritative path for reading, validating, mutating, and writing vault frontmatter. Built around an aggregate `Frontmatter` type that holds the canonicalized YAML and exposes typed projections via standard trait impls.

**Full consolidation (option C):** Every existing frontmatter read/write call site in `temper-cli` and `temper-core` migrates to the new module. Old public APIs (`split_frontmatter_tiers`, `split_frontmatter_block`) are deleted, not deprecated. The project is pre-alpha; no backward compatibility burden.

**Display canonicalization for writes, existing hash canonicalization for sync.** These are two distinct algorithms serving two distinct purposes. The new module owns the first and delegates the second unchanged to `hash::compute_managed_hash`/`compute_open_hash` from PR #40.

**Alias normalization at the parse boundary, canonical form everywhere downstream.** Hyphenated aliases (`relates-to`, `depends-on`, etc.) are normalized to canonical underscore form exactly once, during `TryFrom<&str> for Frontmatter`. After construction, `Frontmatter::value` contains only canonical keys. Mutation APIs accept canonical form only.

**Typed structs stay where they are.** `ResourceRelationships` stays in `graph.rs`, `ManagedMeta` in `managed_meta.rs`, `ResourceFrontmatter` in `vault.rs`. They are projection targets consumed via `From<&Frontmatter>` and `TryFrom<&Frontmatter>` trait impls. Moving them would force churn across `temper-api` and the TypeScript-generated bindings with no clarity gain.

## Module Layout

```
crates/temper-core/src/frontmatter/
├── mod.rs           # public API re-exports, module docs
├── document.rs      # Frontmatter aggregate type + TryFrom<&str> + serialize + write_to
├── parse.rs         # text block split, YAML parse, alias normalization at boundary
├── tiers.rs         # managed/open split (follows base ∪ doc-type schema explicitly)
├── canonical.rs     # 5-tier display ordering for serialize()
├── registry.rs      # KNOWN_OPEN_FIELDS + alias lookup + field categories
├── fields.rs        # IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS, SYSTEM_MANAGED_FIELDS (consolidated)
└── projections.rs   # From<&Frontmatter> and TryFrom<&Frontmatter> for typed structs
```

Broken into focused files rather than one monolith because the logic has several distinct concerns. Each file is independently testable and stays under a manageable size.

## Data Model

### Central type

```rust
pub struct Frontmatter {
    doc_type: DocType,            // enum, not &str — parses at boundary
    value: serde_yaml::Value,     // canonicalized YAML (aliases already normalized)
    body: String,                 // markdown body, preserved byte-for-byte
}
```

**Invariants maintained internally:**

1. `value` is always alias-normalized (hyphens → underscores) and schema-valid for `doc_type`. Construction that produces a value failing either check returns `Err` from `TryFrom`.
2. `doc_type` is a typed enum, not a free-form string. Unknown doctypes fail at parse, not at validation.
3. `body` is preserved byte-for-byte including trailing newlines. Writes concatenate canonical frontmatter + `---\n` + body.

**`DocType` enum** — added if not already present as a typed wrapper:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocType {
    Task, Goal, Session, Research, Decision, Concept,
}

impl DocType {
    pub fn as_str(&self) -> &'static str { /* … */ }
    pub fn from_str(s: &str) -> Result<Self> { /* … */ }
}
```

### Public API surface

```rust
// Parse + validate
impl TryFrom<&str> for Frontmatter { type Error = TemperError; }
impl Frontmatter {
    pub fn parse_file(path: &Path) -> Result<Self>;
    pub fn validate(&self) -> Vec<ValidationIssue>;
    pub fn doc_type(&self) -> DocType;
}

// Projection — read direction, trait impls
impl From<&Frontmatter> for ResourceRelationships;         // infallible via #[serde(default)]
impl TryFrom<&Frontmatter> for ManagedMeta;                // fails on schema mismatch
impl TryFrom<&Frontmatter> for ResourceFrontmatter;        // fails on missing required

// Mutation — write direction, explicit methods (not From, because it's a merge not a replace)
impl Frontmatter {
    pub fn set_relationships(&mut self, rels: &ResourceRelationships);
    pub fn set_managed_field(&mut self, key: &str, value: serde_json::Value);
    pub fn set_open_field(&mut self, key: &str, value: serde_json::Value);
    pub fn remove_field(&mut self, key: &str);
}

// Write
impl Frontmatter {
    pub fn serialize(&self) -> String;                     // canonical text representation
    pub fn write_to(&self, path: &Path) -> Result<()>;
}

// Tier views + hashing (replaces hash::split_frontmatter_tiers + hash::compute_frontmatter_hashes_from_yaml)
impl Frontmatter {
    pub fn managed_json(&self) -> serde_json::Value;
    pub fn open_json(&self) -> serde_json::Value;
    pub fn hashes(&self) -> (String, String);              // (managed_hash, open_hash)
}

// Registry (re-exported from frontmatter::registry)
pub use registry::{KNOWN_OPEN_FIELDS, KnownOpenField, OpenFieldType, FieldCategory};
```

### Projection rationale: `From` for reads, explicit methods for writes

The read direction uses standard trait impls because projecting a `Frontmatter` to a typed struct is a pure computation — no state mutation, no merge semantics, no loss of information beyond what the target struct can represent. `From<&Frontmatter> for ResourceRelationships` is infallible because every field is `#[serde(default)]`. `TryFrom<&Frontmatter> for ManagedMeta` can fail when required fields are missing or have wrong types.

The write direction does **not** use `From<ResourceRelationships> for Frontmatter`. That would masquerade a merge operation as a replacement: applying a `ResourceRelationships` to a `Frontmatter` must preserve the non-projected fields (body content, other frontmatter keys, managed tier values). Wrapping a merge in `From` is misleading. Explicit methods (`set_relationships`, `set_managed_field`, etc.) make the mutation semantics visible at call sites.

## Canonical Display Ordering

When `Frontmatter::serialize()` runs, keys emerge in this deterministic order regardless of the input YAML's original ordering:

1. **Identity fields** — `temper-id`, `temper-provisional-id` (fixed order)
2. **Tier-1 system fields** — `temper-type`, `temper-context`, `temper-owner`, `temper-created`, `temper-updated`, `temper-source` (fixed order)
3. **Managed fields** — `title`, `slug`, then doc-type-specific fields in schema-declaration order (walks resolved base ∪ doc-type schema)
4. **Known open fields** — relationships first in registry order (`relates_to`, `depends_on`, `extends`, `references`, `preceded_by`, `derived_from`, `parent`), then metadata (`tags`, `aliases`, `date`)
5. **Unknown open fields** — preserved in original input order. If the YAML was originally emitted by this module (no preexisting user order), they're alphabetized as a tie-break.

**Properties this gives us:**

- **Determinism** — two different input orderings of the same fields produce byte-identical output.
- **Diff-friendliness** — `git diff` on a re-saved file shows only meaningful content changes, never reshuffles.
- **Respects user intent for unknown fields** — custom open fields added by hand aren't reshuffled relative to each other; they're just placed after the known block as a group.

### This ordering is for display only

Canonical display ordering is for writes to disk. **It is not used for hashing.** See the Hash Stability Invariants section below for why this distinction matters and how the two algorithms relate.

## Registry: Known Open Fields

```rust
pub struct KnownOpenField {
    pub canonical: &'static str,
    pub aliases: &'static [&'static str],
    pub field_type: OpenFieldType,
    pub category: FieldCategory,
}

pub enum OpenFieldType {
    StringList,  // Vec<String>
    String,      // single string
    Tags,        // Vec<String> with Obsidian tag semantics (NOT resource refs)
}

pub enum FieldCategory {
    Relationship,  // drives edge extraction in edge_service
    Metadata,      // Obsidian-compatible universals, non-relational
}

pub const KNOWN_OPEN_FIELDS: &[KnownOpenField] = &[
    // Relationships (drive edges)
    KnownOpenField { canonical: "relates_to",   aliases: &["relates-to"],   field_type: StringList, category: Relationship },
    KnownOpenField { canonical: "depends_on",   aliases: &["depends-on"],   field_type: StringList, category: Relationship },
    KnownOpenField { canonical: "extends",      aliases: &[],                field_type: StringList, category: Relationship },
    KnownOpenField { canonical: "references",   aliases: &[],                field_type: StringList, category: Relationship },
    KnownOpenField { canonical: "preceded_by",  aliases: &["preceded-by"],  field_type: StringList, category: Relationship },
    KnownOpenField { canonical: "derived_from", aliases: &["derived-from"], field_type: StringList, category: Relationship },
    KnownOpenField { canonical: "parent",       aliases: &[],                field_type: String,     category: Relationship },
    // Metadata (Obsidian-compatible, non-relational)
    KnownOpenField { canonical: "tags",         aliases: &[],                field_type: Tags,       category: Metadata },
    KnownOpenField { canonical: "aliases",      aliases: &[],                field_type: StringList, category: Metadata },
    KnownOpenField { canonical: "date",         aliases: &[],                field_type: String,     category: Metadata },
];
```

**Naming convention:** canonical form uses underscores (`relates_to`); hyphenated aliases are accepted at parse time and normalized to canonical form. This matches the existing `ResourceRelationships` struct fields and all deployed code. The inconsistency with `temper-*` hyphens is deliberate: the `temper-` prefix signals a namespace difference — system fields use hyphens, user-owned open fields use underscores.

**Alias handling lives exactly in one place:** `frontmatter::parse::normalize_aliases(&mut serde_yaml::Value)`, called once during `TryFrom<&str> for Frontmatter`. After that call, every downstream consumer (tier split, projection, write) sees canonical form only. Mutation APIs accept canonical form only; passing an alias is a consumer bug.

## Hash Stability Invariants

This section is load-bearing. The existing unified hash work in PR #40 eliminated a class of sync-cycling bugs, and this consolidation must not regress it.

### The two canonicalizations are distinct

**Display canonicalization** — the 5-tier grouped order from the Canonical Display Ordering section. Used by `Frontmatter::serialize()`. Produces human-friendly, diff-friendly YAML text for writing to disk.

**Hash canonicalization** — recursive alphabetical `BTreeMap` sort via `hash::canonicalize_json` at `hash.rs:49`. Used exclusively inside `hash::compute_managed_hash` / `compute_open_hash`. Produces bit-deterministic JSON serialization for SHA-256 digest.

**These are never conflated.** `Frontmatter::hashes()` delegates unchanged to the existing unified hash functions:

```rust
impl Frontmatter {
    pub fn hashes(&self) -> (String, String) {
        let managed = self.managed_json();
        let open = self.open_json();
        (
            hash::compute_managed_hash(self.doc_type.as_str(), &managed),
            hash::compute_open_hash(&open),
        )
    }
}
```

The display-ordering algorithm in `frontmatter::canonical` has no bearing on hash output. A regression test asserts that input YAML with randomized key order produces the same `Frontmatter::hashes()` result as the same YAML in canonical display order — confirming the two algorithms are independent.

### Both sides share the same hash path

Client-side (`temper-cli/src/actions/sync.rs`) and server-side (`temper-api/src/services/ingest_service.rs` at `:296` and `:526`) already call the same `compute_managed_hash` / `compute_open_hash` functions. This consolidation preserves that symmetry exactly. Any test that constructs JSON server-side and compares hashes with a client-side `Frontmatter::hashes()` result must produce byte-identical digests.

### Alias-containing input hashes identically to canonical input

Because `Frontmatter::parse` normalizes aliases at the boundary, a YAML file with `relates-to: [foo]` produces the exact same `open_json()` as a YAML file with `relates_to: [foo]`. Both hash identically. A dedicated test locks this in: parse alias-form YAML → project → hash; construct equivalent canonical JSON → hash; assert equal. This prevents the sync-cycling class of bug where client and server disagree on whether two files with different alias forms are "the same."

### Known smell: `compute_managed_hash` applies defaults at hash time

`compute_managed_hash` at `hash.rs:84` clones its input, applies doc-type defaults to the clone, and hashes the defaulted clone. The original input is not mutated. This means the hash is "what it would hash as if defaults were applied" regardless of whether defaults are actually present in the stored representation.

If the server ever wrote `managed_meta` to jsonb without applying defaults at ingest time, `compute_managed_hash` on that stored value would still return the "defaults applied" hash, and sync comparison would silently accept the default-less jsonb as "equal" to a client-side defaulted file. Nothing would ever correct the stored value.

This is **not a bug** today — `ingest_service.rs` does apply defaults at create/update time — but it is a smell: the hash function hides state from its callers, and the implicit invariant "server ingest writes defaults at create/update" lives only in convention, not enforcement.

**Explicitly out of scope for this task.** Touching `compute_managed_hash` re-opens the sync protocol that just stabilized in PR #40. Instead, we document the smell in "Future Work" below and recommend a separate small task: a pre-write jsonb validator in `temper-api` that asserts `managed_meta` is defaults-applied before INSERT/UPDATE, plus a one-time data cleanup if any historical rows are missing defaults.

## Drive-by Fix: `tags` Phantom Edges

`ResourceRelationships::to_edge_declarations` at `graph.rs:131` currently maps `tags` to `EdgeType::TaggedWith` and runs `TargetRef::parse` on every tag value. Plain-string tags parse as slugs and produce edges pointing at nonexistent resources. Tags are an Obsidian-compatible string vector, not a resource relationship type.

**Fix scope (session 2):**

1. Remove `tags` field from `ResourceRelationships` struct in `graph.rs:87`.
2. Remove `(&self.tags, EdgeType::TaggedWith)` mapping from `to_edge_declarations` at `graph.rs:131`.
3. Remove `tags` from `ResourceRelationships::is_empty()` check at `graph.rs:108`.
4. Delete `EdgeType::TaggedWith` variant entirely — gated on verifying zero `kb_resource_edges` rows currently use it (one SQL query before deletion: `SELECT COUNT(*) FROM kb_resource_edges WHERE edge_type = 'tagged_with';`).
5. Regenerate TypeScript bindings (`graph.ts` loses `tags` from `ResourceRelationships`).
6. Verify `temper-ui` doesn't reference `relationships.tags` anywhere (grep + build).
7. Add `Frontmatter::tags() -> Vec<String>` as the typed accessor for the `tags` field, reading from `open_meta["tags"]`.
8. Verify `edge_service.rs`'s tests at `:556-607` still pass (none should assert on `tags` producing edges; if they do, fix them).

If a later need arises for tag-based graph links, we'll reintroduce them via a dedicated `kb_tag_edges` table or similar — not by conflating "document has tag X" with "document links to resource X."

## Migration Plan

Three sessions, each ending green and independently committable as its own PR.

### Session 1 — Foundation (additive only, no consumer changes)

**Goal:** Build the new module end-to-end with nothing consuming it yet. Pure addition, zero risk to existing code.

**Work:**
- Create `crates/temper-core/src/frontmatter/` module with all files listed in Module Layout
- Add `DocType` enum if not already present
- Implement `TryFrom<&str> for Frontmatter` + projections + mutation methods + `serialize` + `write_to` + `hashes`
- Implement `KNOWN_OPEN_FIELDS` registry in `frontmatter::registry`
- Consolidate field constants in `frontmatter::fields` (re-exported from old locations *for session 1 only* to avoid breaking imports)
- Full unit test suite (see Test Strategy below)
- Full integration test suite with synthetic fixtures
- `cargo make check` clean, full test suite green

**Verification before commit:** Manually run `cargo run --bin temper -- doctor` against `/Users/petetaylor/projects/kb-vault` as a sanity check. (Not a formal test; just developer discipline.)

**Commit:** `feat: new temper-core::frontmatter module (not yet consumed)`

**PR:** Opens independently. Zero behavior change to existing code paths — the new module sits in the tree, nothing imports it from production code, but imports from tests.

### Session 2 — Sync-sensitive migration + `tags` bug fix

**Goal:** Migrate the highest-risk call sites (`sync.rs` and `normalize.rs`) to the new module and land the `tags` phantom-edge fix. Highest-risk session — touches code paths that just stabilized in PR #42.

**Discipline:** Single-subagent dispatch with full matrix, plan-reality verification before dispatch, independent verification of all test results before committing. Same pattern as the last three sessions.

**Migrations:**
- `normalize::normalize_file` — **stays as an orchestrator** but has its guts rewritten. Its role continues to be: parse a vault file, apply doc-type defaults via the existing `crate::defaults::apply_doc_type_defaults` free function, and write back if anything changed. The new implementation does this through `Frontmatter::parse_file` → mutate value to apply defaults → `Frontmatter::write_to`. The "apply defaults" responsibility stays in `normalize_file`, not inside `Frontmatter` itself — `Frontmatter` is a transparent data holder; `normalize_file` is a higher-level orchestrator that decides when defaults should be applied.
- `sync.rs`'s four `split_frontmatter_tiers` call sites (production at lines 801 and 918; test at lines 2249 and 3186) — replaced with `Frontmatter::managed_json` / `open_json` / `hashes`. Comment references at lines 842 and 3182 are updated to point at the new APIs.
- `sync.rs`'s ad-hoc YAML reading (e.g., `rehash_manifest`) — replaced with `Frontmatter::parse_file`
- Delete `hash::split_frontmatter_tiers` (public → gone)
- Delete `hash::compute_frontmatter_hashes_from_yaml` (public → gone; replaced by `Frontmatter::hashes`)
- Delete `normalize::split_frontmatter_block` (public → gone)

**`tags` bug fix** (per Drive-by Fix section above, bundled here because context is right):
- Remove `tags` from `ResourceRelationships` struct + methods
- Delete `EdgeType::TaggedWith` variant after verifying zero production rows use it
- Regenerate TypeScript bindings
- Verify `temper-ui` doesn't reference `relationships.tags`
- Add `Frontmatter::tags()` accessor

**Verification before commit:**
- `cargo nextest run --workspace --features test-db` — full Rust suite
- `cargo nextest run -p temper-e2e --features test-db` — all e2e tests including Phase E2 sync suite
- `cargo make check` — clean
- Manual `target/debug/temper sync run --dry-run` against the real vault, asserting no unexpected content changes
- Independent spot-check of any sync-test adaptations

**Commit:** `refactor: migrate sync + normalize to temper-core::frontmatter; remove phantom tag edges`

**PR:** Opens independently. The riskiest of the three — dedicated review.

### Session 3 — Retire remaining APIs + `doctor --fix aliases`

**Goal:** Finish the consolidation. Migrate the remaining read-heavy paths, retire old public APIs entirely, and land the forward-looking alias canonicalization command.

**Migrations:**
- `doctor_fix.rs` YAML write path → `Frontmatter::write_to`
- `doctor.rs`, `ingest.rs`, `vault.rs` parse paths → `Frontmatter::parse_file`
- `schema_test.rs` and other tests → new module APIs
- `temper-api/src/services/meta_service.rs` — imports `KNOWN_OPEN_FIELDS` from the new module for JSON-level field validation (does not consume `Frontmatter` itself — see Scope Boundaries)
- Move `KNOWN_TEMPER_FIELDS`, `LEGACY_FIELDS`, `SYSTEM_MANAGED_FIELDS` from `schema.rs` to `frontmatter::fields` (delete the re-exports from session 1)
- Delete `schema::validate_frontmatter` / `validate_allowing_provisional` if no callers remain (otherwise leave as thin wrappers delegating to `Frontmatter::validate`)

**New feature:** `temper doctor --fix aliases`
- Walks vault, parses each file through `Frontmatter`, and writes it back via `Frontmatter::write_to`
- Files already in canonical form produce byte-identical output (parse + canonical serialize is a fixed point), so the rewrite is effectively a no-op on canonical files
- Files with alias-form keys get canonicalized on the first pass
- Compares pre/post file bytes and reports the count of files actually changed (distinguishing "aliased → canonicalized" from "already canonical")
- Idempotent: second run reports zero changes
- Integration test with synthetic fixture containing hyphen-form keys
- No need for an `original_had_aliases` flag on `Frontmatter` — detection is a byte comparison of the on-disk text vs. the serialized output

**Verification before commit:**
- Full test suite green
- `cargo machete` clean (no unused deps from the retirement)
- `cargo make check` clean
- `grep -rn "split_frontmatter_tiers\|split_frontmatter_block"` returns zero production-code hits
- Manual run of `temper doctor --fix aliases --dry-run` against the real vault (should report zero changes, since the vault is already canonical)

**Commit:** `refactor: retire scattered frontmatter APIs; add doctor --fix aliases`

**PR:** Opens independently. Cleanup-flavored, quick review.

### After session 3

- Parent KG UI task `2026-04-11-knowledge-graph-ui-and-seeding-the-vault-for-relationships` resumes.
- Spec 2 (`temper graph build`) comes off deferred status and its implementation plan is written in a subsequent session, now consuming the stable `Frontmatter` module as its parse/write foundation.
- Part A of Spec 2 (Known Open Fields Registry) is already landed here, so Spec 2 simplifies to just the `temper graph build` pipeline.

## Test Strategy

### Unit tests (inline `#[cfg(test)] mod tests` in each file)

- `parse.rs` — `normalize_aliases`, text-block split, YAML parse error handling
- `tiers.rs` — tier routing for each field category, including fields that live only in `base.schema.json`
- `canonical.rs` — key ordering across the 5 tiers, determinism under randomized input permutations
- `registry.rs` — lookup by alias, lookup by canonical, category classification, coverage of every entry in `KNOWN_OPEN_FIELDS`
- `fields.rs` — consolidated constants match old-location definitions exactly during session 1's dual-location phase
- `projections.rs` — `From<&Frontmatter> for ResourceRelationships` for every relationship field; `TryFrom<&Frontmatter> for ManagedMeta` success and failure paths
- `document.rs` — `TryFrom<&str>` round-trip, mutation API (`set_relationships`, `set_managed_field`, `remove_field`), `serialize` determinism

### Integration tests (`crates/temper-core/tests/frontmatter_test.rs` + fixtures)

```
crates/temper-core/tests/
├── frontmatter_test.rs                  # driver, shared helpers
└── fixtures/
    └── frontmatter/
        ├── task_minimal.md              # required fields only
        ├── task_full.md                 # every field populated
        ├── task_with_aliases.md         # relates-to, depends-on, etc. in hyphen form
        ├── task_hand_ordered.md         # user-ordered keys with intermixed unknown fields
        ├── goal_minimal.md, goal_full.md
        ├── session_full.md
        ├── research_full.md
        ├── decision_full.md
        ├── concept_full.md
        ├── malformed_yaml.md            # parse error case
        ├── wrong_doc_type.md            # unknown temper-type
        ├── missing_required.md          # required field absent
        ├── tags_as_strings.md           # tags: [auth, observability] — must NOT produce edges
        └── golden/                      # expected canonical output for each round-trippable fixture
            ├── task_minimal.canonical.md
            ├── task_full.canonical.md
            ├── task_with_aliases.canonical.md    # canonical form emerges after serialize
            └── ...
```

**Integration test categories:**

1. **Projection round-trip stability** — parse each fixture → project to typed struct → write → re-parse → byte-equal to golden
2. **Projection losslessness** — for each field in each projected struct, parse + project + reconstruct covers it
3. **Mutate-and-write round-trip** — parse → `set_relationships(new)` → write → re-parse → `relationships()` == new
4. **Alias normalization** — `task_with_aliases.md` parses into canonical-form `Frontmatter`; serialization produces `task_with_aliases.canonical.md`
5. **Write canonicalization** — all six doctype fixtures round-trip to their goldens byte-identically
6. **Tier-split hash stability** — for each fixture, `Frontmatter::hashes()` equals the result of running the old `hash::compute_frontmatter_hashes_from_yaml` path. Regression anchor that proves we're not changing hash semantics.
7. **Display-ordering determinism** — parse the same YAML with three different key permutations, assert all three serialize to identical output
8. **Display-ordering does not affect hashes** — parse the same YAML with three different key permutations, assert all three produce identical `hashes()` output (locks display canonicalization and hash canonicalization as independent)
9. **Alias-containing hash symmetry** — parse alias-form YAML → project → hash; construct equivalent canonical JSON → `compute_open_hash` directly; assert equal (prevents sync-cycling)
10. **Error cases** — `malformed_yaml.md`, `wrong_doc_type.md`, `missing_required.md` produce expected `TemperError` variants
11. **`tags` is not a relationship** — `tags_as_strings.md` projects to `ResourceRelationships` with no `TaggedWith` edges; `Frontmatter::tags()` returns the string vec

### E2E tests

Added in session 2 and session 3, not session 1:

- **Session 2:** No new e2e tests — the existing Phase E2 sync suite (98+ tests) is the regression anchor. If something surfaces, add tests to cover the gap.
- **Session 3:** `temper doctor --fix aliases` e2e test — create a synthetic vault with hyphen-form keys, run the command, verify canonicalized output. Second run is a no-op (idempotency).

### Per-session gates

Every session's final commit must pass:
- `cargo make check` (fmt, clippy, docs, machete, TS typecheck, biome)
- `cargo nextest run --workspace --features test-db` (full Rust suite)
- `cargo nextest run -p temper-e2e --features test-db` (e2e suite)

Manual sanity check against the real vault once per session before committing. Not a formal test; a developer discipline.

## Scope Boundaries

### In scope

- New `temper-core::frontmatter` module with full public API
- `KNOWN_OPEN_FIELDS` registry + alias normalization
- Migration of every frontmatter read/write call site in `temper-cli` and `temper-core`
- Retirement of `hash::split_frontmatter_tiers`, `normalize::split_frontmatter_block`, and ad-hoc YAML write paths in `doctor_fix.rs`
- `tags` phantom-edge bug fix in `ResourceRelationships`
- `temper doctor --fix aliases` command
- Full unit + integration test coverage for every doctype

### Out of scope

- Server-side `meta_service.rs` migration. It operates on JSON values already and never touches YAML or files. It imports `KNOWN_OPEN_FIELDS` from the new module for JSON-level field validation, but does not consume `Frontmatter` itself. The shared types (`ManagedMeta`, `ResourceRelationships`) remain its wire contract.
- Refactoring `compute_managed_hash` to require pre-defaulted input. Documented as a known smell in "Future Work"; a separate task can address it.
- `compute_open_hash` — no changes.
- Format-preserving YAML writes (comment preservation). The project owns its vault; we acknowledge Obsidian compatibility without deferring to it. Comments in frontmatter are not preserved, and we do not need to. Verified: zero `#`-prefixed lines exist in frontmatter blocks across the current 746-file vault.
- `temper graph build` pipeline (Spec 2). Deferred until this task's session 3 merges.
- LLM-inferred relationships. Deferred indefinitely.

### Adjacent / related

- `2026-04-11-open-meta-field-filtering-in-search` benefits from the `KNOWN_OPEN_FIELDS` registry for `meta_filters` support but is a separate task.
- `2026-04-11-open-meta-intentionality-and-graph-build-design.md` spec is partially superseded: Part A (Known Open Fields Registry) moves into this spec; Part B (`temper graph build`) stays deferred.

## Future Work

- **Pre-write JSONB validator in `temper-api`.** A validator that asserts `managed_meta` is defaults-applied (and alias-free, and schema-valid) before INSERT/UPDATE to `kb_resource_manifests`. Plus a one-time data cleanup pass if any historical rows are missing defaults. Makes the implicit "server ingest applies defaults at write time" invariant explicit and enforced. Low effort; just hasn't been a blocker yet.
- **Refactor `compute_managed_hash` to require pre-defaulted input.** Pushes the defaults-application responsibility onto callers and makes the stored-form-equals-hashed-form invariant explicit. Requires touching the sync protocol, so gated on appetite for protocol re-opening.
- **Format-preserving YAML writes.** If ever actually needed, upgrade the serializer to use a comment-preserving AST library (`saphyr` or similar). The `Frontmatter` type's data model doesn't change — only `serialize()`'s implementation. Drop-in replacement.

## Dependencies

- Unified hash computation (PR #40) — done
- Three-tier sync meta-only path (PR #42) — done
- R7 Phases 1-4 knowledge graph foundations (PR #41) — done

No new dependencies on external work. This task can start immediately on the `jct/frontmatter-consolidation` branch.

## Estimated Effort

**Large — 3 sessions** with the partitioning described in Migration Plan above. Session 1 is additive and low-risk; session 2 is high-risk (sync-sensitive) and requires independent verification discipline; session 3 is medium-risk cleanup plus one new feature.

If session 2 stretches, the `tags` bug fix absorbs slack by moving to session 3 as a pure drive-by, keeping session 2 focused on sync migration. Worst case is 4 sessions. Bet is 3.
