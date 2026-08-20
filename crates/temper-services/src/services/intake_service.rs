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
//! This service owns the *matching*, the `references` write, and (S2 chunk C) the projection of
//! the matched set into `kb_subscription_deliveries`. The HTTP transport (S3) is not in scope — a
//! caller hands this service a connection id, a provider event type, and a verbatim payload; this
//! service computes the radius, appends the event, and projects the delivery rows.
//!
//! The delivery rows are a PROJECTION, not a second event (goal C2: one webhook receipt is one
//! ledger event, always). They are written in Rust inside the same transaction as the append —
//! the `region_materialized` precedent, which writes its N member rows in the event's own
//! transaction (`temper-substrate/src/write.rs:196-204`). A payload-first `_project_*` half is
//! unavailable here: the halves read only the payload (`canonical_schema.sql:473`) and this
//! payload is the remote's verbatim body, which does not carry the matched set.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::subscription::SubscriptionSelector;
use temper_substrate::payloads::{AnchorTable, EventRef, RefRel, RefTarget};

use crate::error::{ApiError, ApiResult};
use crate::services::{connection_service, delivery_service};

/// The event type name for a received webhook. Registered permissive (NULL
/// `payload_schema`) by migration `20260819000020` — the payload is the remote's verbatim
/// body, which has no fixed shape temper can publish. `category = 'domain'`: a webhook is
/// the ledger's subject matter, not an authority act.
const WEBHOOK_RECEIVED_TYPE: &str = "webhook_received";

/// Where the provider's event name came from — recorded on the event so the ledger row says
/// where its own routing input originated.
///
/// The event name steers the coarse radius (`GitHubRepository { event_types }`), and the
/// attestation covers **neither** the body nor the headers: `verify_inbound` returns
/// `payload: req.body.to_vec()` with no claim binding content, and the live-captured claim set
/// is `iss/aud/sub/kid/client_id/trigger/exp/iat` (research `019f62e6`). So "an unverified
/// header steering the radius, versus the verified payload" is a false contrast — the
/// attestation authenticates the **connector**, not the content, and header and body are
/// equally the sender's word.
///
/// What is therefore worth recording is not a trust level but a **provenance**: which of the
/// provider's conventions answered. Today only [`Self::Header`] is reachable, so the field is a
/// constant — and that is exactly why it is written now. When a second rule lands, the events
/// that predate it must not be retro-indistinguishable from the ones that used it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTypeSource {
    /// The provider stated its event name in a header of its own convention (GitHub's
    /// `X-GitHub-Event`).
    Header,
}

impl EventTypeSource {
    /// The value written to the event's `metadata`.
    fn as_str(self) -> &'static str {
        match self {
            Self::Header => "header",
        }
    }
}

/// The provider's own event name, together with where it came from. The two travel as one
/// value because a steering input whose provenance is separable from it is a steering input
/// whose provenance gets dropped at the first call site that finds it inconvenient.
#[derive(Debug, Clone, Copy)]
pub struct ProviderEvent<'a> {
    event_type: &'a str,
    source: EventTypeSource,
}

impl<'a> ProviderEvent<'a> {
    /// The provider stated this event name in its own header.
    pub fn from_header(event_type: &'a str) -> Self {
        Self {
            event_type,
            source: EventTypeSource::Header,
        }
    }
}

/// The header a provider states its event name in, or `None` where temper has no witnessed rule
/// for that provider.
///
/// `github` is the only entry, and the omission of every other provider is deliberate. Linear
/// and Slack carry an event name in the *body*, but no forward from either has been observed
/// through this path — and writing a payload-shape classifier from documentation would be temper
/// inventing an opinion about a provider's taxonomy and presenting it as the provider's
/// statement. A missing rule is a declared hole; a guessed rule is a silent one.
///
/// `provider` comes from the attestation's **signed** `trigger` claim, so the dispatch key is the
/// one part of the request that is attested — the same discipline as reading the connector from
/// the JWT rather than the unsigned `x-trigger-*` mirror headers
/// (`broker/vercel_connect.rs:475-476`).
pub fn event_type_header(provider: &str) -> Option<&'static str> {
    match provider {
        "github" => Some("x-github-event"),
        _ => None,
    }
}

