//! The steward's skill doc is a CALL RECIPE, not prose — this keeps it honest against the schemas.
//!
//! `agent/skills/map-stewardship.md` is appended verbatim to the model's context on every steward
//! tick (`load_skill`), and what it writes is what the model sends. In production it drifted from
//! the MCP schemas and the model followed the doc: `create_resource` lost required `title` (the
//! recipe never named it), `assert_relationship` lost `source` (written as the arrow form
//! `node -> source`), `facet_set` lost `resource` (written positionally), and
//! `invocation_open` gained a `parent_cogmap` filled with the dispatch JOB id — tripping the
//! delegation gate and failing the whole tick.
//!
//! Nothing linked the doc to the schemas, so the drift was silent and invisible to review. This
//! test is that link. It is deliberately in temper-mcp rather than the steward package because the
//! schemas are the authority and they are Rust; a copy of them next to the doc would be one more
//! thing to drift.
//!
//! Two checks, matching the two ways the doc broke:
//!   1. every `name=` the doc writes is a REAL property of that tool  (catches `type=`, `kind=`,
//!      `trigger=` — a misnamed argument)
//!   2. every REQUIRED property of a tool the doc calls is mentioned  (catches the silent omission
//!      of `title` / `source` / `resource` / `weight` / `disposition`)
//!
//! Consolidation note (the tool-table rewrite that merged the four relationship verbs into
//! `relationship` and open/close into `invocation_manage`): check 2 now reads the consolidated
//! inputs, whose schemars `required` is `action` alone — every other field is `#[serde(default)]`,
//! and the per-action requirements moved into the dispatchers (`relationships.rs`'s action table,
//! `invocations.rs`'s match arms), enforced per call at tick time. What this guard's check 2
//! therefore still bites on: a doc call missing `action` (or `view`/`cogmap` on `cogmap_read`,
//! whose input kept required fields). What it can no longer see, stated rather than left to be
//! discovered: a doc `assert` omitting `source`, or a `close` omitting `disposition`, passes
//! here and fails only in the dispatcher — that per-action table lives with the dispatch code
//! and its tests, and this file deliberately does not restate it (a second copy is one more
//! thing to drift).

use std::collections::BTreeSet;
use std::path::PathBuf;

use temper_core::types::api::SearchParams;
use temper_core::types::steward::{StewardAdvanceWatermarkInput, StewardDeltaInput};
use temper_mcp::tools::cognitive_maps::{
    with_leading_notice, CogmapReadInput, CHARTER_READING_NOTICE,
};
use temper_mcp::tools::facets::{EdgeFacetSetInput, FacetSetInput, FacetSetUnifiedInput};
use temper_mcp::tools::invocations::InvocationManageInput;
use temper_mcp::tools::relationships::RelationshipInput;
use temper_mcp::tools::resources::CreateResourceInput;

/// Pseudocode placeholders the recipe is allowed to write in an argument list. `act` stands for the
/// authorship envelope (`invocation_id` / `confidence` / `reasoning` / `model`), which the doc
/// defines once at the top of the loop rather than repeating on every call.
const PSEUDOCODE_TOKENS: &[&str] = &["act"];

fn skill_doc() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/agent-workflows/steward/agent/skills/map-stewardship.md");
    // A missing doc must FAIL, never skip — a guard that silently disappears when its subject
    // moves is not a guard.
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read steward skill doc at {}: {e}", path.display()))
}

/// One tool's advertised contract, read from the same schemars derive the MCP server serves.
struct ToolSchema {
    name: &'static str,
    properties: BTreeSet<String>,
    required: BTreeSet<String>,
}

