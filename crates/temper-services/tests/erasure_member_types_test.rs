#![cfg(feature = "test-db")]
//! The per-item member types have four copies (erasure estate-membership spec D7.4, its
//! set-equality half): the `per-item` grains of `policy/erasure/estate.toml`, the literals of
//! `kb_erasure_members`' single-column `member_table` CHECK, temper-core's `ErasureMemberTable`,
//! and the fence views `kb_erasure_fenced_<x>` (20261201000010_erasure_records_schema.sql), each
//! read as the table `kb_<x>` it fences. This holds them set-equal, in both directions. Each probe
//! adds a name to one copy only, in memory, and asserts the comparison reports it missing from
//! exactly the other three, so the comparison is shown to see every copy.

use std::collections::BTreeSet;

use sqlx::PgPool;
use temper_core::erasure_policy::Grain;
use temper_core::types::erasure::ErasureMemberTable;

/// Every variant of the enum. A new variant must be added here; the exhaustive `match` in
/// `enum_table` fails to compile until it is.
const VARIANTS: [ErasureMemberTable; 5] = [
    ErasureMemberTable::KbResources,
    ErasureMemberTable::KbContexts,
    ErasureMemberTable::KbCogmaps,
    ErasureMemberTable::KbBlobs,
    ErasureMemberTable::KbEdges,
];

/// Exhaustive over the enum, so a variant outside [`VARIANTS`] is a compile error.
fn enum_table(table: ErasureMemberTable) -> &'static str {
    match table {
        ErasureMemberTable::KbResources => "kb_resources",
        ErasureMemberTable::KbContexts => "kb_contexts",
        ErasureMemberTable::KbCogmaps => "kb_cogmaps",
        ErasureMemberTable::KbBlobs => "kb_blobs",
        ErasureMemberTable::KbEdges => "kb_edges",
    }
}

/// The literals of every single-column CHECK on `kb_erasure_members.member_table`, read as the
/// catalog test's `VOCABULARY` query reads single-column CHECKs.
const MEMBER_TABLE_CHECK: &str = "
  SELECT DISTINCT m[1]
    FROM pg_constraint con
    JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = con.conkey[1]
    CROSS JOIN LATERAL regexp_matches(pg_get_constraintdef(con.oid), '''([^'']*)''', 'g') AS m
   WHERE con.contype = 'c' AND con.connamespace = 'public'::regnamespace
     AND con.conrelid = 'kb_erasure_members'::regclass
     AND cardinality(con.conkey) = 1
     AND a.attname = 'member_table'";

const VIEW_PREFIX: &str = "kb_erasure_fenced_";

type Names = BTreeSet<String>;

#[derive(Debug, Clone)]
struct Copies {
    manifest: Names,
    check: Names,
    enum_values: Names,
    views: Names,
}

fn manifest_copy() -> Names {
    temper_services::erasure_policy::in_force()
        .estate
        .tables
        .iter()
        .filter(|(_, entry)| matches!(entry.grain, Grain::PerItem { .. }))
        .map(|(table, _)| table.clone())
        .collect()
}

fn enum_copy() -> Names {
    VARIANTS
        .iter()
        .map(|variant| {
            let table = serde_json::to_value(variant)
                .expect("a variant serializes")
                .as_str()
                .expect("a variant serializes as a string")
                .to_owned();
            assert_eq!(table, enum_table(*variant), "the wire form is the DDL name");
            table
        })
        .collect()
}

async fn check_copy(pool: &PgPool) -> Names {
    sqlx::query_scalar::<_, String>(MEMBER_TABLE_CHECK)
        .fetch_all(pool)
        .await
        .expect("read the member_table CHECK literals")
        .into_iter()
        .collect()
}

async fn views_copy(pool: &PgPool) -> Names {
    let views: Vec<String> = sqlx::query_scalar(
        "SELECT viewname::text FROM pg_views \
          WHERE schemaname = 'public' AND viewname LIKE 'kb\\_erasure\\_fenced\\_%'",
    )
    .fetch_all(pool)
    .await
    .expect("read the fence views");
    views
        .into_iter()
        .map(|view| {
            let x = view
                .strip_prefix(VIEW_PREFIX)
                .expect("the LIKE pattern selected the prefix");
            format!("kb_{x}")
        })
        .collect()
}

async fn copies(pool: &PgPool) -> Copies {
    Copies {
        manifest: manifest_copy(),
        check: check_copy(pool).await,
        enum_values: enum_copy(),
        views: views_copy(pool).await,
    }
}

/// Every table one copy lacks that another has. Empty when the copies agree.
fn disagreements(c: &Copies) -> Vec<String> {
    let named = [
        ("manifest", &c.manifest),
        ("member_table CHECK", &c.check),
        ("enum", &c.enum_values),
        ("fence views", &c.views),
    ];
    let all: BTreeSet<&String> = named.iter().flat_map(|(_, copy)| copy.iter()).collect();
    let mut found = Vec::new();
    for table in all {
        for (name, copy) in &named {
            if !copy.contains(table) {
                found.push(format!("{table} is missing from the {name}"));
            }
        }
    }
    found
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_four_copies_of_the_member_types_are_equal(pool: PgPool) {
    let c = copies(&pool).await;
    for (name, copy) in [
        ("manifest", &c.manifest),
        ("member_table CHECK", &c.check),
        ("enum", &c.enum_values),
        ("fence views", &c.views),
    ] {
        assert!(
            !copy.is_empty(),
            "the {name} copy is not empty, or the comparison proves nothing"
        );
    }
    assert_eq!(disagreements(&c), Vec::<String>::new());
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_name_added_to_any_one_copy_is_reported(pool: PgPool) {
    let c = copies(&pool).await;
    assert_eq!(
        disagreements(&c),
        Vec::<String>::new(),
        "setup: the copies agree"
    );
    let probe = "kb_probe_items".to_owned();
    let sides = ["manifest", "member_table CHECK", "enum", "fence views"];
    for side in sides {
        let mut probed = c.clone();
        let target = match side {
            "manifest" => &mut probed.manifest,
            "member_table CHECK" => &mut probed.check,
            "enum" => &mut probed.enum_values,
            _ => &mut probed.views,
        };
        target.insert(probe.clone());
        let expected: Vec<String> = sides
            .iter()
            .filter(|other| **other != side)
            .map(|other| format!("{probe} is missing from the {other}"))
            .collect();
        assert_eq!(
            disagreements(&probed),
            expected,
            "a name only the {side} carries is reported missing from each other copy"
        );
    }
}
