#![cfg(feature = "test-db")]

//! The admin ledger's read surface (spec 2026-07-16 §5, §11.1b).
//!
//! Admin events ride `kb_events` with a **both-NULL producing anchor** — the cognition firewall.
//! That firewall hides them from every anchor-scoped reader, so the read path is
//! `kb_events."references"`: typed provenance pointers, GIN-indexed, consulted by no cognition
//! reader.
//!
//! Two axes, gated differently, and the difference is the point:
//!   - **by subject** — the read gate MIRRORS the write gate, dispatched per event type. For
//!     grants that is `access_service::can_administer_grant`: `is_system_admin` OR
//!     `can(caller,'grant',subject)`. A resource's owner satisfies the second arm with no team and
//!     no admin, and must be able to read the record of an act they could perform.
//!   - **by actor** — SELF-GATING. You keep the record of what you did, conditioned only on still
//!     having system access. Losing a capability, a role, or ownership of a subject does not take
//!     your own authorship from you.
//!
//! The fixture is inline: `crates/temper-services/tests/` has no `common/` module — every test
//! there declares its own (see `context_read_predicate_test.rs`, `act_correlation_test.rs`).

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiError;
use temper_services::services::access_service;
use temper_services::services::admin_ledger_service;
use temper_substrate::payloads::{AnchorTable, RefTarget};
use temper_workflow::operations::Surface;
use uuid::Uuid;

/// A real system-admin, an outsider, and a non-admin who nonetheless owns a resource (and so can
/// administer grants on it — the middle case §5 was rewritten to protect).
struct AdminFixture {
    /// Owner of the gating team ⇒ `is_system_admin` is TRUE. Asserted, not assumed.
    admin_profile: ProfileId,
    admin_emitter: Uuid,
    /// Belongs to nothing, owns nothing.
    outsider_profile: ProfileId,
    /// NOT an admin, but owns `owned_resource_id` ⇒ `can(…,'grant',…)` is true for it.
    owner_profile: ProfileId,
    owner_emitter: Uuid,
    /// Homed on `owner_profile`'s OWN context (`owner_table='kb_profiles'`) — the shape that has
    /// no owning team, and that refuted the originally-proposed team-shaped read gate.
    owned_resource_id: Uuid,
    team_id: Uuid,
    cogmap_id: Uuid,
    /// Owned by `team_id`, and reachable from `cogmap_id` via `kb_team_cogmaps` — so the firewall
    /// test's steward window genuinely covers it (see the positive control there).
    context_id: Uuid,
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed team")
}

/// A profile **with its per-surface emitter entities**, which is the whole point.
///
/// `resolve_emitter` is a `fetch_one` with no lazy creation, so a profile without
/// `<handle>@<surface>` entities cannot emit — a fixture that skips them passes while production
/// 500s (the live bug in task `019f6b06-c48f-7a81-a238-cdd6b131f3dc`).
///
/// The emitter loop is driven off `Surface::ALL` — the same driver
/// `profile_service::provision_profile_entities` uses — rather than a hardcoded surface list, so
/// a new surface variant cannot silently drift this fixture away from production. (That fn is
/// `pub(crate)`, hence unreachable from this integration test, and it does not create the profile
/// row either; `Surface::ALL` is the reachable half of its contract.)
async fn profile_with_emitters(pool: &PgPool, handle: &str) -> (ProfileId, Uuid) {
    let profile_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("seed profile");

    let mut api_emitter = None;
    for surface in Surface::ALL {
        let entity_id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name, metadata) \
             VALUES ($1, $2, '{}'::jsonb) RETURNING id",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{}", surface.marker()))
        .fetch_one(pool)
        .await
        .expect("seed emitter entity");
        // `ApiHttp` is the surface whose marker is `web` — the emitter these tests author through.
        if surface == Surface::ApiHttp {
            api_emitter = Some(entity_id);
        }
    }

    (
        ProfileId::from(profile_id),
        api_emitter.expect("Surface::ALL must contain ApiHttp"),
    )
}

