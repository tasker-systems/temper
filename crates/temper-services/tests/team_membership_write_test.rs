#![cfg(feature = "test-db")]

//! `_admin_team_membership_write` called directly (team-role rank plan Task B1).
//!
//! The function never authorizes — the Rust sealed value does — so these tests drive every outcome
//! straight through SQL and pin what must be atomic with the write: the row and its
//! `team_membership_changed` record land together or not at all (C8), the act is told and never
//! inferred (P13), the last-owner guard (C6) and its one unguarded path (R8, `left_ownerless`), and
//! who the record is emitted by (P4).

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn mk_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A profile's `<handle>@web` entity, the emitter a person's act is recorded under.
async fn mk_entity(pool: &PgPool, profile: Uuid, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2 || '@web') RETURNING id",
    )
    .bind(profile)
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn add_row(pool: &PgPool, team: Uuid, profile: Uuid, role: &str, source: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
         VALUES ($1, $2, $3::team_role, $4::team_member_source)",
    )
    .bind(team)
    .bind(profile)
    .bind(role)
    .bind(source)
    .execute(pool)
    .await
    .unwrap();
}

/// One call to the writer. `role`, `expected_*` and `new_source` are SQL text or NULL.
#[derive(Clone, Copy)]
struct Call<'a> {
    emitter: Option<Uuid>,
    actor: Option<Uuid>,
    via: &'a str,
    act: &'a str,
    team: Uuid,
    profile: Uuid,
    role: Option<&'a str>,
    expected_role: Option<&'a str>,
    expected_source: Option<&'a str>,
    new_source: Option<&'a str>,
    guard: bool,
}

async fn write(pool: &PgPool, c: Call<'_>) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT _admin_team_membership_write($1, $2, $3, $4, $5, $6, $7::team_role, $8::team_role, \
                $9::team_member_source, $10::team_member_source, $11, NULL, NULL)",
    )
    .bind(c.emitter)
    .bind(c.actor)
    .bind(c.via)
    .bind(c.act)
    .bind(c.team)
    .bind(c.profile)
    .bind(c.role)
    .bind(c.expected_role)
    .bind(c.expected_source)
    .bind(c.new_source)
    .bind(c.guard)
    .fetch_one(pool)
    .await
}

/// `(payload, emitter_entity_id, references)` of every `team_membership_changed`, oldest first.
async fn events(pool: &PgPool) -> Vec<(Value, Uuid, Value)> {
    sqlx::query_as(
        "SELECT e.payload, e.emitter_entity_id, e.\"references\" FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_membership_changed' ORDER BY e.id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn role_of(pool: &PgPool, team: Uuid, profile: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT role::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(team)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .unwrap()
}

struct World {
    team: Uuid,
    actor: Uuid,
    emitter: Uuid,
    member: Uuid,
}

/// A team with an owner (the actor, with an emitter) and a native `member`.
async fn world(pool: &PgPool) -> World {
    let actor = mk_profile(pool, "actor").await;
    let emitter = mk_entity(pool, actor, "actor").await;
    let member = mk_profile(pool, "member").await;
    let team = mk_team(pool, "acme").await;
    add_row(pool, team, actor, "owner", "native").await;
    add_row(pool, team, member, "member", "native").await;
    World {
        team,
        actor,
        emitter,
        member,
    }
}

fn by(w: &World) -> Call<'static> {
    Call {
        emitter: Some(w.emitter),
        actor: Some(w.actor),
        via: "member_act",
        act: "change",
        team: w.team,
        profile: w.member,
        role: None,
        expected_role: Some("member"),
        expected_source: Some("native"),
        new_source: None,
        guard: true,
    }
}

