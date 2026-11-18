use super::*;
use crate::services::intake_service::{self, ProviderEvent};
use crate::services::subscription_test_support::*;
use sqlx::PgPool;
use temper_core::types::subscription::SubscriptionSelector;

/// The whole world one delivery needs: admin, team, team-owned context, connection with a
/// reach grant, and a subscription. Returns (admin, team, context, connection, subscription).
async fn seed_world(pool: &PgPool) -> (ProfileId, Uuid, Uuid, Uuid, Uuid) {
    let admin = seed_admin(pool).await;
    let team = seed_team(pool, admin).await;
    let ctx = seed_context_owned_by_team(pool, team).await;
    let conn = seed_connection(pool, Some(team), admin).await;
    grant_reach(pool, admin, conn, team).await;
    let sub = create_subscription(
        pool,
        admin,
        "kb_contexts",
        ctx,
        team,
        conn,
        SubscriptionSelector::GitHubRepository {
            repo: GITHUB_REPO.into(),
            event_types: vec![],
        },
    )
    .await;
    (admin, team, ctx, conn, sub)
}

/// Drive one webhook through intake and return (event_id, the single delivery).
async fn deliver_one(pool: &PgPool, conn: Uuid, sub: Uuid, admin: ProfileId) -> (Uuid, Delivery) {
    let event_id = intake_service::receive_webhook(
        pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive webhook");
    let rows = list_for_subscription(
        pool,
        &crate::test_support::human_principal_for(pool, admin.uuid()).await,
        sub,
        50,
        0,
    )
    .await
    .expect("list deliveries");
    assert_eq!(rows.len(), 1, "expected exactly one delivery");
    (event_id, rows.into_iter().next().unwrap())
}

// ── what a resource erasure names ──────────────────────────────────────

/// (Task 01a0fedb item 1) The erasure survey names, by delivery id, every delivery that may
/// quote the resource, in any case and with or without hyphens: through its `scope_reason`,
/// its `rationale`, or its webhook's body, wrapped (version 2) or bare (version 1). A
/// delivery that names nothing of R is not listed. A bare body naming R at a top-level id key
/// is in neither R's element trail nor its erasure trail scope.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn the_erasure_survey_names_the_deliveries_that_may_quote_the_resource(pool: PgPool) {
    let (admin, _team, ctx, conn, sub) = seed_world(&pool).await;
    let authed = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
    let emitter: Uuid =
        sqlx::query_scalar("SELECT emitter_entity_id FROM kb_connections WHERE id = $1")
            .bind(conn)
            .fetch_one(&pool)
            .await
            .expect("the connection's emitter");
    let resource = temper_substrate::writes::create_resource(
        &pool,
        temper_substrate::writes::CreateParams {
            idempotency_key: None,
            title: "a resource a steward wrote about",
            origin_uri: "test://quoted",
            body: "prose",
            doc_type: "research",
            home: temper_substrate::payloads::AnchorRef::context(ctx.into()),
            owner: admin,
            originator: admin,
            emitter: emitter.into(),
            properties: &[],
            chunks: None,
            sources: vec![],
        },
    )
    .await
    .expect("create the resource")
    .uuid();

    // Four deliveries of four webhooks, one per subscription-event pair. The fourth body
    // quotes R, upper-cased.
    let mut ids = Vec::new();
    for i in 0..4 {
        let mut payload = github_pr_payload(GITHUB_REPO);
        if i == 3 {
            payload["comment"] =
                serde_json::json!(format!("see {}", resource.to_string().to_uppercase()));
        }
        intake_service::receive_webhook(
            &pool,
            conn,
            ProviderEvent::from_header("pull_request"),
            &payload,
        )
        .await
        .expect("receive webhook");
    }
    for d in list_for_subscription(&pool, &authed, sub, 50, 0)
        .await
        .expect("list deliveries")
    {
        ids.push(d.id);
    }
    ids.sort();
    let [scoped, judged, untouched, quoted_body] = ids[..] else {
        panic!("expected four deliveries, got {ids:?}");
    };

    record_scope(
        &pool,
        &authed,
        scoped,
        &RecordScopeRequest {
            status: DeliveryStatus::Undetermined,
            reason: Some(format!(
                "enrichment stalled reading {}",
                resource.to_string().to_uppercase()
            )),
        },
    )
    .await
    .expect("record scope");
    record_scope(
        &pool,
        &authed,
        judged,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("record scope");
    record_disposition(
        &pool,
        &authed,
        judged,
        &RecordDispositionRequest {
            disposition: Disposition::Acted,
            rationale: format!("filed as {}", resource.simple()),
            confidence: 0.9,
            invocation_id: None,
        },
    )
    .await
    .expect("dispose");

    // A version-1 webhook, stored bare before intake wrapped every body, whose body named the
    // resource at a top-level id key. Written directly, as intake no longer can.
    let older: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, payload, \
                                producing_anchor_table, producing_anchor_id) \
         SELECT t.id, c.emitter_entity_id, jsonb_build_object('resource_id', $2::text), \
                'kb_contexts', c.home_context_id \
           FROM kb_event_types t, kb_connections c \
          WHERE t.name = 'webhook_received' AND c.id = $1 RETURNING id",
    )
    .bind(conn)
    .bind(resource)
    .fetch_one(&pool)
    .await
    .expect("an older webhook naming the resource");
    let bare: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_subscription_deliveries (subscription_id, event_id) \
         VALUES ($1, $2) RETURNING id",
    )
    .bind(sub)
    .bind(older)
    .fetch_one(&pool)
    .await
    .expect("its delivery");

    let plan: serde_json::Value = sqlx::query_scalar("SELECT resource_erasure_survey_plan($1)")
        .bind(resource)
        .fetch_one(&pool)
        .await
        .expect("survey");
    let mut named: Vec<String> = plan["remainder"]
        .as_array()
        .expect("remainder")
        .iter()
        .filter(|r| r["target"] == "kb_subscription_deliveries")
        .map(|r| r["outcome"].as_str().unwrap().to_owned())
        .collect();
    named.sort();
    let mut expected: Vec<Uuid> = vec![scoped, judged, quoted_body, bare];
    expected.sort();
    assert_eq!(named.len(), 4, "four deliveries named, not five: {named:?}");
    for (outcome, id) in named.iter().zip(&expected) {
        assert!(
            outcome.starts_with(&format!("delivery {id} ")),
            "named by delivery id: {outcome}"
        );
    }
    assert!(
        !named.iter().any(|o| o.contains(&untouched.to_string())),
        "a delivery naming nothing of the resource is not listed"
    );

    let in_scope: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _resource_erasure_trail_scope($1) WHERE event_id = $2",
    )
    .bind(resource)
    .bind(older)
    .fetch_one(&pool)
    .await
    .expect("trail scope");
    assert_eq!(
        in_scope, 0,
        "a webhook is in no resource's erasure trail scope"
    );
    let in_trail: i64 =
        sqlx::query_scalar("SELECT count(*) FROM element_trail_node($1, $2) WHERE event_id = $3")
            .bind(admin.uuid())
            .bind(resource)
            .bind(older)
            .fetch_one(&pool)
            .await
            .expect("element trail");
    assert_eq!(in_trail, 0, "a webhook is in no resource's element trail");
    let trail_len: i64 = sqlx::query_scalar("SELECT count(*) FROM element_trail_node($1, $2)")
        .bind(admin.uuid())
        .bind(resource)
        .fetch_one(&pool)
        .await
        .expect("element trail");
    assert!(
        trail_len > 0,
        "the resource's own trail is readable to this profile"
    );

    // The same for an edge: a bare body naming an edge at `edge_id` is not in its trail.
    let other = temper_substrate::writes::create_resource(
        &pool,
        temper_substrate::writes::CreateParams {
            idempotency_key: None,
            title: "the other end",
            origin_uri: "test://other-end",
            body: "prose",
            doc_type: "research",
            home: temper_substrate::payloads::AnchorRef::context(ctx.into()),
            owner: admin,
            originator: admin,
            emitter: emitter.into(),
            properties: &[],
            chunks: None,
            sources: vec![],
        },
    )
    .await
    .expect("create the other resource");
    let edge = temper_substrate::writes::assert_relationship(
        &pool,
        temper_substrate::writes::AssertParams {
            src: resource.into(),
            tgt: other,
            kind: temper_substrate::affinity::EdgeKind::LeadsTo,
            polarity: temper_substrate::payloads::EdgePolarity::Forward,
            label: None,
            weight: 1.0,
            home: ctx.into(),
            emitter: emitter.into(),
        },
    )
    .await
    .expect("assert an edge")
    .uuid();
    let on_edge: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, payload, \
                                producing_anchor_table, producing_anchor_id) \
         SELECT t.id, c.emitter_entity_id, jsonb_build_object('edge_id', $2::text), \
                'kb_contexts', c.home_context_id \
           FROM kb_event_types t, kb_connections c \
          WHERE t.name = 'webhook_received' AND c.id = $1 RETURNING id",
    )
    .bind(conn)
    .bind(edge)
    .fetch_one(&pool)
    .await
    .expect("an older webhook naming the edge");
    let edge_trail: Vec<Uuid> =
        sqlx::query_scalar("SELECT event_id FROM element_trail_edge($1, $2)")
            .bind(admin.uuid())
            .bind(edge)
            .fetch_all(&pool)
            .await
            .expect("edge trail");
    assert!(
        !edge_trail.is_empty(),
        "the edge's own trail is readable to this profile"
    );
    assert!(
        !edge_trail.contains(&on_edge),
        "a webhook is in no edge's element trail"
    );
}