async fn context_owned_by(pool: &PgPool, owner_table: &str, owner_id: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), $1, $2, $3, $3) RETURNING id",
    )
    .bind(owner_table)
    .bind(owner_id)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed context")
}

/// A resource homed on `context`, owned by `owner`. The `owner_profile_id` is what
/// `derived_access_profile`'s grant arm reads:
/// `EXISTS (SELECT 1 FROM kb_resource_homes WHERE resource_id = … AND owner_profile_id = p_profile)`.
async fn resource_owned_by(pool: &PgPool, context: Uuid, owner: ProfileId, title: &str) -> Uuid {
    let resource_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) \
         VALUES (uuid_generate_v7(), $1, $2) RETURNING id",
    )
    .bind(title)
    .bind(format!("temper://test/{title}"))
    .fetch_one(pool)
    .await
    .expect("seed resource");

    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource_id)
    .bind(context)
    .bind(owner.uuid())
    .execute(pool)
    .await
    .expect("seed resource home");

    resource_id
}

/// A system-admin with emitters, an outsider, a non-admin resource owner, a team, a cogmap, and a
/// context to grant on.
///
/// **`admin_profile` is not an admin because a column says so.** `is_system_admin(p)` is *owner of
/// the gating team* — it reads `kb_team_members`, joined to the team whose slug is
/// `kb_system_settings.gating_team_slug`. It never looks at `kb_profiles.system_access` (verified
/// live: the `system` profile has `system_access='admin'` and `is_system_admin()=f`). So this
/// fixture creates the team, points `gating_team_slug` at it — the column is EMPTY out of the box,
/// which means *nobody* is admin — and adds `admin_profile` as `owner` (`member` is not enough).
///
/// Both halves are asserted below rather than trusted: a fixture whose admin is not an admin makes
/// `a_non_admin_cannot_read_the_ledger` pass for the wrong reason (everyone is a non-admin).
async fn admin_fixture(pool: &PgPool) -> AdminFixture {
    let nonce = &Uuid::now_v7().simple().to_string()[..8];

    let team_slug = format!("gating-{nonce}");
    let team_id = team(pool, &team_slug).await;

    // The gating team is what MAKES an admin. Empty by default ⇒ nobody is admin.
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = $1 WHERE id = 1")
        .bind(&team_slug)
        .execute(pool)
        .await
        .expect("point gating_team_slug at the fixture team");

    let (admin_profile, admin_emitter) =
        profile_with_emitters(pool, &format!("admin-{nonce}")).await;
    // `owner`, not `member`: is_system_admin requires role = 'owner'.
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner'::team_role)",
    )
    .bind(team_id)
    .bind(admin_profile.uuid())
    .execute(pool)
    .await
    .expect("make admin_profile an OWNER of the gating team");

    let (outsider_profile, _) = profile_with_emitters(pool, &format!("outsider-{nonce}")).await;
    let (owner_profile, owner_emitter) =
        profile_with_emitters(pool, &format!("owner-{nonce}")).await;

    // Team-owned: this is the context the steward window covers (via kb_team_cogmaps below), and
    // the one `owner_profile` has NO grant capability over.
    let context_id = context_owned_by(pool, "kb_teams", team_id, "team-ctx").await;

    // `owner_profile`'s own context — owner_table='kb_profiles', so it has no owning team. That is
    // exactly the shape §5's refutation turned on.
    let owner_context =
        context_owned_by(pool, "kb_profiles", owner_profile.uuid(), "default").await;
    let owned_resource_id =
        resource_owned_by(pool, owner_context, owner_profile, "owned-doc").await;

    // A cogmap joined to the team, so `steward_team_contexts(cogmap)` reaches `context_id`.
    let telos = resource_owned_by(pool, context_id, admin_profile, "telos").await;
    let cogmap_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (id, name, telos_resource_id) \
         VALUES (uuid_generate_v7(), $1, $2) RETURNING id",
    )
    .bind(format!("map-{nonce}"))
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("seed cogmap");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap_id)
        .bind(team_id)
        .execute(pool)
        .await
        .expect("join cogmap to team");

    // Assert the fixture is what it claims. Without this every admin assertion is vacuous.
    assert!(
        access_service::is_system_admin(pool, admin_profile)
            .await
            .unwrap(),
        "fixture admin_profile MUST be a real system admin (owner of the gating team)"
    );
    assert!(
        !access_service::is_system_admin(pool, outsider_profile)
            .await
            .unwrap(),
        "fixture outsider_profile must NOT be a system admin"
    );

    AdminFixture {
        admin_profile,
        admin_emitter,
        outsider_profile,
        owner_profile,
        owner_emitter,
        owned_resource_id,
        team_id,
        cogmap_id,
        context_id,
    }
}

