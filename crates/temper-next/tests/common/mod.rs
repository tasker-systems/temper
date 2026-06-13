//! Shared setup for the scenario write-path artifact tests.
//!
//! These tests OWN the `temper_next` namespace: each resets it to a clean `01_schema` + `02_functions`
//! (no `03_seed.sql`), then boot-seeds + loads its own scenario. `01_schema.sql` drops and recreates
//! the `temper_next` schema, so loading 01+02 is a full reset. The tests are serialized via the
//! `temper-next-write` nextest test-group so resets never race a sibling's queries.
//!
//! (The legacy read-path tests — materialize/substrate_read/embed_job — instead assume `03_seed.sql`
//! is loaded externally, so the two suites are run separately. M2 retires the legacy path.)

#![allow(dead_code)]

/// Drop + reload the artifact schema and functions, leaving a clean (un-seeded) `temper_next`.
/// `00_namespace_reset.sql` carries the destructive DROP/CREATE preamble (factored out of `01_schema`
/// so the body is a namespace-resident, no-DROP install source); `01`+`02` are the shared DDL body.
pub fn reset_artifact() {
    load_files(&["00_namespace_reset", "01_schema", "02_functions"]);
}

/// Like [`reset_artifact`] but also loads the hand-written `03_seed.sql` (the legacy SQL-seed path) —
/// used by the cross-path equivalence test to materialize the SQL-seeded onboarding-cogmap.
pub fn reset_artifact_with_seed() {
    load_files(&["00_namespace_reset", "01_schema", "02_functions", "03_seed"]);
}

/// Read the committed, generated additive install migration (the single source of truth the drift
/// guard compares the generator output against). Fixed path — the generator writes it in place.
pub fn read_latest_install_migration(root: &str) -> String {
    let path = format!("{root}/migrations/20260613000001_install_temper_next.sql");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read install migration at {path}: {e}"))
}

/// Drop `temper_next` then apply the committed additive install migration via psql — proving the
/// run-once `CREATE SCHEMA temper_next;` path works on an absent namespace, leaving `public` untouched.
/// Async only for ergonomics with the tokio tests that call it; the psql invocation is blocking.
pub async fn apply_install_migration(_pool: &sqlx::PgPool) {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for artifact tests");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    // Drop the namespace so CREATE SCHEMA in the migration runs against an absent namespace.
    let drop = std::process::Command::new("psql")
        .args([
            url.as_str(),
            "-q",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            "DROP SCHEMA IF EXISTS temper_next CASCADE",
        ])
        .status()
        .expect("failed to run psql (is it on PATH?)");
    assert!(drop.success(), "psql DROP SCHEMA temper_next failed");
    let path = format!("{root}/migrations/20260613000001_install_temper_next.sql");
    let status = std::process::Command::new("psql")
        .args([url.as_str(), "-q", "-v", "ON_ERROR_STOP=1", "-f", &path])
        .status()
        .expect("failed to run psql (is it on PATH?)");
    assert!(status.success(), "psql -f install migration failed");
}

/// Fire a cogmap genesis + one `resource_create` homed in it, whose single chunk's sidecar entry
/// carries `header_path` + `heading_depth`. Returns the created resource's uuid. Uses the boot-seeded
/// canonical `system` profile/entity as owner+emitter, so call after `bootseed::seed_system`.
pub async fn fire_resource_with_headed_chunk(
    pool: &sqlx::PgPool,
    header_path: &str,
    heading_depth: i16,
) -> uuid::Uuid {
    use temper_next::content::{PreparedBlock, PreparedChunk};
    use temper_next::events::{fire, SeedAction};
    use temper_next::ids::{BlockId, ChunkId, EntityId, ProfileId};
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
            block_id: BlockId::from(Uuid::now_v7()),
            seq: 0,
            role: None,
            chunks: vec![PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index: 0,
                content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
                content: content.to_string(),
                embedding,
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
            home: cogmap,
            owner,
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

/// Fixed UUIDs the prod-shape fixture seeds, so synthesis + parity tests can assert against known
/// ids. Mirrors `tests/fixtures/prod_shape.sql`.
pub mod fixture_ids {
    use uuid::{uuid, Uuid};
    pub const OWNER_PROFILE: Uuid = uuid!("00000000-0000-0000-00f1-000000000001");
    pub const ORIGINATOR_PROFILE: Uuid = uuid!("00000000-0000-0000-00f1-000000000002");
    pub const CONTEXT_ONE: Uuid = uuid!("00000000-0000-0000-00c0-000000000001");
    pub const CONTEXT_TWO: Uuid = uuid!("00000000-0000-0000-00c0-000000000002");
    pub const EVENT: Uuid = uuid!("00000000-0000-0000-00e0-000000000001");
    /// R1 — concept, the temper-goal target.
    pub const RESOURCE_GOAL: Uuid = uuid!("00000000-0000-0000-00a0-000000000001");
    /// R2 — task carrying temper-goal + the §7 key spread (originator≠owner).
    pub const RESOURCE_TASK: Uuid = uuid!("00000000-0000-0000-00a0-000000000002");
    /// R3 — decision.
    pub const RESOURCE_DECISION: Uuid = uuid!("00000000-0000-0000-00a0-000000000003");
    /// R4 — soft-deleted (must be excluded by synthesis).
    pub const RESOURCE_DELETED: Uuid = uuid!("00000000-0000-0000-00a0-000000000004");
    pub const EDGE_NORMAL: Uuid = uuid!("00000000-0000-0000-0dd0-000000000001");
    pub const EDGE_FOLDED: Uuid = uuid!("00000000-0000-0000-0dd0-000000000002");
}

/// Seed the production-shape `public.*` corpus into the given pool's database. Intended for the
/// self-contained `#[sqlx::test(migrator = "temper_next::MIGRATOR")]` tests, whose ephemeral DB has
/// the full migration chain applied (System/Anonymous profiles + seeded doc/event types present) but
/// an otherwise empty `public`. Runs the SQL through `pool` (NOT psql) so it lands in that DB.
pub async fn seed_prod_shape_fixture(pool: &sqlx::PgPool) {
    let sql = include_str!("../fixtures/prod_shape.sql");
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("prod-shape fixture failed to seed");
}

fn load_files(files: &[&str]) {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for artifact tests");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    for f in files {
        let path = format!("{root}/schema-artifact/{f}.sql");
        let status = std::process::Command::new("psql")
            .args([url.as_str(), "-q", "-v", "ON_ERROR_STOP=1", "-f", &path])
            .status()
            .expect("failed to run psql (is it on PATH?)");
        assert!(status.success(), "psql -f {f}.sql failed during reset");
    }
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