// ── the projection ──────────────────────────────────────────────────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn one_event_matching_n_subscriptions_projects_n_delivery_rows(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    // Three DISTINCT declarations against one connection. Distinct selectors, because the
    // unique index refuses the same selector twice for one authoring team.
    let mut subs = Vec::new();
    for repo in [GITHUB_REPO, "acme/other", GITHUB_REPO] {
        let ctx = seed_context_owned_by_team(&pool, team).await;
        let selector = if repo == GITHUB_REPO && !subs.is_empty() {
            SubscriptionSelector::GitHubCodeownersPaths {
                repo: repo.into(),
                paths: vec!["src/**".into()],
            }
        } else {
            SubscriptionSelector::GitHubRepository {
                repo: repo.into(),
                event_types: vec![],
            }
        };
        subs.push(
            create_subscription(&pool, admin, "kb_contexts", ctx, team, conn, selector).await,
        );
    }

    intake_service::receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive webhook");

    // Two of the three match acme/temper; the acme/other declaration must not.
    let mut counts: Vec<usize> = Vec::new();
    for s in &subs {
        counts.push(
            list_for_subscription(
                &pool,
                &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
                *s,
                50,
                0,
            )
            .await
            .expect("list")
            .len(),
        );
    }
    assert_eq!(
        counts,
        vec![1, 0, 1],
        "one delivery per MATCHED declaration, none for the rest"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_payload_matching_zero_subscriptions_projects_zero_delivery_rows(pool: PgPool) {
    let (admin, _team, _ctx, conn, sub) = seed_world(&pool).await;

    // A payload for a repo nothing declared an interest in. The empty radius is the noise
    // filter: the event is stored, and it routes nowhere.
    intake_service::receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload("someone-else/unrelated"),
    )
    .await
    .expect("receive webhook");

    let rows = list_for_subscription(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        sub,
        50,
        0,
    )
    .await
    .expect("list");
    assert!(
        rows.is_empty(),
        "the empty radius must project no delivery rows"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_delivery_is_born_pending_scope(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (event_id, d) = deliver_one(&pool, conn, sub, admin).await;
    assert_eq!(d.status, DeliveryStatus::PendingScope);
    assert_eq!(
        d.event_id, event_id,
        "the delivery projects from the intake event"
    );
    assert!(d.disposition.is_none());
    assert!(d.scoped_at.is_none());
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn the_delivery_row_and_its_event_are_one_transaction(pool: PgPool) {
    // The delivery projects from the SAME event the radius was computed for — there is no
    // second event, and no window in which the routing exists without the rows that make it
    // readable. Witnessed by counting webhook_received events against delivery rows.
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    deliver_one(&pool, conn, sub, admin).await;

    let events: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "c!" FROM kb_events e
                 JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE t.name = 'webhook_received'"#
    )
    .fetch_one(&pool)
    .await
    .expect("count events");
    assert_eq!(events, 1, "one webhook receipt is one ledger event, always");
}

// ── the scope leg ───────────────────────────────────────────────────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn scope_walks_pending_to_in_scope(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    let scoped = record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("record scope");
    assert_eq!(scoped.status, DeliveryStatus::InScope);
    assert!(scoped.scoped_at.is_some());
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_undetermined_delivery_must_say_why(pool: PgPool) {
    // Invariant 6 wants the DLQ LEGIBLE, not merely visible. The service refuses before the
    // CHECK does, so the caller gets an answer instead of a 500.
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    let err = record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::Undetermined,
            reason: None,
        },
    )
    .await
    .expect_err("undetermined without a reason must be refused");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_undetermined_delivery_stays_visible_and_never_becomes_out_of_scope_silently(
    pool: PgPool,
) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::Undetermined,
            reason: Some("enrichment failed: 502 from GitHub".into()),
        },
    )
    .await
    .expect("record undetermined");

    // It is on the DLQ read, and it says what stopped it.
    let stuck = list_undetermined(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        sub,
    )
    .await
    .expect("dlq read");
    assert_eq!(stuck.len(), 1);
    assert_eq!(
        stuck[0].scope_reason.as_deref(),
        Some("enrichment failed: 502 from GitHub")
    );

    // And it is still surfaced for judgment — invariant 6 requires the DLQ reach the tick,
    // not be quietly held back until someone fixes the enrichment.
    assert!(stuck[0].status.is_surfaced_for_judgment());
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_judged_delivery_cannot_be_rescoped(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("scope");
    record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Declined,
            rationale: "touched only test fixtures".into(),
            confidence: 0.7,
            invocation_id: None,
        },
    )
    .await
    .expect("dispose");

    let err = record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::OutOfScope,
            reason: Some("late".into()),
        },
    )
    .await
    .expect_err("re-scoping a judged delivery must be refused");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

