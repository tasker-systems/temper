use super::*;
use uuid::Uuid;

/// Seed two profiles, a team, and a context owned by profile 1. Profile 1 is made a system
/// admin — under D11 a `kb_principal_governance` grant (`is_system_admin` reads governance, not
/// gating-team ownership). The gating-team config is retained because other tests on this
/// fixture exercise the instance-team target guard (`is_root_or_gating_team`). Fixture inserts
/// use runtime `sqlx::query(...)`, not the compile-time macro, per project convention for
/// test-fixture writes.
async fn seed_admin_team_context(pool: &PgPool) -> (ProfileId, ProfileId, Uuid, ContextId) {
    let admin: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('admin', 'Admin') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let non_admin: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('member', 'Member') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    // Every profile that ACTS needs a `<handle>@web` emitter entity for `resolve_emitter`
    // (real profiles get one on first auth). `share`/`unshare` are non-evented so they never
    // needed it, but the event-sourced `reassign` resolves the caller's emitter — without
    // this, a transfer as `admin` panics in `resolve_emitter`.
    sqlx::query(
        "INSERT INTO kb_entities (name, profile_id) \
         VALUES ('admin@web', $1), ('member@web', $2)",
    )
    .bind(admin)
    .bind(non_admin)
    .execute(pool)
    .await
    .unwrap();

    // `temper-system` is created by migration 20260625000001 — use the existing row.
    let team_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(pool)
        .await
        .unwrap();
    // Promote to `owner` of the gating team (kept for the instance-team target-guard tests
    // that share this fixture) and point the gating slug at it.
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner') \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = 'owner'",
    )
    .bind(team_id)
    .bind(admin)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1")
        .execute(pool)
        .await
        .unwrap();
    // What confers admin-ness under D11: a governance grant (`is_system_admin`).
    crate::test_support::grant_governance(pool, admin).await;

    let context_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'ctx', 'Ctx') RETURNING id",
    )
    .bind(admin)
    .fetch_one(pool)
    .await
    .unwrap();

    (
        ProfileId::from(admin),
        ProfileId::from(non_admin),
        team_id,
        ContextId::from(context_id),
    )
}

#[sqlx::test(migrations = "../../migrations")]
async fn share_is_admin_gated_and_idempotent(pool: PgPool) {
    // Non-admin caller → Forbidden.
    let (admin, non_admin, team_id, context_id) = seed_admin_team_context(&pool).await;
    let req = ShareContextRequest { team_id };
    let denied = share(
        &pool,
        &crate::test_support::human_principal_for(&pool, non_admin.uuid()).await,
        *context_id,
        &req,
    )
    .await;
    assert!(matches!(denied, Err(ApiError::Forbidden)));

    // Admin → shares; first call inserts, second is a no-op.
    let first = share(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        *context_id,
        &req,
    )
    .await
    .unwrap();
    assert!(first.shared);
    let second = share(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        *context_id,
        &req,
    )
    .await
    .unwrap();
    assert!(!second.shared);

    // Unshare removes it; second unshare is a no-op.
    let u1 = unshare(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        *context_id,
        team_id,
    )
    .await
    .unwrap();
    assert!(u1.unshared);
    let u2 = unshare(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        *context_id,
        team_id,
    )
    .await
    .unwrap();
    assert!(!u2.unshared);
}

// ── Context ownership transfer (reassign) ────────────────────────────────
//
// The target team must be NON-gating: `can_share` refuses the gating team as a target,
// so these helpers build a fresh `kb_teams` row (never `temper-system`). A caller must
// administer the source context AND manage the target team to pass the two-sided gate.

/// A profile plus its `<handle>@web` emitter entity (required by `resolve_emitter`, which
/// the event-sourced transfer resolves before firing `context_reassigned`).
async fn mk_profile_ent(pool: &PgPool, handle: &str) -> ProfileId {
    let id = crate::test_support::profile::insert(pool, handle).await;
    crate::test_support::entity::insert(pool, id, &format!("{handle}@web")).await;
    ProfileId::from(id)
}

async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
    crate::test_support::team::insert(pool, slug).await
}

async fn add_member(pool: &PgPool, team: Uuid, p: ProfileId, role: &str) {
    crate::test_support::team::set_member(pool, team, *p, role).await;
}

async fn mk_personal_context(pool: &PgPool, slug: &str, owner: ProfileId) -> Uuid {
    crate::test_support::context::insert_personal(pool, *owner, slug, slug).await
}

async fn mk_team_context(pool: &PgPool, slug: &str, team: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
         VALUES ($1, $1, 'kb_teams', $2) RETURNING id",
    )
    .bind(slug)
    .bind(team)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn mk_homed_resource(pool: &PgPool, ctx: Uuid, owner: ProfileId) -> Uuid {
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('r', 'r') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(rid)
    .bind(ctx)
    .bind(*owner)
    .execute(pool)
    .await
    .unwrap();
    rid
}

async fn owner_of_context(pool: &PgPool, ctx: Uuid) -> (String, Uuid) {
    let row = sqlx::query!(
        r#"SELECT owner_table AS "t!", owner_id AS "i!" FROM kb_contexts WHERE id = $1"#,
        ctx
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (row.t, row.i)
}

async fn can_modify(pool: &PgPool, profile: ProfileId, resource: Uuid) -> bool {
    sqlx::query_scalar!(
        r#"SELECT can_modify_resource($1, $2) AS "e!""#,
        *profile,
        resource
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn transfers_personal_context_to_team_and_members_can_author(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await; // context owner + team owner
    let bob = mk_profile_ent(&pool, "bob").await; // plain member
    let wanda = mk_profile_ent(&pool, "wanda").await; // watcher
    let stranger = mk_profile_ent(&pool, "stranger").await; // non-member
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, alice, "owner").await;
    add_member(&pool, acme, bob, "member").await;
    add_member(&pool, acme, wanda, "watcher").await;
    let ctx = mk_personal_context(&pool, "proj", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;

    // Before transfer: only the owner (alice) can author; team members cannot.
    assert!(can_modify(&pool, alice, r).await);
    assert!(!can_modify(&pool, bob, r).await);

    let outcome = reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice.uuid()).await,
        ctx,
        acme,
    )
    .await
    .expect("transfer");
    assert!(outcome.reassigned);
    assert_eq!(outcome.owner_ref, "+acme");
    assert_eq!(
        owner_of_context(&pool, ctx).await,
        ("kb_teams".to_string(), acme)
    );

    // After transfer: authoring-role members can author; watcher cannot; non-member cannot.
    assert!(can_modify(&pool, bob, r).await, "member can author");
    assert!(
        !can_modify(&pool, wanda, r).await,
        "watcher stays read-only"
    );
    assert!(
        !can_modify(&pool, stranger, r).await,
        "non-member unaffected"
    );
    assert!(
        can_modify(&pool, alice, r).await,
        "transferrer retains write"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn transfer_is_idempotent(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, alice, "owner").await;
    let ctx = mk_personal_context(&pool, "proj", alice).await;

    let first = reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice.uuid()).await,
        ctx,
        acme,
    )
    .await
    .unwrap();
    assert!(first.reassigned);
    let second = reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice.uuid()).await,
        ctx,
        acme,
    )
    .await
    .unwrap();
    assert!(!second.reassigned, "already team-owned → no-op");
}

