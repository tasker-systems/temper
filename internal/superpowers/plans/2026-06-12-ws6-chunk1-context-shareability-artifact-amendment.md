# WS6 Chunk 1: Context-Shareability Artifact Amendment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the `kb_team_contexts` + default-personal-team artifact amendment with leak-safety scenario proofs, per the WS6 adjudication spec §2 (`docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md`).

**Architecture:** Pure artifact-side work (the WS2 pattern, PR #129's mold): DDL + SQL-function amendments in `schema-artifact/`, scenario-model/loader extensions in `crates/temper-next/src/scenario/access/`, and a new declarative access-scenario fixture whose checks ARE the leak-safety proof. Context-shares enter visibility exactly where team grants live (`vis_team` / `resources_visible_to`), so producer-intersection leak-safety composes without any new trust path. The default personal team is a trigger (molded on `sync_system_membership`) so solo maps read their own contexts through unchanged intersection mechanics.

**Tech Stack:** Postgres (temper_next artifact namespace), Rust (temper-next crate, sqlx `!`-macros with per-crate offline cache), serde_yaml scenario DSL, cargo-nextest (`temper-next-write` serialized group; artifact tests are LOCAL-ONLY — no CI job enables `artifact-tests`).

---

## WS6 chunk roadmap (context for this and subsequent plans)

Sequenced plannable units. Each later chunk gets its own plan in a future session; the pointers below are that session's pre-work. The binding decisions all live in the adjudication spec — **a moved call reopens the spec, never a local choice.**

| Chunk | Scope | Pre-work pointers / named verifications |
|---|---|---|
| **1 (this plan)** | `kb_team_contexts` + personal-team trigger + leak-safety scenarios + charter-seed §D amendment | — |
| **2: synthesis machinery** | Production new-schema bring-up expressed FROM the artifact; genesis-event synthesis (resource_created + properties + blocks + edges per spec §0/§1/§4/§7/§8); `migration` entity; per-resource hash-parity gate; Neon-branch rehearsal runner; the **in-DB backend-selection config flag** (spec §D, decided 2026-06-13) — a one-row config table + trivial set/swap migration | Verify `temper-goal` edge kind+label against `crates/temper-core/src/types/graph.rs` frontmatter-edge projection (spec §7 named check). **Refinement surfaced in this planning session:** ownership→share synthesis is uniform — a profile-owned context synthesizes a `kb_team_contexts` share to the owner's **personal team**; a team-owned context shares to **that team** (root-team share = the public floor). **Migrations must be strictly additive** — new tables/schema alongside live ones, synthesis explicitly-invoked (never migrate-time); a destructive migration here moves the cutover blocker earlier (spec §D). |
| **3: parity-read harness** | Legacy reads answered identically from the synthesized substrate; rehearsal loop until boring | Spec §D step 1–2; harness compares production read-path outputs (list/show/search/graph) against new-substrate equivalents |
| **4: surface ports** | api (neutral handlers over new schema) → cli/mcp (identifier contract §5: `ResourceRef` collapse, decorated refs, shared resolver in temper-workflow) → ui (ts-rs regen); read-surface floor per §9 (graph→kernel, FTS→Domain-A) | Each surface lands **gated OFF in production** behind the chunk-2 config flag (spec §D — this is what keeps chunk 4 PR-over-PR; an ungated live repoint forces all surfaces at once). `temper-core` shared-type changes (§5) are compile-time atomic — one PR, all callers. Verify zero callers of sync machinery then delete (`sync_service.rs`, `handlers/sync.rs`, `sync_diff_for_device`, `kb_device_sync_state`) — spec §7 plan-time gate |
| **5: cutover** | Write-freeze → final synthesis → parity gate → flip → `legacy` schema + Neon branch; crate extraction last | Spec §D binding sequence |
| *(parallel, own spec)* | Agent information-access read surface (spec §9 successor design unit) | Not a chunk of this roadmap; design-first |

---

## File structure (chunk 1)

- Modify: `schema-artifact/01_schema.sql` — `kb_team_contexts` table (after `kb_team_cogmaps`, line ~193)
- Modify: `schema-artifact/02_functions.sql` — `sync_personal_team()` trigger (after `sync_system_membership`, line ~77); amend `resources_visible_to` (line 88), `vis_team` (line 117), `anchor_readable_by_profile` (line 185)
- Modify: `crates/temper-next/src/ids.rs` — `ContextId` newtype
- Modify: `crates/temper-next/src/payloads.rs` — `AnchorRef::context()` constructor
- Modify: `crates/temper-next/src/events.rs` — `EdgeHome` enum; `RelationshipAssert.home: EdgeHome`
- Modify: `crates/temper-next/src/scenario/runner.rs:229`, `crates/temper-next/src/scenario/loader.rs:144` — wrap existing homes in `EdgeHome::Cogmap`
- Modify: `crates/temper-next/src/scenario/access/model.rs` — `ContextDef`, `ContextShareDef`, `HomeDef::Context{name}`, `EdgeHomeDef`
- Modify: `crates/temper-next/src/scenario/access/loader.rs` — context inserts, team-map refresh, shares, polymorphic edge homes
- Modify: `crates/temper-next/src/replay.rs` — `INPUT_TABLES` order + `kb_team_contexts`
- Modify: `crates/temper-next/tests/access_scenario.rs` — row-count assertions (trigger) + new fixture tests
- Modify: `schema-artifact/access-scenarios/epd-bridge-access.yaml` — edge `home:` tagged form
- Create: `schema-artifact/access-scenarios/context-share-access.yaml` — the new leak-safety fixture
- Modify: `schema-artifact/seeds/temper-convergence.yaml` — framing line 27 (§D supersession)
- Regenerate: `schema-artifact/access-scenarios/access-scenario.schema.json` (UPDATE_SCHEMA=1), `crates/temper-next/.sqlx` (`cargo make prepare-next`)

**Environment for every DB step:** Docker Postgres up (`cargo make docker-up`), ONNX present (dev box has it). Artifact tests run locally only: `cargo nextest run -p temper-next --features artifact-tests`.

**Branch:** `jct/ws6-chunk1-context-shareability` off `jct/ws6-delta-adjudication-spec` (rebase onto main once PR #133 merges).

---

### Task 1: DDL — `kb_team_contexts` + personal-team trigger (TDD via existing row-count test)

**Files:**
- Modify: `crates/temper-next/tests/access_scenario.rs:31-56` (assertions)
- Modify: `schema-artifact/01_schema.sql` (after line 193)
- Modify: `schema-artifact/02_functions.sql` (after line 77)

- [ ] **Step 1: Update `loads_topology_row_counts` to the post-trigger world (failing test)**

In `crates/temper-next/tests/access_scenario.rs`, the epd-bridge world has 6 declared teams and 6 profiles; the personal-team trigger adds one team per profile. Update:

```rust
    // 6 declared + 6 trigger-created personal teams (one per profile).
    let teams: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_teams")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(teams, 12);
    // alice: temper-system root (approved) + epd-team-a + personal-alice => 3 memberships.
    let alice_teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members m JOIN kb_profiles p ON p.id=m.profile_id WHERE p.handle='alice'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alice_teams, 3);
    // nomad (system_access=none) gets ONLY the personal team.
    let nomad_teams: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members m JOIN kb_profiles p ON p.id=m.profile_id WHERE p.handle='nomad'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nomad_teams, 1);
```

Also update the `loaded.teams.len()` assertion comment context if needed — `loaded.teams` is the *declared* map and stays 6 until Task 4's refresh makes it 12; assert `loaded.teams.len() >= 6` for now and tighten in Task 4. (Write `assert!(loaded.teams.len() >= 6);`.)

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-next --features artifact-tests -E 'test(loads_topology_row_counts)'
```
Expected: FAIL — `teams` is 6, assertion wants 12 (trigger doesn't exist yet).

- [ ] **Step 3: Add the table to `schema-artifact/01_schema.sql`** (directly after the `idx_kb_team_cogmaps_team` index, line 193):

```sql
-- NEW (WS6 adjudication §2): context-shareability. Joins a context to 0+ teams —
-- "this team's vis-reach includes this context's resources (and context-homed
-- edges)." Sibling of kb_team_cogmaps in shape; GRANT-like in semantics (inherits
-- DOWN the teams DAG via team_ancestors, exactly as team grants do). Cognitive
-- maps reach workflow content ONLY through producer-intersection over their
-- joined teams — there is no direct map↔context coupling.
CREATE TABLE kb_team_contexts (
    context_id  UUID NOT NULL REFERENCES kb_contexts(id) ON DELETE CASCADE,
    team_id     UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    PRIMARY KEY (context_id, team_id)
);
CREATE INDEX idx_kb_team_contexts_team ON kb_team_contexts(team_id);
```

- [ ] **Step 4: Add the personal-team trigger to `schema-artifact/02_functions.sql`** (directly after the `trg_sync_system_membership` trigger, line 77):

```sql
-- NEW (WS6 adjudication §2): the default personal team — a loopback self-reference
-- so a solo profile's maps read their own contexts through the SAME intersection
-- mechanics (share context → personal team; join map → personal team). No
-- visibility-model special case. Idempotent by slug: replay restores kb_teams
-- BEFORE kb_profiles, so the trigger's insert no-ops against restored rows and
-- the original team ids survive (mirrors the kb_team_members tolerance).
CREATE FUNCTION sync_personal_team()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    v_team uuid;
    v_root uuid;
