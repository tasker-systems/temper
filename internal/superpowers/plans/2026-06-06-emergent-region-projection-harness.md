# Emergent Region Projection — Plan 2: The `temper-next` Clustering Harness

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use `- [ ]` checkboxes.
>
> **GROUNDING DISCIPLINE (inject + obey):** `~/.claude/skills/temper/guidance/implementation-grounding.md` — GD-1 cite-or-it's-invention, GD-2 executable grounding (`cargo test`, `psql` verdicts), GD-3 CONFORM/EXTEND/AMEND tags, GD-5 escalate-don't-fabricate.

> ## ✅ RE-GROUNDED 2026-06-06 (post-Plan-1) — READY TO EXECUTE
> Plan 1 shipped; every forward-reference below was re-verified against disk (GD-2 executable) and the
> plan corrected inline. Verified facts (quoted from live checks):
> - **Lens columns** (`\d kb_cogmap_lenses`): `id, cogmap_id, name, selection_kind, w_express, w_contains,
>   w_leads_to, w_near, w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id, created` —
>   match the `Lens` struct in Task 2 **exactly**. Seeded `telos-default` row = `(1, 1, 0.6, 0.3, 0.4,
>   0.5, 0.3, 0.2, 0.5)` ⇒ mirrors `Lens::telos_default()` value-for-value. ✓
> - **Region readout columns** present: `centrality, content_cohesion, internal_tension, lens_id,
>   reference_standing, telos_alignment`. ✓
> - **Readout fn signatures** (verified): `cogmap_region_content_cohesion(p_region uuid)`,
>   `cogmap_region_telos_alignment(p_region uuid, p_cogmap uuid)`, `cogmap_region_reference_standing(p_region uuid)`,
>   `cogmap_region_centrality(p_region uuid)`, `cogmap_region_internal_tension(p_region uuid, p_opposed_labels text[])`.
>   **No `cogmap_region_salience` function shipped** — Task 6's inline salience blend stays (see T6 fix below). ✓
> - **temper-ingest API** (no `pub use` re-exports → call via submodules): `temper_ingest::chunk::chunk_markdown(&str) -> Vec<ChunkData>`,
>   `temper_ingest::embed::embed_texts(&[&str]) -> Result<Vec<Vec<f32>>>`, `temper_ingest::embed::EMBEDDING_DIM = 768`.
>   `embed` module is gated behind `feature = "embed"`. `ChunkData` carries a real `content_hash` (SHA-256 of content). ✓
> - **Substrate-read columns**: `kb_resource_homes(resource_id, anchor_table, anchor_id)`; `kb_properties`
>   HAS a `weight` column + `property_value jsonb`; `kb_chunks.version` defaults to `1` (so the ON CONFLICT
>   insert needs no explicit version); `edge_kind` enum = `{express, contains, leads_to, near}`. ✓
> - **Workspace shape**: root `Cargo.toml` `members = ["crates/*", "tests/e2e"]` — a **glob**, so
>   `crates/temper-next` is auto-included (NO members edit). There is **no `[workspace.dependencies]`** table;
>   crates pin versions directly. No crate declares `[lib]` — `src/lib.rs` + `src/main.rs` auto-detect as lib+bin.
>
> **Six corrections applied inline below** (search for `RE-GROUNDED FIX`): (1) Cargo.toml uses direct
> version pins + correct sqlx features, drops `pgvector` (Rust never binds a vector type); (2) no members
> edit (glob); (3) no explicit `[lib]/[[bin]]` split; (4) embed job uses `ChunkData.content_hash` + submodule
> call paths; (5) centroid placeholder uses an explicit zero-vector literal (the `SELECT … LIMIT 1` was
> NULL-fragile once Plan 3 removes the hand-seeded region); (6) salience blend wraps `coalesce(telos_alignment, 0)`
> (it is nullable ⇒ would violate `salience NOT NULL`). **One remaining dependency:** Task 5's `block_text`
> table is created by **Plan 3 T1** — the embed integration test stays red until Plan 3 lands (flagged at T5).

**Goal:** A production-quality `temper-next` crate (publish=false) that, against the `temper_next` artifact DB, (A) chunks+embeds content blocks via `temper-ingest`, and (B) computes a cogmap's emergent telos-lens regions deterministically — declared-only affinity → average-link agglomerative clustering → write region rows+members and populate the readouts — through one `materialize_cogmap` entry point.

**Architecture:** Thin binary + a clean, unit-tested clustering **core** (`affinity.rs`, `cluster.rs`) written to lift wholesale into `temper-cogmap` later (spec §6b decision b). Runtime `sqlx` against `temper_next` (separate namespace ⇒ no compile-time macros; matches `temper-api/src/services/search_service.rs` runtime `query_as`). Readouts stay in SQL (Plan 1 functions); Rust owns only embed + cluster-membership (spec §6a).

