#![cfg(feature = "test-db")]
//! The estate manifest covers the catalog: every content or identity table has exactly one grain
//! in `policy/erasure/estate.toml`, every grain names live tables and columns, every link is the
//! kind it says, every bound mechanism exists, every covered chain ends in a unit, and every scan
//! column of an identity table is reached by the person act or recorded as a gap, never both.
//!
//! **Which tables must have a grain is derived, not listed.** A content or identity table is one
//! with at least one column classed `content`, `incidental`, `derived` or `identifier` in
//! `personal-data.toml`, or `scan` in `sensitivity-scan.toml`. Those two files are already held to
//! the live catalog (`personal_data_surface_test.rs`, `sensitivity_scan_surface_test.rs`), so a new
//! carrier column lands in one of them, and through the derivation here it demands a grain. The
//! derivation is a function over an [`ErasurePolicy`], so the bite probes run it on a policy they
//! changed rather than on the shipped one.
//!
//! **One grain per table is structural**: the grain is the value under a TOML table key, and a key
//! declared twice fails the parse. A table reached by a second path records it as `also_reached`.
//! A gap's task and description are required by the parser, so nothing here checks them.
//!
//! **What the catalog check reads.** Names and liveness: tables, link columns, home columns, flag
//! columns, an `also_reached` entry's `column`, function names, foreign-key constraints, and the
//! vocabulary of a `cited_by` discriminator (its enum labels, or the literals of its CHECK). It
//! does not read what a bound function does, except for the scan columns of identity tables,
//! which it binds to the person act's statements the way `personal_data_surface_test.rs` binds an
//! `erased` claim: statements, not the rows they reach or the branches that run them.

mod erasure_binding;
mod person_act_binding;

use std::collections::BTreeSet;

use erasure_binding::{deletes_from, tokens};
use person_act_binding::assigns_erasure;
use sqlx::PgPool;
use temper_core::erasure_policy::{
    Achieved, Ceiling, Class, ErasurePolicy, Grain, Link, Mechanism, ParentPath, PersonalColumn,
    ScanColumn, ScanDisposition,
};
use temper_services::erasure_policy::in_force;

/// The personal-data classes that make a table a content or identity table.
const CARRIER_CLASSES: &[Class] = &[
    Class::Content,
    Class::Incidental,
    Class::Derived,
    Class::Identifier,
];

/// The person act's redaction: what an identity grain's scan columns are bound to.
const REDACTION_FN: &str = "_erasure_apply_redaction";

/// The tables whose columns carry content or identity, derived from `policy`'s column entries.
fn content_or_identity_tables(policy: &ErasurePolicy) -> BTreeSet<&str> {
    let personal = CARRIER_CLASSES
        .iter()
        .flat_map(move |&class| policy.personal_data_keys(class));
    let scanned = policy.sensitivity_scan.keys(ScanDisposition::Scan);
    personal.chain(scanned).map(|key| key.table()).collect()
}

/// The derived content or identity tables with no grain in `policy`'s estate.
fn ungrained(policy: &ErasurePolicy) -> Vec<&str> {
    content_or_identity_tables(policy)
        .into_iter()
        .filter(|table| !policy.estate.tables.contains_key(*table))
        .collect()
}

/// What a `Bound` name must be in the catalog.
#[derive(Debug)]
enum Binds<'a> {
    /// A function in `public`.
    Function,
    /// A flag column on the grained table.
    Column { table: &'a str },
}

/// One place `policy` names a mechanism.
#[derive(Debug)]
struct Site<'a> {
    at: String,
    binds: Binds<'a>,
    mechanism: &'a Mechanism,
}