/// Insert a NULL-anchored admin event by hand. Task 5 replaces this with a real fire arm; until
/// then the read surface must be provable against a crafted row.
///
/// The payload spells `subject_table`/`subject_id` — never `resource_id`/`block_id`/`edge_id`/
/// `owner`, which `element_trail_*` match on by key shape with no type filter (spec §5's ban;
/// Task 3 makes it a tested invariant).
async fn seed_admin_event(
    pool: &PgPool,
    emitter: Uuid,
    subject_kind: AnchorTable,
    subject: Uuid,
    principal: Uuid,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO kb_events
               (event_type_id, emitter_entity_id, payload, "references")
           SELECT et.id, $1,
                  jsonb_build_object('subject_table', $4::text, 'subject_id', $2::text),
                  jsonb_build_array(
                    jsonb_build_object('rel','subject',  'target', jsonb_build_object('kind',$4::text,'id',$2)),
                    jsonb_build_object('rel','principal','target', jsonb_build_object('kind','kb_teams','id',$3))
                  )
             FROM kb_event_types et WHERE et.name = 'grant_created'
           RETURNING id"#,
    )
    .bind(emitter)
    .bind(subject)
    .bind(principal)
    .bind(subject_kind.as_str())
    .fetch_one(pool)
    .await
    .expect("seed admin event")
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn list_by_subject_finds_the_admin_event(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    let ev = seed_admin_event(
        &pool,
        f.admin_emitter,
        AnchorTable::Contexts,
        f.context_id,
        f.team_id,
    )
    .await;

    let got = admin_ledger_service::list_by_subject(
        &pool,
        f.admin_profile,
        RefTarget {
            kind: AnchorTable::Contexts,
            id: f.context_id,
        },
        50,
        0,
    )
    .await
    .expect("list_by_subject");

    assert_eq!(
        got.len(),
        1,
        "the seeded grant_created must be found by its subject reference"
    );
    assert_eq!(got[0].event_id, ev);
    assert_eq!(got[0].event_type, "grant_created");
    assert_eq!(got[0].actor_profile_id, f.admin_profile.uuid());
}

