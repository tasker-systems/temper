#![cfg(feature = "test-db")]

//! Context access predicates (spec §3.8).
//!
//! Every test here is built on one fixture — the org enclosure hierarchy the model is actually
//! about:
//!
//! ```text
//!   EPD ─▸ engineering ─▸ payroll-group ─▸ squad-two
//!                      └▸ security-it-ops        (the sibling — must stay invisible)
//! ```
//!
//! `dana` is a DIRECT member of `squad-two` only, and is therefore a transitive member of
//! `payroll-group`, `engineering`, and `EPD`.
//!
//! The two axes under test, which are NOT the same axis:
//!
//! * **READ inherits UP the enclosure chain.** Dana reads what is at or above her. Never sideways.
//! * **WRITE requires DIRECT membership** in the owning team, with an authoring role.
//!   `watcher` is read-only.

use sqlx::PgPool;
use uuid::Uuid;

/// The EPD hierarchy, plus Dana at the leaf.
struct Org {
    epd: Uuid,
    engineering: Uuid,
    payroll_group: Uuid,
    squad_two: Uuid,
    security_it_ops: Uuid,
    /// Direct member of `squad_two` only.
    dana: Uuid,
    /// Owns nothing, belongs to nothing.
    outsider: Uuid,
}

async fn team(pool: &PgPool, slug: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
}

async fn profile(pool: &PgPool, handle: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
}

async fn encloses(pool: &PgPool, parent: Uuid, child: Uuid) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await?;
    Ok(())
}

async fn join_team(pool: &PgPool, team_id: Uuid, profile_id: Uuid, role: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team_id)
    .bind(profile_id)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(())
}

async fn org(pool: &PgPool) -> sqlx::Result<Org> {
    let epd = team(pool, "epd").await?;
    let engineering = team(pool, "engineering").await?;
    let payroll_group = team(pool, "payroll-group").await?;
    let squad_two = team(pool, "squad-two").await?;
    let security_it_ops = team(pool, "security-it-ops").await?;

    encloses(pool, epd, engineering).await?;
    encloses(pool, engineering, payroll_group).await?;
    encloses(pool, payroll_group, squad_two).await?;
    encloses(pool, engineering, security_it_ops).await?;

    let dana = profile(pool, "dana").await?;
    join_team(pool, squad_two, dana, "member").await?;

    let outsider = profile(pool, "outsider").await?;

    Ok(Org {
        epd,
        engineering,
        payroll_group,
        squad_two,
        security_it_ops,
        dana,
        outsider,
    })
}

/// A context owned by a team.
async fn team_context(pool: &PgPool, owner_team: Uuid, slug: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(owner_team)
    .bind(slug)
    .fetch_one(pool)
    .await
}

/// A context owned by a profile.
async fn personal_context(pool: &PgPool, owner: Uuid, slug: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
}

