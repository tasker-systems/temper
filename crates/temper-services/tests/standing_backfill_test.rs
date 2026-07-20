#![cfg(feature = "test-db")]
//! Differential backfill test (spec §12).
//!
//! The claim under test is NOT `old(p) == new(p) ∀p` — that is unsatisfiable on any population
//! containing a deactivated profile, by §11's deliberate flip. It is:
//!   * `old(p) == new(p)` ∀p WHERE is_active   — preservation, scoped to where it is true
//!   * deactivated profiles flip true → false  — the intended change, pinned not tolerated
//!   * connection profiles get no row          — D7 / rule 0
//!   * pending requests land in `requested`    — the pass the single rule cannot express
//!
//! The governance pass (PASS 3 of migration 20260720000120) and its `gating_team_slug <> ''` guard
//! are covered separately at the end of this file — the differential harness here re-runs only the
//! standing passes, matching §12's scope (the predicate), so the governance pass needs its own.

use sqlx::PgPool;

/// The PRE-CUTOVER predicate, transcribed from 20260624000002_canonical_functions.sql:1388.
/// Inlined because 20260720000110 has already replaced the real function's body by the time any
/// `#[sqlx::test]` body runs.
const OLD_PREDICATE: &str = r#"
    WITH settings AS (SELECT access_mode, gating_team_slug FROM kb_system_settings LIMIT 1)
    SELECT CASE
        WHEN settings.access_mode = 'open' THEN true
        WHEN settings.access_mode = 'invite_only' THEN EXISTS (
            SELECT 1 FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id
             WHERE tm.profile_id = $1 AND t.slug = settings.gating_team_slug)
        ELSE false
    END FROM settings
"#;

struct Population {
    /// (handle, id, is_active, is_connection, has_pending_request)
    rows: Vec<(String, uuid::Uuid, bool, bool, bool)>,
}

/// One representative per tier ONLY — `system_access` is not a dimension of the predicate (it
/// appears nowhere in has_system_access's body), so fanning across all three tiers would triple
/// the population and test nothing about admission. The tiers are here purely to exercise
/// trg_sync_system_membership, which does read the column.
async fn build_population(pool: &PgPool) -> Population {
    let mut rows = Vec::new();

    let team: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) VALUES ('temper-system','System','watcher')
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    for (handle, tier, active, member, connection, pending) in [
        ("p-none-out", "none", true, false, false, false),
        ("p-none-in", "none", true, true, false, false), // the `anonymous` shape (D8)
        ("p-approved-in", "approved", true, true, false, false),
        ("p-admin-in", "admin", true, true, false, false),
        ("p-inactive-in", "approved", false, true, false, false), // the deliberate flip
        ("p-inactive-out", "none", false, false, false, false),
        ("p-pending", "none", true, false, false, true), // pass 2
        ("p-connection", "none", true, false, true, false), // rule 0
    ] {
        let id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name, system_access, is_active)
             VALUES ($1,$1,$2::system_access,$3) RETURNING id",
        )
        .bind(handle)
        .bind(tier)
        .bind(active)
        .fetch_one(pool)
        .await
        .unwrap();

        if member {
            sqlx::query(
                "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'watcher')
                 ON CONFLICT DO NOTHING",
            )
            .bind(team)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        }
        if connection {
            // A connection profile needs a real kb_connections row (rule 0 detects it by
            // profile_id), which requires a non-null emitter entity and home context. A directly
            // inserted profile has neither and a fresh test DB has zero contexts, so both are
            // created here rather than joined from seed data. `.unwrap()`, not `.ok()`: if this
            // fails, p-connection would silently become a non-connection and the rule-0 assertion
            // would test a fiction. Entity/context shapes borrowed from admin_ledger_test.rs.
            let entity: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO kb_entities (profile_id, name, metadata)
                 VALUES ($1,$2,'{}'::jsonb) RETURNING id",
            )
            .bind(id)
            .bind(handle)
            .fetch_one(pool)
            .await
            .unwrap();
            let context: uuid::Uuid = sqlx::query_scalar(
                "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
                 VALUES (uuid_generate_v7(),'kb_profiles',$1,$2,$2) RETURNING id",
            )
            .bind(id)
            .bind(handle)
            .fetch_one(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO kb_connections
                    (provider, slug, name, registered_by_profile_id, profile_id,
                     emitter_entity_id, home_context_id)
                 VALUES ('test',$1,$1,$2,$2,$3,$4)",
            )
            .bind(handle)
            .bind(id)
            .bind(entity)
            .bind(context)
            .execute(pool)
            .await
            .unwrap();
        }
        if pending {
            sqlx::query(
                "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, source)
                 VALUES (uuid_generate_v7(), $1, $2, 'pending', 'cli')",
            )
            .bind(team)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        }

        rows.push((handle.to_string(), id, active, connection, pending));
    }

    Population { rows }
}

