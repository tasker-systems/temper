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
//! validator and of running it on content is bounded by the schema's caps times the content's size (spec
//! `2026-10-27-a-shape-is-a-field-tree-design`, §1 and §2). [`ShapeSchemaUncompilable`]'s sentence
//! teaches the vocabulary.
//!
//! **A `const` and every `enum` member is a scalar.** [`is_shape_value`] holds each to `null`, a
//! boolean, a number, or a string of at most [`MAX_SHAPE_VALUE_STRING_BYTES`] JSON-escaped bytes;
//! an object, an array or a longer string makes the schema uncompilable. `default` and `examples`
//! stay free-form data, which the compile never reads.
//!
//! **A schema's parsed size has a budget.** Before anything else reads the document, [`compile`]
//! holds it to [`MAX_SHAPE_NODES`] JSON values at every depth, at most [`MAX_SHAPE_OBJECTS`] of
//! them objects, `default` and `examples` included ([`within_node_budget`]). One weight model,
//! [`parsed_weight`], sizes the two caps, and an assertion holds them to
//! [`MAX_SHAPE_PARSED_WEIGHT`].
//!
//! The draft is not left to `jsonschema`'s detection: `$schema` is absent, or it appears only at
//! the root and is the bundled 2020-12 spelling, with or without a trailing `#`. `$id` is also
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

use jsonschema::{Retrieve, Uri};
use serde_json::Value;
use temper_core::types::artifact_ask::is_scalar;
use temper_core::types::query::filter::{json_string_bytes, MAX_FILTER_STRING_BYTES};

/// The compiled form of a shape's JSON Schema, as [`compile`] returns it.
pub use jsonschema::Validator;

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
             false or a schema), items (one schema), enum (at most {MAX_ENUM_ENTRIES} strings, or \
             {MAX_MIXED_ENUM_ENTRIES} values of any other kind), const and enum members that are \
             strings (at most {MAX_SHAPE_VALUE_STRING_BYTES} bytes, JSON-escaped), numbers, \
             booleans or null, pattern (at most {MAX_PATTERN_SOURCE_BYTES} bytes of source, \
             compiling within {pattern_kib} KiB, at most {MAX_SHAPE_PATTERNS} in a shape, no \
             lookaround or backreferences), minLength, \
             maxLength, minItems, maxItems, minimum, maximum, title, description, $comment, \
             default and examples, with property names (in properties and required) of at most \
             {MAX_PROPERTY_NAME_BYTES} bytes, $schema and $id only at the root ($schema, if \
             present, spelled {schema_uri} with or without a trailing #), nested at most \
             {MAX_SHAPE_DEPTH} levels, holding at most {MAX_SHAPE_NODES} JSON values in all, at \
             most {MAX_SHAPE_OBJECTS} of them objects (a budget that can refuse a schema every \
             other cap allows); $ref, $defs, allOf, anyOf, oneOf, not, if, format and every \
             other keyword are refused",
            pattern_kib = MAX_PATTERN_BYTES / 1024,
            schema_uri = ACCEPTED_SCHEMA_URIS[0],
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

const _: () = assert!(
    MAX_SHAPE_DEPTH == temper_core::types::artifact_ask::walk::MAX_WALK_DEPTH,
    "a field tree's depth cap is the ask walk's, so the ask can see a field tree whole"
);

/// The most entries an `enum` may list when every entry is a string — the cap for string-only
/// enums. `jsonschema` hashes a string-only enum above 10 entries, so a check against one does not
/// scan the entries. Any other enum is held to [`MAX_MIXED_ENUM_ENTRIES`].
pub const MAX_ENUM_ENTRIES: usize = 256;

/// The most entries an `enum` may list when any entry is not a string: every other enum, a
/// numbers-only enum included.
///
/// `jsonschema` compares such an enum against a value entry by entry, so every checked value costs
/// a scan of the whole enum. Measured: about 50 ms for 32 scalars checked over 900k integers. The
/// 256 entries a string-only enum may list would cost eight times that.
pub const MAX_MIXED_ENUM_ENTRIES: usize = 32;

/// The longest a string may be as a `const` value or an `enum` member, in JSON-escaped bytes, as
/// the ask surface counts a string it sends. It is a literal, held equal to
/// [`MAX_FILTER_STRING_BYTES`] by the assert below, so a change to either one fails the build
/// rather than letting the two drift. [`MAX_PROPERTY_NAME_BYTES`] is also 256, but measures raw
/// bytes.
pub const MAX_SHAPE_VALUE_STRING_BYTES: usize = 256;

const _: () = assert!(
    MAX_SHAPE_VALUE_STRING_BYTES == MAX_FILTER_STRING_BYTES,
    "a shape's string value cap is the ask surface's string cap, so the ask can name every value"
);

/// Whether a value may stand as a `const` or an `enum` member: a scalar (`null`, a boolean, a
/// number or a string) and, for a string, at most [`MAX_SHAPE_VALUE_STRING_BYTES`] bytes
/// JSON-escaped. The raw length is read first, which is free, and escaping only lengthens a string,
/// so one past the cap raw is never serialized to be refused.
pub fn is_shape_value(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            text.len() <= MAX_SHAPE_VALUE_STRING_BYTES
                && json_string_bytes(text) <= MAX_SHAPE_VALUE_STRING_BYTES
        }
        other => is_scalar(other),
    }
}

