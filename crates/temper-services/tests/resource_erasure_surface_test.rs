#![cfg(feature = "test-db")]
//! Every carrier a resource reaches is handled by the erasure act or declared out of scope; this
//! test is the join that says so.
//!
//! Under goal *"A single resource can be erased out of a live estate"* (spec D9, the table half;
//! Witness 13). Four sources meet here, and only one of them is written for this test:
//!
//! - **What a column holds** comes from the two sibling manifests. A carrier is a column
//!   `policy/erasure/sensitivity-scan.toml` classes `scan`, or one
//!   `policy/erasure/personal-data.toml` classes `content`, `incidental` or `derived` (ruled
//!   2026-10-02). Nothing here re-classifies a column.
//! - **Which tables a resource reaches** comes from the live catalog. A foreign key, or a
//!   single-column CHECK on a `*_table` discriminator that enumerates tables, makes its table a
//!   child of the table it names; the walk takes every child of a reached table, recursively, from
//!   five roots, the ledger included (ruled 2026-10-02, amended 2026-10-03). Nothing enumerates
//!   tables by hand, so a new table is reached by construction rather than by someone remembering
//!   it.
//! - **What the act does about each carrier** is declared in `policy/erasure/resource-erasure.toml`.
//! - **Whether the act really does it** is read from the live body of
//!   `_resource_erasure_apply_redaction`, the D2 function: a `handled` line must find an UPDATE of
//!   its table assigning its column an erasure value.
//!
//! **Why the binding to the function body exists.** Without it, the manifest could say a column is
//! handled while the act had stopped erasing it, and every test here would stay green. Reverting
//! the joint-read fixes (`kb_chunks.header_path`, `kb_citation_audits.reason`) is the miss this
//! fence exists for, and `reverting_the_joint_read_fixes_fails_the_fence` proves it turns red.
//!
//! **What this does not claim.** The binding proves the statement is there and assigns an erasure
//! value. It does not prove its WHERE clause reaches every row tied to the resource; the act's own
//! witnesses own that, and the manifest's notes name the rows the act leaves. The walk is blind to
//! references spelled neither as a foreign key nor as a CHECKed `*_table` discriminator; the
//! manifest header lists them. The CHECK derivation here is a widened form of the personal-data
//! fence's `poly` derivation (`personal_data_surface_test.rs`), which names `kb_profiles` only.

mod erasure_binding;

use std::collections::{BTreeMap, BTreeSet};

use erasure_binding::{assignments, deletes_from, is_erasure_value, is_identity, tokens};
use sqlx::PgPool;
use temper_core::erasure_policy::{
    Class, ColumnKey, Disposition, PayloadDisposition, Qualifier, ScanDisposition, TableLine,
};
use temper_services::erasure_policy::in_force;

/// The personal-data classes that make a column a carrier. `identifier` and `reference` are about
/// a person, not about what a resource says; `discriminator` and `none` hold nothing.
const CARRIER_CLASSES: &[Class] = &[Class::Content, Class::Incidental, Class::Derived];

/// The D2 function: the one home of the content shape (spec D2).
const REDACTION_FN: &str = "_resource_erasure_apply_redaction";

/// The parent→child edges of the walk: every foreign key, and every single-column CHECK on a
/// `*_table` discriminator, read as an edge to each public table it names.
const EDGES: &str = r#"
  SELECT DISTINCT conrelid::regclass::text AS child, confrelid::regclass::text AS parent
    FROM pg_constraint
   WHERE contype = 'f' AND connamespace = 'public'::regnamespace
  UNION
  SELECT DISTINCT c.conrelid::regclass::text, m[1]
    FROM pg_constraint c
    JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
   CROSS JOIN LATERAL regexp_matches(pg_get_constraintdef(c.oid), '''([a-z_][a-z0-9_]*)''', 'g') m
   WHERE c.contype = 'c' AND c.connamespace = 'public'::regnamespace
     AND cardinality(c.conkey) = 1 AND a.attname LIKE '%\_table'
     AND m[1] IN (SELECT relname FROM pg_class
                   WHERE relnamespace = 'public'::regnamespace AND relkind IN ('r', 'p'))
"#;

/// Base-table `*_table` columns with no CHECK that ENUMERATES the tables they may name. A
/// discriminator the walk cannot read as an edge could point at a resource unseen.
///
/// "Enumerates" is a positive match on the two shapes Postgres prints for `col IN ('a', 'b')` and
/// `col = 'a'`, once casts, parentheses and whitespace are stripped: `CHECK col=ANY ARRAY['a','b']`
/// and `CHECK col='a'`. Anything else (a pairing CHECK, a pattern, a negation, `IS DISTINCT FROM`,
/// an OR with an open arm) does not confine the column and does not count. At least one literal
/// must name a public table.
const UNENUMERATED_DISCRIMINATORS: &str = r#"
SELECT c.table_name || '.' || c.column_name
  FROM information_schema.columns c
  JOIN information_schema.tables t
    ON t.table_name = c.table_name AND t.table_schema = 'public' AND t.table_type = 'BASE TABLE'
 WHERE c.table_schema = 'public' AND c.column_name LIKE '%\_table'
   AND NOT EXISTS (
     SELECT 1
       FROM pg_constraint k
      CROSS JOIN LATERAL regexp_replace(pg_get_constraintdef(k.oid),
                                        '::[a-z ]+(\[\])?|[()\s]', '', 'g') AS def(shape)
      CROSS JOIN LATERAL regexp_matches(def.shape, '''([a-z_][a-z0-9_]*)''', 'g') m
      WHERE k.contype = 'c' AND k.conrelid = (quote_ident(c.table_name::text))::regclass
        AND def.shape ~ ('^CHECK' || c.column_name
                         || '=(ANYARRAY\[(''[a-z_][a-z0-9_]*'',)*''[a-z_][a-z0-9_]*''\]|''[a-z_][a-z0-9_]*'')$')
        AND m[1] IN (SELECT relname FROM pg_class
                      WHERE relnamespace = 'public'::regnamespace AND relkind IN ('r', 'p')))
 ORDER BY 1
