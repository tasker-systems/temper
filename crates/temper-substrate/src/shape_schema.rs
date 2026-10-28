//! The one place a data-artifact shape's JSON Schema is compiled.
//!
//! A shape is declared by any author of its home, so its schema is untrusted input. `jsonschema`
//! under its default options resolves a `$ref` (or an off-list `$schema`) by fetching it: an
//! `http(s)` URI over the network, a `file:` URI from the server's disk. A caller who can declare
//! a shape could therefore make the server connect to an internal address, or read a local file,
//! on every compile — and the compiler's error text can echo what came back.
//!
//! **A shape is a field tree.** [`compile`] first holds the document to a small vocabulary — the
//! keywords that describe one value's fields, their types and their bounds — in the positions a
//! schema can sit (the root, each `properties` value, an `additionalProperties` schema, an `items`
//! schema), and refuses every other keyword. With no `$ref`, `$defs` or combinator, nothing can make
//! a value in a data position (`const`, `enum`, `default`, `examples`) into a schema, so the walk
//! never descends into one; with the depth, `enum` and `pattern` bounded, the work of building the
//! validator and of running it on content is bounded by the schema's own size (spec
//! `2026-10-27-a-shape-is-a-field-tree-design`, §1 and §2). [`ShapeSchemaUncompilable`]'s sentence
//! teaches the vocabulary.
//!
//! The draft is not left to `jsonschema`'s detection: `$schema` is absent, or it appears only at
//! the root and is exactly the bundled 2020-12 spelling (optionally ending in `#`). `$id` is also
//! root-only. A subschema therefore cannot register a custom meta-schema, and no spelling of
//! `$schema` can reach the retriever.
//!
//! Two layers still keep the compile from fetching, and both stay as defence in depth:
//!
//! 1. **The retriever.** `compile` builds with a retriever that refuses every URI, for `$ref` and
//!    `$schema` alike, so nothing is fetched and nothing remote is ever read. A retriever holds
//!    under cargo feature unification: another crate in the graph enabling a `jsonschema` resolver
//!    feature cannot widen what this compile does. No field tree can reach it — the walk has
//!    refused every `$ref` and every `$schema` it would be asked about — so it is unreachable
//!    through [`compile`] and has no bite of its own.
//! 2. **No resolvers compiled in.** Every workspace `jsonschema` dependency sets
//!    `default-features = false`, which drops the http and file resolvers and the TLS provider from
//!    the binary. A feature switch-off alone may not hold — unification can turn the features back
//!    on for a dependency that asks for them — which is why it is not the only layer (task
//!    `01a15661-8460-739d-9ed3-f1e8ccbf9b69`).
//!
//! The tripwire (`audit-shape-schema-compile.sh`) stops a new default-options compile being
//! written. Every server-side compile of a shape schema goes through here — commit, reconcile and
//! declare today, and any later reader of a shape — and the tripwire fails CI when another
//! `jsonschema` entry point appears under `crates/*/src`, so a new caller cannot reintroduce the
//! defaults. The `shape_schema_no_fetch` witnesses now show that the walk refuses before any fetch
//! could be made.
//!
//! **Each `pattern` regex is held to a size.** `jsonschema` compiles every distinct `pattern` when
//! a schema is built, and [`compile`] holds each regex to [`MAX_PATTERN_BYTES`] rather than the
//! engine's 10 MiB default; one past it makes the schema uncompilable, with the same fixed
//! sentence, for every caller (declare, commit, reconcile and the shape loader all compile here).

use jsonschema::{Retrieve, Uri, Validator};
use serde_json::Value;

/// The schema is not a field tree this server can compile.
///
/// Carries nothing: the compiler's own error is dropped on purpose. Under a fetching compiler it
/// describes a remote response, and under any compiler it is the declarer's material, not the
/// reader's. [`Display`](std::fmt::Display) is one fixed sentence that tells a declarer what to
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeSchemaUncompilable;

impl std::fmt::Display for ShapeSchemaUncompilable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the shape's JSON Schema cannot be compiled: a shape is a field tree — a JSON Schema \
             2020-12 document using only type, properties, required, additionalProperties (true, \
             false or a schema), items (one schema), enum (at most {MAX_ENUM_ENTRIES} values), \
             const, pattern (at most {MAX_PATTERN_SOURCE_BYTES} bytes, at most \
             {MAX_SHAPE_PATTERNS} in a shape, no lookaround or backreferences), minLength, \
             maxLength, minItems, maxItems, minimum, maximum, title, description, $comment, \
             default and examples, with $schema and $id only at the root, nested at most \
             {MAX_SHAPE_DEPTH} levels; $ref, $defs, allOf, anyOf, oneOf, not, if, format and \
             every other keyword are refused",
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

