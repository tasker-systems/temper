#![cfg(feature = "artifact-tests")]
//! The measurement corpus asserts its own fitness, because a degenerate corpus is SILENT.
//!
//! Everything downstream of this fixture — Task 2's plan-identity diff, Task 3's
//! crowded-out-of-top-k witness, Task 6's gate-shape comparison — reads as green against a corpus
//! that cannot answer the question. Three ways that happens, each guarded below:
//!
//!   * **Uniform-random 768-dim vectors are all equidistant.** In high dimensions independent
//!     uniform draws concentrate: every pairwise cosine distance lands in a narrow band around one
//!     value, so ANN ranking becomes arbitrary and "the top-k crowded X out" stops meaning
//!     anything. `topic_clusters_are_separated` is the guard, and it is the reason the generator
//!     draws around per-topic centroids rather than at random.
//!   * **Everything visible to everyone.** The corpus we have measurements from passes 97.6% of
//!     active resources through the gate with every team arm empty. A gate that filters nothing
//!     cannot show what hoisting it costs.
//!   * **Rows with no searchable content.** Resources whose chunks carry no embedding, or whose FTS
//!     vector was never built, are invisible to both arms — the corpus would look large and search
//!     like it was empty.
//!
//! Each test names the production change that would make it fail, so none of them is decorative.
mod common;

use std::collections::HashMap;
use temper_substrate::scenario::access::{self, model::AccessScenario};
use temper_substrate::scenario::bootseed;

const MEASUREMENT_CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/access-scenarios/measurement-corpus.yaml"
);

fn load_corpus_yaml() -> AccessScenario {
    serde_yaml::from_str(&std::fs::read_to_string(MEASUREMENT_CORPUS).unwrap()).unwrap()
}

/// Declared `count` per population, summed. Kept beside the fixture rather than read from it so a
/// silently-ignored `populations:` block cannot make the assertion agree with itself.
const DECLARED_TOTAL: i64 = 240;

async fn load(pool: &sqlx::PgPool) -> access::loader::LoadedAccess {
    common::reset_schema(pool).await;
    bootseed::seed_system(pool).await.unwrap();
    access::load(pool, &load_corpus_yaml().world).await.unwrap()
}

/// The populations produce rows at all.
///
/// Fails if `populations:` is ignored (serde skips unknown fields silently, so an unimplemented
/// block deserializes clean and yields the five hand-declared resources — here, zero).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn populations_generate_the_declared_number_of_resources(pool: sqlx::PgPool) {
    let loaded = load(&pool).await;

    assert_eq!(
        loaded.resources.len() as i64,
        DECLARED_TOTAL,
        "every generated resource must be registered under its `<key_prefix>-<n>` key so the \
         declarative checks and downstream measurements can name it"
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE is_active")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        rows >= DECLARED_TOTAL,
        "expected at least {DECLARED_TOTAL} live kb_resources, found {rows}"
    );
}

/// Every generated resource is reachable by BOTH arms.
///
/// Fails if the generator writes `kb_chunks` without an embedding (the wide arm sees nothing), or
/// skips `_rebuild_resource_search_vector` (the exact arm sees nothing). Either one leaves a corpus
/// that is large and unsearchable — the failure that looks like a slow query rather than a bad
/// fixture.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn every_generated_resource_is_searchable_by_both_arms(pool: sqlx::PgPool) {
    load(&pool).await;

    // Scoped to GENERATED rows by origin_uri. Bootseed's L0 telos resource is a legitimate
    // chunk-less row and is not part of the corpus — but scoping a "count the violations" query is
    // how such a test goes quietly vacuous, so the denominator is asserted first: `== 0 of 0` must
    // never read as success.
    let generated: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resources
          WHERE is_active AND origin_uri LIKE 'synthetic://measurement-corpus/%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        generated, DECLARED_TOTAL,
        "the scoped denominator must be the whole declared corpus, or the violation counts below \
         pass by having nothing to check"
    );

    let without_embedding: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resources r
          WHERE r.is_active AND r.origin_uri LIKE 'synthetic://measurement-corpus/%'
            AND NOT EXISTS (SELECT 1 FROM kb_chunks c
                             WHERE c.resource_id = r.id AND c.is_current
                               AND c.embedding IS NOT NULL)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        without_embedding, 0,
        "{without_embedding} of {generated} generated resources carry no current embedded chunk; \
         the wide arm cannot draw them and any top-k measurement over this corpus is vacuous"
    );

    let without_fts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resources r
          WHERE r.is_active AND r.origin_uri LIKE 'synthetic://measurement-corpus/%'
            AND NOT EXISTS (SELECT 1 FROM kb_resource_search_index si
                             WHERE si.resource_id = r.id AND si.search_vector <> ''::tsvector)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        without_fts, 0,
        "{without_fts} of {generated} generated resources have no FTS vector; the exact arm \
         cannot match them"
    );
}