"#;

fn reachable_tables_sql() -> String {
    format!(
        "WITH RECURSIVE edge AS ({EDGES}),
         reach(tbl) AS (
           SELECT unnest(ARRAY['kb_resources','kb_content_blocks','kb_chunks',
                               'kb_block_revisions','kb_edges'])
           UNION
           SELECT edge.child FROM edge JOIN reach ON edge.parent = reach.tbl)
         SELECT tbl FROM reach ORDER BY 1"
    )
}

/// `table.column` → its disposition and note: the declaration half, the `[tables]` of
/// `policy/erasure/resource-erasure.toml` in force. The parser (`temper_core::erasure_policy::parse`)
/// refuses an unknown disposition, a malformed `repointed` column and a column declared
/// twice, so a malformed entry fails the parse rather than dropping a declaration and reading as
/// uncovered.
fn declarations() -> &'static BTreeMap<ColumnKey, TableLine> {
    &in_force().resource_erasure.tables
}

/// Every carrier column, by the two sibling manifests: `scan` in sensitivity-scan.toml, and
/// `content`/`incidental`/`derived` in personal-data.toml.
fn carriers() -> BTreeSet<String> {
    let policy = in_force();
    policy
        .sensitivity_scan
        .keys(ScanDisposition::Scan)
        .chain(
            CARRIER_CLASSES
                .iter()
                .flat_map(|class| policy.personal_data_keys(*class)),
        )
        .map(ColumnKey::to_string)
        .collect()
}

fn table_of(key: &str) -> &str {
    key.split_once('.').map_or(key, |(t, _)| t)
}

async fn reachable_tables(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar::<_, String>(&reachable_tables_sql())
        .fetch_all(pool)
        .await
        .expect("derive the tables a resource reaches")
        .into_iter()
        .collect()
}

async fn redaction_body(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>(&format!(
        "SELECT pg_get_functiondef('{REDACTION_FN}'::regproc)"
    ))
    .fetch_one(pool)
    .await
    .expect("read the D2 function's live definition")
}

/// Whether `expr` reads `column`, bare or qualified: an assignment of the column's own value.
fn reads_column(expr: &[String], column: &str) -> bool {
    expr.iter()
        .any(|tok| tok == column || tok.rsplit_once('.').is_some_and(|(_, col)| col == column))
}

/// Whether `body` does what `disposition` claims for `key`.
///
/// A `handled` column must be assigned at least once and EVERY assignment must erase, so a later
/// UPDATE that writes the original back unbinds it. A `repointed` column needs a placing
/// assignment to its via column: one that neither reads the column (a no-op) nor is a bare
/// identity (the act's park pass, `source_id = bp.id`, which parks a row on its own id before the
/// place pass) — and the DELETE.
fn binds(body: &str, key: &str, disposition: &Disposition) -> bool {
    let t = tokens(body);
    match disposition {
        Disposition::Handled => {
            let values = assignments(&t, key);
            !values.is_empty() && values.into_iter().all(is_erasure_value)
        }
        Disposition::Repointed { via } => {
            assignments(&t, via.as_str()).into_iter().any(|expr| {
                !expr.is_empty()
                    && !reads_column(expr, via.column())
                    && !matches!(expr, [only] if is_identity(only))
            }) && deletes_from(&t, table_of(key))
        }
        Disposition::OutOfScope => true,
    }
}

/// Carriers in reachable tables with no entry in the manifest. `carriers` is a parameter, not read
/// inside, so the Witness 13 probes can stand in for the sibling-manifest PR that would class them.
fn uncovered(reach: &BTreeSet<String>, carriers: &BTreeSet<String>) -> Vec<String> {
    let declared: BTreeSet<&str> = declarations().keys().map(ColumnKey::as_str).collect();
    carriers
        .iter()
        .filter(|k| reach.contains(table_of(k)) && !declared.contains(k.as_str()))
        .cloned()
        .collect()
}

/// Entries whose disposition the D2 function does not carry out.
fn unbound(body: &str) -> Vec<String> {
    declarations()
        .iter()
        .filter(|(k, line)| !binds(body, k.as_str(), &line.disposition))
        .map(|(k, _)| k.to_string())
        .collect()
}

async fn unenumerated_discriminators(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar::<_, String>(UNENUMERATED_DISCRIMINATORS)
        .fetch_all(pool)
        .await
        .expect("probe base-table *_table columns for an enumerating CHECK")
}

/// FAILS IF: a carrier sits in a table a resource reaches and the manifest says nothing about what
/// the erasure act does with it. This is the direction that matters: content that survives an
/// erasure while the act reports success.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_reachable_carrier_is_handled_or_declared(pool: PgPool) {
    let missing = uncovered(&reachable_tables(&pool).await, &carriers());
    assert!(
        missing.is_empty(),
        "these columns are carriers (`scan` in policy/erasure/sensitivity-scan.toml, or \
         content/incidental/derived in policy/erasure/personal-data.toml), sit in a table a \
         resource reaches, and have no entry in policy/erasure/resource-erasure.toml [tables].\n\
         Either make `{REDACTION_FN}` erase the column and declare it `handled`, or \
         declare it `out-of-scope` with the reason the act leaves it.\n\
         Uncovered: {missing:#?}"
    );
}

/// FAILS IF: a line claims something the D2 function no longer does: a `handled` column it does not
/// assign an erasure value, or a `repointed` column whose re-point or delete is gone.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_handled_declaration_is_carried_out_by_the_redaction(pool: PgPool) {
    let broken = unbound(&redaction_body(&pool).await);
    assert!(
        broken.is_empty(),
        "policy/erasure/resource-erasure.toml [tables] declares these columns handled, but the live \
         `{REDACTION_FN}` does not do what the entry says: a `handled` column needs an UPDATE of its \
         table assigning NULL, '', '{{}}', '\"erased\"' or an 'erased…' sentinel; a `repointed` \
         one needs the re-point UPDATE and the DELETE.\n\
         Restore the statement, or change the entry to say what the act now does.\n\
         Unbound: {broken:#?}"
    );
}

