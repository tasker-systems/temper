#![cfg(feature = "artifact-tests")]
//! `kb_contexts` is an owner-scoped, slugged namespace (WS6 §2 amendment 2026-06-13): a slug is unique
//! only WITHIN one owner, so two owners may each hold a same-named/same-slug context, while a duplicate
//! slug under one owner is rejected by `UNIQUE(owner_table, owner_id, slug)`. Isolated ephemeral DB via
//! `temper_substrate::MIGRATOR` (`#[sqlx::test]` provisions a fresh `public`-schema database per test).

mod common;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn contexts_are_owner_scoped_not_globally_unique(pool: sqlx::PgPool) {
    // Two owners may each have a 'general' context (the bug the global name UNIQUE caused).
    let p1 = common::insert_profile(&pool, "alice").await;
    let p2 = common::insert_profile(&pool, "bob").await;
    common::insert_context(&pool, "kb_profiles", p1, "general", "general")
        .await
        .expect("first owner's context inserts");
    common::insert_context(&pool, "kb_profiles", p2, "general", "general")
        .await
        .expect("same name+slug under a DIFFERENT owner must be allowed");
    // Duplicate slug under the SAME owner is rejected.
    let dup = common::insert_context(&pool, "kb_profiles", p1, "general", "Another").await;
    assert!(
        dup.is_err(),
        "duplicate slug within one owner must violate UNIQUE(owner_table,owner_id,slug)"
    );
}
