//! Webhook intake — the radius-matching service and the first-ever writer of
//! `kb_events.references` (S2 chunk B of "external systems as subscribed emitters").
//!
//! One webhook receipt is one ledger event, always (goal C2). The matched subscribers ride
//! `references` (the fan) — computed at intake BEFORE the INSERT, because `kb_events` is
//! append-only (a `BEFORE UPDATE OR DELETE` trigger raises unconditionally, see
//! `migrations/20260624000001_canonical_schema.sql:504-506`). Writing `references` as an
//! UPDATE after the event is INSERTed is the trap this service avoids by construction: the
//! matched set is built in Rust, serialized to JSONB, and passed to `_event_append` in the
//! single INSERT. There is no second statement.
//!
//! Receipt produces NO EGRESS to the remote (goal C3). This is coarse radius — payload-only.
//! The fine radius (CODEOWNERS paths) is enrichment (S4), not intake. A `GitHubCodeownersPaths`
//! subscription matches on repo at intake and gets a `touches` reference; the path filter is
//! applied post-enrichment. A subscription that never gets enriched stays `undetermined` —
//! visible, never silently `out_of_scope` (goal invariant 6).
//!
//! The empty radius is the noise filter (goal C4): a payload matching zero subscriptions is
//! stored with `references = []` and routes nowhere. It is not an error — it is a well-formed
//! act the system said no to, and it is itself auditable (it is in the ledger).
//!
//! This service owns the *matching* and the `references` write. The HTTP transport (S3) is
//! not in scope — a caller hands this service a connection id, a provider event type, and a
//! verbatim payload; this service computes the radius and appends the event.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::subscription::SubscriptionSelector;
use temper_substrate::payloads::{AnchorTable, EventRef, RefRel, RefTarget};

use crate::error::{ApiError, ApiResult};
use crate::services::connection_service;

/// The event type name for a received webhook. Registered permissive (NULL
/// `payload_schema`) by migration `20260819000020` — the payload is the remote's verbatim
/// body, which has no fixed shape temper can publish. `category = 'domain'`: a webhook is
/// the ledger's subject matter, not an authority act.
const WEBHOOK_RECEIVED_TYPE: &str = "webhook_received";

/// Receive a webhook: compute the coarse radius, append one event with the matched
/// subscribers in `references`.
///
/// `connection_id` identifies the connection the payload arrived on. `provider_event_type`
/// is the remote's own event name (e.g. GitHub's `pull_request`, Linear's `issue.updated`)
/// — it rides `metadata`, not `payload`, so the verbatim payload is preserved untouched.
/// `payload` is the remote's verbatim body, stored as-is in `kb_events.payload`.
///
/// Returns the appended event id. Never performs egress. Never UPDATEs `kb_events`.
pub async fn receive_webhook(
    pool: &PgPool,
    connection_id: Uuid,
    provider_event_type: &str,
    payload: &serde_json::Value,
) -> ApiResult<Uuid> {
    // Load the connection: emitter_entity_id + home_context_id. The emitter is the
    // connection's own entity (`<handle>@webhook`); the producing anchor is the
    // connection's home context — one event, one anchor, the receipt fact in one place.
    let conn = connection_service::get(pool, connection_id).await?;

    // Coarse radius: query the live subscriptions for this connection. The
    // idx_kb_subscriptions_connection partial index (WHERE revoked_at IS NULL) is the hot
    // path — this is the indexed lookup goal C4 names.
    let subs = live_subscriptions_for_connection(pool, connection_id).await?;

    // Build the matched references: one `touches` entry per subscription whose selector
    // matches the payload. Computed BEFORE the INSERT — kb_events is append-only, so the
    // references cannot be UPDATEd after.
    let references = match_subscriptions(&subs, provider_event_type, payload);

    // Serialize the references to JSONB. An empty array is `[]` — the noise filter: a
    // payload matching zero subscriptions is stored and routes nowhere.
    let references_json = serde_json::to_value(&references)
        .map_err(|e| ApiError::Internal(format!("references serialization failed: {e}")))?;

    // One INSERT via the chokepoint writer. `_event_append` looks up the event type's
    // category from the registry, generates the event id, and inserts. The producing
    // anchor is the connection's home context (kb_contexts). The references are passed
    // here — never as a second statement.
    let event_id: Uuid =
        sqlx::query_scalar("SELECT _event_append($1, $2, 'kb_contexts', $3, $4, $5, $6, $7, $8)")
            .bind(WEBHOOK_RECEIVED_TYPE)
            .bind(conn.emitter_entity_id)
            .bind(conn.home_context_id)
            .bind(payload)
            .bind(&references_json)
            .bind::<Option<Uuid>>(None) // correlation: self-roots inside _event_append
            .bind(1) // payload_version
            .bind(serde_json::json!({ "provider_event_type": provider_event_type })) // metadata
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Internal(format!("_event_append failed: {e}")))?;

    Ok(event_id)
}

