use super::*;
use temper_substrate::scenario::model::Seed;
use temper_substrate::scenario::{bootseed, loader};

const CHARTER_SEED_YAML: &str = r#"
name: charter-select-test
cogmap:
  telos:
    title: "Charter select test"
    statement: "Read the telos charter through the composed read."
    questions:
      - question: "Does the composed read preserve seq order?"
        context: "Statement, then questions, then framing, in that order."
      - question: "Does a bare question round-trip verbatim?"
    framing:
      - "Framing block one."
  owner: alice
  emitter: "charter-agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "charter-agent#1", profile: alice }]
resources: []
uses_lenses: [telos-default]
"#;

#[sqlx::test(migrations = "../../migrations")]
async fn returns_blocks_in_seq_order_and_gates_by_readability(pool: PgPool) {
    bootseed::seed_system(&pool).await.expect("seed_system");

    let seed: Seed = serde_yaml::from_str(CHARTER_SEED_YAML).expect("parse seed yaml");
    let loaded = loader::load_seed(&pool, &seed).await.expect("load_seed");
    let owner = ProfileId::from(loaded.owner);

    let blocks = cogmap_charter_select(&pool, owner, loaded.cogmap)
        .await
        .expect("readable charter select");
    assert_eq!(
        blocks.len(),
        4,
        "statement + 2 questions + 1 framing: {blocks:?}"
    );
    assert_eq!(blocks[0].role, "statement");
    assert_eq!(
        blocks[0].body,
        "Read the telos charter through the composed read."
    );
    assert_eq!(blocks[1].role, "question");
    assert_eq!(
        blocks[1].body,
        "Does the composed read preserve seq order?\n\nStatement, then questions, then framing, in that order."
    );
    assert_eq!(blocks[2].role, "question");
    assert_eq!(blocks[2].body, "Does a bare question round-trip verbatim?");
    assert_eq!(blocks[3].role, "framing");
    assert_eq!(blocks[3].body, "Framing block one.");
    assert!(
        blocks.windows(2).all(|w| w[0].seq < w[1].seq),
        "blocks come back in strictly increasing seq order: {blocks:?}"
    );

    // A profile with no ownership/grant on the telos resource is denied by the in-SQL gate.
    let outsider: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, email) VALUES ($1, $1, $1) RETURNING id",
    )
    .bind("outsider@example.com")
    .fetch_one(&pool)
    .await
    .expect("insert outsider profile");

    let denied = cogmap_charter_select(&pool, ProfileId::from(outsider), loaded.cogmap)
        .await
        .expect("gate denial is empty, not an error");
    assert!(
        denied.is_empty(),
        "non-owner must see no charter blocks: {denied:?}"
    );
}
