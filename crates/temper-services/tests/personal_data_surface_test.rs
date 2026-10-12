#![cfg(feature = "test-db")]
//! The personal-data surface is DERIVED from the catalog and DECLARED in
//! `policy/erasure/personal-data.toml`; this test is the join between the two.
//!
//! Under goal *"A person can be erased without the ledger losing what happened"*. An erasure path
//! that enumerates its targets from a hand-written list is an erasure path that misses the column
//! somebody added last week and reports success anyway — so the list is not hand-written. Seven
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
//! **Why `bytes` exists.** Every other derivation keys on column TYPE or constraint shape, and
//! none of them sees BYTEA: a raw-binary column is invisible to fk/poly/prin/vec/json/named alike.
//! `kb_blob_upload_segments.bytes` (review F3) staged raw user bytes undeclared precisely because
//! the test structurally could not fail on it. Binary content is personal content the moment a
//! person uploads it, so every bytea column nominates.
//!
//! This test does NOT claim the surface is complete. The `text` columns nominated by nothing here
//! are declared in `policy/erasure/sensitivity-scan.toml`, and `sensitivity_scan_surface_test.rs`
//! asserts the two manifests partition every text/varchar column. That makes the remainder
//! declared, not detected. Coverage is never inferred from absence.

mod erasure_binding;
mod person_act_binding;

use std::collections::{BTreeMap, BTreeSet};

use erasure_binding::{deletes_from, tokens};
use person_act_binding::assigns_erasure;
use sqlx::PgPool;
use temper_core::erasure_policy::{Achieved, Ceiling, Claim, Class, ColumnKey, PersonalColumn};
use temper_services::erasure_policy::in_force;

/// The derivation half — seven sources, unioned and de-duplicated.
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
denorm AS (VALUES ('kb_teams','slug'),('kb_teams','name'),('kb_teams','personal_of')),
bytes AS (
  SELECT c.table_name tbl, c.column_name col FROM information_schema.columns c
  JOIN information_schema.tables t ON t.table_name=c.table_name
   AND t.table_schema='public' AND t.table_type='BASE TABLE'
  WHERE c.table_schema='public' AND c.data_type='bytea'),
named AS (
  SELECT c.table_name tbl, c.column_name col FROM information_schema.columns c
  JOIN information_schema.tables t ON t.table_name=c.table_name
   AND t.table_schema='public' AND t.table_type='BASE TABLE'
  WHERE c.table_schema='public' AND c.data_type IN ('text','character varying')
    AND c.column_name ~ '(email|handle|display_name|principal_id|_user_id|name_id|subject|actor)')
SELECT DISTINCT tbl||'.'||col FROM (
  SELECT * FROM fk        UNION ALL SELECT * FROM poly_pair UNION ALL SELECT * FROM prin
  UNION ALL SELECT * FROM vecs UNION ALL SELECT * FROM js   UNION ALL SELECT * FROM denorm
  UNION ALL SELECT * FROM bytes UNION ALL SELECT * FROM named
) u ORDER BY 1
"#;