/// The firewall — the design's load-bearing claim. A NULL-anchored event must not be counted by
/// the steward's ingest delta.
///
/// **The positive control is not decoration.** `steward_ingest_delta` windows on
/// `producing_anchor_id IN (SELECT context_id FROM steward_team_contexts(p_cogmap))`, so if the
/// fixture's cogmap did not actually reach `context_id`, the window would be empty and
/// `new_events = 0` would hold no matter what the firewall did. Seeding an ANCHORED event on the
/// same context and asserting it IS counted proves the window covers this context — which is what
/// makes the admin event's absence from it mean something.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_admin_event_is_invisible_to_cognition(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    seed_admin_event(
        &pool,
        f.admin_emitter,
        AnchorTable::Contexts,
        f.context_id,
        f.team_id,
    )
    .await;

    // The positive control: one ordinary anchored event on the same context.
    sqlx::query(
        r#"INSERT INTO kb_events
               (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, payload)
           SELECT et.id, $1, 'kb_contexts', $2, '{}'::jsonb
             FROM kb_event_types et WHERE et.name = 'resource_created'"#,
    )
    .bind(f.admin_emitter)
    .bind(f.context_id)
    .execute(&pool)
    .await
    .expect("seed anchored cognition event");

    // NOTE steward_ingest_delta(p_cogmap, p_watermark) takes a COGMAP, not a team
    // (migrations/20260701000005_steward_ingest_watermark.sql:40).
    let new_events: i64 =
        sqlx::query_scalar("SELECT new_events FROM steward_ingest_delta($1, NULL)")
            .bind(f.cogmap_id)
            .fetch_one(&pool)
            .await
            .expect("steward_ingest_delta");

    assert_eq!(
        new_events, 1,
        "the steward delta must count the ANCHORED event and ONLY it — a count of 0 would mean \
         the window does not reach this context (making the firewall assertion vacuous); a count \
         of 2 would mean the NULL-anchored admin event reached cognition"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_non_admin_cannot_read_the_ledger(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    seed_admin_event(
        &pool,
        f.admin_emitter,
        AnchorTable::Contexts,
        f.context_id,
        f.team_id,
    )
    .await;

    let err = admin_ledger_service::list_by_subject(
        &pool,
        f.outsider_profile,
        RefTarget {
            kind: AnchorTable::Contexts,
            id: f.context_id,
        },
        50,
        0,
    )
    .await
    .expect_err("an outsider must not read the admin ledger");

    assert!(
        matches!(err, ApiError::NotFound),
        "reads deny with 404, not 403 (the deny-split invariant); got {err:?}"
    );
}

/// THE TEST THIS SUITE WAS MISSING. The three tests above are all satisfied by an
/// `is_system_admin`-only gate — admin reads, outsider is denied, neither exercises the middle.
/// But the middle **is** §5's entire correction: a non-admin who could WRITE the grant must be able
/// to READ the record of it. Without this test the refuted gate passes green.
///
/// `derived_access_profile` gives a resource's owner `can(…,'grant',…)` on it — derived, no
/// explicit grant, no team, no admin. Probed live in §5: `is_sysadmin=f, can_write_the_grant=t`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_grant_writer_can_read_their_own_grant_record(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    // Assert BOTH halves first — if the fixture silently made them an admin, or silently failed to
    // give them the capability, this test would pass while proving nothing.
    assert!(
        !access_service::is_system_admin(&pool, f.owner_profile)
            .await
            .unwrap(),
        "the grant writer must NOT be an admin, or this test proves nothing"
    );
    let can_write_the_grant: bool =
        sqlx::query_scalar("SELECT can('kb_profiles', $1, 'grant', 'kb_resources', $2)")
            .bind(f.owner_profile.uuid())
            .bind(f.owned_resource_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        can_write_the_grant,
        "the fixture's owner must satisfy can(…,'grant',…) on their own resource — that is the \
         write gate this read gate mirrors"
    );

    seed_admin_event(
        &pool,
        f.admin_emitter,
        AnchorTable::Resources,
        f.owned_resource_id,
        f.team_id,
    )
    .await;

    let got = admin_ledger_service::list_by_subject(
        &pool,
        f.owner_profile,
        RefTarget {
            kind: AnchorTable::Resources,
            id: f.owned_resource_id,
        },
        50,
        0,
    )
    .await
    .expect("the actor who could write this grant must be able to read its record");

    assert_eq!(
        got.len(),
        1,
        "the grant writer sees the record of the act they could perform"
    );
}

/// §11.1b, decided 2026-07-16: the actor axis is SELF-GATING. This is the test that distinguishes
/// it from a subject-gated axis — the actor authored the act but cannot administer its subject, so
/// under subject-gating they would lose sight of their own authorship. Which is the exact defect
/// this whole spec exists to undo.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_actor_keeps_their_own_history_without_the_capability(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    // An act authored BY the owner, ON a subject the owner cannot administer (the team's context,
    // not theirs). Authorship and capability deliberately pulled apart.
    seed_admin_event(
        &pool,
        f.owner_emitter,
        AnchorTable::Contexts,
        f.context_id,
        f.team_id,
    )
    .await;

    // The subject axis denies them — they have no can_grant on this subject. This half must hold
    // or the test is not exercising the distinction.
    let subject_err = admin_ledger_service::list_by_subject(
        &pool,
        f.owner_profile,
        RefTarget {
            kind: AnchorTable::Contexts,
            id: f.context_id,
        },
        50,
        0,
    )
    .await
    .expect_err("no capability on this subject ⇒ the subject axis denies");
    assert!(matches!(subject_err, ApiError::NotFound));

    // The actor axis returns it anyway. That is the decision.
    let mine = admin_ledger_service::list_by_actor(&pool, f.owner_profile, f.owner_profile, 50, 0)
        .await
        .expect("an actor always reads their own acts");

    assert_eq!(
        mine.len(),
        1,
        "authorship survives the loss of capability over the subject"
    );
    assert_eq!(mine[0].actor_profile_id, f.owner_profile.uuid());
}