/// One live subscription row for matching, with the deserialized selector. The subscriber's
/// `subscriber_table` → `AnchorTable` mapping is resolved once here, not per match.
struct LiveSubscription {
    subscriber_table: String,
    subscriber_id: Uuid,
    selector: SubscriptionSelector,
}

/// Fetch all live (non-revoked) subscriptions for a connection, deserializing each selector.
/// The `connection_id` leg is indexed; the result set is expected to be small (a team has a
/// few subscriptions against a connection), so deserializing in Rust is cheap.
async fn live_subscriptions_for_connection(
    pool: &PgPool,
    connection_id: Uuid,
) -> ApiResult<Vec<LiveSubscription>> {
    let rows = sqlx::query!(
        r#"SELECT subscriber_table, subscriber_id, selector
             FROM kb_subscriptions
            WHERE connection_id = $1 AND revoked_at IS NULL"#,
        connection_id,
    )
    .fetch_all(pool)
    .await?;

    let mut subs = Vec::with_capacity(rows.len());
    for row in rows {
        let selector: SubscriptionSelector = serde_json::from_value(row.selector).map_err(|e| {
            ApiError::Internal(format!(
                "subscription {} has unparseable selector: {e}",
                row.subscriber_id
            ))
        })?;
        subs.push(LiveSubscription {
            subscriber_table: row.subscriber_table,
            subscriber_id: row.subscriber_id,
            selector,
        });
    }
    Ok(subs)
}

/// Compute the matched references: one `touches` entry per subscription whose selector
/// matches the payload at the coarse (payload-only) radius. No egress. The fine radius
/// (CODEOWNERS paths) is enrichment (S4).
fn match_subscriptions(
    subs: &[LiveSubscription],
    provider_event_type: &str,
    payload: &serde_json::Value,
) -> Vec<EventRef> {
    let mut refs = Vec::new();
    for sub in subs {
        if selector_matches(&sub.selector, provider_event_type, payload) {
            let kind = subscriber_table_to_anchor(&sub.subscriber_table);
            refs.push(EventRef {
                rel: RefRel::Touches,
                target: RefTarget {
                    kind,
                    id: sub.subscriber_id,
                },
            });
        }
    }
    refs
}

/// Coarse radius matching per selector variant. Payload-only — no fetch, no egress.
fn selector_matches(
    selector: &SubscriptionSelector,
    provider_event_type: &str,
    payload: &serde_json::Value,
) -> bool {
    match selector {
        SubscriptionSelector::GitHubRepository { repo, event_types } => {
            // GitHub's webhook payload carries repository.full_name as "owner/repo".
            let payload_repo = payload
                .get("repository")
                .and_then(|r| r.get("full_name"))
                .and_then(|n| n.as_str());
            let Some(payload_repo) = payload_repo else {
                return false;
            };
            if payload_repo != repo {
                return false;
            }
            // Empty event_types = match all registered event types on the connection.
            if event_types.is_empty() {
                return true;
            }
            // GitHub's event type rides the X-GitHub-Event header, passed here as
            // provider_event_type. Match against the selector's list.
            event_types.iter().any(|et| et == provider_event_type)
        }
        SubscriptionSelector::GitHubCodeownersPaths { repo, .. } => {
            // Coarse match on repo only. The paths need the changed-file list, which
            // GitHub's pull_request payload does not carry — that is enrichment (S4).
            // At intake, a CodeownersPaths subscription matches on repo and gets a
            // `touches` reference; the fine radius is applied post-enrichment. A
            // subscription that never gets enriched stays `undetermined` (invariant 6).
            let payload_repo = payload
                .get("repository")
                .and_then(|r| r.get("full_name"))
                .and_then(|n| n.as_str());
            payload_repo == Some(repo.as_str())
        }
        SubscriptionSelector::LinearProject { project_id } => {
            // Linear's issue webhook carries the project id under data.project.id.
            let payload_project = payload
                .get("data")
                .and_then(|d| d.get("project"))
                .and_then(|p| p.get("id"))
                .and_then(|i| i.as_str());
            payload_project == Some(project_id.as_str())
        }
    }
}

