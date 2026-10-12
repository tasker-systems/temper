//! What it means for the person act's redaction, `_erasure_apply_redaction`, to erase a column.
//! `personal_data_surface_test.rs` binds each `erased` claim with it, and
//! `erasure_policy_catalog_test.rs` uses the same binding to check the scan columns of identity
//! tables. One copy, so the two cannot disagree.
//!
//! It builds on `erasure_binding`, which every user of this module also declares.

use crate::erasure_binding::{assignments, is_erasure_value};

/// The person act's subject sentinel, as its body declares it. `v_sentinel` is an erasure value
/// only while this declaration stands: the binding reads a variable's value from nowhere else.
const SENTINEL_DECLARATION: &[&str] = &[
    "v_sentinel",
    "text",
    ":",
    "=",
    "'erased-'",
    "||",
    "p_subject",
    "::",
    "text",
    ";",
];

/// Whether a SET value erases in the person act: one of the resource binder's shapes, or a `||`
/// chain of string literals and the declared `v_sentinel` that holds the sentinel at least once
/// (`'personal-' || v_sentinel`). Nothing read from a row may join the chain.
fn erases(expr: &[String], sentinel_declared: bool) -> bool {
    if is_erasure_value(expr) {
        return true;
    }
    let operand = |tok: &String| tok == "v_sentinel" || tok.starts_with('\'');
    sentinel_declared
        && expr.len() % 2 == 1
        && expr.iter().step_by(2).all(operand)
        && expr.iter().skip(1).step_by(2).all(|op| op == "||")
        && expr.iter().any(|tok| tok == "v_sentinel")
}

/// Whether `t` assigns `key` at least once, and every assignment erases.
pub fn assigns_erasure(t: &[String], key: &str) -> bool {
    let sentinel = t
        .windows(SENTINEL_DECLARATION.len())
        .any(|w| w.iter().zip(SENTINEL_DECLARATION).all(|(a, b)| a == b));
    let values = assignments(t, key);
    !values.is_empty() && values.into_iter().all(|v| erases(v, sentinel))
}
