//! `door_coverage` is checked against the deployed system, not merely pinned — ADJ-9d.
//!
//! # What this file is for
//!
//! [`ActDeclaration::door_coverage`] claims, per act and per door, *can a caller standing here ask
//! this*. Until this file existed, every test over that field was **internal-consistency only**:
//! `registry.rs`'s own `mod tests` asserts the declaration is self-coherent (every door accounted
//! for, no shortfall naming a term the act does not admit) and pins the literal values. Not one of
//! them read the code the declaration describes.
//!
//! That is not a hypothetical gap. On 2026-08-10 a conformance pass found **six of twenty-one cells
//! overstated** — `follow-from` and `survey` each declared full reach at all three doors while
//! nothing outside temper-substrate's own tests had called `search_graph_expand` or
//! `wayfind_region_scores` since `/api/search` stopped expanding across edges. The declaration was
//! corrected by hand and the correction was evidenced by inline `[verified — file:line]` markers,
//! which are prose: they went stale the moment the code moved, and nothing would have said so.
//!
//! The failure mode is worth naming precisely, because the pinning tests look like coverage.
//! `the_cli_cannot_page_the_find_acts_and_that_is_declared` used to assert `terms_unreachable ==
//! [Offset]` for the three find acts at the CLI, comparing the declaration to itself — if `temper
//! search` gained `--offset`, that test would stay **green** regardless, because it reads the
//! declaration and compares it to itself. A declaration checked only against its own literal is a
//! declaration nothing checks. That is exactly what happened: `temper search` gained `--offset`,
//! and the pinning test was inverted by hand into
//! `the_cli_can_now_page_the_find_acts_and_that_is_declared`. What would have caught the gap
//! independently of that hand-edit is the sibling in `temper-cli`,
//! `act_door_coverage_cli_terms.rs`'s `the_cli_term_shortfall_is_what_clap_actually_lacks` — it
//! derives the shortfall from `Cli::command()`'s real parser tree rather than from the declaration,
//! which is why it went red the moment the flag shipped. THIS file's oracle is the two doors that
//! share `SearchParams` (`Door::Api`/`Door::Mcp`, see below) — the CLI's half of tier 2 needs
//! clap's command tree and lives in that sibling instead.
//!
//! # The oracle, and why it is two things rather than one
//!
//! Every act's `served_by` names a **set-returning SQL function**. So "is this act reachable at all"
//! reduces to "does production code invoke that function", which two independent artifacts answer —
//! and neither is a call graph, because Rust hands a test no call graph and a hand-written one would
//! be [the `ADMIN_EVENT_TYPES` failure](../src/types/query/act.rs) this field exists to avoid: a
//! const beside a registry, with a test holding its own second copy.
//!
//! 1. **The `.sqlx` cache.** Compiler-produced, and it excludes test-target queries *by
//!    construction* rather than by filter — the workspace `cargo sqlx prepare` ritual does not cover
//!    them. Presence therefore **proves** a production compile-time call site.
//! 2. **A `FROM <name>(` scan of production `crates/*/src/`.** Catches what the cache cannot see.
//!
//! **It takes both, and the reason is measured rather than defensive.** `search_wide` is invoked
//! through a runtime `sqlx::query_as` because its `::vector` cast forbids the compile-time macro, so
//! it is live and **absent from `.sqlx`**. Trusting the cache alone would report the two
//! `find-about-*` acts unreachable while they serve every door — a false negative, which is the
//! dangerous direction here: it would make the gate demand that a true declaration be downgraded.
//!
//! The scan is `FROM <name>(` rather than a bare name because the loose forms are all wrong in a way
//! that was checked, not guessed. A plain name match hits two classes of non-invocation, both with
//! live instances in production `src/` today:
//!
//! - **prose** — `temper-services/src/backend/substrate_read.rs:938` discusses
//!   `wayfind_region_scores` in a doc comment about the T7 NaN trap;
//! - **string literals** — `registry.rs:246` carries `served_by: Some("search_graph_expand"…)`,
//!   which is the declaration of the mechanic rather than a call to it, and is precisely the field
//!   whose truth this file is checking.
//!
//! `[re-cited — 2026-08-12]` The second bullet used to cite *"the validator's identifier allowlist
//! (`validate.rs:63` carries `"search_graph_expand"` as a string)"*, and both halves of that are now
//! dead: `validate.rs` was split into `validate/{shape,capability,mod}.rs`, and
//! `search_graph_expand` left `CALLABLE_FRAGMENTS` when `follow-from` was flipped to refuse
//! statically. The ARGUMENT survives the citation — a `served_by` literal is the same class of
//! false positive, and a nearer one.
//!
//! Requiring a call shape excludes both; requiring the SQL `FROM` form additionally excludes the
//! Rust wrapper *definitions* that share their SQL function's name (`pub async fn search_exact(`),
//! which are declarations rather than invocations and could in principle sit there unused.
//!
//! # What this file checks, and what it structurally cannot
//!
//! **Checked (tier 1):** whether an act is reachable from *anywhere*. This is the tier that catches
//! the six, and it goes red on the day a mechanic's last production caller disappears rather than at
//! the next audit.
//!
//! **Checked (tier 2):** which *bound terms* a door can supply, for the two doors whose surface is a
//! shared Rust type. The CLI's half of tier 2 needs clap's command tree and therefore lives in
//! `temper-cli` — see `act_door_coverage_cli_terms.rs`.
//!
//! **NOT checked, and declared so rather than left silent:** *which* door reaches a mechanic, among
//! doors that could. Both oracles are workspace-global — they answer "production calls this", never
//! "the MCP door calls this". Discriminating would need a per-door entry-point table maintained by
//! hand, i.e. exactly the second copy this field exists to avoid. The live instance is
//! `substantiate`: it declares `Serves` at CLI and API and `Absent` at MCP, and tier 1 sees only
//! that `resource_standing_shape` is reachable from somewhere. That cell is reviewed, not checked.
//!
//! [`the_cells_tier_one_cannot_discriminate_are_exactly_these`] pins that uncovered set, so a new
//! act or a new door lands in it and forces a deliberate restatement instead of quietly inheriting
//! "we check door coverage."
//!
//! Also not checked: `bounds_unreachable` and `filters_unapplied`. Both are reviewed. The bound axis
//! is close to mechanical — `SearchParams` has no resource-id slot, which is the whole of the live
//! claim — but the filter axis asks whether a door *applies* a filter it accepted, which is a
//! question about behaviour rather than about surface, and no artifact answers it.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use temper_core::types::api::SearchParams;
use temper_core::types::query::{search_family, ActName, BoundTerm, Door, DoorReach};

