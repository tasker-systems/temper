# L0 Kernel Cognitive Map — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Birth the L0 kernel "what is temper" cognitive map deterministically at boot — the public, root-team-joined system-default cogmap — through the same genesis SQL functions every map uses, via a new canonical-seed migration.

**Architecture:** A new immutable migration (`migrations/20260625000001_l0_kernel_cogmap.sql`) runs through the existing `sqlx::migrate!` MIGRATOR at boot. It (1) creates the `temper-system` root team — a pre-existing latent gap (functions reference it; production never created it), (2) calls `cogmap_genesis(payload, content, emitter)` to seed the L0 cogmap with a minimal empty-charter telos under the `system` actor, and (3) joins L0 to the root team. The rich charter *content* (prose blocks, facets, edges) is a separate deferred authoring task — L0 is born content-light (empty `blocks` array → `_project_blocks` no-ops → no embeddings needed in a migration). Everything is idempotent (guard by name/slug) and event-sourced (one `cogmap_seeded` event → replay-safe).

**Tech Stack:** PostgreSQL (plpgsql SQL functions), sqlx migrations, Rust integration tests via `#[sqlx::test]` (cargo-nextest).

## Global Constraints

- **Migrations are immutable once shipped.** `migrations/2026062400000{1,2,3}_*.sql` are live on temperkb.io — NEVER edit them. L0 ships as a NEW migration file.
- **Additive-only-on-`main`.** This migration only INSERTs data (a team, a cogmap, a join row) — no schema/DDL changes, no column adds. Safe for auto-deploy to live instances.
- **Idempotent.** The migration must be safe to (logically) re-run: guard every insert with `ON CONFLICT DO NOTHING` or `WHERE NOT EXISTS` by stable natural key (team `slug`, cogmap `name`). `cogmap_genesis` is NOT internally idempotent, so its call is wrapped in a `NOT EXISTS` guard.
- **Same genesis functions.** L0 is born via `cogmap_genesis` / (later) `facet_set` / `relationship_assert` — the exact SQL functions every map's genesis uses. No special-cased inserts into `kb_cogmaps`/`kb_resources` for the genesis itself.
- **System actor is the emitter.** Events are attributed to the seeded `system` entity: `(SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id WHERE p.handle = 'system' AND e.name = 'system')`.
- **Reserved L0 UUIDs (decision — flag for reviewer):** L0 cogmap = `00000000-0000-0000-0005-000000000001`; L0 telos resource = `00000000-0000-0000-0005-000000000002`. Fixed literals (not `uuid_generate_v7()`) so future migrations and code reference L0 deterministically. Follows the existing reserved-UUID precedent (the `0003`-group context id). Postgres `UUID` accepts any 128-bit value (no version enforcement).
- **Tests:** integration tests live in `crates/temper-api/tests/` and use `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` (each test gets a fresh migrated DB). Run with `cargo nextest run -p temper-api --features test-db <name>`. Files with `#[sqlx::test]` MUST start with `#![cfg(feature = "test-db")]`.
- Run `cargo make check` before every commit.

---

## File Structure

- **Create:** `migrations/20260625000001_l0_kernel_cogmap.sql` — the L0 seed (root team + cogmap_genesis + team-join). One responsibility: deterministically birth L0.
- **Create:** `crates/temper-api/tests/l0_kernel_cogmap_test.rs` — integration tests proving L0 exists, is correctly homed/joined, is event-sourced, idempotent, and behaves under the access functions.
- **Modify:** `CLAUDE.md` — a short note on the L0 reserved UUIDs + "evolve L0 via additive migrations" pattern (so future sessions know).

---

### Task 1: The L0 seed migration (root team + cogmap genesis + join)

**Files:**
- Create: `migrations/20260625000001_l0_kernel_cogmap.sql`
- Test: `crates/temper-api/tests/l0_kernel_cogmap_test.rs`

**Interfaces:**
- Consumes (existing, verified): `cogmap_genesis(p_payload jsonb, p_content jsonb, p_emitter uuid) RETURNS TABLE(cogmap_id uuid, telos_resource_id uuid)` [migrations/20260624000002_canonical_functions.sql:701]; the `system` profile/entity + `kb_system_settings` + `cogmap_seeded` event type seeded by `20260624000003_canonical_seed.sql`; `kb_teams(slug, name)` with a defaulted `id` (the test fixture inserts without `id`).
- Produces: a `kb_cogmaps` row named `system-default` with id `00000000-0000-0000-0005-000000000001`, telos resource id `00000000-0000-0000-0005-000000000002`; a `kb_teams` row slug `temper-system`; a `kb_team_cogmaps` join; one `cogmap_seeded` event emitted by the system entity.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/l0_kernel_cogmap_test.rs`:

```rust
#![cfg(feature = "test-db")]
//! L0 kernel cognitive map: the public, root-team-joined system-default cogmap,
//! born deterministically by migration 20260625000001 via cogmap_genesis.

