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
//! drafts names a custom meta-schema, which is never guessed at: [`compile`] refuses it wherever it
//! appears (the root or any embedded subschema), including when the schema registers that very
//! URI as one of its own resources, a case `jsonschema` itself would otherwise compile under a
//! default draft.

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

/// Whether every `$schema` in the document, at the root and in each embedded subschema, names one
/// of the five standard drafts.
///
/// `jsonschema` refuses an unknown `$schema` only when the URI is not among the document's own
/// resources, so `{"$id": "urn:x", "$schema": "urn:x"}` (or an embedded subschema carrying that
/// `$id`) registers itself as its meta-schema and compiles under a guessed draft. The walk uses
/// the draft's own subresource iterator, so it visits exactly the subschemas `jsonschema` treats as
/// resources, and an explicit stack keeps it off the call stack. It is linear in the schema's size,
/// which the request body already bounds.
fn names_only_standard_drafts(schema: &Value) -> bool {
    let root = Draft::default().detect(schema);
    if root == Draft::Unknown {
        return false;
    }
    let mut pending = vec![schema];
    while let Some(node) = pending.pop() {
        if Draft::default().detect(node) == Draft::Unknown {
            return false;
        }
        pending.extend(root.subresources_of(node));
    }
    true
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