/// Every mechanism `policy`'s estate names: scope rules, per-item fences and units, covered units,
/// identity mechanisms, and every `also_reached` path's mechanism.
fn mechanisms(policy: &ErasurePolicy) -> Vec<Site<'_>> {
    let estate = &policy.estate;
    let mut sites: Vec<Site<'_>> = estate
        .scope
        .iter()
        .map(|rule| Site {
            at: format!("scope rule {:?}", rule.rule),
            binds: Binds::Function,
            mechanism: &rule.mechanism,
        })
        .collect();
    for (table, entry) in &estate.tables {
        match &entry.grain {
            Grain::PerItem { fence, unit, .. } => {
                sites.push(Site {
                    at: format!("{table} fence"),
                    binds: Binds::Column { table },
                    mechanism: fence,
                });
                sites.push(Site {
                    at: format!("{table} unit"),
                    binds: Binds::Function,
                    mechanism: unit,
                });
            }
            Grain::Covered {
                unit: Some(unit), ..
            } => sites.push(Site {
                at: format!("{table} unit"),
                binds: Binds::Function,
                mechanism: unit,
            }),
            Grain::Identity { mechanism, .. } => sites.push(Site {
                at: format!("{table} identity"),
                binds: Binds::Function,
                mechanism,
            }),
            Grain::Covered { unit: None, .. } | Grain::DeclaredOut { .. } => {}
        }
        for reach in &entry.also_reached {
            sites.push(Site {
                at: format!("{table} also_reached {}", reach.path.table),
                binds: Binds::Function,
                mechanism: &reach.mechanism,
            });
        }
    }
    sites
}

/// The public catalog: tables and views, their columns, function names, single-column foreign
/// keys, and the value vocabulary of each column that has one.
#[derive(Debug)]
struct Catalog {
    tables: BTreeSet<String>,
    columns: BTreeSet<(String, String)>,
    functions: BTreeSet<String>,
    /// `(table, column, referenced table)` for every single-column foreign key.
    foreign_keys: BTreeSet<(String, String, String)>,
    /// `(table, column, value)`: an enum column's labels, and the literals of a single-column
    /// CHECK.
    vocabulary: BTreeSet<(String, String, String)>,
}

impl Catalog {
    fn has_column(&self, table: &str, column: &str) -> bool {
        self.columns
            .contains(&(table.to_owned(), column.to_owned()))
    }

    fn has_foreign_key(&self, table: &str, column: &str, to: &str) -> bool {
        self.foreign_keys
            .contains(&(table.to_owned(), column.to_owned(), to.to_owned()))
    }

    fn has_any_foreign_key(&self, table: &str, column: &str) -> bool {
        self.foreign_keys
            .iter()
            .any(|(t, c, _)| t == table && c == column)
    }

    fn admits(&self, table: &str, column: &str, value: &str) -> bool {
        self.vocabulary
            .contains(&(table.to_owned(), column.to_owned(), value.to_owned()))
    }
}

const FOREIGN_KEYS: &str = "
  SELECT c.conrelid::regclass::text, a.attname::text, c.confrelid::regclass::text
    FROM pg_constraint c
    JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
   WHERE c.contype = 'f' AND c.connamespace = 'public'::regnamespace
     AND cardinality(c.conkey) = 1";

const VOCABULARY: &str = "
  SELECT c.table_name::text, c.column_name::text, e.enumlabel::text
    FROM information_schema.columns c
    JOIN pg_type t ON t.typname = c.udt_name AND t.typtype = 'e'
    JOIN pg_enum e ON e.enumtypid = t.oid
   WHERE c.table_schema = 'public'
  UNION
  SELECT con.conrelid::regclass::text, a.attname::text, m[1]
    FROM pg_constraint con
    JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = con.conkey[1]
    CROSS JOIN LATERAL regexp_matches(pg_get_constraintdef(con.oid), '''([^'']*)''', 'g') AS m
   WHERE con.contype = 'c' AND con.connamespace = 'public'::regnamespace
     AND cardinality(con.conkey) = 1";

