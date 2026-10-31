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

/// The compiled form of a shape's JSON Schema, as [`compile`] returns it.
pub use jsonschema::Validator;

/// The schema is not a field tree this server can compile.
///
/// Carries what the walk refused first, when the walk refused it: one keyword, cap or form, and the
/// JSON-pointer path of the schema position it sits in (`keyword allOf at /properties/when`). That
/// is schema text the walk itself chose — a keyword name cut to [`MAX_REFUSED_KEYWORD_BYTES`], a
/// count, a cap, and the property names on the path to it — never a `const`, `enum`, `default` or
/// `examples` value, never an `x-*` value, and never the compiler's own error, which is still
/// dropped on purpose: under a fetching compiler it would describe a remote response.
///
/// Who may see it is the door's decision, not this type's. [`Display`](std::fmt::Display) is the
/// fixed sentence alone, the same for every schema, so a door that forgets to decide shows nothing
/// of the schema. A door whose reader can already read the shape — the declarer at declare; at
/// ask, `askable` and an enforcing commit, a reader the gated shape read admits — opts in with
/// [`detailed`](Self::detailed). Either form fits [`MAX_FAILURE_MESSAGE_BYTES`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShapeSchemaUncompilable {
    refused: Option<Box<str>>,
}

impl ShapeSchemaUncompilable {
    /// What the walk refused first, as `<what> at <path>`, already bounded; `None` when the walk
    /// admitted the schema and the build refused it (2020-12 meta-validation, or a pattern that
    /// compiles past [`MAX_PATTERN_BYTES`]), where the build's position is given instead.
    #[must_use]
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// The fixed sentence with what was refused named inside it, for a reader who can already read
    /// the shape: `the shape's JSON Schema cannot be compiled (keyword allOf at /properties/when):
    /// a shape is a field tree — …`. The fixed sentence when nothing was named. At most
    /// [`MAX_FAILURE_MESSAGE_BYTES`].
    #[must_use]
    pub fn detailed(&self) -> String {
        let explanation = self.to_string();
        let Some(refused) = self.refused() else {
            return explanation;
        };
        let rest = explanation
            .strip_prefix(UNCOMPILABLE_LEAD)
            .unwrap_or(&explanation);
        format!("{UNCOMPILABLE_LEAD} ({refused}){rest}")
    }

    fn naming(refused: String) -> Self {
        Self {
            refused: Some(cut_middle(refused, refused_room()).into_boxed_str()),
        }
    }
}

/// The words every form of the refusal opens with; [`ShapeSchemaUncompilable::detailed`] names
/// what was refused right after them.
const UNCOMPILABLE_LEAD: &str = "the shape's JSON Schema cannot be compiled";

/// The most bytes a refused keyword's own name is quoted to. An unknown key can be any length; the
/// longest 2020-12 keyword (`unevaluatedProperties`) is 21.
pub const MAX_REFUSED_KEYWORD_BYTES: usize = 64;

/// `text` if it fits in `limit` bytes; otherwise its head and its tail, on char boundaries, with
/// `…` between, in `limit` bytes. What was refused leads and where it sits ends a refusal, so a
/// long path loses its middle: `nesting past 16 levels at /properties/a/…/properties/a`.
fn cut_middle(text: String, limit: usize) -> String {
    if text.len() <= limit {
        return text;
    }
    let room = limit - '…'.len_utf8();
    let mut head = room / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - (room - head);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}…{}", &text[..head], &text[tail..])
}

/// The room [`ShapeSchemaUncompilable::detailed`] leaves for what was refused: the bound, less the
/// fixed sentence and the ` (` `)` around the insert.
fn refused_room() -> usize {
    MAX_FAILURE_MESSAGE_BYTES - ShapeSchemaUncompilable::default().to_string().len() - 3
}