/// FAILS IF: a line names a column that is not a carrier (the manifest's subject is the siblings'
/// carriers, nothing else), or an `out-of-scope` line names a table no resource reaches (there is
/// nothing to be out of scope OF). A `handled` or `repointed` line outside the walk is allowed:
/// the binding test checks it against the function body instead (today, `kb_remote_sources.uri`).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_declaration_is_live(pool: PgPool) {
    let carriers = carriers();
    let reach = reachable_tables(&pool).await;
    let stale: Vec<String> = declarations()
        .iter()
        .filter(|(k, line)| {
            !carriers.contains(k.as_str())
                || (line.disposition == Disposition::OutOfScope && !reach.contains(k.table()))
        })
        .map(|(k, _)| k.to_string())
        .collect();
    assert!(
        stale.is_empty(),
        "policy/erasure/resource-erasure.toml [tables] declares columns that are not carriers in \
         either sibling manifest, or are out of scope in a table no resource reaches. Delete the \
         entry, or \
         (if a sibling reclassified the column) check that manifest's reason first.\n\
         Stale: {stale:#?}"
    );
}

/// FAILS IF: a base-table `*_table` discriminator has no CHECK enumerating the tables it may name. The walk reads polymorphic edges from those CHECKs; a discriminator without one
/// could point at a resource and the walk would never see its table.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_polymorphic_discriminator_enumerates_its_targets(pool: PgPool) {
    let bare = unenumerated_discriminators(&pool).await;
    assert!(
        bare.is_empty(),
        "these `*_table` columns carry no CHECK enumerating their target tables \
         (`col IN ('kb_a', 'kb_b')` or `col = 'kb_a'`), so the erasure fence cannot tell what they \
         reference. A pairing CHECK, a pattern, a negation or `IS DISTINCT FROM` does not count. \
         Add the enumerating CHECK, the convention every other discriminator follows.\n\
         Unenumerated: {bare:#?}"
    );
}

/// FAILS IF: an `out-of-scope` line gives no reason. The class means "the act leaves this", and a
/// reader needs to know why to tell a judgement from an omission.
#[test]
fn every_out_of_scope_line_states_its_reason() {
    let silent: Vec<String> = declarations()
        .iter()
        .filter(|(_, line)| line.disposition == Disposition::OutOfScope && line.note.is_empty())
        .map(|(k, _)| k.to_string())
        .collect();
    assert!(
        silent.is_empty(),
        "`out-of-scope` requires the reason in the note.\nSilent: {silent:#?}"
    );
}

/// Witness 13. FAILS IF: a carrier added to a resource-reachable table, in any of the ways a table
/// becomes reachable, is not reported; or one added to an unreached table is.
///
/// The reachable probes are: a text column on an existing table, a jsonb column on one (the
/// non-text carriers, ruled 2026-10-02), a new child of a root, a new grandchild (the walk is
/// transitive), a new polymorphic owner of `kb_resources`, a new polymorphic owner of a root other
/// than `kb_resources`, a new polymorphic owner of a reached table that is not a root (edges come
/// from a CHECK naming ANY table), and a new table holding only a foreign key into `kb_events` (the
/// walk descends from the ledger, ruled 2026-10-03). The new tables are the case a hand-maintained
/// table list would miss. The last probe references only `kb_teams`, which no resource reaches, and
/// must NOT be reported: reach is not everything.
///
/// The probe columns join the carrier set here, standing in for the sibling-manifest PR that would
/// class them; the "until declared" half is `every_reachable_carrier_is_handled_or_declared`, which
/// reads the shipped manifest. Each `sqlx::test` runs in its own database, so nothing escapes.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_added_carrier_in_reach_is_reported(pool: PgPool) {
    assert!(
        uncovered(&reachable_tables(&pool).await, &carriers()).is_empty(),
        "precondition: the fence holds before the probes are added"
    );

    for ddl in [
        "ALTER TABLE kb_content_blocks ADD COLUMN erasure_fence_probe text",
        "ALTER TABLE kb_content_blocks ADD COLUMN erasure_fence_probe_doc jsonb",
        "CREATE TABLE erasure_fence_probe_child (
             id uuid PRIMARY KEY,
             block_id uuid NOT NULL REFERENCES kb_content_blocks(id),
             note text)",
        "CREATE TABLE erasure_fence_probe_grandchild (
             child_id uuid NOT NULL REFERENCES erasure_fence_probe_child(id),
             note text)",
        "CREATE TABLE erasure_fence_probe_owner (
             owner_table text NOT NULL CHECK (owner_table IN ('kb_resources', 'kb_cogmaps')),
             owner_id uuid NOT NULL,
             note text)",
        "CREATE TABLE erasure_fence_probe_block_owner (
             target_table text NOT NULL CHECK (target_table IN ('kb_content_blocks')),
             target_id uuid NOT NULL,
             note text)",
        "CREATE TABLE erasure_fence_probe_artifact_owner (
             target_table text NOT NULL CHECK (target_table IN ('kb_data_artifacts')),
             target_id uuid NOT NULL,
             note text)",
        "CREATE TABLE erasure_fence_probe_caused (
             caused_by_event_id uuid NOT NULL REFERENCES kb_events(id),
             note text)",
        "CREATE TABLE erasure_fence_probe_unrelated (
             team_id uuid NOT NULL REFERENCES kb_teams(id),
             note text)",
    ] {
        sqlx::query(ddl)
            .execute(&pool)
            .await
            .expect("apply probe DDL");
    }

    let reported = [
        "erasure_fence_probe_artifact_owner.note",
        "erasure_fence_probe_block_owner.note",
        "erasure_fence_probe_caused.note",
        "erasure_fence_probe_child.note",
        "erasure_fence_probe_grandchild.note",
        "erasure_fence_probe_owner.note",
        "kb_content_blocks.erasure_fence_probe",
        "kb_content_blocks.erasure_fence_probe_doc",
    ];
    let mut carriers = carriers();
    carriers.extend(reported.iter().map(|p| p.to_string()));
    carriers.insert("erasure_fence_probe_unrelated.note".to_string());

    assert_eq!(
        uncovered(&reachable_tables(&pool).await, &carriers),
        reported.map(String::from).to_vec(),
        "every reachable probe must be reported, and the unrelated one must not"
    );
}