async fn catalog(pool: &PgPool) -> Catalog {
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_all(pool)
    .await
    .expect("read the public tables");
    let columns = sqlx::query_as::<_, (String, String)>(
        "SELECT table_name::text, column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public'",
    )
    .fetch_all(pool)
    .await
    .expect("read the public columns");
    let functions = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT proname::text FROM pg_proc WHERE pronamespace = 'public'::regnamespace",
    )
    .fetch_all(pool)
    .await
    .expect("read the public functions");
    let foreign_keys = sqlx::query_as::<_, (String, String, String)>(FOREIGN_KEYS)
        .fetch_all(pool)
        .await
        .expect("read the single-column foreign keys");
    let vocabulary = sqlx::query_as::<_, (String, String, String)>(VOCABULARY)
        .fetch_all(pool)
        .await
        .expect("read the enum labels and CHECK literals");
    Catalog {
        tables: tables.into_iter().collect(),
        columns: columns.into_iter().collect(),
        functions: functions.into_iter().collect(),
        foreign_keys: foreign_keys.into_iter().collect(),
        vocabulary: vocabulary.into_iter().collect(),
    }
}

/// The problems with one path from a row of `row_table`. The table it reaches must exist. Its link
/// columns must exist on `row_table`, or, for a `cited_by` path, on the citing table it names. An
/// `fk` link must be a foreign key from the link's table to the other table, and a `ref` link must
/// have no foreign key. A `cited_by` value must be in its discriminator's vocabulary.
fn path_problems(at: &str, row_table: &str, path: &ParentPath, catalog: &Catalog) -> Vec<String> {
    let mut problems = Vec::new();
    if !catalog.tables.contains(&path.table) {
        problems.push(format!("{at}: reaches no table {}", path.table));
    }
    let (link_table, other_table) = if path.cited_by.is_some() {
        (path.table.as_str(), row_table)
    } else {
        (row_table, path.table.as_str())
    };
    let columns: Vec<&str> = match &path.link {
        Link::Fk(column) | Link::Ref(column) => vec![column.as_str()],
        Link::Pair {
            table_column,
            id_column,
        } => vec![table_column.as_str(), id_column.as_str()],
    };
    for column in columns {
        if !catalog.has_column(link_table, column) {
            problems.push(format!("{at}: no column {link_table}.{column}"));
        }
    }
    match &path.link {
        Link::Fk(column) if !catalog.has_foreign_key(link_table, column, other_table) => {
            problems.push(format!(
                "{at}: no foreign key {link_table}.{column} -> {other_table}"
            ));
        }
        Link::Ref(column) if catalog.has_any_foreign_key(link_table, column) => {
            problems.push(format!(
                "{at}: {link_table}.{column} has a foreign key; the link is an `fk`"
            ));
        }
        Link::Fk(_) | Link::Ref(_) | Link::Pair { .. } => {}
    }
    if let Some(cited_by) = &path.cited_by {
        if !catalog.has_column(link_table, &cited_by.column) {
            problems.push(format!("{at}: no column {link_table}.{}", cited_by.column));
        } else if !catalog.admits(link_table, &cited_by.column, &cited_by.value) {
            problems.push(format!(
                "{at}: {:?} is not in the vocabulary of {link_table}.{}",
                cited_by.value, cited_by.column
            ));
        }
    }
    problems
}

/// Every name `policy`'s estate gives that `catalog` does not have, and every link that is not the
/// kind it says.
fn catalog_problems(policy: &ErasurePolicy, catalog: &Catalog) -> Vec<String> {
    let mut problems = Vec::new();
    for (table, entry) in &policy.estate.tables {
        if !catalog.tables.contains(table) {
            problems.push(format!("{table}: no such table"));
        }
        match &entry.grain {
            Grain::PerItem {
                home: Some(home), ..
            } => {
                if !catalog.tables.contains(&home.table) {
                    problems.push(format!("{table} home: no table {}", home.table));
                }
                for column in [&home.table_column, &home.id_column] {
                    if !catalog.has_column(&home.table, column) {
                        problems.push(format!("{table} home: no column {}.{column}", home.table));
                    }
                }
            }
            Grain::Covered { parent, .. } => {
                problems.extend(path_problems(
                    &format!("{table} parent"),
                    table,
                    parent,
                    catalog,
                ));
            }
            Grain::PerItem { home: None, .. }
            | Grain::Identity { .. }
            | Grain::DeclaredOut { .. } => {}
        }
        for reach in &entry.also_reached {
            problems.extend(path_problems(
                &format!("{table} also_reached"),
                table,
                &reach.path,
                catalog,
            ));
            if let Some(column) = &reach.column {
                if !catalog.has_column(table, column) {
                    problems.push(format!("{table} also_reached: no column {table}.{column}"));
                }
            }
        }
    }
    for site in mechanisms(policy) {
        let Mechanism::Bound(name) = site.mechanism else {
            continue;
        };
        match site.binds {
            Binds::Function if !catalog.functions.contains(name) => {
                problems.push(format!("{}: no function {name}", site.at));
            }
            Binds::Column { table } if !catalog.has_column(table, name) => {
                problems.push(format!("{}: no flag column {table}.{name}", site.at));
            }
            Binds::Function | Binds::Column { .. } => {}
        }
    }
    problems
}