fn tool(name: &'static str, schema: serde_json::Value) -> ToolSchema {
    let properties = schema
        .pointer("/properties")
        .and_then(|p| p.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let required = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    ToolSchema {
        name,
        properties,
        required,
    }
}

fn schemas() -> Vec<ToolSchema> {
    vec![
        tool(
            "create_resource",
            serde_json::to_value(schemars::schema_for!(CreateResourceInput)).unwrap(),
        ),
        tool(
            "relationship",
            serde_json::to_value(schemars::schema_for!(RelationshipInput)).unwrap(),
        ),
        tool(
            "facet_set",
            serde_json::to_value(schemars::schema_for!(FacetSetInput)).unwrap(),
        ),
        tool(
            "invocation_manage",
            serde_json::to_value(schemars::schema_for!(InvocationManageInput)).unwrap(),
        ),
        tool(
            "cogmap_read",
            serde_json::to_value(schemars::schema_for!(CogmapReadInput)).unwrap(),
        ),
        tool(
            "steward_ingest_delta",
            serde_json::to_value(schemars::schema_for!(StewardDeltaInput)).unwrap(),
        ),
        tool(
            "steward_advance_watermark",
            serde_json::to_value(schemars::schema_for!(StewardAdvanceWatermarkInput)).unwrap(),
        ),
        tool(
            "search",
            serde_json::to_value(schemars::schema_for!(SearchParams)).unwrap(),
        ),
    ]
}

/// Every `temper__<tool>(...)` call site in the doc, as (tool name, raw argument text).
///
/// Parens are matched by depth so a multi-line call and a nested `[<src id(s)>]` both survive; a
/// bare `temper__foo` mention in prose has no paren and is skipped.
fn call_sites(doc: &str) -> Vec<(String, String)> {
    let mut sites = Vec::new();
    let bytes: Vec<char> = doc.chars().collect();
    let mut idx = 0;
    while let Some(found) = doc[idx..].find("temper__") {
        let start = idx + found;
        let after = start + "temper__".len();
        let name: String = doc[after..]
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || *c == '_')
            .collect();
        let open = after + name.len();
        idx = open.max(start + 1);
        if !doc[open..].starts_with('(') {
            continue; // prose mention, not a call
        }
        let open_char = doc[..open].chars().count();
        let mut depth = 0usize;
        let mut args = String::new();
        for &c in &bytes[open_char..] {
            match c {
                '(' => {
                    depth += 1;
                    if depth == 1 {
                        continue;
                    }
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            args.push(c);
        }
        sites.push((name, args));
    }
    sites
}

/// Argument names written as `name=` in a call's argument text.
fn named_args(args: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (i, ch) in args.char_indices() {
        if ch != '=' {
            continue;
        }
        // Skip `==` / `!=` / `>=` and the inside of a `<...>` placeholder's prose.
        let head = &args[..i];
        let ident: String = head
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !ident.is_empty() {
            names.insert(ident);
        }
    }
    names
}

/// Every bare identifier token in a call's argument text — used for the "is it mentioned at all"
/// check, so the Authorship section's positional form (`facet_set(resource, values, ...)`) counts.
fn mentioned_tokens(args: &str) -> BTreeSet<String> {
    args.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

#[test]
fn skill_recipe_uses_only_real_tool_arguments() {
    let doc = skill_doc();
    let schemas = schemas();
    let mut failures = Vec::new();

    for (tool_name, args) in call_sites(&doc) {
        let Some(schema) = schemas.iter().find(|s| s.name == tool_name) else {
            continue; // a tool this test does not model; the coverage test below guards the set
        };
        for name in named_args(&args) {
            if PSEUDOCODE_TOKENS.contains(&name.as_str()) || schema.properties.contains(&name) {
                continue;
            }
            failures.push(format!(
                "temper__{tool_name}: `{name}=` is not a property of the tool. Real properties: {:?}",
                schema.properties
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the steward skill doc writes arguments the MCP schema does not have — the model will send \
         them and the call will fail:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn skill_recipe_names_every_required_argument() {
    let doc = skill_doc();
    let schemas = schemas();
    let mut failures = Vec::new();

    for (tool_name, args) in call_sites(&doc) {
        let Some(schema) = schemas.iter().find(|s| s.name == tool_name) else {
            continue;
        };
        let mentioned = mentioned_tokens(&args);
        let missing: Vec<&String> = schema
            .required
            .iter()
            .filter(|r| !mentioned.contains(*r))
            .collect();
        if !missing.is_empty() {
            failures.push(format!(
                "temper__{tool_name}({}) omits required {missing:?}",
                args.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "the steward skill doc shows calls missing a REQUIRED argument — the model follows the \
         recipe and the server rejects the call with `missing field`:\n  {}",
        failures.join("\n  ")
    );
}

/// The facet payload's shape is part of the advertised contract, and the boundary enforces it.
///
/// In production (steward runs `wrun_01M1MH5F9W3TC086NYQJ0D7HR7`, `wrun_01M1MYW3AMPDFQ4ZKH4AZZQWSY`,
/// 2026-09-03/04) the model sent `values` as a JSON string and only learned the object constraint
/// from a Postgres error — eight times across two runs, ~2 minutes of full-context re-read per
/// retry. An untyped `serde_json::Value` field advertises *any* JSON type, so the model had nothing
/// to guess from. These tests pin both halves of the fix: the schema says `object`, and a
/// non-object payload is rejected at deserialization — before any database round trip.
#[test]
fn facet_inputs_type_values_as_an_object_and_reject_scalars_at_the_boundary() {
    for (name, schema) in [
        (
            "facet_set (unified)",
            serde_json::to_value(schemars::schema_for!(FacetSetUnifiedInput)).unwrap(),
        ),
        (
            "facet_set (resource)",
            serde_json::to_value(schemars::schema_for!(FacetSetInput)).unwrap(),
        ),
        (
            "edge_facet_set",
            serde_json::to_value(schemars::schema_for!(EdgeFacetSetInput)).unwrap(),
        ),
    ] {
        let values = schema
            .pointer("/properties/values")
            .unwrap_or_else(|| panic!("{name}: `values` missing from the advertised schema"));
        assert_eq!(
            values.get("type").and_then(|t| t.as_str()),
            Some("object"),
            "{name}: `values` must be typed `object` in the advertised schema so a caller can \
             learn the shape without calling the tool; got {values}"
        );
    }

    // The advertised type must also be the accepted type: a scalar payload fails at the
    // deserialization boundary, never at the database.
    let scalar_payloads = [
        serde_json::json!("a string"),
        serde_json::json!(7),
        serde_json::json!(true),
        serde_json::json!([1, 2]),
    ];
    for payload in &scalar_payloads {
        let reject = |name: &str, ok: bool| {
            assert!(
                ok,
                "{name}: a non-object `values` payload ({payload}) must be rejected at the \
                 deserialization boundary — never forwarded to the database"
            );
        };
        reject(
            "facet_set (unified)",
            serde_json::from_value::<FacetSetUnifiedInput>(payload.clone()).is_err(),
        );
        reject(
            "facet_set (resource)",
            serde_json::from_value::<FacetSetInput>(payload.clone()).is_err(),
        );
        reject(
            "edge_facet_set",
            serde_json::from_value::<EdgeFacetSetInput>(payload.clone()).is_err(),
        );
    }

    // And the positive control: an object payload still parses — the guard has a direction.
    assert!(
        serde_json::from_value::<FacetSetUnifiedInput>(serde_json::json!({
            "target": "resource",
            "resource": "019f2391-e001-7933-b88a-28fb92e56ac1",
            "values": {"node_label": "concern", "status": "open"}
        }))
        .is_ok(),
        "an object `values` payload must remain accepted"
    );
}

/// The charter's trust-tier marking, on both halves where it must hold: the DELIVERY side (the
/// notice rides first on every door that hands out charter content, stated by the door itself)
/// and the GUIDANCE side (the steward and auditor instruction files name the rule and the
/// notice's limit). The marking is a reading aid, not a control — these witnesses pin that it is
/// present and honest about its limit, never that it prevents anything.
#[test]
fn charter_trust_tier_marking_rides_first_and_guidance_names_the_rule() {
    // Delivery: the notice is the FIRST content item, ahead of the JSON body, and the body is
    // byte-identical — programmatic consumers keep their shape.
    let body = serde_json::to_string_pretty(&serde_json::json!([
        {"seq": 0, "role": "statement", "body": "purpose prose"}
    ]))
    .unwrap();
    let result = with_leading_notice(CHARTER_READING_NOTICE, body.clone());
    assert_eq!(result.content.len(), 2, "notice + body, nothing else");
    let notice = result.content[0]
        .as_text()
        .unwrap_or_else(|| panic!("first content item must be the text notice"))
        .text
        .clone();
    let delivered = result.content[1]
        .as_text()
        .unwrap_or_else(|| panic!("second content item must be the text body"))
        .text
        .clone();
    assert!(
        notice.starts_with("READING NOTICE — stated by temper"),
        "the notice must ride FIRST, before the charter content: {notice}"
    );
    assert_eq!(delivered, body, "the JSON body is delivered unchanged");
    assert!(
        CHARTER_READING_NOTICE.contains("not a control"),
        "the notice must state its own limit — it must never read as prevention"
    );

    // Guidance: every agent that reads the charter carries the rule where it acts.
    for path in [
        "packages/agent-workflows/steward/agent/instructions.md",
        "packages/agent-workflows/steward/agent/subagents/auditor/instructions.md",
        "packages/agent-workflows/steward/agent/skills/map-stewardship.md",
    ] {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path);
        let doc = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
        assert!(
            doc.contains("a claim to weigh, not a direction to follow"),
            "{path} must carry the charter trust-tier rule — the guidance half of the marking"
        );
    }
}

/// The parser must actually be finding the calls. Without this, a regex/paren bug that matches
/// nothing would make both tests above pass vacuously — the classic way a guard dies quietly.
#[test]
fn parser_finds_the_recipe_calls() {
    let doc = skill_doc();
    let sites = call_sites(&doc);
    let found: BTreeSet<&str> = sites.iter().map(|(n, _)| n.as_str()).collect();

    for expected in [
        "create_resource",
        "relationship",
        "facet_set",
        "invocation_manage",
        "cogmap_read",
        "steward_advance_watermark",
    ] {
        assert!(
            found.contains(expected),
            "parser found no temper__{expected}(...) call site in the skill doc — the guard would \
             pass vacuously. Found: {found:?}"
        );
    }

    let create = sites
        .iter()
        .find(|(n, _)| n == "create_resource")
        .expect("create_resource call site");
    assert!(
        named_args(&create.1).contains("title"),
        "sanity: the create_resource site should parse a `title=` argument, got {:?}",
        named_args(&create.1)
    );
}