#[sqlx::test(migrations = "../../migrations")]
async fn non_context_admin_forbidden(pool: PgPool) {
    // mallory manages the target team but does NOT own alice's context.
    let alice = mk_profile_ent(&pool, "alice").await;
    let mallory = mk_profile_ent(&pool, "mallory").await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, mallory, "owner").await;
    let ctx = mk_personal_context(&pool, "proj", alice).await;

    let err = reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, mallory.uuid()).await,
        ctx,
        acme,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}

#[sqlx::test(migrations = "../../migrations")]
async fn non_target_manager_forbidden(pool: PgPool) {
    // alice owns the context but is only a plain member of the target team.
    let alice = mk_profile_ent(&pool, "alice").await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, alice, "member").await;
    let ctx = mk_personal_context(&pool, "proj", alice).await;

    let err = reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice.uuid()).await,
        ctx,
        acme,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}

#[sqlx::test(migrations = "../../migrations")]
async fn gating_team_target_forbidden(pool: PgPool) {
    // The gating-team refusal applies to non-admins — a system admin bypasses `can_share`
    // entirely (mirrors context_share_e2e's `share_into_gating_team_denied_for_non_admin`).
    // So use a non-admin who owns the context AND even maintains the gating team.
    let (_admin, _member, gating_team, _ctx) = seed_admin_team_context(&pool).await;
    let alice = mk_profile_ent(&pool, "alice").await;
    let alice_ctx = mk_personal_context(&pool, "proj", alice).await;
    add_member(&pool, gating_team, alice, "maintainer").await;

    let err = reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice.uuid()).await,
        alice_ctx,
        gating_team,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Forbidden));
}

/// Team DAG review G14: when the gating team is some other team, `temper-system` is still every
/// profile's ancestor, so a share or reassign into it publishes to the whole instance. A
/// non-admin who manages it is refused both, by the Rust gate and by `context_reassign`'s
/// plpgsql twin; the same caller shares into an ordinary team they maintain (the control).
#[sqlx::test(migrations = "../../migrations")]
async fn the_root_is_a_target_for_admins_only_when_it_is_not_the_gating_team(pool: PgPool) {
    let gate = mk_team(&pool, "gate").await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'gate' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(root, gate, "fixture: the root must not be the gating team");

    let alice = mk_profile_ent(&pool, "alice").await;
    let alice_ctx = mk_personal_context(&pool, "proj", alice).await;
    add_member(&pool, root, alice, "maintainer").await;
    let ordinary = mk_team(&pool, "ordinary").await;
    add_member(&pool, ordinary, alice, "maintainer").await;
    let authed = crate::test_support::human_principal_for(&pool, alice.uuid()).await;

    let err = share(
        &pool,
        &authed,
        alice_ctx,
        &ShareContextRequest { team_id: root },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, ApiError::Forbidden),
        "share into the root: {err:?}"
    );
    let err = reassign(&pool, &authed, alice_ctx, root).await.unwrap_err();
    assert!(
        matches!(err, ApiError::Forbidden),
        "reassign into the root: {err:?}"
    );

    // The plpgsql twin refuses on its own, without the Rust pre-check in front of it.
    let emitter: Uuid = sqlx::query_scalar("SELECT id FROM kb_entities WHERE name = 'alice@web'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let sql_err = sqlx::query("SELECT context_reassign($1, $2)")
        .bind(serde_json::json!({
            "context_id": alice_ctx,
            "to_owner_table": "kb_teams",
            "to_owner_id": root,
        }))
        .bind(emitter)
        .execute(&pool)
        .await
        .expect_err("context_reassign must refuse the root to a non-admin");
    assert!(
        sql_err
            .to_string()
            .contains("cannot transfer into temper-system"),
        "{sql_err}"
    );

    share(
        &pool,
        &authed,
        alice_ctx,
        &ShareContextRequest { team_id: ordinary },
    )
    .await
    .expect("control: a share into an ordinary team the caller maintains");
}

/// Task 01a19006: attaching a context to someone's personal team, or detaching one from it, is
/// acting on that team — its owner's alone. A system admin is refused both ways; the owner
/// shares their own context into it as before.
#[sqlx::test(migrations = "../../migrations")]
async fn a_personal_team_target_is_its_owners_alone(pool: PgPool) {
    let (admin, _member, _gating, admin_ctx) = seed_admin_team_context(&pool).await;
    let alice = mk_profile_ent(&pool, "alice").await;
    let alice_ctx = mk_personal_context(&pool, "proj", alice).await;
    let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(*alice)
        .fetch_one(&pool)
        .await
        .unwrap();
    let req = ShareContextRequest { team_id: personal };

    let as_alice = crate::test_support::human_principal_for(&pool, alice.uuid()).await;
    assert!(
        share(&pool, &as_alice, alice_ctx, &req)
            .await
            .unwrap()
            .shared
    );

    let as_admin = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
    // Refused, and told what they hit (task `01a1933d`).
    let told = |err: &ApiError| matches!(err, ApiError::ForbiddenDetail(m) if m.starts_with("this team is @alice's personal team"));
    let err = share(&pool, &as_admin, *admin_ctx, &req).await.unwrap_err();
    assert!(told(&err), "got {err:?}");
    let err = unshare(&pool, &as_admin, alice_ctx, personal)
        .await
        .unwrap_err();
    assert!(told(&err), "got {err:?}");
    let shares: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_team_contexts WHERE team_id = $1")
            .bind(personal)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(shares, 1, "only alice's own share stands");
}