// ---------------------------------------------------------------------------
// The scanners
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    // `crates/temper-core` -> the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("temper-core sits two levels below the workspace root")
        .to_path_buf()
}

/// Every `.rs` file under `crates/*/<segment>`, where `segment` is `src` or `tests`.
///
/// Split deliberately: the production/test partition is the whole point of tier 1, and
/// [`the_partition_between_production_and_test_sources_is_load_bearing`] proves it is doing work
/// rather than scanning an empty set.
fn rs_files_under(segment: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let crates = workspace_root().join("crates");
    let entries = fs::read_dir(&crates).expect("crates/ is readable");
    for entry in entries.flatten() {
        let dir = entry.path().join(segment);
        if dir.is_dir() {
            collect_rs(&dir, &mut out);
        }
    }
    out.sort();
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `path:line` for every SQL invocation of `sql_fn` in `files`.
///
/// The `FROM <name>(` form — see the module note on why the looser forms are wrong.
fn sql_call_sites(files: &[PathBuf], sql_fn: &str) -> Vec<String> {
    let mut sites = Vec::new();
    for path in files {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line_invokes(line, sql_fn) {
                sites.push(format!("{}:{}", path.display(), i + 1));
            }
        }
    }
    sites
}

/// Does this line carry `FROM <sql_fn>(`, case-insensitively on the keyword?
fn line_invokes(line: &str, sql_fn: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find("from ") {
        let after = from + rel + "from ".len();
        let rest = line[after..].trim_start();
        if let Some(tail) = rest.strip_prefix(sql_fn) {
            if tail.trim_start().starts_with('(') {
                return true;
            }
        }
        from = after;
    }
    false
}

