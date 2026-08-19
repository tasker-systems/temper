# Access-scaffold scenario proof Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `temper_next` access model's leak-safety invariants (S1–S5, S8 in `schema-artifact/04_scenarios.sql`, currently a manual psql read) declarative and CI-checkable, via a new self-contained access-scenario YAML kind.

**Architecture:** A new `AccessScenario` document kind (`crates/temper-next/src/scenario/access/`) carries a full access **world** (profiles, entities, teams + DAG, multiple cogmaps + team-joins, resources + homes + grants, homed edges) plus inline **checks**. A loader persists the world atomically (direct inserts for topology rows, `cogmap_genesis` only for the charter-bearing onboarding map, `relationship_assert` for the homed edge); an evaluator asserts each check against one kernel gate function (`resources_visible_to`, `resources_accessible_to_cogmap`, `edges_visible_to`, `cogmaps_share_a_team`, `resource_blocks`). No DDL change — the model is already schema'd.

**Tech Stack:** Rust, sqlx (compile-time `query!` macros against the `temper_next` namespace — cached per-crate, regenerated with `cargo make prepare-next`), serde_yaml, schemars (gated snapshot), cargo-nextest (the `temper-next-write` group), Postgres 18 + pgvector.

**Spec:** `docs/superpowers/specs/2026-06-11-access-scaffold-scenario-proof-design.md`

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/temper-next/src/scenario/access/mod.rs` | Module decls + re-exports | 1 |
| `crates/temper-next/src/scenario/access/model.rs` | YAML structs + `AccessCheck` enum + parse unit tests | 1 |
| `crates/temper-next/src/scenario/mod.rs` | Add `pub mod access;` | 1 |
| `schema-artifact/access-scenarios/epd-bridge-access.yaml` | The S1–S5 fixture (ports `03_seed.sql`) | 2 |
| `crates/temper-next/src/scenario/access/loader.rs` | `load()` — persist the world; return `key → Uuid` maps | 3 |
| `crates/temper-next/tests/access_scenario.rs` | Integration tests (load counts, run all checks) + S8 | 3,4,5 |
| `.config/nextest.toml` | Add `access_scenario` to the `temper-next-write` group | 3 |
| `crates/temper-next/src/scenario/access/runner.rs` | `eval_access_check` + `run_access_scenario` | 4 |
| `schema-artifact/access-scenarios/access-scenario.schema.json` | JSON-Schema snapshot | 6 |
| `crates/temper-next/tests/scenario_schema.rs` | Add `AccessScenario` snapshot test | 6 |

**Conventions you must follow (from CLAUDE.md):**
- temper-next `sqlx::query!` macros resolve against the **`temper_next` namespace**. After adding/changing any macro query in `src/scenario/access/`, run **`cargo make prepare-next`** to regenerate `crates/temper-next/.sqlx`, and commit the cache. Never `cargo sqlx prepare --workspace` (clobbers per-crate caches).
- All `cargo make` tasks force `SQLX_OFFLINE=true`. To compile + run the artifact tests, use the offline cache: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests <filter>`. The tests connect to the live dev DB at runtime (`postgresql://temper:temper@localhost:5437/temper_development`); ensure it's up with `cargo make docker-up`.
- Run `cargo make fix` then `cargo make check` before any commit (the pre-commit hook is a backstop, not the first line).
- Typed structs over inline JSON; no stringly-typed matches over bounded sets (the `anchor`-tagged enums below).

---

### Task 1: Access-scenario model + parse test

**Files:**
- Create: `crates/temper-next/src/scenario/access/mod.rs`
- Create: `crates/temper-next/src/scenario/access/model.rs`
- Modify: `crates/temper-next/src/scenario/mod.rs` (add `pub mod access;`)

- [ ] **Step 1: Write the module tree**

Create `crates/temper-next/src/scenario/access/mod.rs`:

```rust
//! Self-contained access-scenario kind: a world (profiles / entities / teams + DAG / cogmaps +
//! team-joins / resources + homes + grants / homed edges) plus inline access **checks** that assert
//! the kernel gate functions. Separate from the charter seed/scenario kinds — access proofs are
//! static (seed the topology, assert), with no materialize / lens / telos machinery. Ports
//! `schema-artifact/03_seed.sql` (the topology) + `04_scenarios.sql` (the invariants) into the
//! declarative harness.
pub mod loader;
pub mod model;
pub mod runner;

pub use loader::{load, LoadedAccess};
pub use runner::run_access_scenario;
```

Add `pub mod access;` to `crates/temper-next/src/scenario/mod.rs` (after `pub mod bootseed;`, keeping the list alphabetical):

```rust
pub mod access;
pub mod bootseed;
pub mod loader;
pub mod model;
pub mod runner;
```

(`loader` and `runner` are created in Tasks 3 and 4. To compile Task 1 alone, temporarily comment out the `pub mod loader;` / `pub mod runner;` lines and the `pub use` re-exports in `access/mod.rs`, then restore them in Task 3/4. Alternatively create empty `loader.rs`/`runner.rs` stubs now — your choice; the parse test in this task does not need them.)

- [ ] **Step 2: Write the model + the failing parse test**

Create `crates/temper-next/src/scenario/access/model.rs`:

