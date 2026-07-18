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
use temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiError;
use temper_services::services::access_service;
use temper_services::services::admin_ledger_service;
use temper_services::services::connection_service;
use temper_substrate::payloads::{AnchorTable, RefTarget};
use temper_workflow::operations::Surface;
use uuid::Uuid;

/// The admin event types this suite scans for. Mirrors the (private) `ADMIN_EVENT_TYPES` in
/// `admin_ledger_service` — a test-local copy because that const is not (and should not be) part of
/// the service's public API; a two-element drift here is caught by
/// `no_admin_payload_spells_a_trail_matched_key`, which asserts every one of these types.
const ADMIN_EVENT_TYPES_FOR_TEST: &[&str] =
    &["admin_ledger_opened", "grant_created", "grant_revoked"];

/// Keys the `element_trail_*` functions match on by shape, with **no** event-type filter. An admin
/// payload spelling any of these would leak an authority record into a cognition read gated only by
/// `resources_visible_to` (spec 2026-07-16 §5). Derived from the live function bodies:
/// `element_trail_node` matches `resource_id`, `owner.{table,id}`, `block_id`
/// (`migrations/20260706000002_element_trail_payload_actor.sql:32,35-36,39`); `element_trail_edge`
/// matches `edge_id` (`:14`). Subjects are spelled `subject_table`/`subject_id` and carried in
/// `references` instead.
const BANNED_ADMIN_PAYLOAD_KEYS: &[&str] = &["resource_id", "block_id", "edge_id", "owner"];

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

// ---------------------------------------------------------------------------------------------
// Task 3 (spec 2026-07-16 §5): the element_trail payload-key invariant.
//
// `element_trail_node`/`element_trail_edge` match purely on payload KEY SHAPE with NO event-type
// filter, gated only by `resources_visible_to`. An admin payload spelling `resource_id` — natural,
// since a grant whose subject is a resource *is about* that resource — would surface WHO was granted
// access to it to any reader of the resource. These land before any admin payload exists (Tasks 4/5)
// so the invariant is never retrofitted. Both are written to be able to FAIL: seeded via the
// canonical `seed_admin_event`, with positive controls, because this whole spec §5 exists because an
// earlier suite's tests *could not fail* on the defect they nominally guarded.
// ---------------------------------------------------------------------------------------------

/// The corpus invariant: no admin payload — across every admin event type — spells a key the
/// `element_trail_*` functions match on. Non-vacuous today because it scans REAL payloads written by
/// `seed_admin_event` (the canonical writer until Task 5's fire arm replaces it), for both subject
/// kinds an admin act can carry. Spell a banned key in that writer and this fails immediately.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn no_admin_payload_spells_a_trail_matched_key(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    // Both subject kinds, through the canonical writer, so the scan runs against real payloads
    // rather than an empty corpus (which would pass vacuously).
    seed_admin_event(
        &pool,
        f.admin_emitter,
        AnchorTable::Contexts,
        f.context_id,
        f.team_id,
    )
    .await;
    seed_admin_event(
        &pool,
        f.admin_emitter,
        AnchorTable::Resources,
        f.owned_resource_id,
        f.team_id,
    )
    .await;

    let bad: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT t.name, k.key
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
             CROSS JOIN LATERAL jsonb_object_keys(e.payload) AS k(key)
            WHERE t.name = ANY($1) AND k.key = ANY($2)"#,
    )
    .bind(ADMIN_EVENT_TYPES_FOR_TEST)
    .bind(BANNED_ADMIN_PAYLOAD_KEYS)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        bad.is_empty(),
        "admin payloads must not spell element_trail-matched keys — these leak authority records \
         to any reader of the resource. Use subject_table/subject_id + references. Offenders: {bad:?}"
    );
}

/// The end-to-end guard: an admin event that is *about a resource* must still never surface in that
/// resource's element trail. The event stays out for exactly one reason — its payload spells
/// `subject_table`/`subject_id`, never `resource_id`. Spell the banned key and this fails.
///
/// Non-vacuous by construction. The subject is `f.owned_resource_id` (a real `kb_resources` row the
/// owner can see — asserted below), and the seeded event is *about* it, so if the writer ever spelled
/// `resource_id`, `element_trail_node` would surface it to the owner and `leaked` would be > 0. A
/// context-subject event, by contrast, could never match a node trail and so could never fail — the
/// vacuity this test is deliberately shaped to avoid.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_event_never_appears_in_an_element_trail(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    // Positive control: the scanning profile genuinely sees the subject resource, so the trail scan
    // below actually covers it. Without this the `leaked == 0` assertion could hold vacuously.
    let owner_sees_it: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(f.owner_profile.uuid())
    .bind(f.owned_resource_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        owner_sees_it,
        "the owner must see their own resource, or the trail scan below is vacuous"
    );

    // An admin event ABOUT that resource — the payload most tempted to spell `resource_id`.
    seed_admin_event(
        &pool,
        f.admin_emitter,
        AnchorTable::Resources,
        f.owned_resource_id,
        f.team_id,
    )
    .await;

    // element_trail_node over every resource the owner can see must return no admin event.
    // NOTE its RETURNS TABLE spells the type column `kind`, not `event_type`
    // (migrations/20260706000002_element_trail_payload_actor.sql:27).
    let leaked: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
             FROM kb_resources r
             CROSS JOIN LATERAL element_trail_node($1, r.id) AS tr
            WHERE tr.kind = ANY($2)"#,
    )
    .bind(f.owner_profile.uuid())
    .bind(ADMIN_EVENT_TYPES_FOR_TEST)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        leaked, 0,
        "no admin event may surface in a cognition element trail"
    );
}