/// How many `.sqlx` cache entries mention `sql_fn`.
///
/// Sound but incomplete — see the module note. Used as a cross-check, never as the sole oracle.
fn sqlx_cache_mentions(sql_fn: &str) -> usize {
    let cache = workspace_root().join(".sqlx");
    let Ok(entries) = fs::read_dir(&cache) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| fs::read_to_string(e.path()).is_ok_and(|text| text.contains(sql_fn)))
        .count()
}

fn production_files() -> Vec<PathBuf> {
    rs_files_under("src")
}

fn test_files() -> Vec<PathBuf> {
    rs_files_under("tests")
}

// ---------------------------------------------------------------------------
// Tier 1 — reachability
// ---------------------------------------------------------------------------

/// **The gate.** An act declaring `Serves` at any door must have a production call site for its
/// mechanic; an act `Absent` at every door must have none.
///
/// This is the assertion that would have caught the six. It is stated as an equality rather than as
/// two implications on purpose: the two failure directions are different defects and both matter.
/// Over-claiming (`Serves` with no caller) is the one that occurred — a door promised and not there.
/// Under-claiming (`Absent` with a live caller) is the ADJ-9a shape in reverse, and it is how a
/// shipped affordance goes unlisted; `substantiate` spent a release declared `Unbuilt` while
/// `GET /api/resources/{id}/evidence` served it, caught by a human reading a doc comment.
#[test]
fn every_declared_reach_has_a_production_call_site() {
    let prod = production_files();
    assert!(
        prod.len() > 100,
        "scanned {} production files — the walk is broken, and a broken walk makes every act \
         look unreachable",
        prod.len()
    );

    for act in search_family() {
        let serves_somewhere = act
            .door_coverage
            .values()
            .any(|reach| matches!(reach, DoorReach::Serves { .. }));

        match &act.served_by {
            Some(sql_fn) => {
                let sites = sql_call_sites(&prod, sql_fn);
                assert_eq!(
                    serves_somewhere,
                    !sites.is_empty(),
                    "{:?} declares reach at {} door(s) and `{sql_fn}` has {} production call \
                     site(s): {sites:?}. Either the declaration overstates a door nobody can \
                     stand at, or a shipped affordance is unlisted.",
                    act.name,
                    act.door_coverage
                        .values()
                        .filter(|r| matches!(r, DoorReach::Serves { .. }))
                        .count(),
                    sites.len(),
                );
            }
            None => {
                // No mechanic named, so there is nothing a door could reach. `admit` is the live
                // instance and its absence is the anti-act's entire point.
                assert!(
                    !serves_somewhere,
                    "{:?} names no `served_by` yet claims a door reaches it",
                    act.name
                );
            }
        }
    }
}

/// Where the `.sqlx` cache can see a mechanic, it agrees with the source scan.
///
/// A cross-check between two independent derivations, not a second gate. The cache is **sound and
/// incomplete**: presence proves a production compile-time call site, absence proves nothing,
/// because a runtime `sqlx::query_as` never enters it. So this asserts only the one direction the
/// cache can support — and [`the_two_oracles_disagree_exactly_where_the_vector_cast_forces_it`]
/// pins the known incompleteness so it cannot silently widen.
#[test]
fn the_sqlx_cache_never_contradicts_the_source_scan() {
    let prod = production_files();
    for act in search_family() {
        let Some(sql_fn) = &act.served_by else {
            continue;
        };
        if sqlx_cache_mentions(sql_fn) > 0 {
            assert!(
                !sql_call_sites(&prod, sql_fn).is_empty(),
                "`{sql_fn}` is in the .sqlx cache — which only a production compile-time query \
                 macro can put there — yet the source scan found no call site. The scan's rule is \
                 too narrow."
            );
        }
    }
}

