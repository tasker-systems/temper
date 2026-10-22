//! The one place a data-artifact shape's JSON Schema is compiled.
//!
//! A shape is declared by any author of its home, so its schema is untrusted input. `jsonschema`
//! under its default options resolves a `$ref` (or an off-list `$schema`) by fetching it: an
//! `http(s)` URI over the network, a `file:` URI from the server's disk. A caller who can declare
//! a shape could therefore make the server connect to an internal address, or read a local file,
//! on every compile — and the compiler's error text can echo what came back.
//!
//! [`compile`] closes that, in two layers:
//!
//! 1. **The retriever (the control that survives).** `compile` builds with a retriever that refuses
//!    every URI, for `$ref` and `$schema` alike, so nothing is fetched and nothing remote is ever
//!    read. A retriever holds under cargo feature unification: another crate in the graph enabling
//!    a `jsonschema` resolver feature cannot widen what this compile does.
//! 2. **No resolvers compiled in (defence in depth).** Every workspace `jsonschema` dependency
//!    sets `default-features = false`, which drops the http and file resolvers and the TLS
//!    provider from the binary. A feature switch-off alone may not hold — unification can turn the
//!    features back on for a dependency that asks for them — which is why it is the second layer
//!    and not the first (task `01a15661-8460-739d-9ed3-f1e8ccbf9b69`).
//!
//! **How each layer is bitten.** With the resolvers compiled out, dropping `.with_retriever(..)`
//! alone no longer makes a witness connect, because the default retriever has nothing to call. To
//! bite the retriever, re-enable the `resolve-http` and `resolve-file` features AND drop the
//! retriever; the `shape_schema_no_fetch` witnesses then fail. Re-enabling the features while
//! keeping the retriever fails nothing, which is the point of the retriever being the control. The
//! tripwire (`audit-shape-schema-compile.sh`) stops a new default-options compile being written.
//!
//! Every server-side compile of a shape schema goes through here — commit, reconcile and declare
//! today, and any later reader of a shape. The tripwire fails CI when another `jsonschema` entry
//! point appears under `crates/*/src`, so a new caller cannot reintroduce the defaults.
//!
//! The draft is not chosen here: `jsonschema` selects it from the schema's own `$schema`, which is
//! the selection a validator built any other way would make. A `$schema` outside the five standard
//! drafts names a custom meta-schema, and a schema whose draft cannot be determined is never
//! guessed at (spec §3.3 row 2). `jsonschema` refuses a custom `$schema` only when its URI is not
//! among the document's own resources, so a schema that registers that URI under an `$id` of its
//! own (at the root or in an embedded subschema) would compile under a default draft.
//!
//! [`compile`] therefore refuses, before it builds, any document in which **any** object carries
//! a string `$schema` that `Draft::detect` does not map to a standard draft — wherever in the
//! document that object sits. A non-string `$schema` is left to meta-validation.
//!
//! **Accepted cost.** The rule is document-wide, not positional, so a custom `$schema` string
//! inside data — a `const`, `enum`, `default` or `examples` value, or an unknown keyword — is
//! refused too, with the same fixed sentence. Position cannot tell data from schema: a reference
//! can make any position in the document a schema, and telling them apart would mean reproducing
//! every spelling of a reference that `jsonschema` resolves. The rule fails closed instead.
//!
//! **Each `pattern` and `patternProperties` regex is held to a size.** `jsonschema` compiles every
//! distinct `pattern` and `patternProperties` key when a schema is built, and [`compile`] holds
//! each of those regexes to [`MAX_PATTERN_BYTES`] rather than the engine's 10 MiB default; one past
//! it makes the schema uncompilable, with the same fixed sentence, for every caller (declare,
//! commit, reconcile and the shape loader all compile here). The bound is per regex: no schema
//! that compiles carries a `pattern` or `patternProperties` regex past it.

use jsonschema::{Draft, Retrieve, Uri, Validator};
use serde_json::Value;

/// The schema cannot be compiled without a fetch, or cannot be compiled at all.
///
/// Carries nothing: the compiler's own error is dropped on purpose. Under a fetching compiler it
/// describes a remote response, and under any compiler it is the declarer's material, not the
/// reader's. [`Display`](std::fmt::Display) is one fixed sentence that tells a declarer what to
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeSchemaUncompilable;

impl std::fmt::Display for ShapeSchemaUncompilable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "the shape's JSON Schema cannot be compiled: it must be a valid JSON Schema in a \
             standard draft (2020-12 unless its $schema names another), and every $ref must stay \
             inside the document (for example #/$defs/name) because a $ref to a URL, a file or \
             another document is never fetched",
        )
    }
}

impl std::error::Error for ShapeSchemaUncompilable {}

/// The retriever that refuses every URI. It never reads the URI's scheme, host or path, so it
/// has no input from which a fetch could be made.
struct RefuseEveryUri;