/// `table.column` → its declaration, from the policy in force. The parser
/// (`temper_core::erasure_policy::parse`) refuses an unknown class, ceiling, arm or claim, and a
/// column declared twice, so a malformed entry fails the parse rather than dropping a declaration.
fn declarations() -> &'static BTreeMap<ColumnKey, PersonalColumn> {
    &in_force().personal_data
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
    let declared: BTreeSet<String> = declarations().keys().map(ColumnKey::to_string).collect();
    let candidates = derived(&pool).await;
    let undeclared: Vec<&String> = candidates.difference(&declared).collect();

    assert!(
        undeclared.is_empty(),
        "these columns can hold or reference personal data and are not declared in \
         policy/erasure/personal-data.toml.\n\
         Add an entry per column under `[columns]` — `\"table.column\" = {{ class, ceiling, \
         achieved, note }}` — deciding what it holds and how far erasure reaches it. Declaring one \
         `class = \"none\", ceiling = \"n-a\"` is a fine answer; leaving it out is not, because \
         the erasure path enumerates from that file.\n\
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
        .keys()
        .map(ColumnKey::to_string)
        .filter(|k| !live.contains(k))
        .collect();

    assert!(
        stale.is_empty(),
        "policy/erasure/personal-data.toml declares columns the catalog no longer nominates. \
         Either the column was dropped (delete the entry) or a derivation stopped finding it \
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
    let bad: Vec<(String, Class)> = declarations()
        .iter()
        .filter(|(_, d)| d.ceiling == Ceiling::Pseudonym && d.class != Class::Reference)
        .map(|(k, d)| (k.to_string(), d.class))
        .collect();

    assert!(
        bad.is_empty(),
        "`pseudonym` says a value stops identifying anyone once the profile is tombstoned. That is \
         only true of an opaque reference — a stored identifier survives the tombstone unchanged.\n\
         Mis-declared: {bad:#?}"
    );
}

// ── The achieved half: what the person act reaches, bound to its live body (R12) ───────────────
//
// The ceiling says how far erasure could reach; nothing said what the act achieves, and the
// ceiling was read as that (2026-10-10: 9 of 27 `full` lines fully reached). Each `full` line now
// names what reaches it or its remainder, and each claim is bound here to a statement in the live
// act. Like the resource fence, the binding reads statements: not their rows, not the branches
// that run them, and not which arm a statement sits in (the manifest's header).

/// The person act's entry: it runs the resource erasures, then the redaction.
const EXECUTE_FN: &str = "principal_erasure_execute";

/// The person act's redaction: the body `erased`, `deleted` and `cascaded` claims bind to.
const REDACTION_FN: &str = "_erasure_apply_redaction";

/// What a `resource-erasure:` claim runs through, to the body the resource fence binds.
const RESOURCE_EXECUTE_FN: &str = "resource_erasure_execute";
const RESOURCE_REDACTION_FN: &str = "_resource_erasure_apply_redaction";

/// The live act, tokenized, and the catalog's cascading foreign keys as (child, parent).
struct Act {
    execute: Vec<String>,
    redaction: Vec<String>,
    resource_execute: Vec<String>,
    cascades: BTreeSet<(String, String)>,
}

async fn function_body(pool: &PgPool, name: &str) -> String {
    sqlx::query_scalar::<_, String>(&format!("SELECT pg_get_functiondef('{name}'::regproc)"))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("read {name}'s live definition: {e}"))
}

async fn act(pool: &PgPool) -> Act {
    let cascades = sqlx::query_as::<_, (String, String)>(
        "SELECT conrelid::regclass::text, confrelid::regclass::text FROM pg_constraint
          WHERE contype = 'f' AND confdeltype = 'c' AND connamespace = 'public'::regnamespace",
    )
    .fetch_all(pool)
    .await
    .expect("read the cascading foreign keys")
    .into_iter()
    .collect();
    Act {
        execute: tokens(&function_body(pool, EXECUTE_FN).await),
        redaction: tokens(&function_body(pool, REDACTION_FN).await),
        resource_execute: tokens(&function_body(pool, RESOURCE_EXECUTE_FN).await),
        cascades,
    }
}

/// Every `table.column` declared `handled` in `policy/erasure/resource-erasure.toml` `[tables]`:
/// which columns resource erasure handles. Its own test binds each `handled` entry to
/// `_resource_erasure_apply_redaction`; this test reuses that binding rather than repeating it.
fn resource_handled() -> BTreeSet<&'static str> {
    in_force()
        .resource_erasure
        .handled()
        .map(ColumnKey::as_str)
        .collect()
}

fn calls(t: &[String], function: &str) -> bool {
    t.windows(2).any(|w| w[0] == function && w[1] == "(")
}

/// Whether the live act does what `claim` says for `key`.
fn binds(act: &Act, handled: &BTreeSet<&str>, key: &ColumnKey, claim: &Claim) -> bool {
    let table = key.table();
    let redaction_runs = calls(&act.execute, REDACTION_FN);
    match claim {
        Claim::Erased { .. } => redaction_runs && assigns_erasure(&act.redaction, key.as_str()),
        Claim::Deleted { .. } => redaction_runs && deletes_from(&act.redaction, table),
        Claim::Cascaded { parent, .. } => {
            redaction_runs
                && deletes_from(&act.redaction, parent)
                && act.cascades.contains(&(table.to_string(), parent.clone()))
        }
        Claim::ResourceErasure => {
            calls(&act.execute, RESOURCE_EXECUTE_FN)
                && calls(&act.resource_execute, RESOURCE_REDACTION_FN)
                && handled.contains(key.as_str())
        }
    }
}

/// Lines with a claim the live act does not carry out.
fn unbound(act: &Act) -> Vec<String> {
    let handled = resource_handled();
    declarations()
        .iter()
        .filter(|(key, d)| match &d.achieved {
            Achieved::Claims(claims) => !claims.iter().all(|c| binds(act, &handled, key, c)),
            Achieved::Unstated | Achieved::Remainder(_) => false,
        })
        .map(|(key, _)| key.to_string())
        .collect()
}

