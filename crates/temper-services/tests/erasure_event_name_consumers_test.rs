#![cfg(feature = "test-db")]
//! Every consumer of erasure event names covers its whole class, and every erasure kind is in a
//! class (erasure estate-membership spec D7.6).
//!
//! **The vocabulary** ([`Vocabulary`]). The erasure family is the union of temper-substrate's five
//! class constants (`PER_ITEM_AUTHORITY_KINDS`, `REDACTION_AUTHORITY_KINDS`, `STRIKE_SOURCE_KINDS`,
//! `ADMIN_ACT_KINDS`, `ANCHORED_ACT_KINDS`) and the six erasure record kinds ([`RECORD_KINDS`]).
//!
//! **The kinds are closed too** ([`kind_problems`]). Every registered kind (a `kb_event_types`
//! row, or a `TYPED_EVENT_NAMES` entry) whose name has an erasure's shape (`*_erased`,
//! `*_scrubbed`, or holding `erasure`), and every kind in [`AUTHORITY_BY_ROLE`], is either in the
//! family or in [`EXCLUDED_KINDS`] with a reason, never both. So a new `*_erased` kind registered
//! without a class fails here before any consumer of it can be missed. An exclusion must name a
//! registered kind, and every family kind must be registered.
//!
//! **The consumers.** Every text the database or the crates hold that could name a kind, read as
//! it stands now:
//!
//!   * every live function, `pg_get_functiondef` of each `pg_proc` row outside the system
//!     schemas, keyed by `regprocedure`. Migrations carry superseded bodies, so they are never
//!     read;
//!   * every view (`view:<name>`, `pg_get_viewdef`), row-level security policy
//!     (`policy:<table>.<name>`, its USING and WITH CHECK), CHECK constraint
//!     (`constraint:<table>.<name>`), user trigger, its WHEN clause included
//!     (`trigger:<table>.<name>`), and column default (`default:<table>.<column>`);
//!   * every item of every `crates/*/src/**/*.rs` file, keyed `rust:<path>#<item>`. The items of
//!     temper-substrate's `events.rs` and `payloads.rs` that ARE the vocabulary (the `EventKind`
//!     name maps, the registration lists, the class constants, `verify_ledger_roundtrip`) are
//!     listed in [`VOCABULARY_ITEMS`] and set aside; every other item of those files is scanned like
//!     any other, and a listed item that stops naming a kind fails. The scanner ([`rust_items`]) is
//!     line-based and deliberately simple: a line whose first non-blank characters are `//` is
//!     skipped; a line whose first token (after `pub`, `pub(..)`, `async`, `unsafe` and
//!     `extern "C"`) is `fn`, `const fn`, `const` or `static` starts an item named by the next
//!     token; every line belongs to the nearest item declared on or above it, and the lines above
//!     the first item to `rust:<path>`. So a literal is attributed to its enclosing `fn` in a flat
//!     file; a literal after a nested item's closing brace is attributed to the nested item, and
//!     two items of one name in one file share a key.
//!
//! A text *names* a kind when it holds the kind as a quoted literal: `'kind'` (SQL, also SQL text
//! inside a Rust string), `"kind"` (a Rust string literal) or `\"kind\"` (a JSON string inside a
//! Rust string). Every one that names any family kind is a consumer. A text that matches kind
//! names by pattern (`name LIKE '%_erased'`, `kind ~ '^erasure_'`, `.ends_with("_erased")`) fails
//! outright ([`pattern_references`]): it reaches kinds this test cannot enumerate, so it must name
//! them.
//!
//! **The roster** ([`ROSTER`]) assigns each consumer one claim:
//!
//!   * a class: the consumer must name every kind of that class and no family kind outside it;
//!   * `Union`: the consumer must name every kind of each of its classes, plus exactly its own
//!     extra kinds, and no other family kind;
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
//! **Declared limits.** What this test cannot see, each by choice:
//!
//!   * a reference by `EventKind` variant (`EventKind::ResourceErased`, `matches!(k, …)`) or by a
//!     name computed at run time (`as_canonical_name()`, `format!`, SQL `||`). Replay's exhaustive
//!     matches name every kind by variant, and the compiler, not this test, holds them total;
//!   * an `Own` claim does not grow with a class: an act that appends its own kinds is not a
//!     reader of the class, so a kind added to the class does not fail it. Whether a new consumer
//!     is a class reader or its own kinds' act is decided by whoever rosters it, in review;
//!   * SQL outside the five catalog sources above: rules, event triggers, and text a function
//!     builds and runs with EXECUTE;
//!   * an erasure kind whose name has none of the shapes the kind closure looks for
//!     ([`erasure_shaped`]: `*_erased`, `*_scrubbed`, `*erasure*`) and no role in
//!     `AUTHORITY_BY_ROLE`. A kind named `*_purged`, `*_redacted`, `*_struck` or `*_tombstoned`
//!     would be in no class and fail nothing. The registry carries no flag that marks a kind as an
//!     erasure's, so the closure matches names; whoever registers such a kind must add it to a
//!     class or to `AUTHORITY_BY_ROLE`, in review.
//!
//! The checker is a pure function over (consumers, roster, vocabulary), so the bite probes run in
//! memory over the real bodies with one change each, one probe per failure branch.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sqlx::PgPool;
use temper_substrate::events::EventKind;
use temper_substrate::payloads::{
    ADMIN_ACT_KINDS, ANCHORED_ACT_KINDS, PER_ITEM_AUTHORITY_KINDS, REDACTION_AUTHORITY_KINDS,
    STRIKE_SOURCE_KINDS, TYPED_EVENT_NAMES,
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

/// The items under `crates/*/src` that are the vocabulary, not consumers of it: the `EventKind`
/// name maps, the registration lists, the class constants and the typed round-trip's dispatch.
/// Each must still name a family kind, or the entry is stale.
const VOCABULARY_ITEMS: [&str; 10] = [
    "rust:crates/temper-substrate/src/events.rs#as_canonical_name",
    "rust:crates/temper-substrate/src/events.rs#from_canonical_name",
    "rust:crates/temper-substrate/src/payloads.rs#TYPED_EVENT_NAMES",
    "rust:crates/temper-substrate/src/payloads.rs#ADMIN_EVENT_NAMES",
    "rust:crates/temper-substrate/src/payloads.rs#PER_ITEM_AUTHORITY_KINDS",
    "rust:crates/temper-substrate/src/payloads.rs#REDACTION_AUTHORITY_KINDS",
    "rust:crates/temper-substrate/src/payloads.rs#STRIKE_SOURCE_KINDS",
    "rust:crates/temper-substrate/src/payloads.rs#ADMIN_ACT_KINDS",
    "rust:crates/temper-substrate/src/payloads.rs#ANCHORED_ACT_KINDS",
    "rust:crates/temper-substrate/src/payloads.rs#verify_ledger_roundtrip",
];

/// Kinds that are an erasure authority by their role though their name does not say so. Each is
/// checked by [`kind_problems`] as an erasure-shaped name is.
const AUTHORITY_BY_ROLE: [&str; 1] = ["relationship_folded"];

/// Registered kinds with an erasure's shape that are deliberately in no class, each with why.
const EXCLUDED_KINDS: [(&str, &str); 3] = [
    (
        "principal_erasure_refused",
        "a refusal record: it mutates nothing and authorises nothing (its replay arm is a no-op)",
    ),
    (
        "resource_erasure_refused",
        "a refusal record: it mutates nothing and authorises nothing (its replay arm is a no-op)",
    ),
    (
        "relationship_folded",
        "an ordinary edge fold, which the erasure acts also append under their own correlation; it \
         is an erasure authority only through a `rel: 'erasure'` reference, which slice 2a adds \
         (slice 1b plan P9), and every name-keyed reader of it reads folds, not erasures",
    ),
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
    AnchoredAct,
}

const CLASSES: [Class; 5] = [
    Class::PerItemAuthority,
    Class::RedactionAuthority,
    Class::StrikeSource,
    Class::AdminAct,
    Class::AnchoredAct,
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
                (Class::AnchoredAct, set(&ANCHORED_ACT_KINDS)),
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
    /// Names every kind of each of `classes`, plus exactly `own`, and no other family kind.
    Union {
        classes: &'static [Class],
        own: &'static [&'static str],
    },
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

const fn union_of(
    who: &'static str,
    classes: &'static [Class],
    own: &'static [&'static str],
    reason: &'static str,
) -> Entry {
    Entry {
        who,
        claim: Claim::Union { classes, own },
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
    union_of(
        "_event_is_admin_act(text,uuid,uuid,xid8)",
        &[Class::AdminAct, Class::AnchoredAct],
        &[],
        "an event correlated to an admin erasure act is the act's own work, and an anchored \
         erasure kind is an admin act by its kind alone",
    ),
    union_of(
        "_profile_change_events(uuid)",
        &[Class::AdminAct, Class::AnchoredAct],
        &[],
        "reader changes exclude the events an admin erasure act appended, and every anchored \
         erasure",
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
    union_of(
        "resource_erasure_survey_plan(uuid)",
        &[Class::PerItemAuthority],
        &["blob_erased"],
        "reads one item's own erasure authority record, and names blob_erased as the kind its \
         blobs are struck under",
    ),
    own(
        "_principal_erasure_contexts_to_erase(uuid[])",
        &["context_erased"],
        "the estate contexts with no context_erased record of their own yet",
    ),
    own(
        "context_reassign(jsonb,uuid,jsonb,uuid,uuid)",
        &["context_erased"],
        "refuses a context its own context_erased record has erased",
    ),
    own(
        "context_restore(jsonb,uuid,jsonb,uuid,uuid)",
        &["context_erased"],
        "refuses a context its own context_erased record has erased",
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
        &["principal_erased", "blob_erased", "context_erased"],
        "the person erasure act, appending its own kinds: the record, each struck blob, each \
         estate context",
    ),
    own(
        "resource_erasure_execute(uuid,uuid,uuid,uuid,uuid[])",
        &["resource_erased", "blob_erased"],
        "the resource erasure act, appending its own kinds: the record and each struck blob",
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
            "edge_erased",
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
    own(
        "rust:crates/temper-services/src/services/context_service.rs#restore",
        &["context_erased"],
        "refuses to restore a context its own context_erased record has erased",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_service.rs#estate_stragglers",
        &["context_erased"],
        "counts the estate contexts with no context_erased record of their own",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_service.rs#strike_verdicts",
        &["blob_erased"],
        "reads the person act's own blob_erased records",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_service/tests.rs#a_personal_homed_blob_is_struck_and_its_verdict_rides_the_payload",
        &["blob_erased"],
        "a unit test reading the person act's own blob_erased",
    ),
    own(
        "rust:crates/temper-services/src/services/erasure_service/tests.rs#re_erasing_a_tombstoned_subject_records_a_no_op_completion",
        &["blob_erased"],
        "a unit test reading the person act's own blob_erased",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#a_legacy_erasure_declared_omits_what_a_legacy_erasure_lacks",
        &["erasure_declared"],
        "a unit test of its own kind's payload",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#erasure_batch_applied_round_trips_both_outcomes",
        &["erasure_batch_applied"],
        "a unit test of its own kind's payload",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#erasure_completed_round_trips_with_and_without_its_optional_fields",
        &["erasure_completed"],
        "a unit test of its own kind's payload",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#erasure_declared_round_trips_an_estate_declaration",
        &["erasure_declared"],
        "a unit test of its own kind's payload",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#erasure_member_arrived_round_trips_each_source",
        &["erasure_member_arrived"],
        "a unit test of its own kind's payload",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#erasure_reopened_round_trips",
        &["erasure_reopened"],
        "a unit test of its own kind's payload",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#erasure_scope_sealed_round_trips",
        &["erasure_scope_sealed"],
        "a unit test of its own kind's payload",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#resource_erasure_payloads_roundtrip_and_carry_no_trail_join_key",
        &["resource_erased", "block_history_scrubbed"],
        "a unit test of its own kinds' payloads",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#resource_scrubbed_carries_no_trail_join_key_and_no_owner",
        &["resource_scrubbed"],
        "a unit test of its own kind's payload",
    ),
    own(
        "rust:crates/temper-substrate/src/payloads.rs#edge_erased_round_trips_and_carries_no_trail_join_key",
        &["edge_erased"],
        "a unit test of its own kind's payload",
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

/// The family kinds `text` holds as a quoted literal: `'kind'`, `"kind"` or `\"kind\"`.
fn named_in(text: &str, family: &BTreeSet<&'static str>) -> BTreeSet<&'static str> {
    family
        .iter()
        .copied()
        .filter(|kind| {
            text.contains(&format!("'{kind}'"))
                || text.contains(&format!("\"{kind}\""))
                || text.contains(&format!("\\\"{kind}\\\""))
        })
        .collect()
}

/// Whether a quoted pattern could match an erasure kind's name.
fn erasure_pattern(literal: &str) -> bool {
    [
        "_erased",
        "_scrubbed",
        "erasure",
        "_erase",
        "%erase",
        "%scrub",
    ]
    .iter()
    .any(|shape| literal.contains(shape))
}

/// Each place `text` matches kind names by pattern rather than naming them: a SQL `LIKE`,
/// `ILIKE`, `SIMILAR TO` or `~` whose left side is a `name` or `kind` and whose pattern could match
/// an erasure kind, or a Rust `ends_with` / `starts_with` on an erasure name's suffix or prefix.
fn pattern_references(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    for op in [" like '", " ilike '", " similar to '", " ~ '", " ~* '"] {
        for (at, _) in lower.match_indices(op) {
            let mut tokens = lower[..at]
                .split(|c: char| c.is_whitespace() || c == '(')
                .filter(|token| !token.is_empty())
                .rev();
            let mut operand = tokens.next().unwrap_or("");
            if operand == "not" {
                operand = tokens.next().unwrap_or("");
            }
            let operand = operand.trim_end_matches(['\'', '"', ')']);
            let start = at + op.len();
            let literal = lower[start..].split('\'').next().unwrap_or("");
            if (operand.ends_with("name") || operand.ends_with("kind")) && erasure_pattern(literal)
            {
                out.push(format!("{operand}{op}{literal}'"));
            }
        }
    }
    for call in [
        "ends_with(\"_erased\")",
        "ends_with(\"_scrubbed\")",
        "starts_with(\"erasure_\")",
    ] {
        if text.contains(call) {
            out.push(call.to_owned());
        }
    }
    out
}

/// Consumer key → its text.
type Bodies = BTreeMap<String, String>;

/// Outside the system schemas, as `live_bodies` reads every source.
const USER_SCHEMA: &str =
    "n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg\\_%'";

async fn live_bodies(pool: &PgPool) -> Bodies {
    let sources = [
        // Functions and procedures, keyed by regprocedure.
        format!(
            "SELECT p.oid::regprocedure::text, pg_get_functiondef(p.oid) \
               FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
              WHERE {USER_SCHEMA} AND p.prokind IN ('f', 'p')"
        ),
        // Views and materialized views.
        format!(
            "SELECT 'view:' || c.oid::regclass::text, pg_get_viewdef(c.oid) \
               FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
              WHERE {USER_SCHEMA} AND c.relkind IN ('v', 'm')"
        ),
        // Row-level security policies: USING and WITH CHECK.
        format!(
            "SELECT 'policy:' || c.oid::regclass::text || '.' || pol.polname::text, \
                    coalesce(pg_get_expr(pol.polqual, pol.polrelid), '') || ' ' || \
                    coalesce(pg_get_expr(pol.polwithcheck, pol.polrelid), '') \
               FROM pg_policy pol JOIN pg_class c ON c.oid = pol.polrelid \
               JOIN pg_namespace n ON n.oid = c.relnamespace \
              WHERE {USER_SCHEMA}"
        ),
        // CHECK constraints, on tables and on domains.
        format!(
            "SELECT 'constraint:' || coalesce(nullif(con.conrelid, 0)::regclass::text, \
                                              con.contypid::regtype::text) \
                    || '.' || con.conname::text, pg_get_constraintdef(con.oid) \
               FROM pg_constraint con JOIN pg_namespace n ON n.oid = con.connamespace \
              WHERE {USER_SCHEMA} AND con.contype = 'c'"
        ),
        // User triggers, their WHEN clause included.
        format!(
            "SELECT 'trigger:' || c.oid::regclass::text || '.' || tg.tgname::text, \
                    pg_get_triggerdef(tg.oid) \
               FROM pg_trigger tg JOIN pg_class c ON c.oid = tg.tgrelid \
               JOIN pg_namespace n ON n.oid = c.relnamespace \
              WHERE {USER_SCHEMA} AND NOT tg.tgisinternal"
        ),
        // Column defaults.
        format!(
            "SELECT 'default:' || c.oid::regclass::text || '.' || a.attname::text, \
                    pg_get_expr(d.adbin, d.adrelid) \
               FROM pg_attrdef d JOIN pg_class c ON c.oid = d.adrelid \
               JOIN pg_attribute a ON a.attrelid = d.adrelid AND a.attnum = d.adnum \
               JOIN pg_namespace n ON n.oid = c.relnamespace \
              WHERE {USER_SCHEMA}"
        ),
    ];
    let mut bodies = Bodies::new();
    for source in &sources {
        let rows: Vec<(String, String)> = sqlx::query_as(source)
            .fetch_all(pool)
            .await
            .unwrap_or_else(|e| panic!("read the live catalog ({source}): {e}"));
        bodies.extend(rows);
    }
    bodies
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

/// Every item of every `crates/*/src/**/*.rs` file, the vocabulary's among them.
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
        rust_items(&relative, &read_repo_file(&relative), &mut bodies);
    }
    bodies
}

/// The consumers: every body or item that names a family kind, with the kinds it names. The
/// vocabulary's own items ([`VOCABULARY_ITEMS`]) are not consumers.
fn consumers(
    bodies: &Bodies,
    vocabulary: &Vocabulary,
) -> BTreeMap<String, (String, BTreeSet<&'static str>)> {
    let family = vocabulary.family();
    bodies
        .iter()
        .filter(|(who, _)| !VOCABULARY_ITEMS.contains(&who.as_str()))
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
    let family = vocabulary.family();
    for item in VOCABULARY_ITEMS {
        if !bodies
            .get(item)
            .is_some_and(|text| !named_in(text, &family).is_empty())
        {
            out.push(format!(
                "{item} is listed as vocabulary but names no erasure kind: remove it from \
                 VOCABULARY_ITEMS"
            ));
        }
    }
    for (who, text) in bodies {
        if VOCABULARY_ITEMS.contains(&who.as_str()) {
            continue;
        }
        for pattern in pattern_references(text) {
            out.push(format!(
                "{who} matches erasure kinds by pattern ({pattern}): name each kind instead"
            ));
        }
    }
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
            Claim::Union { classes, own } => {
                let mut required: BTreeSet<&str> = own.iter().copied().collect();
                for class in classes {
                    required.extend(vocabulary.kinds(*class).iter().copied());
                }
                let missing: BTreeSet<&str> = required.difference(named).copied().collect();
                if !missing.is_empty() {
                    out.push(format!(
                        "{} is rostered to {classes:?} plus its own {own:?} but does not name {}",
                        entry.who,
                        list(&missing)
                    ));
                }
                let outside: BTreeSet<&str> = named.difference(&required).copied().collect();
                if !outside.is_empty() {
                    out.push(format!(
                        "{} is rostered to {classes:?} plus its own {own:?} but also names {}",
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

// ── the kinds ────────────────────────────────────────────────────────────────────────────────

/// Whether a kind's name has an erasure's shape.
fn erasure_shaped(name: &str) -> bool {
    name.ends_with("_erased") || name.ends_with("_scrubbed") || name.contains("erasure")
}

/// Every breach of the kind closure by `registered` (every registered kind's name) under
/// `vocabulary`. Empty when every erasure-shaped or role-named registered kind is in the family or
/// excluded, never both, every exclusion names a registered kind with a reason, and every family
/// kind is registered.
fn kind_problems(registered: &BTreeSet<String>, vocabulary: &Vocabulary) -> Vec<String> {
    let family = vocabulary.family();
    let excluded: BTreeMap<&str, &str> = EXCLUDED_KINDS.into_iter().collect();
    let mut out = Vec::new();
    for name in registered {
        let name = name.as_str();
        if !erasure_shaped(name) && !AUTHORITY_BY_ROLE.contains(&name) {
            continue;
        }
        match (family.contains(name), excluded.contains_key(name)) {
            (false, false) => out.push(format!(
                "{name} is a registered erasure kind in no class: add it to a class constant or \
                 RECORD_KINDS, or to EXCLUDED_KINDS with a reason"
            )),
            (true, true) => out.push(format!(
                "{name} is in the erasure family and also excluded from it"
            )),
            _ => {}
        }
    }
    for (name, reason) in EXCLUDED_KINDS {
        if !registered.contains(name) {
            out.push(format!(
                "{name} is excluded but is not a registered kind: remove the exclusion"
            ));
        }
        if reason.trim().is_empty() {
            out.push(format!("{name} is excluded without a reason"));
        }
    }
    for kind in family {
        if !registered.contains(kind) {
            out.push(format!(
                "{kind} is in the erasure family but is not a registered kind"
            ));
        }
    }
    out
}

/// Every registered kind: the `kb_event_types` rows the migrations seed, and the typed names.
async fn registered_kinds(pool: &PgPool) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = sqlx::query_scalar("SELECT name FROM kb_event_types")
        .fetch_all(pool)
        .await
        .expect("read the registered kinds")
        .into_iter()
        .collect();
    names.extend(TYPED_EVENT_NAMES.iter().map(|name| (*name).to_owned()));
    names
}

#[test]
fn the_classes_and_record_kinds_are_registered_kinds() {
    let vocabulary = Vocabulary::live();
    for kind in vocabulary.family() {
        assert!(
            EventKind::from_canonical_name(kind).is_some(),
            "{kind} is in the erasure family but has no EventKind arm"
        );
    }
    for class in CLASSES {
        assert!(
            !vocabulary.kinds(class).is_empty(),
            "{class:?} is not empty"
        );
        assert!(
            ROSTER.iter().any(|entry| match entry.claim {
                Claim::Class(c) => c == class,
                Claim::Union { classes, .. } => classes.contains(&class),
                Claim::Own(_) => false,
            }),
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

/// Every registered erasure kind is in a class, in the record kinds, or excluded with a reason.
///
/// FAILS IF: a registered kind named `*_erased`, `*_scrubbed` or `*erasure*` (or one in
/// `AUTHORITY_BY_ROLE`) is in no class and not excluded; a kind is both; an exclusion names no
/// registered kind; or a family kind is not registered.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_registered_erasure_kind_is_in_a_class_or_excluded(pool: PgPool) {
    let registered = registered_kinds(&pool).await;
    assert!(
        registered.contains("context_erased") && registered.contains("erasure_declared"),
        "setup: the registered kinds are read from the migrated database ({registered:?})"
    );
    assert_eq!(
        kind_problems(&registered, &Vocabulary::live()),
        Vec::<String>::new()
    );
}

/// The kind closure's bite: a registered `foo_erased` kind in no class fails, and so does a family
/// kind that is not registered, and an exclusion of a kind that is.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unassigned_erased_kind_fails_the_kind_closure(pool: PgPool) {
    let real = registered_kinds(&pool).await;
    let vocabulary = Vocabulary::live();
    assert_eq!(
        kind_problems(&real, &vocabulary),
        Vec::<String>::new(),
        "setup: the closure holds"
    );

    let mut registered = real.clone();
    registered.insert("foo_erased".to_owned());
    assert_eq!(
        kind_problems(&registered, &vocabulary),
        vec![
            "foo_erased is a registered erasure kind in no class: add it to a class constant or \
             RECORD_KINDS, or to EXCLUDED_KINDS with a reason"
                .to_owned()
        ],
        "a registered foo_erased in no class fails, and nothing else does"
    );

    let mut assigned = vocabulary.clone();
    assigned
        .classes
        .get_mut(&Class::AnchoredAct)
        .expect("the class is in the vocabulary")
        .insert("foo_erased");
    assert_eq!(
        kind_problems(&registered, &assigned),
        Vec::<String>::new(),
        "assigned to a class, foo_erased holds the closure"
    );
    assert_eq!(
        kind_problems(&real, &assigned),
        vec!["foo_erased is in the erasure family but is not a registered kind".to_owned()],
        "a family kind that is not registered fails"
    );

    let mut excluded_too = vocabulary.clone();
    excluded_too.records.insert("resource_erasure_refused");
    assert_eq!(
        kind_problems(&real, &excluded_too),
        vec![
            "resource_erasure_refused is in the erasure family and also excluded from it"
                .to_owned()
        ],
        "a kind both in the family and excluded fails"
    );
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

/// A kind added to `PER_ITEM_AUTHORITY_KINDS` fails every consumer rostered to the class, alone or
/// in a `Union`, and nothing else.
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
        .filter_map(|entry| match entry.claim {
            Claim::Class(Class::PerItemAuthority) => Some(format!(
                "{} is rostered to PerItemAuthority but does not name {added}",
                entry.who
            )),
            Claim::Union { classes, own } if classes.contains(&Class::PerItemAuthority) => {
                Some(format!(
                    "{} is rostered to {classes:?} plus its own {own:?} but does not name {added}",
                    entry.who
                ))
            }
            _ => None,
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

/// A kind added to `ANCHORED_ACT_KINDS` fails both `Union` consumers that hold the class, and
/// nothing else: a `Union` claim grows with each of its classes.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_kind_added_to_the_anchored_act_class_fails_its_union_consumers(pool: PgPool) {
    let bodies = holding_bodies(&pool).await;
    let added = "d76_probe_anchored_kind";
    let mut vocabulary = Vocabulary::live();
    vocabulary
        .classes
        .get_mut(&Class::AnchoredAct)
        .expect("the class is in the vocabulary")
        .insert(added);
    let expected: Vec<String> = ROSTER
        .iter()
        .filter_map(|entry| match entry.claim {
            Claim::Union { classes, own } if classes.contains(&Class::AnchoredAct) => {
                Some(format!(
                    "{} is rostered to {classes:?} plus its own {own:?} but does not name {added}",
                    entry.who
                ))
            }
            _ => None,
        })
        .collect();
    assert_eq!(expected.len(), 2, "setup: two consumers hold the class");
    assert_eq!(problems(&bodies, ROSTER, &vocabulary), expected);
}

/// A consumer that matches kind names by pattern fails, in SQL text and in Rust, whatever it is
/// rostered to.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_consumer_matching_kinds_by_pattern_fails(pool: PgPool) {
    let real = holding_bodies(&pool).await;
    for (line, pattern) in [
        (
            "\n    \"SELECT 1 FROM kb_event_types t WHERE t.name LIKE '%_erased'\"\n",
            "t.name like '%_erased'",
        ),
        (
            "\n    let probe = kind.ends_with(\"_erased\");\n",
            "ends_with(\"_erased\")",
        ),
    ] {
        let mut bodies = real.clone();
        bodies
            .get_mut(FENCE_SEED)
            .expect("the fence seed is a Rust item")
            .push_str(line);
        assert_eq!(
            problems(&bodies, ROSTER, &Vocabulary::live()),
            vec![format!(
                "{FENCE_SEED} matches erasure kinds by pattern ({pattern}): name each kind instead"
            )],
            "the pattern {pattern} is reported, and nothing else"
        );
    }
}

/// A kind named inside an escaped JSON string in Rust is a reference like any other.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_kind_named_in_an_escaped_json_string_is_a_consumer(pool: PgPool) {
    let mut bodies = holding_bodies(&pool).await;
    let text = format!(
        "{}\nfn d76_probe_json() -> &'static str {{\n    \"{{\\\"kind\\\": \\\"blob_erased\\\"}}\"\n}}\n",
        read_repo_file(FENCE_FILE)
    );
    bodies.retain(|who, _| !who.starts_with(&format!("{RUST}{FENCE_FILE}")));
    rust_items(FENCE_FILE, &text, &mut bodies);
    let who = format!("{RUST}{FENCE_FILE}#d76_probe_json");
    assert_eq!(
        problems(&bodies, ROSTER, &Vocabulary::live()),
        vec![format!(
            "{who} names blob_erased but is not rostered: assign it a class, or its own kinds \
             with a reason"
        )]
    );
}

/// A new item in `payloads.rs`, outside its vocabulary items, is scanned like any other item, and
/// a vocabulary item that stops naming a kind fails as stale.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_vocabulary_files_are_scanned_outside_their_vocabulary_items(pool: PgPool) {
    let real = holding_bodies(&pool).await;
    let payloads = "crates/temper-substrate/src/payloads.rs";

    let mut bodies = real.clone();
    let text = format!(
        "{}\nfn d76_probe_payload_reader() -> &'static str {{\n    \"resource_erased\"\n}}\n",
        read_repo_file(payloads)
    );
    bodies.retain(|who, _| !who.starts_with(&format!("{RUST}{payloads}")));
    rust_items(payloads, &text, &mut bodies);
    assert_eq!(
        problems(&bodies, ROSTER, &Vocabulary::live()),
        vec![format!(
            "{RUST}{payloads}#d76_probe_payload_reader names resource_erased but is not rostered: \
             assign it a class, or its own kinds with a reason"
        )]
    );

    let mut bodies = real.clone();
    let item = VOCABULARY_ITEMS[0];
    bodies.insert(item.to_owned(), String::new());
    assert_eq!(
        problems(&bodies, ROSTER, &Vocabulary::live()),
        vec![format!(
            "{item} is listed as vocabulary but names no erasure kind: remove it from \
             VOCABULARY_ITEMS"
        )]
    );
}

/// Each catalog source beyond functions is read: a view, a CHECK constraint, a column default, a
/// row-level security policy and a trigger's WHEN clause, each naming one kind, are each a consumer
/// under its own key. One object per source, each naming a different kind, so a source query that
/// returned no rows would lose exactly its line.
///
/// FAILS IF: any of the five source queries misses its object (its key is absent from the
/// report), or reads it under another key.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_catalog_source_naming_a_kind_is_a_consumer(pool: PgPool) {
    for ddl in [
        "CREATE VIEW d76_probe_view AS SELECT 'resource_erased'::text AS kind",
        "CREATE TABLE d76_probe_table (k text CHECK (k <> 'principal_erased'), \
                                       d text DEFAULT 'block_history_scrubbed')",
        "CREATE POLICY d76_probe_policy ON d76_probe_table USING (k <> 'resource_scrubbed')",
        "CREATE FUNCTION d76_probe_noop() RETURNS trigger LANGUAGE plpgsql \
             AS $$ BEGIN RETURN NEW; END $$",
        "CREATE TRIGGER d76_probe_trigger BEFORE INSERT ON d76_probe_table FOR EACH ROW \
             WHEN (NEW.k = 'erasure_completed') EXECUTE FUNCTION d76_probe_noop()",
    ] {
        sqlx::query(ddl)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{ddl}: {e}"));
    }
    let unrostered = |who: &str, kind: &str| {
        format!(
            "{who} names {kind} but is not rostered: assign it a class, or its own kinds with a \
             reason"
        )
    };
    assert_eq!(
        problems(&all_bodies(&pool).await, ROSTER, &Vocabulary::live()),
        vec![
            unrostered(
                "constraint:d76_probe_table.d76_probe_table_k_check",
                "principal_erased"
            ),
            unrostered("default:d76_probe_table.d", "block_history_scrubbed"),
            unrostered(
                "policy:d76_probe_table.d76_probe_policy",
                "resource_scrubbed"
            ),
            unrostered(
                "trigger:d76_probe_table.d76_probe_trigger",
                "erasure_completed"
            ),
            unrostered("view:d76_probe_view", "resource_erased"),
        ]
    );
}
