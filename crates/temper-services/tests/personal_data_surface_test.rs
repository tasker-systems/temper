#![cfg(feature = "test-db")]
//! The personal-data surface is DERIVED from the catalog and DECLARED in
//! `scripts/personal-data-surface.txt`; this test is the join between the two.
//!
//! Under goal *"A person can be erased without the ledger losing what happened"*. An erasure path
//! that enumerates its targets from a hand-written list is an erasure path that misses the column
//! somebody added last week and reports success anyway — so the list is not hand-written. Six
//! derivations nominate candidates from `pg_catalog` / `information_schema`; the manifest carries
//! the judgement each candidate needs and no query can supply.
//!
//! **The asymmetry that decides the failure modes.** A candidate with no declaration is the
//! dangerous direction: personal data nobody classified, silently outside the erasure path. A
//! declaration naming a dropped column is merely stale. Both fail here, because a manifest that
//! accumulates dead lines stops being read.
//!
//! **Why `named` is in the derivation set even though it is a heuristic.** Four identifier columns
//! carry no structure to find them by — `kb_team_invitations.invited_email` (an email for someone
//! who may have no profile at all), both `slack_principal_id` columns, and `kb_saml_idp.email_attr`.
//! Structure finds none of them. The heuristic nominates; the manifest adjudicates, and does mark
//! one of the four as `none` — which is the split working, not the split failing.
//!
//! This test does NOT claim the surface is complete. 150 `text` columns are nominated by nothing
//! here, and the manifest says so in its own header. Coverage is never inferred from absence.

use std::collections::{BTreeMap, BTreeSet};

use sqlx::PgPool;

/// The declaration half. Read from the shipped file so the manifest and this test cannot drift.
const MANIFEST: &str = include_str!("../../../scripts/personal-data-surface.txt");

/// The derivation half — six sources, unioned and de-duplicated.
///
/// Held here rather than in a `.sql` file because it is this test's question, not a migration: it
/// is never applied, and a reader debugging a failure wants it beside the assertion.
const DERIVE_CANDIDATES: &str = r#"
WITH principal(t) AS (VALUES ('kb_profiles'),('kb_entities'),('kb_profile_auth_links')),
fk AS (
  SELECT c.conrelid::regclass::text tbl, a.attname col
  FROM pg_constraint c
  JOIN unnest(c.conkey) k(attnum) ON true
  JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.attnum
  WHERE c.contype='f' AND c.connamespace='public'::regnamespace
    AND c.confrelid::regclass::text IN ('kb_profiles','kb_entities')),
poly AS (
  SELECT DISTINCT c.conrelid::regclass::text tbl, a.attname col
  FROM pg_constraint c
  JOIN unnest(c.conkey) k(attnum) ON true
  JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.attnum
  WHERE c.contype='c' AND c.connamespace='public'::regnamespace
    AND a.attname LIKE '%\_table' AND pg_get_constraintdef(c.oid) LIKE '%kb_profiles%'),
poly_pair AS (
  SELECT tbl, col FROM poly
  UNION ALL SELECT tbl, regexp_replace(col,'_table$','_id') FROM poly),
prin AS (
  SELECT c.table_name tbl, c.column_name col FROM information_schema.columns c
  JOIN principal p ON p.t = c.table_name WHERE c.table_schema='public'),
vecs AS (
  SELECT table_name tbl, column_name col FROM information_schema.columns
  WHERE table_schema='public' AND udt_name IN ('vector','tsvector')),
js AS (
  SELECT c.table_name tbl, c.column_name col FROM information_schema.columns c
  JOIN information_schema.tables t ON t.table_name=c.table_name
   AND t.table_schema='public' AND t.table_type='BASE TABLE'
  WHERE c.table_schema='public' AND c.data_type='jsonb'),
denorm AS (VALUES ('kb_teams','slug'),('kb_teams','name')),
named AS (
  SELECT c.table_name tbl, c.column_name col FROM information_schema.columns c
  JOIN information_schema.tables t ON t.table_name=c.table_name
   AND t.table_schema='public' AND t.table_type='BASE TABLE'
  WHERE c.table_schema='public' AND c.data_type IN ('text','character varying')
    AND c.column_name ~ '(email|handle|display_name|principal_id|_user_id|name_id|subject|actor)')
SELECT DISTINCT tbl||'.'||col FROM (
  SELECT * FROM fk        UNION ALL SELECT * FROM poly_pair UNION ALL SELECT * FROM prin
  UNION ALL SELECT * FROM vecs UNION ALL SELECT * FROM js   UNION ALL SELECT * FROM denorm
  UNION ALL SELECT * FROM named
) u ORDER BY 1
"#;

const CLASSES: &[&str] = &[
    "identifier",
    "reference",
    "content",
    "derived",
    "incidental",
    "discriminator",
    "none",
];
const CEILINGS: &[&str] = &["full", "recompute", "pseudonym", "none", "n-a"];

