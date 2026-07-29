#![cfg(feature = "test-db")]

//! End-to-end coverage for the `tags` filter on resource listing.
//!
//! Until 2026-07-29 tags could be written and displayed but never **asked for**: there was no
//! tag flag on `resource list`, none on `search`, and no field for it in `ResourceListParams`.
//! That mattered because tags had just been adopted (decision `019fa849-db76-7bc3-919e-74889e2b1729`)
//! as the carrier for the *program axis* — recurring work that has no end state and so cannot be
//! a goal. An axis you cannot enumerate is one whose coverage gets inferred from absence.
//!
//! The filter rides `ResourceListParams.tags` (a CSV, because the list endpoint is a GET) into
//! `filtered_visible_page`, which folds it to a lowercased `text[]` and applies array containment
//! against the row's own lowercased tag set.
//!
//! **Three decisions are under test here, and each fails differently.** They are separate tests
//! rather than one, because a suite that only proved "the filter returns something" would go green
//! against a build that got any one of them backwards:
//!
//! - **AND, not OR** (`filters_by_tag_and_narrows`). Repeating a tag must *narrow*. An OR
//!   implementation passes every single-tag assertion and fails only on the two-tag case.
//! - **Case-insensitive** (`tag_match_is_case_insensitive`). Production carries six pairs that
//!   differ only by case (`ci`/`CI`, `authn`/`AuthN`, …), so this is a live distinction rather
//!   than a hypothetical.
//! - **Not doc-type-scoped** (`filters_across_doc_types_without_a_type`). This is the one that
//!   actually delivers the clause. `stage` is task-only and `status` is goal-only, so the
//!   tempting move was to scope tags the same way — but 14 doc types carry tags, and a filter
//!   requiring a doc type turns "enumerate this axis" back into a client-side scan.
//!
//! The negative face is asserted throughout: an untagged resource, and a resource carrying only
//! *some* of the requested tags, must never appear in a filtered result.

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_workflow::types::resource::ResourceListParams;

/// Seed a resource of a given doc type carrying a given tag set, via the API client
/// (cloud-only; no vault files written). Tags ride `open_meta.tags`, which is where the
/// write surface puts them and where the filter reads them from.
async fn seed_tagged(
    client: &temper_client::TemperClient,
    context: &str,
    doc_type: &str,
    slug: &str,
    title: &str,
    tags: &[&str],
) {
    let mut open = serde_json::Map::new();
    open.insert("tags".to_string(), serde_json::json!(tags));

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("kb://{context}/{doc_type}/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: doc_type.to_string(),
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: Some(serde_json::Value::Object(open)),
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    client
        .ingest()
        .create(&payload)
        .await
        .expect("seed tagged resource via client");
}

/// `ResourceRow.slug` is `None` in the substrate (`temper-slug` is a §7-Die key, not
/// persisted); the seed encodes the slug as the last `origin_uri` segment.
fn slugs(rows: &[temper_workflow::types::resource::ResourceRow]) -> Vec<&str> {
    let mut s: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.origin_uri.rsplit('/').next())
        .collect();
    s.sort_unstable();
    s
}

/// Repeating a tag NARROWS: `tags=ci,security` returns only the resource carrying both.
///
/// The four-seeded shape is the point. A resource tagged only `ci` and one tagged only
/// `security` are each returned by the corresponding single-tag query and each excluded by the
/// two-tag one. **An OR implementation passes both single-tag assertions and returns three rows
/// for the pair** — which is exactly why the two-tag case is asserted by identity and not by
/// count alone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_list_filters_by_tag_and_narrows(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("tagfilter", None)
        .await
        .expect("create tagfilter context");

    seed_tagged(
        &app.client,
        "tagfilter",
        "task",
        "both",
        "Carries Both",
        &["ci", "security"],
    )
    .await;
    seed_tagged(
        &app.client,
        "tagfilter",
        "task",
        "ci-only",
        "CI Only",
        &["ci"],
    )
    .await;
    seed_tagged(
        &app.client,
        "tagfilter",
        "task",
        "sec-only",
        "Security Only",
        &["security"],
    )
    .await;
    seed_tagged(
        &app.client,
        "tagfilter",
        "task",
        "untagged",
        "Untagged",
        &[],
    )
    .await;

    let list_tags = |tags: Option<&'static str>| {
        let ctx_id = uuid::Uuid::from(context.id).to_string();
        let client = &app.client;
        async move {
            client
                .resources()
                .list(&ResourceListParams {
                    context_ref: Some(ctx_id),
                    doc_type_name: Some("task".to_string()),
                    tags: tags.map(str::to_string),
                    limit: Some(50),
                    ..Default::default()
                })
                .await
        }
    };

    let ci = list_tags(Some("ci")).await.expect("list tags=ci failed");
    assert_eq!(
        slugs(&ci.rows),
        vec!["both", "ci-only"],
        "tags=ci must return every resource carrying ci and nothing else; got {:?}",
        slugs(&ci.rows)
    );

    let security = list_tags(Some("security"))
        .await
        .expect("list tags=security failed");
    assert_eq!(
        slugs(&security.rows),
        vec!["both", "sec-only"],
        "tags=security must return every resource carrying security; got {:?}",
        slugs(&security.rows)
    );

    // The decision under test. OR would return all three tagged rows here.
    let both = list_tags(Some("ci,security"))
        .await
        .expect("list tags=ci,security failed");
    assert_eq!(
        slugs(&both.rows),
        vec!["both"],
        "multiple tags must AND (narrow), not OR (widen) — only the resource carrying BOTH \
         may be returned; got {:?}",
        slugs(&both.rows)
    );

    // The negative face: `no-filter-returns-what-it-was-asked-to-exclude`. The untagged
    // resource must be absent from every filtered result above, and present with no filter.
    for (label, rows) in [
        ("ci", &ci.rows),
        ("security", &security.rows),
        ("both", &both.rows),
    ] {
        assert!(
            !slugs(rows).contains(&"untagged"),
            "the untagged resource must never appear under tags={label}; got {:?}",
            slugs(rows)
        );
    }

    let all = list_tags(None)
        .await
        .expect("list with no tag filter failed");
    assert_eq!(
        all.rows.len(),
        4,
        "no tag filter must return all four seeded resources; got {}",
        all.rows.len()
    );

    // `total` must be the count UNDER the filter, not over the unfiltered set — a total
    // computed unfiltered is how a broken filter passes for a working one.
    assert_eq!(
        both.total, 1,
        "total must be the count under the filter (1), not the unfiltered set (4); got {}",
        both.total
    );
}

