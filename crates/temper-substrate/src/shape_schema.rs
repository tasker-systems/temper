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
//! drafts names a custom meta-schema, which is never guessed at: [`compile`] refuses it at the root
//! and in every position the compiler compiles as a schema (the list is on
//! `names_only_standard_drafts`): the subschema keywords of every draft, and the target of any
//! same-document `$ref` pointer, including when the schema registers that very URI as one of its
//! own resources, a case `jsonschema` itself would otherwise compile under a default draft. A
//! `$schema` key inside `const`, `enum`, `default` or `examples` is data and is not read unless a
//! pointer reference makes it a schema position.

use std::collections::HashSet;

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

/// The keywords whose string value `jsonschema` resolves as a reference to a schema.
const REFERENCE_KEYWORDS: [&str; 3] = ["$ref", "$dynamicRef", "$recursiveRef"];

/// The most embedded resources (subschemas with their own `$id`) a shape may carry. Pointers are
/// resolved against the root and each such resource, so this bounds that work; a real shape
/// carries none or a handful.
const MAX_EMBEDDED_RESOURCES: usize = 64;

/// Whether a subschema declares its own base URI, so a `#/...` reference inside it resolves
/// against it and not the document root. Both spellings are honoured whatever the draft: treating
/// a stray `id` as a resource only adds a base to try, never removes one.
fn declares_resource(node: &Value) -> bool {
    ["$id", "id"].iter().any(|key| {
        node.get(*key)
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && !id.starts_with('#'))
    })
}

/// Decode `%XX` escapes (a JSON-pointer URI fragment is percent-encoded). `None` on a malformed
/// escape or invalid UTF-8, in which case `jsonschema` cannot resolve it either.
fn percent_decode(fragment: &str) -> Option<String> {
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            decoded.push(u8::from_str_radix(fragment.get(at + 1..at + 3)?, 16).ok()?);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// The node a same-document JSON-pointer reference (`#/a/b`) names inside `base`. References to
/// other documents, `#` alone and `$anchor`-style names (`#name`) yield `None`: the first are
/// refused by the retriever, and an anchor can only name a subschema position the walk already
/// visits, because `referencing` registers anchors only for the resources it finds through the
/// draft's subschema keywords.
fn pointer_target<'a>(base: &'a Value, reference: &str) -> Option<&'a Value> {
    let decoded = percent_decode(reference.strip_prefix('#')?)?;
    if !decoded.starts_with('/') {
        return None;
    }
    base.pointer(&decoded)
}

/// Whether every `$schema` in the document, at the root and in every position the compiler
/// compiles as a schema, names one of the five standard drafts.
///
/// `jsonschema` refuses an unknown `$schema` only when the URI is not among the document's own
/// resources, so `{"$id": "urn:x", "$schema": "urn:x"}` (or an embedded subschema carrying that
/// `$id`) registers itself as its meta-schema and compiles under a guessed draft.
///
/// **Which positions are walked.**
/// - The union of every draft's subschema keywords, through `Draft::Draft4.subresources_of`:
///   `properties`, `patternProperties`, `additionalProperties`, `additionalItems`, `items` (a
///   schema or an array of schemas), `prefixItems`, `contains`, `contentSchema`, `propertyNames`,
///   `not`, `if`/`then`/`else`, `allOf`/`anyOf`/`oneOf`, `$defs`, `definitions`,
///   `dependentSchemas`, `dependencies` (the schema-valued entries) and
///   `unevaluatedItems`/`unevaluatedProperties`. The union rather than the root draft's list,
///   because `jsonschema` compiles `dependencies` and `additionalItems` under every draft, and a
///   subschema carrying its own `$schema` is compiled under that draft's keywords. The
///   `referencing` 0.45 Draft 4 iterator is that union: it is the only one listing all of them.
/// - The target of every same-document pointer reference (`$ref`, `$dynamicRef`, `$recursiveRef`
///   whose value is `#/...`), wherever it points, because `jsonschema` resolves any pointer and
///   compiles what it finds as a schema, `x-custom` and `const` included. A pointer is resolved
///   against the root and against each embedded resource (a subschema with its own `$id`), since a
///   reference resolves within its enclosing resource; percent-escapes and `~0`/`~1` are decoded
///   first. Each node is visited once, so reference cycles terminate, and more than
///   [`MAX_EMBEDDED_RESOURCES`] embedded resources is refused.
///
/// **Which positions are not.** Values of `const`, `enum`, `default`, `examples` and unknown
/// keywords are data, not schemas, so a `$schema` key inside one is not read and a schema that
/// merely mentions one still compiles — unless a pointer reference targets it, which makes it a
/// schema position and walks it. `$anchor` names (`#name`) are not followed: `referencing`
/// registers anchors only for the subschema positions listed above. References to other documents
/// are not followed; the retriever refuses them.
fn names_only_standard_drafts(schema: &Value) -> bool {
    let draft = Draft::default();
    let mut seen: HashSet<*const Value> = HashSet::new();
    let mut pending = vec![schema];
    let mut bases = vec![schema];
    let mut references: HashSet<&str> = HashSet::new();
    let mut resolved: HashSet<(*const Value, &str)> = HashSet::new();
    loop {
        while let Some(node) = pending.pop() {
            if !seen.insert(node) {
                continue;
            }
            if draft.detect(node) == Draft::Unknown {
                return false;
            }
            if !std::ptr::eq(node, schema) && declares_resource(node) {
                bases.push(node);
                if bases.len() > MAX_EMBEDDED_RESOURCES {
                    return false;
                }
            }
            references.extend(
                REFERENCE_KEYWORDS
                    .iter()
                    .filter_map(|keyword| node.get(*keyword).and_then(Value::as_str)),
            );
            pending.extend(Draft::Draft4.subresources_of(node));
        }
        // Resolve each (base, reference) pair once; a new node can add references and bases, so
        // go round again until a pass finds nothing new to walk.
        for base in &bases {
            for reference in &references {
                if resolved.insert((std::ptr::from_ref(*base), *reference)) {
                    pending.extend(pointer_target(base, reference));
                }
            }
        }
        if pending.is_empty() {
            return true;
        }
    }
}

/// Compile a shape's JSON Schema into a validator without fetching anything.
///
/// # Errors
/// [`ShapeSchemaUncompilable`] when the schema is not valid for its draft, names a `$schema`
/// outside the standard drafts (even one the schema itself defines), or has a `$ref` that leaves
/// the document.
pub fn compile(schema: &Value) -> Result<Validator, ShapeSchemaUncompilable> {
    if !names_only_standard_drafts(schema) {
        return Err(ShapeSchemaUncompilable);
    }
    jsonschema::options()
        .with_retriever(RefuseEveryUri)
        .build(schema)
        .map_err(|_| ShapeSchemaUncompilable)
}
