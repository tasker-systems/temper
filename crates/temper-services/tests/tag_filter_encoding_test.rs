//! `filtered_visible_page`'s tag filter — the THIRD encoding of the `tags` convention, and the one
//! that is Rust rather than SQL.
//!
//! ## Why this file exists at all
//!
//! `[found — 2026-08-15]` The `--tag` list filter had **no test anywhere**. Not in
//! `crates/temper-api/tests`, not in `tests/e2e`, not beside `filtered_visible_page` itself. So its
//! predicate — which has shipped since the list endpoint did, and which carried its own inline
//! essay about case folding and bare strings — could be changed in either direction, or deleted
//! outright, with every suite still green.
//!
//! That is precisely the trap task `01a00502-a774-7001-b5b2-0ce462158f1c` names: *"a change that
//! edits only `migrations/` leaves it live, the three encodings do not collapse, and every SQL-side
//! test still passes."* The absence of a witness is what would have made that silent. This file is
//! the witness, and it is added **before** the change it guards rather than after.
//!
//! ## What is under test
//!
//! `[decided — 2026-08-15, Pete]` §7 of the property-conventions design: **a bare-string `tags`
//! value is normalized at write, and it is ONE tag.** `tags: "ci auth"` is one tag named `ci auth`,
//! not two named `ci` and `auth`.
//!
//! The control arm is half the test. Without `an_array_tag_filter_still_matches_case_insensitively`
//! below, a `filtered_visible_page` that returned nothing for any reason at all — a broken bind, a
//! predicate that never matches, a filter accidentally inverted — would satisfy the bare-string
//! assertion vacuously. The two arms fail in opposite directions, so no single defect satisfies
//! both.

#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::ResourceListParams;

/// Seed a substrate profile + a profile-owned `temper` context. Mirrors the inlined fixture in
/// `list_page_query_count_test.rs` / `open_meta_roundtrip_test.rs`.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
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
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// Create one resource carrying `open_meta.tags` exactly as given, and return its id.
///
/// The `tags` value rides through the real create path — `DbBackend::create_resource` →
/// `property_asserted` → `_project_property_asserted` — so a test that writes a bare string
/// witnesses whatever that path does to it, rather than whatever a direct `INSERT INTO
/// kb_properties` would have preserved.
async fn mk(
    backend: &DbBackend,
    context: uuid::Uuid,
    slug: &str,
    tags: serde_json::Value,
) -> uuid::Uuid {
    let created = backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: slug.to_string(),
            doctype: "note".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: slug.to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: Some(serde_json::json!({ "tags": tags })),
            goal: None,
            origin_uri: Some(format!("test://{slug}")),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create");
    created.value.id.into()
}