/// The most `pattern` keywords a shape may hold, counting every occurrence in a schema position.
/// A property *named* `pattern` is not a keyword and is not counted.
pub const MAX_SHAPE_PATTERNS: usize = 32;

/// The most bytes one `pattern`'s source may hold.
pub const MAX_PATTERN_SOURCE_BYTES: usize = 256;

/// The most bytes a `properties` key or a `required` entry may hold.
///
/// `serde_json`'s `preserve_order` map hashes the whole key on every lookup, so a check's cost
/// grows with the key's length; at 256 bytes it measured about 15 ms over 4.5 MB of two-key
/// objects. 256 matches the ask walk's path-segment cap, so the ask never advertises a longer
/// name than a shape can hold.
pub const MAX_PROPERTY_NAME_BYTES: usize = 256;

/// The `$schema` spellings a field tree may name, at the root only: the one `referencing` bundles
/// (so naming it fetches nothing), with or without a trailing `#`.
const ACCEPTED_SCHEMA_URIS: [&str; 2] = [
    "https://json-schema.org/draft/2020-12/schema",
    "https://json-schema.org/draft/2020-12/schema#",
];

/// The most JSON values a shape's schema may hold, counted at every depth: the root, each member
/// of each object, each array element, containers and data positions alike, `default` and
/// `examples` included. That is what Postgres's `strict $.**` counts. [`within_node_budget`] holds
/// a schema to it and to [`MAX_SHAPE_OBJECTS`].
///
/// # 12,288, provisional
///
/// `MAX_SCHEMA_BYTES` already caps a schema's text, but what parsing it holds runs from about 3.9×
/// the text's bytes (a described field tree) to about 98× (tiny objects), both measured. A byte
/// cap low enough to bound memory would refuse ordinary schemas of long descriptions, so the
/// budget counts values instead.
///
/// The two caps are the largest whose weight under [`parsed_weight`], at a text of
/// `MAX_SCHEMA_BYTES`, stays within [`MAX_SHAPE_PARSED_WEIGHT`]; an assertion beside
/// [`parsed_weight`] holds them there. Every admission profile fits inside them:
///
/// | Admission profile | Values | Objects |
/// |---|---|---|
/// | Every live shape (a production read) | ≤ ~750 | ≤ ~500 |
/// | 1,024 fields `{"type":"string","description":…}` | 3,075 | 1,026 |
/// | 1,024 array fields `{"type":"array","items":{"type":"string"}}` | 4,099 | 2,050 |
/// | 1,024 fields `{"type":["string","null"],"title":…,"description":…,"examples":[…]}` | 8,195 | 1,026 |
/// | 200 fields with 32-member enums | 7,003 | 202 |
/// | 32 fields with 256-string enums ([`MAX_ENUM_ENTRIES`]) | 8,291 | 34 |
///
/// The budget can refuse what every other cap allows: 48 enums of [`MAX_ENUM_ENTRIES`] strings
/// each fit the per-keyword caps, but not this one.
pub const MAX_SHAPE_NODES: usize = 12_288;

/// The most of a shape schema's JSON values that may be objects. Each object holds at least one
/// map node of [`MAP_NODE_BYTES`] in the deployed `Value`, which is what makes tiny objects the
/// heaviest thing per byte a schema can hold.
///
/// # 3,072, provisional
///
/// Sized with [`MAX_SHAPE_NODES`], by the derivation and admission profiles in its doc.
pub const MAX_SHAPE_OBJECTS: usize = 3_072;

/// The weight model's charge for one JSON value: the per-value charge the shape loader weighs a
/// vocabulary with. A fixed constant of the deployed build, not `size_of`: under `preserve_order`,
/// which a `--workspace` build unifies in, `Value` is 72 bytes and an object is an `IndexMap` sized
/// to its entries. Fixed constants keep the model, and the assertion on it, meaning the same thing
/// in every build.
pub const VALUE_BYTES: usize = 64;

