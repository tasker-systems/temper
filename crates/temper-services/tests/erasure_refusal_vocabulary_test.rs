#![cfg(feature = "test-db")]
//! The drain's refusal vocabulary has four copies: `policy/erasure/estate.toml`'s
//! `[refusal_reasons]`, the `kb_erasure_refusal_reasons` table, the `erasure_refusal_reasons` seeds
//! in `system.yaml`, and temper-core's `ErasureMemberRefusalReason`. This holds them set-equal, and the
//! manifest's, the table's and the seeds' meanings equal. Each probe adds a code to one copy only,
//! in memory, and asserts the comparison reports it, so the comparison is shown to see every copy.

use std::collections::BTreeMap;

use sqlx::PgPool;
use temper_core::types::erasure::ErasureMemberRefusalReason;

/// Every variant of the enum, listed by hand. Nothing forces a new variant into this list: the
/// exhaustive `match` in `enum_code` forces an arm only. The set comparison catches the omission
/// whenever another copy carries the new code; a variant added to the enum and its arm alone, and
/// to no other copy, is not caught here.
const VARIANTS: [ErasureMemberRefusalReason; 4] = [
    ErasureMemberRefusalReason::CharterResource,
    ErasureMemberRefusalReason::Defect,
    ErasureMemberRefusalReason::BatchFatal,
    ErasureMemberRefusalReason::Unclassified,
];

/// Exhaustive over the enum, so a new variant needs an arm here before this test compiles.
fn enum_code(reason: ErasureMemberRefusalReason) -> &'static str {
    match reason {
        ErasureMemberRefusalReason::CharterResource => "charter_resource",
        ErasureMemberRefusalReason::Defect => "defect",
        ErasureMemberRefusalReason::BatchFatal => "batch_fatal",
        ErasureMemberRefusalReason::Unclassified => "unclassified",
    }
}

/// One copy of the vocabulary: code → meaning (`None` for a copy that carries codes only).
type Vocab = BTreeMap<String, Option<String>>;

struct Copies {
    manifest: Vocab,
    table: Vocab,
    enum_values: Vocab,
    seeds: Vocab,
}

fn manifest_copy() -> Vocab {
    temper_services::erasure_policy::in_force()
        .estate
        .refusal_reasons
        .iter()
        .map(|(code, reason)| (code.clone(), Some(reason.meaning.clone())))
        .collect()
}

fn enum_copy() -> Vocab {
    VARIANTS
        .iter()
        .map(|variant| {
            let code = serde_json::to_value(variant)
                .expect("a variant serializes")
                .as_str()
                .expect("a variant serializes as a string")
                .to_owned();
            assert_eq!(code, enum_code(*variant), "the wire form is the code");
            (code, None)
        })
        .collect()
}

fn seeds_copy() -> Vocab {
    #[derive(serde::Deserialize)]
    struct Seed {
        code: String,
        meaning: String,
    }
    #[derive(serde::Deserialize)]
    struct Boot {
        erasure_refusal_reasons: Vec<Seed>,
    }
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../temper-substrate/tests/fixtures/seeds/system.yaml"
    );
    let boot: Boot =
        serde_yaml::from_str(&std::fs::read_to_string(path).expect("read system.yaml"))
            .expect("system.yaml parses");
    boot.erasure_refusal_reasons
        .into_iter()
        .map(|seed| (seed.code, Some(seed.meaning)))
        .collect()
}

async fn table_copy(pool: &PgPool) -> Vocab {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT code, meaning FROM kb_erasure_refusal_reasons")
            .fetch_all(pool)
            .await
            .expect("read kb_erasure_refusal_reasons");
    rows.into_iter()
        .map(|(code, meaning)| (code, Some(meaning)))
        .collect()
}

async fn copies(pool: &PgPool) -> Copies {
    Copies {
        manifest: manifest_copy(),
        table: table_copy(pool).await,
        enum_values: enum_copy(),
        seeds: seeds_copy(),
    }
}

/// Every disagreement between the four copies: a code one copy lacks that another has, and a
/// meaning two copies state differently. Empty when the copies agree.
fn disagreements(c: &Copies) -> Vec<String> {
    let named = [
        ("manifest", &c.manifest),
        ("table", &c.table),
        ("enum", &c.enum_values),
        ("seeds", &c.seeds),
    ];
    let mut found = Vec::new();
    let all_codes: std::collections::BTreeSet<&String> =
        named.iter().flat_map(|(_, copy)| copy.keys()).collect();
    for code in all_codes {
        for (name, copy) in &named {
            if !copy.contains_key(code) {
                found.push(format!("code {code:?} is missing from the {name}"));
            }
        }
        let meanings: std::collections::BTreeSet<&String> = named
            .iter()
            .filter_map(|(_, copy)| copy.get(code).and_then(Option::as_ref))
            .collect();
        if meanings.len() > 1 {
            found.push(format!(
                "code {code:?} has differing meanings: {meanings:?}"
            ));
        }
    }
    found
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_four_copies_of_the_refusal_vocabulary_are_equal(pool: PgPool) {
    let c = copies(&pool).await;
    assert!(
        !c.manifest.is_empty(),
        "the vocabulary is not empty, or the comparison proves nothing"
    );
    assert_eq!(disagreements(&c), Vec::<String>::new());
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_code_added_to_one_copy_only_is_reported(pool: PgPool) {
    const PROBE: &str = "probe_only_here";
    let probes: [(&str, fn(&mut Copies)); 4] = [
        ("manifest", |c| {
            c.manifest.insert(PROBE.into(), Some("m".into()));
        }),
        ("table", |c| {
            c.table.insert(PROBE.into(), Some("m".into()));
        }),
        ("enum", |c| {
            c.enum_values.insert(PROBE.into(), None);
        }),
        ("seeds", |c| {
            c.seeds.insert(PROBE.into(), Some("m".into()));
        }),
    ];
    for (side, add) in probes {
        let mut c = copies(&pool).await;
        add(&mut c);
        let found = disagreements(&c);
        let missing = found
            .iter()
            .filter(|line| line.contains(PROBE) && line.contains("is missing from"))
            .count();
        assert_eq!(
            missing, 3,
            "a code only in the {side} must be missing from the other three: {found:?}"
        );
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_meaning_that_differs_in_one_copy_is_reported(pool: PgPool) {
    let mut c = copies(&pool).await;
    let code = c.table.keys().next().expect("a seeded code").clone();
    c.table
        .insert(code.clone(), Some("a different meaning".into()));
    let found = disagreements(&c);
    assert!(
        found
            .iter()
            .any(|line| line.contains(&code) && line.contains("differing meanings")),
        "{found:?}"
    );
}
