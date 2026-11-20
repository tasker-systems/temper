#![cfg(feature = "test-db")]
//! Every consumer of erasure event names covers its whole class (erasure estate-membership spec
//! D7.6).
//!
//! **The vocabulary** ([`Vocabulary`]). The erasure family is the union of temper-substrate's four
//! class constants (`PER_ITEM_AUTHORITY_KINDS`, `REDACTION_AUTHORITY_KINDS`, `STRIKE_SOURCE_KINDS`,
//! `ADMIN_ACT_KINDS`) and the six erasure record kinds ([`RECORD_KINDS`]).
//!
//! **The consumers.** Two sources, both read as they stand now:
//!
//!   * every live function, `pg_get_functiondef` of each `pg_proc` row outside the system
//!     schemas, keyed by `regprocedure`. Migrations carry superseded bodies, so they are never
//!     read;
//!   * every item of every `crates/*/src/**/*.rs` file, keyed `rust:<path>#<item>`, except
//!     temper-substrate's `events.rs` and `payloads.rs`, whose `EventKind` arms and registration
//!     lists are the vocabulary itself rather than consumers of it. The scanner
//!     ([`rust_items`]) is line-based and deliberately simple: a line whose first non-blank
//!     characters are `//` is skipped; a line whose first token (after `pub`, `pub(..)`, `async`,
//!     `unsafe` and `extern "C"`) is `fn`, `const fn`, `const` or `static` starts an item named by
//!     the next token; every line belongs to the nearest item declared on or above it, and the
//!     lines above the first item to `rust:<path>`. So a literal is attributed to its enclosing
//!     `fn` in a flat file; a literal after a nested item's closing brace is attributed to the
//!     nested item, and two items of one name in one file share a key.
//!
//! A body or item *names* a kind when it holds the kind as a quoted literal: `'kind'` (SQL, also
//! SQL text inside a Rust string) or `"kind"` (a Rust string literal). Every one that names any
//! family kind is a consumer.
//!
//! **The roster** ([`ROSTER`]) assigns each consumer one claim:
//!
//!   * a class: the consumer must name every kind of that class and no family kind outside it;
//!   * `Own(kinds)`: the consumer is its own kinds' act, or a reader of one subject's own record
//!     of them, and must name exactly those family kinds.
//!
//! Every class has at least one consumer rostered to it, so a kind added to any class fails
//! somewhere.
//!
//! **The rule** ([`problems`]): an unrostered consumer fails; a rostered one that breaks its
//! claim fails; a roster entry with no live consumer fails; an entry rostered twice fails. A
//! declared gap tolerates the kinds it names as missing only while they stay missing: once the
//! consumer names one, the gap has closed and the entry fails until its marker is removed.
//!
//! The checker is a pure function over (consumers, roster, vocabulary), so the bite probes run in
//! memory over the real bodies with one change each, one probe per failure branch.

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

/// The key prefix of a Rust item consumer.
const RUST: &str = "rust:";

/// The task that closes `sensitivity.place_closure`'s missing `principal` authority arm.
const PLACE_CLOSURE_GAP_TASK: &str = "01a1f669-0216-76f1-b86d-933addc55835";

const PLACE_CLOSURE: &str = "sensitivity.place_closure(text,uuid,text,text)";
const FENCE_FILE: &str = "crates/temper-services/src/services/erasure_fence_service.rs";
const FENCE_SEED: &str =
    "rust:crates/temper-services/src/services/erasure_fence_service.rs#seed_from_ledger";
/// The key of the `fn` the per-item probe appends to [`FENCE_FILE`].
const PROBE_SEED: &str =
    "rust:crates/temper-services/src/services/erasure_fence_service.rs#d76_probe_seed";
const FIRST_RECORD_REMAINDER: &str =
    "rust:crates/temper-services/src/services/resource_erasure_service.rs#first_record_remainder";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    PerItemAuthority,
    RedactionAuthority,
    StrikeSource,
    AdminAct,
}

const CLASSES: [Class; 4] = [
    Class::PerItemAuthority,
    Class::RedactionAuthority,
    Class::StrikeSource,
    Class::AdminAct,
];