/// The ids `filtered_visible_page` returns for one `--tag` filter.
async fn list_by_tag(pool: &PgPool, principal: uuid::Uuid, tag: &str) -> Vec<uuid::Uuid> {
    substrate_read::list_select(
        pool,
        ProfileId::from(principal),
        ResourceListParams {
            tags: Some(tag.to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("list")
    .rows
    .into_iter()
    .map(|r| r.id.into())
    .collect()
}

/// A bare-string `tags` value is ONE tag. Filtering for a whitespace-separated fragment of it does
/// not match; filtering for the whole string does.
///
/// **This inverts the shipped behaviour.** Until `20260815000030` a bare string was split on
/// whitespace (`regexp_split_to_array(trim(...), '\s+')`), so `ci` matched `"ci auth"`. §7 rules
/// that it must not: the split infers a list the caller never wrote, and the FTS agreement it was
/// built to preserve does not exist — FTS delegates to a tokenizer that splits differently
/// (`to_tsvector('english','ci-auth deploy')` yields `ci`, `auth`, `ci-auth`, `deploy`;
/// `regexp_split_to_array` yields `{ci-auth, deploy}`).
///
/// It is now the WRITE path that decides this: the resource below is created with a bare string and
/// stored as `["ci auth"]`. What holds the READ path to the same answer is
/// `a_legacy_bare_string_row_reads_as_one_tag_rather_than_its_fragments` below — see its comment for
/// why this arm alone cannot.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_bare_string_tag_is_one_tag_and_does_not_split(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "bare-tag@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let bare = mk(&backend, context, "bare", serde_json::json!("ci auth")).await;

    let by_fragment = list_by_tag(&pool, profile, "ci").await;
    assert!(
        !by_fragment.contains(&bare),
        "`ci` is a whitespace-separated FRAGMENT of the single tag `ci auth`, not a tag the caller \
         wrote. Matching it would infer a list from whitespace — the split §7 retires. Got {:?}",
        by_fragment
    );

    let by_whole = list_by_tag(&pool, profile, "ci auth").await;
    assert!(
        by_whole.contains(&bare),
        "the bare string is a tag and must be matchable AS ONE — a value that matches neither its \
         fragments nor itself is unreachable, which is worse than the split. Got {:?}",
        by_whole
    );
}

/// A bare-string row that PREDATES the write-time normalization still reads as one tag.
///
/// ## This is the only arm that holds the read path accountable, and that is why it exists
///
/// `[found — 2026-08-15]` `a_bare_string_tag_is_one_tag_and_does_not_split` above went green the
/// moment the projectors started normalizing — **without `filtered_visible_page` being touched at
/// all**. That is not a flaw in it; it is the shape of the change. Once nothing can STORE a bare
/// string, the read-side split becomes unreachable, and an unreachable branch is invisible to every
/// behavioural test. So deleting it would have been a claim about deadness dressed as a claim about
/// behaviour.
///
/// This arm restores the accountability by writing the shape the projector can no longer produce:
/// it stores the tags row through the real create path and then rewrites the stored VALUE in place,
/// which is exactly the row a deployment that predates `20260815000030` already holds. `[measured on
/// prod — 2026-08-15]` there are zero such rows, so this is a witness for a population of none —
/// stated in that direction on purpose, because it is the reason the change was cheap, not evidence
/// that it was unnecessary.
///
/// It fails if the whitespace split is restored here, and it is the bite probe for that guard.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_legacy_bare_string_row_reads_as_one_tag_rather_than_its_fragments(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "legacy-tag@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let legacy = mk(
        &backend,
        context,
        "legacy",
        serde_json::json!(["placeholder"]),
    )
    .await;

    // Rewrite the stored value to the shape the projector would now normalize away, leaving every
    // key, owner and event reference exactly as the real write path built them.
    let rewritten = sqlx::query(
        "UPDATE kb_properties SET property_value = '\"ci auth\"'::jsonb \
          WHERE owner_table='kb_resources' AND owner_id=$1 \
            AND property_key='tags' AND NOT is_folded",
    )
    .bind(legacy)
    .execute(&pool)
    .await
    .expect("rewrite stored tags value")
    .rows_affected();
    assert_eq!(
        rewritten, 1,
        "precondition: exactly one live `tags` row to rewrite — a zero here means this probe \
         asserted nothing"
    );

    let by_fragment = list_by_tag(&pool, profile, "ci").await;
    assert!(
        !by_fragment.contains(&legacy),
        "a stored bare string is ONE tag at read time too. Matching `ci` means the read path is \
         still splitting on whitespace — the third encoding survived the collapse. Got {:?}",
        by_fragment
    );

    let by_whole = list_by_tag(&pool, profile, "ci auth").await;
    assert!(
        by_whole.contains(&legacy),
        "and the legacy row must stay reachable as itself — normalization at write must not orphan \
         the rows written before it. Got {:?}",
        by_whole
    );
}

/// The control. Array-shaped tags keep matching, case-folded on both sides.
///
/// Without this arm the assertion above passes for a `filtered_visible_page` that matches nothing
/// at all. With it, the two arms fail in opposite directions and no single defect satisfies both.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_array_tag_filter_still_matches_case_insensitively(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "array-tag@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let arrayed = mk(
        &backend,
        context,
        "arrayed",
        serde_json::json!(["CI", "auth"]),
    )
    .await;
    let other = mk(&backend, context, "other", serde_json::json!(["deploy"])).await;

    let folded = list_by_tag(&pool, profile, "ci").await;
    assert!(
        folded.contains(&arrayed),
        "the bind folds in Rust and the row folds with `lower(t)`, so `ci` matches the stored `CI`"
    );
    assert!(
        !folded.contains(&other),
        "a resource whose tags do not contain the filter must be excluded — otherwise this control \
         proves nothing about narrowing"
    );

    let both = list_by_tag(&pool, profile, "ci,auth").await;
    assert!(
        both.contains(&arrayed),
        "containment IS the AND semantics: every listed tag must be present"
    );

    let unsatisfiable = list_by_tag(&pool, profile, "ci,nonesuch").await;
    assert!(
        !unsatisfiable.contains(&arrayed),
        "adding a tag the resource lacks must narrow it away, not widen the page"
    );
}