/// The done-when sanity check. FAILS IF: reverting the joint-read fixes leaves the fence green.
///
/// The first draft of D2 missed `kb_chunks.header_path` and `kb_citation_audits.reason`. This
/// rewrites the live function so those two UPDATEs keep the column's own value, which still
/// mentions and assigns the column but erases nothing, and asserts the binding reports exactly those
/// two. Each rewrite must match exactly once, so the probe cannot silently test nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reverting_the_joint_read_fixes_fails_the_fence(pool: PgPool) {
    let body = redaction_body(&pool).await;
    assert!(
        unbound(&body).is_empty(),
        "precondition: every line is bound"
    );

    let mut reverted = body.clone();
    for (fix, revert) in [
        ("SET header_path = NULL", "SET header_path = header_path"),
        ("SET reason = NULL", "SET reason = reason"),
    ] {
        assert_eq!(
            reverted.matches(fix).count(),
            1,
            "the probe expects `{fix}` exactly once in {REDACTION_FN}; the function changed shape"
        );
        reverted = reverted.replace(fix, revert);
    }
    sqlx::query(&reverted)
        .execute(&pool)
        .await
        .expect("install the reverted function");

    assert_eq!(
        unbound(&redaction_body(&pool).await),
        vec![
            "kb_chunks.header_path".to_string(),
            "kb_citation_audits.reason".to_string()
        ],
        "reverting the joint-read fixes must unbind exactly those two columns"
    );
}

/// FAILS IF: reverting the derived-vector statements (7b, 7c) leaves the fence green.
///
/// Ruled 2026-10-04: the act nulls the home context's telos snapshot and zeroes a folded region's
/// centroid. This rewrites those two UPDATEs to keep the column's own value and asserts the binding
/// reports exactly those two columns, so the zero-vector shape the fence learned for them binds
/// only the erasing statement. Each rewrite must match exactly once.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reverting_the_derived_vector_statements_fails_the_fence(pool: PgPool) {
    let body = redaction_body(&pool).await;
    assert!(
        unbound(&body).is_empty(),
        "precondition: every line is bound"
    );

    let mut reverted = body.clone();
    for (fix, revert) in [
        (
            "SET telos_centroid = NULL",
            "SET telos_centroid = telos_centroid",
        ),
        (
            "SET centroid = array_fill(0, ARRAY[768])::vector",
            "SET centroid = centroid",
        ),
    ] {
        assert_eq!(
            reverted.matches(fix).count(),
            1,
            "the probe expects `{fix}` exactly once in {REDACTION_FN}; the function changed shape"
        );
        reverted = reverted.replace(fix, revert);
    }
    sqlx::query(&reverted)
        .execute(&pool)
        .await
        .expect("install the reverted function");

    assert_eq!(
        unbound(&redaction_body(&pool).await),
        vec![
            "kb_cogmap_regions.centroid".to_string(),
            "kb_contexts.telos_centroid".to_string()
        ],
        "reverting the derived-vector statements must unbind exactly those two columns"
    );
}

/// FAILS IF: a `*_table` column whose CHECKs do not enumerate public tables is not reported by the
/// guard.
///
/// One probe per way a CHECK can fail. The guard has two conjuncts, the enumeration shape and "at
/// least one literal names a public table", and each probe is excluded by exactly one: no CHECK at
/// all; a CHECK with a second column in it; a pattern; a negation; `IS DISTINCT FROM` (each of
/// these names a real table, so only the shape excludes it); a list whose first value has an
/// embedded quote, `'kb_resources''x'`, which a looser list pattern reads as two items, one of them
/// a table; and a well-shaped
/// enumeration of a name that is no table (only the public-table conjunct excludes it).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unenumerated_discriminator_fails_the_guard(pool: PgPool) {
    assert!(
        unenumerated_discriminators(&pool).await.is_empty(),
        "precondition"
    );
    for ddl in [
        "CREATE TABLE erasure_fence_probe_bare (owner_table text, owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_paired (
             owner_table text, owner_id uuid,
             CHECK (owner_table = 'kb_resources' OR owner_id IS NULL))",
        "CREATE TABLE erasure_fence_probe_pattern (
             owner_table text CHECK (owner_table ~ 'kb_resources'), owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_negated (
             owner_table text CHECK (owner_table <> 'kb_resources'), owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_distinct (
             owner_table text CHECK (owner_table IS DISTINCT FROM 'kb_resources'), owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_quoted (
             owner_table text CHECK (owner_table IN ('kb_resources''x', 'kb_no_such_table')), owner_id uuid)",
        "CREATE TABLE erasure_fence_probe_nontable (
             owner_table text CHECK (owner_table IN ('kb_no_such_table')), owner_id uuid)",
    ] {
        sqlx::query(ddl)
            .execute(&pool)
            .await
            .expect("create probe table");
    }
    assert_eq!(
        unenumerated_discriminators(&pool).await,
        vec![
            "erasure_fence_probe_bare.owner_table".to_string(),
            "erasure_fence_probe_distinct.owner_table".to_string(),
            "erasure_fence_probe_negated.owner_table".to_string(),
            "erasure_fence_probe_nontable.owner_table".to_string(),
            "erasure_fence_probe_paired.owner_table".to_string(),
            "erasure_fence_probe_pattern.owner_table".to_string(),
            "erasure_fence_probe_quoted.owner_table".to_string(),
        ]
    );
}