/// Task 4 (spec 2026-07-16 §8): the epoch marker exists after migration and is NULL-anchored (the
/// cognition firewall). `ledger_epoch` reads the `admin_ledger_opened` event's `opened_at`; a
/// producing anchor on it would mean the epoch had a cognition home, which it must not.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_epoch_is_readable_and_null_anchored(pool: PgPool) {
    let epoch = admin_ledger_service::ledger_epoch(&pool)
        .await
        .expect("ledger_epoch");
    assert!(
        epoch.is_some(),
        "the epoch marker must exist after migration"
    );

    let anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
          WHERE t.name='admin_ledger_opened' AND e.producing_anchor_table IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        anchored, 0,
        "the epoch must be NULL-anchored — it has no cognition home"
    );
}

// ---------------------------------------------------------------------------------------------
// Task 5 (spec 2026-07-16 §7): the grant chokepoint. insert_grant/delete_grant now fire the
// grant_created/grant_revoked event AND write the row in ONE txn, via the SQL fns _admin_grant_*.
// Proven for BOTH the generic grant_capability path and connection_service::grant_reach's direct
// insert_grant bypass — a service-layer sink would have missed the bypass; the SQL chokepoint cannot.
// ---------------------------------------------------------------------------------------------

fn grant_req(subject_id: Uuid, principal_id: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_contexts".into(),
        subject_id,
        principal_table: "kb_teams".into(),
        principal_id,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
    }
}

