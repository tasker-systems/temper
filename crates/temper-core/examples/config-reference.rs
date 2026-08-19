//! Emit the machine-readable description of `TemperConfig` that
//! `docs/reference/config` is rendered from.
//!
//! Prints one JSON object to stdout with two members, because neither alone is
//! enough to document a config field honestly:
//!
//! * `schema` — the schemars JSON Schema. Carries the shape, the types, which
//!   fields are required, and each field's **doc comment** (schemars lifts
//!   `///` into `description`). The doc comments are the whole reason this
//!   exists: they live in Rust source, and lifting them through a derive makes
//!   them a compiled artifact rather than something a docs tool has to scrape
//!   out of `.rs` files.
//! * `defaults` — `TemperConfig::default()` serialized. A JSON Schema records
//!   that a field *has* a default far more reliably than it records what that
//!   default *is*, and "what will I get if I leave this out" is the question a
//!   reader actually has. Taking it from the real `Default` impl means the
//!   documented value is the value, not a transcription of one.
//!
//! An example rather than a subcommand, deliberately. `temper config schema`
//! would put a docs-generation detail into the user-facing CLI surface — and
//! then into the generated CLI reference, which would be documenting the
//! machinery that documents it.
//!
//!   cargo run -q -p temper-core --features config-schema --example config-reference

use temper_core::types::config::TemperConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = schemars::schema_for!(TemperConfig);
    let defaults = serde_json::to_value(TemperConfig::default())?;

    // Also as TOML, because config.toml is the format a reader actually writes. Rendered
    // by the same `toml` crate that PARSES the file at load time, so the documented starting
    // point is guaranteed to round-trip rather than being a plausible-looking transcription.
    let defaults_toml = toml::to_string_pretty(&TemperConfig::default())?;

    let document = serde_json::json!({
        "schema": schema,
        "defaults": defaults,
        "defaults_toml": defaults_toml,
    });

    println!("{}", serde_json::to_string_pretty(&document)?);
    Ok(())
}
