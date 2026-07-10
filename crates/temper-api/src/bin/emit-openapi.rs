//! emit-openapi — print the OpenAPI contract as pretty JSON to stdout.
//!
//! The spec is a product of the router (`openapi_spec()`): pure, no `AppState`,
//! no database, no tokio runtime. `cargo make openapi` pipes this to the
//! checked-in `openapi.json` at the repo root; `cargo make openapi-check`
//! diffs a fresh emission against it. Exits non-zero if serialization fails.

fn main() {
    let spec = temper_api::openapi_spec();
    match spec.to_pretty_json() {
        Ok(json) => println!("{json}"),
        Err(err) => {
            eprintln!("emit-openapi: failed to serialize OpenAPI spec: {err}");
            std::process::exit(1);
        }
    }
}
