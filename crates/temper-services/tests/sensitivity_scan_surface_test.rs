#![cfg(feature = "test-db")]
//! The two text-column manifests PARTITION the catalog's free text; this test is the partition.
//!
//! Under goal *"Sensitivity sweep"* (spec D3, Q9). `personal_data_surface_test.rs` derives its
//! candidates from structure: foreign keys, polymorphic pairs, types, a name heuristic. Structure
//! cannot tell prose from a hash, so every `text` / `character varying` column it does not nominate
//! used to be a stated silence, and that silence held the whole authored corpus. This test closes
//! it. Every text/varchar column of a public base table is declared in **exactly one** of:
//!
//! - `policy/erasure/personal-data.toml`: nominated by a personal-data derivation;
//! - `policy/erasure/sensitivity-scan.toml`: nominated by nothing, and adjudicated there as
//!   `scan`, `structural`, `credential` or `out-of-scope`.
//!
//! **The three failure directions.** A column in neither file is free text nobody classified: the
//! dangerous one, because the sweep enumerates its targets from the scan manifest. A column in both
//! is two contradicting judgements about one value. A scan entry naming a column that is not a live
//! text/varchar column is stale. All three fail, for the sibling's reason: a manifest that
//! accumulates dead lines stops being read.
//!
//! The sibling manifest is read for its KEYS only. Its non-text entries (uuids, vectors, jsonb) are
//! its own business and are checked by its own test; here they simply cannot collide, because the
//! scan manifest may only name text/varchar columns.

use std::collections::{BTreeMap, BTreeSet};

use sqlx::PgPool;
use temper_core::erasure_policy::{ColumnKey, ScanColumn, ScanDisposition};
use temper_services::erasure_policy::in_force;

/// Every text/varchar column of a public base table: the set the two manifests partition.
///
/// The same `BASE TABLE` join and type predicate the sibling's `js`, `bytes` and `named`
/// derivations use, so "a column" means the same thing on both sides.
const TEXT_COLUMNS: &str = r#"
SELECT c.table_name || '.' || c.column_name
  FROM information_schema.columns c
  JOIN information_schema.tables t
    ON t.table_name = c.table_name AND t.table_schema = 'public' AND t.table_type = 'BASE TABLE'
 WHERE c.table_schema = 'public' AND c.data_type IN ('text', 'character varying')
 ORDER BY 1
"#;

/// `table.column` → its disposition and note, the declaration half of this test, from the policy in
/// force. The parser (`temper_core::erasure_policy::parse`) refuses an unknown disposition (there
/// is deliberately no `unadjudicated` one) and a column declared twice, so a malformed entry fails
/// the parse rather than dropping a declaration and reading as an undeclared column.
fn scan_declarations() -> &'static BTreeMap<ColumnKey, ScanColumn> {
    &in_force().sensitivity.columns
}

/// The other side of the partition: the sibling's declared keys. Its own test owns the rest of
/// its entries; this reads the keys only.
fn personal_declarations() -> BTreeSet<String> {
    in_force()
        .personal_data
        .keys()
        .map(ColumnKey::to_string)
        .collect()
}

/// The partition's three failure sets, computed against whatever catalog `pool` sees.
struct Partition {
    /// Text/varchar columns declared in neither manifest.
    undeclared: Vec<String>,
    /// Columns declared in both manifests.
    doubly_declared: Vec<String>,
    /// Scan-manifest entries that name no live text/varchar column.
    stale: Vec<String>,
}

async fn partition(pool: &PgPool) -> Partition {
    let live: BTreeSet<String> = sqlx::query_scalar::<_, String>(TEXT_COLUMNS)
        .fetch_all(pool)
        .await
        .expect("derive the text/varchar column set")
        .into_iter()
        .collect();
    let scan: BTreeSet<String> = scan_declarations()
        .keys()
        .map(ColumnKey::to_string)
        .collect();
    let personal = personal_declarations();

    Partition {
        undeclared: live
            .iter()
            .filter(|k| !scan.contains(*k) && !personal.contains(*k))
            .cloned()
            .collect(),
        doubly_declared: scan.intersection(&personal).cloned().collect(),
        stale: scan.difference(&live).cloned().collect(),
    }
}