/// added / changed / removed each write the row and exactly one event, with before/after, actor
/// and via, and references that are exactly the team as subject and the member as principal.
#[sqlx::test(migrations = "../../migrations")]
async fn each_real_change_writes_its_row_and_one_event(pool: PgPool) {
    let w = world(&pool).await;
    let newcomer = mk_profile(&pool, "newcomer").await;

    let added = write(
        &pool,
        Call {
            act: "add",
            profile: newcomer,
            role: Some("watcher"),
            expected_role: None,
            expected_source: None,
            new_source: Some("native"),
            ..by(&w)
        },
    )
    .await
    .unwrap();
    assert_eq!(added, "added");
    assert_eq!(
        role_of(&pool, w.team, newcomer).await.as_deref(),
        Some("watcher")
    );

    let changed = write(
        &pool,
        Call {
            role: Some("maintainer"),
            ..by(&w)
        },
    )
    .await
    .unwrap();
    assert_eq!(changed, "changed");
    assert_eq!(
        role_of(&pool, w.team, w.member).await.as_deref(),
        Some("maintainer")
    );

    let removed = write(
        &pool,
        Call {
            act: "remove",
            expected_role: Some("maintainer"),
            ..by(&w)
        },
    )
    .await
    .unwrap();
    assert_eq!(removed, "removed");
    assert_eq!(role_of(&pool, w.team, w.member).await, None);

    let evs = events(&pool).await;
    assert_eq!(evs.len(), 3, "one event per real change");
    let (add_p, add_emitter, add_refs) = &evs[0];
    assert_eq!(
        add_p,
        &json!({"team_id": w.team, "profile_id": newcomer, "role": "watcher", "source": "native",
                "actor": w.actor, "via": "member_act"})
    );
    assert_eq!(
        *add_emitter, w.emitter,
        "a person's act is emitted by that person's entity"
    );
    assert_eq!(
        add_refs,
        &json!([
            {"rel": "subject", "target": {"kind": "kb_teams", "id": w.team}},
            {"rel": "principal", "target": {"kind": "kb_profiles", "id": newcomer}},
        ]),
        "references are exactly the team as subject and the member as principal"
    );
    assert_eq!(evs[1].0["previous_role"], "member");
    assert_eq!(evs[1].0["role"], "maintainer");
    assert_eq!(evs[2].0["previous_role"], "maintainer");
    assert!(evs[2].0.get("role").is_none(), "a removal carries no role");

    // The cognition firewall: admin events are unanchored.
    let anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_membership_changed' \
            AND (e.producing_anchor_table IS NOT NULL OR e.producing_anchor_id IS NOT NULL)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(anchored, 0);
}

/// P13 and C8: every outcome that is not a real change writes nothing and records nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn no_other_outcome_writes_or_records(pool: PgPool) {
    let w = world(&pool).await;
    let stranger = mk_profile(&pool, "stranger").await;

    // A change to the current role.
    assert_eq!(
        write(
            &pool,
            Call {
                role: Some("member"),
                ..by(&w)
            }
        )
        .await
        .unwrap(),
        "noop"
    );
    // An add over an existing row: `exists` for a member act, a silent `noop` for the two paths
    // whose adds were always DO NOTHING.
    for (via, want) in [
        ("member_act", "exists"),
        ("invitation", "noop"),
        ("access_approval", "noop"),
    ] {
        let got = write(
            &pool,
            Call {
                via,
                act: "add",
                role: Some("owner"),
                expected_role: None,
                expected_source: None,
                new_source: Some("native"),
                ..by(&w)
            },
        )
        .await
        .unwrap();
        assert_eq!(got, want, "add over an existing row via {via}");
    }
    // A change or remove of an absent row.
    for act in ["change", "remove"] {
        let got = write(
            &pool,
            Call {
                act,
                profile: stranger,
                role: Some("watcher"),
                ..by(&w)
            },
        )
        .await
        .unwrap();
        assert_eq!(got, "absent", "{act} of an absent row");
    }
    // A change or remove authorized against a role or source the row no longer has.
    for (expected_role, expected_source) in [("maintainer", "native"), ("member", "idp")] {
        for act in ["change", "remove"] {
            let got = write(
                &pool,
                Call {
                    act,
                    role: Some("watcher"),
                    expected_role: Some(expected_role),
                    expected_source: Some(expected_source),
                    ..by(&w)
                },
            )
            .await
            .unwrap();
            assert_eq!(
                got, "stale",
                "{act} against {expected_role}/{expected_source}"
            );
        }
    }
    // The last-owner guard.
    for (act, role) in [("change", Some("member")), ("remove", None)] {
        let got = write(
            &pool,
            Call {
                act,
                profile: w.actor,
                role,
                expected_role: Some("owner"),
                ..by(&w)
            },
        )
        .await
        .unwrap();
        assert_eq!(got, "last_owner", "{act} of the last owner");
    }

    assert!(events(&pool).await.is_empty(), "nothing recorded");
    assert_eq!(
        role_of(&pool, w.team, w.member).await.as_deref(),
        Some("member")
    );
    assert_eq!(
        role_of(&pool, w.team, w.actor).await.as_deref(),
        Some("owner")
    );
}