/// The deepest a field tree may nest: the root is depth 0, and each step into a `properties` value,
/// an `additionalProperties` schema or an `items` schema adds 1. This matches the ask walk's own
/// depth cap, so the ask can always see a field tree whole.
pub const MAX_SHAPE_DEPTH: usize = 16;

/// The most values an `enum` may list.
pub const MAX_ENUM_ENTRIES: usize = 256;

/// The most `pattern` keywords a shape may hold, counting every occurrence in a schema position.
/// A property *named* `pattern` is not a keyword and is not counted.
pub const MAX_SHAPE_PATTERNS: usize = 32;

/// The most bytes one `pattern`'s source may hold.
pub const MAX_PATTERN_SOURCE_BYTES: usize = 256;

/// The `$schema` spellings a field tree may name, at the root only: the one `referencing` bundles
/// (so naming it fetches nothing), with or without a trailing `#`.
const ACCEPTED_SCHEMA_URIS: [&str; 2] = [
    "https://json-schema.org/draft/2020-12/schema",
    "https://json-schema.org/draft/2020-12/schema#",
];

/// Whether the document is a field tree: only the allowed keywords, in schema positions, within the
/// depth, `enum` and `pattern` caps.
///
/// A schema position is the root, a `properties` value, an `additionalProperties` that is an object
/// or a boolean, or an `items` that is an object or a boolean. A boolean is a leaf. Values under
/// `const`, `enum`, `default` and `examples` are data and are never walked. Where the form of a
/// value is not fixed here (a `type` value, a `required` item) the walk leaves it to the build's
/// meta-validation. Every schema position is pushed once onto an explicit stack, so the walk is
/// linear in the document's size and never recurses on the Rust stack.
fn is_field_tree(schema: &Value) -> bool {
    let mut pending = vec![(schema, 0usize)];
    let mut patterns = 0usize;
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_SHAPE_DEPTH {
            return false;
        }
        let members = match node {
            Value::Bool(_) => continue,
            Value::Object(members) => members,
            _ => return false,
        };
        for (key, value) in members {
            match key.as_str() {
                "type" | "required" | "const" | "default" | "examples" | "minLength"
                | "maxLength" | "minItems" | "maxItems" | "minimum" | "maximum" | "title"
                | "description" | "$comment" => {}
                "properties" => match value {
                    Value::Object(fields) => {
                        pending.extend(fields.values().map(|field| (field, depth + 1)));
                    }
                    _ => return false,
                },
                "additionalProperties" | "items" => match value {
                    Value::Bool(_) | Value::Object(_) => pending.push((value, depth + 1)),
                    _ => return false,
                },
                "enum" => match value {
                    Value::Array(entries) if entries.len() <= MAX_ENUM_ENTRIES => {}
                    _ => return false,
                },
                "pattern" => match value {
                    Value::String(source) if source.len() <= MAX_PATTERN_SOURCE_BYTES => {
                        patterns += 1;
                        if patterns > MAX_SHAPE_PATTERNS {
                            return false;
                        }
                    }
                    _ => return false,
                },
                "$schema" => match value {
                    Value::String(uri)
                        if depth == 0 && ACCEPTED_SCHEMA_URIS.contains(&uri.as_str()) => {}
                    _ => return false,
                },
                "$id" if depth == 0 => {}
                _ => return false,
            }
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
/// [`ShapeSchemaUncompilable`] when the schema is not a field tree (a keyword outside the allowed
/// vocabulary, a `$schema` or `$id` below the root, a `$schema` other than the bundled 2020-12
/// spelling, or a depth, `enum`, `pattern` size or `pattern` count past its cap), is not valid for
/// 2020-12 where the walk leaves a form to meta-validation, or has a pattern that compiles past
/// [`MAX_PATTERN_BYTES`].
pub fn compile(schema: &Value) -> Result<Validator, ShapeSchemaUncompilable> {
    #[cfg(feature = "test-support")]
    test_support::record(schema);
    if !is_field_tree(schema) {
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
