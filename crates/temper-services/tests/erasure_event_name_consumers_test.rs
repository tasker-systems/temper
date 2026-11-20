#![cfg(feature = "test-db")]
//! Every consumer of erasure event names covers its whole class (erasure estate-membership spec
//! D7.6; plan rulings P12 and P13).
//!
//! **The vocabulary.** The erasure family is the union of temper-substrate's four class constants
//! (`PER_ITEM_AUTHORITY_KINDS`, `REDACTION_AUTHORITY_KINDS`, `STRIKE_SOURCE_KINDS`,
//! `ADMIN_ACT_KINDS`) and the six erasure record kinds ([`RECORD_KINDS`]).
//!
//! **The consumers.** Two sources, both read as they stand now:
//!
//!   * every live function, `pg_get_functiondef` of each `pg_proc` row outside the system
//!     schemas, keyed by `regprocedure`. Migrations carry superseded bodies, so they are never
//!     read;
//!   * every `crates/*/src/**/*.rs` file, keyed by its path, except temper-substrate's
//!     `events.rs` and `payloads.rs`, whose `EventKind` arms and registration lists are the
//!     vocabulary itself rather than consumers of it. A file is one consumer: the names it
//!     mentions are the union over its non-comment lines (a line whose first non-blank characters
//!     are `//` is skipped).
//!
//! A body or file *names* a kind when it holds the kind as a quoted literal: `'kind'` (SQL, also
//! SQL text inside a Rust string) or `"kind"` (a Rust string literal). Every one that names any
//! family kind is a consumer.
//!
//! **The roster** ([`ROSTER`]) assigns each consumer one claim:
//!
//!   * a class: the consumer must name every kind of that class and no family kind outside it;
//!   * `Own(kinds)`: the consumer is its own kinds' act, or a reader of one subject's own record
//!     of them, and must name exactly those family kinds.
//!
//! **The rule** ([`problems`]): an unrostered consumer fails; a rostered one that breaks its
//! claim fails; a roster entry with no live consumer fails. A declared gap (P13) tolerates the
//! kinds it names as missing only while they stay missing: once the consumer names one, the gap
//! has closed and the entry fails until its marker is removed.
//!
//! The checker is a pure function over (consumers, roster), so the bite probes run in memory:
//! a fixture function naming two of the redaction class's three kinds fails, and a real consumer
//! dropped from a copy of the roster fails.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sqlx::PgPool;
use temper_substrate::payloads::{
    ADMIN_ACT_KINDS, PER_ITEM_AUTHORITY_KINDS, REDACTION_AUTHORITY_KINDS, STRIKE_SOURCE_KINDS,
    TYPED_EVENT_NAMES,
};

/// The six erasure record kinds (20261201000020_erasure_record_event_types.sql).
const RECORD_KINDS: [&str; 6] = [
    "erasure_declared",
    "erasure_member_arrived",
    "erasure_scope_sealed",
    "erasure_reopened",
    "erasure_batch_applied",
    "erasure_completed",
];

/// The files under `crates/*/src` that are the vocabulary, not consumers of it.
const VOCABULARY_FILES: [&str; 2] = [
    "crates/temper-substrate/src/events.rs",
    "crates/temper-substrate/src/payloads.rs",
];

/// The key prefix of a Rust file consumer.
const RUST: &str = "rust:";

/// The task that closes `sensitivity.place_closure`'s missing `principal` authority arm.
const PLACE_CLOSURE_GAP_TASK: &str = "01a1f669-0216-76f1-b86d-933addc55835";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    PerItemAuthority,
    RedactionAuthority,
    StrikeSource,
    AdminAct,
}

