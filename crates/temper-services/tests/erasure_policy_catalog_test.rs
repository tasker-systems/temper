#![cfg(feature = "test-db")]
//! The estate manifest covers the catalog (spec D7.4, its 1a half): every content or identity table
//! has exactly one grain in `policy/erasure/estate.toml`, every grain names live tables and
//! columns, every bound mechanism exists, and every gap names the slice that builds it.
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
//!
//! **What the catalog check does not see.** It checks names: tables, link columns, home columns,
//! flag columns, an `also_reached` entry's `column`, functions. It does not check that a bound function does what the entry says, and
//! it does not check a `via`'s discriminator value (`source_kind = 'remote'` for
//! `kb_remote_sources`), which is prose. A `via` path's link columns ARE checked, on the citing
//! table the path names. Gaps are only checked to name their slice.

use std::collections::BTreeSet;

use sqlx::PgPool;
use temper_core::erasure_policy::{
    Class, ErasurePolicy, Grain, Link, Mechanism, ParentPath, ScanColumn, ScanDisposition,
};
use temper_services::erasure_policy::in_force;

/// The personal-data classes that make a table a content or identity table.
const CARRIER_CLASSES: &[Class] = &[
    Class::Content,
    Class::Incidental,
    Class::Derived,
    Class::Identifier,
];

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
            Grain::Identity { mechanism } => sites.push(Site {
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

/// The public catalog: tables and views, their columns, and function names.
#[derive(Debug)]
struct Catalog {
    tables: BTreeSet<String>,
    columns: BTreeSet<(String, String)>,
    functions: BTreeSet<String>,
}

impl Catalog {
    fn has_column(&self, table: &str, column: &str) -> bool {
        self.columns
            .contains(&(table.to_owned(), column.to_owned()))
    }
}

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
    Catalog {
        tables: tables.into_iter().collect(),
        columns: columns.into_iter().collect(),
        functions: functions.into_iter().collect(),
    }
}

/// The problems with one path from a row of `row_table`: the table it reaches must exist, and its
/// link columns must exist on `row_table`, or, for a `via` path, on the citing table it names.
fn path_problems(at: &str, row_table: &str, path: &ParentPath, catalog: &Catalog) -> Vec<String> {
    let mut problems = Vec::new();
    if !catalog.tables.contains(&path.table) {
        problems.push(format!("{at}: reaches no table {}", path.table));
    }
    let columns_on = if path.cited_by.is_some() {
        path.table.as_str()
    } else {
        row_table
    };
    let columns: Vec<&str> = match &path.link {
        Link::Fk(column) | Link::Ref(column) => vec![column.as_str()],
        Link::Pair {
            table_column,
            id_column,
        } => vec![table_column.as_str(), id_column.as_str()],
    };
    for column in columns {
        if !catalog.has_column(columns_on, column) {
            problems.push(format!("{at}: no column {columns_on}.{column}"));
        }
    }
    problems
}

/// Every name `policy`'s estate gives that `catalog` does not have.
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

/// The derivation's bite: a new scan column on a table nobody grained is reported, and nothing
/// else is.
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
    assert_eq!(ungrained(&policy), ["kb_probe_t"]);
}

/// FAILS IF: a grain names a table, a link column, a home column, a flag column or a reached column
/// the schema does not have, or a bound function that does not exist. A `via` path's link columns are checked on
/// the citing table; its discriminator value is not checked (see the module comment).
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