// ── the judgment leg ────────────────────────────────────────────────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_decline_is_an_authored_event_carrying_reasoning_and_confidence(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (webhook_event, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("scope");

    let judged = record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Declined,
            rationale: "judged immaterial: touched only test fixtures".into(),
            confidence: 0.7,
            invocation_id: None,
        },
    )
    .await
    .expect("dispose");

    assert_eq!(judged.disposition, Some(Disposition::Declined));
    assert_eq!(judged.confidence, Some(0.7));
    assert_eq!(judged.decided_by_profile_id, Some(*admin));
    let judgment_event = judged
        .decided_by_event_id
        .expect("a disposition cites its act");

    // The decline is on the LEDGER, not merely in a column — that is what makes it citable
    // rather than a silent cursor bump.
    let row = sqlx::query!(
        r#"SELECT t.name AS "name!", e.payload AS "payload!"
                 FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE e.id = $1"#,
        judgment_event,
    )
    .fetch_one(&pool)
    .await
    .expect("load judgment event");
    assert_eq!(row.name, "subscription_delivery_disposed");
    assert_eq!(
        row.payload["rationale"],
        "judged immaterial: touched only test fixtures"
    );
    assert_eq!(row.payload["confidence"], 0.7);
    assert_eq!(row.payload["event_id"], serde_json::json!(webhook_event));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_human_disposes_with_no_invocation_envelope(pool: PgPool) {
    // The clause's load-bearing property is ACCOUNTABLE AND CITABLE, not enveloped.
    // kb_invocations.originating_cogmap_id is NOT NULL, so requiring an envelope would make
    // this verb unreachable for kb_contexts and kb_teams subscribers entirely.
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("scope");
    let judged = record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Acted,
            rationale: "authored a decision citing this PR".into(),
            confidence: 0.9,
            invocation_id: None,
        },
    )
    .await
    .expect("a human disposition must be accepted without an envelope");
    assert!(judged.decided_by_invocation_id.is_none());
    assert_eq!(
        judged.decided_by_profile_id,
        Some(*admin),
        "the profile carries attribution"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_disposition_without_reasoning_is_refused(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("scope");
    let err = record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Declined,
            rationale: "   ".into(),
            confidence: 0.5,
            invocation_id: None,
        },
    )
    .await
    .expect_err("a decline with no reasoning is the silent cursor bump this prevents");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_pending_scope_delivery_cannot_be_judged(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    let err = record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Acted,
            rationale: "premature".into(),
            confidence: 0.5,
            invocation_id: None,
        },
    )
    .await
    .expect_err("judging an unscoped delivery must be refused");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_out_of_scope_delivery_cannot_be_judged(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::OutOfScope,
            reason: Some("no CODEOWNERS path hit".into()),
        },
    )
    .await
    .expect("scope");
    let err = record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Declined,
            rationale: "nothing to decline".into(),
            confidence: 0.5,
            invocation_id: None,
        },
    )
    .await
    .expect_err("out_of_scope was determined not to touch this subscriber");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

