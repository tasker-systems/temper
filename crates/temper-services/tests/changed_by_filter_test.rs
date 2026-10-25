//! "What did this reader change": the list door's `changed_by=@me` filter and `changed_by_me`
//! sort, and the contexts read's `last_changed_by_you` — both read through the one definition of
//! "your change" in `20261106000000_reader_changes.sql`.
//!
//! The contract, each arm one test below:
//!
//! - **The reader's own changes, by profile.** A change counts when the reader's profile emitted
//!   it, on any surface. A teammate's change to the reader's own resource does not count; the
//!   reader's change to a teammate's resource does — which is exactly what the `owner` filter gets
//!   wrong both ways. A machine client the reader registered is its own profile, and its changes
//!   are not the reader's.
//! - **Most recent first, by the reader's own latest change**, not by the resource's `updated`.
//! - **Under the visibility gate.** A resource the reader changed and can no longer read is absent
//!   from the rows and from `total`.
//! - **The trail agrees.** Every resource the filter admits has an event from the reader's profile
//!   in its element trail, and every visible resource whose trail has one is admitted.
//! - **Refusals:** `changed_by` other than `@me`, and the sort without the filter, are `400`s.
//! - **Reading records nothing.**

#![cfg(feature = "test-db")]

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::authorship::ActContext;
use temper_core::types::element_trail::ElementKind;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_services::error::ApiError;
use temper_services::services::{context_service, event_service};
use temper_workflow::operations::{Backend, CreateResource, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::{
    ResourceListParams, ResourceListResponse, ResourceSortField, SortOrder,
};

/// A profile with its per-surface emitter entities, as `ids_filter_test.rs` seeds one.
async fn seed_profile(pool: &PgPool, local: &str) -> Uuid {
    let profile_id = Uuid::now_v7();
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(local)
        .bind(format!("{local}@example.com"))
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    profile_id
}

/// A context owned by `profile`.
async fn seed_own_context(pool: &PgPool, profile: Uuid, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,$3,$3)",
    )
    .bind(id)
    .bind(profile)
    .bind(slug)
    .execute(pool)
    .await
    .expect("seed context");
    id
}

/// A team with `members`, and a context it owns that every member can read and author.
async fn seed_team_context(pool: &PgPool, members: &[Uuid]) -> (Uuid, Uuid) {
    let team: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1,$1) RETURNING id")
            .bind(format!(
                "changes-{}",
                &Uuid::now_v7().simple().to_string()[..8]
            ))
            .fetch_one(pool)
            .await
            .expect("seed team");
    for m in members {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'member')",
        )
        .bind(team)
        .bind(m)
        .execute(pool)
        .await
        .expect("seed membership");
    }
    let context = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_teams',$2,'shared','shared')",
    )
    .bind(context)
    .bind(team)
    .execute(pool)
    .await
    .expect("seed team context");
    (team, context)
}

fn backend(pool: &PgPool, profile: Uuid) -> DbBackend {
    DbBackend::new(pool.clone(), ProfileId::from(profile))
}

async fn create(backend: &DbBackend, context: Uuid, slug: &str) -> Uuid {
    backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: slug.to_string(),
            doctype: "note".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: slug.to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some(format!("test://{slug}")),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create")
        .value
        .id
        .into()
}