async fn share_to_team(pool: &PgPool, context_id: Uuid, team_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(context_id)
        .bind(team_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// `kb_access_grants.granted_by_profile_id` — NOT `granted_by_event_id`; grants are a projection
/// and do not carry their emitting event.
async fn grant(
    pool: &PgPool,
    context_id: Uuid,
    principal_table: &str,
    principal_id: Uuid,
    granted_by: Uuid,
    can_read: bool,
    can_write: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (id, subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_contexts', $1, $2, $3, $4, $5, $6)",
    )
    .bind(context_id)
    .bind(principal_table)
    .bind(principal_id)
    .bind(can_read)
    .bind(can_write)
    .bind(granted_by)
    .execute(pool)
    .await?;
    Ok(())
}

async fn resource_in(
    pool: &PgPool,
    context_id: Uuid,
    owner: Uuid,
    title: &str,
) -> sqlx::Result<Uuid> {
    let resource: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) \
         VALUES (uuid_generate_v7(), $1, '') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES (uuid_generate_v7(), $1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(context_id)
    .bind(owner)
    .execute(pool)
    .await?;
    Ok(resource)
}

async fn can_read(pool: &PgPool, p: Uuid, c: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT context_readable_by_profile($1, $2)")
        .bind(p)
        .bind(c)
        .fetch_one(pool)
        .await
}

async fn can_author(pool: &PgPool, p: Uuid, c: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(p)
        .bind(c)
        .fetch_one(pool)
        .await
}

async fn sees_resource(pool: &PgPool, p: Uuid, r: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(p)
    .bind(r)
    .fetch_one(pool)
    .await
}

// =================================================================================================
// READ inherits UP the enclosure chain.
// =================================================================================================

/// The bug this migration exists to fix. Dana is a direct member of `squad-two` only, and therefore
/// a transitive member of every team enclosing it. Every one of those teams' OWN contexts must read.
///
/// Before this migration the team-owned arm was flat (direct members only), so all three of these
/// returned false — while a context merely *shared* to the same teams read fine. Owning was somehow
/// more private than sharing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn read_inherits_up_the_enclosure_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    let own = team_context(&pool, o.squad_two, "squad-two-ctx").await?;
    let group = team_context(&pool, o.payroll_group, "payroll-ctx").await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let epd = team_context(&pool, o.epd, "epd-ctx").await?;

    for (label, ctx) in [
        ("her own squad's context", own),
        ("her product group's context", group),
        ("engineering's context", eng),
        ("EPD's context", epd),
    ] {
        assert!(
            can_read(&pool, o.dana, ctx).await?,
            "a squad-two member must read {label} — membership is transitive up the enclosure chain"
        );
    }

    // ...and the resources inside them, or the read is useless.
    let r = resource_in(&pool, eng, o.outsider, "eng-doc").await?;
    assert!(
        sees_resource(&pool, o.dana, r).await?,
        "reading the context must mean reading the resources homed in it"
    );

    Ok(())
}

/// Read never flows sideways, and never flows down. `security-it-ops` is Dana's cousin (a sibling of
/// her product group under engineering); `squad-one` would be her sibling. Neither is reachable.
/// Nor can someone higher up read down into her squad.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn read_never_flows_sideways_or_down(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    // Sideways: a cousin domain group's context.
    let sec = team_context(&pool, o.security_it_ops, "security-ctx").await?;
    assert!(
        !can_read(&pool, o.dana, sec).await?,
        "security-it-ops is a cousin, not an ancestor — it must be invisible"
    );
    let secret = resource_in(&pool, sec, o.outsider, "incident-report").await?;
    assert!(
        !sees_resource(&pool, o.dana, secret).await?,
        "nor its resources"
    );

    // Downward: an engineering-only member must NOT read squad-two's own context. Enclosure grants
    // read UPWARD only — a director does not automatically read every squad beneath them.
    let director = profile(&pool, "director").await?;
    join_team(&pool, o.engineering, director, "owner").await?;
    let squad_ctx = team_context(&pool, o.squad_two, "squad-two-ctx").await?;
    assert!(
        !can_read(&pool, director, squad_ctx).await?,
        "read inherits UP, never DOWN — even for an owner of the enclosing team"
    );

    // A total outsider reads nothing.
    assert!(!can_read(&pool, o.outsider, squad_ctx).await?);

    Ok(())
}

/// The arms that already worked keep working: personal ownership, shares to an enclosing team, and
/// explicit read-grants. This is the floor — the failure mode this migration must not have is a
/// silently dropped branch.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_pre_existing_read_branches_all_survive(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    // personal
    let mine = personal_context(&pool, o.dana, "mine").await?;
    assert!(
        can_read(&pool, o.dana, mine).await?,
        "her own personal context"
    );
    assert!(
        !can_read(&pool, o.outsider, mine).await?,
        "and nobody else's"
    );

    // shared to an enclosing team
    let shared = personal_context(&pool, o.outsider, "shared").await?;
    share_to_team(&pool, shared, o.engineering).await?;
    assert!(
        can_read(&pool, o.dana, shared).await?,
        "a context shared to an enclosing team reaches every member beneath it"
    );

    // explicit read-grant to the profile
    let granted = personal_context(&pool, o.outsider, "granted").await?;
    assert!(
        !can_read(&pool, o.dana, granted).await?,
        "no grant yet ⇒ denied"
    );
    grant(
        &pool,
        granted,
        "kb_profiles",
        o.dana,
        o.outsider,
        true,
        false,
    )
    .await?;
    assert!(
        can_read(&pool, o.dana, granted).await?,
        "an explicit read-grant grants read"
    );

    Ok(())
}