// ── awareness is terminal, not backlog ──────────────────────────────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_awareness_only_subscriber_is_terminal_at_in_scope(pool: PgPool) {
    // A kb_contexts subscriber has no telos, no steward and no envelope. Its undisposed
    // delivery is a record of awareness, and the read surface returns it as an ordinary row
    // — there is no verb on this module that would report it as outstanding work.
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("scope");

    let rows = list_for_subscription(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        sub,
        50,
        0,
    )
    .await
    .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, DeliveryStatus::InScope);
    assert!(
        rows[0].disposition.is_none(),
        "undisposed is terminal here, not pending"
    );
}

// ── C11: the routing is readable by the routed-to ───────────────────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_subscriber_can_read_what_was_routed_to_it(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    for _ in 0..3 {
        intake_service::receive_webhook(
            &pool,
            conn,
            ProviderEvent::from_header("pull_request"),
            &github_pr_payload(GITHUB_REPO),
        )
        .await
        .expect("receive webhook");
    }
    let rows = list_for_subscription(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        sub,
        50,
        0,
    )
    .await
    .expect("list");
    assert_eq!(
        rows.len(),
        3,
        "every routed event is readable by the party it was routed to"
    );
    // Newest first, by uuidv7 id — byte order is time order.
    assert!(rows[0].id > rows[1].id && rows[1].id > rows[2].id);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_stranger_cannot_read_another_teams_deliveries(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    deliver_one(&pool, conn, sub, admin).await;

    // A profile that manages nothing. The delivery's readability is exactly its
    // subscription's, so this is refused by the same gate that guards the declaration.
    let stranger = seed_plain_profile(&pool).await;
    let err = list_for_subscription(
        &pool,
        &crate::test_support::human_principal_for(&pool, stranger.uuid()).await,
        sub,
        50,
        0,
    )
    .await
    .expect_err("a stranger must not read another team's deliveries");
    assert!(
        matches!(err, ApiError::Forbidden | ApiError::NotFound(_)),
        "got {err:?}"
    );
}