/// Every covered table whose chain of parents does not end in a unit: a `per-item` or `identity`
/// grain, or a `covered` grain that names its own `unit`. A chain fails at an ungrained or
/// `declared-out` table, and on a cycle.
fn unended_chains(policy: &ErasurePolicy) -> Vec<String> {
    let tables = &policy.estate.tables;
    let mut problems = Vec::new();
    let covered = tables
        .iter()
        .filter(|(_, entry)| matches!(entry.grain, Grain::Covered { .. }))
        .map(|(table, _)| table);
    for table in covered {
        let mut seen = BTreeSet::from([table.as_str()]);
        let mut at = table.as_str();
        loop {
            let next = match tables.get(at).map(|entry| &entry.grain) {
                Some(Grain::Covered { parent, unit: None }) => parent.table.as_str(),
                Some(
                    Grain::Covered { unit: Some(_), .. }
                    | Grain::PerItem { .. }
                    | Grain::Identity { .. },
                ) => break,
                Some(Grain::DeclaredOut { .. }) => {
                    problems.push(format!(
                        "{table}: its covered chain reaches {at}, which is declared out"
                    ));
                    break;
                }
                None => {
                    problems.push(format!(
                        "{table}: its covered chain reaches {at}, which has no grain"
                    ));
                    break;
                }
            };
            if !seen.insert(next) {
                problems.push(format!("{table}: its covered chain cycles at {next}"));
                break;
            }
            at = next;
        }
    }
    problems
}

/// Each `scan` column of an identity-grained table, with whether the person act's redaction
/// reaches it (an UPDATE assigning it an erasure value, or a DELETE of its table) and whether an
/// `also_reached` gap scoped to that column records it. `redaction` is the act's body, tokenized.
fn identity_scan_columns(
    policy: &ErasurePolicy,
    redaction: &[String],
) -> Vec<(String, bool, bool)> {
    policy
        .sensitivity_scan
        .keys(ScanDisposition::Scan)
        .filter_map(|key| {
            let entry = policy.estate.tables.get(key.table())?;
            if !matches!(entry.grain, Grain::Identity { .. }) {
                return None;
            }
            let reached =
                assigns_erasure(redaction, key.as_str()) || deletes_from(redaction, key.table());
            let gapped = entry.also_reached.iter().any(|reach| {
                reach.column.as_deref() == Some(key.column())
                    && matches!(reach.mechanism, Mechanism::Gap(_))
            });
            Some((key.to_string(), reached, gapped))
        })
        .collect()
}

/// The identity `scan` columns the act does not reach and no column-scoped gap records.
fn unreached_identity_scan_columns(policy: &ErasurePolicy, redaction: &[String]) -> Vec<String> {
    identity_scan_columns(policy, redaction)
        .into_iter()
        .filter(|(_, reached, gapped)| !reached && !gapped)
        .map(|(key, ..)| key)
        .collect()
}

/// The identity `scan` columns a column-scoped gap records although the act already reaches them:
/// a gap left standing after its work was done.
fn stale_identity_scan_gaps(policy: &ErasurePolicy, redaction: &[String]) -> Vec<String> {
    identity_scan_columns(policy, redaction)
        .into_iter()
        .filter(|(_, reached, gapped)| *reached && *gapped)
        .map(|(key, ..)| key)
        .collect()
}