/// The binding itself, on the shapes the D2 function uses and the shapes it must not be fooled by.
/// Each negative case is excluded by exactly one rule, named beside it.
#[test]
fn the_binding_reads_erasing_assignments_not_mentions() {
    let sql = "
        UPDATE kb_chunks c
           SET embedding = NULL, embedded_with = NULL
         WHERE header_path = NULL;
        -- UPDATE kb_chunk_content SET content = NULL;
        /* UPDATE kb_block_content SET content = ''; */
        RAISE EXCEPTION 'UPDATE kb_edges SET label = NULL;';
        UPDATE kb_resources r
           SET title = 'erased-' || r.id::text, origin_uri = 'erased:' || r.origin_uri;
        UPDATE kb_ingestion_records SET source_uri = '' || source_uri;
        UPDATE kb_workflow_jobs j SET payload = '{}'::jsonb || j.payload, last_error = 'erased';
        WITH ranked AS (SELECT 1)
        UPDATE kb_properties p
           SET property_key   = 'erased-key-' || ranked.n::text,
               property_value = '\"erased\"'::jsonb
          FROM ranked;
        UPDATE kb_block_provenance bp SET source_id = (v ->> bp.id::text)::uuid;
        DELETE FROM kb_remote_sources r WHERE r.id = v;
    ";
    let handled = Disposition::Handled;
    for key in [
        "kb_chunks.embedding",
        "kb_chunks.embedded_with",
        "kb_resources.title",
        "kb_properties.property_key",
        "kb_properties.property_value",
    ] {
        assert!(binds(sql, key, &handled), "{key} is erased here");
    }
    for (key, why) in [
        (
            "kb_chunks.header_path",
            "a WHERE comparison (the set/, rule)",
        ),
        ("kb_chunk_content.content", "a line comment"),
        ("kb_block_content.content", "a block comment"),
        ("kb_edges.label", "a string literal"),
        (
            "kb_resources.origin_uri",
            "a sentinel joined to the original value",
        ),
        (
            "kb_ingestion_records.source_uri",
            "an empty string joined to the original",
        ),
        (
            "kb_workflow_jobs.payload",
            "an empty object merged with the original",
        ),
        (
            "kb_workflow_jobs.last_error",
            "a sentinel literal that is not a constant",
        ),
        (
            "kb_remote_sources.uri",
            "a DELETE is not a handled assignment",
        ),
    ] {
        assert!(!binds(sql, key, &handled), "{key}: {why} is not an erasure");
    }

    let repointed = Disposition::Repointed {
        via: "kb_block_provenance.source_id".parse().unwrap(),
    };
    assert!(binds(sql, "kb_remote_sources.uri", &repointed));
    for (body, why) in [
        (
            "DELETE FROM kb_remote_sources r WHERE r.id = v;",
            "a delete without the re-point leaves the citers on the original URL",
        ),
        (
            "UPDATE kb_block_provenance bp SET source_id = v;",
            "a re-point without the delete leaves the original row",
        ),
        (
            "UPDATE kb_block_provenance bp SET source_id = bp.source_id;
             DELETE FROM kb_remote_sources r WHERE r.id = v;",
            "a re-point to its own value leaves every citer in place, so the delete never fires",
        ),
    ] {
        assert!(!binds(body, "kb_remote_sources.uri", &repointed), "{why}");
    }
}

/// The binding's remaining rules, each case excluded by exactly the one named beside it.
#[test]
fn the_binding_is_not_fooled_by_spacing_restoring_or_parking() {
    let handled = Disposition::Handled;
    let bound = |sql: &str, key: &str| binds(sql, key, &handled);

    assert!(
        bound(
            "UPDATE kb_resources r SET title = 'erased-'||r.id::text;",
            "kb_resources.title"
        ),
        "an erasing sentinel binds however it is spaced"
    );
    assert!(
        bound(
            "UPDATE kb_ingestion_records SET source_uri = 'erased:' || p_resource;",
            "kb_ingestion_records.source_uri"
        ),
        "an uncast sentinel binds: a three-token value is tried as both a cast and a sentinel"
    );
    for (sql, key, why) in [
        (
            "UPDATE kb_data_artifact_content SET content = '{}'::jsonb||content;",
            "kb_data_artifact_content.content",
            "a merge glued to its cast (operator tokens)",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' || title||r.id;",
            "kb_resources.title",
            "the original glued in front of an id (operator tokens)",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' || title->>k.id;",
            "kb_resources.title",
            "an id read out of the original value (the alias must be an identifier)",
        ),
        (
            "UPDATE kb_resources r SET title = 'x-' || r.id;",
            "kb_resources.title",
            "a literal that is not an 'erased' sentinel",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' + r.id;",
            "kb_resources.title",
            "a join that is not ||",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' || r.id::varchar;",
            "kb_resources.title",
            "a sentinel cast to something other than text",
        ),
        (
            "UPDATE kb_resources r SET title = 'erased-' || r.id::text;
             UPDATE kb_resources r SET title = h.title FROM h;",
            "kb_resources.title",
            "a later UPDATE writing the original back (every assignment must erase)",
        ),
        (
            "UPDATE kb_chunk_content SET content_hash = content_hash RETURNING id, content = '';",
            "kb_chunk_content.content",
            "a comparison after RETURNING (targets end with the SET list)",
        ),
        (
            "UPDATE kb_chunk_content SET content_hash = coalesce(x, content = '');",
            "kb_chunk_content.content",
            "a comparison inside parentheses (targets are at depth 0)",
        ),
    ] {
        assert!(!bound(sql, key), "{key}: {why} is not an erasure");
    }

    let repointed = Disposition::Repointed {
        via: "kb_block_provenance.source_id".parse().unwrap(),
    };
    assert!(
        !binds(
            "UPDATE kb_block_provenance bp SET source_id = bp.id;
             DELETE FROM kb_remote_sources r WHERE r.id = v;",
            "kb_remote_sources.uri",
            &repointed
        ),
        "the park pass alone (a bare identity) leaves provenance on its own ids: no place pass"
    );
}

// ── D9's payload half: every free-text path on the ledger has a disposition ─────────────────────
//
// The `[payload]` section declares, per (event type, JSON path), what the act does with a string
// the ledger holds: `redact:<class>` (cut 2 rewrites it to the D4 sentinel; today the record names
// it in `ledger_remainder`), `structural`, or `out-of-scope` with its reason. Cut 2's verifier
// takes its allowlist from the `redact` lines, so this is where widening the ledger exception is
// reviewed. The candidates come from the registered payload schemas; a type registered without
// one is declared `permissive` and its paths are listed by hand, which this test cannot check for
// completeness (the manifest's header says so).

