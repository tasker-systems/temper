# Access-scaffold scenario proof: making the leak-safety invariants declarative and reproducible

**Date:** 2026-06-11
**Status:** Design — drafted, pending review
**Goal:** `substrate-kernel-to-cognitive-map`, workstream 2 (access scaffold)
**Scope:** artifact-side proof only. Production team-mechanics convergence stays in workstream 6 (migration).

---

## Context

The access/authorization model of the `temper_next` artifact is **fully schema'd** — `01_schema.sql`
carries the tables (`kb_teams`, `kb_team_members`, `kb_teams_parents`, `kb_team_cogmaps`,
`kb_resource_homes`, `kb_resource_access` with the four capability booleans, `kb_edges` with
denormalized home columns) and `02_functions.sql` carries the gate functions (`resources_visible_to`,
`resources_accessible_to_cogmap`, `edges_visible_to`, `cogmaps_share_a_team`, `team_ancestors`,
`sync_system_membership`). The model was designed in
[`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md) and is
reviewed and sound.

It is also **already exercised** — by hand. `03_seed.sql` builds one coherent access topology (the
access spec's own worked example: the epd-team-a / epd-team-b intersection bridge), and
`04_scenarios.sql` walks every load-bearing invariant (S1–S8) against it with `\echo` comments and
eyeballed psql output. This is the empirical read the fresh-schema strategy is after — but it is a
**manual** read: nothing reproduces it, nothing fails CI when a refactor breaks a leak-safety bound.

The goal's workstream 2 mandate is exact: *"Every leak-safety invariant lands as a scenario
expectation."* This spec makes the `04_scenarios.sql` invariants **declarative** (authored as YAML),
**reproducible** (run by the temper-next harness), and **CI-checkable** (the `temper-next-write`
nextest group).

### Why the existing scenario DSL cannot express it

The current scenario DSL (`crates/temper-next/src/scenario/model.rs`) proves *clustering*, not
*access*. Two structural gaps:

1. **The `Seed` model is single-cogmap** (`cogmap: CogmapDef`) and carries only
   `{ profiles (+system_access), entities, resources, edges, uses_lenses }`. It cannot express the
   access topology: **multiple cogmaps** (S2's producer-intersection needs side-map ∩ bridge-map ∩
   directors-map), **teams + the teams-DAG** (`kb_teams_parents`), **team↔cogmap joins**
   (`kb_team_cogmaps`), **capability grants** (`kb_resource_access`, team- *and* profile-anchored),
   and **edge homes** in specific cogmaps.
2. **No `Expectation` variant asserts visibility.** All existing expectations are region/membership/
   clustering/staleness. None can express "profile P sees resource R" or "cogmap M's producer reach
   excludes R."

### The decided shape (brainstorm, 2026-06-11)

- **A separate, self-contained access-scenario document kind** — not an extension of the
  charter `Seed`. The foundational-charter seeds are single-cogmap telos charters; the access
  topology (multi-cogmap, teams, DAG, grants) is a different animal, and the access proof is *static*
  (seed the world, assert the gate functions) with no `materialize`/lens/telos-charter machinery to
  share. One document carries both the world and its checks.
- **The capability coherence CHECK (S8) is a focused unit test, not a declarative check.** "This
  invalid INSERT must raise `check_violation`" is a schema-constraint assertion, not a
  topology-behavior proof; contorting the declarative DSL to express a negative write is not worth it.
- **Bare producer cogmaps are direct inserts.** The point of this test set is access levels, not
  exercising `cogmap_genesis`. Producer maps (bridge/side/directors) get a direct `kb_cogmaps` +
  `kb_team_cogmaps` insert; only the charter-bearing onboarding map goes through genesis (S4 needs real
  telos blocks).

---

## 1. The access-scenario document (new kind)

A new YAML kind under `schema-artifact/access-scenarios/`, parsed by a new `AccessScenario` model in
`crates/temper-next/src/scenario/access.rs`. It carries a full access **world** plus inline
**checks** — no `steps`, `materialize`, lens, or telos machinery in the base case.

```yaml
name: epd-bridge-access
world:
  profiles:                # reuses ProfileDef (handle, display_name, system_access)
    - { handle: alice,    display_name: Alice,    system_access: approved }
    - { handle: bob,      display_name: Bob,      system_access: approved }
    - { handle: dave,     display_name: Dave,     system_access: approved }
    - { handle: carol,    display_name: Carol,    system_access: approved }
    - { handle: sysadmin, display_name: Sysadmin, system_access: admin }
    - { handle: nomad,    display_name: Nomad,    system_access: none }
  entities:                # reuses EntityDef (name, profile) — one emitter per producing cogmap
    - { name: carol-agent,      profile: carol }
    - { name: bridge-agent,     profile: alice }
    - { name: side-agent,       profile: alice }
    - { name: sysadmin-agent,   profile: sysadmin }
    - { name: onboarding-agent, profile: dave }
  teams:                   # NEW — slug, name, optional parents (the DAG, child→parents)
    - { slug: temper-system,  name: Temper System }
    - { slug: org-common,     name: Org Common }
    - { slug: epd-department, name: EPD Department }
    - { slug: directors,      name: Directors }
    - { slug: epd-team-a,     name: EPD Team A, parents: [epd-department, org-common] }
    - { slug: epd-team-b,     name: EPD Team B, parents: [epd-department, org-common] }
  memberships:             # NEW — sub-team joins (root joins are maintained by the system_access trigger)
    - { team: epd-team-a, profile: alice, role: member }
    - { team: epd-team-b, profile: bob,   role: member }
    - { team: org-common, profile: dave,  role: member }
    - { team: directors,  profile: carol, role: member }
  cogmaps:                 # NEW — multiple; teams[] = kb_team_cogmaps joins; optional telos = genesis
    - { name: system-default, teams: [temper-system],         owner: sysadmin, emitter: sysadmin-agent }
    - { name: bridge-map,     teams: [epd-team-a, epd-team-b], owner: alice,    emitter: bridge-agent }
    - { name: side-map,       teams: [epd-team-a],             owner: alice,    emitter: side-agent }
    - { name: directors-map,  teams: [directors],             owner: carol,    emitter: carol-agent }
    - name: onboarding-cogmap
      teams: [org-common]
      owner: dave
      emitter: onboarding-agent
      telos: { title: …, statement: …, questions: [ … ], framing: [ … ] }   # the only genesis path
  resources:               # NEW shape — home (context|cogmap) + owner + explicit grants
    - { key: concept_sprint, title: "concept: sprint-rituals",   origin_uri: temper://c/sprint,
        home: { cogmap: system-default }, owner: sysadmin,
        grants: [{ to: team:temper-system, can_read: true }] }
    - { key: concept_formal, title: "concept: formalization-mandate", origin_uri: temper://c/formal,
        home: { cogmap: system-default }, owner: sysadmin,
        grants: [{ to: team:temper-system, can_read: true }] }
    - { key: r_common,       title: "doc: org-common-policy",     origin_uri: temper://d/common,
        home: { context: {} }, owner: dave,
        grants: [{ to: team:org-common, can_read: true, can_write: true }] }
    - { key: r_a_private,    title: "doc: team-a-private",        origin_uri: temper://d/aprivate,
        home: { context: {} }, owner: alice,
        grants: [{ to: team:epd-team-a, can_read: true }] }
    - { key: r_profile_shared, title: "doc: shared-with-alice",   origin_uri: temper://d/pshared,
        home: { context: {} }, owner: dave,
        grants: [{ to: profile:alice, can_read: true }] }          # the leak-safety crux
  edges:                   # EdgeDef + explicit home cogmap + emitter (the private-edge case)
    - { from: concept_sprint, to: concept_formal, kind: leads_to,
        label: "sprint-rituals→formalization", home: directors-map, emitter: carol-agent }
checks: [ … see §2 … ]
```

**New vocabulary:** `teams` (with `parents`), `memberships`, multi-`cogmaps` (with `teams` joins and
optional `telos`), resource `home` (`{ cogmap: <name> }` or `{ context: {} }`), resource `owner`,
resource `grants` (`{ to: team:<slug> | profile:<handle>, can_read, can_write?, can_delete?,
can_grant? }`), and edge `home` + `emitter`. Everything else reuses the existing `ProfileDef`,
`EntityDef`, `EdgeDef`, `TelosDef`, `QuestionDef`.

The topology is the access spec's worked example, ported faithfully from `03_seed.sql`:

```
Teams DAG (child → parent):           Cogmaps (→ joined teams):
  temper-system (root)                  system-default → {temper-system}    (public floor)
     ├── org-common                     bridge-map     → {epd-team-a, epd-team-b}  (the intersection)
     ├── epd-department                 side-map       → {epd-team-a}        (shares team-a w/ bridge)
     └── directors                      directors-map  → {directors}         (homes the private edge)
  epd-team-a → {epd-department, org-common}    onboarding-cogmap → {org-common}    (genesis, telos blocks)
  epd-team-b → {epd-department, org-common}
People: alice∈a · bob∈b · dave∈org-common · carol∈directors · sysadmin(admin) · nomad(none)
```

## 2. Check variants (the leak-safety set)

A new `AccessCheck` enum, internally tagged by `check:` (same serde convention as `Step`/`Expectation`,
which serde_yaml 0.9 requires). Each variant resolves its named referents through the loader's
`key → Uuid` map, calls one gate function, and asserts a boolean or count:

| `check:` | gate function called | invariant |
|---|---|---|
| `visible_to { profile, resource, expect }` | `resources_visible_to(p)` ∋ r | **S1** consumer reach |
| `producer_reach { cogmap, resource, expect }` | `resources_accessible_to_cogmap(m)` ∋ r | **S2** intersection + profile-grant leak-safety |
| `edge_visible_to { profile, edge, expect }` | `edges_visible_to(p)` ∋ e | **S3** edge-home protection |
| `cogmaps_share_team { a, b, expect }` | `cogmaps_share_a_team(a, b)` | **S5** delegation priming |
| `charter_blocks_visible { cogmap, profile, expect_count }` | `count(resource_blocks(telos(m), 'profile', p, NULL))` | **S4** charter-block gating |

`edge` is resolved by `label`; `cogmap`/`profile`/`resource` by name/handle/key. Each check evaluates
to a boolean (or count) compared against `expect` (or `expect_count`), with an assertion-failure
message naming the referents — the same per-check style as `runner::eval_expectation`, and a direct
declarative echo of the `04_scenarios.sql` `\echo` intents.

The representative scenario's checks (the S1–S5 set in full):

```yaml
checks:
  # S1 — consumer axis
  - { check: visible_to, profile: alice, resource: r_a_private,     expect: true  }
  - { check: visible_to, profile: bob,   resource: r_a_private,     expect: false }
  - { check: visible_to, profile: alice, resource: r_profile_shared, expect: true  }
  - { check: visible_to, profile: bob,   resource: r_profile_shared, expect: false }
  - { check: visible_to, profile: nomad, resource: r_common,        expect: false }
  - { check: visible_to, profile: bob,   resource: r_common,        expect: true  }  # via org-common DAG
  # S2 — producer intersection + leak-safety
  - { check: producer_reach, cogmap: side-map,   resource: r_a_private,     expect: true  }
  - { check: producer_reach, cogmap: bridge-map, resource: r_a_private,     expect: false }  # narrows
  - { check: producer_reach, cogmap: bridge-map, resource: r_common,        expect: true  }  # common ground
  - { check: producer_reach, cogmap: bridge-map, resource: r_profile_shared, expect: false }  # profile grant never enters vis(T)
  # S3 — edge-home protection (private edge, public endpoints)
  - { check: edge_visible_to, profile: carol, edge: "sprint-rituals→formalization", expect: true  }
  - { check: edge_visible_to, profile: alice, edge: "sprint-rituals→formalization", expect: false }
  - { check: edge_visible_to, profile: nomad, edge: "sprint-rituals→formalization", expect: false }
  # S4 — charter-block gating
  - { check: charter_blocks_visible, cogmap: onboarding-cogmap, profile: nomad, expect_count: 0 }
  # S5 — delegation priming
  - { check: cogmaps_share_team, a: bridge-map, b: side-map,      expect: true  }
  - { check: cogmaps_share_team, a: bridge-map, b: directors-map, expect: false }
```

## 3. Loader (`access::load`)

A new loader in `access.rs`, mirroring `loader::load_seed`'s atomic single-transaction pattern but for
the access world. It loads in dependency order so foreign keys and the `sync_system_membership` trigger
resolve:

1. **Teams + DAG** first (`kb_teams`, then `kb_teams_parents`) — the root must exist before profiles
   enable, since enabling a profile (`system_access <> 'none'`) auto-joins `temper-system` via the
   trigger.
2. **Profiles + entities** (direct inserts, exactly as `load_seed` does today).
3. **Sub-team memberships** (`kb_team_members`; the root joins are already trigger-maintained).
4. **Cogmaps:** bare producer maps via direct `kb_cogmaps` + `kb_team_cogmaps` inserts (sharing a
   `telos_resource_id` placeholder as `03_seed` does); the charter-bearing onboarding map via
   `cogmap_genesis` (the only genesis call) so S4 has real telos blocks, then its `kb_team_cogmaps`
   join.
5. **Resources:** for each, insert `kb_resources`, then `kb_resource_homes` (anchor `kb_contexts` with
   a generated anchor for docs, `kb_cogmaps` for concepts — **ported from `03_seed.sql` exactly**),
   then one `kb_resource_access` row per grant (team- or profile-anchored).
6. **Edges:** the homed directors' edge via `kb_edges` insert with `home_anchor_table='kb_cogmaps'`,
   `home_anchor_id = directors-map` (or `relationship_assert` if the home is threadable through it —
   plan-level).

These are "tiny identity rows, direct, not event-projected" — the convention `loader.rs` already
documents for world rows. The **access checks** read no embeddings and run **no `materialize`** — the
gate functions read homes/grants/teams, never chunk vectors. The one place embeddings enter is the
onboarding charter: the Rust `cogmap_genesis` path embeds its telos blocks inline via
`content::prepare_blocks` (bge-768), exactly like any resource body. That is fine — the test sits in
the `temper-next-write` group, which already provisions ONNX (§4); we do not hand-roll NULL-embedding
blocks. The loader returns a `Loaded`-style map of `key → Uuid` (profiles by handle, resources by key,
cogmaps by name, teams by slug, edges by label) for the check-evaluator.

## 4. Runner and wiring

- **Runner:** `access::run_access_scenario(pool, &doc)` → `load(world)`, then `eval_access_check` per
  check, accumulating failures with referent-named context (same failure style as
  `runner::run_scenario`).
- **Scenario file:** `schema-artifact/access-scenarios/epd-bridge-access.yaml` — the full S1–S5 cast
  above. (Onboarding telos prose is shared verbatim with the existing
  `schema-artifact/scenarios/onboarding-cogmap.yaml` seed, as `03_seed` does.)
- **Test:** `crates/temper-next/tests/access_scenario.rs`, gated `#![cfg(feature = "artifact-tests")]`,
  assigned to the **`temper-next-write`** nextest group (it owns and resets the `temper_next`
  namespace to a clean `01`+`02` then loads, and already provisions ONNX). Runs locally via
  `cargo nextest run -p temper-next --features artifact-tests`; no CI job enables it (matches the
  existing write-path tests).
- **JSON-Schema snapshot:** derive `schemars::JsonSchema` (gated on `scenario-schema`) on
  `AccessScenario` and its new sub-structs, and add an `access-scenario.schema.json` snapshot
  alongside the existing scenario-schema snapshot test (`tests/scenario_schema.rs` pattern).
- **sqlx cache:** any new `sqlx::query!`/`query_scalar!` in `access.rs` resolves against the
  `temper_next` namespace — regenerate the per-crate cache with `cargo make prepare-next` after
  writing the loader/evaluator SQL.

## 5. S8 and coverage

- **S8** (capability coherence CHECK: `can_write|can_delete|can_grant ⇒ can_read`) lands as a focused
  test in `access_scenario.rs`: open a savepoint, attempt an invalid `kb_resource_access` insert
  (`can_read=false, can_write=true`), assert the `check_violation`, roll back. Not a declarative check.
- **Coverage map:** S1 → `visible_to`, S2 → `producer_reach`, S3 → `edge_visible_to`, S4 →
  `charter_blocks_visible`, S5 → `cogmaps_share_team`, S8 → unit test. S6 (shape/staleness) and S7
  (entity launch-metadata) are clustering/entity, not access — out of scope.

The hand-written `04_scenarios.sql` stays as the human-readable companion read; this spec makes every
access invariant in it reproducible and CI-checkable.

---

## Components and their boundaries

| Unit | Does | Used by | Depends on |
|---|---|---|---|
| `AccessScenario` model (`access.rs`) | Parse the YAML world + checks | loader, runner, schema snapshot | serde, schemars (gated) |
| `access::load` | Persist the world atomically; return `key → Uuid` | runner | sqlx, `cogmap_genesis`, `01`/`02` schema |
| `AccessCheck` + `eval_access_check` | Evaluate one check against a gate function | runner | the `02_functions.sql` gates |
| `access::run_access_scenario` | Orchestrate load → eval-all | the integration test | the three above |
| `access_scenario.rs` test (+S8) | Drive the scenario; assert; prove S8 constraint | — | `artifact-tests` feature, `temper-next-write` group |

## DDL / schema delta

**None.** The access model is already schema'd in `01_schema.sql` + `02_functions.sql`. This spec adds
only Rust (model, loader, evaluator, test), one YAML scenario, and one JSON-Schema snapshot.

## Out of scope

- **Production team-mechanics convergence** (migrating `crates/temper-api` / `migrations` to teams-DAG
  + capability booleans + `kb_team_cogmaps`) — workstream 6.
- **Scenario *steps* / dynamic access events** (re-grant, revoke, fold mid-scenario) — the access proofs
  are static. A `steps` runbook on the access-scenario kind is a future extension if a dynamic
  invariant needs it (YAGNI now).
- **Clustering invariants** (S6) and **entity metadata** (S7) — not access; already partly covered by
  the onboarding-cogmap scenario.
- **Access-fidelity re-derivation under the lens model** (the `fidelity=centroid_only` degraded read) —
  a separate open in the goal (with workstream 2's later phases), not this proof set.

## Connections

- **Proves:** [`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md)
  (the model being made reproducible) — its S1–S8 worked example.
- **Ports from:** `schema-artifact/03_seed.sql` (the topology) + `schema-artifact/04_scenarios.sql`
  (the invariants, currently manual).
- **Extends the pattern of:** the scenario YAML DSL (`crates/temper-next/src/scenario/`) and its
  artifact-test harness (`temper-next-write` nextest group).
- **Goal:** `substrate-kernel-to-cognitive-map`, workstream 2.