/// Receive a webhook: compute the coarse radius, append one event with the matched
/// subscribers in `references`.
///
/// `connection_id` identifies the connection the payload arrived on. `event` carries the
/// remote's own event name (e.g. GitHub's `pull_request`, Linear's `issue.updated`) and the
/// provenance of that name ([`EventTypeSource`]) — both ride `metadata`, not `payload`, so the
/// verbatim payload is preserved untouched. `payload` is the remote's verbatim body, stored
/// as-is in `kb_events.payload`.
///
/// Returns the appended event id. Never performs egress. Never UPDATEs `kb_events`.
pub async fn receive_webhook(
    pool: &PgPool,
    connection_id: Uuid,
    event: ProviderEvent<'_>,
    payload: &serde_json::Value,
) -> ApiResult<Uuid> {
    let provider_event_type = event.event_type;
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
    let matched = match_subscriptions(&subs, provider_event_type, payload);

    // Serialize the references to JSONB. An empty array is `[]` — the noise filter: a
    // payload matching zero subscriptions is stored and routes nowhere.
    let references_json = serde_json::to_value(&matched.references)
        .map_err(|e| ApiError::Internal(format!("references serialization failed: {e}")))?;

    // The event and its delivery rows move together. A webhook either lands as one event with
    // its full fan of deliveries or lands not at all — there is no window in which the routing
    // exists and the rows that make it readable do not.
    let mut tx = pool.begin().await?;

    // One INSERT via the chokepoint writer. `_event_append` looks up the event type's
    // category from the registry, generates the event id, and inserts. The producing
    // anchor is the connection's home context (kb_contexts). The references are passed
    // here — never as a second statement.
    //
    // A `query_scalar!` macro (not runtime `query_scalar`): the statement is fully static,
    // so this is the compile-time-checked form the sqlx-macro-exceptions tripwire expects.
    // `_event_append` returns uuid (non-nullable — it always generates an id), but
    // `query_scalar!` wraps scalars in Option (a query could return no rows), so the
    // `.expect` is the assertion that the function did return.
    let event_id: Uuid = sqlx::query_scalar!(
        "SELECT _event_append($1, $2, 'kb_contexts', $3, $4, $5, $6, $7, $8)",
        WEBHOOK_RECEIVED_TYPE,
        conn.emitter_entity_id,
        conn.home_context_id,
        payload,
        &references_json,
        None::<Uuid> as Option<Uuid>, // correlation: self-roots inside _event_append
        1i32,                         // payload_version
        // metadata: the event name AND where it came from. See `EventTypeSource` — the
        // provenance is written even though only one source is reachable today, so events
        // that predate a second rule are not retro-indistinguishable from ones that used it.
        serde_json::json!({
            "provider_event_type": provider_event_type,
            "provider_event_type_source": event.source.as_str(),
        }),
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| ApiError::Internal(format!("_event_append failed: {e}")))?
    .expect("_event_append always returns a uuid");

    // Project one delivery row per matched subscription (S2 chunk C). A payload matching zero
    // subscriptions projects zero rows — the empty radius is the noise filter, and a
    // routed-nowhere payload is a well-formed act the system said no to, not an error.
    delivery_service::project(&mut tx, event_id, &matched.subscription_ids).await?;

    tx.commit().await?;

    Ok(event_id)
}

/// One live subscription row for matching, with the deserialized selector. The subscriber's
/// `subscriber_table` → `AnchorTable` mapping is resolved once here, not per match.
struct LiveSubscription {
    /// The subscription's own id. Carried because the delivery row keys on it and `references`
    /// cannot recover it: two subscriptions of the same subscriber produce two `touches` entries
    /// with identical targets, so the fan is not invertible back to declarations.
    id: Uuid,
    subscriber_table: String,
    subscriber_id: Uuid,
    selector: SubscriptionSelector,
}

/// What the coarse match produced: the `references` fan for the event, and the declarations those
/// entries came from. The two are parallel but NOT interchangeable — see [`LiveSubscription::id`].
struct MatchedRadius {
    references: Vec<EventRef>,
    subscription_ids: Vec<Uuid>,
}

/// Fetch all live (non-revoked) subscriptions for a connection, deserializing each selector.
/// The `connection_id` leg is indexed; the result set is expected to be small (a team has a
/// few subscriptions against a connection), so deserializing in Rust is cheap.
async fn live_subscriptions_for_connection(
    pool: &PgPool,
    connection_id: Uuid,
) -> ApiResult<Vec<LiveSubscription>> {
    let rows = sqlx::query!(
        r#"SELECT id, subscriber_table, subscriber_id, selector
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
            id: row.id,
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
) -> MatchedRadius {
    let mut references = Vec::new();
    let mut subscription_ids = Vec::new();
    for sub in subs {
        if selector_matches(&sub.selector, provider_event_type, payload) {
            let kind = subscriber_table_to_anchor(&sub.subscriber_table);
            references.push(EventRef {
                rel: RefRel::Touches,
                target: RefTarget {
                    kind,
                    id: sub.subscriber_id,
                },
            });
            subscription_ids.push(sub.id);
        }
    }
    MatchedRadius {
        references,
        subscription_ids,
    }
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
        let sub_id =
            create_subscription(&pool, admin, "kb_teams", team, team, conn, selector).await;

        // Revoke the subscription.
        crate::services::subscription_service::revoke(&pool, admin, sub_id)
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
}