/// Each class's kinds, and the record kinds: what the rule reads. [`Vocabulary::live`] is
/// temper-substrate's; a probe changes a copy.
#[derive(Debug, Clone)]
struct Vocabulary {
    classes: BTreeMap<Class, BTreeSet<&'static str>>,
    records: BTreeSet<&'static str>,
}

impl Vocabulary {
    fn live() -> Self {
        let set = |kinds: &[&'static str]| kinds.iter().copied().collect::<BTreeSet<_>>();
        Vocabulary {
            classes: BTreeMap::from([
                (Class::PerItemAuthority, set(&PER_ITEM_AUTHORITY_KINDS)),
                (Class::RedactionAuthority, set(&REDACTION_AUTHORITY_KINDS)),
                (Class::StrikeSource, set(&STRIKE_SOURCE_KINDS)),
                (Class::AdminAct, set(&ADMIN_ACT_KINDS)),
            ]),
            records: set(&RECORD_KINDS),
        }
    }

    fn kinds(&self, class: Class) -> &BTreeSet<&'static str> {
        &self.classes[&class]
    }

    fn family(&self) -> BTreeSet<&'static str> {
        self.classes
            .values()
            .flatten()
            .chain(self.records.iter())
            .copied()
            .collect()
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

/// A known, tracked shortfall of a class consumer. `missing` are the class kinds the consumer is
/// let off; `absent` are further literals whose appearance also means the gap is being closed.
/// Both must stay absent, or the marker is stale.
#[derive(Debug, Clone, Copy)]
struct Gap {
    task: &'static str,
    missing: &'static [&'static str],
    absent: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    /// `regprocedure` text for a live function, or `rust:<path>#<item>` for a Rust item.
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
        who: PLACE_CLOSURE,
        claim: Claim::Class(Class::RedactionAuthority),
        reason: "a ledger finding closes on a redaction row whose authorising event is of its \
                 authority's kind",
        gap: Some(Gap {
            task: PLACE_CLOSURE_GAP_TASK,
            missing: &["principal_erased"],
            absent: &["'principal'"],
        }),
    },
    class(
        "_resource_erasure_payload_redaction(uuid)",
        Class::PerItemAuthority,
        "the resource act's ledger derivation, which leaves out the events an item's own \
         correlated erasure authority appended",
    ),
    class(
        "resource_erasure_survey_plan(uuid)",
        Class::PerItemAuthority,
        "reads one item's own erasure authority record",
    ),
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
        "_erasure_legacy_backfill()",
        &["resource_erased", "erasure_declared"],
        "re-projects resource_erased first records and records each owed completion pass as a \
         legacy erasure_declared",
    ),
    // ── Rust items ─────────────────────────────────────────────────────────────────────────
    own(
        "rust:crates/temper-services/src/services/admin_ledger_service.rs#ADMIN_EVENT_TYPES",
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
        "the admin ledger's readable catalogue: a copy of the admin vocabulary, held to it by \
         the_readable_catalogue_is_exactly_the_admin_vocabulary",
    ),
    own(
        "rust:crates/temper-services/src/services/admin_ledger_service.rs#the_wire_page_withholds_erasure_references_and_keeps_the_rest",
        &["principal_erased"],
        "a unit test whose fixture row is a principal_erased",
    ),
    class(
        FENCE_SEED,
        Class::StrikeSource,
        "the blob fence seeds from the targets of every strike source",
    ),
    class(
        "rust:crates/temper-services/src/services/erasure_fence_service.rs#fence_channel_report",
        Class::StrikeSource,
        "the fence channel's report reads the ledger's records of every strike source",
    ),
    own(
        FIRST_RECORD_REMAINDER,
        &["resource_erased"],
        "reads one husk's own resource_erased first record",
    ),
    class(
        "rust:crates/temper-substrate/src/replay.rs#replay",
        Class::PerItemAuthority,
        "defers an item's erasure authority's projection body to the last event of its own \
         transaction (apply_after)",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_fence_service/tests.rs#an_unparseable_blob_verdict_is_counted_and_alertable",
        &["principal_erased"],
        "a unit-test fixture appending principal_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_service/tests.rs#a_same_hash_resource_in_another_governed_home_survives_the_erasure",
        &["principal_erased"],
        "a unit test reading the person act's own principal_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_service/tests.rs#an_authorized_act_is_attributed_to_the_surface_it_ran_on",
        &["principal_erased"],
        "a unit test reading the person act's own principal_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/resource_erasure_service/tests.rs#a_charter_resource_is_refused_recorded_with_the_fixed_detail",
        &["resource_erased"],
        "a unit test reading the resource act's own resource_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/resource_erasure_service/tests.rs#a_cut1_husk_surveys_its_completion_and_completes_once",
        &["resource_erased"],
        "a unit test appending and reading the resource act's own resource_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/resource_erasure_service/tests.rs#a_failed_or_hung_post_commit_release_still_answers_completed",
        &["resource_erased"],
        "a unit test reading the resource act's own resource_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/resource_erasure_service/tests.rs#a_repeat_erasure_is_refused_already_erased_and_changes_nothing",
        &["resource_erased"],
        "a unit test reading the resource act's own resource_erased",
    ),
];