**Tech Stack:** Rust, `sqlx` (runtime, Postgres), `tokio`, `uuid`, `serde_json`, `temper-ingest` (embed). No `pgvector` crate — vectors are written as `'[…]'::vector` string literals and pooled/compared in SQL. Spec: [`…/2026-06-06-emergent-region-projection-design.md`](../specs/2026-06-06-emergent-region-projection-design.md) §1, §2a, §2b, §6.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/temper-next/Cargo.toml` | crate manifest; deps; `publish = false` |
| `crates/temper-next/src/main.rs` | binary: connect, parse cogmap arg, run embed + materialize |
| `crates/temper-next/src/substrate.rs` | DB reads: homed concept-resources, declared edges, facets, for a cogmap; + the lens row |
| `crates/temper-next/src/affinity.rs` | **pure core** — declared-only `affinity(i,j)` + `facet_overlap` |
| `crates/temper-next/src/cluster.rs` | **pure core** — deterministic average-link agglomerative clustering |
| `crates/temper-next/src/embed.rs` | Job A — chunk+embed content blocks → write `kb_chunks.embedding` |
| `crates/temper-next/src/write.rs` | Job B write — region event + rows + members, then invoke SQL readouts |
| `crates/temper-next/tests/cluster_determinism.rs` | reproducibility + known-fixture clustering (pure, no DB) |

---

## Task 1: Scaffold the crate

**Tag:** EXTEND (NEW crate). CONFORM to the workspace's crate conventions (see a sibling, e.g. `crates/temper-ingest/Cargo.toml`).

**Files:** Create `crates/temper-next/Cargo.toml`, `crates/temper-next/src/main.rs`; Modify root `Cargo.toml` (workspace members).

- [ ] **Step 1: (re-grounded) workspace shape** — root `Cargo.toml` `members = ["crates/*", "tests/e2e"]` is a glob; `crates/temper-next` is picked up automatically. Edition/conventions: edition `2021`, no per-crate `[lib]` declarations across the workspace.

- [ ] **Step 2: Create the manifest**

`crates/temper-next/Cargo.toml`:
```toml
[package]
name = "temper-next"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
temper-ingest = { path = "../temper-ingest", features = ["embed"] }
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "postgres", "uuid", "json"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
uuid = { version = "1", features = ["v7", "serde"] }
serde_json = "1"
anyhow = "1"
```
> **RE-GROUNDED FIX (1):** there is **no `[workspace.dependencies]`** table — crates pin versions
> directly (verified: `temper-api` pins `sqlx = { version = "0.8", … }`, `uuid = { version = "1", … }`,
> `tokio = { version = "1", … }`; `anyhow = "1"` is used elsewhere). Use the same TLS variant the
> workspace uses — `runtime-tokio-rustls`, NOT bare `runtime-tokio`. Add `json` to sqlx features +
> `serde_json` (the facet read deserializes `property_value jsonb` into `serde_json::Value`). **No
> `pgvector` crate** — embeddings are written as a `'[…]'::vector` string literal and pooled/compared
> in SQL; Rust never binds a vector type, so pgvector is not a dependency.

- [ ] **Step 3: Minimal `main.rs`** so the crate builds:
```rust
fn main() {
    println!("temper-next harness");
}
```

- [ ] **Step 4: (RE-GROUNDED FIX 2) NO workspace-members edit** — `members = ["crates/*", …]` already
  globs in `crates/temper-next`. Do **not** touch root `Cargo.toml`. (Sanity: `cargo metadata --no-deps
  | grep temper-next` after creating the manifest confirms the workspace sees it.)

- [ ] **Step 5: Verify build** — `cargo build -p temper-next` → Expected: compiles. Commit:
```bash
git add crates/temper-next/Cargo.toml crates/temper-next/src/main.rs
git commit -m "feat(temper-next): scaffold the region-clustering harness crate"
```

---

## Task 2: The declared-only affinity core (pure, no DB)

**Tag:** EXTEND (NEW, spec §2a). The single most load-bearing invention — keep it pure and fully unit-tested.

**Files:** Create `crates/temper-next/src/affinity.rs`; Test in the same file (`#[cfg(test)]`).