use sqlx::PgPool;
use uuid::Uuid;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);
const L0_TELOS: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000002);

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_cogmap_is_born_at_migration(pool: PgPool) {
    // The L0 cogmap exists with the reserved id, the canonical name, and its telos.
    let (name, telos): (String, Uuid) =
        sqlx::query_as("SELECT name, telos_resource_id FROM kb_cogmaps WHERE id = $1")
            .bind(L0_COGMAP)
            .fetch_one(&pool)
            .await
            .expect("L0 cogmap must exist after migrations");
    assert_eq!(name, "system-default");
    assert_eq!(telos, L0_TELOS);

    // Its telos resource exists and is stamped doc_type = cogmap_charter (genesis does this).
    let (title,): (String,) =
        sqlx::query_as("SELECT title FROM kb_resources WHERE id = $1")
            .bind(L0_TELOS)
            .fetch_one(&pool)
            .await
            .expect("L0 telos resource must exist");
    assert_eq!(title, "What Temper Is");

    let doc_type: serde_json::Value = sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
         WHERE owner_table = 'kb_resources' AND owner_id = $1 AND property_key = 'doc_type'",
    )
    .bind(L0_TELOS)
    .fetch_one(&pool)
    .await
    .expect("L0 telos must have a doc_type property");
    assert_eq!(doc_type, serde_json::json!("cogmap_charter"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db l0_cogmap_is_born_at_migration`
Expected: FAIL — `L0 cogmap must exist after migrations` (the migration does not exist yet, so no row is found).

- [ ] **Step 3: Write the migration**

Create `migrations/20260625000001_l0_kernel_cogmap.sql`:

```sql
-- L0 — the kernel "what is temper" cognitive map (cognitive-map agent-invocation architecture,
-- 2026-06-25 spec). The public, root-team-joined system-default cogmap, born deterministically
-- here via the SAME genesis SQL functions every map uses. Born content-light: a minimal empty-charter
-- telos (blocks=[] → _project_blocks no-ops → no embeddings needed in a migration). The rich charter
-- content (prose, facets, edges) is a separate deferred authoring task via the real ingest path.
-- Idempotent: guarded by natural key so a logical re-run is a no-op. Additive (data only).

-- 1. The root team `temper-system`. Canonical triggers/functions REFERENCE
--    `kb_teams WHERE slug = 'temper-system'` (e.g. 20260624000002_canonical_functions.sql:63,102)
--    but production migrations never created it — a latent gap L0 closes. id is defaulted (the test
--    fixture inserts slug/name only). Idempotent on the UNIQUE slug.
INSERT INTO kb_teams (slug, name)
VALUES ('temper-system', 'Temper System')
ON CONFLICT (slug) DO NOTHING;

-- 2. L0 itself, via cogmap_genesis under the system actor. Reserved fixed ids so future migrations
--    and code reference L0 deterministically. Empty telos.blocks (content-light birth). p_content is
--    '{}' (no chunks ⇒ no sidecar needed). Guarded by name so the genesis runs exactly once.
DO $l0$
DECLARE
    v_emitter uuid := (SELECT e.id FROM kb_entities e
                         JOIN kb_profiles p ON p.id = e.profile_id
                        WHERE p.handle = 'system' AND e.name = 'system');
    v_owner   uuid := (SELECT id FROM kb_profiles WHERE handle = 'system');
BEGIN
    IF NOT EXISTS (SELECT 1 FROM kb_cogmaps WHERE id = '00000000-0000-0000-0005-000000000001') THEN
        PERFORM cogmap_genesis(
            jsonb_build_object(
                'cogmap_id',        '00000000-0000-0000-0005-000000000001',
                'name',             'system-default',
                'owner_profile_id', v_owner,
                'telos', jsonb_build_object(
                    'resource_id', '00000000-0000-0000-0005-000000000002',
                    'title',       'What Temper Is',
                    'origin_uri',  'temper://system/what-is-temper',
                    'blocks',      '[]'::jsonb
                )
            ),
            '{}'::jsonb,   -- content sidecar: empty (no chunks)
            v_emitter
        );
    END IF;
END
$l0$;

-- 3. Join L0 to the root team (public cognitive-map home, spec §8). Idempotent on the join's identity.
INSERT INTO kb_team_cogmaps (cogmap_id, team_id)
SELECT '00000000-0000-0000-0005-000000000001', t.id
  FROM kb_teams t
 WHERE t.slug = 'temper-system'
   AND NOT EXISTS (
       SELECT 1 FROM kb_team_cogmaps tc
        WHERE tc.cogmap_id = '00000000-0000-0000-0005-000000000001' AND tc.team_id = t.id
   );
```

> **Note for implementer:** confirm the `kb_team_cogmaps` column names (`cogmap_id`, `team_id`) and the `kb_teams` defaulted `id` against `migrations/20260624000001_canonical_schema.sql` before running — the test fixture `crates/temper-next/tests/fixtures/03_seed.sql:66,124` uses exactly these. If `kb_team_cogmaps` has a different unique constraint, adjust the `NOT EXISTS` guard to match it.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db l0_cogmap_is_born_at_migration`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add migrations/20260625000001_l0_kernel_cogmap.sql crates/temper-api/tests/l0_kernel_cogmap_test.rs
git commit -m "feat(l0): birth the kernel system-default cogmap via canonical-seed migration

Creates the temper-system root team (closing a latent production gap) and the
public system-default cogmap L0 ('what is temper') via cogmap_genesis under the
system actor, content-light (empty telos), idempotent + event-sourced.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Prove L0 is event-sourced, idempotent, and well-formed under the access model

**Files:**
- Modify: `crates/temper-api/tests/l0_kernel_cogmap_test.rs` (add tests; no migration change)

**Interfaces:**
- Consumes: `resources_accessible_to_cogmap(p_cogmap uuid) RETURNS TABLE(resource_id uuid)` [20260624000002_canonical_functions.sql:222]; `cogmaps_share_a_team(p_cogmap_a uuid, p_cogmap_b uuid) RETURNS boolean` [:323]; `kb_events` / `kb_event_types` (a `cogmap_seeded` event row); `kb_resource_homes` (L0 telos homed in `kb_cogmaps`).
- Produces: nothing new (verification only).

- [ ] **Step 1: Write the failing tests**

Append to `crates/temper-api/tests/l0_kernel_cogmap_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_is_event_sourced_and_homed(pool: PgPool) {
    // Exactly one cogmap_seeded event, emitted by the system entity, producing L0.
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'cogmap_seeded' \
            AND ev.producing_anchor_table = 'kb_cogmaps' \
            AND ev.producing_anchor_id = $1",
    )
    .bind(L0_COGMAP)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "L0 must be born via exactly one cogmap_seeded event");

    // The telos resource is homed in L0 (anchor_table = kb_cogmaps).
    let (anchor_table, anchor_id): (String, Uuid) = sqlx::query_as(
        "SELECT anchor_table, anchor_id FROM kb_resource_homes WHERE resource_id = $1",
    )
    .bind(L0_TELOS)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(anchor_table, "kb_cogmaps");
    assert_eq!(anchor_id, L0_COGMAP);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_joined_only_to_root_team(pool: PgPool) {
    // L0 is joined to exactly one team, and that team is temper-system.
    let slugs: Vec<String> = sqlx::query_scalar(
        "SELECT t.slug FROM kb_team_cogmaps tc JOIN kb_teams t ON t.id = tc.team_id \
          WHERE tc.cogmap_id = $1 ORDER BY t.slug",
    )
    .bind(L0_COGMAP)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(slugs, vec!["temper-system".to_string()]);

    // cogmaps_share_a_team is reflexive for L0 (sanity: the access predicate sees the join).
    let shares: bool =
        sqlx::query_scalar("SELECT cogmaps_share_a_team($1, $1)")
            .bind(L0_COGMAP)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(shares, "L0 must share a team with itself once joined to temper-system");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_genesis_guard_is_idempotent(pool: PgPool) {
    // Re-running the guarded genesis block must NOT create a second L0 cogmap or a second event.
    // (Simulates a logical re-application of the seed's body.)
    sqlx::query(
        "DO $$ DECLARE v_emitter uuid := (SELECT e.id FROM kb_entities e \
              JOIN kb_profiles p ON p.id = e.profile_id \
             WHERE p.handle='system' AND e.name='system'); \
            v_owner uuid := (SELECT id FROM kb_profiles WHERE handle='system'); \
         BEGIN \
            IF NOT EXISTS (SELECT 1 FROM kb_cogmaps WHERE id='00000000-0000-0000-0005-000000000001') THEN \
               PERFORM cogmap_genesis( \
                 jsonb_build_object('cogmap_id','00000000-0000-0000-0005-000000000001', \
                   'name','system-default','owner_profile_id',v_owner, \
                   'telos', jsonb_build_object('resource_id','00000000-0000-0000-0005-000000000002', \
                     'title','What Temper Is','origin_uri','temper://system/what-is-temper', \
                     'blocks','[]'::jsonb)), '{}'::jsonb, v_emitter); \
            END IF; \
         END $$;",
    )
    .execute(&pool)
    .await
    .unwrap();

    let cogmaps: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_cogmaps WHERE id = $1")
            .bind(L0_COGMAP)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(cogmaps, 1, "re-running the guarded genesis must not duplicate L0");
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo nextest run -p temper-api --features test-db l0_`
Expected: all `l0_*` tests PASS (Task 1 + Task 2). The idempotency test passes because the `NOT EXISTS` guard short-circuits the second genesis call.

> If `l0_is_event_sourced_and_homed` fails on the event count, check whether `_recompute_resource_body_hash` on a zero-chunk resource raised — inspect the genesis path in Task 1 rather than weakening the assertion. Escalate (do not soften the test) if genesis cannot seed an empty-charter telos; that would be a real finding about `cogmap_genesis`.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/l0_kernel_cogmap_test.rs
git commit -m "test(l0): event-sourced birth, root-team join, and genesis idempotency

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Document the L0 reserved UUIDs and the evolve-via-migration pattern

**Files:**
- Modify: `CLAUDE.md` (add a short subsection under an appropriate Key Patterns area)

**Interfaces:**
- Consumes: nothing.
- Produces: durable guidance so future sessions know L0's reserved ids and how L0 evolves.

- [ ] **Step 1: Add the note to CLAUDE.md**

Add this paragraph to `CLAUDE.md` (in the Key Patterns section, after the cloud-operations bullets):

```markdown
- **L0 kernel cognitive map (`system-default`)** — the public, root-team-joined kernel "what is temper"
  cogmap, born deterministically by migration `20260625000001_l0_kernel_cogmap.sql` via `cogmap_genesis`
  under the `system` actor. Reserved ids: cogmap `00000000-0000-0000-0005-000000000001`, telos resource
  `00000000-0000-0000-0005-000000000002`; root team slug `temper-system`. L0 is a *living* map but
  **release/operator-governed, not operationally-stewarded** — it evolves by shipping **new additive
  migrations** that call the same mutation functions (`facet_set`/`relationship_assert`/`block_mutated`)
  against L0's reserved id (never by editing the birth migration, which is immutable). Its charter
  declares ambient steward wake = never. See
  `docs/superpowers/specs/2026-06-25-cognitive-map-agent-invocation-architecture-design.md`.
```

- [ ] **Step 2: Verify the file is coherent**

Run: `git diff CLAUDE.md`
Expected: a single clean paragraph addition, no broken markdown.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(l0): record L0 reserved ids + evolve-via-additive-migration pattern

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage (L0 tier of the 2026-06-25 invocation-architecture spec):**
- "L0 = system-default cogmap given content, public, root team" → Task 1 (cogmap `system-default`, joined to `temper-system`). ✓
- "Born deterministically through the same genesis functions" → Task 1 (`cogmap_genesis` call). ✓
- "SQL-native seed migration (not the test-only Rust loader)" → Task 1 (migration file). ✓
- "Closes the latent root-team gap" → Task 1 (root team insert). ✓
- "Idempotent / replay-safe (event-sourced)" → Task 2 (idempotency + single `cogmap_seeded` event). ✓
- "Evolve via additive migrations; ambient wake = never" → Task 3 (documented). The `facet_set`/edge content and the wake-policy *mechanism* are correctly OUT of scope here (deferred to the content-authoring task and the steward-queue plan respectively). ✓
- Deferred-and-correctly-absent: rich charter content (embeddings), the `stewardship_requested`/queue, the `DeploymentProfile` ceiling, `kb_system_settings.steward_scheduler`. These belong to later plans (steward queue, L1). ✓

**2. Placeholder scan:** No TBD/TODO. Every step has runnable SQL/Rust and exact commands. The two implementer "Note"s point at concrete verification actions (confirm column names; escalate on genesis failure), not vague instructions. ✓

**3. Type/name consistency:** `L0_COGMAP` / `L0_TELOS` constants reused across Tasks 1–2; the cogmap name `system-default`, telos title `What Temper Is`, team slug `temper-system`, and origin_uri `temper://system/what-is-temper` are identical in the migration and every test. Reserved UUIDs identical in SQL literals and the Rust `Uuid::from_u128` constants. ✓

**Open verification the implementer MUST do (not a placeholder — a real grounding step):** confirm `kb_team_cogmaps` column names + unique constraint and that `kb_teams.id` is defaulted, against `migrations/20260624000001_canonical_schema.sql`. The plan's values mirror the test fixture `03_seed.sql`, but the canonical schema is authoritative.
