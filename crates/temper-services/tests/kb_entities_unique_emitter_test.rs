#![cfg(feature = "test-db")]

//! Coverage for the unique constraint on `kb_entities (profile_id, name)`.
//!
//! `<handle>@<surface>` is the natural key `writes::resolve_emitter` resolves against, but the
//! schema never enforced it. A duplicate does not raise: `fetch_one` is `fetch_optional` +
//! `RowNotFound` in sqlx, which has no too-many-rows variant, so the write path silently binds
//! events for one logical emitter to whichever of the two rows Postgres yields first. The ledger
//! splits, quietly.
//!
//! The migration therefore has to do two things, in order: make `(profile_id, name)` unique among
//! existing rows, then create the index. It does the first by *renaming* every non-survivor to
//! `<name>#dup-<id>` rather than deleting it — `kb_events` is append-only, so the events a
//! duplicate carries can be neither moved nor dropped. What these tests pin, above all, is that no
//! row of the ledger is rewritten.
//!
//! Both halves are exercised against the shipped `.sql`, read with `include_str!` rather than
//! retyped, so these tests cannot drift from what runs against a real database.

use sqlx::{PgPool, Row};
use uuid::{uuid, Uuid};

/// The shipped migration, executed verbatim.
const MIGRATION_SQL: &str =
    include_str!("../../../migrations/20260709000040_kb_entities_unique_profile_name.sql");

/// Born by `20260625000001_l0_kernel_cogmap.sql`. Reused rather than seeded because
/// `kb_invocations` demands a real cogmap and a real telos resource.
const L0_COGMAP_ID: Uuid = uuid!("00000000-0000-0000-0005-000000000001");
const L0_TELOS_RESOURCE_ID: Uuid = uuid!("00000000-0000-0000-0005-000000000002");

const UNIQUE_INDEX: &str = "kb_entities_profile_id_name_key";

async fn run_migration(pool: &PgPool) {
    sqlx::raw_sql(MIGRATION_SQL)
        .execute(pool)
        .await
        .expect("run the unique-constraint migration");
}

/// The migration is part of `migrations/`, so `#[sqlx::test]` has already applied it. Tests that
/// need to observe the *pre-migration* world drop the index first, seed the duplicates the
/// constraint would forbid, and then re-run the migration to watch it repair them.
async fn drop_unique_index(pool: &PgPool) {
    sqlx::query(&format!("DROP INDEX IF EXISTS {UNIQUE_INDEX}"))
        .execute(pool)
        .await
        .expect("drop the unique index");
}

async fn unique_index_exists(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE tablename = 'kb_entities' AND indexname = $1)",
    )
    .bind(UNIQUE_INDEX)
    .fetch_one(pool)
    .await
    .expect("look up the unique index")
}

async fn seed_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("seed profile")
}

/// Insert an emitter entity with an explicit id, so tests control which row is the survivor
/// (lowest id — UUIDv7 compares bytewise in Postgres, so lowest is oldest).
async fn seed_entity(pool: &PgPool, id: Uuid, profile_id: Uuid, name: &str) {
    sqlx::query("INSERT INTO kb_entities (id, profile_id, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(profile_id)
        .bind(name)
        .execute(pool)
        .await
        .expect("seed emitter entity");
}

async fn any_event_type(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_event_types LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("event types are seeded by migration")
}

async fn seed_event(pool: &PgPool, emitter_entity_id: Uuid) -> Uuid {
    let event_type_id = any_event_type(pool).await;
    sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(event_type_id)
    .bind(emitter_entity_id)
    .fetch_one(pool)
    .await
    .expect("seed event")
}

async fn seed_invocation(pool: &PgPool, scoped_entity_id: Uuid, opened_by_event_id: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_invocations \
         (id, opened_by_event_id, trigger_kind, originating_cogmap_id, scoped_entity_id, \
          telos_resource_id, opened_at) \
         VALUES ($1, $2, 'test', $3, $4, $5, now())",
    )
    .bind(id)
    .bind(opened_by_event_id)
    .bind(L0_COGMAP_ID)
    .bind(scoped_entity_id)
    .bind(L0_TELOS_RESOURCE_ID)
    .execute(pool)
    .await
    .expect("seed invocation");
    id
}

async fn emitter_of_event(pool: &PgPool, event_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT emitter_entity_id FROM kb_events WHERE id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .expect("read event emitter")
}

async fn scoped_entity_of_invocation(pool: &PgPool, invocation_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT scoped_entity_id FROM kb_invocations WHERE id = $1")
        .bind(invocation_id)
        .fetch_one(pool)
        .await
        .expect("read invocation scoped entity")
}

async fn name_of_entity(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT name FROM kb_entities WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read entity name")
}

async fn entity_exists(pool: &PgPool, id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM kb_entities WHERE id = $1)")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("check entity existence")
}

async fn entity_ids(pool: &PgPool, profile_id: Uuid, name: &str) -> Vec<Uuid> {
    sqlx::query("SELECT id FROM kb_entities WHERE profile_id = $1 AND name = $2 ORDER BY id")
        .bind(profile_id)
        .bind(name)
        .fetch_all(pool)
        .await
        .expect("read entity ids")
        .iter()
        .map(|r| r.get::<Uuid, _>("id"))
        .collect()
}

