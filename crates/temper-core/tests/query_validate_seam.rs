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
        ("DuplicateStageName", 1),
        ("EmptyContains", 1),
        ("EmptyPropertyKey", 1),
        ("MissingIntention", 1),
        ("MissingProvenance", 1),
        ("NoReturns", 1),
        ("NoStages", 1),
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
