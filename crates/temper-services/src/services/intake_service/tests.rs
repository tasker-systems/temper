use super::*;
use sqlx::PgPool;
use temper_core::types::subscription::SubscriptionSelector;

use crate::services::subscription_test_support::*;

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn payload_matching_n_subscriptions_produces_one_event_with_n_touches(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team_a = seed_team(&pool, admin).await;
    let team_b = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team_a), admin).await;
    grant_reach(&pool, admin, conn, team_a).await;
    grant_reach(&pool, admin, conn, team_b).await;

    let selector = SubscriptionSelector::GitHubRepository {
        repo: GITHUB_REPO.into(),
        event_types: vec!["pull_request".into()],
    };
    create_subscription(
        &pool,
        admin,
        "kb_teams",
        team_a,
        team_a,
        conn,
        selector.clone(),
    )
    .await;
    create_subscription(
        &pool,
        admin,
        "kb_teams",
        team_b,
        team_b,
        conn,
        selector.clone(),
    )
    .await;

    let event_id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive webhook");

    // Exactly one event row (goal C2: one webhook receipt is one ledger event, always).
    assert_eq!(event_count_for_connection(&pool, conn).await, 1);

    // Two touches references (goal C4: blast radius = matched subscriptions).
    let refs = event_references(&pool, event_id).await;
    let arr = refs.as_array().expect("references is an array");
    assert_eq!(
        arr.len(),
        2,
        "two matched subscriptions => two touches entries"
    );
    for entry in arr {
        assert_eq!(entry.get("rel").and_then(|v| v.as_str()), Some("touches"));
        let kind = entry
            .get("target")
            .and_then(|t| t.get("kind"))
            .and_then(|k| k.as_str());
        assert!(
            kind == Some("kb_teams"),
            "subscriber kind is kb_teams, got {kind:?}"
        );
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn payload_matching_zero_subscriptions_produces_one_event_with_empty_references(
    pool: PgPool,
) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    // A subscription for a DIFFERENT repo — the payload below is for acme/temper, this
    // subscription is for acme/other-repo, so it does not match.
    let selector = SubscriptionSelector::GitHubRepository {
        repo: "acme/other-repo".into(),
        event_types: vec!["pull_request".into()],
    };
    create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

    let event_id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive webhook");

    // One event (C2: one webhook receipt is one ledger event, even a refused one).
    assert_eq!(event_count_for_connection(&pool, conn).await, 1);

    // references = [] — the empty radius is the noise filter (C4).
    let refs = event_references(&pool, event_id).await;
    let arr = refs.as_array().expect("references is an array");
    assert!(
        arr.is_empty(),
        "zero matches => empty references, got {refs}"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn github_repository_selector_matches_on_repo_and_event_type(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let selector = SubscriptionSelector::GitHubRepository {
        repo: GITHUB_REPO.into(),
        event_types: vec!["pull_request".into()],
    };
    create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

    // Matching event type.
    let id_match = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("match");
    let refs = event_references(&pool, id_match).await;
    assert_eq!(refs.as_array().unwrap().len(), 1);

    // Non-matching event type (push vs pull_request).
    let id_no_match = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("push"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive");
    let refs = event_references(&pool, id_no_match).await;
    assert!(
        refs.as_array().unwrap().is_empty(),
        "event_type mismatch => no match"
    );

    // Non-matching repo.
    let id_repo_no_match = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload("acme/other"),
    )
    .await
    .expect("receive");
    let refs = event_references(&pool, id_repo_no_match).await;
    assert!(
        refs.as_array().unwrap().is_empty(),
        "repo mismatch => no match"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn github_repository_selector_with_empty_event_types_matches_all(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let selector = SubscriptionSelector::GitHubRepository {
        repo: GITHUB_REPO.into(),
        event_types: vec![], // empty = match all
    };
    create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

    let id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("push"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive");
    let refs = event_references(&pool, id).await;
    assert_eq!(
        refs.as_array().unwrap().len(),
        1,
        "empty event_types matches any event_type"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn github_codeowners_paths_matches_on_repo_at_intake(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    // A CodeownersPaths selector: coarse match is on repo only. The paths need the
    // changed-file list (enrichment, S4), which the pull_request payload does not carry.
    let selector = SubscriptionSelector::GitHubCodeownersPaths {
        repo: GITHUB_REPO.into(),
        paths: vec!["src/api/**".into()],
    };
    create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

    let id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive");
    let refs = event_references(&pool, id).await;
    assert_eq!(
        refs.as_array().unwrap().len(),
        1,
        "CodeownersPaths matches on repo at intake; the fine radius is enrichment (S4)"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn linear_project_selector_matches_on_project_id(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let selector = SubscriptionSelector::LinearProject {
        project_id: "proj-123".into(),
    };
    create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

    // Matching project.
    let id_match = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("issue.updated"),
        &linear_issue_payload("proj-123"),
    )
    .await
    .expect("receive");
    let refs = event_references(&pool, id_match).await;
    assert_eq!(refs.as_array().unwrap().len(), 1);

    // Non-matching project.
    let id_no_match = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("issue.updated"),
        &linear_issue_payload("proj-456"),
    )
    .await
    .expect("receive");
    let refs = event_references(&pool, id_no_match).await;
    assert!(refs.as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoked_subscription_does_not_match(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let selector = SubscriptionSelector::GitHubRepository {
        repo: GITHUB_REPO.into(),
        event_types: vec!["pull_request".into()],
    };
    let sub_id = create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

    // Revoke the subscription.
    crate::services::subscription_service::revoke(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        sub_id,
    )
    .await
    .expect("revoke");

    let id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive");
    let refs = event_references(&pool, id).await;
    assert!(
        refs.as_array().unwrap().is_empty(),
        "a revoked subscription does not match"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn references_are_written_on_insert_not_as_an_update(pool: PgPool) {
    // The trap: writing references as an UPDATE after the event is INSERTed. kb_events
    // is append-only (a BEFORE UPDATE OR DELETE trigger raises), so the references
    // MUST be populated in the INSERT. This test witnesses that by asserting the
    // references column is non-empty on the very row that was inserted — there is no
    // second statement that could have set it.
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let selector = SubscriptionSelector::GitHubRepository {
        repo: GITHUB_REPO.into(),
        event_types: vec!["pull_request".into()],
    };
    create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

    let event_id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive");

    // The references column is populated on the inserted row. The append-only trigger
    // would refuse any UPDATE, so this is the proof the references were written in the
    // INSERT, not after.
    let refs = event_references(&pool, event_id).await;
    assert_eq!(
        refs.as_array().unwrap().len(),
        1,
        "references written on INSERT, not as a subsequent UPDATE"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn context_subscriber_produces_kb_contexts_touches_entry(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    let context_id = seed_context_owned_by_team(&pool, team).await;

    let selector = SubscriptionSelector::GitHubRepository {
        repo: GITHUB_REPO.into(),
        event_types: vec!["pull_request".into()],
    };
    create_subscription(
        &pool,
        admin,
        "kb_contexts",
        context_id,
        team,
        conn,
        selector,
    )
    .await;

    let event_id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive");
    let refs = event_references(&pool, event_id).await;
    let arr = refs.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let entry = &arr[0];
    assert_eq!(entry.get("rel").and_then(|v| v.as_str()), Some("touches"));
    assert_eq!(
        entry
            .get("target")
            .and_then(|t| t.get("kind"))
            .and_then(|k| k.as_str()),
        Some("kb_contexts"),
        "a kb_contexts subscriber produces a touches entry with kind=kb_contexts"
    );
    let entry_id = entry
        .get("target")
        .and_then(|t| t.get("id"))
        .and_then(|i| i.as_str());
    assert_eq!(entry_id, Some(context_id.to_string().as_str()));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn payload_is_preserved_verbatim_on_the_event_row(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let payload = github_pr_payload(GITHUB_REPO);
    let event_id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &payload,
    )
    .await
    .expect("receive");

    let stored = sqlx::query!(
        r#"SELECT payload, payload_version FROM kb_events WHERE id = $1"#,
        event_id,
    )
    .fetch_one(&pool)
    .await
    .expect("read payload");

    assert_eq!(stored.payload_version, WEBHOOK_PAYLOAD_VERSION);
    assert_eq!(
        stored.payload,
        serde_json::json!({ "body": payload }),
        "the body is stored whole under `body`"
    );
    assert_eq!(
        webhook_body(&stored.payload, stored.payload_version),
        Some(&payload),
        "payload preserved verbatim"
    );
}

/// A version-1 row, written before intake wrapped bodies, is its own body.
#[test]
fn webhook_body_reads_both_versions() {
    let body = serde_json::json!({ "action": "opened" });
    assert_eq!(webhook_body(&body, 1), Some(&body));
    assert_eq!(
        webhook_body(
            &serde_json::json!({ "body": body }),
            WEBHOOK_PAYLOAD_VERSION
        ),
        Some(&body)
    );
    assert_eq!(
        webhook_body(&body, 3),
        None,
        "an unknown version is not guessed at"
    );
}

/// A remote body carrying a top-level key the ledger reads as an id is received and routes.
/// Stored bare, a non-uuid value failed the `kb_events` expression index's cast, so intake
/// refused the webhook outright.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_body_carrying_an_id_key_is_received_and_routes(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    create_subscription(
        &pool,
        admin,
        "kb_teams",
        team,
        team,
        conn,
        SubscriptionSelector::GitHubRepository {
            repo: GITHUB_REPO.into(),
            event_types: vec!["pull_request".into()],
        },
    )
    .await;

    let cases = [
        ("resource_id", serde_json::json!("PROJ-123")),
        ("block_id", serde_json::json!(42)),
        ("edge_id", serde_json::json!("not-a-uuid")),
        ("owner", serde_json::json!({ "id": "octocat" })),
        // Stored bare, a uuid value would file the webhook under that resource's trail.
        ("resource_id", serde_json::json!(Uuid::now_v7())),
    ];
    for (key, value) in cases {
        let mut payload = github_pr_payload(GITHUB_REPO);
        payload[key] = value;
        let event_id = receive_webhook(
            &pool,
            conn,
            ProviderEvent::from_header("pull_request"),
            &payload,
        )
        .await
        .unwrap_or_else(|e| panic!("a body with top-level {key} is received: {e:?}"));

        let stored: serde_json::Value =
            sqlx::query_scalar!(r#"SELECT payload FROM kb_events WHERE id = $1"#, event_id)
                .fetch_one(&pool)
                .await
                .expect("read event");
        assert_eq!(
            stored,
            serde_json::json!({ "body": payload }),
            "{key}: the body is stored whole under `body`"
        );
        let deliveries: i64 = sqlx::query_scalar!(
            r#"SELECT count(*) AS "c!" FROM kb_subscription_deliveries WHERE event_id = $1"#,
            event_id
        )
        .fetch_one(&pool)
        .await
        .expect("count deliveries");
        assert_eq!(deliveries, 1, "{key}: routing reads the original body");
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn provider_event_type_rides_metadata_not_payload(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    let event_id = receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive");

    let metadata: serde_json::Value =
        sqlx::query_scalar!(r#"SELECT metadata FROM kb_events WHERE id = $1"#, event_id,)
            .fetch_one(&pool)
            .await
            .expect("read metadata");

    assert_eq!(
        metadata.get("provider_event_type").and_then(|v| v.as_str()),
        Some("pull_request"),
        "provider event type rides metadata, not payload"
    );
}

/// Present one PR webhook under the attestation keyed `replay_key`, at `now`.
async fn present(pool: &PgPool, conn: Uuid, replay_key: &str, now: i64, enforced: bool) -> Receipt {
    receive_attested_webhook(
        pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
        AttestationSeen {
            replay_key,
            expires_at: now + 3_600,
            now,
            enforced,
        },
    )
    .await
    .expect("receive")
}

/// Enforced, a repeat up to [`ATTESTATION_REPEAT_WINDOW_SECS`] after the first acceptance is
/// one of Connect's bursts and lands; a repeat a second later is a replay and lands nothing.
/// The window is measured from the first acceptance, not the latest repeat, so a burst cannot
/// keep an attestation alive.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_enforced_receipt_accepts_a_repeat_within_the_window_and_refuses_one_after(
    pool: PgPool,
) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    let t = 1_750_000_000;

    for at in [t, t + 1, t + ATTESTATION_REPEAT_WINDOW_SECS] {
        assert!(
            matches!(
                present(&pool, conn, "k", at, true).await,
                Receipt::Recorded(_)
            ),
            "a repeat {}s after the first is the same burst",
            at - t
        );
    }
    assert_eq!(
        present(
            &pool,
            conn,
            "k",
            t + ATTESTATION_REPEAT_WINDOW_SECS + 1,
            true
        )
        .await,
        Receipt::RefusedAsReplay,
        "past the window, measured from the first acceptance"
    );
    assert_eq!(event_count_for_connection(&pool, conn).await, 3);
}

/// Observe mode: a replay past the window is recorded and reported.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_observing_receipt_records_a_replay_past_the_window(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    let t = 1_750_000_000;

    present(&pool, conn, "k", t, false).await;
    assert!(matches!(
        present(&pool, conn, "k", t + 3_000, false).await,
        Receipt::Recorded(_)
    ));
    assert_eq!(event_count_for_connection(&pool, conn).await, 2);
}

#[test]
fn a_repeat_is_a_burst_up_to_the_window_and_a_replay_after() {
    assert_eq!(Claim::of_repeat(0), Claim::Burst { age_secs: 0 });
    assert_eq!(
        Claim::of_repeat(ATTESTATION_REPEAT_WINDOW_SECS),
        Claim::Burst {
            age_secs: ATTESTATION_REPEAT_WINDOW_SECS
        }
    );
    assert_eq!(
        Claim::of_repeat(ATTESTATION_REPEAT_WINDOW_SECS + 1),
        Claim::Replay {
            age_secs: Some(ATTESTATION_REPEAT_WINDOW_SECS + 1)
        }
    );
    // An instance whose clock runs behind the first acceptance's sees a negative age.
    assert_eq!(Claim::of_repeat(-2), Claim::Burst { age_secs: -2 });
    assert_eq!(
        Claim::of_repeat(-ATTESTATION_REPEAT_WINDOW_SECS - 1),
        Claim::Replay {
            age_secs: Some(-ATTESTATION_REPEAT_WINDOW_SECS - 1)
        }
    );
}