// =================================================================================================
// WRITE requires DIRECT membership. It does NOT inherit up.
// =================================================================================================

/// The inversion this migration closes. Before it, `context_authorable_by_profile` ancestor-expanded
/// while the read predicate was flat — so Dana could AUTHOR into engineering's context while being
/// unable to READ it. Write was strictly wider than read on the same object.
///
/// Now: she reads it (above) and cannot write it (here).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn write_does_not_inherit_up_the_enclosure_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    let own = team_context(&pool, o.squad_two, "squad-two-ctx").await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let epd = team_context(&pool, o.epd, "epd-ctx").await?;

    assert!(
        can_author(&pool, o.dana, own).await?,
        "she authors in her OWN team's context — direct membership, authoring role"
    );

    for (label, ctx) in [("engineering's", eng), ("EPD's", epd)] {
        assert!(
            can_read(&pool, o.dana, ctx).await?,
            "she reads {label} context..."
        );
        assert!(
            !can_author(&pool, o.dana, ctx).await?,
            "...but must NOT author into it — mutation needs DIRECT membership. \
             (Before this migration she could write here but not read here.)"
        );
    }

    Ok(())
}

/// `watcher` is read-only. No access predicate consulted `kb_team_members.role` at all before this
/// migration — 0 of 15 — so a watcher could author.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_watcher_reads_but_cannot_author(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let ctx = team_context(&pool, o.squad_two, "squad-two-ctx").await?;

    let watcher = profile(&pool, "watcher").await?;
    join_team(&pool, o.squad_two, watcher, "watcher").await?;

    assert!(
        can_read(&pool, watcher, ctx).await?,
        "a watcher reads the team's context"
    );
    assert!(
        !can_author(&pool, watcher, ctx).await?,
        "a watcher must never author"
    );

    // ...while the authoring roles all may.
    for role in ["owner", "maintainer", "member"] {
        let p = profile(&pool, &format!("{role}-p")).await?;
        join_team(&pool, o.squad_two, p, role).await?;
        assert!(
            can_author(&pool, p, ctx).await?,
            "{role} must be able to author"
        );
    }

    Ok(())
}

/// An explicit WRITE grant still reaches through the enclosure chain. A grant is a deliberate act of
/// delegation, not an accident of membership — granting write to an umbrella team is a considered
/// decision to let everyone under it author. This arm is intentionally NOT narrowed.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_explicit_write_grant_still_reaches_through_the_chain(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let ctx = personal_context(&pool, o.outsider, "delegated").await?;

    assert!(
        !can_author(&pool, o.dana, ctx).await?,
        "no grant ⇒ no write"
    );

    // granted to ENGINEERING — an enclosing team, not Dana's own
    grant(
        &pool,
        ctx,
        "kb_teams",
        o.engineering,
        o.outsider,
        true,
        true,
    )
    .await?;

    assert!(
        can_author(&pool, o.dana, ctx).await?,
        "an explicit write-grant to an enclosing team reaches its members — deliberate delegation"
    );

    Ok(())
}

// =================================================================================================
// resources_readable_by gains the 'context' principal kind.
// =================================================================================================