/// The weight model's charge for one node of the deployed `serde_json::Map`, a B-tree. Fixed for
/// the reason [`VALUE_BYTES`] is.
pub const MAP_NODE_BYTES: usize = 632;

/// The most a shape's parsed schema may weigh under [`parsed_weight`], with both caps full and a
/// text of `MAX_SCHEMA_BYTES`. The assertion below holds [`MAX_SHAPE_NODES`] and
/// [`MAX_SHAPE_OBJECTS`] to it, so raising either cap fails the build unless this target moves
/// with it.
///
/// # 4.5 MiB, provisional
///
/// At the provisional caps the model weighs 632 × 3,072 + 190 × 12,288 + 262,144 = 4,538,368
/// bytes, about 4.33 MiB: by the model, the most one read of a stored shape builds. The model errs
/// high: it charges each object's map node a second time through its members, and a schema it
/// weighs at about 3 MiB measured 1.37 MB of real heap.
pub const MAX_SHAPE_PARSED_WEIGHT: usize = 9 << 19;

/// What `nodes` JSON values, `objects` of them objects, parsed from `text_bytes` bytes of text,
/// weigh in the deployed `Value`: a map node of [`MAP_NODE_BYTES`] per object, [`VALUE_BYTES`] per
/// value plus a fifth of a map node per value (a non-root B-tree node holds at least five keys;
/// the fifth is integer division, 126 bytes), and the text itself.
pub const fn parsed_weight(nodes: usize, objects: usize, text_bytes: usize) -> usize {
    objects * MAP_NODE_BYTES + nodes * (VALUE_BYTES + MAP_NODE_BYTES / 5) + text_bytes
}

const _: () = assert!(
    parsed_weight(
        MAX_SHAPE_NODES,
        MAX_SHAPE_OBJECTS,
        temper_core::types::artifact_ask::walk::MAX_SCHEMA_BYTES,
    ) <= MAX_SHAPE_PARSED_WEIGHT,
    "a shape at both caps and the byte cap weighs no more than its target: raise the target, \
     with its reason, or lower a cap"
);

/// What [`count_up_to`] counted: JSON values and, of them, objects, and whether it stopped past a
/// cap. When `over` is false the counts are the whole document's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeCount {
    /// JSON values counted, the root included.
    pub nodes: usize,
    /// Of those values, the objects.
    pub objects: usize,
    /// Whether counting stopped past `max_nodes` or `max_objects`.
    pub over: bool,
}

impl NodeCount {
    /// Count one container's `members`, then, only if both counts are still within their caps,
    /// push the members that are themselves containers onto `pending`.
    fn count_members<'a>(
        &mut self,
        members: impl ExactSizeIterator<Item = &'a Value> + Clone,
        pending: &mut Vec<&'a Value>,
        max_nodes: usize,
        max_objects: usize,
    ) {
        self.nodes += members.len();
        self.objects += members.clone().filter(|member| member.is_object()).count();
        self.over = self.nodes > max_nodes || self.objects > max_objects;
        if !self.over {
            pending.extend(members.filter(|member| !is_scalar(member)));
        }
    }
}

/// Count `schema`'s JSON values and objects, stopping at the first container that takes either
/// count past its cap.
///
/// A container's members are counted when it is reached, before any of them is pushed, so the
/// count never passes a cap by more than one container's width, and a wide array past the cap is
/// never pushed. The walk keeps an explicit stack and never recurses on the Rust stack.
pub fn count_up_to(schema: &Value, max_nodes: usize, max_objects: usize) -> NodeCount {
    let mut count = NodeCount {
        nodes: 0,
        objects: 0,
        over: false,
    };
    let mut pending = Vec::new();
    count.count_members(
        std::slice::from_ref(schema).iter(),
        &mut pending,
        max_nodes,
        max_objects,
    );
    while let Some(node) = pending.pop() {
        match node {
            Value::Object(members) => {
                count.count_members(members.values(), &mut pending, max_nodes, max_objects);
            }
            Value::Array(items) => {
                count.count_members(items.iter(), &mut pending, max_nodes, max_objects);
            }
            _ => {}
        }
        if count.over {
            break;
        }
    }
    count
}

/// Whether `schema` holds at most [`MAX_SHAPE_NODES`] JSON values, at most [`MAX_SHAPE_OBJECTS`]
/// of them objects. It reads no more of a schema past either cap than [`count_up_to`] does.
pub fn within_node_budget(schema: &Value) -> bool {
    !count_up_to(schema, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS).over
}