/// Two entities sharing `(profile_id, name)`, the second one lower-numbered than the first is
/// *not* — `survivor` is deliberately the lesser id so the assertion pins the tie-break rule
/// rather than accidentally agreeing with insertion order.
struct Duplicated {
    profile_id: Uuid,
    name: String,
    survivor: Uuid,
    loser: Uuid,
}

async fn seed_duplicate_pair(pool: &PgPool, handle: &str) -> Duplicated {
    let profile_id = seed_profile(pool, handle).await;
    let name = format!("{handle}@web");

    // Two v7 ids minted in order, then assigned so the *later* one is the loser.
    let survivor = Uuid::now_v7();
    let loser = Uuid::now_v7();
    assert!(survivor < loser, "UUIDv7 should be time-ordered");

    // Insert the loser first: if the migration picked "first inserted" rather than "lowest id",
    // this seeding order would catch it.
    seed_entity(pool, loser, profile_id, &name).await;
    seed_entity(pool, survivor, profile_id, &name).await;

    Duplicated {
        profile_id,
        name,
        survivor,
        loser,
    }
}

/// The constraint's whole purpose: a second `(profile_id, name)` cannot land.
#[sqlx::test(migrations = "../../migrations")]
async fn unique_constraint_rejects_a_second_entity_with_the_same_profile_and_name(pool: PgPool) {
    let profile_id = seed_profile(&pool, "dup-reject").await;

    seed_entity(&pool, Uuid::now_v7(), profile_id, "dup-reject@web").await;

    let second = sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
        .bind(profile_id)
        .bind("dup-reject@web")
        .execute(&pool)
        .await;

    let err = second.expect_err("a duplicate (profile_id, name) must be rejected");
    let db_err = err.as_database_error().expect("a database error");
    assert_eq!(
        db_err.code().as_deref(),
        Some("23505"),
        "expected a unique_violation, got: {err}"
    );
}

/// Same `name`, different profile — not a duplicate. Guards against a constraint accidentally
/// declared on `name` alone.
#[sqlx::test(migrations = "../../migrations")]
async fn the_same_entity_name_is_allowed_under_a_different_profile(pool: PgPool) {
    let a = seed_profile(&pool, "tenant-a").await;
    let b = seed_profile(&pool, "tenant-b").await;

    seed_entity(&pool, Uuid::now_v7(), a, "shared-name@web").await;
    seed_entity(&pool, Uuid::now_v7(), b, "shared-name@web").await;

    assert_eq!(entity_ids(&pool, a, "shared-name@web").await.len(), 1);
    assert_eq!(entity_ids(&pool, b, "shared-name@web").await.len(), 1);
}

/// The canonical name is freed for exactly one row — the oldest — and the loser is renamed out of
/// the way rather than deleted, so `resolve_emitter` resolves and the constraint can be built.
#[sqlx::test(migrations = "../../migrations")]
async fn quarantine_leaves_the_canonical_name_to_the_oldest_row(pool: PgPool) {
    drop_unique_index(&pool).await;
    let dup = seed_duplicate_pair(&pool, "quarantine-name").await;

    run_migration(&pool).await;

    assert_eq!(
        entity_ids(&pool, dup.profile_id, &dup.name).await,
        vec![dup.survivor],
        "exactly the lowest-id row keeps the canonical name",
    );
    assert_eq!(
        name_of_entity(&pool, dup.loser).await,
        format!("{}#dup-{}", dup.name, dup.loser),
        "the loser is renamed into the quarantine namespace, not deleted",
    );
    assert!(unique_index_exists(&pool).await);
}

/// The heart of it. `kb_events` is append-only (`kb_events_append_only`, BEFORE DELETE OR UPDATE),
/// so the migration must not move a single event. An event emitted against the duplicate stays
/// attributed to the duplicate — the split in history is real and is preserved, not rewritten.
#[sqlx::test(migrations = "../../migrations")]
async fn quarantine_does_not_rewrite_the_event_ledger(pool: PgPool) {
    drop_unique_index(&pool).await;
    let dup = seed_duplicate_pair(&pool, "quarantine-events").await;

    let event_on_loser = seed_event(&pool, dup.loser).await;
    let event_on_survivor = seed_event(&pool, dup.survivor).await;

    run_migration(&pool).await;

    assert_eq!(
        emitter_of_event(&pool, event_on_loser).await,
        dup.loser,
        "an event emitted against the duplicate must stay attributed to it",
    );
    assert_eq!(
        emitter_of_event(&pool, event_on_survivor).await,
        dup.survivor,
    );
}

