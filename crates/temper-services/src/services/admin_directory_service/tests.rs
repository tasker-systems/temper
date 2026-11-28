use super::*;
use sqlx::PgPool;
use temper_core::types::machine::{ProvisionMachineRequest, RebindMachineRequest};

use crate::services::machine_registration_service;

// ── fixtures ─────────────────────────────────────────────────────────────

/// Seed a HUMAN profile: an auth link under a non-machine provider, verified unless
/// `verified: false`. No standing row unless one is added.
async fn human(pool: &PgPool, handle: &str, email: &str, verified: bool) -> (ProfileId, Uuid) {
    let profile_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
        .bind(profile_id)
        .bind(handle)
        .execute(pool)
        .await
        .expect("seed human profile");
    let link_id = auth_link(pool, profile_id, "saml:okta", email, verified, true).await;
    (ProfileId::from(profile_id), link_id)
}

async fn auth_link(
    pool: &PgPool,
    profile_id: Uuid,
    provider: &str,
    email: &str,
    verified: bool,
    is_default: bool,
) -> Uuid {
    let link_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profile_auth_links \
         (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, \
          is_default) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(link_id)
    .bind(profile_id)
    .bind(provider)
    .bind(format!("{provider}-{}", link_id.simple()))
    .bind(email)
    .bind(verified)
    .bind(is_default)
    .execute(pool)
    .await
    .expect("seed auth link");
    link_id
}

/// Seed a standing row the way the transition functions leave it (the fixture wants the
/// end state, not the append-only history — CONFORM `test_support`'s own rationale).
async fn standing(pool: &PgPool, profile_id: Uuid, state: &str) {
    sqlx::query!(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, $2) \
         ON CONFLICT (profile_id) DO UPDATE SET state = $2",
        profile_id,
        state,
    )
    .execute(pool)
    .await
    .expect("seed standing");
}

async fn governance(pool: &PgPool, profile_id: Uuid) {
    sqlx::query!(
        "INSERT INTO kb_principal_governance (profile_id) VALUES ($1) \
         ON CONFLICT (profile_id) DO NOTHING",
        profile_id,
    )
    .execute(pool)
    .await
    .expect("seed governance");
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    crate::test_support::team::insert(pool, slug).await
}

async fn membership(pool: &PgPool, team_id: Uuid, profile_id: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
                 VALUES ($1, $2, $3::team_role)",
    )
    .bind(team_id)
    .bind(profile_id)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed membership");
}

async fn admin(pool: &PgPool) -> SystemAdmin {
    crate::test_support::system_admin_proof(pool).await
}

fn query(standing: Option<&str>) -> AdminProfilesListQuery {
    AdminProfilesListQuery {
        standing: standing.map(str::to_owned),
        ..Default::default()
    }
}

// ── §4 the discriminator pinned against both machine mint doors ──────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn provisioned_and_rebound_machines_never_appear_but_a_saml_human_does(pool: PgPool) {
    let admin = admin(&pool).await;

    // Mint door 1: a provisioned machine, through the REAL registration service.
    let provisioned = machine_registration_service::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.actor().uuid()).await,
        &ProvisionMachineRequest {
            client_id: format!("m2m-provisioned-{}", Uuid::now_v7()),
            label: "door one".to_owned(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        },
        crate::auth_config::AuthMode::ExternalIdp,
    )
    .await
    .expect("provision machine");

    // Mint door 2: a rebound machine — a SECOND auth0-m2m link on a fresh profile.
    let rebound = machine_registration_service::provision(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.actor().uuid()).await,
        &ProvisionMachineRequest {
            client_id: format!("m2m-rebind-src-{}", Uuid::now_v7()),
            label: "rebind source".to_owned(),
            owner_team_id: None,
            teams: vec![],
            grants: vec![],
        },
        crate::auth_config::AuthMode::ExternalIdp,
    )
    .await
    .expect("provision rebind source");
    machine_registration_service::rebind(
        &pool,
        &admin,
        &RebindMachineRequest {
            client_id: format!("m2m-rebound-{}", Uuid::now_v7()),
            from_machine_client_id: rebound.id,
            label: "rebound".to_owned(),
            keep_old_active: false,
        },
        crate::auth_config::AuthMode::ExternalIdp,
    )
    .await
    .expect("rebind machine");

    // The human the directory exists for: SAML-provisioned, never requested anything.
    let (human_id, _) = human(&pool, "saml-alice", "alice@corp.example", true).await;

    let page = list_profiles(&pool, &admin, &query(None))
        .await
        .expect("list");
    let listed: Vec<Uuid> = page.entries.iter().map(|e| e.profile_id).collect();
    assert!(listed.contains(&*human_id), "SAML human must appear");
    assert!(
        !listed.contains(&provisioned.profile_id),
        "provisioned machine leaked into the human directory"
    );
    assert_eq!(
        listed.iter().filter(|id| **id == *human_id).count(),
        1,
        "exactly one row for the human"
    );
    // The rebound machine shares the SOURCE profile (rebind adds a second m2m link to the
    // SAME profile), so the same-never-appears assertion covers both links.
    assert_eq!(
        page.total, 1,
        "the operator's own machine-minted profiles and any seeded humans except alice are absent"
    );
}