BEGIN
    INSERT INTO kb_teams (slug, name)
    VALUES ('personal-' || NEW.handle, NEW.display_name || ' (personal)')
    ON CONFLICT (slug) DO NOTHING;
    SELECT id INTO v_team FROM kb_teams WHERE slug = 'personal-' || NEW.handle;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    VALUES (v_team, NEW.id, 'owner'::team_role)
    ON CONFLICT (team_id, profile_id) DO NOTHING;
    SELECT id INTO v_root FROM kb_teams WHERE slug = 'temper-system';
    IF v_root IS NOT NULL THEN
        INSERT INTO kb_teams_parents (child_id, parent_id)
        VALUES (v_team, v_root)
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER trg_sync_personal_team
    AFTER INSERT ON kb_profiles
    FOR EACH ROW EXECUTE FUNCTION sync_personal_team();
```

Note: if the `temper-system` root doesn't exist at profile-insert time, the personal team is created unparented (mirrors `sync_system_membership`'s NULL guard); loaders insert teams first so this only occurs in bare unit fixtures.

- [ ] **Step 5: Run to verify it passes**

```bash
cargo nextest run -p temper-next --features artifact-tests -E 'test(loads_topology_row_counts) or test(proves_all_access_invariants) or test(s8_capability_check_rejects_write_without_read)'
```
Expected: all 3 PASS (each test resets the namespace from amended 01+02).

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/01_schema.sql schema-artifact/02_functions.sql crates/temper-next/tests/access_scenario.rs
git commit -m "WS6 chunk 1: kb_team_contexts DDL + default-personal-team trigger (adjudication §2)"
```