#[sqlx::test(migrations = "../../migrations")]
async fn slug_collision_is_conflict(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, alice, "owner").await;
    // The team already owns a context with slug "proj"; alice's personal one collides.
    mk_team_context(&pool, "proj", acme).await;
    let ctx = mk_personal_context(&pool, "proj", alice).await;

    let err = reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice.uuid()).await,
        ctx,
        acme,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ApiError::Conflict(_)));
    assert_eq!(
        owner_of_context(&pool, ctx).await,
        ("kb_profiles".to_string(), *alice),
        "owner unchanged on conflict"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn system_admin_can_transfer_any_context(pool: PgPool) {
    // The seeded admin is a system admin (holds a governance grant). Transfer a context
    // it owns to a fresh non-gating team it also owns.
    let (admin, _member, _gating, context_id) = seed_admin_team_context(&pool).await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, admin, "owner").await;

    let outcome = reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin.uuid()).await,
        *context_id,
        acme,
    )
    .await
    .expect("admin transfer");
    assert!(outcome.reassigned);
    assert_eq!(
        owner_of_context(&pool, *context_id).await,
        ("kb_teams".to_string(), acme)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn transfer_emits_context_reassigned_event(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, alice, "owner").await;
    let ctx = mk_personal_context(&pool, "proj", alice).await;
    reassign(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice.uuid()).await,
        ctx,
        acme,
    )
    .await
    .unwrap();

    let n = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE t.name = 'context_reassigned' AND (e.payload->>'context_id')::uuid = $1",
        ctx,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap();
    assert_eq!(n, 1);
}

/// The `<handle>@web` emitter entity id for a profile (what `resolve_emitter` returns and what
/// `context_reassign` reads back to `kb_entities.profile_id` for its authz).
async fn emitter_of(pool: &PgPool, p: ProfileId, handle: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 AND name = $2")
        .bind(*p)
        .bind(format!("{handle}@web"))
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The RBAC gate is an invariant of the `context_reassign` SQL function, not merely a service
/// pre-check — this is the TOCTOU backstop the review asked for. Calling the function DIRECTLY
/// (bypassing `can_share`) with an unauthorized emitter must still be rejected, atomically:
/// mallory manages the target team but does not administer alice's context, so the guard raises
/// SQLSTATE 42501 and the owner is left untouched.
#[sqlx::test(migrations = "../../migrations")]
async fn sql_guard_rejects_unauthorized_emitter_directly(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let mallory = mk_profile_ent(&pool, "mallory").await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, mallory, "owner").await;
    let ctx = mk_personal_context(&pool, "proj", alice).await;

    let payload = temper_substrate::payloads::ContextReassigned {
        context_id: ContextId::from(ctx),
        from_owner_table: "kb_profiles".to_string(),
        from_owner_id: *alice,
        to_owner_table: "kb_teams".to_string(),
        to_owner_id: acme,
    };
    let payload_json = serde_json::to_value(&payload).unwrap();
    let emitter = emitter_of(&pool, mallory, "mallory").await;

    let res = sqlx::query_scalar::<_, Uuid>("SELECT context_reassign($1,$2,$3,$4,$5)")
        .bind(&payload_json)
        .bind(emitter)
        .bind(serde_json::json!({}))
        .bind(None::<Uuid>)
        .bind(None::<Uuid>)
        .fetch_one(&pool)
        .await;

    match res.unwrap_err() {
        sqlx::Error::Database(db) => assert_eq!(
            db.code().as_deref(),
            Some("42501"),
            "authz raise must use insufficient_privilege"
        ),
        other => panic!("expected the SQL RBAC guard to raise, got {other:?}"),
    }
    // Atomic: the rejected write leaves ownership untouched (no partial mutation).
    assert_eq!(
        owner_of_context(&pool, ctx).await,
        ("kb_profiles".to_string(), *alice),
        "owner unchanged when the SQL guard rejects"
    );
}

/// The system-admin bypass also holds at the SQL layer: the seeded admin (governance grant)
/// may reassign a context it administers directly through the function.
#[sqlx::test(migrations = "../../migrations")]
async fn sql_guard_allows_system_admin_directly(pool: PgPool) {
    let (admin, _member, _gating, context_id) = seed_admin_team_context(&pool).await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, admin, "owner").await;

    let payload = temper_substrate::payloads::ContextReassigned {
        context_id,
        from_owner_table: "kb_profiles".to_string(),
        from_owner_id: *admin,
        to_owner_table: "kb_teams".to_string(),
        to_owner_id: acme,
    };
    let payload_json = serde_json::to_value(&payload).unwrap();
    let emitter = emitter_of(&pool, admin, "admin").await;

    sqlx::query_scalar::<_, Uuid>("SELECT context_reassign($1,$2,$3,$4,$5)")
        .bind(&payload_json)
        .bind(emitter)
        .bind(serde_json::json!({}))
        .bind(None::<Uuid>)
        .bind(None::<Uuid>)
        .fetch_one(&pool)
        .await
        .expect("system admin transfer through the SQL function");

    assert_eq!(
        owner_of_context(&pool, *context_id).await,
        ("kb_teams".to_string(), acme)
    );
}

// ── Context rename ───────────────────────────────────────────────────────
//
// A rename moves the `(name, slug)` identity pair in place under an UNCHANGED owner. That is
// the difference from `reassign` that every trap below descends from: the row can match itself.

/// A personal context whose stored `name` differs from its `slug` — `mk_personal_context` sets
/// both to the same string, which cannot express the legacy-repair or name-only cases.
async fn mk_named_personal_context(
    pool: &PgPool,
    slug: &str,
    name: &str,
    owner: ProfileId,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
         VALUES ($1, $2, 'kb_profiles', $3) RETURNING id",
    )
    .bind(slug)
    .bind(name)
    .bind(*owner)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The stored identity pair, read straight from the row — the projection, not the outcome.