/// FAILS IF: a text/varchar column exists that neither manifest declares. The union is total.
///
/// This is the direction that matters. A new `note` column holding a person's name is found by no
/// structural derivation; it is found here, the day it is added.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_text_column_is_declared_in_one_manifest(pool: PgPool) {
    let undeclared = partition(&pool).await.undeclared;
    assert!(
        undeclared.is_empty(),
        "these text/varchar columns are declared in neither policy/erasure/personal-data.toml nor \
         policy/erasure/sensitivity-scan.toml.\n\
         Add an entry under `[columns]` in sensitivity-scan.toml — \
         `\"table.column\" = {{ disposition, note }}` — with \
         `scan` for authored or user-supplied text, `structural` for hashes, enums and vocabularies, \
         `credential` for secrets, or `out-of-scope` with the reason. (If the column is nominated \
         by a personal-data derivation, its own test will already be asking for it there instead.)\n\
         Undeclared: {undeclared:#?}"
    );
}

/// FAILS IF: one column is declared in both manifests. The intersection is empty.
///
/// Two judgements about one value contradict each other the day either is edited, and a reader
/// cannot tell which one the sweep or the erasure path obeys.
#[test]
fn no_column_is_declared_in_both_manifests() {
    let personal = personal_declarations();
    let both: Vec<String> = scan_declarations()
        .keys()
        .map(ColumnKey::to_string)
        .filter(|k| personal.contains(k))
        .collect();
    assert!(
        both.is_empty(),
        "these columns are declared in BOTH manifests. A column nominated by a personal-data \
         derivation belongs in personal-data.toml only; delete its entry from \
         sensitivity-scan.toml.\n\
         Doubly declared: {both:#?}"
    );
}

/// FAILS IF: the scan manifest names a column that is not a live text/varchar column of a public
/// base table: dropped, renamed, or retyped.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_scan_declaration_names_a_live_text_column(pool: PgPool) {
    let stale = partition(&pool).await.stale;
    assert!(
        stale.is_empty(),
        "policy/erasure/sensitivity-scan.toml declares columns that are not live text/varchar \
         columns of a public base table. Delete the entry if the column was dropped; move it if it \
         was renamed; if it was retyped, the sibling manifest's derivations may now own it.\n\
         Stale: {stale:#?}"
    );
}

/// FAILS IF: adding a text column does not break the partition until it is declared (Witness 8).
///
/// The other tests prove the partition holds today. This one proves the gate is live: a column the
/// catalog grows is reported as undeclared, not silently absorbed. Each `sqlx::test` runs in its own
/// database, so the probe column never escapes this test.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_added_text_column_fails_the_partition_until_declared(pool: PgPool) {
    let before = partition(&pool).await;
    assert!(
        before.undeclared.is_empty(),
        "precondition: the partition holds before the probe column is added"
    );

    sqlx::query("ALTER TABLE kb_resources ADD COLUMN sensitivity_witness_probe text")
        .execute(&pool)
        .await
        .expect("add a probe text column");

    let after = partition(&pool).await;
    assert_eq!(
        after.undeclared,
        vec!["kb_resources.sensitivity_witness_probe".to_string()],
        "an undeclared text column must be reported, and only it"
    );
    assert!(after.doubly_declared.is_empty() && after.stale.is_empty());
}

/// FAILS IF: an `out-of-scope` entry gives no reason. The class means "everything else", and
/// without its reason a reader cannot tell a judgement from a shrug.
#[test]
fn every_out_of_scope_line_states_its_reason() {
    let silent: Vec<String> = scan_declarations()
        .iter()
        .filter(|(_, c)| c.disposition == ScanDisposition::OutOfScope && c.note.is_empty())
        .map(|(k, _)| k.to_string())
        .collect();
    assert!(
        silent.is_empty(),
        "`out-of-scope` requires the reason in the note.\nSilent: {silent:#?}"
    );
}

/// FAILS IF: the manifest's stated count (`declared_count`) disagrees with its declarations.
///
/// The manifest states the count so the next drift is readable without running anything. A count
/// that is never checked drifts, which is how the sibling's header came to say 150 while the
/// catalog said 147.
#[test]
fn the_header_count_matches_the_declarations() {
    let sensitivity = &in_force().sensitivity;
    let declared = sensitivity.columns.len();
    let stated = sensitivity.declared_count;
    assert_eq!(
        stated, declared,
        "policy/erasure/sensitivity-scan.toml's `declared_count` says {stated} columns are declared \
         there, but it declares {declared}. Update `declared_count` (and `count_at`, the migration it \
         was taken at)."
    );
}