- [ ] **Step 1: Write the failing tests** (`affinity.rs` bottom):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ids() -> (Uuid, Uuid) {
        (Uuid::from_u128(1), Uuid::from_u128(2))
    }

    #[test]
    fn edge_affinity_is_lens_weighted_kind_times_weight() {
        let (a, b) = ids();
        let lens = Lens { w_leads_to: 0.6, w_prop: 0.4, ..Lens::telos_default() };
        let edges = vec![Edge { src: a, tgt: b, kind: EdgeKind::LeadsTo, weight: 0.8, label: None }];
        // 0.6 * 0.8 * label_factor(None)=1.0 = 0.48
        assert!((affinity(a, b, &edges, &[], &lens) - 0.48).abs() < 1e-9);
    }

    #[test]
    fn no_declared_edge_no_facet_means_zero_affinity() {
        let (a, b) = ids();
        assert_eq!(affinity(a, b, &[], &[], &Lens::telos_default()), 0.0);
    }

    #[test]
    fn facet_overlap_is_min_weighted_shared_pairs() {
        let (a, b) = ids();
        let facets = vec![
            Facet { owner: a, path: "topic".into(), value: "deployment".into(), weight: 0.9 },
            Facet { owner: b, path: "topic".into(), value: "deployment".into(), weight: 0.5 },
            Facet { owner: b, path: "phase".into(), value: "first-week".into(), weight: 1.0 },
        ];
        let lens = Lens { w_prop: 1.0, ..Lens::telos_default() };
        // shared ("topic","deployment"): min(0.9,0.5)=0.5; "phase" not shared. w_prop*0.5 = 0.5
        assert!((affinity(a, b, &[], &facets, &lens) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn label_factor_defaults_to_one_no_reserved_words() {
        let (a, b) = ids();
        let lens = Lens { w_near: 1.0, ..Lens::telos_default() };
        let edges = vec![Edge { src: a, tgt: b, kind: EdgeKind::Near, weight: 1.0,
                                label: Some("contradicts".into()) }];
        // contradiction BINDS: label_factor('contradicts') == 1.0 (no reserved literal)
        assert!((affinity(a, b, &edges, &[], &lens) - 1.0).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p temper-next affinity` → Expected: compile errors (types/fn undefined).

- [ ] **Step 3: Implement** (`affinity.rs` top):
```rust
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeKind { Express, Contains, LeadsTo, Near }

#[derive(Clone, Debug)]
pub struct Edge { pub src: Uuid, pub tgt: Uuid, pub kind: EdgeKind, pub weight: f64, pub label: Option<String> }

#[derive(Clone, Debug)]
pub struct Facet { pub owner: Uuid, pub path: String, pub value: String, pub weight: f64 }

#[derive(Clone, Debug)]
pub struct Lens {
    pub w_express: f64, pub w_contains: f64, pub w_leads_to: f64, pub w_near: f64,
    pub w_prop: f64, pub s_telos: f64, pub s_ref: f64, pub s_central: f64, pub resolution: f64,
}
impl Lens {
    /// Concrete starting defaults (spec §5c; tunable, OQ-2). MUST mirror the seeded telos-default row.
    pub fn telos_default() -> Self {
        Lens { w_express: 1.0, w_contains: 1.0, w_leads_to: 0.6, w_near: 0.3,
               w_prop: 0.4, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }
    }
    fn w_kind(&self, k: EdgeKind) -> f64 {
        match k { EdgeKind::Express => self.w_express, EdgeKind::Contains => self.w_contains,
                  EdgeKind::LeadsTo => self.w_leads_to, EdgeKind::Near => self.w_near }
    }
}

/// Declared `contradicts`/`reinforces` etc. are NOT reserved (spec §2a): default factor 1.0.
/// A lens MAY override specific labels explicitly later; the telos-default treats every label as
/// ordinary positive relatedness — contradiction BINDS (shared frame), never separates.
fn label_factor(_label: &Option<String>, _lens: &Lens) -> f64 { 1.0 }

/// min-weighted overlap over shared (path,value) facet pairs (spec §4b). Declared only — never cosine.
fn facet_overlap(a: Uuid, b: Uuid, facets: &[Facet]) -> f64 {
    let fa: Vec<&Facet> = facets.iter().filter(|f| f.owner == a).collect();
    let fb: Vec<&Facet> = facets.iter().filter(|f| f.owner == b).collect();
    let mut sum = 0.0;
    for x in &fa {
        for y in &fb {
            if x.path == y.path && x.value == y.value {
                sum += x.weight.min(y.weight);
            }
        }
    }
    sum
}

/// Declared-only symmetric affinity (spec §2a). Cosine is ABSENT — it enters only as a downstream
/// readout (Plan 1 SQL), never here.
pub fn affinity(a: Uuid, b: Uuid, edges: &[Edge], facets: &[Facet], lens: &Lens) -> f64 {
    let edge_sum: f64 = edges.iter()
        .filter(|e| (e.src == a && e.tgt == b) || (e.src == b && e.tgt == a))
        .filter(|e| !e.weight.is_nan())
        .map(|e| lens.w_kind(e.kind) * e.weight * label_factor(&e.label, lens))
        .sum();
    edge_sum + lens.w_prop * facet_overlap(a, b, facets)
}
```

- [ ] **Step 4: Run to verify it passes** — `cargo test -p temper-next affinity` → Expected: 4 passed.
- [ ] **Step 5: Commit** — `git commit -am "feat(temper-next): declared-only affinity core (cosine absent; contradiction binds)"`

---

## Task 3: Deterministic average-link agglomerative clustering (pure, no DB)

**Tag:** EXTEND (NEW, spec §2b). The determinism contract lives here — order-stable, no random init, reproducible.

**Files:** Create `crates/temper-next/src/cluster.rs`; reproducibility test in `tests/cluster_determinism.rs`.

- [ ] **Step 1: Write the failing tests**

`tests/cluster_determinism.rs`:
```rust
use temper_next::cluster::cluster;
use temper_next::affinity::{affinity, Edge, EdgeKind, Facet, Lens};
use uuid::Uuid;

fn id(n: u128) -> Uuid { Uuid::from_u128(n) }

/// Three nodes: a—b strongly edged (above resolution), c isolated. Expect {a,b} and {c}.
fn fixture() -> (Vec<Uuid>, Vec<Edge>, Vec<Facet>, Lens) {
    let (a, b, c) = (id(1), id(2), id(3));
    let lens = Lens { w_leads_to: 1.0, resolution: 0.5, ..Lens::telos_default() };
    let edges = vec![Edge { src: a, tgt: b, kind: EdgeKind::LeadsTo, weight: 0.9, label: None }];
    (vec![a, b, c], edges, vec![], lens)
}

#[test]
fn isolated_node_forms_its_own_cluster() {
    let (nodes, edges, facets, lens) = fixture();
    let aff = |x: Uuid, y: Uuid| affinity(x, y, &edges, &facets, &lens);
    let clusters = cluster(&nodes, &aff, lens.resolution);
    assert_eq!(clusters.len(), 2);
    assert!(clusters.iter().any(|c| c == &vec![id(1), id(2)]));
    assert!(clusters.iter().any(|c| c == &vec![id(3)]));
}

#[test]
fn reproducible_byte_identical_on_rerun() {
    let (nodes, edges, facets, lens) = fixture();
    let aff = |x: Uuid, y: Uuid| affinity(x, y, &edges, &facets, &lens);
    let one = cluster(&nodes, &aff, lens.resolution);
    let two = cluster(&nodes, &aff, lens.resolution);
    assert_eq!(one, two);
}
```
> **RE-GROUNDED FIX (3):** add `src/lib.rs` with `pub mod affinity; pub mod cluster;` (plus `pub mod
> substrate; pub mod embed; pub mod write;` as those land). **No `[lib]`/`[[bin]]` split needed in
> Cargo.toml** — no crate in the workspace declares `[lib]`; Cargo auto-detects `src/lib.rs` as the lib
> and `src/main.rs` as the `temper-next` bin. `main.rs` references the lib by crate name
> (`use temper_next::write::materialize_cogmap;`). Integration tests under `tests/` see only the lib's
> `pub` surface, which is why these modules must be `pub`.

- [ ] **Step 2: Run to verify it fails** — `cargo test -p temper-next --test cluster_determinism` → Expected: unresolved `cluster`.

- [ ] **Step 3: Implement** (`cluster.rs`):
```rust
use uuid::Uuid;

const EPS: f64 = 1e-12;

/// Deterministic average-link agglomerative clustering (spec §2b).
/// - nodes are processed in ascending-UUID order (stable);
/// - merges the two clusters of highest average-link affinity until the best falls below `resolution`;
/// - ties (within EPS) broken by the lexicographically-smaller merged UUID set (stable);
/// - a node with no above-resolution link remains its own cluster (separation = absence, spec §2a).
/// No random initialization. Same inputs -> identical output.
pub fn cluster<F: Fn(Uuid, Uuid) -> f64>(nodes: &[Uuid], aff: &F, resolution: f64) -> Vec<Vec<Uuid>> {
    let mut sorted = nodes.to_vec();
    sorted.sort();
    let mut clusters: Vec<Vec<Uuid>> = sorted.into_iter().map(|n| vec![n]).collect();

    loop {
        let mut best: Option<(usize, usize, f64)> = None;
        for i in 0..clusters.len() {
            for j in (i + 1)..clusters.len() {
                let a = avg_link(&clusters[i], &clusters[j], aff);
                best = match best {
                    None => Some((i, j, a)),
                    Some((bi, bj, b)) => {
                        if a > b + EPS {
                            Some((i, j, a))
                        } else if (a - b).abs() <= EPS
                            && tie_key(&clusters[i], &clusters[j]) < tie_key(&clusters[bi], &clusters[bj])
                        {
                            Some((i, j, a))
                        } else {
                            Some((bi, bj, b))
                        }
                    }
                };
            }
        }
        match best {
            Some((i, j, a)) if a >= resolution => {
                let mut merged = clusters[i].clone();
                merged.extend(clusters[j].clone());
                merged.sort();
                clusters.remove(j); // j > i, remove the later index first
                clusters[i] = merged;
            }
            _ => break,
        }
    }
    clusters.sort_by(|x, y| x[0].cmp(&y[0]));
    clusters
}

fn avg_link<F: Fn(Uuid, Uuid) -> f64>(a: &[Uuid], b: &[Uuid], aff: &F) -> f64 {
    let mut sum = 0.0;
    for &x in a {
        for &y in b {
            sum += aff(x, y);
        }
    }
    sum / (a.len() * b.len()) as f64
}

fn tie_key(a: &[Uuid], b: &[Uuid]) -> Uuid {
    let mut all: Vec<Uuid> = a.iter().chain(b.iter()).copied().collect();
    all.sort();
    all[0]
}
```

- [ ] **Step 4: Run to verify it passes** — `cargo test -p temper-next` → Expected: all pass (affinity + cluster determinism).
- [ ] **Step 5: Commit** — `git commit -am "feat(temper-next): deterministic average-link agglomerative clustering core"`

---

## Task 4: Substrate read (DB → typed structs)

**Tag:** EXTEND (NEW). CONFORM to the runtime-`sqlx` pattern (verified `temper-api/src/services/search_service.rs:82` `sqlx::query_as::<_, Row>`). Reads against verified columns: `kb_edges` (`source_id/target_id/edge_kind/label/weight/home_anchor_*/is_folded`), `kb_properties` (`owner_table/owner_id/property_key/property_value/weight`), homing via `kb_resource_homes`.

**Files:** Create `crates/temper-next/src/substrate.rs`.

- [ ] **Step 1: Write the integration test** (gated; needs the loaded artifact):
```rust
// tests/substrate_read.rs  — requires the temper_next artifact loaded (Plan 1 + seed).
// RE-GROUND: confirm the seeded cogmap name and member count after Plan 3's enriched seed;
// until then this asserts against the CURRENT (sparse) seed.
#[tokio::test]
async fn loads_homed_concepts_and_edges_for_a_cogmap() {
    let pool = temper_next::substrate::connect().await.unwrap();
    let cogmap = temper_next::substrate::cogmap_by_name(&pool, "onboarding-cogmap").await.unwrap();
    let s = temper_next::substrate::load(&pool, cogmap).await.unwrap();
    assert!(!s.nodes.is_empty(), "expected ≥1 homed concept-resource");
    // edges/facets may be empty in the sparse seed; structure must load without error.
}
```
> RE-GROUNDED (GD-1): `kb_resource_homes` columns confirmed `(id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id, created)` — the load query's `WHERE anchor_table='kb_cogmaps' AND anchor_id=$1` is valid as written.

- [ ] **Step 2: Run to verify it fails** — `cargo test -p temper-next --test substrate_read` → Expected: unresolved `substrate::*`.

- [ ] **Step 3: Implement** (`substrate.rs`) — runtime queries, `search_path` set per connection:
```rust
use anyhow::Result;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;
use crate::affinity::{Edge, EdgeKind, Facet, Lens};

pub struct Substrate { pub nodes: Vec<Uuid>, pub edges: Vec<Edge>, pub facets: Vec<Facet>, pub lens: Lens, pub lens_id: Uuid }

pub async fn connect() -> Result<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://temper:temper@localhost:5437/temper_development".into());
    let pool = PgPoolOptions::new().after_connect(|c, _| Box::pin(async move {
        sqlx::query("SET search_path = temper_next, public").execute(c).await.map(|_| ())
    })).connect(&url).await?;
    Ok(pool)
}

pub async fn cogmap_by_name(pool: &PgPool, name: &str) -> Result<Uuid> {
    let row = sqlx::query("SELECT id FROM kb_cogmaps WHERE name = $1").bind(name).fetch_one(pool).await?;
    Ok(row.get::<Uuid, _>("id"))
}

pub async fn load(pool: &PgPool, cogmap: Uuid) -> Result<Substrate> {
    // concept-resources homed in the cogmap
    let node_rows = sqlx::query(
        "SELECT resource_id FROM kb_resource_homes WHERE anchor_table='kb_cogmaps' AND anchor_id=$1")
        .bind(cogmap).fetch_all(pool).await?;
    let nodes: Vec<Uuid> = node_rows.iter().map(|r| r.get::<Uuid, _>("resource_id")).collect();

    // declared edges homed in the cogmap, both endpoints resources
    let edge_rows = sqlx::query(
        "SELECT source_id, target_id, edge_kind::text AS kind, label, weight \
         FROM kb_edges WHERE home_anchor_table='kb_cogmaps' AND home_anchor_id=$1 \
           AND source_table='kb_resources' AND target_table='kb_resources' AND NOT is_folded")
        .bind(cogmap).fetch_all(pool).await?;
    let edges = edge_rows.iter().map(|r| Edge {
        src: r.get("source_id"), tgt: r.get("target_id"),
        kind: parse_kind(r.get::<String, _>("kind")),
        weight: r.get("weight"), label: r.get("label"),
    }).collect();

    // facets on those resources (property_key='facet', value jsonb {path:value})
    let facet_rows = sqlx::query(
        "SELECT owner_id, property_value, weight FROM kb_properties \
         WHERE owner_table='kb_resources' AND property_key='facet' AND NOT is_folded \
           AND owner_id = ANY($1)")
        .bind(&nodes).fetch_all(pool).await?;
    let facets = facet_rows.iter().filter_map(|r| {
        let v: serde_json::Value = r.get("property_value");
        let (path, value) = v.as_object()?.iter().next()?;
        Some(Facet { owner: r.get("owner_id"), path: path.clone(),
                     value: value.as_str()?.to_string(), weight: r.get("weight") })
    }).collect();

    // the telos-default lens for this cogmap (or the global default)
    let lr = sqlx::query(
        "SELECT id, w_express, w_contains, w_leads_to, w_near, w_prop, s_telos, s_ref, s_central, resolution \
         FROM kb_cogmap_lenses WHERE name='telos-default' AND (cogmap_id=$1 OR cogmap_id IS NULL) \
         ORDER BY cogmap_id NULLS LAST LIMIT 1")
        .bind(cogmap).fetch_one(pool).await?;
    let lens = Lens {
        w_express: lr.get("w_express"), w_contains: lr.get("w_contains"),
        w_leads_to: lr.get("w_leads_to"), w_near: lr.get("w_near"), w_prop: lr.get("w_prop"),
        s_telos: lr.get("s_telos"), s_ref: lr.get("s_ref"), s_central: lr.get("s_central"),
        resolution: lr.get("resolution"),
    };
    Ok(Substrate { nodes, edges, facets, lens, lens_id: lr.get("id") })
}

fn parse_kind(s: String) -> EdgeKind {
    match s.as_str() {
        "express" => EdgeKind::Express, "contains" => EdgeKind::Contains,
        "leads_to" => EdgeKind::LeadsTo, _ => EdgeKind::Near,
    }
}
```
> RE-GROUNDED: `serde_json` is in the manifest (fix 1) and sqlx has the `json` feature; `kb_properties`
> has both `property_value jsonb` AND a `weight double precision` column (verified `\d kb_properties`),
> so the facet read's `SELECT owner_id, property_value, weight` is valid as written.

- [ ] **Step 4: Run to verify it passes** — `cargo test -p temper-next --test substrate_read` (artifact loaded) → Expected: pass.
- [ ] **Step 5: Commit** — `git commit -am "feat(temper-next): substrate read (homed concepts, declared edges, facets, lens)"`

---

## Task 5: Embed job (Job A)

**Tag:** EXTEND (NEW). Reuse `temper-ingest` (verified `chunk_markdown`, `embed_texts`, `EMBEDDING_DIM=768`). CONFORM: write into `kb_chunks.embedding` (verified `vector(768)` column, currently NULL in seed).

**Files:** Create `crates/temper-next/src/embed.rs`.

- [ ] **Step 1: Write the integration test:**
```rust
// tests/embed_job.rs — requires artifact loaded. After embedding, seeded resources have current chunks with embeddings.
#[tokio::test]
async fn embeds_content_blocks_into_chunks() {
    let pool = temper_next::substrate::connect().await.unwrap();
    temper_next::embed::embed_all_blocks(&pool).await.unwrap();
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_chunks WHERE embedding IS NOT NULL AND is_current")
        .fetch_one(&pool).await.unwrap();
    assert!(row > 0, "expected embedded chunks after the embed job");
}
```
> RE-GROUNDED: the current seed authors **no** block content and the `block_text` table does not exist
> until Plan 3 T1, so this test cannot pass inside Plan 2. Land it `#[ignore]`d (or red-with-comment)
> and un-gate it in Plan 3 once `block_text` + content are seeded. Do not seed a stand-in here.

- [ ] **Step 2: Run to verify it fails** — `cargo test -p temper-next --test embed_job` → Expected: unresolved `embed::embed_all_blocks`.

- [ ] **Step 3: Implement** (`embed.rs`):
```rust
use anyhow::Result;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Job A (spec §6a): chunk + embed every non-folded block's content, write kb_chunks rows with
/// 768-dim embeddings. Block content source: RE-GROUND — Plan 3 authors block text; this reads it
/// from wherever the seed stores it (a `content` column on a block-text table, or inline). The shape
/// below assumes a `block_text(block_id, body)` source seeded by Plan 3; reconcile before running.
pub async fn embed_all_blocks(pool: &PgPool) -> Result<()> {
    let blocks = sqlx::query(
        "SELECT b.id AS block_id, b.resource_id, bt.body \
         FROM kb_content_blocks b JOIN block_text bt ON bt.block_id = b.id \
         WHERE NOT b.is_folded")
        .fetch_all(pool).await?;
    for row in blocks {
        let block_id: Uuid = row.get("block_id");
        let resource_id: Uuid = row.get("resource_id");
        let body: String = row.get("body");
        // RE-GROUNDED FIX (4): submodule call paths (no re-exports in temper-ingest lib.rs).
        let chunks = temper_ingest::chunk::chunk_markdown(&body);
        let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        if texts.is_empty() { continue; }
        let embeddings = temper_ingest::embed::embed_texts(&texts)?; // 768-dim, l2-normalized
        for (i, emb) in embeddings.iter().enumerate() {
            let vec_lit = format!("[{}]", emb.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(","));
            sqlx::query(
                "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding, is_current) \
                 VALUES ($1,$2,$3,$4,$5::vector,true) \
                 ON CONFLICT (block_id, chunk_index, version) DO UPDATE SET embedding = EXCLUDED.embedding")
                .bind(block_id).bind(resource_id).bind(i as i32)
                // RE-GROUNDED FIX (4): ChunkData carries a real SHA-256 content_hash; use it (1:1 with embeddings).
                .bind(chunks[i].content_hash.clone())
                .bind(vec_lit).execute(pool).await?;
        }
    }
    Ok(())
}
```
> **GD-5 flag (re-grounded):** `content_hash` is now resolved — use `ChunkData.content_hash` (fix 4 above).
> The `block_text(block_id, body)` source table is **created by Plan 3 T1**, not Plan 1, so it does NOT
> exist when Plan 2 lands. Consequence: write `embed.rs` against this shape, but the `tests/embed_job.rs`
> integration test **stays red until Plan 3 seeds `block_text` + content** — do not try to make it green
> inside Plan 2 (and do not invent a stand-in table; that's Plan 3's decision). Either gate the test
> `#[ignore]` with a note, or land it red-with-explanation; either way, flag it, don't fabricate content.

- [ ] **Step 4/5: Verify + commit** — `cargo test -p temper-next --test embed_job` (after Plan 3 content) → pass; `git commit -am "feat(temper-next): embed job — chunk+embed blocks into kb_chunks (bge-768)"`

---

## Task 6: `materialize_cogmap` — write + readouts (Job B)

**Tag:** EXTEND (NEW). CONFORM to the event-write pattern (`INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_*)`, verified `03_seed.sql:220`) and the fold-old/assert-new batch (spec §6/map-regions §6). Populates the Plan-1 readout columns via the Plan-1 SQL functions.

**Files:** Create `crates/temper-next/src/write.rs`; wire `main.rs`.

- [ ] **Step 1: Write the integration test:**
```rust
// tests/materialize.rs — requires artifact + Plan 3 enriched seed + embeddings.
#[tokio::test]
async fn materialize_is_reproducible_and_populates_readouts() {
    let pool = temper_next::substrate::connect().await.unwrap();
    temper_next::embed::embed_all_blocks(&pool).await.unwrap();
    let cogmap = temper_next::substrate::cogmap_by_name(&pool, "onboarding-cogmap").await.unwrap();
    let first = temper_next::write::materialize_cogmap(&pool, cogmap).await.unwrap();
    let second = temper_next::write::materialize_cogmap(&pool, cogmap).await.unwrap();
    assert_eq!(first.membership_fingerprint, second.membership_fingerprint, "reproducible membership");
    assert!(first.regions >= 2, "expected ≥2 emergent regions on the enriched seed");
    // readouts populated, not null:
    let nulls = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_cogmap_regions WHERE content_cohesion IS NULL AND NOT is_folded")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(nulls, 0, "all live regions have a computed content_cohesion");
}
```

- [ ] **Step 2: Run to verify it fails** — Expected: unresolved `write::materialize_cogmap`.

- [ ] **Step 3: Implement** (`write.rs`):
```rust
use anyhow::Result;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use crate::{affinity::affinity, cluster::cluster, substrate};

pub struct MaterializeOutcome { pub regions: usize, pub membership_fingerprint: String }

/// Job B (spec §6a): read substrate -> declared-only affinity -> deterministic clustering ->
/// fold prior regions + assert new ones + members under ONE materialization event -> populate the
/// SQL readouts (Plan 1 functions). Cosine never enters formation; it enters only via the readouts.
pub async fn materialize_cogmap(pool: &PgPool, cogmap: Uuid) -> Result<MaterializeOutcome> {
    let s = substrate::load(pool, cogmap).await?;
    let aff = |x: Uuid, y: Uuid| affinity(x, y, &s.edges, &s.facets, &s.lens);
    let clusters = cluster(&s.nodes, &aff, s.lens.resolution);

    let mut tx = pool.begin().await?;
    // one materialization event (correlation root)
    let ev: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
         SELECT (SELECT id FROM kb_event_types WHERE name='region_materialized'), \
                (SELECT emitter_entity_id FROM kb_events ORDER BY occurred_at DESC LIMIT 1), \
                'kb_cogmaps', $1 RETURNING id")
        .bind(cogmap).fetch_one(&mut *tx).await?;
    // fold prior live regions for this lens
    sqlx::query("UPDATE kb_cogmap_regions SET is_folded=true, last_event_id=$1 \
                 WHERE cogmap_id=$2 AND lens_id=$3 AND NOT is_folded")
        .bind(ev).bind(cogmap).bind(s.lens_id).execute(&mut *tx).await?;

    // RE-GROUNDED FIX (5): a zero-vector(768) literal placeholder for the NOT-NULL centroid (overwritten
    // by the UPDATE below before any readout reads it). The original `SELECT centroid … LIMIT 1` was
    // NULL-fragile: Plan 3 removes the hand-seeded region, so on a clean run no region exists yet ⇒
    // the subquery returns NULL ⇒ NOT NULL violation. The zero literal is unconditional.
    let zero_centroid = format!("[{}]", vec!["0"; temper_ingest::embed::EMBEDDING_DIM].join(","));
    let mut fingerprint_parts: Vec<String> = Vec::new();
    for members in &clusters {
        // centroid computed in SQL after members are inserted; insert a placeholder then UPDATE via readouts.
        let region: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmap_regions \
               (cogmap_id, lens_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id) \
             VALUES ($1,$2, $5::vector, 0.0, NULL, $3, $4, $4) RETURNING id")
            .bind(cogmap).bind(s.lens_id).bind(members.len() as i32).bind(ev)
            .bind(&zero_centroid)
            .fetch_one(&mut *tx).await?;
        for m in members {
            sqlx::query("INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id) \
                         VALUES ($1,'kb_resources',$2)")
                .bind(region).bind(m).execute(&mut *tx).await?;
        }
        // populate centroid + readouts via the Plan-1 SQL functions + a centroid recompute
        sqlx::query(
            "UPDATE kb_cogmap_regions r SET \
               centroid = (SELECT avg(ch.embedding) FROM kb_cogmap_region_members mm \
                           JOIN kb_chunks ch ON ch.resource_id=mm.member_id AND ch.is_current \
                           WHERE mm.region_id=r.id), \
               content_cohesion   = cogmap_region_content_cohesion(r.id), \
               telos_alignment    = cogmap_region_telos_alignment(r.id, r.cogmap_id), \
               reference_standing = cogmap_region_reference_standing(r.id), \
               centrality         = cogmap_region_centrality(r.id), \
               internal_tension   = cogmap_region_internal_tension(r.id, ARRAY['contradicts']) \
             WHERE r.id=$1")
            .bind(region).execute(&mut *tx).await?;
        // salience = lens-weighted blend of the three parts.
        // RE-GROUNDED FIX (6): telos_alignment is NULLABLE (NULL when the telos has no embedded chunks),
        // and salience is NOT NULL — so `$2*NULL = NULL` would violate the constraint. coalesce to 0.
        // (reference_standing/centrality are coalesce'd to 0 inside their SQL functions, so only
        // telos_alignment needs guarding here. No cogmap_region_salience fn shipped in Plan 1 ⇒ inline.)
        sqlx::query(
            "UPDATE kb_cogmap_regions SET salience = \
               $2*coalesce(telos_alignment,0) + $3*reference_standing + $4*centrality WHERE id=$1")
            .bind(region).bind(s.lens.s_telos).bind(s.lens.s_ref).bind(s.lens.s_central)
            .execute(&mut *tx).await?;
        let mut ms: Vec<String> = members.iter().map(|m| m.to_string()).collect();
        ms.sort();
        fingerprint_parts.push(ms.join("+"));
    }
    sqlx::query("UPDATE kb_cogmaps SET shape_materialized_event_id=$1 WHERE id=$2")
        .bind(ev).bind(cogmap).execute(&mut *tx).await?;
    tx.commit().await?;

    fingerprint_parts.sort();
    Ok(MaterializeOutcome { regions: clusters.len(), membership_fingerprint: fingerprint_parts.join("|") })
}
```
> **GD-1/GD-5 (re-grounded):** the placeholder-centroid-then-UPDATE dance avoids the `centroid NOT NULL`
> constraint at insert — now using an unconditional zero-vector literal (fix 5), not a fragile `SELECT …
> LIMIT 1`. A cleaner path (compute the centroid in Rust from the fetched member embeddings and insert it
> directly) is a legitimate future simplification, but the placeholder+UPDATE keeps all vector math in SQL,
> which matches the "readouts stay in SQL" architecture — keep it for now. Plan 1 shipped **no**
> `cogmap_region_salience` function (verified), so the inline blend stays; it is the one place region math
> lives in Rust rather than SQL — acceptable because it's a pure lens-weighted sum of already-computed parts.

- [ ] **Step 4: Wire `main.rs`** — connect, `embed_all_blocks`, `materialize_cogmap` for a cogmap named via `args`. (Code: ~15 lines, straightforward.)
- [ ] **Step 5: Verify + commit** — `cargo test -p temper-next` (artifact + Plan 3 seed) → pass; `git commit -am "feat(temper-next): materialize_cogmap — declared clustering write + SQL readouts"`

---

## Self-Review

**1. Spec coverage:** §1 entry point → T6 ✓ · §2a declared affinity → T2 ✓ · §2b deterministic clustering → T3 ✓ · §6a embed (reuse) + cluster-membership-in-Rust + readouts-in-SQL → T4/T5/T6 ✓ · §6b production-quality liftable core (pure `affinity`/`cluster` modules) ✓.
**2. Placeholder scan:** real code throughout. The one remaining *external dependency* — Task 5's
`block_text` source — is owned by Plan 3 T1 (flagged at T5), not a silent placeholder. `content_hash`
and the centroid-insert are now resolved (fixes 4 & 5), not assumptions.
**3. Type consistency:** `Lens`/`Edge`/`Facet`/`EdgeKind` consistent T2↔T3↔T4; `materialize_cogmap`/`embed_all_blocks`/`load`/`connect` consistent T4↔T5↔T6.
**4. Grounding:** RE-GROUNDED 2026-06-06 against live Plan-1 disk — lens/region columns, all five readout
fn signatures, temper-ingest submodule API, substrate-read columns, and workspace shape all verified with
quoted values in the banner; six corrections (fixes 1–6) applied inline. Only forward-ref remaining: Plan 3's
`block_text` (T5).

---

**Plan 2 is RE-GROUNDED and READY TO EXECUTE** (verified against live Plan-1 disk 2026-06-06; six fixes
applied inline — banner). Execution: this is a 6-task TDD Rust crate with a clean pure-core (`affinity`/
`cluster`) — a good Variant-B candidate (subagent-driven, orchestrator reviews each commit), or Variant A
inline given the plan is now concrete. The one known red test is `tests/embed_job.rs` (waits on Plan 3's
`block_text`); everything else (scaffold, affinity, cluster determinism, substrate read) is green-able
against the current artifact.