async fn name_slug_of(pool: &PgPool, ctx: Uuid) -> (String, String) {
    let row = sqlx::query!("SELECT name, slug FROM kb_contexts WHERE id = $1", ctx)
        .fetch_one(pool)
        .await
        .unwrap();
    (row.name, row.slug)
}

/// How many `context_renamed` events this context carries. The trail is the ONLY attributability
/// surface a rename has — `kb_contexts` keeps no before-image and no `updated` column.
async fn rename_events(pool: &PgPool, ctx: Uuid) -> i64 {
    sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE t.name = 'context_renamed' AND (e.payload->>'context_id')::uuid = $1",
        ctx,
    )
    .fetch_one(pool)
    .await
    .unwrap()
    .unwrap()
}

async fn is_readable(pool: &PgPool, profile: ProfileId, resource: Uuid) -> bool {
    sqlx::query_scalar!(
        r#"SELECT EXISTS(
                 SELECT 1 FROM resources_visible_to($1) WHERE resource_id = $2) AS "e!""#,
        *profile,
        resource,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The empty-slug `400`, and **both** of the ways to reach it — the spec's "one `400` check, not
/// two": an all-whitespace name canonicalizes to empty, a punctuation-only name sluggifies to
/// empty. Neither may fall through to `create`'s `"context"` fallback, which would silently
/// re-address the context to a slug the caller never asked for.
#[sqlx::test(migrations = "../../migrations")]
async fn a_name_with_no_addressable_content_is_refused(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_named_personal_context(&pool, "notes", "Notes", alice).await;

    for bad in ["   ", "!!!"] {
        match rename(
            &pool,
            &crate::test_support::caller_for(&pool, alice.uuid()).await,
            ctx,
            bad,
        )
        .await
        {
            Err(ApiError::BadRequest(_)) => {}
            other => panic!("{bad:?} must be refused with 400, got {other:?}"),
        }
    }
    assert_eq!(
        name_slug_of(&pool, ctx).await,
        ("Notes".to_string(), "notes".to_string()),
        "a refused rename writes nothing"
    );
    assert_eq!(rename_events(&pool, ctx).await, 0);
}

/// The no-op arm: the canonical name already equals the stored one, so nothing is written and
/// **no event is emitted**. The input differs from the stored name only in whitespace, which is
/// precisely what canonicalization erases.
#[sqlx::test(migrations = "../../migrations")]
async fn a_canonically_identical_name_is_a_no_op_and_emits_nothing(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_named_personal_context(&pool, "temper-kb", "Temper KB", alice).await;

    let outcome = rename(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
        "  Temper   KB  ",
    )
    .await
    .unwrap();
    assert!(!outcome.renamed);
    assert_eq!(outcome.name, "Temper KB");
    assert_eq!(outcome.slug, "temper-kb");
    assert_eq!(outcome.context_ref, "@alice/temper-kb");
    assert_eq!(rename_events(&pool, ctx).await, 0, "no-op emits no event");
}

/// The `409`, and the divergence from `create` it encodes: rename does not auto-suffix, so the
/// caller is told the address is taken rather than handed `notes-2` without being asked.
#[sqlx::test(migrations = "../../migrations")]
async fn a_slug_taken_by_another_context_under_the_same_owner_conflicts(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    mk_named_personal_context(&pool, "notes", "Notes", alice).await;
    let scratch = mk_named_personal_context(&pool, "scratch", "Scratch", alice).await;

    match rename(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        scratch,
        "Notes",
    )
    .await
    .unwrap_err()
    {
        ApiError::Conflict(msg) => assert!(
            msg.contains("notes"),
            "the refusal names the colliding slug: {msg}"
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }
    assert_eq!(
        name_slug_of(&pool, scratch).await,
        ("Scratch".to_string(), "scratch".to_string()),
        "a refused rename writes nothing"
    );
    assert_eq!(rename_events(&pool, scratch).await, 0);
}

/// The happy path: **both** columns move, the event lands, and the outcome carries the composed
/// new ref — the address the caller must use from now on, since the old one no longer resolves.
#[sqlx::test(migrations = "../../migrations")]
async fn a_rename_moves_both_name_and_slug_and_emits(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_named_personal_context(&pool, "proj", "Proj", alice).await;

    let outcome = rename(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
        "  Temper   KB ",
    )
    .await
    .unwrap();
    assert!(outcome.renamed);
    assert_eq!(outcome.name, "Temper KB", "stored canonical, not verbatim");
    assert_eq!(outcome.slug, "temper-kb");
    assert_eq!(outcome.owner_ref, "@alice");
    assert_eq!(
        outcome.context_ref, "@alice/temper-kb",
        "one sigil, not two — composed from the already-decorated owner_ref"
    );
    assert_eq!(
        name_slug_of(&pool, ctx).await,
        ("Temper KB".to_string(), "temper-kb".to_string())
    );
    assert_eq!(rename_events(&pool, ctx).await, 1);
}

/// **The self-exclusion regression test.** A rename whose slug does not move is still a rename.
/// `reassign`'s collision query has no `AND id <> …` and correctly needs none; copied verbatim
/// here it would find this context's own row and 409 against its own slug.
#[sqlx::test(migrations = "../../migrations")]
async fn a_name_only_rename_is_still_a_rename_and_does_not_collide_with_itself(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    // "Temper KB" and "Temper-KB" are different names that sluggify identically.
    let ctx = mk_named_personal_context(&pool, "temper-kb", "Temper KB", alice).await;

    let outcome = rename(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
        "Temper-KB",
    )
    .await
    .unwrap();
    assert!(outcome.renamed, "a display-name change is not a no-op");
    assert_eq!(outcome.slug, "temper-kb", "the address stayed put");
    assert_eq!(
        name_slug_of(&pool, ctx).await,
        ("Temper-KB".to_string(), "temper-kb".to_string())
    );
    assert_eq!(
        rename_events(&pool, ctx).await,
        1,
        "it emits like any rename"
    );
}

/// **The legacy-repair case**, and the reason the no-op arm compares the NAME. This context's
/// stored name predates canonicalization; its own repaired form sluggifies identically, so a
/// slug-comparison no-op arm would decline the one rename that fixes it — permanently.
#[sqlx::test(migrations = "../../migrations")]
async fn a_non_canonical_stored_name_can_be_repaired_to_its_canonical_form(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_named_personal_context(&pool, "temper-kb", "Temper  KB", alice).await;

    let outcome = rename(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
        "Temper KB",
    )
    .await
    .unwrap();
    assert!(outcome.renamed, "the repair is a real state change");
    assert_eq!(
        name_slug_of(&pool, ctx).await,
        ("Temper KB".to_string(), "temper-kb".to_string())
    );
}

/// `canonical_name`'s **second** caller: a create stores the canonical name too. Without this
/// the invariant would be honoured by one of two write paths, which is not an invariant.
#[sqlx::test(migrations = "../../migrations")]
async fn create_stores_a_canonical_name(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;

    let row = create(&pool, alice, "kb_profiles", *alice, "  Temper   KB \n")
        .await
        .unwrap();
    assert_eq!(row.name, "Temper KB");
    assert_eq!(row.slug, "temper-kb");
    assert_eq!(
        name_slug_of(&pool, *row.id).await,
        ("Temper KB".to_string(), "temper-kb".to_string())
    );
}

/// The gate is wired, and it answers in **two** dialects: a plain member of the owning team can
/// read the context but does not `can_manage` it → `403`; a stranger cannot see it at all →
/// the incumbent `404`, byte-identical to `get_visible`'s refusal.
#[sqlx::test(migrations = "../../migrations")]
async fn rename_refuses_a_reader_with_403_and_a_stranger_with_404(pool: PgPool) {
    let member = mk_profile_ent(&pool, "member").await;
    let stranger = mk_profile_ent(&pool, "stranger").await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, member, "member").await;
    let ctx = mk_team_context(&pool, "eng-notes", acme).await;

    assert!(matches!(
        rename(
            &pool,
            &crate::test_support::caller_for(&pool, member.uuid()).await,
            ctx,
            "Engineering Notes"
        )
        .await
        .unwrap_err(),
        ApiError::Forbidden
    ));
    match rename(
        &pool,
        &crate::test_support::caller_for(&pool, stranger.uuid()).await,
        ctx,
        "Engineering Notes",
    )
    .await
    .unwrap_err()
    {
        ApiError::NotFound(msg) => assert_eq!(msg, CONTEXT_REFUSAL),
        other => panic!("expected NotFound, got {other:?}"),
    }
    assert_eq!(rename_events(&pool, ctx).await, 0);
}

/// The RBAC gate is an INVARIANT of `context_rename`, not merely a service pre-check — the
/// TOCTOU backstop. Calling the function DIRECTLY with an emitter belonging to a profile that
/// does not administer the context must still be rejected, atomically: `42501`, row untouched.
/// (Model: `sql_guard_rejects_unauthorized_emitter_directly` above.)
#[sqlx::test(migrations = "../../migrations")]
async fn sql_guard_rejects_unauthorized_rename_emitter_directly(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let mallory = mk_profile_ent(&pool, "mallory").await;
    let ctx = mk_named_personal_context(&pool, "proj", "Proj", alice).await;

    let payload = temper_substrate::payloads::ContextRenamed {
        context_id: ContextId::from(ctx),
        from_name: "Proj".to_string(),
        from_slug: "proj".to_string(),
        to_name: "Hijacked".to_string(),
        to_slug: "hijacked".to_string(),
    };
    let payload_json = serde_json::to_value(&payload).unwrap();
    let emitter = emitter_of(&pool, mallory, "mallory").await;

    let res = sqlx::query_scalar::<_, Uuid>("SELECT context_rename($1,$2,$3,$4,$5)")
        .bind(&payload_json)
        .bind(emitter)
        .bind(serde_json::json!({}))
        .bind(None::<Uuid>)
        .bind(None::<Uuid>)
        .fetch_one(&pool)
        .await;

    match res.unwrap_err() {
        sqlx::Error::Database(db) => assert_eq!(
            db.code().as_deref(),
            Some("42501"),
            "authz raise must use insufficient_privilege"
        ),
        other => panic!("expected the SQL RBAC guard to raise, got {other:?}"),
    }
    assert_eq!(
        name_slug_of(&pool, ctx).await,
        ("Proj".to_string(), "proj".to_string()),
        "identity pair unchanged when the SQL guard rejects"
    );
}

/// The system-admin bypass holds at the SQL layer too. The subject is a context the admin
/// **does not own** — the arm that admits an actor who cannot even *read* the context, and the
/// reason the Rust gate probes `is_system_admin` above visibility. An admin-owned subject would
/// have passed through the ownership branch and proved nothing about the bypass.
#[sqlx::test(migrations = "../../migrations")]
async fn sql_guard_allows_system_admin_rename_directly(pool: PgPool) {
    let (admin, _member, _gating, _own_ctx) = seed_admin_team_context(&pool).await;
    // A shared team's context: someone else's personal context is outside the bypass (task
    // `01a191b4`, pinned by `a_system_admin_does_not_act_on_or_list_someone_elses_personal_context`).
    let elsewhere = mk_team(&pool, "elsewhere").await;
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
         VALUES ('private', 'Private', 'kb_teams', $1) RETURNING id",
    )
    .bind(elsewhere)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !context_visible(&pool, *admin, ctx).await.unwrap(),
        "fixture precondition: the admin must NOT be able to read this context"
    );

    let payload = temper_substrate::payloads::ContextRenamed {
        context_id: ContextId::from(ctx),
        from_name: "Private".to_string(),
        from_slug: "private".to_string(),
        to_name: "Renamed By Admin".to_string(),
        to_slug: "renamed-by-admin".to_string(),
    };
    let payload_json = serde_json::to_value(&payload).unwrap();
    let emitter = emitter_of(&pool, admin, "admin").await;

    sqlx::query_scalar::<_, Uuid>("SELECT context_rename($1,$2,$3,$4,$5)")
        .bind(&payload_json)
        .bind(emitter)
        .bind(serde_json::json!({}))
        .bind(None::<Uuid>)
        .bind(None::<Uuid>)
        .fetch_one(&pool)
        .await
        .expect("system admin rename through the SQL function");

    assert_eq!(
        name_slug_of(&pool, ctx).await,
        (
            "Renamed By Admin".to_string(),
            "renamed-by-admin".to_string()
        )
    );
}

