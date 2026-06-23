//! Shared setup for the scenario write-path artifact tests.
//!
//! These tests OWN the `temper_next` namespace: each resets it to a clean canonical baseline (the
//! `00_namespace_reset` fixture drops+recreates the schema, then the namespace-free baseline body
//! files from `migrations/` are loaded under a `search_path=temper_next,public` PGOPTIONS wrapper),
//! then boot-seeds + loads its own scenario. The tests are serialized via the `temper-next-write`
//! nextest test-group so resets never race a sibling's queries.
//!
//! (The legacy read-path tests — materialize/substrate_read/embed_job — instead assume `03_seed.sql`
//! is loaded, so the two suites are run separately. M2 retires the legacy path.)

#![allow(dead_code)]

/// Drop + reload the canonical baseline schema and functions into a clean (un-seeded) `temper_next`.
/// `00_namespace_reset.sql` (a test-only fixture) carries the destructive DROP/CREATE preamble; the
/// two baseline body files come from `migrations/` and land in `temper_next` via the PGOPTIONS wrapper.
pub fn reset_artifact() {
    load_files(&[
        "00_namespace_reset",
        "20260624000001_canonical_schema",
        "20260624000002_canonical_functions",
    ]);
}

/// Like [`reset_artifact`] but also loads the hand-written `03_seed.sql` (the legacy SQL-seed path) —
/// used by the cross-path equivalence test to materialize the SQL-seeded onboarding-cogmap.
pub fn reset_artifact_with_seed() {
    load_files(&[
        "00_namespace_reset",
        "20260624000001_canonical_schema",
        "20260624000002_canonical_functions",
        "03_seed",
    ]);
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
            resource_id: None,
            home: temper_next::payloads::AnchorRef::cogmap(cogmap),
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

/// Fixed UUIDs the prod-shape fixture seeds, so synthesis + parity tests can assert against known
/// ids. Mirrors `tests/fixtures/prod_shape.sql`.
pub mod fixture_ids {
    use uuid::{uuid, Uuid};
    pub const OWNER_PROFILE: Uuid = uuid!("00000000-0000-0000-00f1-000000000001");
    pub const ORIGINATOR_PROFILE: Uuid = uuid!("00000000-0000-0000-00f1-000000000002");
    pub const CONTEXT_ONE: Uuid = uuid!("00000000-0000-0000-00c0-000000000001");
    pub const CONTEXT_TWO: Uuid = uuid!("00000000-0000-0000-00c0-000000000002");
    /// C3 — team-owned context (exercises the §2-amended team branch + kb_team_contexts auto-share).
    pub const CONTEXT_TEAM: Uuid = uuid!("00000000-0000-0000-00c0-000000000003");
    /// The team that owns C3 (the only team in the fixture).
    pub const TEAM: Uuid = uuid!("00000000-0000-0000-0701-000000000001");
    pub const EVENT: Uuid = uuid!("00000000-0000-0000-00e0-000000000001");
    /// R1 — concept, the temper-goal target.
    pub const RESOURCE_GOAL: Uuid = uuid!("00000000-0000-0000-00a0-000000000001");
    /// R2 — task carrying temper-goal + the §7 key spread (originator≠owner).
    pub const RESOURCE_TASK: Uuid = uuid!("00000000-0000-0000-00a0-000000000002");
    /// R3 — decision.
    pub const RESOURCE_DECISION: Uuid = uuid!("00000000-0000-0000-00a0-000000000003");
    /// R4 — soft-deleted (must be excluded by synthesis).
    pub const RESOURCE_DELETED: Uuid = uuid!("00000000-0000-0000-00a0-000000000004");
    /// R5 — active, homed in the team-owned context C3.
    pub const RESOURCE_TEAM: Uuid = uuid!("00000000-0000-0000-00a0-000000000005");
    pub const EDGE_NORMAL: Uuid = uuid!("00000000-0000-0000-0dd0-000000000001");
    pub const EDGE_FOLDED: Uuid = uuid!("00000000-0000-0000-0dd0-000000000002");
    /// The inverse-polarity (leads_to, R3→R1) edge — proves polarity carries verbatim (§4).
    pub const EDGE_INVERSE: Uuid = uuid!("00000000-0000-0000-0dd0-000000000003");
}

/// Insert one `temper_next.kb_profiles` row by handle (display_name = handle, `system_access` defaults
/// to `'none'`), returning its new id. Runs inside a `SET LOCAL search_path TO temper_next, public`
/// transaction so the `sync_personal_team` / `sync_system_membership` AFTER-INSERT triggers resolve
/// their unqualified table references into `temper_next` (same discipline as `synthesis::bootstrap`).
/// Runtime `sqlx::query` (a test-fixture insert) so it needs no offline-cache entry.
pub async fn insert_profile(pool: &sqlx::PgPool, handle: &str) -> uuid::Uuid {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL search_path TO temper_next, public")
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

/// Insert one owner-scoped `temper_next.kb_contexts` row, returning its new id (or the DB error if it
/// violates `UNIQUE(owner_table, owner_id, slug)`). Wrapped in a `SET LOCAL search_path` transaction
/// for parity with the other temper_next writers. Runtime `sqlx::query` (a test-fixture insert).
pub async fn insert_context(
    pool: &sqlx::PgPool,
    owner_table: &str,
    owner_id: uuid::Uuid,
    slug: &str,
    name: &str,
) -> Result<uuid::Uuid, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO temper_next, public")
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

/// Seed the prod-shape fixture into `public.*` then run synthesis into `temper_next.*`, returning
/// the synthesis report. The standard setup for the chunk-3 parity-read tests.
pub async fn seed_and_synthesize(pool: &sqlx::PgPool) -> temper_next::synthesis::SynthReport {
    seed_prod_shape_fixture(pool).await;
    temper_next::synthesis::run(pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run")
}

fn load_files(files: &[&str]) {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for artifact tests");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    for f in files {
        // The namespace-free canonical baseline body files live in `migrations/` and won't self-set
        // search_path, so inject it via PGOPTIONS so their unqualified DDL lands in `temper_next`
        // (not `public`). The reset + legacy-seed fixtures live in `tests/fixtures/`; the reset is
        // fully qualified (no search_path needed), `03_seed` self-sets but we wrap it anyway.
        let (path, set_search_path) = match *f {
            "00_namespace_reset" => (format!("{fixtures}/00_namespace_reset.sql"), false),
            "03_seed" => (format!("{fixtures}/03_seed.sql"), true),
            other => (format!("{root}/migrations/{other}.sql"), true),
        };
        let mut cmd = std::process::Command::new("psql");
        if set_search_path {
            cmd.env("PGOPTIONS", "-csearch_path=temper_next,public");
        }
        let status = cmd
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