---

### Task 2: Scenario model — contexts, shares, named context homes, polymorphic edge homes

**Files:**
- Modify: `crates/temper-next/src/scenario/access/model.rs`

- [ ] **Step 1: Write the failing parse test** — extend the `YAML` const in `model.rs`'s test module with the new fields, and the assertions in `parses_access_scenario`:

In the `YAML` const, add after the `memberships:` block:
```yaml
  contexts:
    - { name: research }
  context_shares:
    - { context: research, team: epd-team-a }
```
Change resource `d`'s home to the named form and the edge home to the tagged form:
```yaml
    - { key: d, title: "doc: d", origin_uri: "temper://d",
        home: { anchor: context, name: research }, owner: alice,
        grants: [{ to: { anchor: profile, handle: alice }, can_read: true }] }
  edges:
    - { from: c, to: d, kind: leads_to, label: "c->d", home: { anchor: cogmap, name: side-map }, emitter: carol-agent }
```
Add assertions:
```rust
        assert_eq!(s.world.contexts.len(), 1);
        assert_eq!(s.world.context_shares.len(), 1);
        assert!(matches!(
            &s.world.resources[1].home,
            HomeDef::Context { name: Some(n) } if n == "research"
        ));
        assert!(matches!(
            &s.world.edges[0].home,
            EdgeHomeDef::Cogmap { name } if name == "side-map"
        ));
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-next -E 'test(parses_access_scenario)'
```
Expected: FAIL to compile (`contexts` field, `EdgeHomeDef` don't exist).

- [ ] **Step 3: Implement the model changes**

In `AccessWorld` (after `memberships`):
```rust
    #[serde(default)]
    pub contexts: Vec<ContextDef>,
    #[serde(default)]
    pub context_shares: Vec<ContextShareDef>,
```

New defs (after `MembershipDef`):
```rust
/// A named context — a real `kb_contexts` row, the referent for named homes and shares.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ContextDef {
    pub name: String,
}

/// A context-share (`kb_team_contexts`): the team's vis-reach includes the context's
/// resources and context-homed edges. `team` may name a trigger-created personal team
/// (`personal-<handle>`) — the loader refreshes its team map from the DB after profiles load.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ContextShareDef {
    pub context: String, // name in world.contexts
    pub team: String,    // slug (declared or trigger-created)
}
```

`HomeDef::Context` gains an optional name (anonymous form stays valid — a synthetic unshared workspace anchor):
```rust
pub enum HomeDef {
    Cogmap { name: String },
    Context {
        #[serde(default)]
        name: Option<String>,
    },
}
```

`AccessEdgeDef.home` becomes polymorphic (replaces `pub home: String`):
```rust
    pub home: EdgeHomeDef, // cogmap or context home anchor
```
New tagged enum (after `AccessEdgeDef`):
```rust
/// An edge home anchor — `{ anchor: cogmap, name: .. }` or `{ anchor: context, name: .. }`.
#[derive(Debug, Deserialize)]
#[serde(tag = "anchor", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum EdgeHomeDef {
    Cogmap { name: String },
    Context { name: String },
}
```

- [ ] **Step 4: Run to verify the parse test passes** (loader won't compile yet if it referenced `e.home` as String — that's Task 4; if the crate fails to compile, apply Task 4's loader change for `home` resolution first, then return here. To keep commits bite-sized, Steps here only need `cargo nextest run -p temper-next -E 'test(parses_access_scenario)'` green **after Task 4's Step 1**; if you prefer strictly compiling commits, squash Tasks 2–4 into one commit at Task 4's Step 5.)

- [ ] **Step 5: Commit** (or defer to Task 4 Step 5 if the crate doesn't compile standalone)

```bash
git add crates/temper-next/src/scenario/access/model.rs
git commit -m "WS6 chunk 1: access-scenario model gains contexts, context_shares, named context homes, polymorphic edge homes"
```

---

### Task 3: `ContextId`, `AnchorRef::context`, `EdgeHome` in the fire path

**Files:**
- Modify: `crates/temper-next/src/ids.rs`
- Modify: `crates/temper-next/src/payloads.rs:53-66`
- Modify: `crates/temper-next/src/events.rs:85-97, ~253, ~271-290`
- Modify: `crates/temper-next/src/scenario/runner.rs:229`, `crates/temper-next/src/scenario/loader.rs:144`

- [ ] **Step 1: Add `ContextId`** in `ids.rs`, alongside the existing newtypes (same `id_newtype!` macro):

```rust
id_newtype! {
    /// A `kb_contexts.id`.
    ContextId
}
```
(Match the exact invocation style of the sibling newtypes in the file — read one before writing.)

- [ ] **Step 2: Add the constructor** in `payloads.rs` `impl AnchorRef` (after `cogmap`):

```rust
    pub fn context(id: ContextId) -> Self {
        AnchorRef {
            table: AnchorTable::Contexts,
            id: id.uuid(),
        }
    }
```
Add `ContextId` to the existing `use crate::ids::{...}` import.

- [ ] **Step 3: Introduce `EdgeHome` and retype the action** in `events.rs`. Near `SeedAction` (above the enum):

```rust
/// Where an asserted edge homes — polymorphic per the payload's `AnchorRef`
/// (`kb_cogmaps` | `kb_contexts`); the typed fire-path mirror of edge-home polymorphism.
#[derive(Debug, Clone, Copy)]
pub enum EdgeHome {
    Cogmap(CogmapId),
    Context(ContextId),
}

impl EdgeHome {
    fn anchor_ref(self) -> payloads::AnchorRef {
        match self {
            EdgeHome::Cogmap(c) => payloads::AnchorRef::cogmap(c),
            EdgeHome::Context(c) => payloads::AnchorRef::context(c),
        }
    }
}
```

In `SeedAction::RelationshipAssert` change `home: CogmapId` → `home: EdgeHome`. In `fire()`'s `RelationshipAssert` arm (line ~288) change `home: payloads::AnchorRef::cogmap(home)` → `home: home.anchor_ref()`. Import `ContextId` in the `use crate::ids::...` line. (Line ~253 is `RelationshipFold`/other arms — only touch the assert arm's `home`.)

- [ ] **Step 4: Mechanically wrap the two existing call sites**

`crates/temper-next/src/scenario/runner.rs:229` and `crates/temper-next/src/scenario/loader.rs:144`: where `home: <cogmap-id-expr>` is passed, wrap as `home: EdgeHome::Cogmap(<cogmap-id-expr>)`, importing `EdgeHome` from `crate::events`.

- [ ] **Step 5: Verify the crate compiles + lib tests pass**

```bash
cargo nextest run -p temper-next
```
Expected: PASS (ungated unit tests: affinity, cluster, model parse, payload roundtrips). The access loader still passes `CogmapId`-derived homes only.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/ids.rs crates/temper-next/src/payloads.rs crates/temper-next/src/events.rs crates/temper-next/src/scenario/runner.rs crates/temper-next/src/scenario/loader.rs
git commit -m "WS6 chunk 1: ContextId + polymorphic EdgeHome through the fire path"
```

---

### Task 4: Access loader — contexts, team-map refresh, shares, polymorphic homes

**Files:**
- Modify: `crates/temper-next/src/scenario/access/loader.rs`

- [ ] **Step 1: Implement the loader extensions.** After step 5 (memberships, line ~105) and before the placeholder-telos insert, add:

```rust
    // 5b. Contexts — real kb_contexts rows, referents for named homes and shares.
    let mut contexts: HashMap<String, Uuid> = HashMap::new();
    for c in &world.contexts {
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_contexts (name) VALUES ($1) RETURNING id",
            c.name,
        )
        .fetch_one(&mut *tx)
        .await?;
        contexts.insert(c.name.clone(), id);
    }
    // 5c. Refresh the team map from the DB — profile inserts trigger personal teams
    //     (personal-<handle>) that world.teams never declares.
    for row in sqlx::query!("SELECT slug, id FROM kb_teams")
        .fetch_all(&mut *tx)
        .await?
    {
        teams.entry(row.slug).or_insert(row.id);
    }
    // 5d. Context shares (kb_team_contexts).
    for s in &world.context_shares {
        let cid = contexts
            .get(&s.context)
            .with_context(|| format!("share references unknown context {}", s.context))?;
        let tid = teams
            .get(&s.team)
            .with_context(|| format!("share references unknown team {}", s.team))?;
        sqlx::query!(
            "INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1,$2)",
            cid,
            tid,
        )
        .execute(&mut *tx)
        .await?;
    }
```

Home resolution (replace the `HomeDef::Context {}` arm, line 198):
```rust
            HomeDef::Context { name } => (
                "kb_contexts",
                match name {
                    Some(n) => *contexts.get(n).with_context(|| {
                        format!("resource {} homes in unknown context {}", r.key, n)
                    })?,
                    None => Uuid::now_v7(), // anonymous unshared workspace anchor (pre-amendment form)
                },
            ),
```

Edge home resolution (replace the `home` lookup in step 9, line ~257):
```rust
        let home = match &e.home {
            EdgeHomeDef::Cogmap { name } => EdgeHome::Cogmap(CogmapId::from(
                *cogmaps
                    .get(name)
                    .with_context(|| format!("edge homes in unknown cogmap {}", name))?,
            )),
            EdgeHomeDef::Context { name } => EdgeHome::Context(ContextId::from(
                *contexts
                    .get(name)
                    .with_context(|| format!("edge homes in unknown context {}", name))?,
            )),
        };
```
Imports: add `ContextId` to the `crate::ids` use, `EdgeHome` to the `crate::events` use.

`LoadedAccess` gains the contexts map (and return it):
```rust
pub struct LoadedAccess {
    pub profiles: HashMap<String, Uuid>,  // handle -> id
    pub teams: HashMap<String, Uuid>,     // slug -> id (incl. trigger-created personal teams)
    pub contexts: HashMap<String, Uuid>,  // name -> id
    pub cogmaps: HashMap<String, Uuid>,   // name -> id
    pub resources: HashMap<String, Uuid>, // key -> id
}
```

- [ ] **Step 2: Update the epd-bridge fixture's edge home to the tagged form.** In `schema-artifact/access-scenarios/epd-bridge-access.yaml`, the single edge's `home: <name>` becomes `home: { anchor: cogmap, name: <name> }` (read the file for the actual cogmap name; change only the `home` key).

- [ ] **Step 3: Regenerate the access-scenario JSON-Schema snapshot**

```bash
UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema --test scenario_schema
cargo test -p temper-next --features scenario-schema --test scenario_schema
```
Expected: first run rewrites `schema-artifact/access-scenarios/access-scenario.schema.json`; second run PASSES clean.

- [ ] **Step 4: Regenerate the sqlx offline cache** (new `query!` strings against the amended artifact):

```bash
cargo make prepare-next
```
Expected: `crates/temper-next/.sqlx/` gains entries for the three new queries. NEVER `cargo sqlx prepare --workspace` (clobbers per-crate caches).

- [ ] **Step 5: Run the write-group access tests + tighten the Task 1 deferred assertion**

In `access_scenario.rs`, tighten `assert!(loaded.teams.len() >= 6)` → `assert_eq!(loaded.teams.len(), 12);` (declared 6 + refreshed 6 personal).

```bash
cargo nextest run -p temper-next --features artifact-tests -E 'test(loads_topology_row_counts) or test(proves_all_access_invariants)'
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/scenario/access/loader.rs crates/temper-next/src/scenario/access/model.rs crates/temper-next/tests/access_scenario.rs schema-artifact/access-scenarios/epd-bridge-access.yaml schema-artifact/access-scenarios/access-scenario.schema.json crates/temper-next/.sqlx
git commit -m "WS6 chunk 1: access loader — contexts, shares, personal-team refresh, polymorphic edge homes"
```
(Include `model.rs` here if Task 2's commit was deferred.)

---

### Task 5: The leak-safety fixture (failing first — SQL arms don't exist yet)

**Files:**
- Create: `schema-artifact/access-scenarios/context-share-access.yaml`
- Modify: `crates/temper-next/tests/access_scenario.rs`

- [ ] **Step 1: Write the fixture.** Full content:

```yaml
# Context-shareability leak-safety proof (WS6 adjudication §2). Proves: consumer reach
# through a share; producer-intersection with a shared context in the topology (the
# crux: a context shared to ONE of a map's teams is NOT reachable); DAG-down share
# inheritance; the default-personal-team loopback (a solo map reads its owner's
# context with zero declared teams); and the §3 edge-home re-proof with a
# context-homed edge (private edge between public endpoints).
name: context-share-access

world:
  profiles:
    - { handle: alice, display_name: Alice, system_access: approved }
    - { handle: bob,   display_name: Bob,   system_access: approved }
    - { handle: carol, display_name: Carol, system_access: approved }
    - { handle: nomad, display_name: Nomad, system_access: none }
  entities:
    - { name: alice-agent, profile: alice }
    - { name: carol-agent, profile: carol }
  teams:
    - { slug: temper-system, name: Temper System }
    - { slug: team-a, name: Team A, parents: [temper-system] }
    - { slug: team-b, name: Team B, parents: [temper-system] }
    - { slug: team-a-sub, name: Team A Sub, parents: [team-a] }
  memberships:
    - { team: team-a, profile: alice, role: member }
    - { team: team-b, profile: bob, role: member }
  contexts:
    - { name: research }      # shared to team-a
    - { name: carol-notes }   # shared to carol's trigger-created personal team
    - { name: scratch }       # unshared — reachable by nobody but owners
  context_shares:
    - { context: research, team: team-a }
    - { context: carol-notes, team: personal-carol }
  cogmaps:
    - { name: map-a,     teams: [team-a] }
    - { name: map-ab,    teams: [team-a, team-b] }   # intersection probe
    - { name: map-sub,   teams: [team-a-sub] }       # ancestor-share inheritance probe
    - { name: map-carol, teams: [personal-carol] }   # loopback probe
  resources:
    - { key: rdoc, title: "research doc", origin_uri: "temper://rdoc",
        home: { anchor: context, name: research }, owner: alice }
    - { key: rnote, title: "research note", origin_uri: "temper://rnote",
        home: { anchor: context, name: research }, owner: alice }
    - { key: cnote, title: "carol note", origin_uri: "temper://cnote",
        home: { anchor: context, name: carol-notes }, owner: carol }
    - { key: sdoc, title: "scratch doc", origin_uri: "temper://sdoc",
        home: { anchor: context, name: scratch }, owner: alice }
    - { key: pub1, title: "public concept 1", origin_uri: "temper://pub1",
        home: { anchor: context }, owner: alice,
        grants: [{ to: { anchor: team, slug: temper-system }, can_read: true }] }
    - { key: pub2, title: "public concept 2", origin_uri: "temper://pub2",
        home: { anchor: context }, owner: alice,
        grants: [{ to: { anchor: team, slug: temper-system }, can_read: true }] }
  edges:
    # context-homed edge between PUBLIC endpoints — the §3 re-proof: bob reads both
    # endpoints (root grant) but NOT the edge (home context unshared to team-b).
    - { from: pub1, to: pub2, kind: near, label: "pub1~pub2(research)",
        home: { anchor: context, name: research }, emitter: alice-agent }
    # cogmap-homed edge unchanged by the amendment (control).
    - { from: rdoc, to: rnote, kind: leads_to, label: "rdoc->rnote",
        home: { anchor: cogmap, name: map-a }, emitter: alice-agent }

checks:
  # S1 — consumer reach through a context-share
  - { check: visible_to, profile: alice, resource: rdoc, expect: true }    # team-a share
  - { check: visible_to, profile: bob,   resource: rdoc, expect: false }   # no share to team-b
  - { check: visible_to, profile: nomad, resource: rdoc, expect: false }   # no teams at all
  - { check: visible_to, profile: bob,   resource: sdoc, expect: false }   # unshared context
  # S2 — producer-intersection with shared contexts in the topology
  - { check: producer_reach, cogmap: map-a,     resource: rdoc,  expect: true }   # team-a share in vis(team-a)
  - { check: producer_reach, cogmap: map-ab,    resource: rdoc,  expect: false }  # CRUX: team-b lacks the share ⇒ intersection excludes
  - { check: producer_reach, cogmap: map-sub,   resource: rdoc,  expect: true }   # share on ancestor team-a inherits DOWN to team-a-sub
  - { check: producer_reach, cogmap: map-carol, resource: cnote, expect: true }   # LOOPBACK: solo map reads own context via personal team
  - { check: producer_reach, cogmap: map-a,     resource: cnote, expect: false }  # personal share never leaks cross-team
  - { check: producer_reach, cogmap: map-a,     resource: sdoc,  expect: false }  # unshared context unreachable by any map
  # S3 — edge-home protection, re-proven with a context home
  - { check: edge_visible_to, profile: alice, edge: "pub1~pub2(research)", expect: true }
  - { check: edge_visible_to, profile: bob,   edge: "pub1~pub2(research)", expect: false }  # endpoints public, HOME unshared
  - { check: edge_visible_to, profile: alice, edge: "rdoc->rnote", expect: true }           # cogmap-homed control
```

- [ ] **Step 2: Add the test entries** in `crates/temper-next/tests/access_scenario.rs`:

```rust
const CONTEXT_SHARE_SCENARIO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema-artifact/access-scenarios/context-share-access.yaml"
);