/// `a-context-never-loses-its-contents-to-a-rename`. Spec §"What a rename does not touch" argues
/// this holds *because nothing is keyed by slug* — resources are homed on `(anchor_table,
/// anchor_id)` where `anchor_id` is a UUID. The argument is sound and it is still an argument;
/// this makes it falsifiable.
#[sqlx::test(migrations = "../../migrations")]
async fn a_rename_does_not_disturb_the_contexts_contents(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let stranger = mk_profile_ent(&pool, "stranger").await;
    let ctx = mk_named_personal_context(&pool, "proj", "Proj", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;

    let before = sqlx::query!(
        r#"SELECT anchor_table AS "t!", anchor_id AS "i!", owner_profile_id
                 FROM kb_resource_homes WHERE resource_id = $1"#,
        r,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(is_readable(&pool, alice, r).await);
    assert!(!is_readable(&pool, stranger, r).await);

    let outcome = rename(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
        "Temper KB",
    )
    .await
    .unwrap();
    assert!(outcome.renamed);

    let after = sqlx::query!(
        r#"SELECT anchor_table AS "t!", anchor_id AS "i!", owner_profile_id
                 FROM kb_resource_homes WHERE resource_id = $1"#,
        r,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((after.t, after.i), (before.t, before.i), "home row unmoved");
    assert_eq!(after.owner_profile_id, before.owner_profile_id);
    assert!(is_readable(&pool, alice, r).await, "still readable");
    assert!(
        !is_readable(&pool, stranger, r).await,
        "and still not readable by anyone new"
    );
    assert!(can_modify(&pool, alice, r).await, "authorship unchanged");
}

// ── Context retirement ────────────────────────────────────────────────────

/// Retirement preserves everything. This is the whole difference from the hard delete it
/// replaces: the guard is gone because there is nothing to guard against.
#[sqlx::test(migrations = "../../migrations")]
async fn retire_preserves_every_row_it_homes(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_personal_context(&pool, "proj", alice).await;
    let r = mk_homed_resource(&pool, ctx, alice).await;

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("retire succeeds WITH a resource homed here");

    let row = sqlx::query!(
        r#"SELECT is_active AS "is_active!" FROM kb_contexts WHERE id = $1"#,
        ctx
    )
    .fetch_one(&pool)
    .await
    .expect("the kb_contexts row still exists");
    assert!(
        !row.is_active,
        "retirement flips is_active, it does not delete the row"
    );

    let resource_active: bool = sqlx::query_scalar!(
        r#"SELECT is_active AS "a!" FROM kb_resources WHERE id = $1"#,
        r
    )
    .fetch_one(&pool)
    .await
    .expect("the kb_resources row is untouched");
    assert!(resource_active, "the homed resource stays live");

    let home_count: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "c!" FROM kb_resource_homes
                WHERE resource_id = $1 AND anchor_table = 'kb_contexts' AND anchor_id = $2"#,
        r,
        ctx
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(home_count, 1, "the home row is untouched");
}

/// The address is freed; the display label is not touched.
#[sqlx::test(migrations = "../../migrations")]
async fn retire_frees_the_slug_and_keeps_the_name(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_personal_context(&pool, "scratch", alice).await;

    let out = retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("retire");
    assert!(
        out.slug.starts_with("scratch-retired"),
        "got slug {:?}",
        out.slug
    );
    assert_eq!(out.name, "scratch");

    let row = create(&pool, alice, "kb_profiles", *alice, "scratch")
        .await
        .expect("the freed slug is immediately reusable");
    assert_eq!(row.slug, "scratch");
}

/// Retiring twice is a clean refusal, never a 500 and never a second mangle.
#[sqlx::test(migrations = "../../migrations")]
async fn retiring_an_already_retired_context_is_not_found(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_personal_context(&pool, "temp", alice).await;

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("first retire succeeds");

    let second = retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await;
    assert!(
        matches!(second, Err(ApiError::NotFound(ref msg)) if msg == CONTEXT_REFUSAL),
        "expected NotFound, got {second:?}"
    );
}

// ── Context restoration ───────────────────────────────────────────────────

/// Restore round-trips the address when nothing has since claimed it.
#[sqlx::test(migrations = "../../migrations")]
async fn restore_returns_the_original_address_when_it_is_free(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_personal_context(&pool, "scratch", alice).await;

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("retire");

    let out = restore(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("restore");
    assert_eq!(out.slug, "scratch");
    assert!(!out.slug_changed, "the address was free, nothing to report");
    assert_eq!(out.name, "scratch");

    let readable: bool = sqlx::query_scalar("SELECT context_readable_by_profile($1, $2)")
        .bind(*alice)
        .bind(ctx)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(readable, "a restored context is readable again");
}

/// Restore into a collision lands on the suffix and SAYS SO. Handing back a different
/// address silently is the failure mode `rename` explicitly refuses (spec §2.4).
#[sqlx::test(migrations = "../../migrations")]
async fn restore_into_a_taken_address_suffixes_and_reports_it(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_personal_context(&pool, "scratch", alice).await;

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("retire");

    // A new context claims the freed slug before the first one is restored.
    create(&pool, alice, "kb_profiles", *alice, "scratch")
        .await
        .expect("the freed slug is immediately reusable");

    let out = restore(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("restore");
    assert_eq!(out.slug, "scratch-2", "got slug {:?}", out.slug);
    assert!(
        out.slug_changed,
        "landing on a suffix must be reported, not applied silently"
    );
}

/// The baseline is the address the caller RETIRED UNDER, not the name's canonical slug.
/// `restore_into_a_taken_address_suffixes_and_reports_it` cannot see the difference: its
/// context is named `scratch`, where the two coincide by construction. Here they do not —
/// two contexts share the name "Notes", so the
/// second lives at `notes-2`. Retire both, restore the second, and it lands on the freed
/// `notes`: the caller handed in `@alice/notes-2` and gets `@alice/notes` back. Against
/// `sluggify("Notes")` (= `notes`) that reads as UNCHANGED — the silent re-addressing this
/// field exists to refuse.
#[sqlx::test(migrations = "../../migrations")]
async fn restore_reports_a_change_when_it_lands_on_a_freed_sibling_address(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let first = create(&pool, alice, "kb_profiles", *alice, "Notes")
        .await
        .expect("create the first Notes");
    let second = create(&pool, alice, "kb_profiles", *alice, "Notes")
        .await
        .expect("create the second Notes");
    assert_eq!(first.slug, "notes");
    assert_eq!(
        second.slug, "notes-2",
        "the same name a second time is suffixed, got {:?}",
        second.slug
    );

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        *second.id,
    )
    .await
    .expect("retire notes-2");
    // Retiring the first frees `notes`, so the restore below can reclaim it.
    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        *first.id,
    )
    .await
    .expect("retire notes");

    let out = restore(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        *second.id,
    )
    .await
    .expect("restore");
    assert_eq!(out.slug, "notes", "got slug {:?}", out.slug);
    assert_eq!(out.context_ref, "@alice/notes");
    assert!(
        out.slug_changed,
        "retired under `notes-2`, handed back `notes` — that must be reported"
    );
}

/// The mirror: landing back on the suffixed address it was retired under is NO change, even
/// though that address is not the name's canonical slug. `first` stays live and keeps `notes`,
/// so the restore returns to `notes-2` — the exact address the caller retired. Against
/// `sluggify("Notes")` this reads as changed, sending the caller to re-address a context that
/// never moved.
#[sqlx::test(migrations = "../../migrations")]
async fn restore_reports_no_change_when_it_lands_back_on_its_suffixed_address(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let first = create(&pool, alice, "kb_profiles", *alice, "Notes")
        .await
        .expect("create the first Notes");
    let second = create(&pool, alice, "kb_profiles", *alice, "Notes")
        .await
        .expect("create the second Notes");
    assert_eq!(first.slug, "notes");
    assert_eq!(second.slug, "notes-2");

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        *second.id,
    )
    .await
    .expect("retire notes-2");

    let out = restore(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        *second.id,
    )
    .await
    .expect("restore");
    assert_eq!(out.slug, "notes-2", "got slug {:?}", out.slug);
    assert_eq!(out.context_ref, "@alice/notes-2");
    assert!(
        !out.slug_changed,
        "back on the address it was retired under — nothing to report"
    );
}

/// Restoring a context that was never retired is a clean refusal.
#[sqlx::test(migrations = "../../migrations")]
async fn restoring_a_live_context_is_not_found(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let ctx = mk_personal_context(&pool, "scratch", alice).await;

    let out = restore(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await;
    assert!(
        matches!(out, Err(ApiError::NotFound(ref msg)) if msg == CONTEXT_REFUSAL),
        "expected NotFound, got {out:?}"
    );
}

// ── The admin axis (retired listing / show) ─────────────────────────────────

/// You can see a retired context only if you could have retired it. A team MEMBER who could
/// read and author a team-owned context before retirement sees NOTHING in the retired
/// listing; an owner/maintainer of that team sees it. This is the property that makes the
/// listing an admin-axis door and not `list_visible` with a flipped flag.
#[sqlx::test(migrations = "../../migrations")]
async fn the_retired_listing_rides_the_admin_axis_not_the_read_axis(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await; // team owner
    let bob = mk_profile_ent(&pool, "bob").await; // plain member
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, alice, "owner").await;
    add_member(&pool, acme, bob, "member").await;
    let ctx = mk_team_context(&pool, "proj", acme).await;

    // Before retirement: bob (a plain member) can read AND author the context.
    let r = mk_homed_resource(&pool, ctx, alice).await;
    assert!(
        context_visible(&pool, *bob, ctx).await.unwrap(),
        "member can read before retirement"
    );
    assert!(
        can_modify(&pool, bob, r).await,
        "member can author before retirement"
    );

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("retire");

    // After retirement: the member — who could read and author it a moment ago — sees
    // nothing in the retired listing. This is the property under test.
    let bob_listing = list_retired_administered(
        &pool,
        &crate::test_support::caller_for(&pool, bob.uuid()).await,
    )
    .await
    .unwrap();
    assert!(
        bob_listing.is_empty(),
        "a member who could read/author before retirement must see nothing after: {bob_listing:?}"
    );

    // The owner, who manages the owning team, sees exactly the one retired context.
    let alice_listing = list_retired_administered(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
    )
    .await
    .unwrap();
    assert_eq!(alice_listing.len(), 1, "got {alice_listing:?}");
    assert_eq!(alice_listing[0].id, ContextId::from(ctx));
    assert!(alice_listing[0].retired, "the listed row must say retired");
}

/// Listing a thing you cannot then inspect is an incoherent pair: `get_retired_administered`
/// must answer for exactly what `list_retired_administered` lists.
#[sqlx::test(migrations = "../../migrations")]
async fn an_administrator_can_show_a_retired_context(pool: PgPool) {
    let alice = mk_profile_ent(&pool, "alice").await;
    let bob = mk_profile_ent(&pool, "bob").await; // plain member — must NOT be able to show it
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, alice, "owner").await;
    add_member(&pool, acme, bob, "member").await;
    let ctx = mk_team_context(&pool, "proj", acme).await;

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ctx,
    )
    .await
    .expect("retire");

    let shown = get_retired_administered(
        &pool,
        &crate::test_support::caller_for(&pool, alice.uuid()).await,
        ContextId::from(ctx),
    )
    .await
    .expect("the administrator can show the retired context");
    assert_eq!(shown.id, ContextId::from(ctx));
    assert!(shown.retired, "the shown row must say retired");

    let denied = get_retired_administered(
        &pool,
        &crate::test_support::caller_for(&pool, bob.uuid()).await,
        ContextId::from(ctx),
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::NotFound(ref msg)) if msg == CONTEXT_REFUSAL),
        "a non-administering member must be refused, got {denied:?}"
    );
}