impl Class {
    fn kinds(self) -> &'static [&'static str] {
        match self {
            Class::PerItemAuthority => &PER_ITEM_AUTHORITY_KINDS,
            Class::RedactionAuthority => &REDACTION_AUTHORITY_KINDS,
            Class::StrikeSource => &STRIKE_SOURCE_KINDS,
            Class::AdminAct => &ADMIN_ACT_KINDS,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Claim {
    /// Names every kind of the class and no family kind outside it.
    Class(Class),
    /// Names exactly these family kinds: its own kinds' act, or a reader of one subject's own
    /// record of them.
    Own(&'static [&'static str]),
}

/// A known, tracked shortfall of a class consumer (P13). `missing` are the class kinds the
/// consumer is let off; `absent` are further literals whose appearance also means the gap is
/// being closed. Both must stay absent, or the marker is stale.
#[derive(Debug, Clone, Copy)]
struct Gap {
    task: &'static str,
    missing: &'static [&'static str],
    absent: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    /// `regprocedure` text for a live function, or `rust:<path>` for a Rust file.
    who: &'static str,
    claim: Claim,
    reason: &'static str,
    gap: Option<Gap>,
}

const fn class(who: &'static str, class: Class, reason: &'static str) -> Entry {
    Entry {
        who,
        claim: Claim::Class(class),
        reason,
        gap: None,
    }
}

const fn own(who: &'static str, kinds: &'static [&'static str], reason: &'static str) -> Entry {
    Entry {
        who,
        claim: Claim::Own(kinds),
        reason,
        gap: None,
    }
}

/// Every consumer of an erasure-family kind, and what it must name.
const ROSTER: &[Entry] = &[
    // ── live functions ─────────────────────────────────────────────────────────────────────
    class(
        "_event_is_admin_act(text,uuid,uuid,xid8)",
        Class::AdminAct,
        "an event correlated to an admin erasure act is the act's own work",
    ),
    class(
        "_profile_change_events(uuid)",
        Class::AdminAct,
        "reader changes exclude the events an admin erasure act appended",
    ),
    class(
        "kb_events_append_only()",
        Class::RedactionAuthority,
        "the ledger exception admits a rewrite only under a redaction row of its authority's kind",
    ),
    Entry {
        who: "sensitivity.place_closure(text,uuid,text,text)",
        claim: Claim::Class(Class::RedactionAuthority),
        reason: "a ledger finding closes on a redaction row whose authorising event is of its \
                 authority's kind",
        gap: Some(Gap {
            task: PLACE_CLOSURE_GAP_TASK,
            missing: &["principal_erased"],
            absent: &["'principal'"],
        }),
    },
    own(
        "_block_history_scrub_apply(uuid)",
        &["block_history_scrubbed"],
        "the block history scrub's projection, refusing an event of any other kind",
    ),
    own(
        "block_history_scrub_execute(uuid,uuid[],uuid,uuid,uuid)",
        &["block_history_scrubbed"],
        "the block history scrub act, appending its own kind",
    ),
    own(
        "principal_erasure_execute(uuid,uuid,uuid,uuid)",
        &["principal_erased"],
        "the person erasure act, appending its own kind",
    ),
    own(
        "resource_erasure_execute(uuid,uuid,uuid,uuid,uuid[])",
        &["resource_erased"],
        "the resource erasure act, appending its own kind",
    ),
    own(
        "resource_field_scrub_execute(uuid,text,uuid,boolean,uuid,uuid,uuid)",
        &["resource_scrubbed"],
        "the field scrub act, appending its own kind",
    ),
    own(
        "_resource_erasure_payload_redaction(uuid)",
        &["resource_erased"],
        "the resource act's ledger derivation, which leaves out events its own correlated \
         resource_erased appended",
    ),
    own(
        "resource_erasure_survey_plan(uuid)",
        &["resource_erased"],
        "reads one husk's own resource_erased first record",
    ),
    own(
        "_erasure_legacy_backfill()",
        &["resource_erased", "erasure_declared"],
        "re-projects resource_erased first records and records each owed completion pass as a \
         legacy erasure_declared",
    ),
    // ── Rust files ─────────────────────────────────────────────────────────────────────────
    own(
        "rust:crates/temper-services/src/services/admin_ledger_service.rs",
        &[
            "resource_erased",
            "principal_erased",
            "resource_scrubbed",
            "block_history_scrubbed",
            "erasure_declared",
            "erasure_member_arrived",
            "erasure_scope_sealed",
            "erasure_reopened",
            "erasure_batch_applied",
            "erasure_completed",
        ],
        "ADMIN_EVENT_TYPES, the admin ledger's readable catalogue: a copy of the admin \
         vocabulary, held to it by the_readable_catalogue_is_exactly_the_admin_vocabulary",
    ),
    class(
        "rust:crates/temper-services/src/services/erasure_fence_service.rs",
        Class::StrikeSource,
        "the blob fence seeds from the targets of every strike source",
    ),
    own(
        "rust:crates/temper-services/src/services/resource_erasure_service.rs",
        &["resource_erased"],
        "reads one husk's own resource_erased first record",
    ),
    own(
        "rust:crates/temper-substrate/src/replay.rs",
        &["resource_erased"],
        "defers the resource act's projection body to the last event of its own transaction",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_fence_service/tests.rs",
        &["principal_erased"],
        "unit-test fixtures appending principal_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_service/tests.rs",
        &["principal_erased"],
        "unit tests reading the person act's own principal_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/resource_erasure_service/tests.rs",
        &["resource_erased"],
        "unit tests appending and reading the resource act's own resource_erased",
    ),
];

// ── the vocabulary and the consumers ─────────────────────────────────────────────────────────

fn family() -> BTreeSet<&'static str> {
    PER_ITEM_AUTHORITY_KINDS
        .iter()
        .chain(REDACTION_AUTHORITY_KINDS.iter())
        .chain(STRIKE_SOURCE_KINDS.iter())
        .chain(ADMIN_ACT_KINDS.iter())
        .chain(RECORD_KINDS.iter())
        .copied()
        .collect()
}

