//! The CLI half of `door_coverage`'s term AND bound-kind axes, read from clap's own tree —
//! ADJ-9d, tier 2.
//!
//! # Why this half lives here, and why it carries the weight
//!
//! `door_coverage`'s three shortfall axes are declared per door. The **term** axis is empty at
//! every door today — `registry.rs`'s three `unified_doors` call sites all pass `vec![]` for
//! `cli_unreachable`, and the sibling in `temper-core` (`act_door_coverage_reachability.rs`, which
//! checks the two doors that share `SearchParams`) checks the same emptiness against that type's
//! wire slots. This file still carries the weight, though: its oracle is clap's parser tree, the
//! only artifact that can prove a term unreachable rather than merely assert it. The direction with
//! content has inverted — every term an act admits must have a flag, not the other way around — and
//! [`the_cli_term_shortfall_is_what_clap_actually_lacks`] now checks it in that direction.
//!
//! The **bound-kind** axis is empty at every door too, as of the same branch that emptied it in
//! `registry.rs`: `find-exact` and `find-about-within` can now supply a `Resource` bound
//! everywhere. [`temper_search_can_bound_to_a_resource_set`] is this file's half of that —
//! `--within`'s presence, asserted unconditionally against the parser rather than derived, the
//! same shape [`temper_search_can_page`] already established for `--offset`.
//!
//! It lives in `temper-cli` because the evidence does. The claim is *a caller standing at the CLI
//! has no way to supply this term*, and the only artifact that answers it is clap's command tree —
//! which is the thing that actually parses the flags, not a list of them.
//!
//! # The defect this replaces
//!
//! The declaration used to be evidenced by prose. `survey`'s `door_coverage` comment cited
//! `--wayfind` and `--regions` as CLI flags qualifying its reach; `temper search` has neither, and
//! had neither when that comment was written (`registry.rs`, the ADJ-9a note). A citation to a flag
//! that does not exist reads exactly like a citation to one that does, and nothing in the build
//! could tell them apart. Reading `get_arguments()` cannot cite a flag that is not there.
//!
//! And the pinning test it complemented was blind in the direction that mattered:
//! `the_cli_cannot_page_the_find_acts_and_that_is_declared` asserted `terms_unreachable ==
//! [Offset]` by comparing the declaration to a literal, so the day `temper search` gained
//! `--offset` that test would have stayed green while the declaration went false. That day has
//! arrived — the flag shipped, the declaration was updated by hand alongside it, and the pinning
//! test is now `the_cli_can_now_page_the_find_acts_and_that_is_declared` (inverted, asserting full
//! reach rather than a literal shortfall). This file's `temper_search_can_page`, below, is the live
//! instance: it reads clap's tree rather than the declaration, so it would go red on its own if the
//! flag were ever removed while the declaration still claimed full reach.
//!
//! # Scope, stated rather than assumed
//!
//! `Door::Cli` is not one command. `temper search` is the CLI door for the acts served by the two
//! search fragments; `substantiate`'s CLI door is `temper resource evidence`. So this file speaks
//! only for acts whose mechanic is `query_find_exact` or `query_find_wide`, and
//! [`no_act_outside_the_search_command_declares_a_cli_term_shortfall`] asserts that every other act
//! has nothing for it to check — which is what keeps "this file covers the CLI term axis" true
//! rather than true-of-the-part-I-looked-at.

use std::collections::BTreeSet;

use clap::CommandFactory;
use temper_cli::cli::Cli;
use temper_core::types::query::{search_family, ActDeclaration, BoundTerm, Door, DoorReach};

/// The mechanics whose CLI door is `temper search`.
///
/// `[moved — 2026-08-12]` These were `search_exact` / `search_wide` until `/api/search` gained a
/// resource bound and `served_by` was repointed at the bound-accepting twins. The literals did not
/// travel, so `is_search_command_act` answered `false` for every act and the gate below iterated an
/// empty loop — passing, with nothing in it. `temper_search_can_page` is what still had content, and
/// it covers one flag rather than the axis.
const SEARCH_COMMAND_MECHANICS: [&str; 2] = ["query_find_exact", "query_find_wide"];

/// Every long flag `temper search` accepts, from the parser itself.
fn search_flags() -> BTreeSet<String> {
    let cmd = Cli::command();
    let search = cmd
        .get_subcommands()
        .find(|c| c.get_name() == "search")
        .expect("`temper search` exists");
    search
        .get_arguments()
        .filter_map(|a| a.get_long().map(str::to_string))
        .collect()
}

/// The flag a bound term arrives on, or `None` if the term has no flag spelling at this door.
///
/// Exhaustive — no `_` arm — so a fourth [`BoundTerm`] cannot be scored against whatever a wildcard
/// caught. `Regions` returning `None` is a real statement: `survey` is the only act admitting it,
/// it is `Absent` at every door, and `temper search` has no funnel-width flag.
fn cli_flag(term: BoundTerm) -> Option<&'static str> {
    match term {
        BoundTerm::Limit => Some("limit"),
        BoundTerm::Offset => Some("offset"),
        BoundTerm::Regions => None,
    }
}

