//! The two guards on the shape/capability seam — spec ⟨3⟩.
//!
//! Guard one asks whether the shape module can reach a declaration. Guard two asks which reasons
//! it actually emits. Both are needed, and the second catches what the first structurally cannot:
//! six capability sites read no declaration at all, so an import scan alone would happily let them
//! sit in the shape pass, where a stale client would raise them against a newer server.
//!
//! **What the pair still does not cover is larger than an aliased re-export.** `[stated —
//! 2026-08-12]` A shape check that starts consulting a declaration *through a helper in
//! `validate/mod.rs`* — `emitted_fragment_for` is one that exists today — reaches the registry
//! without naming any of guard one's four forbidden strings, and if it emits an already-pinned
//! reason at the same count it also leaves guard two's table untouched. Both guards are then green
//! over exactly the migration they exist to catch. Nothing here closes that: the boundary is the
//! module's own imports plus review, and these two only make the common careless routes loud.

use std::collections::BTreeMap;

const SHAPE_SRC: &str = include_str!("../src/types/query/validate/shape.rs");

/// The shape module's source with comment lines removed.
///
/// **Both guards scan CODE, not prose**, and this is not a convenience. The rule is about what
/// `shape.rs` can call, and its own header necessarily *names* the things it may not reach — it
/// explains the seam. Scanning raw source made guard one fail on the doc comment that documents
/// it, which is a test failing on its own subject matter. Found while writing this file.
///
/// Line-oriented and deliberately crude: it strips `//` line comments only. A `/* */` block
/// comment would defeat it, which is why `no_block_comments_in_the_shape_module` exists below.
fn shape_code() -> String {
    SHAPE_SRC
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The stripper above only understands `//`. Say so, and hold it.
#[test]
fn no_block_comments_in_the_shape_module() {
    assert!(
        !SHAPE_SRC.contains("/*"),
        "`validate/shape.rs` gained a block comment. `shape_code()` strips only `//` lines, so \
         a forbidden name inside `/* */` would slip past both guards. Use `//`, or teach the \
         stripper."
    );
}

/// Guard one — the shape module has no route to the act declarations.
#[test]
fn the_shape_module_reaches_no_declaration() {
    let code = shape_code();
    for forbidden in [
        "registry",
        "declaration(",
        "search_family",
        "CALLABLE_FRAGMENTS",
    ] {
        assert!(
            !code.contains(forbidden),
            "`validate/shape.rs` calls `{forbidden}` in code. A refusal that consults what this \
             server has built cannot be raised by a client that does not share its binary."
        );
    }
}

/// Guard two — the shape pass emits exactly these reasons, exactly this many times each.
///
/// Pinned rather than derived, because the classification is a JUDGMENT. `FilterNotApplicable`
/// at `capability.rs`'s "this door does not yet apply" sites reads no declaration and would pass
/// guard one; it belongs to capability because Task 10b retires it. Nothing but this pin records
/// that.
///
/// **Counts, not a set** — and this is the whole reason the guard works. Two variants are emitted
/// by BOTH passes: `BoundTermNotApplicable` (negative value here, 32-bit and not-admitted there)
/// and `AnchorTakesOneId` (zero ids here, more than one there). Over a SET, either module's site
/// migrating into shape would change nothing, because the reason is already in shape's set — the
/// guard would sit there green while the exact defect it exists to catch walked past it. Over
/// counts, shape's tally for that reason goes 1 → 2 and it fails.
#[test]
fn the_shape_pass_emits_exactly_these_reasons() {
    let code = shape_code();
    let mut found: BTreeMap<&str, usize> = BTreeMap::new();
    // **The uncovered direction that matters is not a stray `RefusalReason::` in a string literal**
    // — that inflates a count and reddens loudly, which is the harmless way to be wrong. It is a
    // refusal pushed through a `RefusalReason`-valued VARIABLE or helper parameter: the call site
    // names no variant textually, this scan counts nothing, and a capability check moved here that
    // way is invisible to guard two entirely.
    for (i, _) in code.match_indices("RefusalReason::") {
        let tail = &code[i + "RefusalReason::".len()..];
        let end = tail
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(tail.len());
        *found.entry(&tail[..end]).or_insert(0) += 1;
    }

    // Every shape reason but one is emitted from exactly ONE site. That uniformity was a fact
    // about the code rather than a rule, and this comment already said what to do when it stopped
    // holding: "if a future shape check legitimately raises an existing reason from a second site,
    // bump its count here and say why".
    //
    // `CombinatorArity` is 2 `[since 2026-08-15]`. `CombineOp::Difference` is ordered, so it has a
    // ceiling as well as a floor — a third input is refused rather than folded into the `EXCEPT`
    // chain Postgres would otherwise evaluate happily. The two sites raise the same reason because
    // the caller's repair is the same shape of thing (fix the input count) and a second variant
    // would make a client's handling depend on which end of the range it missed.
    //
    // **Both sites are still shape's**, which is what this guard is actually protecting: the
    // ceiling reads `CombineOp::is_ordered`, a property of the wire enum, and consults no
    // declaration — so it remains a refusal a client can raise against a server it does not share
    // a binary with.
    let expected: BTreeMap<&str, usize> = [
        ("AnchorTakesOneId", 1),
        ("BoundTermNotApplicable", 1),
        ("CombinatorArity", 2),
        ("StageNotReturnable", 1),
        ("Cycle", 1),
        ("DanglingReference", 1),
        ("DuplicateInputRelation", 1),
        ("DuplicateReturnStage", 1),
        // `DuplicateSetMember` is **2** `[added — 2026-08-28]`, and the count is the point: the two
        // sites are `ReturnSpec::with` and `EdgeFilter::edge_kinds`, both closed vocabularies
        // carried as lists where a repeat changes no answer. One reason for both because the
        // caller's repair is the same shape of thing — the second-site precedent is
        // `CombinatorArity` above, and the rule it set is that a shared reason bumps the count here
        // rather than earning a variant.
        //
        // **3 as of 2026-08-28**, the third being `CombineNode::inputs` — found in review, and
        // missed first time because this list holds stage NAMES rather than a closed enum while
        // the argument is identical: inputs must name declared stages, so a repeat is the one way
        // to make an unbounded list out of a bounded vocabulary.
        //
        // Both clear the shape bar without needing anything published, and that is worth stating
        // because the neighbouring caps DID need it: this refuses a list that is malformed against
        // ANY vocabulary, so no server can widen its way out of it. A server that admitted a new
        // section would still not admit that section twice.
        ("DuplicateSetMember", 3),
        ("DuplicateStageName", 1),
        // `FilterStringTooLong` `[added — 2026-10-06]`: one site over every narrowing string.
        // Clears the bar through `max_length` on each scalar field (`inner` on the MCP door's
        // lists) and the published description on the lists.
        ("FilterStringTooLong", 1),
        // `ListTooLong` and `TextBudgetExceeded` `[added — 2026-10-06]`: `max_items` on `returns`
        // and both `inputs` fields, and the text budget's number in `TextBudgetExceeded`'s and
        // `MAX_COMPOSITION_TEXT_BYTES`' published description.
        ("ListTooLong", 1),
        ("EmptyContains", 1),
        ("EmptyPropertyKey", 1),
        // `IntentionTooLong` and `TooManyIds` `[added — 2026-08-28]`. Two more entries clearing the
        // same bar `TooManyStages` cleared, and clearing it the same way: each ceiling is published
        // on the field it bounds — `max_length` on `Intention::query`, `max_items` on `IdSet::ids`
        // — so raising either is an `openapi.json` change the drift gates see, never a
        // per-deployment choice a stale `temper query --check` could contradict.
        //
        // **`TooManyIds` is not `AnchorTakesOneId`'s many arm arriving here**, which is what a
        // reader checking this table against the seam would want to know. That arm stays in
        // capability: it refuses a cogmap or context bound above one id because today's fragments
        // take an `(anchor_table, anchor_id)` pair, and an `anchor_ids uuid[]` retires it. This one
        // is a contract fact about every kind. `AnchorTakesOneId`'s count below is therefore still
        // 1 — its zero arm — and a migration of the many arm into this module would still show up
        // as that count going to 2.
        ("IntentionTooLong", 1),
        // `MalformedEmbedding` `[added — 2026-08-28]`. Published as `max_items` on
        // `Intention::embedding`, so it clears the bar the same way — but the refusal names the
        // SHAPE rather than the ceiling, because a vector of any other length belongs to a
        // different space and a caller told about a maximum would fix the wrong end.
        // `[widened — 2026-09-02]` Count 1 → 3: the length arm was joined by the finite arm and
        // the norm arm, all under the same reason. Three sites, one wire vocabulary — a caller
        // cannot tell which arm fired from the reason and should not have to; the message text
        // carries the difference.
        ("MalformedEmbedding", 3),
        ("MissingIntention", 1),
        ("MissingProvenance", 1),
        ("NoReturns", 1),
        ("NoStages", 1),
        // Both clear the bar through the operator's published description: the byte bounds are
        // stated there as numbers, so changing either is an `openapi.json` change, and
        // `the_published_property_value_bounds_are_the_enforced_ones` pins them to the constants.
        ("PropertyValueBudgetExceeded", 1),
        ("PropertyValueTooLarge", 1),
        // `TooManyStages` `[added — 2026-08-26]`. A new entry, and this guard's own message names
        // the bar it has to clear: "asserting it cannot change without a wire-contract change".
        // It clears it — the ceiling is `max_items` on `Composition::stages`, so raising the cap
        // is an `openapi.json` change the drift gates see, not a per-deployment choice a stale
        // `temper query --check` could contradict.
        // `TooManyFilterValues` `[added — 2026-08-28]`, one site over three fields on purpose —
        // three sites would be three numbers waiting to drift, and would say three judgments were
        // made where there was one. Clears the bar the same way `TooManyIds` does: `max_items` on
        // each of `doc_type`, `tags` and `labels`, published on both doors.
        ("TextBudgetExceeded", 1),
        ("TooManyFilterValues", 1),
        ("TooManyIds", 1),
        ("TooManyStages", 1),
        ("UnknownAct", 1),
        ("UnknownReturnStage", 1),
    ]
    .into_iter()
    .collect();

    assert_eq!(
        found, expected,
        "the shape pass's emitted reasons moved. A new entry means asserting it cannot change \
         without a wire-contract change; a removed entry means it moved to capability and \
         `temper query --check` stopped reporting it; a changed COUNT means a capability site \
         migrated into shape under a reason shape already raises."
    );
}

// ── The artifact asks the offline check reaches ────────────────────────────────────────────────
//
// `validate_shape` reaches two more modules than `shape.rs`: `validate/asks.rs` (the request caps,
// through `check_forms`) and `artifact_ask/validate.rs` (each ask's form, through
// `validate_ask_form`). Both modules ALSO hold refusals only the server raises — the scope conflict
// in `asks.rs`, the shape steps and `include_folded` in `validate.rs` — because each module is one
// definition shared by both passes. So guard two's per-file count cannot say which pass a site
// belongs to, and the two guards below say what they can: guard one, that neither module can reach a
// declaration; and a pinned count per module, each reason classed by the pass that raises it, so a
// new refusal site in either module is a reviewed change. A site moving between an offline and a
// server-only function inside one module, at the same count, is the remainder these cannot see;
// `the_offline_check_refuses_an_asks_form_and_never_its_shape` (temper-core, `validate/mod.rs`)
// pins that direction by behaviour for the cases that exist.

const ASKS_SRC: &str = include_str!("../src/types/query/validate/asks.rs");
const ASK_VALIDATE_SRC: &str = include_str!("../src/types/artifact_ask/validate.rs");

/// A module's code before its test module, with `//` comment lines removed — the same crude
/// stripping as [`shape_code`], and held to the same no-block-comments rule.
fn production_code(src: &str) -> String {
    src.lines()
        .take_while(|l| !l.starts_with("#[cfg(test)]"))
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn reason_sites(code: &str) -> BTreeMap<&str, usize> {
    let mut found: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, _) in code.match_indices("RefusalReason::") {
        let tail = &code[i + "RefusalReason::".len()..];
        let end = tail
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(tail.len());
        *found.entry(&tail[..end]).or_insert(0) += 1;
    }
    found
}

#[test]
fn no_block_comments_in_the_ask_modules_the_offline_check_reaches() {
    for (module, src) in [
        ("validate/asks.rs", ASKS_SRC),
        ("artifact_ask/validate.rs", ASK_VALIDATE_SRC),
    ] {
        assert!(
            !production_code(src).contains("/*"),
            "`{module}` gained a block comment; `production_code()` strips only `//` lines"
        );
    }
}

/// Guard one, for the ask modules: the form checks `validate_shape` runs consult no declaration.
#[test]
fn the_ask_modules_the_offline_check_reaches_reach_no_declaration() {
    for (module, src) in [
        ("validate/asks.rs", ASKS_SRC),
        ("artifact_ask/validate.rs", ASK_VALIDATE_SRC),
    ] {
        let code = production_code(src);
        for forbidden in [
            "registry",
            "declaration(",
            "search_family",
            "CALLABLE_FRAGMENTS",
        ] {
            assert!(
                !code.contains(forbidden),
                "`{module}` calls `{forbidden}` in code, and `validate_shape` reaches it. A refusal \
                 that consults what this server has built cannot be raised by a client that does \
                 not share its binary."
            );
        }
    }
}

/// Guard two, for the ask modules: the reasons each emits and how many sites each, classed by the
/// pass that raises it. OFFLINE sites are reached by `validate_shape` and the server alike; SERVER
/// sites only through `AskSession::validate` or `validate_asks`, after shapes are loaded.
#[test]
fn the_ask_modules_emit_exactly_these_reasons() {
    let asks: BTreeMap<&str, usize> = [
        // SERVER: `check_scope` needs each admitted ask's shape home.
        ("AskScopeConflict", 1),
        // SERVER: `repeats_an_ask` compares the validated binds, which need the loaded shape.
        ("DuplicateSetMember", 1),
        // OFFLINE: the two request caps, `within_request_caps`.
        ("TooManyFilterValues", 2),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        reason_sites(&production_code(ASKS_SRC)),
        asks,
        "`validate/asks.rs`'s refusal sites moved. Class each new one as OFFLINE (reached by \
         `check_forms`) or SERVER, and keep every capability fact out of the offline half."
    );

    let ask_validate: BTreeMap<&str, usize> = [
        // OFFLINE: a repeated intent or state term (`resolve_terms`).
        ("DuplicateSetMember", 1),
        // OFFLINE: the path and value form (`well_formed`).
        ("EmptyArtifactPath", 1),
        ("EmptyContains", 1),
        // OFFLINE ×2: the predicate and probe caps (`within_caps`). SERVER ×1: `include_folded`
        // (`refuse_unserved`) — whether it is served is the server's capability, so it is never
        // in the form check.
        ("FilterNotApplicable", 3),
        ("FilterStringTooLong", 1),
        // SERVER: step 4, against the loaded shape.
        ("OperatorOutsideShape", 1),
        ("PathOutsideShape", 1),
        // OFFLINE.
        ("PropertyValueTooLarge", 1),
        // SERVER: steps 1–3, the loader's outcomes (`ShapeLoad::loaded`, `not_askable`).
        ("ShapeNotAskable", 1),
        ("ShapeSchemaUncompilable", 2),
        ("ShapeSuperseded", 2),
        // OFFLINE ×2: path depth (`well_formed`), and a term list longer than its vocabulary
        // (`resolve_terms`).
        ("TooManyFilterValues", 2),
        // OFFLINE: the term vocabularies (`INTENTS`, `SHAPE_STATES`).
        ("UnknownArtifactIntent", 1),
        ("UnknownShapeState", 1),
        // OFFLINE: a NUL no stored content can hold.
        ("UnstorableValue", 1),
        // SERVER: step 4.
        ("ValueOutsideShape", 1),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        reason_sites(&production_code(ASK_VALIDATE_SRC)),
        ask_validate,
        "`artifact_ask/validate.rs`'s refusal sites moved. Class each new one as OFFLINE (reached \
         by `validate_ask_form`) or SERVER, and keep every capability fact out of the form check."
    );
}