/// One declared ledger-path disposition: an entry of `[payload."<event_type>"]` under its path key,
/// or one of that entry's refinements, whose `where` is `qualifier`. `event_type` is `metadata` for
/// an authorship key.
#[derive(Debug, Clone, Copy)]
struct PayloadEntry {
    event_type: &'static str,
    path: &'static str,
    qualifier: Option<&'static Qualifier>,
    disposition: &'static PayloadDisposition,
    note: &'static str,
}

impl PayloadEntry {
    /// Where the entry sits in `policy/erasure/resource-erasure.toml`, for a failure message.
    fn at(&self) -> String {
        let (ty, path) = (self.event_type, self.path);
        match self.qualifier {
            None => format!("[payload.{ty:?}] {path:?}"),
            Some(q) => format!("[payload.{ty:?}] {path:?} refine {q:?}"),
        }
    }
}

/// Every `[payload]` entry and refinement of the policy in force, flattened.
fn payload_entries() -> Vec<PayloadEntry> {
    let mut out = Vec::new();
    for (event_type, paths) in &in_force().resource_erasure.payload {
        for (path, line) in paths {
            out.push(PayloadEntry {
                event_type,
                path,
                qualifier: None,
                disposition: &line.disposition,
                note: &line.note,
            });
            out.extend(line.refine.iter().map(|r| PayloadEntry {
                event_type,
                path,
                qualifier: Some(&r.r#where),
                disposition: &r.disposition,
                note: &r.note,
            }));
        }
    }
    assert!(
        !out.is_empty(),
        "the manifest parse found no [payload] entries"
    );
    out
}

/// The paths of `schema` that hold a string or an open value, as JSON pointers (`*` for an array's
/// items, `?` for a map's keys). A `format: uuid` or `date-time` string, and a string confined by
/// `enum` or `const`, are classed by the walk itself and not returned (the manifest's header).
fn string_paths(schema: &serde_json::Value) -> BTreeSet<String> {
    // Every branch below adds to `out` and none stops the others: a node may carry `$ref` or
    // composition arms beside its own `properties` (a `#[serde(flatten)]` enum does), and a type
    // list may admit a string beside an object. A node with no structure at all is an open value.
    fn walk(
        node: &serde_json::Value,
        root: &serde_json::Value,
        path: &str,
        depth: usize,
        out: &mut BTreeSet<String>,
    ) {
        assert!(depth < 32, "payload schema recursion at {path}");
        let obj = match node {
            serde_json::Value::Bool(true) => {
                out.insert(path.to_string());
                return;
            }
            serde_json::Value::Object(o) => o,
            _ => return,
        };
        let mut structured = false;
        if let Some(r) = obj.get("$ref").and_then(|v| v.as_str()) {
            let name = r.rsplit('/').next().unwrap();
            let target = root
                .get("$defs")
                .and_then(|d| d.get(name))
                .unwrap_or_else(|| panic!("unresolved $ref {r} at {path}"));
            walk(target, root, path, depth + 1, out);
            structured = true;
        }
        for key in ["anyOf", "oneOf", "allOf"] {
            if let Some(arms) = obj.get(key).and_then(|v| v.as_array()) {
                for arm in arms {
                    walk(arm, root, path, depth + 1, out);
                }
                structured = true;
            }
        }
        if obj.contains_key("enum") || obj.contains_key("const") {
            return;
        }
        let types: Vec<&str> = match obj.get("type") {
            Some(serde_json::Value::String(s)) => vec![s.as_str()],
            Some(serde_json::Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).collect(),
            _ => vec![],
        };
        let props = obj.get("properties").and_then(|v| v.as_object());
        let patterns = obj.get("patternProperties").and_then(|v| v.as_object());
        if types.contains(&"object") || props.is_some() || patterns.is_some() {
            for (k, v) in props.into_iter().flatten() {
                walk(v, root, &format!("{path}/{k}"), depth + 1, out);
            }
            for v in patterns.into_iter().flat_map(|p| p.values()) {
                walk(v, root, &format!("{path}/?"), depth + 1, out);
            }
            match obj.get("additionalProperties") {
                Some(serde_json::Value::Bool(false)) => {}
                // An object that names nothing and closes nothing holds anything.
                None if props.is_none() && patterns.is_none() && !structured => {
                    out.insert(path.to_string());
                }
                None => {}
                Some(ap) => walk(ap, root, &format!("{path}/?"), depth + 1, out),
            }
            structured = true;
        }
        if types.contains(&"array") || obj.contains_key("items") || obj.contains_key("prefixItems")
        {
            let tuple = ["items", "prefixItems"]
                .iter()
                .filter_map(|k| obj.get(*k))
                .flat_map(|v| match v {
                    serde_json::Value::Array(a) => a.iter().collect::<Vec<_>>(),
                    other => vec![other],
                });
            for item in tuple {
                walk(item, root, &format!("{path}/*"), depth + 1, out);
            }
            structured = true;
        }
        if types.contains(&"string") {
            let format = obj.get("format").and_then(|v| v.as_str());
            if !matches!(format, Some("uuid" | "date-time")) {
                out.insert(path.to_string());
            }
            structured = true;
        }
        if types.is_empty() && !structured {
            out.insert(path.to_string());
        }
    }
    let mut out = BTreeSet::new();
    walk(schema, schema, "", 0, &mut out);
    out
}

/// The committed payload-schema snapshots, emitted from the structs `fire()` serializes
/// (temper-substrate `tests/payload_schema.rs`), keyed by event type at their latest version.
const SCHEMA_SNAPSHOTS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../temper-substrate/tests/fixtures/payloads"
);

