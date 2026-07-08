//! Shared setup helpers for the temper-substrate artifact tests.
//!
//! Tests use `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` to receive an isolated
//! ephemeral database with the canonical schema already applied. Each test owns its ephemeral
//! database — no serialization group and no shared namespace teardown needed.
//!
//! `reset_schema` is provided for tests that need to drop and re-apply the baseline within
//! a single test run (e.g. snapshot→replay phases). All other setup goes through the
//! `MIGRATOR`-driven ephemeral DB.

#![allow(dead_code)]

/// Reset the substrate schema IN THE CURRENT DATABASE to a clean, unseeded `01+02` baseline.
/// Use as a test's first line when the seed from `MIGRATOR` would perturb a global count or a
/// replay/projection diff, or to re-clean between snapshot→replay phases.
pub async fn reset_schema(pool: &sqlx::PgPool) {
    use sqlx::Executor;
    // Rebuild the substrate schema by applying every migration in version order, then empty all data
    // tables. This is reset_schema's contract: a complete-schema, SEED-FREE baseline the tests seed via
    // bootseed::seed_system and repopulate via ledger replay. Applying the whole chain (vs a hand-copied
    // file list) keeps it from drifting — the old 2-file list rejected scenarios once a later migration
    // added a param to a canonical mutation function (block_mutate et al.). TRUNCATE (not a partial
    // "structural-only" apply) keeps the seed-free contract: the seed migrations (canonical_seed,
    // l0_kernel) populate kb_event_types / the system actor / L0 rows, which would otherwise collide with
    // each test's bootseed + replay re-seeding (replay re-inserts event types with no ON CONFLICT).
    //
    // EXCEPTION: `auto_join_team_generalization` is skipped. It CREATE-OR-REPLACEs
    // `sync_system_membership` from the slug-based temper-system root join to a flag-based
    // (`auto_join_role`) one. The access-scenario tests here (and the access loader) are written for the
    // slug-based root join (alice joins by approval, nomad excluded); the generalized flag+mode trigger
    // is covered directly by `auto_join_team.rs` (full `MIGRATOR`, fresh DB). Skipping it on this baseline
    // keeps both worlds testable. (Applied via the filesystem rather than `MIGRATOR.run` so the one
    // migration can be excluded; ordering is by version-prefixed filename.)
    //
    // `cogmap_write_tightening` is skipped TOO: it belongs to the post-auto-join world (its backfill
    // references `kb_teams.auto_join_role`, the column the excluded migration adds), so it cannot apply
    // on this pre-auto-join baseline. It changes only `cogmap_authorable_by_profile` (cogmap WRITE) — an
    // axis this scenario never probes (its write checks are `can_modify_resource` on resources, S6) — and
    // is exercised on the full `MIGRATOR` by the temper-api + e2e tiers.
    pool.execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .expect("drop/recreate public schema");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let mut migrations: Vec<String> = std::fs::read_dir(format!("{root}/migrations"))
        .expect("read migrations dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            n.ends_with(".sql")
                && !n.contains("auto_join_team_generalization")
                && !n.contains("cogmap_write_tightening")
        })
        .collect();
    migrations.sort();
    for f in &migrations {
        let sql = std::fs::read_to_string(format!("{root}/migrations/{f}"))
            .unwrap_or_else(|e| panic!("read migration {f}: {e}"));
        pool.execute(sql.as_str())
            .await
            .unwrap_or_else(|e| panic!("apply migration {f}: {e}"));
    }
    pool.execute(
        "DO $$ DECLARE r record; BEGIN \
           FOR r IN SELECT tablename FROM pg_tables \
                     WHERE schemaname = 'public' AND tablename LIKE 'kb\\_%' \
           LOOP EXECUTE 'TRUNCATE TABLE ' || quote_ident(r.tablename) || ' RESTART IDENTITY CASCADE'; \
           END LOOP; END $$;",
    )
    .await
    .expect("truncate kb_ data tables to a seed-free baseline");
}