async fn redaction(pool: &PgPool) -> Vec<String> {
    let body = sqlx::query_scalar::<_, String>(&format!(
        "SELECT pg_get_functiondef('{REDACTION_FN}'::regproc)"
    ))
    .fetch_one(pool)
    .await
    .expect("read the person act's redaction");
    tokens(&body)
}

/// FAILS IF: a table with a content or identity column has no grain, so no erasure path says how
/// its rows are reached.
#[test]
fn every_content_or_identity_table_has_exactly_one_grain() {
    let policy = in_force();
    assert!(
        !content_or_identity_tables(policy).is_empty(),
        "the derivation found no content or identity table; it is reading nothing"
    );
    let missing = ungrained(policy);
    assert!(
        missing.is_empty(),
        "these tables carry content or identity columns (personal-data.toml content, incidental, \
         derived or identifier; sensitivity-scan.toml scan) and have no grain in \
         policy/erasure/estate.toml.\n\
         Add a `[tables.<table>]` entry with a `grain`: per-item, covered (with its parent path), \
         identity, or declared-out (with its reason).\n\
         Ungrained: {missing:#?}"
    );
}

/// The derivation's bite, through both of its sources: a new scan column, and a new personal-data
/// column classed `content`, each on a table nobody grained, are reported, and nothing else is.
#[test]
fn an_added_carrier_table_without_a_grain_is_reported() {
    let mut policy = in_force().clone();
    policy.sensitivity_scan.columns.insert(
        "kb_probe_t.note".parse().expect("a well-formed column key"),
        ScanColumn {
            disposition: ScanDisposition::Scan,
            note: String::new(),
        },
    );
    policy.personal_data.insert(
        "kb_probe_u.body".parse().expect("a well-formed column key"),
        PersonalColumn {
            class: Class::Content,
            ceiling: Ceiling::Full,
            achieved: Achieved::Unstated,
            note: String::new(),
        },
    );
    assert_eq!(ungrained(&policy), ["kb_probe_t", "kb_probe_u"]);
}

/// FAILS IF: a grain names a table, a link column, a home column, a flag column or a reached column
/// the schema does not have, or a bound function that does not exist; an `fk` link has no foreign
/// key or a `ref` link has one; or a `cited_by` value is not in its discriminator's vocabulary.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_grain_names_live_tables_and_columns(pool: PgPool) {
    let problems = catalog_problems(in_force(), &catalog(&pool).await);
    assert!(
        problems.is_empty(),
        "policy/erasure/estate.toml names what the live schema does not have:\n{problems:#?}"
    );
}

/// The catalog check's bite: a bound unit renamed to a function that does not exist is reported,
/// and nothing else is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_bound_function_that_does_not_exist_is_reported(pool: PgPool) {
    let mut policy = in_force().clone();
    let resources = policy
        .estate
        .tables
        .get_mut("kb_resources")
        .expect("kb_resources is grained");
    let Grain::PerItem { unit, .. } = &mut resources.grain else {
        panic!("kb_resources is per-item; the probe expects its unit");
    };
    *unit = Mechanism::Bound("resource_erasure_execute_probe".into());

    let problems = catalog_problems(&policy, &catalog(&pool).await);
    assert_eq!(
        problems,
        ["kb_resources unit: no function resource_erasure_execute_probe"]
    );
}