/// `resources_readable_by` is `LANGUAGE sql` — a UNION with `WHERE p_principal_kind = …` guards, not
/// a plpgsql IF/ELSIF. An unhandled kind did NOT raise; it silently returned zero rows.
///
/// That is why this test homes a resource and asserts it comes BACK. Asserting an empty result would
/// have passed against the unmigrated schema and proved nothing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn resources_readable_by_dispatches_a_context_principal(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let ctx = personal_context(&pool, o.dana, "c2").await?;
    let other = personal_context(&pool, o.dana, "c3").await?;

    let inside = resource_in(&pool, ctx, o.dana, "inside").await?;
    let outside = resource_in(&pool, other, o.dana, "outside").await?;

    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resources_readable_by('context', $1)")
            .bind(ctx)
            .fetch_all(&pool)
            .await?;
    assert!(
        ids.contains(&inside),
        "the context's own interior must come back"
    );
    assert!(
        !ids.contains(&outside),
        "a resource homed elsewhere must not"
    );

    // soft-delete floor
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(inside)
        .execute(&pool)
        .await?;
    let after: Vec<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resources_readable_by('context', $1)")
            .bind(ctx)
            .fetch_all(&pool)
            .await?;
    assert!(
        !after.contains(&inside),
        "a soft-deleted resource must drop out"
    );

    Ok(())
}

/// The other kinds are unchanged, and an unknown kind stays fail-closed.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_other_principal_kinds_are_unchanged(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let ctx = personal_context(&pool, o.dana, "pk").await?;
    let mine = resource_in(&pool, ctx, o.dana, "mine").await?;

    let by_profile: Vec<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resources_readable_by('profile', $1)")
            .bind(o.dana)
            .fetch_all(&pool)
            .await?;
    assert!(
        by_profile.contains(&mine),
        "the 'profile' kind still resolves"
    );

    let unknown: i64 =
        sqlx::query_scalar("SELECT count(*) FROM resources_readable_by('nonsense', $1)")
            .bind(o.dana)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        unknown, 0,
        "an unknown kind stays fail-closed (empty, not an error)"
    );

    Ok(())
}

// =================================================================================================
// The copies that used to restate the rule now route through the one read-set.
// =================================================================================================

/// `graph_home_contexts`'s `candidates` CTE documented itself as "a proven superset (same branches)"
/// of `context_visible_to` — a claim that held only while both were equally wrong. It had ALSO gone
/// flat on the share arm. Had the predicate been widened without it, it would have become a SUBSET
/// and dropped contexts out of the graph view entirely.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_graph_view_lists_every_context_the_profile_can_read(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let sec = team_context(&pool, o.security_it_ops, "security-ctx").await?;

    let listed: Vec<Uuid> = sqlx::query_scalar("SELECT context_id FROM graph_home_contexts($1)")
        .bind(o.dana)
        .fetch_all(&pool)
        .await?;

    assert!(
        listed.contains(&eng),
        "engineering's context must appear in Dana's graph view"
    );
    assert!(!listed.contains(&sec), "the cousin team's context must not");

    Ok(())
}

/// `edges_visible_to` gates each edge on its HOME anchor. An edge homed in engineering's context
/// must now be visible to Dana — it was not, because that arm carried its own flat copy of the rule.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn edges_homed_in_an_enclosing_teams_context_are_visible(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;

    let a = resource_in(&pool, eng, o.outsider, "a").await?;
    let b = resource_in(&pool, eng, o.outsider, "b").await?;

    // `edge_kind` is the enum (express/contains/leads_to/near); `relates_to` is a free-text label.
    // The two event references are NOT NULL — reuse an event the migrations already emitted.
    let edge: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_edges \
           (id, source_table, source_id, target_table, target_id, edge_kind, label, \
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         SELECT uuid_generate_v7(), 'kb_resources', $1, 'kb_resources', $2, 'near', 'relates_to', \
                'kb_contexts', $3, e.id, e.id \
           FROM kb_events e ORDER BY e.id LIMIT 1 \
         RETURNING id",
    )
    .bind(a)
    .bind(b)
    .bind(eng)
    .fetch_one(&pool)
    .await?;

    let visible: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) e WHERE e.edge_id = $2)",
    )
    .bind(o.dana)
    .bind(edge)
    .fetch_one(&pool)
    .await?;

    assert!(
        visible,
        "an edge homed in an enclosing team's context must be visible"
    );

    Ok(())
}