/// Tag matching is case-insensitive, and the fold happens server-side so every door agrees.
///
/// Production carries six tag pairs differing only by case. Asserting BOTH directions matters:
/// a fold applied to only one side (the bind, say, but not the row) passes whichever direction
/// happens to match the seeded casing and fails the other.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_list_tag_match_is_case_insensitive(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("tagcase", None)
        .await
        .expect("create tagcase context");

    // One resource tagged lowercase, one tagged uppercase — the shape production actually has.
    seed_tagged(&app.client, "tagcase", "task", "lower", "Lower", &["ci"]).await;
    seed_tagged(&app.client, "tagcase", "task", "upper", "Upper", &["CI"]).await;

    let list_tags = |tags: &'static str| {
        let ctx_id = uuid::Uuid::from(context.id).to_string();
        let client = &app.client;
        async move {
            client
                .resources()
                .list(&ResourceListParams {
                    context_ref: Some(ctx_id),
                    doc_type_name: Some("task".to_string()),
                    tags: Some(tags.to_string()),
                    limit: Some(50),
                    ..Default::default()
                })
                .await
        }
    };

    for query in ["ci", "CI", "Ci"] {
        let found = list_tags(query).await.expect("list by tag failed");
        assert_eq!(
            slugs(&found.rows),
            vec!["lower", "upper"],
            "tags={query} must match both the `ci`- and `CI`-tagged resources — the fold \
             applies to the query AND the stored tag; got {:?}",
            slugs(&found.rows)
        );
    }

    // Negative control, and it is load-bearing. Every assertion above returns BOTH seeded rows,
    // so all of them also pass against a build where the tag filter is dropped entirely — the
    // case fold and the filter's existence are separate properties, and this test would
    // otherwise only cover the first. Verified by neutering the bind: the assertions above
    // stayed green, this one does not.
    let none = list_tags("cd").await.expect("list by absent tag failed");
    assert!(
        none.rows.is_empty(),
        "a tag nothing carries must return NOTHING — a filter that returns rows here is not \
         filtering at all, whatever it does about case; got {:?}",
        slugs(&none.rows)
    );
}

/// A tag filter with NO doc type returns matching resources of every doc type.
///
/// This is the test that delivers the clause. `stage` is task-only and `status` is goal-only,
/// each refusing a mismatched `doc_type_name`; copying that shape for tags would have been the
/// natural move and would have been wrong. Tags span 14 doc types in production, so a filter
/// that requires a doc type means enumerating one axis takes one call per type plus prior
/// knowledge of which types exist — a client-side scan wearing a filter's clothes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_list_filters_across_doc_types_without_a_type(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("tagcrosstype", None)
        .await
        .expect("create tagcrosstype context");

    // The same tag on three different doc types, plus a decoy carrying a different tag.
    seed_tagged(
        &app.client,
        "tagcrosstype",
        "task",
        "t-1",
        "Tagged Task",
        &["lifecycle"],
    )
    .await;
    seed_tagged(
        &app.client,
        "tagcrosstype",
        "research",
        "r-1",
        "Tagged Research",
        &["lifecycle"],
    )
    .await;
    seed_tagged(
        &app.client,
        "tagcrosstype",
        "decision",
        "d-1",
        "Tagged Decision",
        &["lifecycle"],
    )
    .await;
    seed_tagged(
        &app.client,
        "tagcrosstype",
        "task",
        "other",
        "Other Axis",
        &["perf"],
    )
    .await;

    let found = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(uuid::Uuid::from(context.id).to_string()),
            // Deliberately NO doc_type_name — this is the whole point.
            tags: Some("lifecycle".to_string()),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("list by tag with no doc type failed");

    assert_eq!(
        slugs(&found.rows),
        vec!["d-1", "r-1", "t-1"],
        "a tag filter with no doc type must return matching resources of EVERY doc type — \
         that is what makes the axis enumerable in one call; got {:?}",
        slugs(&found.rows)
    );

    // Three distinct doc types actually came back, not three rows that happen to be tasks.
    let mut types: Vec<&str> = found
        .rows
        .iter()
        .map(|r| r.doc_type_name.as_str())
        .collect();
    types.sort_unstable();
    types.dedup();
    assert_eq!(
        types,
        vec!["decision", "research", "task"],
        "the result must span the three seeded doc types; got {types:?}"
    );

    assert!(
        !slugs(&found.rows).contains(&"other"),
        "a resource on a different axis must not be returned; got {:?}",
        slugs(&found.rows)
    );
}