/// The third admitting arm: a system admin sees a retired context through
/// [`list_retired_administered`] and [`get_retired_administered`] with **neither** ownership
/// **nor** team membership — the property the other two retired-listing tests above cannot
/// exercise, since their admins are also the context's owner or a manager of its owning team.
///
/// `root` here has no row in `kb_contexts` naming it as owner and no row in
/// `kb_team_members` at all, so the first two arms of both queries' WHERE clause are false for
/// every context in this fixture — only the `$3`/`$4` system-admin bool can admit it. Without
/// that arm, `root_listing` would be empty (failing the `len() == 1` assertion below) and
/// `get_retired_administered` would return the `CONTEXT_REFUSAL` `NotFound` both non-admins get
/// above, rather than the row.
#[sqlx::test(migrations = "../../migrations")]
async fn a_system_admin_sees_a_retired_context_they_neither_own_nor_manage(pool: PgPool) {
    let owner = mk_profile_ent(&pool, "owner").await;
    let root = mk_profile_ent(&pool, "root").await;
    // `root` is a system admin under D11 (a `kb_principal_governance` grant), and nothing
    // else: no `kb_contexts` row it owns, no `kb_team_members` row at all — the fixture the
    // other two retired-listing tests do not build, and the one this arm needs.
    crate::test_support::grant_governance(&pool, *root).await;

    // A team's context: personal space is outside the admin arm (task `01a191b4`, pinned by
    // `a_system_admin_does_not_act_on_or_list_someone_elses_personal_context` below).
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, owner, "owner").await;
    let ctx = mk_team_context(&pool, "priv", acme).await;
    mk_homed_resource(&pool, ctx, owner).await;

    retire(
        &pool,
        &crate::test_support::caller_for(&pool, owner.uuid()).await,
        ctx,
    )
    .await
    .expect("retire");

    let root_listing = list_retired_administered(
        &pool,
        &crate::test_support::caller_for(&pool, root.uuid()).await,
    )
    .await
    .unwrap();
    assert_eq!(
        root_listing.len(),
        1,
        "a system admin with no ownership and no membership must still see the retired \
         context: got {root_listing:?}"
    );
    assert_eq!(root_listing[0].id, ContextId::from(ctx));
    assert!(root_listing[0].retired, "the listed row must say retired");

    let shown = get_retired_administered(
        &pool,
        &crate::test_support::caller_for(&pool, root.uuid()).await,
        ContextId::from(ctx),
    )
    .await
    .expect("a system admin can show a retired context they neither own nor manage");
    assert_eq!(shown.id, ContextId::from(ctx));
    assert!(shown.retired, "the shown row must say retired");
}