/// FAILS IF: a `full` line says nothing about what the act achieves, a remainder gives no reason,
/// or a line whose ceiling is not `full` claims an achievement. A `full` line with neither a claim
/// nor a remainder is the gap this column exists to close: the ceiling read as the result.
#[test]
fn every_full_line_states_its_achieved_reach() {
    let (mut silent, mut unreasoned, mut misplaced) = (Vec::new(), Vec::new(), Vec::new());
    for (key, d) in declarations() {
        let key = key.to_string();
        match (&d.achieved, d.ceiling == Ceiling::Full) {
            (Achieved::Unstated, true) => silent.push(key),
            (Achieved::Remainder(reason), true) if reason.is_empty() => unreasoned.push(key),
            (Achieved::Unstated, false) | (_, true) => {}
            (_, false) => misplaced.push(key),
        }
    }
    assert!(
        silent.is_empty(),
        "these `full` lines say how far erasure could reach and nothing about what the person act \
         does. Name what reaches each (`erased:<arm>`, `deleted:<arm>`, `cascaded:<arm>:<table>`, \
         `resource-erasure`) or declare `remainder: <reason>`.\nSilent: {silent:#?}"
    );
    assert!(
        unreasoned.is_empty(),
        "a remainder needs its reason.\nUnreasoned: {unreasoned:#?}"
    );
    assert!(
        misplaced.is_empty(),
        "only a `full` line states an achieved reach; every other ceiling carries `-`.\n\
         Misplaced: {misplaced:#?}"
    );
}

/// FAILS IF: a line claims a reach the live person act does not have: an `erased` column no UPDATE
/// assigns an erasure value, a `deleted` or `cascaded` table no DELETE reaches, or a
/// `resource-erasure` column the act no longer runs resource erasure for, or whose resource
/// manifest line no longer declares it `handled`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_achieved_claim_is_carried_out_by_the_act(pool: PgPool) {
    let broken = unbound(&act(&pool).await);
    assert!(
        broken.is_empty(),
        "policy/erasure/personal-data.toml says the person act reaches these columns, and the live \
         act does not do what the entry's `achieved` claims. Restore the arm, or change the entry to \
         say what the act now does, a `remainder: <reason>` included.\nUnbound: {broken:#?}"
    );
}

/// Installs `function`'s live definition with each `(from, to)` rewrite applied exactly once.
async fn install_rewritten(pool: &PgPool, function: &str, rewrites: &[(&str, &str)]) {
    let mut body = function_body(pool, function).await;
    for (from, to) in rewrites {
        assert_eq!(
            body.matches(from).count(),
            1,
            "the probe expects `{from}` exactly once in {function}; the function changed shape"
        );
        body = body.replace(from, to);
    }
    sqlx::query(&body)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("install the rewritten {function}: {e}"));
}

/// FAILS IF: removing an arm leaves its lines bound. One probe per claim kind: an erasing UPDATE
/// made to keep the column's own value (twice: a constant, and the sentinel chain), a DELETE moved
/// to another table (a delete, a cascade parent, and a delete reaching three lines), and one branch
/// of the refresh-token IF, which unbinds its line although the other branch still deletes.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn removing_an_arm_unbinds_exactly_its_lines(pool: PgPool) {
    assert!(
        unbound(&act(&pool).await).is_empty(),
        "precondition: every claim binds"
    );

    install_rewritten(
        &pool,
        REDACTION_FN,
        &[
            ("SET metadata = '{}'::jsonb", "SET metadata = metadata"),
            (
                "name = v_sentinel || ' (personal)'",
                "name = name || ' (personal)'",
            ),
            (
                "DELETE FROM kb_oauth_flow f",
                "DELETE FROM kb_oauth_flow_probe f",
            ),
            (
                "DELETE FROM kb_blob_uploads u",
                "DELETE FROM kb_blob_uploads_probe u",
            ),
            (
                "DELETE FROM kb_slack_grant_vault v",
                "DELETE FROM kb_slack_grant_vault_probe v",
            ),
            ("claims     = '{}'::jsonb", "claims     = t.claims"),
        ],
    )
    .await;

    assert_eq!(
        unbound(&act(&pool).await),
        [
            "kb_blob_upload_segments.bytes",
            "kb_entities.metadata",
            "kb_oauth_flow.claims",
            "kb_oauth_refresh_tokens.claims",
            "kb_slack_grant_vault.at_ciphertext",
            "kb_slack_grant_vault.rt_ciphertext",
            "kb_slack_grant_vault.slack_principal_id",
            "kb_teams.name",
        ]
        .map(String::from)
        .to_vec(),
        "each removed arm must unbind exactly the lines it reaches"
    );
}