/// Fire a cogmap genesis + one `resource_create` homed in it, whose single chunk's sidecar entry
/// carries `header_path` + `heading_depth`. Returns the created resource's uuid. Uses the boot-seeded
/// canonical `system` profile/entity as owner+emitter, so call after `bootseed::seed_system`.
pub async fn fire_resource_with_headed_chunk(
    pool: &sqlx::PgPool,
    header_path: &str,
    heading_depth: i16,
) -> uuid::Uuid {
    use temper_substrate::content::{PreparedBlock, PreparedChunk};
    use temper_substrate::events::{fire, SeedAction};
    use temper_substrate::ids::{BlockId, ChunkId, EntityId, ProfileId};
    use uuid::Uuid;

    // One prepared block with a single chunk; a non-degenerate 768-d unit embedding keeps the cosine
    // HNSW index well-defined. `header_path`/`heading_depth` ride the chunk into the sidecar.
    fn one_chunk_block(
        content: &str,
        header_path: Option<String>,
        heading_depth: Option<i16>,
    ) -> PreparedBlock {
        let mut embedding = vec![0.0_f32; 768];
        embedding[0] = 1.0;
        PreparedBlock {
            incorporated: vec![],
            block_id: BlockId::from(Uuid::now_v7()),
            seq: 0,
            role: None,
            chunks: vec![PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index: 0,
                content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
                content: content.to_string(),
                embedding: Some(embedding),
                header_path,
                heading_depth,
            }],
        }
    }

    // The boot-seeded canonical system actor (profile + entity) — owner + emitter for both fires.
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    let owner = ProfileId::from(profile);
    let emitter = EntityId::from(entity);

    // Genesis a cogmap to home the resource into (its charter block has no production headings).
    let charter = vec![one_chunk_block("charter statement", None, None)];
    let mut tx = pool.begin().await.unwrap();
    let fired = fire(
        &mut tx,
        SeedAction::CogmapGenesis {
            name: "heading-carry-cogmap",
            telos_title: "Heading Carry",
            charter: &charter,
            cogmap_id: None,
            telos_resource_id: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (cogmap, _telos) = fired.cogmap_genesis().unwrap();

    // The resource under test: one block, one chunk carrying header_path + heading_depth verbatim.
    let blocks = vec![one_chunk_block(
        "body under a heading",
        Some(header_path.to_string()),
        Some(heading_depth),
    )];
    let mut tx = pool.begin().await.unwrap();
    let fired = fire(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "Headed Resource",
            origin_uri: "temper://heading-carry/r",
            resource_id: None,
            home: temper_substrate::payloads::AnchorRef::cogmap(cogmap),
            owner,
            originator: None,
            blocks: &blocks,
            doc_type: None,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    fired.resource().unwrap().uuid()
}

/// Insert one `kb_profiles` row by handle (display_name = handle, `system_access` defaults
/// to `'none'`), returning its new id. Runtime `sqlx::query` (a test-fixture insert) so it
/// needs no offline-cache entry.
pub async fn insert_profile(pool: &sqlx::PgPool, handle: &str) -> uuid::Uuid {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL search_path TO public")
        .execute(&mut *tx)
        .await
        .expect("set search_path");
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(&mut *tx)
    .await
    .expect("insert profile");
    tx.commit().await.expect("commit");
    id
}

/// Insert one owner-scoped `kb_contexts` row, returning its new id (or the DB error if it
/// violates `UNIQUE(owner_table, owner_id, slug)`). Runtime `sqlx::query` (a test-fixture insert).
pub async fn insert_context(
    pool: &sqlx::PgPool,
    owner_table: &str,
    owner_id: uuid::Uuid,
    slug: &str,
    name: &str,
) -> Result<uuid::Uuid, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO public")
        .execute(&mut *tx)
        .await?;
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) VALUES ($1,$2,$3,$4) RETURNING id",
    )
    .bind(owner_table)
    .bind(owner_id)
    .bind(slug)
    .bind(name)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Seed the canonical system actor (profile + entity + event types + global lenses).
/// Thin wrapper around `bootseed::seed_system` for artifact tests that call through `common::`.
pub async fn seed_system(pool: &sqlx::PgPool) {
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .expect("seed_system");
}