/// The link-kind check's bite, three ways: a constrained column declared `ref`, an unconstrained
/// one declared `fk`, and an `fk` whose constraint references a table other than the one named,
/// are each reported, and nothing else is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_link_of_the_wrong_kind_is_reported(pool: PgPool) {
    let mut policy = in_force().clone();
    let tables = &mut policy.estate.tables;
    let blocks = tables
        .get_mut("kb_content_blocks")
        .expect("kb_content_blocks is grained");
    let Grain::Covered { parent, .. } = &mut blocks.grain else {
        panic!("kb_content_blocks is covered; the probe expects its parent");
    };
    assert_eq!(parent.link, Link::Fk("resource_id".into()), "precondition");
    parent.link = Link::Ref("resource_id".into());
    let teams = tables.get_mut("kb_teams").expect("kb_teams is grained");
    let reach = teams
        .also_reached
        .iter_mut()
        .find(|reach| reach.column.as_deref() == Some("description"))
        .expect("kb_teams records its description");
    assert_eq!(
        reach.path.link,
        Link::Ref("personal_of".into()),
        "precondition"
    );
    reach.path.link = Link::Fk("personal_of".into());
    let chunks = tables.get_mut("kb_chunks").expect("kb_chunks is grained");
    let Grain::Covered { parent, .. } = &mut chunks.grain else {
        panic!("kb_chunks is covered; the probe expects its parent");
    };
    assert_eq!(
        (parent.table.as_str(), &parent.link),
        ("kb_resources", &Link::Fk("resource_id".into())),
        "precondition"
    );
    parent.table = "kb_content_blocks".into();

    let problems = catalog_problems(&policy, &catalog(&pool).await);
    assert_eq!(
        problems,
        [
            "kb_chunks parent: no foreign key kb_chunks.resource_id -> kb_content_blocks",
            "kb_content_blocks parent: kb_content_blocks.resource_id has a foreign key; the link \
             is an `fk`",
            "kb_teams also_reached: no foreign key kb_teams.personal_of -> kb_profiles",
        ]
    );
}

/// The vocabulary check's bite: a `cited_by` value its discriminator does not admit is reported,
/// and nothing else is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_cited_by_value_outside_the_vocabulary_is_reported(pool: PgPool) {
    let mut policy = in_force().clone();
    let sources = policy
        .estate
        .tables
        .get_mut("kb_remote_sources")
        .expect("kb_remote_sources is grained");
    let Grain::Covered { parent, .. } = &mut sources.grain else {
        panic!("kb_remote_sources is covered; the probe expects its parent");
    };
    let cited_by = parent
        .cited_by
        .as_mut()
        .expect("kb_remote_sources is cited by kb_block_provenance");
    cited_by.value = "remote_source".into();

    let problems = catalog_problems(&policy, &catalog(&pool).await);
    assert_eq!(
        problems,
        [
            "kb_remote_sources parent: \"remote_source\" is not in the vocabulary of \
          kb_block_provenance.source_kind"
        ]
    );
}

/// FAILS IF: a covered chain ends anywhere but a unit: at a table with no grain, at a declared-out
/// table, or in a cycle. Then the rows read as covered by a parent nothing erases.
#[test]
fn every_covered_chain_ends_in_a_unit() {
    let problems = unended_chains(in_force());
    assert!(
        problems.is_empty(),
        "these covered entries in policy/erasure/estate.toml reach no unit through their parents. \
         Grain the table the chain stops at, or give the covered entry its own `unit`.\n\
         {problems:#?}"
    );
}

/// The chain check's bite: with kb_block_revisions ungrained, kb_block_content's chain stops
/// there, and nothing else is reported.
#[test]
fn a_covered_chain_through_an_ungrained_parent_is_reported() {
    let mut policy = in_force().clone();
    assert!(
        policy.estate.tables.remove("kb_block_revisions").is_some(),
        "precondition: kb_block_revisions is grained"
    );
    assert_eq!(
        unended_chains(&policy),
        ["kb_block_content: its covered chain reaches kb_block_revisions, which has no grain"]
    );
}

/// The chain check's bite at a declared-out parent: with kb_block_revisions declared out,
/// kb_block_content's chain stops there, and nothing else is reported.
#[test]
fn a_covered_chain_through_a_declared_out_parent_is_reported() {
    let mut policy = in_force().clone();
    let revisions = policy
        .estate
        .tables
        .get_mut("kb_block_revisions")
        .expect("precondition: kb_block_revisions is grained");
    revisions.grain = Grain::DeclaredOut {
        reason: "a probe".into(),
        retention: None,
    };
    assert_eq!(
        unended_chains(&policy),
        ["kb_block_content: its covered chain reaches kb_block_revisions, which is declared out"]
    );
}