/// The one mechanic the cache cannot see, pinned so the hole cannot widen unnoticed.
///
/// `search_wide` binds `$2::vector`, and the cast forbids the compile-time macro (`readback/mod.rs`
/// says so at the call site). If this test ever fails because the set grew, the module note's claim
/// that the cache alone is unsound needs restating with the new members — and if it fails because
/// the set shrank to empty, the second oracle has become redundant and should be retired
/// deliberately rather than left as unexplained belt-and-braces.
#[test]
fn the_two_oracles_disagree_exactly_where_the_vector_cast_forces_it() {
    let prod = production_files();
    let invisible_to_cache: BTreeSet<String> = search_family()
        .into_iter()
        .filter_map(|act| act.served_by)
        .filter(|sql_fn| {
            !sql_call_sites(&prod, sql_fn).is_empty() && sqlx_cache_mentions(sql_fn) == 0
        })
        .collect();

    assert_eq!(
        invisible_to_cache,
        BTreeSet::from(["search_wide".to_string()]),
        "the set of live mechanics invisible to the .sqlx cache has moved"
    );
}

// ---------------------------------------------------------------------------
// Bite probes — the gate's own non-vacuity
// ---------------------------------------------------------------------------

/// The production/test partition is real, and it is what tier 1 rests on.
///
/// `search_graph_expand` is invoked in the `FROM` form the scanner looks for — in
/// `crates/temper-substrate/tests/search_graph_expand.rs`, a **test**. So the scanner is not
/// reporting it absent because it finds nothing anywhere; it is reporting it absent from
/// production while finding it in tests. Without this probe, a path bug that made both scans empty
/// would leave `follow-from`'s and `survey`'s `Absent` declarations passing for the wrong reason.
#[test]
fn the_partition_between_production_and_test_sources_is_load_bearing() {
    let stranded = "search_graph_expand";
    let in_tests = sql_call_sites(&test_files(), stranded);
    let in_prod = sql_call_sites(&production_files(), stranded);

    assert!(
        !in_tests.is_empty(),
        "`{stranded}` is invoked in no test either — the scanner is finding nothing anywhere, so \
         tier 1's `Absent` verdicts prove nothing"
    );
    assert!(
        in_prod.is_empty(),
        "`{stranded}` now has a production caller at {in_prod:?} — `follow-from` has acquired a \
         door and its declaration must say so"
    );
}

/// The scanner finds every mechanic that is genuinely live, and nothing for a name that is not.
///
/// The positive half guards against a scan rule so strict it matches nothing (which would make
/// tier 1 demand that every true `Serves` be downgraded). The negative half guards against one so
/// loose it matches prose (which is how a bare-name scan fails — the module note records both).
#[test]
fn the_scanner_separates_live_mechanics_from_names_that_merely_appear() {
    let prod = production_files();

    for live in ["search_exact", "search_wide", "resource_standing_shape"] {
        assert!(
            !sql_call_sites(&prod, live).is_empty(),
            "`{live}` is invoked in production and the scanner missed it"
        );
    }

    // Named nowhere in the repo: a floor under the rule.
    assert!(
        sql_call_sites(&prod, "search_sideways_thrice").is_empty(),
        "the scanner reports a call site for a function that does not exist"
    );

    // Named in production prose and in the validator's allowlist, invoked nowhere. This is the
    // discrimination a bare-name scan gets wrong, so it is asserted rather than assumed.
    assert!(
        sql_call_sites(&prod, "wayfind_region_scores").is_empty(),
        "the scanner is matching doc comments or identifier allowlists, not invocations"
    );
}

// ---------------------------------------------------------------------------
// Tier 2 — bound terms, for the doors that share one Rust surface
// ---------------------------------------------------------------------------

/// The wire slot a bound term arrives in, or `None` if the shared params type has no slot for it.
///
/// Exhaustive on purpose — no `_` arm — so a fourth [`BoundTerm`] cannot land here silently and be
/// scored against whatever the wildcard caught.
fn wire_slot(term: BoundTerm) -> Option<&'static str> {
    match term {
        BoundTerm::Limit => Some("limit"),
        BoundTerm::Offset => Some("offset"),
        // `survey`'s funnel width. No door carries it, and `survey` is `Absent` at all three, so
        // there is nothing for this to be checked against today.
        BoundTerm::Regions => None,
    }
}

