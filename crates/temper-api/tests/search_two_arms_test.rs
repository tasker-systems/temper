#![cfg(feature = "test-db")]
//! `POST /api/search` returns two arms that are never combined.
//!
//! Phase 1 step 2/3 of task `019fd25e-95f0-7373-9a6e-0574deea5ab3`, under decision
//! `019fd25a-ef4c-7473-b72e-265a7d36dd65`. Steps 2 and 3 are one change: "returns them unmerged" is
//! not expressible through a body that is a single array, which is what `/api/search` returns today.
//!
//! What this asserts, and why each one is here rather than implied:
//!
//! 1. The body is an OBJECT with `exact` and `wide`, not one ranked list. A single list with a
//!    per-row arm tag would re-create one order and therefore re-invite the sum.
//! 2. Each arm's hits carry that arm's OWN quantity name — `fts_norm`, `vec_norm`. Not a shared
//!    `score`: once both are called the same thing, adding them looks like arithmetic instead of a
//!    category error.
//! 3. No row anywhere carries `combined_score` or `origin`. Those are the sum and its label, and
//!    `origin` never named a producing arm on any path — it was the constant `"unified"`.
//! 4. Each arm carries its own `reason`. One shared reason has to describe both arms at once and
//!    can report `no_match` while the other arm returned hits.
//! 5. Diagnostics are in the BODY. They rode the `x-temper-search-diagnostics` header only because
//!    the body was a bare array, and that reason is gone.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;

async fn post_search(app: &common::TestApp, token: &str, params: Value) -> reqwest::Response {
    app.client
        .post(app.url("/api/search"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&params)
        .send()
        .await
        .expect("search request failed")
}

/// Every key present anywhere in a JSON tree — so an assertion about a field's absence cannot be
/// satisfied merely by it having moved one level down.
fn keys_deep(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                out.push(k.clone());
                keys_deep(child, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|c| keys_deep(c, out)),
        _ => {}
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn search_returns_two_arms_each_with_its_own_quantity_and_no_sum(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("two-arms-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://two-arms-{}", uuid::Uuid::new_v4()),
            "title": "ztmptestword the peregrine stoops",
        }))
        .send()
        .await
        .expect("create resource");
    assert!(resp.status().is_success(), "seed resource must be created");

    let resp = post_search(
        &app,
        &token,
        json!({ "query": "ztmptestword", "limit": 50 }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200, "search must return 200");

    let body: Value = resp.json().await.expect("search JSON");

    // 1. Two named arms, not one ranked list.
    assert!(
        body.is_object(),
        "the body must be an object carrying two arms, not a single ranked array; got {body}"
    );
    let exact = body
        .get("exact")
        .unwrap_or_else(|| panic!("body must carry an `exact` arm; got keys {body:?}"));
    let wide = body
        .get("wide")
        .unwrap_or_else(|| panic!("body must carry a `wide` arm; got keys {body:?}"));

    // 2. Each arm's hits carry that arm's own quantity, under its declared name.
    let exact_hits = exact["hits"]
        .as_array()
        .expect("the exact arm carries `hits`");
    assert!(
        !exact_hits.is_empty(),
        "the seeded resource carries the query term, so the exact arm must match it"
    );
    assert!(
        exact_hits[0].get("fts_norm").is_some(),
        "an exact hit is ordered by `fts_norm` — the name its act declaration already carries"
    );
    assert!(
        wide.get("hits").map(|h| h.is_array()).unwrap_or(false),
        "the wide arm carries `hits`, even when empty"
    );

    // 3. The sum and its label are gone everywhere, at every depth.
    let mut keys = Vec::new();
    keys_deep(&body, &mut keys);
    for banned in ["combined_score", "origin", "graph_score", "results"] {
        assert!(
            !keys.iter().any(|k| k == banned),
            "`{banned}` must not appear anywhere in the response; keys were {keys:?}"
        );
    }

    // 4. Each arm reports its own disposition.
    assert!(
        exact.get("reason").is_some() && wide.get("reason").is_some(),
        "each arm carries its own `reason`; one shared reason can report no_match for a response \
         whose other arm returned hits"
    );

    // 5. Diagnostics live in the body now, not in the header.
    assert!(
        body.get("scope").is_some(),
        "scope diagnostics belong in the body; the header existed only because the body was a \
         bare array"
    );
}