fn load_context_share_yaml() -> AccessScenario {
    serde_yaml::from_str(&std::fs::read_to_string(CONTEXT_SHARE_SCENARIO).unwrap()).unwrap()
}

#[tokio::test]
async fn proves_context_share_invariants() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    access::run_access_scenario(&pool, &load_context_share_yaml())
        .await
        .expect("all context-share leak-safety checks pass");
}
```

- [ ] **Step 3: Run to verify it fails for the right reason**

```bash
cargo nextest run -p temper-next --features artifact-tests -E 'test(proves_context_share_invariants)'
```
Expected: FAIL on the FIRST `visible_to: alice / rdoc = false, expected true` check (the world loads — DDL exists — but `resources_visible_to`/`vis_team` have no context-share arm yet). A failure earlier than check evaluation (load error) means Tasks 2–4 are incomplete — fix there first.

- [ ] **Step 4: Commit the red fixture**

```bash
git add schema-artifact/access-scenarios/context-share-access.yaml crates/temper-next/tests/access_scenario.rs
git commit -m "WS6 chunk 1: context-share leak-safety fixture (red — SQL arms land next)"
```

---

### Task 6: SQL function amendments — the arms that make the fixture pass

**Files:**
- Modify: `schema-artifact/02_functions.sql:88-107` (`resources_visible_to`), `:117-123` (`vis_team`), `:185-192` (`anchor_readable_by_profile`)

- [ ] **Step 1: Amend `vis_team`** — context-shares enter team visibility exactly where team grants live (DAG-down via `team_ancestors`):

```sql
-- vis(T): team T's visibility. TEAM-anchored grants on T or its ancestors, plus
-- resources homed in contexts SHARED to T or its ancestors (WS6 adjudication §2 —
-- context-shares are grant-like and inherit DOWN identically). Profile-anchored
-- grants NEVER enter vis(T) — the A2 leak-safety invariant (access §4).
CREATE FUNCTION vis_team(p_team uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT ra.resource_id
    FROM team_ancestors(p_team) a
    JOIN kb_resource_access ra
      ON ra.anchor_table = 'kb_teams' AND ra.anchor_id = a.team_id AND ra.can_read
    UNION
    SELECT h.resource_id
    FROM team_ancestors(p_team) a
    JOIN kb_team_contexts tc ON tc.team_id = a.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id;
$$;
```
(Replace the whole function body in place; keep its position in the file.)

- [ ] **Step 2: Amend `resources_visible_to`** — same arm on the consumer axis (append a fourth UNION branch using the existing `reachable_teams` CTE):

```sql
    UNION
    -- context-share: resources homed in a context shared to a reachable team (WS6 §2)
    SELECT h.resource_id
    FROM kb_team_contexts tc
    JOIN reachable_teams rt ON tc.team_id = rt.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id;
```
(Also update the function's doc comment: add "or belong to a context shared to a reachable team".)

- [ ] **Step 3: Make the context arm of `anchor_readable_by_profile` real** (replaces `WHEN 'kb_contexts' THEN true`):

```sql
-- Can a Profile read a polymorphic anchor (an edge/region home)?
--   cogmap  → cogmap_readable_by_profile
--   context → ∃ a context-share on a reachable (self-or-ancestor) team (WS6 §2 —
--             replaces the pre-amendment 'always true' simplification; a
--             context-homed edge is gated exactly like the context's resources).
CREATE FUNCTION anchor_readable_by_profile(p_profile uuid, p_anchor_table text, p_anchor_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_cogmaps'  THEN cogmap_readable_by_profile(p_profile, p_anchor_id)
        WHEN 'kb_contexts' THEN EXISTS (
            SELECT 1
            FROM profile_effective_teams(p_profile) e
            CROSS JOIN LATERAL team_ancestors(e.team_id) a
            JOIN kb_team_contexts tc ON tc.team_id = a.team_id
            WHERE tc.context_id = p_anchor_id
        )
        ELSE false
    END;
$$;
```
Note: `resources_accessible_to_cogmap` needs **no change** — it composes `vis_team` per joined team, so context-shares flow through the intersection automatically. That composition IS the design.

- [ ] **Step 4: Run the fixture to green, then the whole access file**

```bash
cargo nextest run -p temper-next --features artifact-tests -E 'test(proves_context_share_invariants)'
cargo nextest run -p temper-next --features artifact-tests -E 'binary(access_scenario)'
```
Expected: all PASS. If `proves_all_access_invariants` (epd-bridge) regresses, the likely cause is a check that depended on the old `'kb_contexts' → true` simplification — inspect which check, and verify against the fixture's intent before changing anything (escalate, don't soften).

- [ ] **Step 5: Regenerate the sqlx cache (function bodies changed)**

```bash
cargo make prepare-next
```

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/.sqlx
git commit -m "WS6 chunk 1: context-share arms in vis_team / resources_visible_to / anchor_readable_by_profile (fixture green)"
```

---

### Task 7: Replay safety — input order + `kb_team_contexts`

**Files:**
- Modify: `crates/temper-next/src/replay.rs:71-81`

- [ ] **Step 1: Reorder `INPUT_TABLES` and add the share table.** Teams must restore BEFORE profiles so the personal-team trigger's idempotent insert no-ops against the restored originals (preserving original team ids for the membership restore):

```rust
const INPUT_TABLES: &[&str] = &[
    "kb_teams",          // BEFORE profiles: the personal-team trigger no-ops by slug
    "kb_teams_parents",  // against restored rows, keeping original team ids intact
    "kb_profiles",
    "kb_entities",
    "kb_team_members",
    "kb_contexts",
    "kb_team_contexts",
    "kb_topics",
    "kb_event_types",
    "kb_events",
];
```
Ordering rationale (verify against the file, don't trust this prose blindly): `kb_team_contexts` must restore after both `kb_contexts` and `kb_teams` (FKs) — the position shown satisfies both. With teams-and-parents restored first, the personal-team trigger's inserts (fired during the subsequent profile restore) all hit existing rows and no-op via the trigger's own `ON CONFLICT DO NOTHING` clauses; the `kb_team_members` tolerance already in `restore_table` (line 181-183) covers the membership rows both triggers insert. `restore_table` itself needs no change.

- [ ] **Step 2: Run the replay proof**

```bash
cargo nextest run -p temper-next --features artifact-tests -E 'test(/replay/)'
```
Expected: PASS (`replay_roundtrip` byte-identical). If it fails on a kb_teams unique violation, re-check the order actually landed; if it fails on a projection diff, STOP — that means the trigger mutated projected state, which must not happen (escalate).

- [ ] **Step 3: Commit**

```bash
git add crates/temper-next/src/replay.rs
git commit -m "WS6 chunk 1: replay inputs — teams before profiles, kb_team_contexts restored"
```

---

### Task 8: Charter-seed §D amendment

**Files:**
- Modify: `schema-artifact/seeds/temper-convergence.yaml:27`

- [ ] **Step 1: Replace the dual-write framing line** (the §D supersession the adjudication spec mandates). Line 27 currently reads:

```yaml
      - "Cutover runs one surface at a time — api, then cli/mcp, then ui — behind a parity-read harness, with only a short dual-write window; crate extraction comes last, against a stable schema."
```
Replace with:
```yaml
      - "Cutover is a single rehearsed hard cut: surfaces are ported in order (api, then cli/mcp, then ui) against a re-synthesizable rehearsal substrate behind a parity-read harness, with no dual-write window; crate extraction comes last, against a stable schema."
```

- [ ] **Step 2: Run the corpus proofs over the changed seed**

```bash
cargo nextest run -p temper-next --features artifact-tests -E 'test(/corpus|sweep|seed_load/)'
```
Expected: PASS (sweep asserts schema-validity/loads/roundtrip, not exact charter hashes; the temper-convergence smoke runbook asserts non-degenerate + reproducible regions).

- [ ] **Step 3: Commit**

```bash
git add schema-artifact/seeds/temper-convergence.yaml
git commit -m "WS6 chunk 1: charter seed — hard-cut framing supersedes dual-write (adjudication §D)"
```

---

### Task 9: Full verification sweep

**Files:** none (verification only; fix-forward anything it surfaces)

- [ ] **Step 1: Full write-path artifact group** (LOCAL-ONLY proof — no CI job runs these):

```bash
cargo nextest run -p temper-next --features artifact-tests
```
Expected: all PASS. The personal-team trigger touches every world-loading test — failures here are most likely team-count assertions in other fixtures; update assertions to the trigger reality (declared + one-per-profile), never weaken checks.

- [ ] **Step 2: Legacy read-path suite** (separate feature; needs 01+02+03_seed loaded — `03_seed.sql` inserts profiles, so the trigger fires during seed load):

```bash
cargo nextest run -p temper-next --features artifact-tests-legacy
```
Expected: PASS. If `03_seed.sql` collides with trigger-created rows (slug conflict on a `personal-*` team it happens to declare, or count assertions), amend `03_seed.sql` minimally — it is M2-retirement-bound.

- [ ] **Step 3: Quality gates + ungated tests**

```bash
cargo make check
cargo nextest run -p temper-next
```
Expected: clean. `cargo make check` runs SQLX_OFFLINE — the honest probe of the regenerated `.sqlx` cache.

- [ ] **Step 4: Commit anything the sweep fixed, then push**

```bash
git push -u origin jct/ws6-chunk1-context-shareability
```

---

## Self-review notes (done at write time)

- **Spec §2 coverage:** DDL (Task 1), share semantics in all three gate functions (Task 6), producer-intersection composition unchanged-by-design (Task 6 note), default personal team (Task 1), context-homed edge re-proof (Task 5 fixture), loopback proof (Task 5), DAG-down inheritance proof (Task 5), charter §D amendment (Task 8). Open residue NOT built (per spec): share capability shape (read vs contribute) — shares are read-reach only here; map-home-in-context stays disallowed (no change needed — `HomeDef` for cogmaps is unchanged).
- **Known judgment calls baked in:** anonymous context homes stay legal (synthetic uuid, unreadable home — strictly tighter than before); `anchor_readable_by_profile`'s context arm flips from `true` to share-gated, which is the adjudicated semantics — watch epd-bridge for reliance on the old simplification (Task 6 Step 4 names the escalation).
- **Type consistency:** `ContextId` (Task 3) is used by `EdgeHome::Context` (Task 3) and the loader (Task 4); `EdgeHomeDef` (Task 2) resolves to `EdgeHome` (Task 4); `LoadedAccess.contexts` (Task 4) feeds nothing else yet (checks resolve resources/edges by key/label — no new check kinds, per the established "existing checks suffice" precedent).