/// The API and MCP doors reach exactly the terms `SearchParams` carries a slot for.
///
/// One check covers two doors because the two doors take the **same Rust type** —
/// `temper-api`'s handler takes `Json<SearchParams>` and the MCP tool takes
/// `Parameters<SearchParams>`. That identity is what makes this test's reach legitimate, and it is
/// asserted at the MCP door itself rather than assumed here (`temper-mcp`'s tool tests), because if
/// the two ever diverged this test would silently keep speaking for a door it no longer describes.
#[test]
fn the_shared_params_doors_declare_exactly_the_terms_that_type_carries() {
    let slots: BTreeSet<String> =
        match serde_json::to_value(SearchParams::default()).expect("SearchParams serializes") {
            serde_json::Value::Object(map) => map.keys().cloned().collect(),
            other => panic!("SearchParams must serialize as an object, got {other:?}"),
        };

    for act in search_family() {
        for door in [Door::Api, Door::Mcp] {
            let Some(DoorReach::Serves {
                terms_unreachable, ..
            }) = act.door_coverage.get(&door)
            else {
                continue;
            };

            let derived: BTreeSet<BoundTerm> = act
                .accepts_bound_terms
                .iter()
                .copied()
                .filter(|term| wire_slot(*term).is_none_or(|slot| !slots.contains(slot)))
                .collect();
            let declared: BTreeSet<BoundTerm> = terms_unreachable.iter().copied().collect();

            assert_eq!(
                declared, derived,
                "{:?} at {door:?} declares {declared:?} unreachable; `SearchParams` carries \
                 slots {slots:?}, which makes {derived:?} unreachable",
                act.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The declared hole
// ---------------------------------------------------------------------------

/// The cells tier 1 admits it cannot discriminate — pinned, so the hole cannot grow in silence.
///
/// Tier 1 answers *is this mechanic reachable from production*, which is workspace-global. For an
/// act whose mechanic **is** reachable, every one of its per-door cells is therefore consistent with
/// tier 1 whatever it says, and the per-door claim rests on review alone. Today that is
/// `substantiate` at MCP: it declares `Absent` while CLI and API declare `Serves`, and no artifact
/// this file can read distinguishes the three.
///
/// The pinned set is *(act, door)* for every act with a reachable mechanic — the whole row, not just
/// the interesting cell, because the property is "tier 1 cannot speak here", which is true of the
/// `Serves` cells and the `Absent` one alike.
///
/// Failing this test is not a defect. It means the surface moved and someone must restate what is
/// checked — which is the entire point, since the alternative is inheriting the phrase "we check
/// door coverage" over a set that has quietly changed shape.
#[test]
fn the_cells_tier_one_cannot_discriminate_are_exactly_these() {
    let prod = production_files();
    let mut uncovered: BTreeSet<(String, Door)> = BTreeSet::new();

    for act in search_family() {
        let Some(sql_fn) = &act.served_by else {
            continue;
        };
        if sql_call_sites(&prod, sql_fn).is_empty() {
            continue; // tier 1 speaks for this act: no caller, so every cell must be `Absent`.
        }
        for door in Door::ALL {
            uncovered.insert((format!("{:?}", act.name), door));
        }
    }

    let expected: BTreeSet<(String, Door)> = [
        ActName::FindExact,
        ActName::FindAboutAnywhere,
        ActName::FindAboutWithin,
        ActName::Substantiate,
    ]
    .into_iter()
    .flat_map(|name| Door::ALL.into_iter().map(move |d| (format!("{name:?}"), d)))
    .collect();

    assert_eq!(
        uncovered, expected,
        "the set of per-door cells resting on review rather than on a check has moved; restate \
         the module note's 'NOT checked' paragraph before re-pinning this"
    );
}
