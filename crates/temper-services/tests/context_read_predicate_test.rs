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

/// A write-grant on a RESOURCE subject (the context `grant` helper is context-subject only).
async fn grant_resource(
    pool: &PgPool,
    resource_id: Uuid,
    principal_table: &str,
    principal_id: Uuid,
    granted_by: Uuid,
    can_write: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (id, subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_resources', $1, $2, $3, true, $4, $5)",
    )
    .bind(resource_id)
    .bind(principal_table)
    .bind(principal_id)
    .bind(can_write)
    .bind(granted_by)
    .execute(pool)
    .await?;
    Ok(())
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

// =================================================================================================
// The soft-delete WRITE floor (I6). A tombstone is unmodifiable on every axis — the write peer of
// the read-side soft-delete floor. Surfaced by the adversarial review of the write axis.
// =================================================================================================

async fn can_modify(pool: &PgPool, p: Uuid, r: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(p)
        .bind(r)
        .fetch_one(pool)
        .await
}

async fn soft_delete(pool: &PgPool, r: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(r)
        .execute(pool)
        .await?;
    Ok(())
}

/// The tombstone freeze. `can_modify_resource` reached a resource through four arms — author, direct
/// write-grant, team write-grant, container-authorship — none of which checked `is_active`. So a
/// soft-deleted resource was writable through EVERY arm while being read-invisible on every axis:
/// `can_modify` said yes, `resources_visible_to` said no, on the same pair. And the write committed
/// (a body-only PATCH gates solely on this predicate).
///
/// After the floor: every arm denies once the resource is a tombstone, and read/write agree.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_tombstone_is_unmodifiable_on_every_arm(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    // The four ways in. Each resource is homed in a team context Dana can author into (her own
    // squad), so the container-cascade arm is live; then we attach the other three arms.
    let ctx = team_context(&pool, o.squad_two, "sq2-ctx").await?;

    // Arm 1 — author/originator (resource_in homes with originator = owner = the given profile).
    let authored = resource_in(&pool, ctx, o.dana, "authored").await?;

    // Arm 2 — direct profile write-grant to Dana on a resource she doesn't own.
    let by_profile_grant = resource_in(&pool, ctx, o.outsider, "by-profile-grant").await?;
    grant_resource(
        &pool,
        by_profile_grant,
        "kb_profiles",
        o.dana,
        o.outsider,
        true,
    )
    .await?;

    // Arm 3 — team write-grant on Dana's own team.
    let by_team_grant = resource_in(&pool, ctx, o.outsider, "by-team-grant").await?;
    grant_resource(
        &pool,
        by_team_grant,
        "kb_teams",
        o.squad_two,
        o.outsider,
        true,
    )
    .await?;

    // Arm 4 — container-authorship cascade (Dana authors her squad's context ⇒ may modify its nodes).
    let by_container = resource_in(&pool, ctx, o.outsider, "by-container").await?;

    let all = [authored, by_profile_grant, by_team_grant, by_container];

    // Live: every arm permits.
    for r in all {
        assert!(
            can_modify(&pool, o.dana, r).await?,
            "a live resource is modifiable via its arm"
        );
    }

    // Tombstone every one, then re-probe: every arm must now DENY, and read must agree.
    for r in all {
        soft_delete(&pool, r).await?;
        assert!(
            !can_modify(&pool, o.dana, r).await?,
            "a soft-deleted resource must be unmodifiable — the write floor gates every arm"
        );
        let readable: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
        )
        .bind(o.dana)
        .bind(r)
        .fetch_one(&pool)
        .await?;
        assert!(
            !readable,
            "and read agrees — a tombstone is invisible; write and read no longer diverge"
        );
    }

    Ok(())
}