fn schema_snapshots() -> BTreeMap<String, serde_json::Value> {
    let mut by_type: BTreeMap<String, (u32, serde_json::Value)> = BTreeMap::new();
    for entry in std::fs::read_dir(SCHEMA_SNAPSHOTS).expect("read the payload schema snapshots") {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(stem) = file.strip_suffix(".schema.json") else {
            continue;
        };
        let (name, version) = stem.rsplit_once(".v").expect("<type>.v<n>.schema.json");
        let version: u32 = version.parse().expect("a numeric schema version");
        let schema = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        if by_type.get(name).is_none_or(|(v, _)| *v < version) {
            by_type.insert(name.to_string(), (version, schema));
        }
    }
    by_type.into_iter().map(|(k, (_, s))| (k, s)).collect()
}

/// Each registered event type with every schema the walk reads: its committed snapshot, and its
/// registered `payload_schema`. Empty is a type with neither (a permissive type).
///
/// **Both are walked.** A migrated database's `payload_schema` is stamped by the migration that
/// registered the type and lags the code: on 2026-10-08 it had no `incorporated` on the block
/// events and no `role` on `block_created`, all of which the code writes, and the snapshots are
/// emitted from the structs the code serializes. But a field a struct has since dropped is still
/// on the ledger's older rows, and only the registry may still name it, so neither schema alone is
/// the ledger's shape.
async fn registry(pool: &PgPool) -> BTreeMap<String, Vec<serde_json::Value>> {
    let snapshots = schema_snapshots();
    sqlx::query_as::<_, (String, Option<serde_json::Value>)>(
        "SELECT name, payload_schema FROM kb_event_types",
    )
    .fetch_all(pool)
    .await
    .expect("read the event registry")
    .into_iter()
    .map(|(name, registered)| {
        let schemas = snapshots
            .get(&name)
            .cloned()
            .into_iter()
            .chain(registered)
            .collect();
        (name, schemas)
    })
    .collect()
}

/// `(event_type, path)` of every string path the registered schemas nominate.
fn walked(registry: &BTreeMap<String, Vec<serde_json::Value>>) -> BTreeSet<(String, String)> {
    registry
        .iter()
        .flat_map(|(name, schemas)| {
            schemas
                .iter()
                .flat_map(string_paths)
                .map(move |p| (name.clone(), p))
        })
        .collect()
}

/// The schema of what `EventContext` writes into `kb_events.metadata`, emitted from
/// `AgentAuthorship` (temper-substrate `tests/payload_schema.rs`).
const METADATA_SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../temper-substrate/tests/fixtures/metadata/authorship.v1.schema.json"
);

/// `("metadata", path)` of every string path an event's metadata can carry.
fn metadata_walked() -> BTreeSet<(String, String)> {
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(METADATA_SNAPSHOT).expect("read the metadata schema snapshot"),
    )
    .unwrap();
    string_paths(&schema)
        .into_iter()
        .map(|p| ("metadata".to_string(), p))
        .collect()
}

/// The `[payload]` problems against `registry`: undeclared walked paths, entries naming a walked
/// type's path the schema does not have, and permissive declarations that disagree with the
/// registry.
fn payload_problems(registry: &BTreeMap<String, Vec<serde_json::Value>>) -> Vec<String> {
    let lines = payload_entries();
    let mut walked = walked(registry);
    walked.extend(metadata_walked());
    let declared: BTreeSet<(String, String)> = lines
        .iter()
        .filter(|l| l.qualifier.is_none() && l.path != "*")
        .map(|l| (l.event_type.to_string(), l.path.to_string()))
        .collect();
    let permissive: BTreeSet<&str> = lines
        .iter()
        .filter(|l| *l.disposition == PayloadDisposition::Permissive)
        .map(|l| l.event_type)
        .collect();
    let mut problems = Vec::new();
    for (ty, path) in walked.difference(&declared) {
        problems.push(format!(
            "undeclared: {ty}:{path} (its payload schema holds a string there)"
        ));
    }
    for l in &lines {
        let at = l.at();
        let walked_here = walked.contains(&(l.event_type.to_string(), l.path.to_string()));
        if l.event_type == "metadata" {
            if !walked_here {
                problems.push(format!(
                    "{at}: stale: metadata:{} is not a string path of AgentAuthorship",
                    l.path
                ));
            }
            continue;
        }
        match registry.get(l.event_type).map(Vec::is_empty) {
            None => problems.push(format!(
                "{at}: {} is not a registered event type",
                l.event_type
            )),
            Some(false) if l.path == "*" => problems.push(format!(
                "{at}: {} has a payload schema, so the walk reads it; drop `permissive`",
                l.event_type
            )),
            Some(false) if l.qualifier.is_none() && !walked_here => problems.push(format!(
                "{at}: stale: {}:{} is not a string path of its schema",
                l.event_type, l.path
            )),
            Some(true) if !permissive.contains(l.event_type) => problems.push(format!(
                "{at}: {ty} has no payload schema; declare `[payload.\"{ty}\"] \"*\" = \
                 {{ disposition = \"permissive\", note = \"<where its fields came from>\" }}`",
                ty = l.event_type
            )),
            _ => {}
        }
        // A refinement with no unqualified entry to refine cannot be written: it nests under its
        // entry (`temper_core::erasure_policy::PayloadLine::refine`), so the parser enforces it.
        if *l.disposition == PayloadDisposition::OutOfScope && l.note.is_empty() {
            problems.push(format!("{at}: `out-of-scope` needs its reason"));
        }
    }
    for (name, schemas) in registry {
        if schemas.is_empty() && !permissive.contains(name.as_str()) {
            problems.push(format!(
                "undeclared: {name} is registered with no payload schema; declare \
                 `[payload.\"{name}\"] \"*\" = {{ disposition = \"permissive\", note = \"…\" }}` \
                 and its string paths"
            ));
        }
    }
    problems
}

/// FAILS IF: a string a registered payload schema holds has no `[payload]` line, a line names a
/// path or type the registry does not have, or a schema-less type is not declared permissive.
/// This is D9's payload half: a new free-text field cannot ship without a disposition (goal §8).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_payload_string_path_is_declared(pool: PgPool) {
    let problems = payload_problems(&registry(&pool).await);
    assert!(
        problems.is_empty(),
        "policy/erasure/resource-erasure.toml [payload] disagrees with the registered payload \
         schemas (kb_event_types). Declare each new string path `redact:<class>`, `structural` or \
         `out-of-scope` with its reason, and delete an entry whose path is gone.\n\
         Problems: {problems:#?}"
    );
}