/// Map the subscription's `subscriber_table` to the `AnchorTable` variant chunk B writes as
/// the `touches` rel's `target.kind`. The three admissible subscriber tables map 1:1 to the
/// three `AnchorTable` variants (spec D5); an unknown table is a data error (the CHECK on
/// `kb_subscriptions.subscriber_table` makes this unreachable for rows in the table, but a
/// deserialized row could in principle carry a stale string).
fn subscriber_table_to_anchor(subscriber_table: &str) -> AnchorTable {
    match subscriber_table {
        "kb_contexts" => AnchorTable::Contexts,
        "kb_cogmaps" => AnchorTable::Cogmaps,
        "kb_teams" => AnchorTable::Teams,
        other => unreachable!(
            "subscriber_table '{other}' is not admissible; the CHECK on \
             kb_subscriptions.subscriber_table should have refused it"
        ),
    }
}

// ── tests ───────────────────────────────────────────────────────────────────
//
// Run the tests you wrote or changed, the neighbouring ones in the same file/crate, and
// anything that regenerates a committed artifact. CI owns the broad suites.
// (`cargo nextest run -p temper-services --features test-db intake_service`)

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use temper_core::types::connection::ProvisionConnectionRequest;
    use temper_core::types::ids::ProfileId;
    use temper_core::types::subscription::{CreateSubscriptionRequest, SubscriptionSelector};

    const GITHUB_REPO: &str = "acme/temper";

    /// Seed a system admin and return its profile id. Mirrors subscription_service::seed_admin.
    async fn seed_admin(pool: &PgPool) -> ProfileId {
        let id = Uuid::now_v7();
        let handle = format!("admin-{id}");
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)",
            id,
            &handle,
            &handle,
        )
        .execute(pool)
        .await
        .expect("seed admin profile");

        let mut conn = pool.acquire().await.expect("acquire");
        crate::services::profile_service::provision_profile_entities(&mut conn, id, &handle)
            .await
            .expect("provision caller emitters");
        drop(conn);

        let team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System') \
             ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name \
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("gating team");

        sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
            .execute(pool)
            .await
            .expect("configure gating team");

        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'owner'::team_role) \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
            team,
            id,
        )
        .execute(pool)
        .await
        .expect("join gating team as owner");

        crate::test_support::approved_admin(pool, id).await;
        ProfileId::from(id)
    }

    async fn seed_team(pool: &PgPool, owner: ProfileId) -> Uuid {
        let team_id = Uuid::now_v7();
        let slug = format!("team-{team_id}");
        sqlx::query!(
            r#"INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $3)"#,
            team_id,
            &slug,
            &slug,
        )
        .execute(pool)
        .await
        .expect("seed team");
        sqlx::query!(
            r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, 'owner'::team_role)"#,
            team_id,
            *owner,
        )
        .execute(pool)
        .await
        .expect("seed team owner");
        team_id
    }

    async fn seed_context_owned_by_team(pool: &PgPool, team_id: Uuid) -> Uuid {
        let context_id = Uuid::now_v7();
        let slug = format!("ctx-{context_id}");
        sqlx::query!(
            r#"INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
               VALUES ($1, 'kb_teams', $2, $3, $4)"#,
            context_id,
            team_id,
            &slug,
            &slug,
        )
        .execute(pool)
        .await
        .expect("seed context");
        sqlx::query!(
            r#"INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)"#,
            context_id,
            team_id,
        )
        .execute(pool)
        .await
        .expect("seed team_contexts");
        context_id
    }

    async fn seed_connection(
        pool: &PgPool,
        owner_team_id: Option<Uuid>,
        caller: ProfileId,
    ) -> Uuid {
        let req = ProvisionConnectionRequest {
            provider: "github".into(),
            name: format!("test-conn-{}", Uuid::now_v7()),
            owner_team_id,
            reach_granularity: None,
            reach_covers: None,
        };
        let conn = crate::services::connection_service::provision(pool, caller, &req)
            .await
            .expect("seed connection");
        conn.id
    }

    async fn grant_reach(pool: &PgPool, caller: ProfileId, connection_id: Uuid, team_id: Uuid) {
        crate::services::connection_service::grant_reach(
            pool,
            caller,
            connection_id,
            team_id,
            None,
        )
        .await
        .expect("grant reach");
    }

    async fn create_subscription(
        pool: &PgPool,
        caller: ProfileId,
        subscriber_table: &str,
        subscriber_id: Uuid,
        authoring_team_id: Uuid,
        connection_id: Uuid,
        selector: SubscriptionSelector,
    ) -> Uuid {
        let req = CreateSubscriptionRequest {
            subscriber_table: subscriber_table.into(),
            subscriber_id,
            authoring_team_id,
            connection_id,
            selector,
        };
        let sub = crate::services::subscription_service::create(pool, caller, &req)
            .await
            .expect("create subscription");
        sub.id
    }

    /// A GitHub pull_request webhook payload for `acme/temper`.
    fn github_pr_payload(repo: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "opened",
            "repository": { "full_name": repo },
            "pull_request": { "number": 42 }
        })
    }

    /// A Linear issue webhook payload for project `proj-123`.
    fn linear_issue_payload(project_id: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "update",
            "data": { "project": { "id": project_id } }
        })
    }

    /// Read the references column for an event id.
    async fn event_references(pool: &PgPool, event_id: Uuid) -> serde_json::Value {
        sqlx::query_scalar!(
            r#"SELECT "references" FROM kb_events WHERE id = $1"#,
            event_id,
        )
        .fetch_one(pool)
        .await
        .expect("read references")
    }

    /// Count kb_events rows for a connection's emitter.
    async fn event_count_for_connection(pool: &PgPool, connection_id: Uuid) -> i64 {
        let count: Option<i64> = sqlx::query_scalar!(
            r#"SELECT count(*)::bigint FROM kb_events e
                JOIN kb_connections c ON c.id = $1 AND e.emitter_entity_id = c.emitter_entity_id"#,
            connection_id,
        )
        .fetch_one(pool)
        .await
        .expect("count events");
        count.unwrap_or(0)
    }

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

        let event_id =
            receive_webhook(&pool, conn, "pull_request", &github_pr_payload(GITHUB_REPO))
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

        let event_id =
            receive_webhook(&pool, conn, "pull_request", &github_pr_payload(GITHUB_REPO))
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
        let id_match =
            receive_webhook(&pool, conn, "pull_request", &github_pr_payload(GITHUB_REPO))
                .await
                .expect("match");
        let refs = event_references(&pool, id_match).await;
        assert_eq!(refs.as_array().unwrap().len(), 1);

        // Non-matching event type (push vs pull_request).
        let id_no_match = receive_webhook(&pool, conn, "push", &github_pr_payload(GITHUB_REPO))
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
            "pull_request",
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

        let id = receive_webhook(&pool, conn, "push", &github_pr_payload(GITHUB_REPO))
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

        let id = receive_webhook(&pool, conn, "pull_request", &github_pr_payload(GITHUB_REPO))
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
            "issue.updated",
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
            "issue.updated",
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
        let sub_id =
            create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

        // Revoke the subscription.
        crate::services::subscription_service::revoke(&pool, admin, sub_id)
            .await
            .expect("revoke");

        let id = receive_webhook(&pool, conn, "pull_request", &github_pr_payload(GITHUB_REPO))
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

        let event_id =
            receive_webhook(&pool, conn, "pull_request", &github_pr_payload(GITHUB_REPO))
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

        let event_id =
            receive_webhook(&pool, conn, "pull_request", &github_pr_payload(GITHUB_REPO))
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
        let event_id = receive_webhook(&pool, conn, "pull_request", &payload)
            .await
            .expect("receive");

        let stored: serde_json::Value =
            sqlx::query_scalar!(r#"SELECT payload FROM kb_events WHERE id = $1"#, event_id,)
                .fetch_one(&pool)
                .await
                .expect("read payload");

        assert_eq!(stored, payload, "payload preserved verbatim");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provider_event_type_rides_metadata_not_payload(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        let event_id =
            receive_webhook(&pool, conn, "pull_request", &github_pr_payload(GITHUB_REPO))
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
}