/// FAILS IF: the sentinel stops being one and the lines assigning it stay bound. The act assigns
/// `v_sentinel` by name, so its declaration is what makes those assignments erase.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_sentinel_that_reads_the_profile_unbinds_its_lines(pool: PgPool) {
    install_rewritten(
        &pool,
        REDACTION_FN,
        &[(
            "v_sentinel text := 'erased-' || p_subject::text;",
            "v_sentinel text := (SELECT handle FROM kb_profiles WHERE id = p_subject);",
        )],
    )
    .await;

    assert_eq!(
        unbound(&act(&pool).await),
        [
            "kb_profiles.display_name",
            "kb_profiles.handle",
            "kb_teams.name",
            "kb_teams.slug",
        ]
        .map(String::from)
        .to_vec(),
    );
}

/// FAILS IF: the act stops running a body and the lines that body reaches stay bound: the resource
/// erasures (every `resource-erasure` claim), then the redaction (every other claim).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_act_that_skips_a_body_unbinds_what_it_reaches(pool: PgPool) {
    let original = function_body(&pool, EXECUTE_FN).await;
    install_rewritten(
        &pool,
        EXECUTE_FN,
        &[(
            "v_out := resource_erasure_execute(",
            "v_out := resource_erasure_execute_probe(",
        )],
    )
    .await;
    assert_eq!(
        unbound(&act(&pool).await),
        [
            "kb_chunks.embedding",
            "kb_data_artifact_content.content",
            "kb_data_artifact_verdicts.detail",
            "kb_properties.property_value",
            "kb_resource_search_index.search_vector",
            "kb_workflow_jobs.payload",
        ]
        .map(String::from)
        .to_vec(),
    );

    sqlx::query(&original)
        .execute(&pool)
        .await
        .expect("reinstall the live act");
    install_rewritten(
        &pool,
        EXECUTE_FN,
        &[(
            "PERFORM _erasure_apply_redaction(",
            "PERFORM _erasure_apply_redaction_probe(",
        )],
    )
    .await;
    let claimed_by_redaction: Vec<String> = declarations()
        .iter()
        .filter(|(_, d)| match &d.achieved {
            Achieved::Claims(c) => c.iter().any(|c| !matches!(c, Claim::ResourceErasure)),
            _ => false,
        })
        .map(|(k, _)| k.to_string())
        .collect();
    assert!(!claimed_by_redaction.is_empty(), "precondition");
    assert_eq!(unbound(&act(&pool).await), claimed_by_redaction);
}

/// The sentinel chain, on the shapes the act uses and the ones it must not be fooled by. Each
/// negative case is excluded by exactly one rule, named beside it.
#[test]
fn the_sentinel_chain_binds_literals_and_the_declared_sentinel_only() {
    let declared = "v_sentinel text := 'erased-' || p_subject::text;";
    let bound = |sql: &str, key: &str| assigns_erasure(&tokens(sql), key);
    let teams = format!(
        "{declared} UPDATE kb_teams SET slug = 'personal-' || v_sentinel, \
         name = v_sentinel || ' (personal)';"
    );
    assert!(bound(&teams, "kb_teams.slug"));
    assert!(bound(&teams, "kb_teams.name"));
    assert!(bound(
        &format!("{declared} UPDATE kb_profiles SET handle = v_sentinel;"),
        "kb_profiles.handle"
    ));

    for (sql, key, why) in [
        (
            "UPDATE kb_profiles SET handle = v_sentinel;".to_string(),
            "kb_profiles.handle",
            "a sentinel nobody declared",
        ),
        (
            "v_sentinel text := handle; UPDATE kb_profiles SET handle = v_sentinel;".to_string(),
            "kb_profiles.handle",
            "a sentinel declared from a row",
        ),
        (
            format!("{declared} UPDATE kb_teams SET slug = 'personal-' || handle;"),
            "kb_teams.slug",
            "a chain without the sentinel",
        ),
        (
            format!("{declared} UPDATE kb_teams SET name = v_sentinel || t.name;"),
            "kb_teams.name",
            "a row value in the chain",
        ),
        (
            format!("{declared} UPDATE kb_teams SET name = coalesce(v_sentinel, name);"),
            "kb_teams.name",
            "a function over the sentinel",
        ),
        (
            format!("{declared} UPDATE kb_teams SET name = v_sentinel + ' (personal)';"),
            "kb_teams.name",
            "a join that is not ||",
        ),
    ] {
        assert!(!bound(&sql, key), "{key}: {why} is not an erasure");
    }
}
