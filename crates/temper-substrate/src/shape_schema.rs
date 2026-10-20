//! The one place a data-artifact shape's JSON Schema is compiled.
//!
//! A shape is declared by any author of its home, so its schema is untrusted input. `jsonschema`
//! under its default options resolves a `$ref` (or an off-list `$schema`) by fetching it: an
//! `http(s)` URI over the network, a `file:` URI from the server's disk. A caller who can declare
//! a shape could therefore make the server connect to an internal address, or read a local file,
//! on every compile — and the compiler's error text can echo what came back.
//!
//! [`compile`] closes that. It builds with a retriever that refuses every URI, for `$ref` and
//! `$schema` alike, so nothing is fetched and nothing remote is ever read. A retriever, rather
//! than switching the crate's `resolve-*` features off, because a retriever holds under cargo
//! feature unification and a feature switch-off may not (task `01a15661-8460-739d-9ed3-f1e8ccbf9b69`).
//!
//! Every server-side compile of a shape schema goes through here — commit, reconcile and declare
//! today, and any later reader of a shape. `audit-shape-schema-compile.sh` fails CI when another
//! `jsonschema` entry point appears under `crates/*/src`, so a new caller cannot reintroduce the
//! defaults.
//!
//! The draft is not chosen here: `jsonschema` selects it from the schema's own `$schema`, which is
//! the selection a validator built any other way would make. A `$schema` outside the five
//! standard drafts is a custom meta-schema that would have to be fetched, so it is uncompilable.

use jsonschema::{Retrieve, Uri, Validator};
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

/// Compile a shape's JSON Schema into a validator without fetching anything.
///
/// # Errors
/// [`ShapeSchemaUncompilable`] when the schema is not valid for its draft, names a `$schema`
/// outside the standard drafts, or has a `$ref` that leaves the document.
pub fn compile(schema: &Value) -> Result<Validator, ShapeSchemaUncompilable> {
    jsonschema::options()
        .with_retriever(RefuseEveryUri)
        .build(schema)
        .map_err(|_| ShapeSchemaUncompilable)
}