// ── C12: a silent declaration vs a quiet source ─────────────────────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn liveness_reports_matching_once_a_delivery_exists(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    deliver_one(&pool, conn, sub, admin).await;
    let live = declaration_liveness(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        sub,
    )
    .await
    .expect("liveness");
    assert_eq!(live, DeclarationLiveness::Matching { delivery_count: 1 });
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn liveness_distinguishes_a_wrong_selector_from_a_quiet_source(pool: PgPool) {
    // THE case the clause exists for. Payloads arrived on the connection; this declaration
    // matched none of them. Zero deliveries must NOT read as "the repo has been quiet".
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let ctx = seed_context_owned_by_team(&pool, team).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    let typo = create_subscription(
        &pool,
        admin,
        "kb_contexts",
        ctx,
        team,
        conn,
        SubscriptionSelector::GitHubRepository {
            repo: "acme/tempr".into(), // the typo
            event_types: vec![],
        },
    )
    .await;

    // A connection with no credential was never going to receive anything, and the read
    // says so rather than blaming the source. (Caught by this test failing the first time it
    // ran, which is the discriminator doing its job.)
    assert_eq!(
        declaration_liveness(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            typo
        )
        .await
        .expect("liveness"),
        DeclarationLiveness::ConnectionNotAuthenticated,
        "an unauthenticated connection must not read as a quiet source"
    );

    // With a credential and nothing received, the honest answer is "the source has been quiet".
    attach_stub_credential(&pool, conn).await;
    assert_eq!(
        declaration_liveness(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            typo
        )
        .await
        .expect("liveness"),
        DeclarationLiveness::SourceQuiet,
        "authenticated and silent means the source has been quiet"
    );

    intake_service::receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("receive webhook");

    // Now two payloads-worth of evidence says the selector is the problem.
    assert_eq!(
        declaration_liveness(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            typo
        )
        .await
        .expect("liveness"),
        DeclarationLiveness::SelectorMatchesNothing {
            events_on_connection: 1
        },
        "a live source with zero deliveries means the SELECTOR matched nothing"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn liveness_reports_revoked_before_anything_else(pool: PgPool) {
    let (admin, _t, _c, _conn, sub) = seed_world(&pool).await;
    crate::services::subscription_service::revoke(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        sub,
    )
    .await
    .expect("revoke");
    let live = declaration_liveness(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        sub,
    )
    .await
    .expect("liveness");
    assert!(
        matches!(live, DeclarationLiveness::Revoked { .. }),
        "revocation explains everything downstream of it; got {live:?}"
    );
}

// ── review findings: the guards must hold, and mean what they say ──────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_judged_undetermined_delivery_leaves_the_dlq(pool: PgPool) {
    // `undetermined` admits judgment on purpose, so a steward can decline an enrichment
    // failure on its merits. Once judged the delivery is RESOLVED, and a "what is stuck?"
    // sweep that keeps returning it is a queue that can only grow. The status stays
    // `undetermined` — that is still the true statement about what enrichment concluded.
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::Undetermined,
            reason: Some("enrichment failed: 502".into()),
        },
    )
    .await
    .expect("scope");
    assert_eq!(
        list_undetermined(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            sub
        )
        .await
        .expect("dlq")
        .len(),
        1
    );

    let judged = record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Declined,
            rationale: "judged immaterial despite the failed enrichment".into(),
            confidence: 0.4,
            invocation_id: None,
        },
    )
    .await
    .expect("an undetermined delivery is dispositionable");

    assert!(
        list_undetermined(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            sub
        )
        .await
        .expect("dlq")
        .is_empty(),
        "a judged delivery has left the DLQ"
    );
    assert_eq!(
        judged.status,
        DeliveryStatus::Undetermined,
        "the status still records what enrichment concluded"
    );
    assert_eq!(
        judged.scope_reason.as_deref(),
        Some("enrichment failed: 502")
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn liveness_does_not_blame_the_selector_for_traffic_that_predates_the_declaration(
    pool: PgPool,
) {
    // The delivery count can only include events from after the subscription existed, so an
    // all-time event count compares two different windows and reports SelectorMatchesNothing
    // on a brand-new declaration against an established connection. The clause exists to stop
    // a maintainer misreading silence; that would manufacture the opposite misreading.
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let ctx = seed_context_owned_by_team(&pool, team).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    attach_stub_credential(&pool, conn).await;

    // History on the connection BEFORE any declaration exists.
    intake_service::receive_webhook(
        &pool,
        conn,
        ProviderEvent::from_header("pull_request"),
        &github_pr_payload(GITHUB_REPO),
    )
    .await
    .expect("historical webhook");

    let fresh = create_subscription(
        &pool,
        admin,
        "kb_contexts",
        ctx,
        team,
        conn,
        SubscriptionSelector::GitHubRepository {
            repo: GITHUB_REPO.into(),
            event_types: vec![],
        },
    )
    .await;

    assert_eq!(
        declaration_liveness(
            &pool,
            &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
            fresh
        )
        .await
        .expect("liveness"),
        DeclarationLiveness::SourceQuiet,
        "nothing has arrived SINCE this declaration; its selector is not the problem"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_scope_cannot_move_underneath_a_judgment(pool: PgPool) {
    // The `undetermined` direction is the one the schema does NOT rescue: every CHECK permits
    // it, so without the `disposition IS NULL` predicate the row would end up carrying a
    // judgment made against `in_scope` while reading `undetermined`.
    //
    // WHAT THIS DOES NOT WITNESS: the sequential path was already guarded by the pre-check, so
    // this test passes against the pre-fix code too. It is a regression guard, not evidence for
    // the concurrency fix — the interleaving that fix addresses (read on the pool, judge, then
    // write) has no deterministic test here, and the `disposition IS NULL` predicate plus the
    // `FOR UPDATE` in `record_disposition` are verified by construction rather than by
    // execution. Stated rather than left to be inferred from a green run.
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("scope");
    record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Acted,
            rationale: "authored against it".into(),
            confidence: 0.9,
            invocation_id: None,
        },
    )
    .await
    .expect("dispose");

    let err = record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::Undetermined,
            reason: Some("late enrichment failure".into()),
        },
    )
    .await
    .expect_err("re-scoping a judged delivery must be refused");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");

    let after = get(&pool, d.id).await.expect("reload");
    assert_eq!(
        after.status,
        DeliveryStatus::InScope,
        "the judged scope is intact"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_envelope_is_refused_for_a_subscriber_that_cannot_act_under_one(pool: PgPool) {
    // A kb_contexts subscriber has no telos. An envelope offered for it is incoherent, not
    // merely unverified — and the column's whole argument is that a citation is accountable.
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    record_scope(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordScopeRequest {
            status: DeliveryStatus::InScope,
            reason: None,
        },
    )
    .await
    .expect("scope");

    let err = record_disposition(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        d.id,
        &RecordDispositionRequest {
            disposition: Disposition::Declined,
            rationale: "borrowed someone else's envelope".into(),
            confidence: 0.5,
            invocation_id: Some(Uuid::now_v7()),
        },
    )
    .await
    .expect_err("a context subscriber cannot cite an invocation envelope");
    let ApiError::BadRequest(msg) = err else {
        panic!("expected BadRequest, got {err:?}")
    };
    assert!(
        msg.contains("kb_cogmaps"),
        "must say which subscriber kind can: {msg}"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn the_refusal_message_is_not_mangled_by_source_wrapping(pool: PgPool) {
    // `cargo fmt` collapses a wrapped string literal without `\` continuations into one line,
    // baking the source indentation into the message. This message exists to be READ by a
    // maintainer chasing a stale registration record, so the mangling defeats its purpose.
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let ctx = seed_context_owned_by_team(&pool, team).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    register_webhook_events(&pool, conn, &["push"]).await;

    let err = crate::services::subscription_service::create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &temper_core::types::subscription::CreateSubscriptionRequest {
            subscriber_table: "kb_contexts".into(),
            subscriber_id: ctx,
            authoring_team_id: team,
            connection_id: conn,
            selector: SubscriptionSelector::GitHubRepository {
                repo: GITHUB_REPO.into(),
                event_types: vec!["pull_request".into()],
            },
        },
    )
    .await
    .expect_err("refused");
    let ApiError::BadRequest(msg) = err else {
        panic!("expected BadRequest, got {err:?}")
    };
    assert!(
        !msg.contains("  "),
        "no run of double spaces in a message meant to be read: {msg:?}"
    );
}

// ── the inert declaration is refused at create, not disclosed forever ───

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_selector_waiting_on_an_unregistered_event_kind_is_refused_at_create(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let ctx = seed_context_owned_by_team(&pool, team).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    register_webhook_events(&pool, conn, &["push"]).await;

    let err = crate::services::subscription_service::create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &temper_core::types::subscription::CreateSubscriptionRequest {
            subscriber_table: "kb_contexts".into(),
            subscriber_id: ctx,
            authoring_team_id: team,
            connection_id: conn,
            selector: SubscriptionSelector::GitHubRepository {
                repo: GITHUB_REPO.into(),
                event_types: vec!["pull_request".into()],
            },
        },
    )
    .await
    .expect_err("a declaration that can never match must be refused at create");
    let ApiError::BadRequest(msg) = err else {
        panic!("expected BadRequest, got {err:?}")
    };
    // The refusal CITES WHAT IT COMPARED, so a maintainer who knows the registration record
    // is stale can see why it said no rather than guess.
    assert!(
        msg.contains("pull_request"),
        "must name what the selector waits for: {msg}"
    );
    assert!(
        msg.contains("push"),
        "must name what the connection receives: {msg}"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_unregistered_connection_does_not_trigger_the_refusal(pool: PgPool) {
    // An EMPTY webhook_events is the not-yet-ledger-capable state, not proof that the
    // connection receives nothing forever. Refusing on it would reject a subscription against
    // a connection whose registration simply has not landed — the false-refusal risk that
    // made "warn instead" a real alternative. The bar is proof, and this is not proof.
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let ctx = seed_context_owned_by_team(&pool, team).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;

    crate::services::subscription_service::create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &temper_core::types::subscription::CreateSubscriptionRequest {
            subscriber_table: "kb_contexts".into(),
            subscriber_id: ctx,
            authoring_team_id: team,
            connection_id: conn,
            selector: SubscriptionSelector::GitHubRepository {
                repo: GITHUB_REPO.into(),
                event_types: vec!["pull_request".into()],
            },
        },
    )
    .await
    .expect("an unregistered connection proves nothing and must not refuse");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_overlapping_selector_is_admitted(pool: PgPool) {
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let ctx = seed_context_owned_by_team(&pool, team).await;
    let conn = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, conn, team).await;
    register_webhook_events(&pool, conn, &["push", "pull_request"]).await;

    crate::services::subscription_service::create(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        &temper_core::types::subscription::CreateSubscriptionRequest {
            subscriber_table: "kb_contexts".into(),
            subscriber_id: ctx,
            authoring_team_id: team,
            connection_id: conn,
            selector: SubscriptionSelector::GitHubRepository {
                repo: GITHUB_REPO.into(),
                event_types: vec!["pull_request".into()],
            },
        },
    )
    .await
    .expect("one overlapping event kind is enough for the declaration to be live");
}

/// The existence oracle, closed one level down: a delivery-id act refuses a caller who cannot
/// act on the delivery's subscription with the **missing-delivery** refusal, not the
/// subscription's. Passing the subscription gate's sentence through would let a prober tell an
/// existing delivery (subscription refusal) from a missing one (delivery refusal).
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_denied_delivery_is_indistinguishable_from_a_missing_one(pool: PgPool) {
    let (admin, _t, _c, conn, sub) = seed_world(&pool).await;
    let (_e, d) = deliver_one(&pool, conn, sub, admin).await;
    let stranger = seed_plain_profile(&pool).await;
    let prober = crate::test_support::human_principal_for(&pool, stranger.uuid()).await;
    let scope = RecordScopeRequest {
        status: DeliveryStatus::InScope,
        reason: None,
    };

    for id in [d.id, Uuid::now_v7()] {
        let refusals = [
            (
                "get",
                get_for_caller(&pool, &prober, id).await.expect_err("get"),
            ),
            (
                "scope",
                record_scope(&pool, &prober, id, &scope)
                    .await
                    .expect_err("scope"),
            ),
        ];
        for (act, err) in refusals {
            assert!(
                matches!(&err, ApiError::NotFound(m) if m == DELIVERY_REFUSAL),
                "{act}: must refuse as a missing delivery, got {err:?}"
            );
        }
    }
    assert_eq!(
        get(&pool, d.id).await.expect("get").status,
        DeliveryStatus::PendingScope,
        "the denied scope wrote nothing"
    );
}