/// A manifest JSON pointer in the dotted form `redacted_fields` records: `/blocks/*/x` is
/// `blocks[*].x`, and a metadata key is `metadata.<key>`.
fn dotted(event_type: &str, pointer: &str) -> String {
    let mut out = String::new();
    for seg in pointer.trim_start_matches('/').split('/') {
        if seg == "*" {
            out.push_str("[*]");
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(seg);
        }
    }
    if event_type == "metadata" {
        format!("metadata.{out}")
    } else {
        out
    }
}

/// `(event_type, path, key qualifier, owner qualifier, class)` as `_erasure_redact_paths` holds it.
type AllowlistRow = (
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    String,
);

/// FAILS IF: the ledger exception's allowlist (`_erasure_redact_paths`, migration
/// 20261009100000) and the manifest's `redact` entries disagree. Every `redact:<class>` entry is a
/// row of that class, refinement or not, and every `structural` refinement is a `keep` row (it
/// refines a redact entry back to structural); nothing else is. D9: the verifier's allowlist is
/// generated from the redact entries, and this is what holds it to them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_allowlist_is_the_manifests_redact_lines(pool: PgPool) {
    let live: BTreeSet<AllowlistRow> = sqlx::query_as(
        "SELECT event_type, path, key_qualifier, owner_qualifier, class FROM _erasure_redact_paths()",
    )
    .fetch_all(&pool)
    .await
    .expect("read the allowlist")
    .into_iter()
    .collect();
    let manifest: BTreeSet<AllowlistRow> = payload_entries()
        .into_iter()
        .filter_map(|l| {
            let class = match (l.disposition, l.qualifier) {
                (PayloadDisposition::Redact(class), _) => class.clone(),
                (PayloadDisposition::Structural, Some(_)) => "keep".to_string(),
                _ => return None,
            };
            let key = l.qualifier.and_then(|q| q.key.clone());
            let owner = l.qualifier.and_then(|q| q.owner.clone());
            let event_type = (l.event_type != "metadata").then(|| l.event_type.to_string());
            Some((event_type, dotted(l.event_type, l.path), key, owner, class))
        })
        .collect();
    assert_eq!(
        manifest.difference(&live).collect::<Vec<_>>(),
        Vec::<&AllowlistRow>::new(),
        "manifest lines missing from _erasure_redact_paths()"
    );
    assert_eq!(
        live.difference(&manifest).collect::<Vec<_>>(),
        Vec::<&AllowlistRow>::new(),
        "_erasure_redact_paths() rows the manifest does not hold"
    );
}

/// FAILS IF: a string field in a newly registered schema, or a type registered without one, is
/// not reported. The bite for `every_payload_string_path_is_declared`: both probes are the ways a
/// new free-text field reaches the ledger (a field added to an existing struct changes its
/// committed snapshot, which `string_paths` reads the same way). Each `sqlx::test` runs in its own
/// database.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_added_payload_string_is_reported(pool: PgPool) {
    assert!(
        payload_problems(&registry(&pool).await).is_empty(),
        "precondition: the payload fence holds before the probes are added"
    );
    sqlx::query(
        "INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
         SELECT p.name, p.schema, 1, t.category
           FROM kb_event_types t,
                (VALUES ('erasure_fence_probe_typed',
                         '{\"type\": \"object\", \"properties\": {\"note\": {\"type\": \"string\"}}}'::jsonb),
                        ('erasure_fence_probe_permissive', NULL::jsonb)) AS p(name, schema)
          WHERE t.name = 'relationship_folded'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let problems = payload_problems(&registry(&pool).await);
    for want in [
        "undeclared: erasure_fence_probe_typed:/note",
        "undeclared: erasure_fence_probe_permissive is registered with no payload schema",
    ] {
        assert!(
            problems.iter().any(|p| p.starts_with(want)),
            "the fence missed {want:?}; it reported {problems:#?}"
        );
    }
}

/// FAILS IF: the walk stops nominating what it must, or nominates an id or a vocabulary word. A
/// walk that returned nothing would make every payload line read stale rather than uncovered, so
/// its own shape is pinned here, against a schema written for the purpose.
#[test]
fn the_walk_nominates_strings_and_open_values_only() {
    let schema = serde_json::json!({
        "$defs": {
            "Id": {"type": "string", "format": "uuid"},
            "Src": {"type": "object", "properties": {
                "kind": {"type": "string", "enum": ["remote", "resource"]},
                "value": {"type": "string"}}}
        },
        "type": "object",
        "properties": {
            "title": {"type": ["string", "null"]},
            "id": {"$ref": "#/$defs/Id"},
            "at": {"type": "string", "format": "date-time"},
            "n": {"type": "integer"},
            "value": {},
            "tags": {"type": "array", "items": {"type": "string"}},
            "srcs": {"type": "array", "items": {"$ref": "#/$defs/Src"}},
            "map": {"type": "object", "additionalProperties": {"type": "string"}},
            "either": {"anyOf": [{"type": "null"}, {"type": "string"}]},
            "mode": {"const": "x"},
            "flat": {"type": "object",
                     "oneOf": [{"properties": {"a": {"type": "string"}}}],
                     "properties": {"b": {"type": "string"}}},
            "both": {"type": ["string", "object"], "properties": {"c": {"type": "string"}}},
            "pair": {"type": "array", "prefixItems": [{"type": "integer"}, {"type": "string"}]},
            "pats": {"type": "object", "patternProperties": {"^x": {"type": "string"}}},
            "open": {"type": "object"}
        }
    });
    let got: Vec<String> = string_paths(&schema).into_iter().collect();
    assert_eq!(
        got,
        [
            "/both",
            "/both/c",
            "/either",
            "/flat/a",
            "/flat/b",
            "/map/?",
            "/open",
            "/pair/*",
            "/pats/?",
            "/srcs/*/value",
            "/tags/*",
            "/title",
            "/value"
        ]
    );
}