// ── the consumers ────────────────────────────────────────────────────────────────────────────

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

/// The item a line declares: the name after `fn`, `const fn`, `const` or `static`, once any of
/// `pub`, `pub(..)`, `async`, `unsafe` and `extern "C"` are skipped. `None` for any other line.
fn declared_item(line: &str) -> Option<String> {
    let mut tokens = line.split_whitespace().peekable();
    while tokens.peek().is_some_and(|t| {
        *t == "pub" || t.starts_with("pub(") || ["async", "unsafe", "extern", "\"C\""].contains(t)
    }) {
        tokens.next();
    }
    let name = match tokens.next()? {
        "fn" => tokens.next()?,
        "const" | "static" => match tokens.next()? {
            "fn" => tokens.next()?,
            name => name,
        },
        _ => return None,
    };
    let name: String = name
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// The items of one Rust file, keyed `rust:<path>#<item>` (and `rust:<path>` for the lines above
/// the first item), each with its non-comment lines. See the module doc for the rule.
fn rust_items(path: &str, text: &str, out: &mut Bodies) {
    let mut key = format!("{RUST}{path}");
    for line in text.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        if let Some(item) = declared_item(line) {
            key = format!("{RUST}{path}#{item}");
        }
        let body = out.entry(key.clone()).or_default();
        body.push_str(line);
        body.push('\n');
    }
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_repo_file(relative: &str) -> String {
    let file = repo().join(relative);
    std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()))
}

/// Every item of every `crates/*/src/**/*.rs` file but the vocabulary's.
fn rust_bodies() -> Bodies {
    let repo = repo();
    let mut crate_dirs: Vec<PathBuf> = std::fs::read_dir(repo.join("crates"))
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
        rust_items(&relative, &read_repo_file(&relative), &mut bodies);
    }
    bodies
}