/// The second FK dependent. `kb_invocations.scoped_entity_id` is NOT NULL; quarantining by rename
/// keeps the row alive, so the reference stays valid without an update.
#[sqlx::test(migrations = "../../migrations")]
async fn quarantine_keeps_invocation_references_valid(pool: PgPool) {
    drop_unique_index(&pool).await;
    let dup = seed_duplicate_pair(&pool, "quarantine-invocations").await;

    let opening_event = seed_event(&pool, dup.survivor).await;
    let invocation = seed_invocation(&pool, dup.loser, opening_event).await;

    run_migration(&pool).await;

    assert_eq!(
        scoped_entity_of_invocation(&pool, invocation).await,
        dup.loser,
        "the invocation still points at the row it was scoped to",
    );
}

/// Zero orphans across both FK dependents. Trivially true under quarantine — which is the point:
/// the repair cannot strand a reference because it removes nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn quarantine_leaves_no_dangling_references(pool: PgPool) {
    drop_unique_index(&pool).await;
    let dup = seed_duplicate_pair(&pool, "quarantine-orphans").await;

    let opening_event = seed_event(&pool, dup.loser).await;
    seed_invocation(&pool, dup.loser, opening_event).await;

    run_migration(&pool).await;

    // Guard against a vacuous pass: if the migration ever went back to deleting, the loser would
    // be gone and the counts below would still read zero only if the repointing worked.
    assert!(
        entity_exists(&pool, dup.loser).await,
        "the loser row must survive quarantine",
    );

    let dangling_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev \
         WHERE NOT EXISTS (SELECT 1 FROM kb_entities e WHERE e.id = ev.emitter_entity_id)",
    )
    .fetch_one(&pool)
    .await
    .expect("count dangling events");
    assert_eq!(
        dangling_events, 0,
        "no event may reference a missing entity"
    );

    let dangling_invocations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_invocations i \
         WHERE NOT EXISTS (SELECT 1 FROM kb_entities e WHERE e.id = i.scoped_entity_id)",
    )
    .fetch_one(&pool)
    .await
    .expect("count dangling invocations");
    assert_eq!(dangling_invocations, 0);
}

/// The quarantine namespace is not reserved: `name` is arbitrary text, and a database we cannot
/// inspect may already hold a row named exactly what a loser is about to be renamed to. Renaming
/// into it would mint a fresh duplicate and blow up the index build — rolling back the whole
/// migration, which is the one outcome the quarantine exists to prevent. Renaming must therefore
/// repeat until the name it lands on is free.
#[sqlx::test(migrations = "../../migrations")]
async fn quarantine_survives_a_preexisting_name_in_the_quarantine_namespace(pool: PgPool) {
    drop_unique_index(&pool).await;
    let profile_id = seed_profile(&pool, "squatted").await;
    let name = "squatted@web";

    let survivor = Uuid::now_v7();
    let loser = Uuid::now_v7();
    seed_entity(&pool, survivor, profile_id, name).await;
    seed_entity(&pool, loser, profile_id, name).await;

    // A row already sitting on the exact name the loser will be renamed to.
    let squatter = Uuid::now_v7();
    let squatted_name = format!("{name}#dup-{loser}");
    seed_entity(&pool, squatter, profile_id, &squatted_name).await;

    run_migration(&pool).await;

    assert_eq!(
        entity_ids(&pool, profile_id, name).await,
        vec![survivor],
        "the canonical name still resolves to exactly the oldest row",
    );
    assert_eq!(
        name_of_entity(&pool, loser).await,
        squatted_name,
        "the loser takes the quarantine name; it is older than the squatter",
    );
    assert_eq!(
        name_of_entity(&pool, squatter).await,
        format!("{squatted_name}#dup-{squatter}"),
        "the displaced squatter is itself quarantined, one level deeper",
    );
    assert!(unique_index_exists(&pool).await);
}

/// Three rows on one name: one survivor, two distinctly-named quarantines — in a single pass, and
/// without tripping the unique index the same statement is about to make enforceable.
#[sqlx::test(migrations = "../../migrations")]
async fn quarantine_handles_more_than_two_duplicates(pool: PgPool) {
    drop_unique_index(&pool).await;
    let profile_id = seed_profile(&pool, "triple").await;
    let name = "triple@web";

    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    let third = Uuid::now_v7();
    for id in [first, second, third] {
        seed_entity(&pool, id, profile_id, name).await;
    }

    run_migration(&pool).await;

    assert_eq!(entity_ids(&pool, profile_id, name).await, vec![first]);
    assert_eq!(
        name_of_entity(&pool, second).await,
        format!("{name}#dup-{second}")
    );
    assert_eq!(
        name_of_entity(&pool, third).await,
        format!("{name}#dup-{third}")
    );
}

/// Operators re-run migrations. A clean database and a second application must both be no-ops.
#[sqlx::test(migrations = "../../migrations")]
async fn migration_is_idempotent_on_an_already_migrated_database(pool: PgPool) {
    let profile_id = seed_profile(&pool, "idempotent").await;
    let entity = Uuid::now_v7();
    seed_entity(&pool, entity, profile_id, "idempotent@web").await;

    run_migration(&pool).await;
    run_migration(&pool).await;

    assert_eq!(
        entity_ids(&pool, profile_id, "idempotent@web").await,
        vec![entity]
    );
    assert!(unique_index_exists(&pool).await);
}