fn serves_cli(act: &ActDeclaration) -> Option<&Vec<BoundTerm>> {
    match act.door_coverage.get(&Door::Cli) {
        Some(DoorReach::Serves {
            terms_unreachable, ..
        }) => Some(terms_unreachable),
        _ => None,
    }
}

fn is_search_command_act(act: &ActDeclaration) -> bool {
    act.served_by
        .as_deref()
        .is_some_and(|f| SEARCH_COMMAND_MECHANICS.contains(&f))
}

/// **The gate.** What `temper search` declares unreachable is what its parser has no flag for.
#[test]
fn the_cli_term_shortfall_is_what_clap_actually_lacks() {
    let flags = search_flags();
    assert!(
        flags.contains("limit"),
        "`temper search` has no --limit — either the flag moved or the parser tree is not being \
         read, and an empty flag set would make every term look unreachable"
    );

    // **The loop must have content, and until 2026-08-12 nothing said so.** Both `continue`s below
    // are silent skips, so a declaration moving out from under this test empties the loop and the
    // gate passes having checked nothing. That is not hypothetical: it is what repointing
    // `served_by` at the twins did, and `temper_search_can_page`'s own doc predicted it in words
    // ("the general gate would pass vacuously with nothing left to check") without asserting it.
    // Counted rather than merely non-zero, because two of the three find acts skipping is the same
    // defect in a quieter form.
    let mut checked = 0usize;

    for act in search_family() {
        if !is_search_command_act(&act) {
            continue;
        }
        let Some(terms_unreachable) = serves_cli(&act) else {
            continue;
        };
        checked += 1;

        let derived: BTreeSet<BoundTerm> = act
            .accepts_bound_terms
            .iter()
            .copied()
            .filter(|term| cli_flag(*term).is_none_or(|flag| !flags.contains(flag)))
            .collect();
        let declared: BTreeSet<BoundTerm> = terms_unreachable.iter().copied().collect();

        assert_eq!(
            declared, derived,
            "{:?} declares {declared:?} unreachable at the CLI; `temper search` accepts \
             {flags:?}, which makes {derived:?} unreachable",
            act.name
        );
    }

    assert_eq!(
        checked, 3,
        "the three find acts stand at `temper search`, and this gate checked {checked} of them. \
         Zero means `SEARCH_COMMAND_MECHANICS` no longer matches any `served_by` — the literals \
         are a second copy of names that are allowed to move, and this assertion is what stops the \
         gate going quiet instead of red when they do."
    );
}

/// The live instance, asserted from the parser rather than from the declaration.
///
/// Kept as its own test beside the general gate for a different reason than its predecessor: the
/// general gate would itself catch `--offset` disappearing today (`derived` would gain `Offset`
/// while `declared` stayed empty). What it cannot catch is the declarations moving out from under
/// it — `served_by` no longer naming a search fragment, or `Door::Cli` going `Absent`, both make
/// `is_search_command_act`/`serves_cli` skip the act, so the general gate would pass vacuously with
/// nothing left to check. This test asserts the flag's presence unconditionally, with no
/// declaration in the loop to short-circuit on.
#[test]
fn temper_search_can_page() {
    let flags = search_flags();
    assert!(
        flags.contains("offset"),
        "`temper search` has lost --offset. Every find act's CLI `terms_unreachable` must \
         regain `Offset`, and `the_cli_can_now_page_the_find_acts_and_that_is_declared` in \
         registry.rs compares the declaration to a literal and will not notice on its own."
    );
}

/// The CLI half of `bounds_unreachable`'s Api/Mcp derivation
/// (`the_shared_params_doors_declare_exactly_the_bound_kinds_that_type_carries` in
/// `act_door_coverage_reachability.rs`), which cannot itself see clap's tree. `bounds_unreachable`
/// flipped from a pessimistic literal (`[Resource]`) to an optimistic empty one (`[]`) with the CLI
/// side pinned nowhere — this is that pin, in the shape `temper_search_can_page` already
/// established for the term axis: an unconditional assertion against the parser, with no
/// declaration in the loop to short-circuit on.
#[test]
fn temper_search_can_bound_to_a_resource_set() {
    let flags = search_flags();
    assert!(
        flags.contains("within"),
        "`temper search` has lost --within. `find-exact` and `find-about-within`'s CLI \
         `bounds_unreachable` must regain `IdKind::Resource`, and \
         `every_door_can_now_supply_the_resource_bound_the_find_acts_accept` in registry.rs \
         compares the declaration to itself and will not notice on its own."
    );
}

/// This file's scope claim, checked rather than asserted in prose.
///
/// An act served by something other than the two search fragments has a different CLI door, and
/// this file has read the wrong parser subtree for it. Today every such act declares an empty term
/// shortfall, so there is nothing being missed. If one ever declares a non-empty one, this test
/// fails and whoever added it must either extend this file to that command or say plainly that the
/// cell is reviewed rather than checked.
#[test]
fn no_act_outside_the_search_command_declares_a_cli_term_shortfall() {
    for act in search_family() {
        if is_search_command_act(&act) {
            continue;
        }
        let Some(terms_unreachable) = serves_cli(&act) else {
            continue;
        };
        assert!(
            terms_unreachable.is_empty(),
            "{:?} declares {terms_unreachable:?} unreachable at the CLI, but its door is not \
             `temper search` — this file cannot speak for it",
            act.name
        );
    }
}