/// Genesis a cogmap (name + telos_title) via `fire(CogmapGenesis)`, using the boot-seeded `system`
/// actor as owner + emitter. Returns `(cogmap_id, telos_resource_id)` as raw UUIDs. Call after
/// `seed_system`.
pub async fn genesis_cogmap(
    pool: &sqlx::PgPool,
    name: &str,
    telos_title: &str,
) -> (uuid::Uuid, uuid::Uuid) {
    use temper_substrate::content::{PreparedBlock, PreparedChunk};
    use temper_substrate::events::{fire, SeedAction};
    use temper_substrate::ids::{BlockId, ChunkId, EntityId, ProfileId};
    use uuid::Uuid;

    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    let owner = ProfileId::from(profile);
    let emitter = EntityId::from(entity);

    let mut charter_embedding = vec![0.0_f32; 768];
    charter_embedding[0] = 1.0;
    let charter = vec![PreparedBlock {
        incorporated: vec![],
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: "charter statement".to_string(),
            embedding: Some(charter_embedding),
            header_path: None,
            heading_depth: None,
        }],
    }];

    let mut tx = pool.begin().await.unwrap();
    let fired = fire(
        &mut tx,
        SeedAction::CogmapGenesis {
            name,
            telos_title,
            charter: &charter,
            cogmap_id: None,
            telos_resource_id: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (cogmap, telos) = fired.cogmap_genesis().unwrap();
    (cogmap.uuid(), telos.uuid())
}

/// Insert one `kb_teams` row by slug (name = slug), returning its new id. Runtime `sqlx::query`
/// (a test-fixture insert).
pub async fn create_team(pool: &sqlx::PgPool, slug: &str) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("create_team")
}

/// Insert one `kb_profiles` row by email used as handle and display_name (email also stored in the
/// `email` column), returning its new id. `system_access` defaults to `'none'`. Runtime
/// `sqlx::query` (a test-fixture insert).
pub async fn create_profile(pool: &sqlx::PgPool, email: &str) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, email) VALUES ($1, $1, $1) RETURNING id",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("create_profile")
}

/// Add a profile to a team as a `'member'`. Runtime `sqlx::query` (a test-fixture insert).
pub async fn add_team_member(pool: &sqlx::PgPool, team: uuid::Uuid, profile: uuid::Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("add_team_member");
}

/// Canonical, UUID-INDEPENDENT region partition signature for a cogmap at lens `telos-default`:
/// each region's member origin_uris sorted within the region, regions sorted among themselves, then
/// hashed. Independent of region/member UUIDs and group order, so it is comparable across seeding
/// paths and across separate instantiations of the same seed (identity-as-input regenerates UUIDs
/// per load, so `membership_fingerprint` is only comparable within one instantiation).
const PARTITION_SQL: &str = r#"
SELECT md5(string_agg(grp, '|' ORDER BY grp)) FROM (
  SELECT string_agg(res.origin_uri, ',' ORDER BY res.origin_uri) AS grp
  FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded
  JOIN kb_cogmap_lenses  l ON l.id = r.lens_id AND l.name = 'telos-default'
  JOIN kb_resources    res ON res.id = m.member_id
  WHERE r.cogmap_id = $1
  GROUP BY m.region_id
) g
"#;

pub async fn telos_default_partition(pool: &sqlx::PgPool, cogmap: uuid::Uuid) -> String {
    sqlx::query_scalar(PARTITION_SQL)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Like [`telos_default_partition`] but folds each region's READOUT values into the signature
/// (content_cohesion, salience, member_count), so it distinguishes a stale-readout reuse from a fresh
/// recompute. UUID-independent (keyed by sorted member origin_uris); floats rounded to 6 places to
/// absorb text-formatting noise (identical SQL over identical embeddings is already bit-stable).
const READOUT_SIG_SQL: &str = r#"
SELECT md5(string_agg(sig, '|' ORDER BY sig)) FROM (
  SELECT string_agg(res.origin_uri, ',' ORDER BY res.origin_uri)
         || ':' || coalesce(round(r.content_cohesion::numeric, 6)::text, 'null')
         || ',' || coalesce(round(r.salience::numeric, 6)::text, 'null')
         || ',' || r.member_count::text AS sig
  FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded
  JOIN kb_cogmap_lenses  l ON l.id = r.lens_id AND l.name = 'telos-default'
  JOIN kb_resources    res ON res.id = m.member_id
  WHERE r.cogmap_id = $1
  GROUP BY r.id, r.content_cohesion, r.salience, r.member_count
) g
"#;

pub async fn telos_default_readout_signature(pool: &sqlx::PgPool, cogmap: uuid::Uuid) -> String {
    sqlx::query_scalar(READOUT_SIG_SQL)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}