/// The other half of the decision: reading SOMEONE ELSE's history is an audit, and audits are
/// admin-only. Self-gating widens the actor's own view; it must not widen anyone else's.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reading_another_actors_history_is_admin_only(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    seed_admin_event(
        &pool,
        f.admin_emitter,
        AnchorTable::Contexts,
        f.context_id,
        f.team_id,
    )
    .await;

    let err =
        admin_ledger_service::list_by_actor(&pool, f.outsider_profile, f.admin_profile, 50, 0)
            .await
            .expect_err("a non-admin must not audit another profile's acts");
    assert!(matches!(err, ApiError::NotFound));

    // ...and the admin may.
    let audit = admin_ledger_service::list_by_actor(&pool, f.admin_profile, f.admin_profile, 50, 0)
        .await
        .expect("an admin audits");
    assert_eq!(audit.len(), 1);
}

/// §11.1b's **"unless"**: self-gating is conditioned on still having system access, and on nothing
/// else. Lose a capability, a role, or ownership of a subject and you keep your history; lose the
/// front door and you keep nothing, because you are no longer a reader at all.
///
/// This is the one guard on the widening the self-gate decision made, and without this test it is
/// **unexercised**: `#[sqlx::test]` databases are born `access_mode = 'open'`, where
/// `has_system_access` short-circuits `true` for everyone, so `list_by_actor`'s front-door branch
/// never runs. A test suite that cannot fail on a gate is not testing the gate.
///
/// Differential by construction — the SAME call, before and after the mode flip. The `before` half
/// is what makes the `after` half mean something: it proves the 404 came from losing the front
/// door, not from the fixture never having worked.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn losing_system_access_takes_your_own_history_with_it(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    // An act authored BY the owner. Under the self-gate they read it regardless of capability.
    seed_admin_event(
        &pool,
        f.owner_emitter,
        AnchorTable::Contexts,
        f.context_id,
        f.team_id,
    )
    .await;

    // BEFORE: access_mode='open' ⇒ has_system_access is true for everyone ⇒ the actor reads.
    let before =
        admin_ledger_service::list_by_actor(&pool, f.owner_profile, f.owner_profile, 50, 0)
            .await
            .expect("with system access, the actor reads their own history");
    assert_eq!(before.len(), 1, "the self-gate returns the actor's own act");

    // Take the front door away. The fixture already points gating_team_slug at its team, and
    // owner_profile is not a member of it — so invite_only mode revokes their system access
    // without touching a single capability, role, or ownership relation.
    sqlx::query("UPDATE kb_system_settings SET access_mode = 'invite_only' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("flip to invite_only");

    assert!(
        !access_service::has_system_access(&pool, f.owner_profile)
            .await
            .expect("has_system_access"),
        "the fixture owner must be outside the gating team, or this test proves nothing"
    );

    // AFTER: same call, same authorship, same everything else.
    let err = admin_ledger_service::list_by_actor(&pool, f.owner_profile, f.owner_profile, 50, 0)
        .await
        .expect_err("without system access there is no reader, so there is no history");
    assert!(
        matches!(err, ApiError::NotFound),
        "reads deny with 404, not 403 (the deny-split invariant); got {err:?}"
    );

    // The admin is an owner OF the gating team, so invite_only does not touch them — proving the
    // flip revoked one profile's access rather than simply breaking the surface for everyone.
    let admin_still_reads =
        admin_ledger_service::list_by_actor(&pool, f.admin_profile, f.owner_profile, 50, 0)
            .await
            .expect("the gating team's owner keeps system access under invite_only");
    assert_eq!(admin_still_reads.len(), 1);
}