/// Task 01a191b4: a system admin does not act on someone else's personal context — a profile's
/// own, or one their personal team owns — nor list it once retired. Every door refuses: rename,
/// retire, restore, share, unshare and reassign through the services, and the plpgsql twin
/// (`context_rename`) on its own. Its owner keeps every one of them.
#[sqlx::test(migrations = "../../migrations")]
async fn a_system_admin_does_not_act_on_or_list_someone_elses_personal_context(pool: PgPool) {
    let (admin, _member, _gating, _admin_ctx) = seed_admin_team_context(&pool).await;
    let alice = mk_profile_ent(&pool, "alice").await;
    let acme = mk_team(&pool, "acme").await;
    add_member(&pool, acme, admin, "owner").await;
    let mine = mk_personal_context(&pool, "journal", alice).await;
    let personal_team: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(*alice)
        .fetch_one(&pool)
        .await
        .unwrap();
    let theirs = mk_team_context(&pool, "drafts", personal_team).await;

    let as_admin = crate::test_support::human_principal_for(&pool, admin.uuid()).await;
    let admin_caller = crate::test_support::caller_for(&pool, admin.uuid()).await;
    for ctx in [mine, theirs] {
        assert!(is_others_personal_context(&pool, ctx, admin).await.unwrap());
        let err = share(
            &pool,
            &as_admin,
            ctx,
            &ShareContextRequest { team_id: acme },
        )
        .await
        .unwrap_err();
        // Every door refuses with the same sentence (task `01a1933d`).
        let told = |err: &ApiError| {
            matches!(err, ApiError::ForbiddenDetail(m)
                if m == "this context is @alice's personal space; the admin arm does not reach it")
        };
        assert!(told(&err), "share: {err:?}");
        let err = reassign(&pool, &as_admin, ctx, acme).await.unwrap_err();
        assert!(told(&err), "reassign: {err:?}");
        let err = rename(&pool, &admin_caller, ctx, "Renamed")
            .await
            .unwrap_err();
        assert!(told(&err), "rename: {err:?}");
        let err = retire(&pool, &admin_caller, ctx).await.unwrap_err();
        assert!(told(&err), "retire: {err:?}");
    }

    // The plpgsql twin refuses on its own: the admin's emitter is no bypass on personal space.
    let emitter: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 LIMIT 1")
            .bind(*admin)
            .fetch_one(&pool)
            .await
            .unwrap();
    let err = sqlx::query_scalar::<_, Uuid>("SELECT context_rename($1, $2)")
        .bind(serde_json::json!({
            "context_id": mine, "from_name": "journal", "to_name": "x",
            "from_slug": "journal", "to_slug": "x"
        }))
        .bind(emitter)
        .fetch_one(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("42501"),
        "{err}"
    );

    // Its owner retires it; the admin neither restores it nor sees it listed.
    let alice_caller = crate::test_support::caller_for(&pool, alice.uuid()).await;
    retire(&pool, &alice_caller, mine)
        .await
        .expect("the owner retires");
    let err = restore(&pool, &admin_caller, mine).await.unwrap_err();
    assert!(
        matches!(&err, ApiError::ForbiddenDetail(m) if m.starts_with("this context is @alice's")),
        "restore: {err:?}"
    );
    let listed = list_retired_administered(&pool, &admin_caller)
        .await
        .unwrap();
    assert!(
        listed.iter().all(|row| row.id != ContextId::from(mine)),
        "{listed:?}"
    );
    let err = get_retired_administered(&pool, &admin_caller, ContextId::from(mine))
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::NotFound(_)), "get: {err:?}");

    // The owner keeps every act.
    restore(&pool, &alice_caller, mine)
        .await
        .expect("the owner restores");
    rename(&pool, &alice_caller, mine, "Journal")
        .await
        .expect("the owner renames");
    assert_eq!(
        owner_of_context(&pool, mine).await,
        ("kb_profiles".to_string(), *alice)
    );
}