impl std::fmt::Display for ShapeSchemaUncompilable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{UNCOMPILABLE_LEAD}: a shape is a field tree — a JSON Schema \
             2020-12 document using only type, properties, required, additionalProperties (true, \
             false or a schema), items (one schema), enum (at most {MAX_ENUM_ENTRIES} strings, or \
             {MAX_MIXED_ENUM_ENTRIES} values of any other kind), const, pattern (at most \
             {MAX_PATTERN_SOURCE_BYTES} bytes of source, compiling within {pattern_kib} KiB, at most \
             {MAX_SHAPE_PATTERNS} in a shape, no lookaround or backreferences), minLength, \
             maxLength, minItems, maxItems, minimum, maximum, title, description, $comment, \
             default, examples and format (a string, never enforced), with property names (in \
             properties and required) of at most {MAX_PROPERTY_NAME_BYTES} bytes, $schema and $id \
             only at the root ($schema, if present, spelled {schema_uri} with or without a \
             trailing #), nested at most {MAX_SHAPE_DEPTH} levels; keys beginning x- are carried \
             and ignored; $ref, $defs, allOf, anyOf, oneOf, not, if and every other keyword are \
             refused",
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
        Err(Box::new(ShapeSchemaUncompilable::default()))
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

/// The most entries an `enum` may list when any entry is not a string.
///
/// `jsonschema` compares such an enum against a value entry by entry, so every checked value costs
/// a scan of the whole enum. Measured: about 50 ms for 32 scalars checked over 900k integers,
/// about 184 ms for 32 structured entries over a 4.5 MB body at one level, and about 159 ms for a
/// 32-entry mixed enum at every one of the 16 levels. The 256 entries a
/// string-only enum may list would cost eight times that.
pub const MAX_MIXED_ENUM_ENTRIES: usize = 32;

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

/// One step from a schema position into one nested in it.
#[derive(Clone, Copy)]
enum Step<'a> {
    Property(&'a str),
    AdditionalProperties,
    Items,
}

/// Every schema position the walk has pushed: the position it was reached from (`None` for the
/// root) and the step into it. A refusal's path is rebuilt from here only when the walk refuses, so
/// the walk carries one index per position instead of a path.
struct Positions<'a>(Vec<(Option<usize>, Option<Step<'a>>)>);

impl<'a> Positions<'a> {
    fn root() -> Self {
        Positions(vec![(None, None)])
    }

    fn push(&mut self, from: usize, step: Step<'a>) -> usize {
        self.0.push((Some(from), Some(step)));
        self.0.len() - 1
    }

    /// Where position `at` sits, as `at <JSON pointer>` (`~` and `/` in a name escaped as RFC 6901
    /// says), or `at the root`.
    fn at(&self, mut at: usize) -> String {
        let mut steps = Vec::new();
        while let (Some(from), Some(step)) = self.0[at] {
            steps.push(step);
            at = from;
        }
        if steps.is_empty() {
            return "at the root".to_owned();
        }
        let mut pointer = "at ".to_owned();
        for step in steps.iter().rev() {
            match step {
                Step::Property(name) => {
                    pointer.push_str("/properties/");
                    pointer.push_str(&name.replace('~', "~0").replace('/', "~1"));
                }
                Step::AdditionalProperties => pointer.push_str("/additionalProperties"),
                Step::Items => pointer.push_str("/items"),
            }
        }
        pointer
    }
}

/// A refused keyword's name as a refusal quotes it: cut to [`MAX_REFUSED_KEYWORD_BYTES`].
fn keyword(key: &str) -> String {
    cut_to(key.to_owned(), MAX_REFUSED_KEYWORD_BYTES)
}