/// Whether the document is a field tree: only the allowed keywords, in schema positions, within the
/// depth, `enum` and `pattern` caps, and with every `const` and `enum` member passing
/// [`is_shape_value`].
///
/// A schema position is the root, a `properties` value, an `additionalProperties` that is an object
/// or a boolean, or an `items` that is an object or a boolean. A boolean is a leaf. Values under
/// `const`, `enum`, `default` and `examples` are data and are never walked; of them a `const` and
/// each `enum` member must be a shape value, and `default` and `examples` are free. Where the form
/// of a value is not fixed here (a `type` value, a `required` item) the walk leaves it to the
/// build's meta-validation. Every schema position is pushed once onto an explicit stack, so the walk is
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
                "required" => {
                    // A non-array `required` is left to the build's meta-validation, as before.
                    if let Value::Array(names) = value {
                        let too_long = |name: &Value| {
                            name.as_str()
                                .is_some_and(|name| name.len() > MAX_PROPERTY_NAME_BYTES)
                        };
                        if names.iter().any(too_long) {
                            return false;
                        }
                    }
                }
                "type" | "default" | "examples" | "minLength" | "maxLength" | "minItems"
                | "maxItems" | "minimum" | "maximum" | "title" | "description" | "$comment" => {}
                "const" => {
                    if !is_shape_value(value) {
                        return false;
                    }
                }
                "properties" => match value {
                    Value::Object(fields) => {
                        if fields
                            .keys()
                            .any(|name| name.len() > MAX_PROPERTY_NAME_BYTES)
                        {
                            return false;
                        }
                        pending.extend(fields.values().map(|field| (field, depth + 1)));
                    }
                    _ => return false,
                },
                "additionalProperties" | "items" => match value {
                    Value::Bool(_) | Value::Object(_) => pending.push((value, depth + 1)),
                    _ => return false,
                },
                "enum" => match value {
                    Value::Array(entries) => {
                        let cap = if entries.iter().all(Value::is_string) {
                            MAX_ENUM_ENTRIES
                        } else {
                            MAX_MIXED_ENUM_ENTRIES
                        };
                        if entries.len() > cap || !entries.iter().all(is_shape_value) {
                            return false;
                        }
                    }
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

/// The most bytes one `pattern` may compile to — the `regex` engine's per-regex `size_limit`, which
/// counted repetitions expand into.
///
/// # 10 KiB
///
/// `jsonschema` compiles every distinct pattern eagerly when a schema is built, and the engine's
/// own default allows 10 MiB a regex. The bound also caps what running one costs: once the lazy DFA
/// gives up, match cost is linear in the compiled NFA's size. At 10 KiB the benchmark's worst case
/// measured about 2.2 s over a 4.5 MB string locally, while the patterns a person writes
/// for a field fit: a UUID is 2.9 KB, an ISO date-time 1.4 KB, `^\w{1,20}$` 2.9 KB,
/// `^[\w.-]{1,59}$` 8.7 KB, `^[A-Za-z0-9][A-Za-z0-9._-]{0,57}$` 8.6 KB. A Unicode class such as
/// `\p{L}` (about 43 KB on its own) does not fit. A `pattern` past it fails to compile, so its
/// schema is [`ShapeSchemaUncompilable`], with the one fixed sentence, which names this size.
pub const MAX_PATTERN_BYTES: usize = 10 << 10;

/// The most bytes the engine's lazy DFA may use as its search cache for one pattern — what
/// `PatternOptions::dfa_size_limit` reaches on the `regex` engine (`hybrid_cache_capacity`).
///
/// # 2 MiB
///
/// The `regex` crate's own default (`2 * (1 << 20)`), pinned. The lazy DFA builds states as input is
/// read and, when the cache is full, clears it and carries on, so the cache bounds memory and
/// never refuses a pattern. On this engine the full-DFA limit can no longer be set; pinning the
/// cache keeps an upstream change of default from moving what searching with one pattern may hold.
pub const MAX_PATTERN_DFA_BYTES: usize = 2 << 20;

/// Compile a shape's JSON Schema into a validator without fetching anything, holding the document
/// to the node budget before anything else reads it, and each pattern to [`MAX_PATTERN_BYTES`].
///
/// # Errors
/// [`ShapeSchemaUncompilable`] when the schema holds more than [`MAX_SHAPE_NODES`] JSON values or
/// more than [`MAX_SHAPE_OBJECTS`] objects ([`within_node_budget`]), is not a field tree (a
/// keyword outside the allowed vocabulary, a `$schema` or `$id` below the root, a `$schema` other
/// than the bundled 2020-12 spelling (with or without a trailing `#`), a `properties` key or
/// `required` entry past [`MAX_PROPERTY_NAME_BYTES`], a `const` or `enum` member that is not an
/// [`is_shape_value`], or a depth, `enum`, `pattern` size or `pattern` count past its cap), is not
/// valid for 2020-12 where the walk leaves a form to meta-validation, or has a pattern that
/// compiles past [`MAX_PATTERN_BYTES`].
pub fn compile(schema: &Value) -> Result<Validator, ShapeSchemaUncompilable> {
    #[cfg(feature = "test-support")]
    test_support::record(schema);
    if !within_node_budget(schema) || !is_field_tree(schema) {
        return Err(ShapeSchemaUncompilable);
    }
    let patterns = jsonschema::PatternOptions::regex()
        .size_limit(MAX_PATTERN_BYTES)
        .dfa_size_limit(MAX_PATTERN_DFA_BYTES);
    jsonschema::options()
        .with_retriever(RefuseEveryUri)
        .with_pattern_options(patterns)
        .build(schema)
        .map_err(|_| ShapeSchemaUncompilable)
}

/// The most bytes of a violation's path, or of its message, a verdict or a refusal may carry.
///
/// A violation's message can quote the content that broke the rule (a `const` or `enum` mismatch
/// echoes the instance), so an unbounded message would copy an author's content into a refusal, a
/// stored verdict detail and a log. 1 KiB names any rule a field tree can state, with room to spare.
pub const MAX_FAILURE_MESSAGE_BYTES: usize = 1024;

/// The first way `content` fails `validator`, as `(path, message)`, or `None` when it satisfies it.
///
/// The verdict and the detail come from one `validate` pass, which stops at the first violation. A
/// field tree has no combinator, so nothing is lost by not asking `is_valid` first; running
/// `is_valid` and then `validate` made a failing check two passes over the content and doubled the
/// worst case. It does not collect every violation, so the work and the text are bounded however
/// many places the content breaks the schema. Each of the two strings is cut to
/// [`MAX_FAILURE_MESSAGE_BYTES`].
pub fn first_failure(validator: &Validator, content: &Value) -> Option<(String, String)> {
    let error = validator.validate(content).err()?;
    Some((
        cut(error.instance_path().to_string()),
        cut(error.to_string()),
    ))
}

/// The stored `detail` of a verdict whose content fails its shape: `[{path, message}]`, the one
/// violation [`first_failure`] names. Commit and reconcile both write it, so the two cannot drift.
#[must_use]
pub fn failure_detail(path: &str, message: &str) -> Value {
    serde_json::json!([{ "path": path, "message": message }])
}

/// `text` if it fits in [`MAX_FAILURE_MESSAGE_BYTES`]; otherwise its longest prefix on a char
/// boundary within `MAX_FAILURE_MESSAGE_BYTES - 3` bytes, followed by `…` (3 bytes).
fn cut(text: String) -> String {
    if text.len() <= MAX_FAILURE_MESSAGE_BYTES {
        return text;
    }
    let mut end = MAX_FAILURE_MESSAGE_BYTES - '…'.len_utf8();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut cut = text[..end].to_owned();
    cut.push('…');
    cut
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

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{count_up_to, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS};

    /// Counting stops at the first container that takes it past a cap, so on a schema ten times
    /// over the budget it counts no more than the cap plus that one container's width. The first
    /// fixture is a single array ten times the cap wide. The second spreads as many values over
    /// hundreds of narrow arrays, so a count that walked the whole document before checking passes
    /// the bound many times over.
    #[test]
    fn counting_stops_at_the_first_container_past_the_cap() {
        let wide = 10 * MAX_SHAPE_NODES;
        let one_array = json!({ "examples": Value::Array(vec![json!(0); wide]) });
        let count = count_up_to(&one_array, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS);
        assert!(count.over, "{count:?}");
        assert!(count.nodes <= MAX_SHAPE_NODES + wide, "{count:?}");

        let widest = 352;
        assert!(
            widest * widest >= 10 * MAX_SHAPE_NODES,
            "the narrow arrays hold ten times the cap"
        );
        let narrow = Value::Array(vec![json!(0); widest]);
        let many_arrays = json!({ "examples": Value::Array(vec![narrow; widest]) });
        let whole = count_up_to(&many_arrays, usize::MAX, usize::MAX);
        assert!(
            whole.nodes > MAX_SHAPE_NODES + widest,
            "a count of the whole document passes the bound: {whole:?}"
        );
        let count = count_up_to(&many_arrays, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS);
        assert!(count.over, "{count:?}");
        assert!(count.nodes <= MAX_SHAPE_NODES + widest, "{count:?}");
    }
}