```rust
//! Declarative YAML model for the access-scenario kind. Reuses `ProfileDef`, `EntityDef`, `TelosDef`,
//! and `EdgeKind` from the charter scenario model; adds the access topology (teams + DAG, multi-cogmap,
//! homes, grants) and the `AccessCheck` set. All enums are **internally tagged** (an `anchor`/`check`
//! discriminator field) because serde_yaml 0.9 rejects the externally-tagged single-key-map form.

use crate::affinity::EdgeKind;
use crate::scenario::model::{EntityDef, ProfileDef, TelosDef};
use serde::Deserialize;

fn one() -> f64 {
    1.0
}

/// The access-scenario document (`schema-artifact/access-scenarios/*.yaml`): a full access world plus
/// the inline checks that assert it.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessScenario {
    pub name: String,
    pub world: AccessWorld,
    pub checks: Vec<AccessCheck>,
}

/// The access topology. `profiles`/`entities` reuse the charter model's defs.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessWorld {
    pub profiles: Vec<ProfileDef>,
    pub entities: Vec<EntityDef>,
    pub teams: Vec<TeamDef>,
    #[serde(default)]
    pub memberships: Vec<MembershipDef>,
    pub cogmaps: Vec<AccessCogmapDef>,
    pub resources: Vec<AccessResourceDef>,
    #[serde(default)]
    pub edges: Vec<AccessEdgeDef>,
}

/// A team. `parents` are slugs in this same `teams` list (the down-only DAG, `kb_teams_parents`).
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct TeamDef {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub parents: Vec<String>,
}

/// A sub-team membership. Root (`temper-system`) joins are maintained by the `sync_system_membership`
/// trigger from `system_access`, so they are NOT listed here.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct MembershipDef {
    pub team: String,    // slug
    pub profile: String, // handle
    pub role: String,    // team_role: owner | maintainer | member | watcher
}

/// A cogmap. Bare producer maps carry only `name` + `teams`. A `telos` (charter) makes it a genesis
/// map (needs `owner` + `emitter`); the loader runs `cogmap_genesis` for it.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessCogmapDef {
    pub name: String,
    #[serde(default)]
    pub teams: Vec<String>, // slugs joined via kb_team_cogmaps
    #[serde(default)]
    pub owner: Option<String>, // handle — required only when `telos` is present
    #[serde(default)]
    pub emitter: Option<String>, // entity name — required only when `telos` is present
    #[serde(default)]
    pub telos: Option<TelosDef>,
}

/// A resource: identity + a single home (context or cogmap) + explicit capability grants.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessResourceDef {
    pub key: String,
    pub title: String,
    pub origin_uri: String,
    pub home: HomeDef,
    pub owner: String, // handle — originator + owner on the home row, granter on the grants
    #[serde(default)]
    pub grants: Vec<GrantDef>,
}

/// The resource home anchor. `{ anchor: cogmap, name: <cogmap> }` or `{ anchor: context }` (a synthetic
/// context anchor — the artifact has no `kb_contexts` table; the anchor is a generated uuid with no FK).
#[derive(Debug, Deserialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum HomeDef {
    Cogmap { name: String },
    Context {},
}

/// A capability grant (`kb_resource_access`). `to` is a team or profile anchor. Caps default false; the
/// DB CHECK enforces `write|delete|grant ⇒ read`.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct GrantDef {
    pub to: GrantAnchor,
    #[serde(default)]
    pub can_read: bool,
    #[serde(default)]
    pub can_write: bool,
    #[serde(default)]
    pub can_delete: bool,
    #[serde(default)]
    pub can_grant: bool,
}

/// A grant anchor — `{ anchor: team, slug: <slug> }` or `{ anchor: profile, handle: <handle> }`.
#[derive(Debug, Deserialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum GrantAnchor {
    Team { slug: String },
    Profile { handle: String },
}

/// An authored edge homed in a named cogmap, fired through `relationship_assert` (the event-backed path
/// — `kb_edges` carries NOT-NULL event FKs). `from`/`to` are resource keys.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AccessEdgeDef {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "one")]
    pub weight: f64,
    pub home: String,    // cogmap name
    pub emitter: String, // entity name
}

/// One access assertion. Internally tagged by `check:` (same serde_yaml constraint as the charter
/// `Expectation`). Each variant resolves its named referents and calls one gate function.
#[derive(Debug, Deserialize)]
#[serde(tag = "check", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum AccessCheck {
    /// S1 — consumer reach: `resources_visible_to(profile)` ∋ resource.
    VisibleTo {
        profile: String,
        resource: String,
        expect: bool,
    },
    /// S2 — producer intersection / leak-safety: `resources_accessible_to_cogmap(cogmap)` ∋ resource.
    ProducerReach {
        cogmap: String,
        resource: String,
        expect: bool,
    },
    /// S3 — edge-home protection: `edges_visible_to(profile)` ∋ edge (resolved by label).
    EdgeVisibleTo {
        profile: String,
        edge: String,
        expect: bool,
    },
    /// S5 — delegation priming: `cogmaps_share_a_team(a, b)`.
    CogmapsShareTeam {
        a: String,
        b: String,
        expect: bool,
    },
    /// S4 — charter-block gating: `count(resource_blocks(cogmap_telos(cogmap), 'profile', profile, NULL))`.
    CharterBlocksVisible {
        cogmap: String,
        profile: String,
        expect_count: i64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
name: t
world:
  profiles:
    - { handle: alice, display_name: Alice, system_access: approved }
    - { handle: nomad, display_name: Nomad, system_access: none }
  entities:
    - { name: carol-agent, profile: alice }
  teams:
    - { slug: temper-system, name: Temper System }
    - { slug: epd-team-a, name: Team A, parents: [temper-system] }
  memberships:
    - { team: epd-team-a, profile: alice, role: member }
  cogmaps:
    - { name: side-map, teams: [epd-team-a] }
    - name: onb
      teams: [epd-team-a]
      owner: alice
      emitter: carol-agent
      telos: { title: T, statement: S, questions: [{ question: q }] }
  resources:
    - { key: c, title: "concept: c", origin_uri: "temper://c",
        home: { anchor: cogmap, name: side-map }, owner: alice,
        grants: [{ to: { anchor: team, slug: temper-system }, can_read: true }] }
    - { key: d, title: "doc: d", origin_uri: "temper://d",
        home: { anchor: context }, owner: alice,
        grants: [{ to: { anchor: profile, handle: alice }, can_read: true }] }
  edges:
    - { from: c, to: d, kind: leads_to, label: "c->d", home: side-map, emitter: carol-agent }
checks:
  - { check: visible_to, profile: alice, resource: c, expect: true }
  - { check: producer_reach, cogmap: side-map, resource: c, expect: true }
  - { check: edge_visible_to, profile: alice, edge: "c->d", expect: true }
  - { check: cogmaps_share_team, a: side-map, b: onb, expect: true }
  - { check: charter_blocks_visible, cogmap: onb, profile: nomad, expect_count: 0 }
"#;

    #[test]
    fn parses_access_scenario() {
        let s: AccessScenario = serde_yaml::from_str(YAML).unwrap();
        assert_eq!(s.world.teams.len(), 2);
        assert_eq!(s.world.teams[1].parents, vec!["temper-system".to_string()]);
        assert_eq!(s.world.cogmaps.len(), 2);
        assert!(s.world.cogmaps[0].telos.is_none());
        assert!(s.world.cogmaps[1].telos.is_some());
        assert_eq!(s.world.resources.len(), 2);
        assert!(matches!(s.world.resources[0].home, HomeDef::Cogmap { .. }));
        assert!(matches!(s.world.resources[1].home, HomeDef::Context {}));
        assert!(matches!(
            s.world.resources[1].grants[0].to,
            GrantAnchor::Profile { .. }
        ));
        assert_eq!(s.world.edges.len(), 1);
        assert_eq!(s.checks.len(), 5);
        assert!(matches!(
            s.checks[0],
            AccessCheck::VisibleTo { expect: true, .. }
        ));
        assert!(matches!(
            s.checks[4],
            AccessCheck::CharterBlocksVisible { expect_count: 0, .. }
        ));
    }
}
```

- [ ] **Step 3: Run the test to verify it fails (then passes)**

Run: `cargo test -p temper-next --lib scenario::access::model::tests::parses_access_scenario`
Expected: this is a pure (no-DB) unit test — it should compile and **pass** once the structs are in place. If it fails, the YAML shape and the structs disagree — fix the struct, not the test. (If a serde_yaml internally-tagged empty-variant error appears on `HomeDef::Context {}`, confirm the variant is written as a struct variant `Context {}` — it is above.)

- [ ] **Step 4: Lint + commit**

Run: `cargo make fix && cargo make check`
Expected: clean. Then:

```bash
git add crates/temper-next/src/scenario/access/mod.rs \
        crates/temper-next/src/scenario/access/model.rs \
        crates/temper-next/src/scenario/mod.rs
git commit -m "feat(temper-next): access-scenario YAML model + parse test"
```

---

### Task 2: Author the `epd-bridge-access.yaml` fixture

**Files:**
- Create: `schema-artifact/access-scenarios/epd-bridge-access.yaml`
- Test: add to `crates/temper-next/src/scenario/access/model.rs` `tests` module

This is the access spec's worked example, ported from `schema-artifact/03_seed.sql`. The onboarding telos prose mirrors `schema-artifact/scenarios/onboarding-cogmap.yaml`'s seed (a short statement + a couple of questions is enough — S4 only needs *some* charter blocks to exist and be gated).

- [ ] **Step 1: Write the fixture file**

Create `schema-artifact/access-scenarios/epd-bridge-access.yaml`:

```yaml
# The access spec's worked example (epd-team-a / epd-team-b intersection bridge), ported from
# schema-artifact/03_seed.sql. Makes every leak-safety invariant (S1-S5) a declarative check.
#
# Teams DAG (child -> parents):          Cogmaps (-> joined teams):
#   temper-system (root)                   system-default -> {temper-system}          (public floor)
#      |- org-common                       bridge-map     -> {epd-team-a, epd-team-b}  (the intersection)
#      |- epd-department                   side-map       -> {epd-team-a}              (shares team-a)
#      \- directors                        directors-map  -> {directors}               (homes private edge)
#   epd-team-a -> {epd-department, org-common}   onboarding-cogmap -> {org-common}     (genesis, telos blocks)
#   epd-team-b -> {epd-department, org-common}
# People: alice in team-a, bob in team-b, dave in org-common, carol in directors,
#         sysadmin (admin), nomad (none).
name: epd-bridge-access
world:
  profiles:
    - { handle: alice,    display_name: Alice,    system_access: approved }
    - { handle: bob,      display_name: Bob,      system_access: approved }
    - { handle: dave,     display_name: Dave,     system_access: approved }
    - { handle: carol,    display_name: Carol,    system_access: approved }
    - { handle: sysadmin, display_name: Sysadmin, system_access: admin }
    - { handle: nomad,    display_name: Nomad,    system_access: none }
  entities:
    - { name: carol-agent,      profile: carol }   # emits the directors' private edge
    - { name: onboarding-agent, profile: dave }    # emits the onboarding genesis
  teams:
    - { slug: temper-system,  name: Temper System }
    - { slug: org-common,     name: Org Common }
    - { slug: epd-department, name: EPD Department }
    - { slug: directors,      name: Directors }
    - { slug: epd-team-a,     name: EPD Team A, parents: [epd-department, org-common] }
    - { slug: epd-team-b,     name: EPD Team B, parents: [epd-department, org-common] }
  memberships:
    - { team: epd-team-a, profile: alice, role: member }
    - { team: epd-team-b, profile: bob,   role: member }
    - { team: org-common, profile: dave,  role: member }
    - { team: directors,  profile: carol, role: member }
  cogmaps:
    - { name: system-default, teams: [temper-system] }
    - { name: bridge-map,     teams: [epd-team-a, epd-team-b] }
    - { name: side-map,       teams: [epd-team-a] }
    - { name: directors-map,  teams: [directors] }
    - name: onboarding-cogmap
      teams: [org-common]
      owner: dave
      emitter: onboarding-agent
      telos:
        title: "Onboarding"
        statement: "Help a new engineer reach a first real, shippable change in week one."
        questions:
          - { question: "What prior knowledge transfers onto this codebase?",
              context: "Surface the mental models a newcomer already has that map onto our stack." }
          - { question: "What is the smallest real change that exercises the whole loop?" }
        framing:
          - "This map situates first-week onboarding, not the whole engineering ladder."
  resources:
    # Public floor: two public concepts homed in system-default, granted-read to the root team
    # (root grants reach every vis(T) via down-only inheritance => universal read).
    - { key: concept_sprint, title: "concept: sprint-rituals", origin_uri: "temper://c/sprint",
        home: { anchor: cogmap, name: system-default }, owner: sysadmin,
        grants: [{ to: { anchor: team, slug: temper-system }, can_read: true }] }
    - { key: concept_formal, title: "concept: formalization-mandate", origin_uri: "temper://c/formal",
        home: { anchor: cogmap, name: system-default }, owner: sysadmin,
        grants: [{ to: { anchor: team, slug: temper-system }, can_read: true }] }
    # R_common: granted to org-common => in vis(a) AND vis(b) (both descend) => in the bridge intersection.
    - { key: r_common, title: "doc: org-common-policy", origin_uri: "temper://d/common",
        home: { anchor: context }, owner: dave,
        grants: [{ to: { anchor: team, slug: org-common }, can_read: true, can_write: true }] }
    # R_a_private: granted to team-a only => in vis(a) but NOT vis(b) => OUT of the intersection.
    - { key: r_a_private, title: "doc: team-a-private", origin_uri: "temper://d/aprivate",
        home: { anchor: context }, owner: alice,
        grants: [{ to: { anchor: team, slug: epd-team-a }, can_read: true }] }
    # R_profile_shared: a PROFILE grant to alice => visible to alice (consumer) but NEVER in any
    # vis(T) => not producer-readable by bridge-map (the leak-safety crux).
    - { key: r_profile_shared, title: "doc: shared-with-alice", origin_uri: "temper://d/pshared",
        home: { anchor: context }, owner: dave,
        grants: [{ to: { anchor: profile, handle: alice }, can_read: true }] }
  edges:
    # The directors' PRIVATE edge between two PUBLIC concepts, homed in directors-map => invisible to
    # anyone who can't read that map, even though both endpoints are public (edge-home protection, S3).
    - { from: concept_sprint, to: concept_formal, kind: leads_to,
        label: "sprint-rituals->formalization", home: directors-map, emitter: carol-agent }
checks:
  # S1 - consumer axis
  - { check: visible_to, profile: alice, resource: r_a_private,      expect: true  }
  - { check: visible_to, profile: bob,   resource: r_a_private,      expect: false }
  - { check: visible_to, profile: alice, resource: r_profile_shared, expect: true  }
  - { check: visible_to, profile: bob,   resource: r_profile_shared, expect: false }
  - { check: visible_to, profile: nomad, resource: r_common,         expect: false }
  - { check: visible_to, profile: bob,   resource: r_common,         expect: true  }   # via org-common DAG
  # S2 - producer intersection + leak-safety
  - { check: producer_reach, cogmap: side-map,   resource: r_a_private,      expect: true  }
  - { check: producer_reach, cogmap: bridge-map, resource: r_a_private,      expect: false }  # narrows
  - { check: producer_reach, cogmap: bridge-map, resource: r_common,         expect: true  }  # common ground
  - { check: producer_reach, cogmap: bridge-map, resource: r_profile_shared, expect: false }  # profile grant never enters vis(T)
  # S3 - edge-home protection
  - { check: edge_visible_to, profile: carol, edge: "sprint-rituals->formalization", expect: true  }
  - { check: edge_visible_to, profile: alice, edge: "sprint-rituals->formalization", expect: false }
  - { check: edge_visible_to, profile: nomad, edge: "sprint-rituals->formalization", expect: false }
  # S4 - charter-block gating
  - { check: charter_blocks_visible, cogmap: onboarding-cogmap, profile: nomad, expect_count: 0 }
  # S5 - delegation priming
  - { check: cogmaps_share_team, a: bridge-map, b: side-map,      expect: true  }
  - { check: cogmaps_share_team, a: bridge-map, b: directors-map, expect: false }
```

- [ ] **Step 2: Add a failing test that the real fixture deserializes**

Add to the `tests` module in `crates/temper-next/src/scenario/access/model.rs`:

```rust
    #[test]
    fn epd_bridge_fixture_deserializes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../schema-artifact/access-scenarios/epd-bridge-access.yaml"
        );
        let s: AccessScenario =
            serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(s.name, "epd-bridge-access");
        assert_eq!(s.world.profiles.len(), 6);
        assert_eq!(s.world.teams.len(), 6);
        assert_eq!(s.world.cogmaps.len(), 5);
        assert_eq!(s.world.resources.len(), 5);
        assert_eq!(s.world.edges.len(), 1);
        assert_eq!(s.checks.len(), 16);
        // exactly one genesis cogmap (the onboarding charter)
        assert_eq!(
            s.world.cogmaps.iter().filter(|c| c.telos.is_some()).count(),
            1
        );
    }
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p temper-next --lib scenario::access::model::tests::epd_bridge_fixture_deserializes`
Expected: PASS (pure parse, no DB). A failure here means a YAML typo or a struct/field mismatch — fix the YAML or the struct.

- [ ] **Step 4: Commit**

```bash
git add schema-artifact/access-scenarios/epd-bridge-access.yaml \
        crates/temper-next/src/scenario/access/model.rs
git commit -m "feat(temper-next): epd-bridge access fixture (ports 03_seed topology)"
```

---

### Task 3: The loader (`access::load`) + topology load test

**Files:**
- Create: `crates/temper-next/src/scenario/access/loader.rs`
- Create: `crates/temper-next/tests/access_scenario.rs`
- Modify: `.config/nextest.toml` (add `access_scenario` to the `temper-next-write` group)

- [ ] **Step 1: Write the loader**

Create `crates/temper-next/src/scenario/access/loader.rs`:

```rust
//! Access-world loader: persists an `AccessWorld` atomically in one transaction and returns the
//! `name → Uuid` maps the check-evaluator resolves through. Topology rows (teams, DAG, profiles,
//! entities, memberships, homes, grants, bare cogmaps) are direct inserts — the "tiny identity rows,
//! direct, not event-projected" convention the charter loader already uses. The only event-backed
//! writes are `cogmap_genesis` for a telos-bearing cogmap (so S4's charter has real blocks) and
//! `relationship_assert` for a homed edge (`kb_edges` carries NOT-NULL event FKs).
//!
//! Ordering is load-bearing: teams are inserted FIRST so the `sync_system_membership` trigger can
//! join enabled profiles to the `temper-system` root by slug.

use crate::content;
use crate::events::{fire, SeedAction};
use crate::ids::{CogmapId, EntityId, ProfileId, ResourceId};
use crate::scenario::access::model::*;
use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

/// Resolved identity maps for the check-evaluator (edges are resolved by label at eval time).
pub struct LoadedAccess {
    pub profiles: HashMap<String, Uuid>, // handle -> id
    pub teams: HashMap<String, Uuid>,    // slug -> id
    pub cogmaps: HashMap<String, Uuid>,  // name -> id
    pub resources: HashMap<String, Uuid>, // key -> id
}

pub async fn load(pool: &PgPool, world: &AccessWorld) -> Result<LoadedAccess> {
    let mut tx = pool.begin().await?;

    // 1. Teams first — the sync_system_membership trigger joins enabled profiles to the
    //    temper-system root by slug, so the root must exist before any profile insert.
    let mut teams: HashMap<String, Uuid> = HashMap::new();
    for t in &world.teams {
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ($1,$2) RETURNING id",
            t.slug,
            t.name,
        )
        .fetch_one(&mut *tx)
        .await?;
        teams.insert(t.slug.clone(), id);
    }
    // 2. Teams DAG (child -> parents).
    for t in &world.teams {
        let child = teams.get(&t.slug).expect("team just inserted");
        for parent in &t.parents {
            let pid = teams
                .get(parent)
                .with_context(|| format!("team {} references unknown parent {}", t.slug, parent))?;
            sqlx::query!(
                "INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1,$2)",
                child,
                pid,
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    // 3. Profiles (trigger auto-joins the temper-system root for non-'none').
    let mut profiles: HashMap<String, Uuid> = HashMap::new();
    for p in &world.profiles {
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name, system_access) \
             VALUES ($1,$2,$3::system_access) RETURNING id",
            p.handle,
            p.display_name,
            p.system_access as _,
        )
        .fetch_one(&mut *tx)
        .await?;
        profiles.insert(p.handle.clone(), id);
    }
    // 4. Entities (event emitters).
    let mut entities: HashMap<String, Uuid> = HashMap::new();
    for e in &world.entities {
        let pid = profiles
            .get(&e.profile)
            .with_context(|| format!("entity {} references unknown profile {}", e.name, e.profile))?;
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb) RETURNING id",
            pid,
            e.name,
        )
        .fetch_one(&mut *tx)
        .await?;
        entities.insert(e.name.clone(), id);
    }
    // 5. Sub-team memberships (root joins already trigger-maintained).
    for m in &world.memberships {
        let tid = teams
            .get(&m.team)
            .with_context(|| format!("membership references unknown team {}", m.team))?;
        let pid = profiles
            .get(&m.profile)
            .with_context(|| format!("membership references unknown profile {}", m.profile))?;
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,$3::team_role)",
            tid,
            pid,
            m.role as _,
        )
        .execute(&mut *tx)
        .await?;
    }
    // 6. A single home-less placeholder telos resource for the bare producer maps
    //    (kb_cogmaps.telos_resource_id is NOT NULL; bare maps carry no charter — mirrors 03_seed's
    //    shared public telos). Genesis maps create their own telos.
    let placeholder_telos = sqlx::query_scalar!(
        "INSERT INTO kb_resources (title, origin_uri) \
         VALUES ('placeholder: bare-cogmap telos','temper://internal/placeholder-telos') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await?;

    // 7. Cogmaps. Bare maps: direct insert + team joins. Telos-bearing maps: cogmap_genesis.
    let mut cogmaps: HashMap<String, Uuid> = HashMap::new();
    for c in &world.cogmaps {
        let cid = match &c.telos {
            None => {
                sqlx::query_scalar!(
                    "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1,$2) RETURNING id",
                    c.name,
                    placeholder_telos,
                )
                .fetch_one(&mut *tx)
                .await?
            }
            Some(telos) => {
                let owner = ProfileId::from(
                    *profiles
                        .get(c.owner.as_deref().context("genesis cogmap needs owner")?)
                        .context("cogmap.owner not in world.profiles")?,
                );
                let emitter = EntityId::from(
                    *entities
                        .get(c.emitter.as_deref().context("genesis cogmap needs emitter")?)
                        .context("cogmap.emitter not in world.entities")?,
                );
                let specs = telos.block_specs();
                let refs: Vec<(Option<&str>, &str)> =
                    specs.iter().map(|(r, p)| (Some(*r), p.as_str())).collect();
                let blocks = content::prepare_blocks(&refs)?;
                let (cogmap, _telos) = fire(
                    &mut tx,
                    SeedAction::CogmapGenesis {
                        name: &c.name,
                        telos_title: &telos.title,
                        charter: &blocks,
                        owner,
                        emitter,
                    },
                )
                .await?
                .cogmap_genesis()?;
                cogmap.uuid()
            }
        };
        for team in &c.teams {
            let tid = teams
                .get(team)
                .with_context(|| format!("cogmap {} joins unknown team {}", c.name, team))?;
            sqlx::query!(
                "INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1,$2)",
                cid,
                tid,
            )
            .execute(&mut *tx)
            .await?;
        }
        cogmaps.insert(c.name.clone(), cid);
    }

    // 8. Resources: identity + home (context|cogmap) + explicit grants. Direct inserts (ports 03_seed).
    let mut resources: HashMap<String, Uuid> = HashMap::new();
    for r in &world.resources {
        let owner = *profiles
            .get(&r.owner)
            .with_context(|| format!("resource {} owner {} not in world.profiles", r.key, r.owner))?;
        let rid = sqlx::query_scalar!(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,$2) RETURNING id",
            r.title,
            r.origin_uri,
        )
        .fetch_one(&mut *tx)
        .await?;
        let (anchor_table, anchor_id) = match &r.home {
            HomeDef::Cogmap { name } => (
                "kb_cogmaps",
                *cogmaps
                    .get(name)
                    .with_context(|| format!("resource {} homes in unknown cogmap {}", r.key, name))?,
            ),
            HomeDef::Context {} => ("kb_contexts", Uuid::now_v7()),
        };
        sqlx::query!(
            "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1,$2,$3,$4,$4)",
            rid,
            anchor_table,
            anchor_id,
            owner,
        )
        .execute(&mut *tx)
        .await?;
        for g in &r.grants {
            let (ga_table, ga_id) = match &g.to {
                GrantAnchor::Team { slug } => (
                    "kb_teams",
                    *teams
                        .get(slug)
                        .with_context(|| format!("grant on {} references unknown team {}", r.key, slug))?,
                ),
                GrantAnchor::Profile { handle } => (
                    "kb_profiles",
                    *profiles.get(handle).with_context(|| {
                        format!("grant on {} references unknown profile {}", r.key, handle)
                    })?,
                ),
            };
            sqlx::query!(
                "INSERT INTO kb_resource_access \
                 (resource_id, anchor_table, anchor_id, can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
                rid,
                ga_table,
                ga_id,
                g.can_read,
                g.can_write,
                g.can_delete,
                g.can_grant,
                owner,
            )
            .execute(&mut *tx)
            .await?;
        }
        resources.insert(r.key.clone(), rid);
    }

    // 9. Edges: homed in a named cogmap, fired through relationship_assert.
    for e in &world.edges {
        let src = ResourceId::from(
            *resources
                .get(&e.from)
                .with_context(|| format!("edge from unknown key {}", e.from))?,
        );
        let tgt = ResourceId::from(
            *resources
                .get(&e.to)
                .with_context(|| format!("edge to unknown key {}", e.to))?,
        );
        let home = CogmapId::from(
            *cogmaps
                .get(&e.home)
                .with_context(|| format!("edge homes in unknown cogmap {}", e.home))?,
        );
        let emitter = EntityId::from(
            *entities
                .get(&e.emitter)
                .with_context(|| format!("edge emitter {} not in world.entities", e.emitter))?,
        );
        fire(
            &mut tx,
            SeedAction::RelationshipAssert {
                src,
                tgt,
                kind: e.kind,
                label: e.label.as_deref(),
                weight: e.weight,
                home,
                emitter,
            },
        )
        .await?;
    }

    tx.commit().await?;
    Ok(LoadedAccess {
        profiles,
        teams,
        cogmaps,
        resources,
    })
}
```

If you stubbed `loader`/`runner` out of `access/mod.rs` in Task 1, restore `pub mod loader;` and `pub use loader::{load, LoadedAccess};` now.

- [ ] **Step 2: Write the integration test harness + load test**

Create `crates/temper-next/tests/access_scenario.rs`:

```rust
#![cfg(feature = "artifact-tests")]
//! Access-scaffold proof: loads the epd-bridge access world from YAML and asserts the kernel gate
//! functions (S1-S5) declaratively, plus the S8 capability-coherence CHECK. These OWN the
//! `temper_next` namespace (each resets it to a clean 01+02 then loads) — serialized via the
//! `temper-next-write` nextest group, ONNX-dependent (the onboarding charter embeds inline).
mod common;

use temper_next::scenario::access::{self, model::AccessScenario};
use temper_next::scenario::bootseed;
use temper_next::substrate;

const ACCESS_SCENARIO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema-artifact/access-scenarios/epd-bridge-access.yaml"
);

fn load_access_yaml() -> AccessScenario {
    serde_yaml::from_str(&std::fs::read_to_string(ACCESS_SCENARIO).unwrap()).unwrap()
}

#[tokio::test]
async fn loads_topology_row_counts() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    let doc = load_access_yaml();
    let loaded = access::load(&pool, &doc.world).await.unwrap();

    assert_eq!(loaded.profiles.len(), 6);
    assert_eq!(loaded.teams.len(), 6);
    assert_eq!(loaded.cogmaps.len(), 5);
    assert_eq!(loaded.resources.len(), 5);

    // Row-count sanity against the DB (bootseed adds no teams).
    let teams: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_teams")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(teams, 6);
    // alice was auto-joined to temper-system root (approved) + joined epd-team-a => 2 memberships.
    let alice_teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members m JOIN kb_profiles p ON p.id=m.profile_id WHERE p.handle='alice'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alice_teams, 2);
    // nomad (system_access=none) joined nothing.
    let nomad_teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members m JOIN kb_profiles p ON p.id=m.profile_id WHERE p.handle='nomad'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nomad_teams, 0);
}
```

- [ ] **Step 3: Register the test binary in the `temper-next-write` group**

In `.config/nextest.toml`, add `access_scenario` to the `binary(...)` regex of the `temper-next-write` override. Change:

```
filter = 'package(temper-next) & binary(/^bootseed$|^scenario_load$|^scenario_roundtrip$|^content_multichunk$|^cogmap_genesis_charter$|^charter_block_roles$|^charter_yaml_roundtrip$|^ledger_envelope$|^replay_roundtrip$|^seed_load_path_equivalence$|^seed_corpus_sweep$/)'
```

to (append `|^access_scenario$` before the closing `/`):

```
filter = 'package(temper-next) & binary(/^bootseed$|^scenario_load$|^scenario_roundtrip$|^content_multichunk$|^cogmap_genesis_charter$|^charter_block_roles$|^charter_yaml_roundtrip$|^ledger_envelope$|^replay_roundtrip$|^seed_load_path_equivalence$|^seed_corpus_sweep$|^access_scenario$/)'
```

- [ ] **Step 4: Regenerate the offline sqlx cache (new macro queries hit `temper_next`)**

Ensure the dev DB is up, then regenerate the per-crate cache:

Run: `cargo make docker-up && cargo make prepare-next`
Expected: `crates/temper-next/.sqlx/` gains entries for the new `loader.rs` queries; no errors. (This task loads the artifact and prepares with `search_path=temper_next`.)

- [ ] **Step 5: Run the load test (offline-compiled, live DB at runtime)**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests -E 'test(loads_topology_row_counts)'`
Expected: PASS. Common failure modes and what they mean:
- A `null value in column "team_id"` from the membership trigger ⇒ `temper-system` not inserted before profiles — confirm the team loop runs before the profile loop.
- A unique-violation on `kb_teams.slug` ⇒ bootseed unexpectedly seeded a team; it should not. Re-read `bootseed::seed_system` (event-types + lenses only).

- [ ] **Step 6: Lint + commit**

Run: `cargo make check`
Expected: clean (offline cache current). Then:

```bash
git add crates/temper-next/src/scenario/access/loader.rs \
        crates/temper-next/src/scenario/access/mod.rs \
        crates/temper-next/tests/access_scenario.rs \
        .config/nextest.toml \
        crates/temper-next/.sqlx
git commit -m "feat(temper-next): access-world loader + topology load test"
```

---

### Task 4: The check evaluator + run-all-invariants test

**Files:**
- Create: `crates/temper-next/src/scenario/access/runner.rs`
- Modify: `crates/temper-next/tests/access_scenario.rs` (add the full-proof test)

- [ ] **Step 1: Write the evaluator**

Create `crates/temper-next/src/scenario/access/runner.rs`:

```rust
//! Access-scenario runner: loads the world, then evaluates each `AccessCheck` against one kernel gate
//! function. Each check is a boolean (or count) compared to its declared expectation, with a failure
//! message naming the referents — a declarative echo of `schema-artifact/04_scenarios.sql`'s S1-S5.

use crate::scenario::access::loader::{self, LoadedAccess};
use crate::scenario::access::model::*;
use anyhow::{bail, Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn run_access_scenario(pool: &PgPool, doc: &AccessScenario) -> Result<()> {
    let loaded = loader::load(pool, &doc.world).await?;
    for (i, c) in doc.checks.iter().enumerate() {
        eval_access_check(pool, &loaded, c)
            .await
            .with_context(|| format!("check {i} failed"))?;
    }
    Ok(())
}

async fn eval_access_check(pool: &PgPool, loaded: &LoadedAccess, c: &AccessCheck) -> Result<()> {
    match c {
        AccessCheck::VisibleTo {
            profile,
            resource,
            expect,
        } => {
            let p = profile_id(loaded, profile)?;
            let r = resource_id(loaded, resource)?;
            let got = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2)",
                p,
                r,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);
            if got != *expect {
                bail!("visible_to: profile {profile} / resource {resource} = {got}, expected {expect}");
            }
        }
        AccessCheck::ProducerReach {
            cogmap,
            resource,
            expect,
        } => {
            let m = cogmap_id(loaded, cogmap)?;
            let r = resource_id(loaded, resource)?;
            let got = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM resources_accessible_to_cogmap($1) a WHERE a.resource_id=$2)",
                m,
                r,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);
            if got != *expect {
                bail!("producer_reach: cogmap {cogmap} / resource {resource} = {got}, expected {expect}");
            }
        }
        AccessCheck::EdgeVisibleTo {
            profile,
            edge,
            expect,
        } => {
            let p = profile_id(loaded, profile)?;
            let eid = sqlx::query_scalar!(
                "SELECT id FROM kb_edges WHERE label=$1 AND NOT is_folded",
                edge,
            )
            .fetch_optional(pool)
            .await?
            .with_context(|| format!("edge_visible_to: no edge labelled {edge:?}"))?;
            let got = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM edges_visible_to($1) e WHERE e.edge_id=$2)",
                p,
                eid,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(false);
            if got != *expect {
                bail!("edge_visible_to: profile {profile} / edge {edge} = {got}, expected {expect}");
            }
        }
        AccessCheck::CogmapsShareTeam { a, b, expect } => {
            let ca = cogmap_id(loaded, a)?;
            let cb = cogmap_id(loaded, b)?;
            let got = sqlx::query_scalar!("SELECT cogmaps_share_a_team($1,$2)", ca, cb)
                .fetch_one(pool)
                .await?
                .unwrap_or(false);
            if got != *expect {
                bail!("cogmaps_share_team: {a} & {b} = {got}, expected {expect}");
            }
        }
        AccessCheck::CharterBlocksVisible {
            cogmap,
            profile,
            expect_count,
        } => {
            let m = cogmap_id(loaded, cogmap)?;
            let p = profile_id(loaded, profile)?;
            let n = sqlx::query_scalar!(
                "SELECT count(*) FROM resource_blocks(cogmap_telos($1), 'profile', $2, NULL)",
                m,
                p,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0);
            if n != *expect_count {
                bail!("charter_blocks_visible: cogmap {cogmap} / profile {profile} = {n} blocks, expected {expect_count}");
            }
        }
    }
    Ok(())
}

fn profile_id(l: &LoadedAccess, h: &str) -> Result<Uuid> {
    l.profiles
        .get(h)
        .copied()
        .with_context(|| format!("unknown profile handle {h}"))
}
fn resource_id(l: &LoadedAccess, k: &str) -> Result<Uuid> {
    l.resources
        .get(k)
        .copied()
        .with_context(|| format!("unknown resource key {k}"))
}
fn cogmap_id(l: &LoadedAccess, n: &str) -> Result<Uuid> {
    l.cogmaps
        .get(n)
        .copied()
        .with_context(|| format!("unknown cogmap name {n}"))
}
```

If you stubbed `runner` out of `access/mod.rs`, restore `pub mod runner;` and `pub use runner::run_access_scenario;` now.

- [ ] **Step 2: Add the full-proof integration test**

Append to `crates/temper-next/tests/access_scenario.rs`:

```rust
#[tokio::test]
async fn proves_all_access_invariants() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    access::run_access_scenario(&pool, &load_access_yaml())
        .await
        .expect("all S1-S5 access checks pass");
}
```

- [ ] **Step 3: Regenerate the offline cache (new evaluator queries)**

Run: `cargo make prepare-next`
Expected: `crates/temper-next/.sqlx/` gains entries for the `runner.rs` queries; no errors.

- [ ] **Step 4: Run the full proof**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests -E 'test(proves_all_access_invariants)'`
Expected: PASS — all 16 checks hold. If a single check fails, the `bail!` message names the profile/cogmap/resource and the got-vs-expected booleans; cross-check against the matching `\echo` block in `schema-artifact/04_scenarios.sql` to see which invariant moved. Do NOT loosen a check to make it pass — if a gate function genuinely disagrees with the spec, STOP and report it (a real leak-safety regression is exactly what this test exists to catch).

- [ ] **Step 5: Lint + commit**

Run: `cargo make check`
Expected: clean. Then:

```bash
git add crates/temper-next/src/scenario/access/runner.rs \
        crates/temper-next/src/scenario/access/mod.rs \
        crates/temper-next/tests/access_scenario.rs \
        crates/temper-next/.sqlx
git commit -m "feat(temper-next): access check evaluator — S1-S5 leak-safety proof"
```

---

### Task 5: S8 — capability coherence CHECK unit test

**Files:**
- Modify: `crates/temper-next/tests/access_scenario.rs` (add the S8 test)

The descriptor coherence CHECK (`(can_write OR can_delete OR can_grant) <= can_read`) is a schema
constraint, proven by attempting an invalid grant and asserting `check_violation` (SQLSTATE 23514).
The profile uses `system_access='none'` so the `sync_system_membership` trigger does not try to join a
(non-existent, un-seeded) `temper-system` root.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-next/tests/access_scenario.rs`:

```rust
#[tokio::test]
async fn s8_capability_check_rejects_write_without_read() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    // Minimal anchors. 'none' avoids the root-join trigger (no temper-system team in a bare reset).
    let pid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, system_access) \
         VALUES ('s8user','S8','none') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let rid: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('s8','temper://s8') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // can_write=true with can_read=false must be rejected by the coherence CHECK.
    let res = sqlx::query(
        "INSERT INTO kb_resource_access \
         (resource_id, anchor_table, anchor_id, can_read, can_write, granted_by_profile_id) \
         VALUES ($1,'kb_profiles',$2,false,true,$2)",
    )
    .bind(rid)
    .bind(pid)
    .execute(&pool)
    .await;

    let err = res.expect_err("write-without-read grant must be rejected");
    let is_check_violation = matches!(
        &err,
        sqlx::Error::Database(e) if e.code().as_deref() == Some("23514")
    );
    assert!(is_check_violation, "expected check_violation (23514), got {err:?}");
}
```

- [ ] **Step 2: Run the test**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests -E 'test(s8_capability_check_rejects_write_without_read)'`
Expected: PASS. (These are runtime `sqlx::query` calls — no macro, so no cache regen needed.)

- [ ] **Step 3: Commit**

Run: `cargo make check`
Expected: clean. Then:

```bash
git add crates/temper-next/tests/access_scenario.rs
git commit -m "test(temper-next): S8 capability-coherence CHECK (write implies read)"
```

---

### Task 6: JSON-Schema snapshot + final verification

**Files:**
- Create: `schema-artifact/access-scenarios/access-scenario.schema.json`
- Modify: `crates/temper-next/tests/scenario_schema.rs` (add the `AccessScenario` snapshot test)

- [ ] **Step 1: Add the snapshot test**

Append to `crates/temper-next/tests/scenario_schema.rs` (it is gated `#![cfg(feature = "scenario-schema")]` already):

```rust
const ACCESS_SCENARIO_SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema-artifact/access-scenarios/access-scenario.schema.json"
);

#[test]
fn access_scenario_json_schema_matches_snapshot() {
    let schema = schemars::schema_for!(temper_next::scenario::access::model::AccessScenario);
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    assert_snapshot(&rendered, ACCESS_SCENARIO_SNAPSHOT, "access-scenario");
}
```

- [ ] **Step 2: Generate the snapshot file**

Run: `UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema access_scenario_json_schema_matches_snapshot`
Expected: writes `schema-artifact/access-scenarios/access-scenario.schema.json`. Then run again WITHOUT `UPDATE_SCHEMA` to confirm it now matches:

Run: `cargo test -p temper-next --features scenario-schema access_scenario_json_schema_matches_snapshot`
Expected: PASS.

- [ ] **Step 3: Full crate verification (both feature surfaces)**

Run the write-path artifact suite (serialized, ONNX) to confirm nothing regressed and the access tests pass together:

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests -E 'binary(access_scenario)'`
Expected: 3 tests pass (`loads_topology_row_counts`, `proves_all_access_invariants`, `s8_capability_check_rejects_write_without_read`).

Then the schema feature:

Run: `cargo test -p temper-next --features scenario-schema`
Expected: all schema snapshot tests pass.

Then the offline quality gate:

Run: `cargo make check`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add schema-artifact/access-scenarios/access-scenario.schema.json \
        crates/temper-next/tests/scenario_schema.rs
git commit -m "test(temper-next): access-scenario JSON-Schema snapshot"
```

---

## Self-Review

**Spec coverage:**
- §1 access-scenario document (world + checks) → Task 1 (model) + Task 2 (fixture). ✓
- §2 check variants (S1 `visible_to`, S2 `producer_reach`, S3 `edge_visible_to`, S5 `cogmaps_share_team`, S4 `charter_blocks_visible`) → Task 1 (enum) + Task 4 (evaluator). ✓
- §3 loader (`access::load`, bare inserts, genesis only for onboarding, `relationship_assert` for the edge, homes/grants ported from `03_seed`) → Task 3. ✓
- §4 wiring (file under `access-scenarios/`, `temper-next-write` group, JSON-Schema snapshot, `prepare-next`) → Tasks 3 + 6. ✓
- §5 S8 unit test + coverage map → Task 5. ✓
- "No DDL delta" → confirmed; no task touches `01_schema.sql`/`02_functions.sql`. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code; every run step shows the command + expected result. ✓

**Type consistency:** `LoadedAccess { profiles, teams, cogmaps, resources }` defined in Task 3, consumed in Task 4. `access::load` / `access::run_access_scenario` names match the `pub use` re-exports in Task 1's `mod.rs`. `HomeDef::{Cogmap{name}, Context{}}`, `GrantAnchor::{Team{slug}, Profile{handle}}`, and the five `AccessCheck` variants are referenced identically in model (Task 1), loader (Task 3), and evaluator (Task 4). Gate-function names and signatures (`resources_visible_to`, `resources_accessible_to_cogmap`, `edges_visible_to`, `cogmaps_share_a_team`, `cogmap_telos`, `resource_blocks(uuid, text, uuid, text)`) match `schema-artifact/02_functions.sql`. ✓

**Order-of-operations risk flagged in-plan:** teams-before-profiles (trigger), placeholder-telos-before-bare-cogmaps (NOT NULL FK), cogmaps-before-resource-homes, resources-before-edges. ✓