/// R8: the unguarded path (SAML reconcile) may leave a team ownerless; when it does the record
/// says `left_ownerless`, and is emitted by the `system` entity — never the member's (P4).
#[sqlx::test(migrations = "../../migrations")]
async fn the_unguarded_path_records_left_ownerless_as_the_system(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let owner_entity = mk_entity(&pool, owner, "owner").await;
    let co = mk_profile(&pool, "co").await;
    let team = mk_team(&pool, "acme").await;
    add_row(&pool, team, owner, "owner", "idp").await;
    add_row(&pool, team, co, "owner", "idp").await;
    let system: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE p.handle = 'system' AND e.name = 'system'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let unguarded = |profile| Call {
        emitter: None,
        actor: None,
        via: "idp_reconcile",
        act: "remove",
        team,
        profile,
        role: None,
        expected_role: Some("owner"),
        expected_source: Some("idp"),
        new_source: None,
        guard: false,
    };
    // Another owner remains: no flag.
    assert_eq!(write(&pool, unguarded(co)).await.unwrap(), "removed");
    // The only owner: removed anyway, flagged.
    assert_eq!(write(&pool, unguarded(owner)).await.unwrap(), "removed");

    let evs = events(&pool).await;
    assert_eq!(evs.len(), 2);
    assert!(
        evs[0].0.get("left_ownerless").is_none(),
        "set only when the team is left ownerless"
    );
    assert_eq!(evs[1].0["left_ownerless"], true);
    for (payload, emitter, _) in &evs {
        assert_eq!(
            *emitter, system,
            "a system act is emitted by the system entity"
        );
        assert_ne!(*emitter, owner_entity, "never by the member acted on");
        assert!(payload.get("actor").is_none(), "the system acted");
        assert_eq!(payload["source"], "idp");
    }
}

/// The guarded path never sets `left_ownerless`, because it never leaves a team ownerless.
#[sqlx::test(migrations = "../../migrations")]
async fn the_guarded_path_never_flags_left_ownerless(pool: PgPool) {
    let w = world(&pool).await;
    let co = mk_profile(&pool, "co").await;
    add_row(&pool, w.team, co, "owner", "native").await;
    let got = write(
        &pool,
        Call {
            act: "remove",
            profile: co,
            expected_role: Some("owner"),
            ..by(&w)
        },
    )
    .await
    .unwrap();
    assert_eq!(got, "removed");
    assert!(events(&pool).await[0].0.get("left_ownerless").is_none());
}

/// The writer refuses a via or act outside its vocabulary, and a person's act with no emitter.
#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_via_or_act_raises(pool: PgPool) {
    let w = world(&pool).await;
    assert!(write(
        &pool,
        Call {
            via: "transfer",
            role: Some("watcher"),
            ..by(&w)
        }
    )
    .await
    .is_err());
    assert!(write(
        &pool,
        Call {
            act: "upsert",
            role: Some("watcher"),
            ..by(&w)
        }
    )
    .await
    .is_err());
    assert!(
        write(
            &pool,
            Call {
                emitter: None,
                role: Some("watcher"),
                ..by(&w)
            }
        )
        .await
        .is_err(),
        "a person's act names that person's emitter"
    );
    assert!(events(&pool).await.is_empty());
    assert_eq!(
        role_of(&pool, w.team, w.member).await.as_deref(),
        Some("member")
    );
}

/// A role change never rewrites a row's source (C7): a change to an idp row keeps it `idp`.
#[sqlx::test(migrations = "../../migrations")]
async fn a_change_never_rewrites_the_source(pool: PgPool) {
    let w = world(&pool).await;
    let provisioned = mk_profile(&pool, "provisioned").await;
    add_row(&pool, w.team, provisioned, "member", "idp").await;
    let got = write(
        &pool,
        Call {
            profile: provisioned,
            role: Some("watcher"),
            expected_source: Some("idp"),
            new_source: Some("native"),
            ..by(&w)
        },
    )
    .await
    .unwrap();
    assert_eq!(got, "changed");
    let source: String = sqlx::query_scalar(
        "SELECT source::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(w.team)
    .bind(provisioned)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(source, "idp");
}
