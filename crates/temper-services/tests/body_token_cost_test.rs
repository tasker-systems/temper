//! What the body token costs, measured: the `/content` read with its token at 1, 50 and 500
//! blocks, and the locked read gate a conditional write asks under the resource's lock, for a
//! principal reached through many teams.
//!
//! `#[ignore]`d: these print timings rather than assert them, and a timing is not a CI gate. Run
//! with `cargo nextest run -p temper-services --features test-db --test body_token_cost_test
//! --run-ignored only --no-capture` (with `TEMPER_ASYNC_EMBED=1` where no embedding model is
//! installed).
#![cfg(feature = "test-db")]

use std::time::{Duration, Instant};

use serde_json::json;
use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, BodyUpdate, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
use uuid::Uuid;

const RUNS: usize = 60;

async fn seed_profile(pool: &PgPool, email: &str) -> (Uuid, Uuid) {
    let profile_id = Uuid::now_v7();
    let handle = format!("cost-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,$3,$3)",
    )
    .bind(context_id)
    .bind(profile_id)
    .bind(format!("ctx-{}", &context_id.simple().to_string()[..8]))
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

async fn create(pool: &PgPool, owner: Uuid, context: Uuid, body: &str) -> ResourceId {
    DbBackend::new(pool.clone(), ProfileId::from(owner))
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "cost".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "cost".to_string(),
            body: Some(BodyUpdate::new(body)),
            managed_meta: ManagedMeta::default(),
            open_meta: Some(json!({})),
            goal: None,
            origin_uri: Some(format!("test://cost/{}", Uuid::now_v7())),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create")
        .value
        .id
}

fn body_of(sections: usize) -> String {
    (0..sections)
        .map(|i| format!("# Section {i}\n\nA paragraph of prose for section {i}, long enough to read like one.\n\n"))
        .collect()
}

fn percentiles(mut samples: Vec<Duration>) -> (Duration, Duration) {
    samples.sort();
    let at = |p: f64| samples[((samples.len() as f64 - 1.0) * p).round() as usize];
    (at(0.50), at(0.95))
}

/// The `/content` read and its token, at 1, 50 and 500 blocks: the whole read, and the hash alone.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
#[ignore = "a measurement, not a gate"]
async fn content_token_cost_by_block_count(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "cost@example.com").await;
    for sections in [1usize, 50, 500] {
        let resource = create(&pool, owner, context, &body_of(sections)).await;
        let blocks: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded",
        )
        .bind(Uuid::from(resource))
        .fetch_one(&pool)
        .await
        .unwrap();
        let mut read = Vec::new();
        let mut hash = Vec::new();
        let mut bytes = 0;
        for _ in 0..RUNS {
            let started = Instant::now();
            let content =
                substrate_read::get_content_select(&pool, ProfileId::from(owner), resource)
                    .await
                    .expect("content");
            read.push(started.elapsed());
            bytes = content.markdown.len();
            let started = Instant::now();
            std::hint::black_box(temper_core::body_token(&content.markdown));
            hash.push(started.elapsed());
        }
        let (r50, r95) = percentiles(read);
        let (h50, h95) = percentiles(hash);
        println!(
            "content: {blocks} blocks, {bytes} bytes: read p50 {r50:?} p95 {r95:?}; \
             token alone p50 {h50:?} p95 {h95:?}"
        );
    }
}

/// The locked read gate (`ensure_visible` on the write's transaction, under R's writer lock) for a
/// principal who reaches R through one team, and for one who is a member of 300 teams, each of
/// which owns a context with a resource in it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
#[ignore = "a measurement, not a gate"]
async fn locked_read_gate_cost_by_team_count(pool: PgPool) {
    let (owner, _) = seed_profile(&pool, "gate-owner@example.com").await;
    for teams in [1usize, 300] {
        let (reader, _) = seed_profile(&pool, &format!("gate-reader-{teams}@example.com")).await;
        let mut target = None;
        for t in 0..teams {
            let team = Uuid::now_v7();
            sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
                .bind(team)
                .bind(format!("t-{teams}-{t}"))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
            )
            .bind(team)
            .bind(reader)
            .execute(&pool)
            .await
            .unwrap();
            let context = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
                 VALUES ($1, 'kb_teams', $2, 'shared', 'shared')",
            )
            .bind(context)
            .bind(team)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
                .bind(context)
                .bind(team)
                .execute(&pool)
                .await
                .unwrap();
            // The owner authors into the team's context; the last team's resource is the target.
            sqlx::query(
                "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner') \
                 ON CONFLICT DO NOTHING",
            )
            .bind(team)
            .bind(owner)
            .execute(&pool)
            .await
            .unwrap();
            target = Some(create(&pool, owner, context, "# Shared\n\nbody\n").await);
        }
        let target = target.unwrap();
        let mut samples = Vec::new();
        for _ in 0..RUNS {
            let mut tx = pool.begin().await.unwrap();
            temper_substrate::writes::lock_resource_for_write(&mut tx, target)
                .await
                .unwrap();
            let started = Instant::now();
            temper_substrate::readback::ensure_visible(&mut *tx, ProfileId::from(reader), target)
                .await
                .expect("the reader reads the target");
            samples.push(started.elapsed());
            tx.rollback().await.unwrap();
        }
        let (p50, p95) = percentiles(samples);
        println!("locked read gate: {teams} teams: p50 {p50:?} p95 {p95:?}");
    }
}