fn revoke_req(subject_id: Uuid, principal_id: Uuid) -> RevokeCapabilityRequest {
    RevokeCapabilityRequest {
        subject_table: "kb_contexts".into(),
        subject_id,
        principal_table: "kb_teams".into(),
        principal_id,
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn granting_writes_an_event_and_the_row(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    let outcome = access_service::grant_capability(
        &pool,
        f.admin_profile,
        &grant_req(f.context_id, f.team_id),
    )
    .await
    .expect("grant_capability");
    assert!(outcome.granted, "a fresh grant reports granted");

    // The row is written...
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants WHERE subject_id=$1 AND principal_id=$2",
    )
    .bind(f.context_id)
    .bind(f.team_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 1, "the grant row is written");

    // ...and the same txn put the act on the ledger, subject-addressable, banned-key-free.
    let entries = admin_ledger_service::list_by_subject(
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
    .unwrap();
    assert_eq!(entries.len(), 1, "the grant must be on the ledger");
    assert_eq!(entries[0].event_type, "grant_created");
    assert_eq!(entries[0].actor_profile_id, f.admin_profile.uuid());
    assert_eq!(entries[0].payload["subject_table"], "kb_contexts");
    assert!(
        entries[0].payload.get("resource_id").is_none(),
        "the banned element_trail key never appears (spec §5)"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revoking_writes_an_event_even_though_the_row_is_deleted(pool: PgPool) {
    let f = admin_fixture(&pool).await;
    access_service::grant_capability(&pool, f.admin_profile, &grant_req(f.context_id, f.team_id))
        .await
        .unwrap();
    let out = access_service::revoke_capability(
        &pool,
        f.admin_profile,
        &revoke_req(f.context_id, f.team_id),
    )
    .await
    .unwrap();
    assert!(out.revoked, "the row existed, so revoke reports revoked");

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_access_grants WHERE subject_id=$1")
        .bind(f.context_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 0,
        "revoke still hard-DELETEs the row — the row is the current-state projection"
    );

    // The ledger keeps BOTH acts even though the row is gone — the temporal record outlives the
    // projection. That asymmetry is the whole point of the sink.
    let entries = admin_ledger_service::list_by_subject(
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
    .unwrap();
    assert_eq!(
        entries.len(),
        2,
        "the ledger keeps both the grant and the revoke"
    );
    assert_eq!(entries[0].event_type, "grant_revoked", "newest first");
    assert_eq!(entries[1].event_type, "grant_created");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_connection_grant_reach_bypass_is_also_on_the_ledger(pool: PgPool) {
    // connection_service::grant_reach calls access_service::insert_grant DIRECTLY, bypassing
    // grant_capability. Because the event now lives INSIDE insert_grant (the SQL chokepoint), the
    // bypass cannot escape the ledger — this is exactly why the sink is SQL-resident, not service-layer.
    let f = admin_fixture(&pool).await;
    let nonce = &Uuid::now_v7().simple().to_string()[..8];
    let (conn_profile, conn_emitter) = profile_with_emitters(&pool, &format!("conn-{nonce}")).await;
    let connection_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_connections \
             (provider, slug, name, registered_by_profile_id, profile_id, emitter_entity_id, \
              home_context_id, owner_team_id) \
         VALUES ('test', $1, $1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(format!("conn-{nonce}"))
    .bind(f.admin_profile.uuid())
    .bind(conn_profile.uuid())
    .bind(conn_emitter)
    .bind(f.context_id)
    .bind(f.team_id)
    .fetch_one(&pool)
    .await
    .expect("seed connection");

    connection_service::grant_reach(&pool, f.admin_profile, connection_id, f.team_id, None)
        .await
        .expect("grant_reach");

    let entries = admin_ledger_service::list_by_subject(
        &pool,
        f.admin_profile,
        RefTarget {
            kind: AnchorTable::Connections,
            id: connection_id,
        },
        50,
        0,
    )
    .await
    .unwrap();
    assert_eq!(
        entries.len(),
        1,
        "grant_reach's direct-insert_grant bypass must still reach the ledger"
    );
    assert_eq!(entries[0].event_type, "grant_created");
    assert_eq!(entries[0].payload["subject_table"], "kb_connections");
}

/// THE SECURITY-RELEVANT BRANCH the other tests miss: a capability CHANGE on an existing grant is an
/// `ON CONFLICT` UPDATE (`inserted = false`), and the event MUST still fire — keying emission on the
/// `inserted` bool alone would silently drop a privilege escalation from the append-only ledger while
/// it lands in the current-state row (`_admin_grant_created` warns about exactly this). Every other
/// grant test does a FRESH grant (`inserted = true`), so the drop-on-update regression ships green
/// without this. Also the only test that asserts the `previous` payload key — the whole reason for
/// the pre-upsert capability capture.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_capability_change_still_writes_an_event_carrying_previous(pool: PgPool) {
    let f = admin_fixture(&pool).await;

    // Fresh grant: read-only. inserted = true.
    let first = access_service::grant_capability(
        &pool,
        f.admin_profile,
        &GrantCapabilityRequest {
            subject_table: "kb_contexts".into(),
            subject_id: f.context_id,
            principal_table: "kb_teams".into(),
            principal_id: f.team_id,
            can_read: true,
            can_write: false,
            can_delete: false,
            can_grant: false,
        },
    )
    .await
    .unwrap();
    assert!(
        first.granted,
        "the fresh grant reports granted=true (inserted)"
    );

    // Re-grant the SAME (subject, principal) with an escalated capability (add write). This is an
    // ON CONFLICT UPDATE, so `granted` is false — but the act is a real privilege escalation and
    // MUST still reach the ledger.
    let second = access_service::grant_capability(
        &pool,
        f.admin_profile,
        &GrantCapabilityRequest {
            subject_table: "kb_contexts".into(),
            subject_id: f.context_id,
            principal_table: "kb_teams".into(),
            principal_id: f.team_id,
            can_read: true,
            can_write: true,
            can_delete: false,
            can_grant: false,
        },
    )
    .await
    .unwrap();
    assert!(
        !second.granted,
        "an upsert that only CHANGES capabilities reports granted=false — and this is the branch \
         that must not drop the event"
    );

    let entries = admin_ledger_service::list_by_subject(
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
    .unwrap();
    assert_eq!(
        entries.len(),
        2,
        "the capability change fires a SECOND grant_created — NOT dropped because inserted=false"
    );
    assert_eq!(entries[0].event_type, "grant_created", "newest first");
    // The newest event carries `previous` = the caps BEFORE the change (read-only), and the new caps.
    assert_eq!(
        entries[0].payload["can_write"], true,
        "the new event carries the escalated capability"
    );
    assert_eq!(
        entries[0].payload["previous"]["can_write"], false,
        "`previous` holds the pre-change capabilities — how a consumer sees WHAT changed"
    );
    assert!(
        entries[1].payload.get("previous").is_none(),
        "the fresh grant carries no `previous`"
    );
}
