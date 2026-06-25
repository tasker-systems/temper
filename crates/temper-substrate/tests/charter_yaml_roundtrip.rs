#![cfg(feature = "artifact-tests")]
//! Charter roundtrip: a seed document's telos charter (statement / questions-with-context / framing)
//! written through `load_seed` → `cogmap_genesis` comes back EXACTLY through the role-filtered
//! `resource_blocks` reads — every block, in order, byte-equal to the YAML input (a question with
//! context persists as the documented `question + "\n\n" + context` join). This is the
//! shape-of-the-seed proof: what a charter author writes is what every later reader gets.
//!
//! Resets the artifact, ONNX-dependent, serialized via the temper-substrate-write group.

mod common;

use temper_substrate::scenario::model::Seed;
use temper_substrate::scenario::{bootseed, loader};
use temper_substrate::substrate;
use uuid::Uuid;

const SEED_YAML: &str = r#"
name: charter-roundtrip
cogmap:
  telos:
    title: "Migration charter"
    statement: "Converge the production system onto the proven substrate without breaking a single reader."
    questions:
      - question: "Which reads must stay byte-identical through the cutover?"
        context: "Parity is the migration's safety net: every legacy read answered from the new substrate must match before any surface cuts over."
      - question: "What is the smallest reversible first step?"
    framing:
      - "This map coordinates with the access-scaffold work; leak-safety invariants gate every share."
      - "Migration is replay: backfill happens by genesis-event synthesis, never by table copy."
  owner: alice
  emitter: "migration-agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "migration-agent#1", profile: alice }]
resources: []
uses_lenses: [telos-default]
"#;

async fn role_blocks(pool: &sqlx::PgPool, telos: Uuid, cogmap: Uuid, role: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT body_text FROM resource_blocks($1, 'cogmap', $2, $3) ORDER BY seq")
        .bind(telos)
        .bind(cogmap)
        .bind(role)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn charter_yaml_reproduces_exactly_through_role_filtered_reads() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();

    let seed: Seed = serde_yaml::from_str(SEED_YAML).unwrap();
    let loaded = loader::load_seed(&pool, &seed).await.unwrap();
    let telos = loaded.keys["telos"];

    let statements = role_blocks(&pool, telos, loaded.cogmap, "statement").await;
    assert_eq!(
        statements,
        vec!["Converge the production system onto the proven substrate without breaking a single reader.".to_string()],
        "the statement comes back as exactly one byte-equal statement block"
    );

    let questions = role_blocks(&pool, telos, loaded.cogmap, "question").await;
    assert_eq!(
        questions,
        vec![
            "Which reads must stay byte-identical through the cutover?\n\nParity is the migration's safety net: every legacy read answered from the new substrate must match before any surface cuts over.".to_string(),
            "What is the smallest reversible first step?".to_string(),
        ],
        "questions come back in order: with-context as the documented join, bare question verbatim"
    );

    let framing = role_blocks(&pool, telos, loaded.cogmap, "framing").await;
    assert_eq!(
        framing,
        vec![
            "This map coordinates with the access-scaffold work; leak-safety invariants gate every share.".to_string(),
            "Migration is replay: backfill happens by genesis-event synthesis, never by table copy.".to_string(),
        ],
        "framing blocks come back in order, byte-equal"
    );

    // and nothing else: the charter is exactly these five blocks
    let all: i64 =
        sqlx::query_scalar("SELECT count(*) FROM resource_blocks($1, 'cogmap', $2, NULL)")
            .bind(telos)
            .bind(loaded.cogmap)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(all, 5, "statement + 2 questions + 2 framing, nothing more");
}