/// The consumers: every body or item that names a family kind, with the kinds it names.
fn consumers(
    bodies: &Bodies,
    vocabulary: &Vocabulary,
) -> BTreeMap<String, (String, BTreeSet<&'static str>)> {
    let family = vocabulary.family();
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

/// Every breach of the rule by `bodies` against `roster` under `vocabulary`. Empty when every
/// consumer is rostered once and keeps its claim, and every entry has a consumer.
fn problems(bodies: &Bodies, roster: &[Entry], vocabulary: &Vocabulary) -> Vec<String> {
    let found = consumers(bodies, vocabulary);
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
                "{} is rostered but no live body or item of that name names an erasure kind: \
                 remove the entry, or re-key it",
                entry.who
            ));
            continue;
        };
        match entry.claim {
            Claim::Class(class) => {
                let kinds = vocabulary.kinds(class);
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
                let outside: BTreeSet<&str> = named.difference(kinds).copied().collect();
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

/// The real bodies, after asserting the real roster holds over them, so each probe's one change is
/// the only thing that can fail.
async fn holding_bodies(pool: &PgPool) -> Bodies {
    let bodies = all_bodies(pool).await;
    assert_eq!(
        problems(&bodies, ROSTER, &Vocabulary::live()),
        Vec::<String>::new(),
        "setup: the roster holds"
    );
    bodies
}

// ── the witness ──────────────────────────────────────────────────────────────────────────────

#[test]
fn the_classes_and_record_kinds_are_registered_typed_kinds() {
    let vocabulary = Vocabulary::live();
    for kind in vocabulary.family() {
        assert!(
            TYPED_EVENT_NAMES.contains(&kind),
            "{kind} is in the erasure family but is not a registered typed kind"
        );
    }
    for name in TYPED_EVENT_NAMES {
        if name.starts_with("erasure_") {
            assert!(
                RECORD_KINDS.contains(&name),
                "{name} is a registered erasure_ kind but not in RECORD_KINDS, so its consumers \
                 are never scanned"
            );
        }
    }
    for class in CLASSES {
        assert!(
            !vocabulary.kinds(class).is_empty(),
            "{class:?} is not empty"
        );
        assert!(
            ROSTER
                .iter()
                .any(|entry| matches!(entry.claim, Claim::Class(c) if c == class)),
            "{class:?} has a consumer rostered to it, so a kind added to it fails somewhere"
        );
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
    let found = consumers(&bodies, &Vocabulary::live());
    assert!(
        found.keys().any(|who| !who.starts_with(RUST))
            && found.keys().any(|who| who.starts_with(RUST)),
        "setup: both sources yield consumers, or the scan proves nothing (found {:?})",
        found.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        problems(&bodies, ROSTER, &Vocabulary::live()),
        Vec::<String>::new()
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn place_closures_declared_gap_is_still_real(pool: PgPool) {
    let bodies = live_bodies(&pool).await;
    let entry = ROSTER
        .iter()
        .find(|entry| entry.who == PLACE_CLOSURE)
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

/// An unrostered consumer fails, and rostered to a class it covers only in part it fails for the
/// kind it leaves out.
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
    let vocabulary = Vocabulary::live();

    let unrostered = problems(&bodies, ROSTER, &vocabulary);
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
        problems(&bodies, &roster, &vocabulary),
        vec![format!(
            "{who} is rostered to RedactionAuthority but does not name principal_erased"
        )],
        "rostered to the redaction class, the probe fails for the kind it leaves out"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_real_consumer_dropped_from_the_roster_fails(pool: PgPool) {
    let bodies = holding_bodies(&pool).await;
    for dropped in ["_event_is_admin_act(text,uuid,uuid,xid8)", FENCE_SEED] {
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
        let found = problems(&bodies, &roster, &Vocabulary::live());
        assert_eq!(found.len(), 1, "only {dropped} is reported: {found:?}");
        assert!(
            found[0].starts_with(&format!("{dropped} names ")) && found[0].contains("not rostered"),
            "{dropped}, unrostered, is reported: {found:?}"
        );
    }
}

/// A kind added to `PER_ITEM_AUTHORITY_KINDS` fails every consumer rostered to the class, and
/// nothing else.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_kind_added_to_the_per_item_authority_class_fails_its_consumers(pool: PgPool) {
    let bodies = holding_bodies(&pool).await;
    let added = "d76_probe_per_item_kind";
    let mut vocabulary = Vocabulary::live();
    vocabulary
        .classes
        .get_mut(&Class::PerItemAuthority)
        .expect("the class is in the vocabulary")
        .insert(added);

    let expected: Vec<String> = ROSTER
        .iter()
        .filter(|entry| matches!(entry.claim, Claim::Class(Class::PerItemAuthority)))
        .map(|entry| {
            format!(
                "{} is rostered to PerItemAuthority but does not name {added}",
                entry.who
            )
        })
        .collect();
    assert!(
        !expected.is_empty(),
        "setup: the class has a consumer to fail"
    );
    assert_eq!(problems(&bodies, ROSTER, &vocabulary), expected);
}

/// A new `fn` in a file whose other items are rostered is its own consumer: naming part of its
/// class, it fails, unrostered or rostered to the class.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_new_fn_in_a_rostered_file_naming_part_of_its_class_fails(pool: PgPool) {
    let mut bodies = holding_bodies(&pool).await;
    let text = format!(
        "{}\nasync fn d76_probe_seed() -> &'static str {{\n    \
         \"SELECT 1 FROM kb_event_types t WHERE t.name = 'resource_erased'\"\n}}\n",
        read_repo_file(FENCE_FILE)
    );
    bodies.retain(|who, _| !who.starts_with(&format!("{RUST}{FENCE_FILE}")));
    rust_items(FENCE_FILE, &text, &mut bodies);
    let who = PROBE_SEED;
    assert_eq!(who, format!("{RUST}{FENCE_FILE}#d76_probe_seed"));
    let vocabulary = Vocabulary::live();

    assert_eq!(
        problems(&bodies, ROSTER, &vocabulary),
        vec![format!(
            "{who} names resource_erased but is not rostered: assign it a class, or its own \
             kinds with a reason"
        )],
        "the new fn is a consumer of its own, beside the file's rostered items"
    );

    let mut roster = ROSTER.to_vec();
    roster.push(class(who, Class::StrikeSource, "the bite probe"));
    assert_eq!(
        problems(&bodies, &roster, &vocabulary),
        vec![format!(
            "{who} is rostered to StrikeSource but does not name principal_erased"
        )]
    );
}

/// `place_closure` naming the kind its declared gap excuses fails as a closing gap, and only so;
/// so does its holding the `'principal'` authority alone.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_declared_gap_that_closes_fails(pool: PgPool) {
    let real = holding_bodies(&pool).await;
    for (literal, holds) in [
        ("'principal_erased'", "principal_erased"),
        ("'principal'", "'principal'"),
    ] {
        let mut bodies = real.clone();
        bodies
            .get_mut(PLACE_CLOSURE)
            .expect("place_closure is a live function")
            .push_str(&format!("\n-- probe: {literal}\n"));
        assert_eq!(
            problems(&bodies, ROSTER, &Vocabulary::live()),
            vec![format!(
                "{PLACE_CLOSURE}'s declared gap (task {PLACE_CLOSURE_GAP_TASK}) is closing: it \
                 now holds {holds}; remove the gap marker from its roster entry"
            )],
            "with {literal}, the gap is closing and nothing else fails"
        );
    }
}

/// A class consumer that also names a family kind outside its class fails.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_class_consumer_naming_a_kind_outside_its_class_fails(pool: PgPool) {
    let mut bodies = holding_bodies(&pool).await;
    bodies
        .get_mut(FENCE_SEED)
        .expect("the fence seed is a Rust item")
        .push_str("    let probe = \"resource_scrubbed\";\n");
    assert_eq!(
        problems(&bodies, ROSTER, &Vocabulary::live()),
        vec![format!(
            "{FENCE_SEED} is rostered to StrikeSource but also names resource_scrubbed: it is \
             not that class alone"
        )]
    );
}

/// A consumer rostered as its own kinds that names another family kind fails.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_own_consumer_naming_another_kind_fails(pool: PgPool) {
    let mut bodies = holding_bodies(&pool).await;
    bodies
        .get_mut(FIRST_RECORD_REMAINDER)
        .expect("first_record_remainder is a Rust item")
        .push_str("    let probe = \"principal_erased\";\n");
    assert_eq!(
        problems(&bodies, ROSTER, &Vocabulary::live()),
        vec![format!(
            "{FIRST_RECORD_REMAINDER} is rostered as its own resource_erased but names \
             principal_erased, resource_erased"
        )]
    );
}

/// A roster entry with no consumer of that name fails.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_stale_roster_entry_fails(pool: PgPool) {
    let bodies = holding_bodies(&pool).await;
    let who = "d76_probe_no_such_function()";
    let mut roster = ROSTER.to_vec();
    roster.push(own(who, &["resource_erased"], "the bite probe"));
    assert_eq!(
        problems(&bodies, &roster, &Vocabulary::live()),
        vec![format!(
            "{who} is rostered but no live body or item of that name names an erasure kind: \
             remove the entry, or re-key it"
        )]
    );
}

/// A consumer rostered twice fails.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_consumer_rostered_twice_fails(pool: PgPool) {
    let bodies = holding_bodies(&pool).await;
    let mut roster = ROSTER.to_vec();
    let twice = *roster
        .iter()
        .find(|entry| entry.who == FENCE_SEED)
        .expect("the fence seed is rostered");
    roster.push(twice);
    assert_eq!(
        problems(&bodies, &roster, &Vocabulary::live()),
        vec![format!("{FENCE_SEED} is rostered twice")]
    );
}