/// A hit's `resource` is byte-identical to the list row for the same resource.
///
/// **The convergence witness for search.** Before this, a hit carried eight hand-inlined identity
/// fields that had drifted from the list projection in every direction at once: it renamed the
/// anchor (`resource_id` vs `id`), collapsed the two homes into one `context` slot, dropped
/// `owner_handle`/`created`/`updated`/`managed_meta` entirely, and invented a `kb_uri` that was
/// `origin_uri` under a second name. An agent reading a hit and a list row had to know all of that.
///
/// Asserting equality of the whole serialized object — rather than field-by-field — is what makes
/// this hold for fields nobody thought to check, including any added later.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn hit_carries_the_same_shape_as_list(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("hit-shape-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://hit-shape-{}", uuid::Uuid::new_v4()),
            "title": "ztmpshapeword the kestrel hovers",
        }))
        .send()
        .await
        .expect("create resource");
    assert!(resp.status().is_success(), "seed resource must be created");
    let created: Value = resp.json().await.expect("create JSON");
    let id = created["id"]
        .as_str()
        .unwrap_or_else(|| panic!("create returns a ResourceView anchored on `id`; got {created}"))
        .to_string();

    let resp = post_search(
        &app,
        &token,
        json!({ "query": "ztmpshapeword", "limit": 50 }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.expect("search JSON");
    let hit = body["exact"]["hits"]
        .as_array()
        .expect("exact hits")
        .iter()
        .find(|h| h["resource"]["id"].as_str() == Some(id.as_str()))
        .unwrap_or_else(|| panic!("the seeded resource must appear in the exact arm; got {body}"));

    let resp = app
        .client
        .get(app.url("/api/resources?limit=50"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list request failed");
    assert_eq!(resp.status().as_u16(), 200, "list must return 200");
    let list: Value = resp.json().await.expect("list JSON");
    let row = list["rows"]
        .as_array()
        .expect("list carries `rows`")
        .iter()
        .find(|r| r["id"].as_str() == Some(id.as_str()))
        .unwrap_or_else(|| panic!("the seeded resource must appear in the list; got {list}"));

    assert_eq!(
        &hit["resource"], row,
        "a hit's `resource` and a list row for the same id are one shape with one set of values"
    );
}

/// The arm's quantity is on the HIT, never on the resource it wraps.
///
/// Frame register `no-cross-act-ranking`: `fts_norm` and `vec_norm` measure different things and
/// are never summed. They sit on the two hit types precisely so that neither can be reached from
/// the shape the two arms SHARE — a quantity on `ResourceView` would appear in a list row, in
/// `show`, and in both arms at once, which is what makes writing `a.score + b.score` look like
/// arithmetic instead of a category error.
///
/// Asserted against the wire body rather than a constructed struct: the wire is where a consumer
/// would find it, and a `#[serde(skip)]` would hide a field this test must still catch.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_quantity_field_exists_on_resource_view(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("no-quantity-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://no-quantity-{}", uuid::Uuid::new_v4()),
            "title": "ztmpquantityword the merlin dives",
        }))
        .send()
        .await
        .expect("create resource");
    assert!(resp.status().is_success(), "seed resource must be created");

    let resp = post_search(
        &app,
        &token,
        json!({ "query": "ztmpquantityword", "limit": 50 }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.expect("search JSON");

    let hits = body["exact"]["hits"].as_array().expect("exact hits");
    assert!(
        !hits.is_empty(),
        "the seeded resource carries the query term, so there must be a hit to inspect: {body}"
    );

    for hit in hits {
        // The hit itself carries its arm's quantity — that half is the point, not an accident.
        assert!(
            hit.get("fts_norm").is_some(),
            "the exact hit carries `fts_norm`: {hit}"
        );

        let resource = hit["resource"]
            .as_object()
            .unwrap_or_else(|| panic!("a hit wraps a `resource` object; got {hit}"));

        // But the resource it wraps carries NEITHER arm's quantity — not its own, not the other's.
        for quantity in ["fts_norm", "vec_norm", "score", "combined_score"] {
            assert!(
                !resource.contains_key(quantity),
                "`{quantity}` is an arm's quantity and belongs on the hit, never on the shared \
                 resource shape: {hit}"
            );
        }
    }

    // And the same for the wide arm, whose own quantity is the other one.
    for hit in body["wide"]["hits"].as_array().expect("wide hits") {
        let resource = hit["resource"]
            .as_object()
            .unwrap_or_else(|| panic!("a hit wraps a `resource` object; got {hit}"));
        for quantity in ["fts_norm", "vec_norm", "score", "combined_score"] {
            assert!(!resource.contains_key(quantity), "{hit}");
        }
    }
}

/// `offset` pages EACH ARM through its own order, and is not silently ignored.
///
/// The arms are incommensurable, so there is no combined sequence for a caller to be at position N
/// of — per-arm is the only reading that means anything. This exists because the first cut of the
/// two-arm read path applied `limit` and dropped `offset` on the floor, which is precisely the
/// "flag that silently does nothing" the same change removed the graph parameters for.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn offset_pages_each_arm_and_is_not_ignored(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("offset-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    for n in 0..3 {
        let resp = app
            .client
            .post(app.url("/api/resources"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&json!({
                "kb_context_id": context_id.to_string(),
                "doc_type": "research",
                "origin_uri": format!("test://offset-{}-{}", n, uuid::Uuid::new_v4()),
                "title": format!("ztmpoffsetword entry number {n}"),
            }))
            .send()
            .await
            .expect("create resource");
        assert!(resp.status().is_success(), "seed {n} must be created");
    }

    async fn page(app: &common::TestApp, token: &str, offset: i64) -> Vec<String> {
        let resp = post_search(
            app,
            token,
            json!({ "query": "ztmpoffsetword", "limit": 2, "offset": offset }),
        )
        .await;
        assert_eq!(resp.status().as_u16(), 200);
        let body: Value = resp.json().await.expect("search JSON");
        body["exact"]["hits"]
            .as_array()
            .expect("exact hits")
            .iter()
            .map(|h| h["resource"]["id"].as_str().unwrap().to_string())
            .collect()
    }

    let first = page(&app, &token, 0).await;
    let second = page(&app, &token, 2).await;

    assert_eq!(first.len(), 2, "limit 2 must yield 2 on the first page");
    assert_eq!(
        second.len(),
        1,
        "three seeded resources, offset 2 leaves exactly one"
    );
    assert!(
        second.iter().all(|id| !first.contains(id)),
        "offset must advance the arm's own order, not re-serve page 1: {first:?} vs {second:?}"
    );
}