/// `table.column` → (class, ceiling). Panics with the offending line, since a malformed manifest
/// line would otherwise silently drop a declaration and read as an undeclared candidate.
fn declarations() -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for (lineno, raw) in MANIFEST.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        assert!(
            cols.len() >= 3,
            "personal-data-surface.txt:{}: expected `table.column | class | ceiling | note`, got {raw:?}",
            lineno + 1
        );
        let (key, class, ceiling) = (cols[0].to_string(), cols[1], cols[2]);
        assert!(
            CLASSES.contains(&class),
            "personal-data-surface.txt:{}: unknown class {class:?} (known: {CLASSES:?})",
            lineno + 1
        );
        assert!(
            CEILINGS.contains(&ceiling),
            "personal-data-surface.txt:{}: unknown ceiling {ceiling:?} (known: {CEILINGS:?})",
            lineno + 1
        );
        assert!(
            out.insert(key.clone(), (class.to_string(), ceiling.to_string()))
                .is_none(),
            "personal-data-surface.txt:{}: {key} declared twice",
            lineno + 1
        );
    }
    out
}

async fn derived(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar::<_, String>(DERIVE_CANDIDATES)
        .fetch_all(pool)
        .await
        .expect("derive the personal-data candidate set")
        .into_iter()
        .collect()
}

/// FAILS IF: a column that can hold or reference personal data exists with nobody having said what
/// it is. That is the whole point — a new `profile_id`, a new jsonb column, or a new polymorphic
/// owner lands here the day it is added, not the day an erasure request arrives.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_derived_candidate_is_declared(pool: PgPool) {
    let declared: BTreeSet<String> = declarations().into_keys().collect();
    let candidates = derived(&pool).await;
    let undeclared: Vec<&String> = candidates.difference(&declared).collect();

    assert!(
        undeclared.is_empty(),
        "these columns can hold or reference personal data and are not declared in \
         scripts/personal-data-surface.txt.\n\
         Add a line per column — `table.column | class | ceiling | note` — deciding what it holds \
         and how far erasure reaches it. Declaring one `none | n-a` is a fine answer; leaving it \
         out is not, because the erasure path enumerates from that file.\n\
         Undeclared: {undeclared:#?}"
    );
}

/// FAILS IF: the manifest names a column that no longer exists. Stale lines are how a manifest
/// stops being read, and a reader who cannot trust it will rebuild the list by hand — which is the
/// failure this whole mechanism exists to prevent.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_declaration_still_names_a_live_candidate(pool: PgPool) {
    let live = derived(&pool).await;
    let stale: Vec<String> = declarations()
        .into_keys()
        .filter(|k| !live.contains(k))
        .collect();

    assert!(
        stale.is_empty(),
        "scripts/personal-data-surface.txt declares columns the catalog no longer nominates. \
         Either the column was dropped (delete the line) or a derivation stopped finding it \
         (which is the more serious reading — check that first).\n\
         Stale: {stale:#?}"
    );
}

/// FAILS IF: a polymorphic principal reference exists whose `*_table` discriminator has no
/// matching `*_id` sibling.
///
/// The `_table` → `_id` rewrite is a NAMING CONVENTION, and derivation `poly` is built on it: a
/// pair that breaks the convention is not partly found, it is invisible — the discriminator is
/// nominated and the column actually holding the profile id is not. So the convention is asserted
/// rather than trusted. A future pair spelled otherwise fails here, where the message can say what
/// to do, instead of silently leaving a profile reference outside the surface.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_polymorphic_discriminator_has_a_resolvable_id_sibling(pool: PgPool) {
    let orphans: Vec<(String, String)> = sqlx::query_as(
        r#"
        WITH poly AS (
          SELECT DISTINCT c.conrelid::regclass::text tbl, a.attname col
          FROM pg_constraint c
          JOIN unnest(c.conkey) k(attnum) ON true
          JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.attnum
          WHERE c.contype='c' AND c.connamespace='public'::regnamespace
            AND a.attname LIKE '%\_table'
            AND pg_get_constraintdef(c.oid) LIKE '%kb_profiles%')
        SELECT p.tbl, p.col FROM poly p
        WHERE NOT EXISTS (
          SELECT 1 FROM information_schema.columns ic
           WHERE ic.table_schema='public' AND ic.table_name = p.tbl
             AND ic.column_name = regexp_replace(p.col,'_table$','_id'))
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("probe polymorphic discriminators");

    assert!(
        orphans.is_empty(),
        "these polymorphic discriminators reference kb_profiles but have no `*_id` sibling, so the \
         column carrying the profile id is NOT nominated by any derivation and is invisible to the \
         erasure path. Either rename the pair to the `<prefix>_table` / `<prefix>_id` convention, \
         or teach derivation `poly` in this file about the new spelling.\n\
         Orphaned: {orphans:#?}"
    );
}

/// FAILS IF: a `pseudonym` ceiling is claimed for something that is not a reference.
///
/// `pseudonym` means *"this stops identifying anyone once kb_profiles is tombstoned"* — a claim
/// that is only true of an opaque reference. Attaching it to an `identifier` would assert that
/// erasing the profile row neutralises a stored email, which is false, and false in the direction
/// that leaves personal data in place while the manifest reads as covered.
#[test]
fn a_pseudonym_ceiling_is_only_claimed_for_references() {
    let bad: Vec<(String, String)> = declarations()
        .into_iter()
        .filter(|(_, (class, ceiling))| ceiling == "pseudonym" && class != "reference")
        .map(|(k, (class, _))| (k, class))
        .collect();

    assert!(
        bad.is_empty(),
        "`pseudonym` says a value stops identifying anyone once the profile is tombstoned. That is \
         only true of an opaque reference — a stored identifier survives the tombstone unchanged.\n\
         Mis-declared: {bad:#?}"
    );
}