async fn retitle(backend: &DbBackend, resource: Uuid, title: &str) {
    backend
        .update_resource(UpdateResource {
            open_meta_add: None,
            resource: ResourceId::from(resource),
            title: Some(title.to_string()),
            slug: None,
            body: None,
            managed_meta: None,
            open_meta: None,
            goal: None,
            move_to: None,
            context_ref: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("update");
}

fn mine() -> ResourceListParams {
    ResourceListParams {
        changed_by: Some("@me".into()),
        sort: Some(ResourceSortField::ChangedByMe),
        order: Some(SortOrder::Desc),
        ..Default::default()
    }
}

async fn list(
    pool: &PgPool,
    reader: Uuid,
    params: ResourceListParams,
) -> Result<ResourceListResponse, ApiError> {
    substrate_read::list_select(pool, ProfileId::from(reader), params).await
}

async fn changed(pool: &PgPool, reader: Uuid) -> Vec<Uuid> {
    list(pool, reader, mine())
        .await
        .expect("list")
        .rows
        .into_iter()
        .map(|r| r.id.into())
        .collect()
}

fn bad_request(result: Result<ResourceListResponse, ApiError>) -> String {
    match result {
        Err(ApiError::BadRequest(msg)) => msg,
        Err(other) => panic!("expected BadRequest, got {other:?}"),
        Ok(_) => panic!("expected BadRequest, got a page"),
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn counts_the_readers_changes_by_profile_not_by_owner(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;
    let teammate = seed_profile(&pool, "teammate").await;
    let (_, shared) = seed_team_context(&pool, &[reader, teammate]).await;

    // The reader's own resource, then changed only by the teammate: owned, not changed.
    let reader_owned = create(&backend(&pool, reader), shared, "reader-owned").await;
    // The teammate's resource, changed by the reader: changed, not owned.
    let teammate_owned = create(&backend(&pool, teammate), shared, "teammate-owned").await;
    retitle(
        &backend(&pool, reader),
        teammate_owned,
        "the reader was here",
    )
    .await;
    // The teammate's resource the reader never touched.
    create(&backend(&pool, teammate), shared, "untouched").await;

    let got = changed(&pool, reader).await;
    assert!(
        got.contains(&teammate_owned),
        "a change to another's resource is the reader's"
    );
    assert!(
        got.contains(&reader_owned),
        "creating it is a change by the reader"
    );
    assert_eq!(got.len(), 2, "nothing the reader did not change: {got:?}");

    // The teammate's side: only what the teammate changed, which includes the reader's resource
    // only if the teammate changed it. They did not, so it is absent.
    let theirs = changed(&pool, teammate).await;
    assert!(!theirs.contains(&reader_owned));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_machine_client_the_reader_registered_is_not_the_reader(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;
    let machine = seed_profile(&pool, "machine").await;
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1,'bot',$2,$3)",
    )
    .bind(format!("test-client-{machine}"))
    .bind(machine)
    .bind(reader)
    .execute(&pool)
    .await
    .expect("register machine client");
    let (_, shared) = seed_team_context(&pool, &[reader, machine]).await;

    let by_machine = create(&backend(&pool, machine), shared, "by-machine").await;

    assert!(
        changed(&pool, reader).await.is_empty(),
        "the machine's change is not the reader's"
    );
    assert_eq!(changed(&pool, machine).await, vec![by_machine]);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn orders_by_the_readers_own_latest_change(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;
    let teammate = seed_profile(&pool, "teammate").await;
    let (_, shared) = seed_team_context(&pool, &[reader, teammate]).await;
    let r = backend(&pool, reader);

    let first = create(&r, shared, "first").await;
    let second = create(&r, shared, "second").await;
    // The reader comes back to `first`, so it is their latest change.
    retitle(&r, first, "first, again").await;
    // A teammate then changes `second`. Its `updated` is now the newest, but the reader's own
    // latest change is still to `first`, and that is what this order is by.
    retitle(&backend(&pool, teammate), second, "second, by the teammate").await;

    assert_eq!(changed(&pool, reader).await, vec![first, second]);

    // The plain `updated` order disagrees, which is what makes the sort above a witness.
    let by_updated: Vec<Uuid> = list(
        &pool,
        reader,
        ResourceListParams {
            changed_by: Some("@me".into()),
            ..Default::default()
        },
    )
    .await
    .expect("list")
    .rows
    .into_iter()
    .map(|r| r.id.into())
    .collect();
    assert_eq!(by_updated, vec![second, first]);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_change_the_reader_can_no_longer_read_is_absent_from_rows_and_total(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;
    let teammate = seed_profile(&pool, "teammate").await;
    let (team, shared) = seed_team_context(&pool, &[reader, teammate]).await;
    let own = seed_own_context(&pool, reader, "own").await;

    let kept = create(&backend(&pool, reader), own, "kept").await;
    // The teammate's resource, which the reader changed. The reader does not own it, so team
    // membership is their only reach to it; a resource they created stays theirs to read.
    let lost = create(&backend(&pool, teammate), shared, "lost").await;
    retitle(&backend(&pool, reader), lost, "the reader was here").await;
    let before: Vec<Uuid> = changed(&pool, reader).await;
    assert!(
        before.contains(&lost),
        "while a member, the reader's change is listed"
    );

    // The reader leaves the team, and with it their reach into the shared context.
    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(team)
        .bind(reader)
        .execute(&pool)
        .await
        .expect("leave team");

    let page = list(&pool, reader, mine()).await.expect("list");
    let rows: Vec<Uuid> = page.rows.iter().map(|r| r.id.into()).collect();
    assert_eq!(rows, vec![kept]);
    assert_eq!(page.total, 1, "the count holds only what the rows hold");
    let facet_total: i64 = page.facets.doc_type.values().sum();
    assert_eq!(
        facet_total, 1,
        "no histogram counts the unreadable change either"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn agrees_with_every_visible_resources_element_trail(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;
    let teammate = seed_profile(&pool, "teammate").await;
    let (_, shared) = seed_team_context(&pool, &[reader, teammate]).await;
    let a = create(&backend(&pool, reader), shared, "a").await;
    let b = create(&backend(&pool, teammate), shared, "b").await;
    let c = create(&backend(&pool, teammate), shared, "c").await;
    retitle(&backend(&pool, reader), b, "b by reader").await;
    retitle(&backend(&pool, teammate), a, "a by teammate").await;

    let reader_entities: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1")
            .bind(reader)
            .fetch_all(&pool)
            .await
            .expect("entities");

    let mut by_trail = Vec::new();
    for id in [a, b, c] {
        let trail =
            event_service::element_trail(&pool, ProfileId::from(reader), ElementKind::Node, id)
                .await
                .expect("trail");
        if trail
            .events
            .iter()
            .any(|e| reader_entities.contains(&e.actor_entity_id))
        {
            by_trail.push(id);
        }
    }
    let mut by_filter = changed(&pool, reader).await;
    by_filter.sort();
    by_trail.sort();
    assert_eq!(
        by_filter, by_trail,
        "the filter and the trail must name the same resources"
    );
    assert!(!by_filter.contains(&c));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn refuses_another_principal_and_the_sort_without_the_filter(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;

    for other in ["someone-else", "", "@ME", " @me"] {
        let msg = bad_request(
            list(
                &pool,
                reader,
                ResourceListParams {
                    changed_by: Some(other.into()),
                    ..Default::default()
                },
            )
            .await,
        );
        assert!(msg.contains("changed_by"), "{other:?}: {msg}");
    }

    let msg = bad_request(
        list(
            &pool,
            reader,
            ResourceListParams {
                sort: Some(ResourceSortField::ChangedByMe),
                ..Default::default()
            },
        )
        .await,
    );
    assert!(msg.contains("changed_by=@me"), "{msg}");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reading_either_answer_records_nothing(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;
    let own = seed_own_context(&pool, reader, "own").await;
    create(&backend(&pool, reader), own, "x").await;

    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_events")
            .fetch_one(&pool)
            .await
            .expect("count")
    };
    let before = count().await;
    list(&pool, reader, mine()).await.expect("list");
    context_service::list_visible(&pool, ProfileId::from(reader))
        .await
        .expect("contexts");
    assert_eq!(count().await, before);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn each_place_says_when_the_reader_last_changed_something_there(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;
    let teammate = seed_profile(&pool, "teammate").await;
    let (_, shared) = seed_team_context(&pool, &[reader, teammate]).await;
    let own = seed_own_context(&pool, reader, "own").await;
    let quiet = seed_own_context(&pool, reader, "quiet").await;

    let x = create(&backend(&pool, reader), own, "x").await;
    retitle(&backend(&pool, reader), x, "x again").await;
    // Only the teammate works in the shared context.
    create(&backend(&pool, teammate), shared, "theirs").await;

    let latest_own: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT max(ev.occurred_at) FROM kb_events ev \
           JOIN kb_entities en ON en.id = ev.emitter_entity_id \
          WHERE en.profile_id = $1 AND ev.producing_anchor_table = 'kb_contexts' \
            AND ev.producing_anchor_id = $2",
    )
    .bind(reader)
    .bind(own)
    .fetch_one(&pool)
    .await
    .expect("latest own");

    let rows = context_service::list_visible(&pool, ProfileId::from(reader))
        .await
        .expect("contexts");
    let at = |id: Uuid| {
        rows.iter()
            .find(|c| Uuid::from(c.id) == id)
            .unwrap_or_else(|| panic!("context {id} listed"))
            .last_changed_by_you
    };
    assert_eq!(at(own), Some(latest_own));
    assert_eq!(
        at(shared),
        None,
        "a teammate's changes are not the reader's"
    );
    assert_eq!(at(quiet), None, "a place never changed has no time");

    // The teammate's view of the shared context carries their own time, not the reader's absence.
    let theirs = context_service::list_visible(&pool, ProfileId::from(teammate))
        .await
        .expect("contexts");
    let shared_for_teammate = theirs.iter().find(|c| Uuid::from(c.id) == shared).unwrap();
    assert!(shared_for_teammate.last_changed_by_you.is_some());
}

/// The two readers gate on their own, not only through their callers: called directly, neither
/// names a resource or a context the reader can no longer read.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_functions_gate_on_their_own(pool: PgPool) {
    let reader = seed_profile(&pool, "reader").await;
    let teammate = seed_profile(&pool, "teammate").await;
    let (team, shared) = seed_team_context(&pool, &[reader, teammate]).await;
    let theirs = create(&backend(&pool, teammate), shared, "theirs").await;
    retitle(&backend(&pool, reader), theirs, "the reader was here").await;

    let resources = |pool: PgPool| async move {
        sqlx::query_scalar::<_, Uuid>("SELECT resource_id FROM resources_changed_by($1)")
            .bind(reader)
            .fetch_all(&pool)
            .await
            .expect("resources_changed_by")
    };
    let contexts = |pool: PgPool| async move {
        sqlx::query_scalar::<_, Uuid>("SELECT context_id FROM contexts_changed_by($1)")
            .bind(reader)
            .fetch_all(&pool)
            .await
            .expect("contexts_changed_by")
    };
    assert_eq!(resources(pool.clone()).await, vec![theirs]);
    assert_eq!(contexts(pool.clone()).await, vec![shared]);

    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(team)
        .bind(reader)
        .execute(&pool)
        .await
        .expect("leave team");

    assert!(resources(pool.clone()).await.is_empty());
    assert!(contexts(pool.clone()).await.is_empty());
}