impl Retrieve for RefuseEveryUri {
    fn retrieve(
        &self,
        _uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(Box::new(ShapeSchemaUncompilable))
    }
}

/// Whether every object in the document whose `$schema` is a string names one of the five
/// standard drafts — every object, wherever it sits, not only the ones in schema positions.
///
/// Detection is `Draft::detect`, the function `jsonschema` itself selects a draft with: a string
/// `$schema` it does not recognise is `Draft::Unknown`, and a missing or non-string one detects as
/// the default draft, so a non-string `$schema` is left to meta-validation. Every object and array
/// is pushed once onto an explicit stack, so the walk is linear in the document's size and never
/// recurses on the Rust stack.
fn names_only_standard_drafts(schema: &Value) -> bool {
    let draft = Draft::default();
    let mut pending = vec![schema];
    while let Some(node) = pending.pop() {
        match node {
            Value::Object(members) => {
                if draft.detect(node) == Draft::Unknown {
                    return false;
                }
                pending.extend(members.values());
            }
            Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    true
}

/// The most bytes one `pattern` (or `patternProperties` key) may compile to — the regex engine's
/// per-regex size limit, which counted repetitions expand into.
///
/// # 1 MiB
///
/// `jsonschema` compiles every distinct pattern eagerly when a schema is built, and the engine's
/// own default allows 10 MiB a regex, so a pattern of twenty bytes such as a large Unicode class
/// repeated a few hundred times can cost milliseconds and megabytes on its own. A pattern a person
/// writes for a field — a code, a date, an identifier — compiles to kilobytes; 1 MiB admits two
/// orders of magnitude more. A `pattern` or `patternProperties` key past it fails to compile, so
/// its schema is [`ShapeSchemaUncompilable`], with the one fixed sentence. It bounds each such
/// regex.
pub const MAX_PATTERN_BYTES: usize = 1 << 20;

/// The most bytes the engine may spend building a **full** DFA for one pattern — what
/// `PatternOptions::dfa_size_limit` reaches (`fancy-regex`'s `delegate_dfa_size_limit`, which is
/// `regex-automata`'s meta `dfa_size_limit`), not the lazy DFA's search cache. Past it the engine
/// does not build a full DFA and searches with another engine; nothing is refused.
///
/// # 40 KiB
///
/// The engine's own default, pinned: a full DFA costs worst-case exponential time and space to
/// build, which is why the default is this small. Setting it to anything like
/// [`MAX_PATTERN_BYTES`] would raise it twenty-five-fold; pinning it keeps an upstream change of
/// default from moving what building one pattern's full DFA may cost.
pub const MAX_PATTERN_DFA_BYTES: usize = 40 << 10;

/// Compile a shape's JSON Schema into a validator without fetching anything, holding each pattern
/// to [`MAX_PATTERN_BYTES`].
///
/// # Errors
/// [`ShapeSchemaUncompilable`] when the schema is not valid for its draft, carries a string
/// `$schema` outside the standard drafts anywhere in the document (even one the schema itself
/// defines, or one inside a value), has a `$ref` that leaves the document, or has a pattern that
/// compiles past [`MAX_PATTERN_BYTES`].
pub fn compile(schema: &Value) -> Result<Validator, ShapeSchemaUncompilable> {
    #[cfg(feature = "test-support")]
    test_support::record(schema);
    if !names_only_standard_drafts(schema) {
        return Err(ShapeSchemaUncompilable);
    }
    let patterns = jsonschema::PatternOptions::fancy_regex()
        .size_limit(MAX_PATTERN_BYTES)
        .dfa_size_limit(MAX_PATTERN_DFA_BYTES);
    jsonschema::options()
        .with_retriever(RefuseEveryUri)
        .with_pattern_options(patterns)
        .build(schema)
        .map_err(|_| ShapeSchemaUncompilable)
}

/// What a test reads to see which schemas [`compile`] was run on — every call, from every caller,
/// so a compile that skipped a bound is seen wherever it was written.
#[cfg(feature = "test-support")]
pub mod test_support {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock, PoisonError};

    use serde_json::Value;

    static COMPILES: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

    /// Count one compile of a schema whose root carries a string `$comment` — the tag a test gives
    /// its fixture, so that suites compiling other schemas in parallel cannot move its count.
    pub(super) fn record(schema: &Value) {
        if let Some(tag) = schema.get("$comment").and_then(Value::as_str) {
            let mut compiles = COMPILES
                .get_or_init(Mutex::default)
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *compiles.entry(tag.to_owned()).or_default() += 1;
        }
    }

    /// How many times [`compile`](super::compile) has run, in this process, on a schema whose root
    /// `$comment` is `tag`.
    pub fn compiles_tagged(tag: &str) -> usize {
        COMPILES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(tag)
            .copied()
            .unwrap_or(0)
    }
}