/// The family kinds `text` holds as a quoted literal.
fn named_in(text: &str, family: &BTreeSet<&'static str>) -> BTreeSet<&'static str> {
    family
        .iter()
        .copied()
        .filter(|kind| text.contains(&format!("'{kind}'")) || text.contains(&format!("\"{kind}\"")))
        .collect()
}

/// Consumer key → its text.
type Bodies = BTreeMap<String, String>;

async fn live_bodies(pool: &PgPool) -> Bodies {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.oid::regprocedure::text, pg_get_functiondef(p.oid) \
           FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
          WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') \
            AND n.nspname NOT LIKE 'pg\\_%' \
            AND p.prokind IN ('f', 'p')",
    )
    .fetch_all(pool)
    .await
    .expect("read the live function bodies");
    rows.into_iter().collect()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Every `crates/*/src/**/*.rs` file but the vocabulary's, keyed `rust:<repo-relative path>`,
/// with its comment lines removed.
fn rust_bodies() -> Bodies {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let crates = repo.join("crates");
    let mut crate_dirs: Vec<PathBuf> = std::fs::read_dir(&crates)
        .expect("read crates/")
        .map(|entry| entry.expect("a directory entry").path())
        .collect();
    crate_dirs.sort();
    let mut files = Vec::new();
    for dir in crate_dirs {
        let src = dir.join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    let mut bodies = Bodies::new();
    for file in files {
        let relative = file
            .strip_prefix(&repo)
            .expect("under the repository")
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if VOCABULARY_FILES.contains(&relative.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        let code: String = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        bodies.insert(format!("{RUST}{relative}"), code);
    }
    bodies
}

/// The consumers: every body or file that names a family kind, with the kinds it names.
fn consumers(bodies: &Bodies) -> BTreeMap<String, (String, BTreeSet<&'static str>)> {
    let family = family();
    bodies
        .iter()
        .filter_map(|(who, text)| {
            let named = named_in(text, &family);
            (!named.is_empty()).then(|| (who.clone(), (text.clone(), named)))
        })
        .collect()
}

// ── the rule ─────────────────────────────────────────────────────────────────────────────────

fn list(kinds: &BTreeSet<&str>) -> String {
    kinds.iter().copied().collect::<Vec<_>>().join(", ")
}

/// Every breach of the rule by `bodies` against `roster`. Empty when every consumer is rostered
/// and keeps its claim, and every entry has a consumer.
fn problems(bodies: &Bodies, roster: &[Entry]) -> Vec<String> {
    let found = consumers(bodies);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in roster {
        if !seen.insert(entry.who) {
            out.push(format!("{} is rostered twice", entry.who));
        }
    }
    for who in found.keys() {
        if !roster.iter().any(|entry| entry.who == who) {
            out.push(format!(
                "{who} names {} but is not rostered: assign it a class, or its own kinds with a \
                 reason",
                list(&found[who].1)
            ));
        }
    }
    for entry in roster {
        let Some((text, named)) = found.get(entry.who) else {
            out.push(format!(
                "{} is rostered but no live body or file of that name names an erasure kind: \
                 remove the entry, or re-key it",
                entry.who
            ));
            continue;
        };
        match entry.claim {
            Claim::Class(class) => {
                let kinds: BTreeSet<&str> = class.kinds().iter().copied().collect();
                let excused: BTreeSet<&str> = entry
                    .gap
                    .map(|g| g.missing.iter().copied().collect())
                    .unwrap_or_default();
                let missing: BTreeSet<&str> = kinds
                    .difference(named)
                    .copied()
                    .filter(|kind| !excused.contains(kind))
                    .collect();
                if !missing.is_empty() {
                    out.push(format!(
                        "{} is rostered to {class:?} but does not name {}",
                        entry.who,
                        list(&missing)
                    ));
                }
                let outside: BTreeSet<&str> = named.difference(&kinds).copied().collect();
                if !outside.is_empty() {
                    out.push(format!(
                        "{} is rostered to {class:?} but also names {}: it is not that class alone",
                        entry.who,
                        list(&outside)
                    ));
                }
            }
            Claim::Own(kinds) => {
                let kinds: BTreeSet<&str> = kinds.iter().copied().collect();
                if *named != kinds {
                    out.push(format!(
                        "{} is rostered as its own {} but names {}",
                        entry.who,
                        list(&kinds),
                        list(named)
                    ));
                }
            }
        }
        if let Some(gap) = entry.gap {
            let closed: Vec<&str> = gap
                .missing
                .iter()
                .copied()
                .filter(|kind| named.contains(kind))
                .chain(
                    gap.absent
                        .iter()
                        .copied()
                        .filter(|literal| text.contains(literal)),
                )
                .collect();
            if !closed.is_empty() {
                out.push(format!(
                    "{}'s declared gap (task {}) is closing: it now holds {}; remove the gap \
                     marker from its roster entry",
                    entry.who,
                    gap.task,
                    closed.join(", ")
                ));
            }
        }
    }
    out
}

async fn all_bodies(pool: &PgPool) -> Bodies {
    let mut bodies = live_bodies(pool).await;
    bodies.extend(rust_bodies());
    bodies
}

// ── the witness ──────────────────────────────────────────────────────────────────────────────

#[test]
fn the_classes_and_record_kinds_are_registered_typed_kinds() {
    for kind in family() {
        assert!(
            TYPED_EVENT_NAMES.contains(&kind),
            "{kind} is in the erasure family but is not a registered typed kind"
        );
    }
    for class in [
        Class::PerItemAuthority,
        Class::RedactionAuthority,
        Class::StrikeSource,
        Class::AdminAct,
    ] {
        assert!(!class.kinds().is_empty(), "{class:?} is not empty");
    }
    for entry in ROSTER {
        assert!(
            !entry.reason.trim().is_empty(),
            "{} has a reason",
            entry.who
        );
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_consumer_of_erasure_event_names_covers_its_class(pool: PgPool) {
    let bodies = all_bodies(&pool).await;
    let found = consumers(&bodies);
    assert!(
        found.keys().any(|who| !who.starts_with(RUST))
            && found.keys().any(|who| who.starts_with(RUST)),
        "setup: both sources yield consumers, or the scan proves nothing (found {:?})",
        found.keys().collect::<Vec<_>>()
    );
    assert_eq!(problems(&bodies, ROSTER), Vec::<String>::new());
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn place_closures_declared_gap_is_still_real(pool: PgPool) {
    let bodies = live_bodies(&pool).await;
    let entry = ROSTER
        .iter()
        .find(|entry| entry.who.starts_with("sensitivity.place_closure("))
        .expect("place_closure is rostered");
    let gap = entry.gap.expect("place_closure carries its declared gap");
    assert_eq!(gap.task, PLACE_CLOSURE_GAP_TASK);
    let body = bodies
        .get(entry.who)
        .expect("place_closure is a live function");
    for kind in gap.missing {
        assert!(
            !body.contains(&format!("'{kind}'")),
            "place_closure now names {kind}: the gap of task {} has closed; remove its marker",
            gap.task
        );
    }
    for literal in gap.absent {
        assert!(
            !body.contains(literal),
            "place_closure now holds {literal}: the gap of task {} has closed; remove its marker",
            gap.task
        );
    }
}

// ── the bite probes ──────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_consumer_naming_two_of_the_redaction_class_fails(pool: PgPool) {
    sqlx::query(
        "CREATE FUNCTION d76_probe_redaction_reader(p_kind text) RETURNS boolean \
         LANGUAGE sql IMMUTABLE AS $$ SELECT p_kind IN ('resource_erased', 'resource_scrubbed') $$",
    )
    .execute(&pool)
    .await
    .expect("create the probe function");
    let who = "d76_probe_redaction_reader(text)";
    let bodies = all_bodies(&pool).await;

    let unrostered = problems(&bodies, ROSTER);
    assert_eq!(
        unrostered.len(),
        1,
        "only the probe is reported: {unrostered:?}"
    );
    assert!(
        unrostered[0].starts_with(&format!("{who} names "))
            && unrostered[0].contains("not rostered"),
        "unrostered, the probe is reported as such: {unrostered:?}"
    );

    let mut roster = ROSTER.to_vec();
    roster.push(class(who, Class::RedactionAuthority, "the bite probe"));
    assert_eq!(
        problems(&bodies, &roster),
        vec![format!(
            "{who} is rostered to RedactionAuthority but does not name principal_erased"
        )],
        "rostered to the redaction class, the probe fails for the kind it leaves out"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_real_consumer_dropped_from_the_roster_fails(pool: PgPool) {
    let bodies = all_bodies(&pool).await;
    assert_eq!(
        problems(&bodies, ROSTER),
        Vec::<String>::new(),
        "setup: the roster holds"
    );
    for dropped in [
        "_event_is_admin_act(text,uuid,uuid,xid8)",
        "rust:crates/temper-services/src/services/erasure_fence_service.rs",
    ] {
        let roster: Vec<Entry> = ROSTER
            .iter()
            .copied()
            .filter(|entry| entry.who != dropped)
            .collect();
        assert_eq!(
            roster.len(),
            ROSTER.len() - 1,
            "setup: {dropped} was rostered"
        );
        let found = problems(&bodies, &roster);
        assert_eq!(found.len(), 1, "only {dropped} is reported: {found:?}");
        assert!(
            found[0].starts_with(&format!("{dropped} names ")) && found[0].contains("not rostered"),
            "{dropped}, unrostered, is reported: {found:?}"
        );
    }
}