/// The floor must not over-narrow: an ACTIVE resource stays modifiable. This guards that the
/// wrapping `EXISTS(... is_active)` conjunct didn't break the live path, and re-checks that the
/// floor composes with the role gate rather than masking it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_write_floor_does_not_touch_live_resources(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;
    let ctx = team_context(&pool, o.squad_two, "sq2-ctx").await?;
    let live = resource_in(&pool, ctx, o.dana, "live").await?;

    assert!(
        can_modify(&pool, o.dana, live).await?,
        "an active authored resource is modifiable"
    );

    // A watcher of the owning team cannot modify via the container cascade even though the resource
    // is live — the role gate denies, not the floor. (Confirms the two gates compose.)
    let watcher = profile(&pool, "wr").await?;
    join_team(&pool, o.squad_two, watcher, "watcher").await?;
    let not_watchers = resource_in(&pool, ctx, o.outsider, "not-watchers").await?;
    assert!(
        !can_modify(&pool, watcher, not_watchers).await?,
        "a watcher cannot author via the container cascade — role gate holds on a live resource"
    );

    Ok(())
}

// =================================================================================================
// The two entry points to the context read-set agree (migration 20260807000010).
// =================================================================================================

/// `contexts_readable_by(profile)` and `contexts_readable_by_teams(profile, teams)` return the same
/// set — across every arm, over the nested hierarchy.
///
/// Migration `20260807000010` moved the four-arm body into the two-argument form so
/// `resources_visible_to` — which already holds the expanded team closure — stops paying for a
/// second full recursive expansion inside the wrapper. The wrapper is now the only thing that
/// expands it.
///
/// **The risk that justifies this test is not the refactor; it is that this is an AUTHORIZATION
/// predicate with two entry points.** Two bodies can drift and a drift here is a leak or a lockout,
/// so the invariant to hold is not "the new one works" but "they are the same set". The fixture is
/// the enclosure hierarchy rather than a flat team, because the arm that changed is the one that
/// walks it — the community corpus this was found on has three teams at depth one and a
/// team-anchored grant arm returning zero rows, and would have passed almost anything.
///
/// The sibling `security-it-ops` context is present throughout and must be in NEITHER set, and the
/// contexts up Dana's chain must be in BOTH — an agreement test over two empty sets, or two
/// identically over-broad ones, would pass while being worthless.
///
/// **What this test does NOT hold, established by bite-check rather than assumed.** Neutering an
/// arm inside the two-argument body leaves the two forms *agreeing* — the wrapper delegates, so
/// both lose it together — and this test passes. It was `the_pre_existing_read_branches_all_survive`
/// that caught that probe. So the division is: **that** test holds arm coverage, **this** one holds
/// that the two entry points cannot drift apart. Neither is sufficient alone, and reading this one
/// as "the context read-set is correct" would be a mistake. The probe it does catch is a wrapper
/// that expands the closure differently from its caller — swapping `profile_reachable_teams` for
/// `profile_effective_teams` in the wrapper fails it on three contexts.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn contexts_readable_by_teams_agrees_with_the_profile_form(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    // Arm 2 — owned by teams up the whole chain, plus the sibling that must stay invisible.
    let squad = team_context(&pool, o.squad_two, "squad-two-ctx").await?;
    let group = team_context(&pool, o.payroll_group, "payroll-ctx").await?;
    let eng = team_context(&pool, o.engineering, "engineering-ctx").await?;
    let epd = team_context(&pool, o.epd, "epd-ctx").await?;
    let sibling = team_context(&pool, o.security_it_ops, "security-ctx").await?;

    let both: Vec<Uuid> = sqlx::query_scalar(
        "SELECT context_id FROM contexts_readable_by($1)
         INTERSECT
         SELECT context_id FROM contexts_readable_by_teams(
             $1, coalesce((SELECT array_agg(team_id) FROM profile_reachable_teams($1)), '{}'::uuid[]))",
    )
    .bind(o.dana)
    .fetch_all(&pool)
    .await?;

    let disagreement: Vec<Uuid> = sqlx::query_scalar(
        "(SELECT context_id FROM contexts_readable_by($1)
          EXCEPT
          SELECT context_id FROM contexts_readable_by_teams(
              $1, coalesce((SELECT array_agg(team_id) FROM profile_reachable_teams($1)), '{}'::uuid[])))
         UNION ALL
         (SELECT context_id FROM contexts_readable_by_teams(
              $1, coalesce((SELECT array_agg(team_id) FROM profile_reachable_teams($1)), '{}'::uuid[]))
          EXCEPT
          SELECT context_id FROM contexts_readable_by($1))",
    )
    .bind(o.dana)
    .fetch_all(&pool)
    .await?;

    assert!(
        disagreement.is_empty(),
        "the two entry points must return the SAME set; they differ on {disagreement:?}"
    );

    // Precondition: the agreement above is over a populated set, not two empty ones.
    for (label, ctx) in [
        ("her own squad's context", squad),
        ("her product group's context", group),
        ("engineering's context", eng),
        ("EPD's context", epd),
    ] {
        assert!(
            both.contains(&ctx),
            "precondition: {label} is readable, so the agreement is not vacuous"
        );
    }
    assert!(
        !both.contains(&sibling),
        "precondition: the sibling domain's context is in NEITHER set — two identically \
         over-broad sets would agree just as well as two correct ones"
    );

    // And the outsider, who belongs to nothing, agrees at zero team-derived rows — the arm where a
    // NULL/empty team array could have fallen open instead of closed.
    let outsider_rows: Vec<Uuid> = sqlx::query_scalar(
        "SELECT context_id FROM contexts_readable_by_teams(
             $1, coalesce((SELECT array_agg(team_id) FROM profile_reachable_teams($1)), '{}'::uuid[]))",
    )
    .bind(o.outsider)
    .fetch_all(&pool)
    .await?;
    for ctx in [squad, group, eng, epd, sibling] {
        assert!(
            !outsider_rows.contains(&ctx),
            "an empty team closure must reach NOTHING through the team arms — falls closed, \
             never open"
        );
    }

    Ok(())
}