/// What makes the document not a field tree, the first thing the walk refuses — `None` when it is
/// one: only the allowed keywords, in schema positions, within the depth, `enum`, `pattern` and
/// property-name caps.
///
/// A schema position is the root, a `properties` value, an `additionalProperties` that is an object
/// or a boolean, or an `items` that is an object or a boolean. A boolean is a leaf. Values under
/// `const`, `enum`, `default` and `examples` are data and are never walked. `format` (a string) is
/// an annotation the compile never asserts — it turns on no format validation — and a key
/// beginning `x-` is carried and ignored: its value is never walked, whatever it holds. Where the
/// form of a value is not fixed here (a `type` value, a `required` item) the walk leaves it to the
/// build's meta-validation.
///
/// **First** is the walk's order: a position's own keywords in the map's key order, then the
/// positions nested in it, depth first, each before the next. Every schema position is pushed once
/// onto an explicit stack, so the walk is linear in the document's size and never recurses on the
/// Rust stack. The refusal is schema text only — a keyword, a count, a cap and the path — and never
/// quotes a value under a data keyword or an `x-` key.
fn field_tree_refusal(schema: &Value) -> Option<String> {
    let mut positions = Positions::root();
    let mut pending = vec![(schema, 0usize, 0usize)];
    let mut patterns = 0usize;
    while let Some((node, depth, at)) = pending.pop() {
        if depth > MAX_SHAPE_DEPTH {
            return Some(format!(
                "nesting past {MAX_SHAPE_DEPTH} levels {}",
                positions.at(at)
            ));
        }
        let members = match node {
            Value::Bool(_) => continue,
            Value::Object(members) => members,
            _ => {
                return Some(format!(
                    "a value that is not a schema (an object or a boolean) {}",
                    positions.at(at)
                ))
            }
        };
        let mut nested = Vec::new();
        for (key, value) in members {
            let refused = match key.as_str() {
                "required" => match value {
                    // A non-array `required` is left to the build's meta-validation, as before.
                    Value::Array(names)
                        if names.iter().any(|name| {
                            name.as_str()
                                .is_some_and(|name| name.len() > MAX_PROPERTY_NAME_BYTES)
                        }) =>
                    {
                        Some(format!(
                            "a name in required past {MAX_PROPERTY_NAME_BYTES} bytes"
                        ))
                    }
                    _ => None,
                },
                "type" | "const" | "default" | "examples" | "minLength" | "maxLength"
                | "minItems" | "maxItems" | "minimum" | "maximum" | "title" | "description"
                | "$comment" => None,
                // An annotation, never an assertion: the compile turns on no format validation.
                "format" => {
                    (!value.is_string()).then(|| "keyword format that is not a string".to_owned())
                }
                // Carried and ignored: the value is data, never walked.
                extension if extension.starts_with("x-") => None,
                "properties" => match value {
                    Value::Object(fields) => {
                        if fields
                            .keys()
                            .any(|name| name.len() > MAX_PROPERTY_NAME_BYTES)
                        {
                            Some(format!(
                                "a name in properties past {MAX_PROPERTY_NAME_BYTES} bytes"
                            ))
                        } else {
                            nested.extend(
                                fields
                                    .iter()
                                    .map(|(name, field)| (field, Step::Property(name))),
                            );
                            None
                        }
                    }
                    _ => Some("keyword properties that is not an object".to_owned()),
                },
                "additionalProperties" | "items" => match value {
                    Value::Bool(_) | Value::Object(_) => {
                        let step = if key == "items" {
                            Step::Items
                        } else {
                            Step::AdditionalProperties
                        };
                        nested.push((value, step));
                        None
                    }
                    _ => Some(format!("keyword {key} that is not one schema")),
                },
                "enum" => match value {
                    Value::Array(entries) => {
                        let cap = if entries.iter().all(Value::is_string) {
                            MAX_ENUM_ENTRIES
                        } else {
                            MAX_MIXED_ENUM_ENTRIES
                        };
                        (entries.len() > cap).then(|| {
                            format!("enum of {} entries, past the cap of {cap},", entries.len())
                        })
                    }
                    _ => Some("keyword enum that is not an array".to_owned()),
                },
                "pattern" => match value {
                    Value::String(source) if source.len() > MAX_PATTERN_SOURCE_BYTES => {
                        Some(format!(
                            "pattern of {} bytes, past the cap of {MAX_PATTERN_SOURCE_BYTES} \
                             bytes of source,",
                            source.len()
                        ))
                    }
                    Value::String(_) => {
                        patterns += 1;
                        (patterns > MAX_SHAPE_PATTERNS).then(|| {
                            format!("pattern past the cap of {MAX_SHAPE_PATTERNS} in a shape")
                        })
                    }
                    _ => Some("keyword pattern that is not a string".to_owned()),
                },
                "$schema" if depth > 0 => Some("keyword $schema below the root".to_owned()),
                "$schema" => match value {
                    Value::String(uri) if ACCEPTED_SCHEMA_URIS.contains(&uri.as_str()) => None,
                    _ => Some("keyword $schema naming a draft other than 2020-12".to_owned()),
                },
                "$id" if depth > 0 => Some("keyword $id below the root".to_owned()),
                "$id" => None,
                other => Some(format!("keyword {}", keyword(other))),
            };
            if let Some(refused) = refused {
                return Some(format!("{refused} {}", positions.at(at)));
            }
        }
        // Pushed last-first, so the nested positions are walked in the map's order.
        for (child, step) in nested.into_iter().rev() {
            let child_at = positions.push(at, step);
            pending.push((child, depth + 1, child_at));
        }
    }
    None
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

/// Compile a shape's JSON Schema into a validator without fetching anything, holding each pattern
/// to [`MAX_PATTERN_BYTES`].
///
/// # Errors
/// [`ShapeSchemaUncompilable`] when the schema is not a field tree (a keyword outside the allowed
/// vocabulary, a `$schema` or `$id` below the root, a `$schema` other than the bundled 2020-12
/// spelling (with or without a trailing `#`), a `properties` key or `required` entry past
/// [`MAX_PROPERTY_NAME_BYTES`], or a depth, `enum`, `pattern` size or `pattern` count past its cap), is not valid for
/// 2020-12 where the walk leaves a form to meta-validation, or has a pattern that compiles past
/// [`MAX_PATTERN_BYTES`].
pub fn compile(schema: &Value) -> Result<Validator, ShapeSchemaUncompilable> {
    #[cfg(feature = "test-support")]
    test_support::record(schema);
    if let Some(refused) = field_tree_refusal(schema) {
        return Err(ShapeSchemaUncompilable::naming(refused));
    }
    let patterns = jsonschema::PatternOptions::regex()
        .size_limit(MAX_PATTERN_BYTES)
        .dfa_size_limit(MAX_PATTERN_DFA_BYTES);
    jsonschema::options()
        .with_retriever(RefuseEveryUri)
        .with_pattern_options(patterns)
        .build(schema)
        .map_err(|error| ShapeSchemaUncompilable::naming(build_refusal(&error)))
}

/// What the build refused in a schema the walk admitted, without the build's own words: which of
/// its two refusals it is, and the position in the schema it names, which is schema text — a JSON
/// pointer to a keyword. The build's message is dropped, as it always was.
///
/// A `pattern` the regex engine will not build — past [`MAX_PATTERN_BYTES`] compiled, or not a
/// regex at all — is named at the schema position that holds it; any other refusal is 2020-12
/// meta-validation, named at the keyword whose value it refused.
fn build_refusal(error: &jsonschema::ValidationError<'_>) -> String {
    let at = |pointer: String| {
        if pointer.is_empty() {
            "at the root".to_owned()
        } else {
            format!("at {pointer}")
        }
    };
    match error.kind() {
        jsonschema::error::ValidationErrorKind::Format { format } if format == "regex" => {
            let pointer = error.schema_path().to_string();
            let position = pointer.strip_suffix("/pattern").unwrap_or(&pointer);
            format!(
                "pattern past the cap of {} KiB compiled, or not a regex, {}",
                MAX_PATTERN_BYTES / 1024,
                at(position.to_owned())
            )
        }
        _ => format!(
            "a value that is not valid 2020-12 {}",
            at(error.instance_path().to_string())
        ),
    }
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
    cut_to(text, MAX_FAILURE_MESSAGE_BYTES)
}

/// `text` if it fits in `limit` bytes; otherwise its longest prefix on a char boundary within
/// `limit - 3` bytes, followed by `…` (3 bytes).
fn cut_to(text: String, limit: usize) -> String {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit - '…'.len_utf8();
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