async fn old_access(pool: &PgPool, id: uuid::Uuid) -> Option<bool> {
    // `FROM settings` yields ZERO rows when kb_system_settings is empty (the `settings-empty`
    // config), where the real SQL function would return NULL. fetch_optional + flatten models that
    // as None rather than erroring on the missing row.
    sqlx::query_scalar::<_, Option<bool>>(OLD_PREDICATE)
        .bind(id)
        .fetch_optional(pool)
        .await
        .unwrap()
        .flatten()
}

async fn new_access(pool: &PgPool, id: uuid::Uuid) -> Option<bool> {
    sqlx::query_scalar("SELECT has_system_access($1)")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Re-run the backfill's rules against the freshly built population. Mirrors
/// migrations/20260720000120 exactly; if the two drift, this test is testing a fiction.
async fn run_backfill(pool: &PgPool) {
    sqlx::query(
        r#"
        INSERT INTO kb_principal_standing (profile_id, state)
        SELECT p.id,
               CASE WHEN p.is_active = false THEN 'deactivated'
                    WHEN (WITH settings AS (SELECT access_mode, gating_team_slug FROM kb_system_settings LIMIT 1)
                          SELECT CASE
                              WHEN s.access_mode = 'open' THEN true
                              WHEN s.access_mode = 'invite_only' THEN EXISTS (
                                  SELECT 1 FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id
                                   WHERE tm.profile_id = p.id AND t.slug = s.gating_team_slug)
                              ELSE false END FROM settings s) IS TRUE THEN 'approved'
                    ELSE 'denied' END
          FROM kb_profiles p
         WHERE NOT EXISTS (SELECT 1 FROM kb_connections c WHERE c.profile_id = p.id)
        ON CONFLICT (profile_id) DO NOTHING
    "#,
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "UPDATE kb_principal_standing s SET state = 'requested'
           FROM kb_join_requests jr
          WHERE jr.requesting_profile_id = s.profile_id AND jr.status = 'pending' AND s.state = 'denied'",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn configure(pool: &PgPool, mode: Option<&str>, gating: Option<&str>) {
    match mode {
        None => {
            sqlx::query("DELETE FROM kb_system_settings")
                .execute(pool)
                .await
                .unwrap();
        }
        Some(m) => {
            sqlx::query(
                "INSERT INTO kb_system_settings (id, access_mode, gating_team_slug) VALUES (1,$1,$2)
                 ON CONFLICT (id) DO UPDATE SET access_mode=EXCLUDED.access_mode,
                                                gating_team_slug=EXCLUDED.gating_team_slug",
            )
            .bind(m)
            .bind(gating)
            .execute(pool)
            .await
            .unwrap();
        }
    }
}

async fn differential(pool: &PgPool, mode: Option<&str>, gating: Option<&str>, label: &str) {
    configure(pool, mode, gating).await;
    let pop = build_population(pool).await;

    // Capture the old verdict BEFORE writing any standing.
    let mut before = Vec::new();
    for (h, id, active, conn, pending) in &pop.rows {
        before.push((
            h.clone(),
            *id,
            *active,
            *conn,
            *pending,
            old_access(pool, *id).await,
        ));
    }

    sqlx::query("DELETE FROM kb_principal_standing")
        .execute(pool)
        .await
        .unwrap();
    run_backfill(pool).await;

    for (h, id, active, conn, pending, old) in before {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
                .bind(id)
                .fetch_optional(pool)
                .await
                .unwrap()
                .flatten();

        if conn {
            // D7 / rule 0. Under `open` the old predicate is true for EVERY profile, so a literal
            // per-profile backfill would mint this row and dissolve D7's structural safety.
            assert!(
                state.is_none(),
                "[{label}] {h}: a connection profile must get NO standing row"
            );
            continue;
        }

        if !active {
            // The DELIBERATE flip. Pinned, not merely tolerated.
            assert_eq!(state.as_deref(), Some("deactivated"), "[{label}] {h}");
            assert_eq!(
                new_access(pool, id).await,
                Some(false),
                "[{label}] {h}: a deactivated principal's predicate flips true→false by design (§11)"
            );
            continue;
        }

        if pending {
            // PASS 2 rescues a pending request ONLY from denial (`WHERE s.state = 'denied'`). Under
            // a config where the old predicate already grants access (e.g. `open`), the principal
            // backfills to `approved` and the pending request is moot — marking them `requested`
            // would DOWNGRADE an approved principal. So the claim is scoped to where denial is what
            // the pass rescues from: requested iff the old predicate did not already approve.
            let expected = if old.unwrap_or(false) {
                "approved"
            } else {
                "requested"
            };
            assert_eq!(
                state.as_deref(),
                Some(expected),
                "[{label}] {h}: a pending request must land in `{expected}` — PASS 2 promotes a \
                 `denied` pending principal to `requested`, but never downgrades an approved one"
            );
            continue;
        }

        // THE PRESERVATION CLAIM, scoped to where it is true.
        assert_eq!(
            new_access(pool, id).await,
            Some(old.unwrap_or(false)),
            "[{label}] {h}: old(p) must equal new(p) for every active, non-connection principal"
        );
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_with_gating_set(pool: PgPool) {
    differential(&pool, Some("open"), Some("temper-system"), "open/gating").await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn invite_only_with_gating_set(pool: PgPool) {
    differential(
        &pool,
        Some("invite_only"),
        Some("temper-system"),
        "invite_only/gating",
    )
    .await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn invite_only_with_gating_null(pool: PgPool) {
    differential(&pool, Some("invite_only"), None, "invite_only/NULL").await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn open_with_gating_null(pool: PgPool) {
    differential(&pool, Some("open"), None, "open/NULL").await;
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn settings_empty_is_the_null_arm(pool: PgPool) {
    // §12: "this one fails against today's function, which is the point." The old predicate
    // returns NULL with no settings row; rule 3's `IS TRUE ... ELSE denied` handles it BY DECISION
    // rather than by omission, and the repointed predicate is EXISTS-total so it returns false.
    differential(&pool, None, None, "settings-empty").await;

    let any: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT profile_id FROM kb_principal_standing LIMIT 1")
            .fetch_optional(&pool)
            .await
            .unwrap()
            .flatten();
    if let Some(id) = any {
        assert_eq!(
            new_access(&pool, id).await,
            Some(false),
            "with no settings row the predicate must be FALSE, never NULL"
        );
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_anonymous_shape_survives_the_backfill(pool: PgPool) {
    // D8, vindicated empirically on prod at exactly the predicted cardinality of one: `anonymous`
    // has tier `none` and access purely via gating-team membership. A tier-based backfill would
    // have locked it out. `p-none-in` is that shape.
    configure(&pool, Some("invite_only"), Some("temper-system")).await;
    build_population(&pool).await;
    sqlx::query("DELETE FROM kb_principal_standing")
        .execute(&pool)
        .await
        .unwrap();
    run_backfill(&pool).await;

    let state: String = sqlx::query_scalar(
        "SELECT s.state FROM kb_principal_standing s JOIN kb_profiles p ON p.id = s.profile_id
          WHERE p.handle = 'p-none-in'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        state, "approved",
        "a tier-`none` gating-team member must backfill to approved — this is the D8 case"
    );
}

// ---------------------------------------------------------------------------------------------
// PASS 3 — governance. Not part of §12's differential scope, but the governance pass is the most
// consequential in the migration (getting it wrong locks the operator out), and its
// `gating_team_slug <> ''` guard is a fix this beat added over the plan. Both deserve a durable
// test, not a one-time BEGIN/ROLLBACK probe.
// ---------------------------------------------------------------------------------------------

/// PASS 3 of migration 20260720000120, verbatim. Mirrors the migration; if the two drift, this is
/// testing a fiction.
async fn run_governance_backfill(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO kb_principal_governance (profile_id, granted_by)
         SELECT tm.profile_id, NULL
           FROM kb_team_members tm
           JOIN kb_teams t ON t.id = tm.team_id
           JOIN kb_system_settings st ON st.gating_team_slug = t.slug
          WHERE tm.role = 'owner' AND st.gating_team_slug <> ''
         ON CONFLICT (profile_id) DO NOTHING",
    )
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn governance_backfill_preserves_gating_team_owners(pool: PgPool) {
    // An existing admin under the old definition IS a gating-team owner. The governance pass must
    // carry them across the cutover, or repointing is_system_admin de-admins them with no door
    // (D11) left to restore admin-ness.
    //
    // Start from an empty governance table: migration …000120 may have backfilled rows from the
    // bootseed, and this test asserts on specific profiles it creates.
    sqlx::query("DELETE FROM kb_principal_governance")
        .execute(&pool)
        .await
        .unwrap();
    let team: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','System')
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_system_settings (id, gating_team_slug) VALUES (1,'temper-system')
         ON CONFLICT (id) DO UPDATE SET gating_team_slug='temper-system'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let owner: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('g-owner','G') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let watcher: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('g-watcher','W') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'owner')")
        .bind(team)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'watcher')")
        .bind(team)
        .bind(watcher)
        .execute(&pool)
        .await
        .unwrap();

    run_governance_backfill(&pool).await;

    // The owner is admin now; the watcher is not. is_system_admin reads governance and only that.
    assert_eq!(
        sqlx::query_scalar::<_, Option<bool>>("SELECT is_system_admin($1)")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap(),
        Some(true),
        "a gating-team owner must be carried into governance"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<bool>>("SELECT is_system_admin($1)")
            .bind(watcher)
            .fetch_one(&pool)
            .await
            .unwrap(),
        Some(false),
        "a non-owner member of the gating team must NOT become admin"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn governance_backfill_does_not_fall_open_on_empty_gating_slug(pool: PgPool) {
    // The `gating_team_slug <> ''` guard mirrors 20260720000100. An empty slug matches any team
    // slugged '', so without the guard PASS 3 would mint governance rows for that team's owners —
    // re-opening the exact fall-open …000100 closed. Prod is safe (slug 'temper-system'); this
    // pins the guard so a future edit cannot silently drop it.
    //
    // Start from an empty governance table: migration …000120 may have backfilled rows from the
    // bootseed, and the count assertion below must see only what THIS test's PASS 3 produces.
    sqlx::query("DELETE FROM kb_principal_governance")
        .execute(&pool)
        .await
        .unwrap();
    let team: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('','empty-slug') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_system_settings (id, gating_team_slug) VALUES (1,'')
         ON CONFLICT (id) DO UPDATE SET gating_team_slug=''",
    )
    .execute(&pool)
    .await
    .unwrap();
    let owner: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('empty-owner','E') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'owner')")
        .bind(team)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();

    run_governance_backfill(&pool).await;

    let governance_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_principal_governance")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        governance_rows, 0,
        "the <> '' guard must refuse an empty gating slug — owning a ''-slugged team confers nothing"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<bool>>("SELECT is_system_admin($1)")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap(),
        Some(false),
        "an empty gating slug must lock everyone out of admin, never let a ''-team owner in"
    );
}