/// The `doc_type` predicate has something to discriminate.
///
/// Fails if the generator omits the property rows, or assigns one doc_type to everything — in which
/// case `p_doc_type` is either a no-op or a full-corpus filter, and neither shape exercises the
/// `uq_kb_properties_active` index path that §2.1/§2.2 of the spec turn on.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn doc_type_is_present_and_discriminating(pool: sqlx::PgPool) {
    load(&pool).await;

    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT p.property_value #>> '{}' AS doc_type, count(*)
           FROM kb_properties p
          WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type'
            AND NOT p.is_folded
          GROUP BY 1 ORDER BY 2 DESC",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        counts.len() >= 3,
        "expected at least 3 distinct doc_types across the corpus, found {counts:?}"
    );
    let total: i64 = counts.iter().map(|(_, n)| n).sum();
    let largest = counts[0].1;
    assert!(
        largest * 2 < total,
        "the most common doc_type holds {largest} of {total} rows; a predicate that selects a \
         majority is not measuring selectivity"
    );
}

/// Vectors carry real cluster structure.
///
/// **This is the guard that matters most, and the one a naive generator fails.** Independent
/// uniform draws in 768 dimensions concentrate: pairwise cosine distances collapse into a narrow
/// band, ANN ranking becomes arbitrary, and Task 3's "a bounded call returned a resource the top-k
/// crowded out" witness passes or fails by coin-flip while looking deterministic.
///
/// Fails if the generator draws each chunk vector independently instead of around a per-topic
/// centroid. The assertion is comparative, never a pinned number — the gap must be real, not a
/// particular size.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn topic_clusters_are_separated(pool: sqlx::PgPool) {
    load(&pool).await;

    // Mean cosine distance within a topic vs. across topics, read straight from pgvector. The
    // topic label rides on the chunk's own resource via the `topic` property so this needs no
    // knowledge of how the generator laid the vectors out.
    let (within, across): (f64, f64) = sqlx::query_as(
        "WITH c AS (
             SELECT ch.embedding AS e,
                    (SELECT p.property_value #>> '{}' FROM kb_properties p
                      WHERE p.owner_table = 'kb_resources' AND p.owner_id = ch.resource_id
                        AND p.property_key = 'topic' AND NOT p.is_folded LIMIT 1) AS topic
               FROM kb_chunks ch
              WHERE ch.is_current AND ch.embedding IS NOT NULL
         ),
         pairs AS (
             SELECT a.topic = b.topic AS same, (a.e <=> b.e) AS dist
               FROM c a JOIN c b ON a.e <> b.e
              WHERE a.topic IS NOT NULL AND b.topic IS NOT NULL
         )
         SELECT avg(dist) FILTER (WHERE same),
                avg(dist) FILTER (WHERE NOT same)
           FROM pairs",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        within < across,
        "mean within-topic cosine distance ({within:.4}) must be below cross-topic ({across:.4}); \
         equal means the vectors carry no cluster structure and ANN ranking over this corpus is \
         arbitrary"
    );
    assert!(
        across - within > 0.05,
        "topic separation is only {:.4} (within {within:.4}, across {across:.4}); too small to \
         make a top-k draw meaningfully prefer one topic, so a crowded-out witness would be \
         measuring noise",
        across - within
    );
}

/// Visibility is a proper fraction, and reach is uneven.
///
/// Fails if the generator grants to the root team (down-only inheritance then hands everyone
/// everything), or drops grants entirely (nobody sees anything but their own homes). Both produce a
/// gate that filters ~nothing or ~everything, and neither can show what the gate costs.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn no_principal_sees_all_of_the_corpus_and_reach_is_uneven(pool: sqlx::PgPool) {
    let loaded = load(&pool).await;
    let seen = visible_counts(&pool, &loaded.profiles).await;

    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE is_active")
        .fetch_one(&pool)
        .await
        .unwrap();

    let ana = seen["ana"];
    let cara = seen["cara"];
    let nomad = seen["nomad"];

    assert_eq!(
        nomad, 0,
        "nomad joins no team and holds no grant; expected 0, saw {nomad}"
    );
    assert!(
        ana > 0 && ana < total,
        "ana must see a PROPER subset: saw {ana} of {total}"
    );
    assert!(
        (ana as f64) / (total as f64) < 0.9,
        "ana sees {ana}/{total}; above ~90% the gate is not discriminating enough to measure \
         (the community instance's 97.6% is the shape this corpus exists to escape)"
    );
    assert!(
        ana > cara,
        "reach must be UNEVEN — ana (platform, reaching engineering by ancestry) should out-see \
         cara (research, a sibling branch): ana={ana} cara={cara}"
    );
}

/// The team arm of `resources_visible_to` is load-bearing, not merely non-empty.
///
/// Fails if every resource reaches its principal by ownership instead of by team grant — the
/// community corpus's shape, where the team arms plan but return nothing. Asserted by comparing a
/// principal's total reach against the part of it they own: the difference IS the team arm.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_team_grant_arm_carries_real_rows(pool: sqlx::PgPool) {
    let loaded = load(&pool).await;
    let ben = loaded.profiles["ben"];

    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM resources_visible_to($1)")
        .bind(ben)
        .fetch_one(&pool)
        .await
        .unwrap();
    let owned: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_homes WHERE owner_profile_id = $1")
            .bind(ben)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(
        total > owned,
        "ben sees {total} resources and owns {owned}; with nothing reaching him by team grant the \
         gate's team arms plan but return nothing, which is exactly the corpus shape that made the \
         existing visibility measurements unrepresentative"
    );
}

async fn visible_counts(
    pool: &sqlx::PgPool,
    profiles: &HashMap<String, uuid::Uuid>,
) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    for (handle, id) in profiles {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM resources_visible_to($1)")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        out.insert(handle.clone(), n);
    }
    out
}