/// `resources_visible_to` still admits a resource reachable only through the enclosure chain.
///
/// The context arm is the one migration `20260807000010` rewired, and it is the arm that carries
/// team-derived access. This asserts the outcome that arm exists for: a resource Dana neither owns
/// nor was granted, homed in a context owned by a team four levels above her, is visible — and the
/// sibling domain's equivalent is not.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn resources_visible_to_still_reaches_up_the_enclosure_chain(
    pool: PgPool,
) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    let epd_ctx = team_context(&pool, o.epd, "epd-ctx").await?;
    let sibling_ctx = team_context(&pool, o.security_it_ops, "security-ctx").await?;

    let reachable = resource_in_context(&pool, epd_ctx, o.outsider, "epd-doc").await?;
    let sideways = resource_in_context(&pool, sibling_ctx, o.outsider, "security-doc").await?;

    let visible: Vec<Uuid> = sqlx::query_scalar("SELECT resource_id FROM resources_visible_to($1)")
        .bind(o.dana)
        .fetch_all(&pool)
        .await?;

    assert!(
        visible.contains(&reachable),
        "a resource homed in an ancestor team's context is visible — this is the arm the migration \
         rewired, and it is owned by someone else and granted to nobody"
    );
    assert!(
        !visible.contains(&sideways),
        "and the sibling domain's resource is still not: read inherits UP, never sideways"
    );

    Ok(())
}

/// Home a resource in `ctx`, owned by `owner`. Enough of a row for the read predicates; no blocks.
async fn resource_in_context(
    pool: &PgPool,
    ctx: Uuid,
    owner: Uuid,
    slug: &str,
) -> sqlx::Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri, is_active) \
         VALUES (uuid_generate_v7(), $1, '', true) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO kb_resource_homes (id, resource_id, anchor_table, anchor_id, \
                                        owner_profile_id, originator_profile_id) \
         VALUES (uuid_generate_v7(), $1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await?;
    Ok(id)
}