/// The chain check's bite on a cycle: with kb_block_revisions' parent pointed back at
/// kb_block_content, each of the two reports the cycle, and nothing else is reported. No other
/// covered entry has either table as its parent, so the cycle is the whole of the change.
#[test]
fn a_covered_chain_that_cycles_is_reported() {
    let mut policy = in_force().clone();
    let revisions = policy
        .estate
        .tables
        .get_mut("kb_block_revisions")
        .expect("precondition: kb_block_revisions is grained");
    let Grain::Covered { parent, .. } = &mut revisions.grain else {
        panic!("kb_block_revisions is covered; the probe expects its parent");
    };
    parent.table = "kb_block_content".into();
    assert_eq!(
        unended_chains(&policy),
        [
            "kb_block_content: its covered chain cycles at kb_block_content",
            "kb_block_revisions: its covered chain cycles at kb_block_revisions",
        ]
    );
}

/// FAILS IF: an identity table has a `scan` column the person act neither erases nor deletes, and
/// no column-scoped gap records it. The grain would then read as erasing the whole row.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_identity_scan_column_is_reached_or_a_gap(pool: PgPool) {
    let unreached = unreached_identity_scan_columns(in_force(), &redaction(&pool).await);
    assert!(
        unreached.is_empty(),
        "these `scan` columns sit on identity-grained tables in policy/erasure/estate.toml, and \
         `{REDACTION_FN}` neither assigns them an erasure value nor deletes their rows. Make the \
         act erase the column, or record it as an `[[tables.<table>.also_reached]]` entry with \
         `column = \"<column>\"` and a `gap` naming the task that tracks it.\n\
         Unreached: {unreached:#?}"
    );
}

/// The identity-scan check's bite: without its column-scoped gap, kb_connections.reach_affirmation
/// is reported, and nothing else is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_identity_scan_column_without_its_gap_is_reported(pool: PgPool) {
    let mut policy = in_force().clone();
    let connections = policy
        .estate
        .tables
        .get_mut("kb_connections")
        .expect("kb_connections is grained");
    let before = connections.also_reached.len();
    connections
        .also_reached
        .retain(|reach| reach.column.as_deref() != Some("reach_affirmation"));
    assert_eq!(
        connections.also_reached.len(),
        before - 1,
        "precondition: kb_connections records reach_affirmation"
    );

    assert_eq!(
        unreached_identity_scan_columns(&policy, &redaction(&pool).await),
        ["kb_connections.reach_affirmation"]
    );
}

/// FAILS IF: a column-scoped gap on an identity table's `scan` column stands although the person
/// act now reaches the column. The gap would keep reading as uncovered work that is done.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn no_identity_scan_gap_is_stale(pool: PgPool) {
    let stale = stale_identity_scan_gaps(in_force(), &redaction(&pool).await);
    assert!(
        stale.is_empty(),
        "`{REDACTION_FN}` now erases these columns, but policy/erasure/estate.toml still records \
         them as column-scoped gaps. Remove the `also_reached` entry and close its task.\n\
         Stale: {stale:#?}"
    );
}

/// The staleness check's bite: a column-scoped gap added on kb_join_requests.message, which the
/// act erases, is reported, and nothing else is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_gap_on_a_column_the_act_reaches_is_reported(pool: PgPool) {
    let mut policy = in_force().clone();
    let tables = &mut policy.estate.tables;
    let mut reach = tables["kb_connections"]
        .also_reached
        .iter()
        .find(|reach| reach.column.as_deref() == Some("reach_affirmation"))
        .expect("precondition: kb_connections records reach_affirmation as a gap")
        .clone();
    reach.column = Some("message".into());
    tables
        .get_mut("kb_join_requests")
        .expect("kb_join_requests is grained")
        .also_reached
        .push(reach);

    assert_eq!(
        stale_identity_scan_gaps(&policy, &redaction(&pool).await),
        ["kb_join_requests.message"]
    );
}