// ── §5 the list affordance ───────────────────────────────────────────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn human_with_no_standing_row_appears_in_default_view_as_denied(pool: PgPool) {
    let admin = admin(&pool).await;
    let (never_requested, _) = human(&pool, "never-requested", "nr@corp.example", true).await;
    let (approved, _) = human(&pool, "approved-one", "ap@corp.example", true).await;
    standing(&pool, *approved, "approved").await;

    // Default view: needs-access.
    let page = list_profiles(&pool, &admin, &query(None))
        .await
        .expect("list");
    assert_eq!(
        page.total, 1,
        "only the never-requested human is needs-access"
    );
    let entry = &page.entries[0];
    assert_eq!(entry.profile_id, *never_requested);
    assert_eq!(entry.standing, "denied", "absence renders as denied");
    assert_eq!(entry.standing_updated, None);

    // The approved principal is findable only under its own state or `all`.
    let all = list_profiles(&pool, &admin, &query(Some("all")))
        .await
        .expect("list all");
    assert_eq!(all.total, 2);
    let approved_only = list_profiles(&pool, &admin, &query(Some("approved")))
        .await
        .expect("list approved");
    assert_eq!(approved_only.total, 1);
    assert_eq!(approved_only.entries[0].profile_id, *approved);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn needs_access_covers_revoked_and_deactivated_too(pool: PgPool) {
    let admin = admin(&pool).await;
    let (revoked, _) = human(&pool, "revoked-one", "rv@corp.example", true).await;
    standing(&pool, *revoked, "revoked").await;
    let (deactivated, _) = human(&pool, "deactivated-one", "da@corp.example", true).await;
    standing(&pool, *deactivated, "deactivated").await;

    let page = list_profiles(&pool, &admin, &query(None))
        .await
        .expect("list");
    assert_eq!(page.total, 2, "revoked and deactivated both lack access");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn unknown_standing_filter_is_bad_request(pool: PgPool) {
    let admin = admin(&pool).await;
    let err = list_profiles(&pool, &admin, &query(Some("unapproved")))
        .await
        .expect_err("unapproved is the retired name, not a filter");
    assert!(matches!(err, ApiError::BadRequest(_)));
}

/// The exact-email lookup folds ASCII case only: a look-alike (U+212A KELVIN SIGN, which the
/// database collation's `lower()` folds to `k`) does not resolve to the `k` address's holder.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn card_by_email_does_not_fold_look_alikes(pool: PgPool) {
    let admin = admin(&pool).await;
    let _ = human(&pool, "look-alike", "\u{212A}ate@corp.example", true).await;
    let err = profile_card_by_email(&pool, &admin, "kate@corp.example")
        .await
        .expect_err("a look-alike holder is not the address's holder");
    assert!(matches!(err, ApiError::NotFound(_)), "got {err:?}");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn email_contains_matches_literals_and_returns_matched_email(pool: PgPool) {
    let admin = admin(&pool).await;
    let (hit, _) = human(
        &pool,
        "literal-hit",
        "alice.under_score%tag@corp.example",
        true,
    )
    .await;
    let (_miss, _) = human(&pool, "literal-miss", "bob@corp.example", true).await;

    // A needle containing all three LIKE metacharacters must match LITERALLY.
    let mut q = query(None);
    q.email_contains = Some("under_score%tag".to_owned());
    let page = list_profiles(&pool, &admin, &q).await.expect("contains");
    assert_eq!(page.total, 1, "% and _ must not act as wildcards");
    assert_eq!(page.entries[0].profile_id, *hit);
    assert_eq!(
        page.entries[0].matched_email.as_deref(),
        Some("alice.under_score%tag@corp.example"),
        "the matched address rides the row"
    );

    // Case-insensitive, over verified emails.
    q.email_contains = Some("ALICE".to_owned());
    let page = list_profiles(&pool, &admin, &q).await.expect("contains");
    assert_eq!(page.total, 1);

    // With no filter active, matched_email stays off the rows.
    let plain = list_profiles(&pool, &admin, &query(Some("all")))
        .await
        .expect("plain");
    assert!(plain.entries.iter().all(|e| e.matched_email.is_none()));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn email_contains_never_matches_an_unverified_email(pool: PgPool) {
    let admin = admin(&pool).await;
    let (_p, _) = human(&pool, "unverified", "secret@corp.example", false).await;

    let mut q = query(None);
    q.email_contains = Some("secret".to_owned());
    let page = list_profiles(&pool, &admin, &q).await.expect("contains");
    assert_eq!(page.total, 0, "unverified addresses are not enumerated");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn team_filter_accepts_slug_and_uuid(pool: PgPool) {
    let admin = admin(&pool).await;
    let (member, _) = human(&pool, "team-member", "tm@corp.example", true).await;
    let (outsider, _) = human(&pool, "team-outsider", "to@corp.example", true).await;
    let team_id = team(&pool, "platform").await;
    membership(&pool, team_id, *member, "member").await;
    let _ = outsider;

    let mut q = query(Some("all"));
    q.team = Some("platform".to_owned());
    let page = list_profiles(&pool, &admin, &q).await.expect("by slug");
    assert_eq!(page.total, 1);
    assert_eq!(page.entries[0].profile_id, *member);

    q.team = Some(team_id.to_string());
    let page = list_profiles(&pool, &admin, &q).await.expect("by uuid");
    assert_eq!(page.total, 1);
    assert_eq!(page.entries[0].profile_id, *member);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn list_rows_carry_row_shape_and_clamps_apply(pool: PgPool) {
    let admin = admin(&pool).await;
    let (a, _) = human(&pool, "row-a", "row-a@corp.example", true).await;
    let (b, _) = human(&pool, "row-b", "row-b@corp.example", true).await;
    let t = team(&pool, "sized").await;
    membership(&pool, t, *a, "member").await;
    standing(&pool, *b, "requested").await;
    governance(&pool, *b).await;
    sqlx::query(
        "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, source) \
         VALUES ($1, $2, $3, 'pending', 'web')",
    )
    .bind(Uuid::now_v7())
    .bind(t)
    .bind(*b)
    .execute(&pool)
    .await
    .expect("seed join request");

    // limit=1 → one row, total still counts both.
    let mut q = query(None);
    q.limit = Some(1);
    let page = list_profiles(&pool, &admin, &q).await.expect("page");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(
        page.total, 2,
        "total is the filtered population, not the page"
    );

    // Every row carries the full §5 shape.
    let all = list_profiles(&pool, &admin, &query(Some("all")))
        .await
        .expect("all");
    for entry in &all.entries {
        assert!(!entry.handle.is_empty());
        assert!(!entry.display_name.is_empty());
        assert!(entry.team_count >= 0);
    }
    let row_b = all
        .entries
        .iter()
        .find(|e| e.profile_id == *b)
        .expect("row b");
    assert_eq!(row_b.standing, "requested");
    // A governance row without approved standing confers nothing (20261205000010), and the
    // directory reports is_system_admin, not the row.
    assert!(!row_b.is_system_admin);
    assert!(row_b.has_pending_request);
    assert_eq!(row_b.email.as_deref(), Some("row-b@corp.example"));
    assert_eq!(row_b.provisioned_via.as_deref(), Some("saml:okta"));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn list_clamps_limit_and_offset(pool: PgPool) {
    let admin = admin(&pool).await;
    let (_only, _) = human(&pool, "clamp-one", "clamp@corp.example", true).await;

    // limit above the cap clamps down (returns everything there is, no error).
    let mut q = query(None);
    q.limit = Some(10_000);
    let page = list_profiles(&pool, &admin, &q).await.expect("clamp limit");
    assert_eq!(page.entries.len(), 1);

    // limit below 1 clamps up to 1.
    q.limit = Some(0);
    let page = list_profiles(&pool, &admin, &q).await.expect("floor limit");
    assert_eq!(page.entries.len(), 1);

    // Offset beyond the population: empty page, total still honest.
    q.limit = None;
    q.offset = Some(5_000);
    let page = list_profiles(&pool, &admin, &q).await.expect("deep offset");
    assert!(page.entries.is_empty());
    assert_eq!(page.total, 1);

    // Negative offset floors at 0.
    q.offset = Some(-50);
    let page = list_profiles(&pool, &admin, &q)
        .await
        .expect("negative offset");
    assert_eq!(page.entries.len(), 1);

    // Offset above the 10_000 cap clamps down to the cap.
    q.offset = Some(99_999);
    let page = list_profiles(&pool, &admin, &q)
        .await
        .expect("capped offset");
    assert!(page.entries.is_empty());
    assert_eq!(page.total, 1);
}

// ── §6 the card ──────────────────────────────────────────────────────────

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn card_composes_identity_admission_governance_and_queue_state(pool: PgPool) {
    let admin = admin(&pool).await;
    let (pid, _) = human(&pool, "card-one", "card@corp.example", true).await;
    let t = team(&pool, "carders").await;
    membership(&pool, t, *pid, "maintainer").await;
    let request_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, message, source) \
         VALUES ($1, $2, $3, 'pending', 'let me in', 'web') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(t)
    .bind(*pid)
    .fetch_one(&pool)
    .await
    .expect("seed join request");
    let review_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_principal_review_requests (id, profile_id, message) \
         VALUES ($1, $2, 'second look') RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(*pid)
    .fetch_one(&pool)
    .await
    .expect("seed review request");

    let card = profile_card(&pool, &admin, pid).await.expect("card");
    assert_eq!(card.handle, "card-one");
    assert_eq!(card.standing, "denied", "no standing row renders denied");
    assert!(card.standing_updated.is_none());
    assert!(!card.is_system_admin);
    assert_eq!(card.email.as_deref(), Some("card@corp.example"));
    assert_eq!(card.provisioned_via.as_deref(), Some("saml:okta"));
    // The profile ALSO holds its trigger-minted `personal-<handle>` team; assert on the
    // seeded membership rather than the count.
    let carders = card
        .teams
        .iter()
        .find(|t| t.team_slug == "carders")
        .expect("seeded membership");
    assert_eq!(carders.role, "maintainer");
    let request = card.open_join_request.expect("open join request");
    assert_eq!(request.id, request_id);
    assert_eq!(request.message.as_deref(), Some("let me in"));
    let review = card.open_reconsideration.expect("open reconsideration");
    assert_eq!(review.id, review_id);
    // Hints, not doors — and machine-legal: this fixture has NO standing row (absence),
    // so approve would refuse from here and the card advertises no access act at all.
    // The open join request still advertises its review command — the request is that
    // hint's condition, not the standing.
    assert!(card
        .hints
        .iter()
        .any(|h| h.contains("admin requests review")));
    assert!(
        !card
            .hints
            .iter()
            .any(|h| h.starts_with("temper admin access ")),
        "absence has no legal admin act; the card must not advertise one: {:?}",
        card.hints
    );
}

/// Hints are machine-legal per standing class (adversarial review F2): approve is legal
/// from denied/requested/revoked; deactivated's door is reactivate (approve REFUSES from
/// deactivated); absence advertises nothing. A hint that 400s when copy-pasted is a
/// wrong-principal bridge, not an affordance.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn card_hints_are_machine_legal_for_the_standing_class(pool: PgPool) {
    let admin = admin(&pool).await;

    // Denied: approve runs.
    let (denied_pid, _) = human(&pool, "hint-denied", "hint-denied@corp.example", true).await;
    standing(&pool, *denied_pid, "denied").await;
    let card = profile_card(&pool, &admin, denied_pid).await.expect("card");
    assert!(
        card.hints
            .iter()
            .any(|h| h.starts_with("temper admin access approve ")),
        "denied must advertise approve: {:?}",
        card.hints
    );

    // Deactivated: the legal door is reactivate, NOT approve.
    let (deact_pid, _) = human(
        &pool,
        "hint-deactivated",
        "hint-deactivated@corp.example",
        true,
    )
    .await;
    standing(&pool, *deact_pid, "deactivated").await;
    let card = profile_card(&pool, &admin, deact_pid).await.expect("card");
    assert!(
        !card
            .hints
            .iter()
            .any(|h| h.starts_with("temper admin access approve ")),
        "approve refuses from deactivated; the card must not advertise it: {:?}",
        card.hints
    );
    assert!(
        card.hints
            .iter()
            .any(|h| h.starts_with("temper admin access reactivate ")),
        "deactivated must advertise reactivate: {:?}",
        card.hints
    );

    // Absence (never signed in): every admin act refuses; nothing is advertised.
    let (absent_pid, _) = human(&pool, "hint-absent", "hint-absent@corp.example", true).await;
    let card = profile_card(&pool, &admin, absent_pid).await.expect("card");
    assert!(
        !card
            .hints
            .iter()
            .any(|h| h.starts_with("temper admin access ")),
        "absence has no legal admin act: {:?}",
        card.hints
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn card_is_404_for_absent_profile(pool: PgPool) {
    let admin = admin(&pool).await;
    let err = profile_card(&pool, &admin, ProfileId::from(Uuid::now_v7()))
        .await
        .expect_err("absent");
    assert!(matches!(err, ApiError::NotFound(_)));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn card_default_email_follows_the_pinned_fallback_rule(pool: PgPool) {
    let admin = admin(&pool).await;
    let (pid, _) = human(&pool, "fallback", "stale@corp.example", true).await;

    // A NEWER default verified link displaces the saml link from the primary slot.
    sqlx::query("UPDATE kb_profile_auth_links SET is_default = false WHERE profile_id = $1")
        .bind(*pid)
        .execute(&pool)
        .await
        .expect("clear old default");
    auth_link(&pool, *pid, "google", "default@corp.example", true, true).await;
    let card = profile_card(&pool, &admin, pid).await.expect("card");
    assert_eq!(card.email.as_deref(), Some("default@corp.example"));
    assert_eq!(card.provisioned_via.as_deref(), Some("google"));

    // All links unverified → email withheld, provenance still named (step 3).
    let (uvid, _) = human(&pool, "unverified-card", "uv@corp.example", false).await;
    let card = profile_card(&pool, &admin, uvid).await.expect("card");
    assert_eq!(
        card.email, None,
        "an unverified email never renders unmarked"
    );
    assert_eq!(card.provisioned_via.as_deref(), Some("saml:okta"));

    // Default link exists but is UNVERIFIED → falls to the earliest VERIFIED link.
    let (mixed, _) = human(&pool, "mixed", "verified-first@corp.example", true).await;
    auth_link(
        &pool,
        *mixed,
        "enterprise",
        "unverified-default@corp.example",
        false,
        true,
    )
    .await;
    let card = profile_card(&pool, &admin, mixed).await.expect("card");
    assert_eq!(card.email.as_deref(), Some("verified-first@corp.example"));
    assert_eq!(card.provisioned_via.as_deref(), Some("saml:okta"));
    // The unverified address still shows on the links list — unmarked, not hidden.
    assert!(card.auth_links.iter().any(|l| l.email.as_deref()
        == Some("unverified-default@corp.example")
        && !l.email_verified));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn card_carries_pending_invitations_without_the_token(pool: PgPool) {
    let admin = admin(&pool).await;
    let (inviter, _) = human(&pool, "inviter", "inviter@corp.example", true).await;
    governance(&pool, *inviter).await; // invitations need an inviter profile only
    let (invitee, _) = human(&pool, "invitee", "invitee@corp.example", true).await;
    let t = team(&pool, "invited").await;
    let token = format!("tok-{}", Uuid::now_v7());
    sqlx::query(
        "INSERT INTO kb_team_invitations \
         (id, team_id, invited_email, invited_by_profile_id, role, token, status, expires_at) \
         VALUES ($1, $2, 'invitee@corp.example', $3, 'member', $4, 'pending', \
                 now() + interval '7 days')",
    )
    .bind(Uuid::now_v7())
    .bind(t)
    .bind(*inviter)
    .bind(&token)
    .execute(&pool)
    .await
    .expect("seed invitation");

    let card = profile_card(&pool, &admin, invitee).await.expect("card");
    assert_eq!(card.pending_invitations.len(), 1);
    assert_eq!(card.pending_invitations[0].team_slug, "invited");
    assert_eq!(card.pending_invitations[0].role, "member");

    // The pinned rule, asserted over the whole serialized card: no redemption token.
    let json = serde_json::to_string(&card).expect("serialize");
    assert!(
        !json.contains(&token),
        "the bearer token leaked into the card"
    );
    assert!(
        !json.contains("token"),
        "no token-bearing field may exist on the card wire shape"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn invitation_shared_by_two_verified_profiles_attributed_to_neither(pool: PgPool) {
    let admin = admin(&pool).await;
    let (inviter, _) = human(&pool, "shared-inviter", "si@corp.example", true).await;
    let (first, _) = human(&pool, "shared-a", "shared@corp.example", true).await;
    let (second, _) = human(&pool, "shared-b", "shared@corp.example", true).await;
    let t = team(&pool, "shared").await;
    sqlx::query(
        "INSERT INTO kb_team_invitations \
         (id, team_id, invited_email, invited_by_profile_id, role, token) \
         VALUES ($1, $2, 'shared@corp.example', $3, 'member', $4)",
    )
    .bind(Uuid::now_v7())
    .bind(t)
    .bind(*inviter)
    .bind(format!("shared-{}", Uuid::now_v7()))
    .execute(&pool)
    .await
    .expect("seed shared invitation");

    for profile in [*first, *second] {
        let card = profile_card(&pool, &admin, ProfileId::from(profile))
            .await
            .expect("card");
        assert!(
            card.pending_invitations.is_empty(),
            "a shared-address invitation must attribute to neither card"
        );
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn email_resolution_exact_case_insensitive_and_ambiguity_refusing(pool: PgPool) {
    let admin = admin(&pool).await;
    let (pid, _) = human(&pool, "exact-one", "Case.Exact@corp.example", true).await;

    // Exact, case-insensitive → the card.
    let card = profile_card_by_email(&pool, &admin, "case.exact@CORP.example")
        .await
        .expect("exact resolution");
    assert_eq!(card.profile_id, *pid);

    // Zero matches → 404.
    let err = profile_card_by_email(&pool, &admin, "nobody@corp.example")
        .await
        .expect_err("absent");
    assert!(matches!(err, ApiError::NotFound(_)));

    // The lookalike hazard (review C1): a verified `…corp.example.evil.io` address is a
    // DIFFERENT identity. Querying the original must still resolve to the original —
    // substring resolution could never guarantee that.
    let (lookalike, _) = human(&pool, "lookalike", "case.exact@corp.example.evil.io", true).await;
    let card = profile_card_by_email(&pool, &admin, "case.exact@CORP.example")
        .await
        .expect("the original address still resolves exactly");
    assert_eq!(card.profile_id, *pid);
    assert_ne!(card.profile_id, *lookalike);

    // More than one verified owner → 404 whose body NAMES the collision.
    let (_dup, _) = human(&pool, "exact-two", "case.exact@corp.example", true).await;
    let err = profile_card_by_email(&pool, &admin, "case.exact@corp.example")
        .await
        .expect_err("ambiguous");
    match err {
        ApiError::NotFound(message) => {
            assert!(
                message.contains("2 profiles"),
                "body must name the collision count: {message}"
            );
            assert!(
                message.contains(pid.to_string().as_str()),
                "body must name the colliding ids: {message}"
            );
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}
