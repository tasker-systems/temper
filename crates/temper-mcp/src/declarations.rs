//! The tool declarations, shipped as a fixture any host asserts against.
//!
//! [`TOOLS_LIST`] is the canonical `tools/list` result this crate declares with the blob door
//! open: every tool's name, description and input schema, plus the list's cache policy. A host
//! runs one test that sends `tools/list` through its own transport and hands the answer, with the
//! [`BlobDoor`] it built its service with, to [`assert_tools_list_response`] (the raw JSON-RPC
//! response bytes) or [`assert_tools_list`] (the `result` value). If the host's build advertises
//! anything else, that host's test goes red. This covers a changed declaration, a different rmcp
//! resolved into the host's graph, or a middleware rewriting the answer.
//!
//! The blob posture is the host's statement, never inferred from the answer: a closed door is
//! held to the set without the blob pair, an open door to the whole set, so a host wired open
//! when it meant closed (or the reverse) goes red too.
//!
//! The comparison is over canonical JSON: every object's keys sorted, arrays in order. Object
//! key order is build-config dependent (serde_json's `preserve_order` is unified into a build by
//! whichever crates enable it) and carries no contract. Array order (`required`, `enum`, the tool
//! order itself) does, and is compared.
//!
//! One spec equivalence is normalized: a result's `resultType: "complete"` is the same answer as
//! an absent `resultType` (MCP 2026-07-28: "the client MUST treat the absent field as
//! `complete`"). rmcp builds every result with it and strips it for a peer that negotiated an
//! older protocol version, so whether it appears depends on the negotiation, not the
//! declarations. Any other `resultType` is compared like every other field.
//!
//! This crate's own tests hold the fixture equal to the router, both through `list_tools`
//! directly and through real rmcp dispatch. Regenerate it with
//! `UPDATE_MCP_DECLARATIONS=1 cargo test -p temperkb-mcp --test declarations_test`, which
//! rewrites the file and then fails, so a regen always costs a second, passing run.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::BlobDoor;

/// The canonical `tools/list` result, blob door open.
pub const TOOLS_LIST: &str = include_str!("declarations/tools_list.json");

/// The blob pair: advertised only when the host's blob door is open.
const BLOB_TOOLS: [&str; 2] = crate::service::BLOB_TOOL_NAMES;

/// The canonical form of a JSON value: every object's keys sorted, recursively; arrays keep
/// their order; compact separators.
pub fn canonical_json(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), sorted(v)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sorted(value)).expect("a JSON value serializes")
}

/// How a host's advertised declarations differ from [`TOOLS_LIST`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationDrift {
    message: String,
}

impl DeclarationDrift {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DeclarationDrift {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DeclarationDrift {}

/// The fixture a host with this blob posture must advertise.
fn expected(blob_door_open: bool) -> Value {
    let mut fixture = without_complete_result_type(
        &serde_json::from_str(TOOLS_LIST).expect("the shipped fixture parses"),
    );
    if !blob_door_open {
        if let Some(tools) = fixture.get_mut("tools").and_then(Value::as_array_mut) {
            tools.retain(|tool| !BLOB_TOOLS.contains(&tool_name(tool)));
        }
    }
    fixture
}

fn tool_name(tool: &Value) -> &str {
    tool.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn tools_of(result: &Value) -> Result<&Vec<Value>, DeclarationDrift> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| DeclarationDrift::new("the tools/list result carries no `tools` array"))
}

/// The result with a `resultType` of `"complete"` removed — the absent field's meaning. The
/// checks apply it to both sides; a host that writes its own fixture applies it too.
pub fn without_complete_result_type(result: &Value) -> Value {
    let mut result = result.clone();
    if let Some(map) = result.as_object_mut() {
        if map.get("resultType").and_then(Value::as_str) == Some("complete") {
            map.remove("resultType");
        }
    }
    result
}

/// Checks a `tools/list` **result** (the JSON-RPC `result` member) against [`TOOLS_LIST`], for a
/// host whose service was built with `blob_door`.
pub fn check_tools_list(result: &Value, blob_door: &BlobDoor) -> Result<(), DeclarationDrift> {
    let result = &without_complete_result_type(result);
    tools_of(result)?;
    let expected = expected(blob_door.is_open());
    if canonical_json(result) == canonical_json(&expected) {
        return Ok(());
    }
    Err(DeclarationDrift::new(describe(result, &expected)))
}

/// Checks a raw `tools/list` JSON-RPC **response** (the bytes a host's transport answered): a
/// JSON-RPC 2.0 envelope of exactly `id`, `jsonrpc` and `result`, whose `result` passes
/// [`check_tools_list`].
pub fn check_tools_list_response(
    bytes: &[u8],
    blob_door: &BlobDoor,
) -> Result<(), DeclarationDrift> {
    let response: Value = serde_json::from_slice(bytes)
        .map_err(|e| DeclarationDrift::new(format!("the tools/list response is not JSON ({e})")))?;
    let envelope = response
        .as_object()
        .ok_or_else(|| DeclarationDrift::new("the tools/list response is not a JSON object"))?;
    let mut keys: Vec<&str> = envelope.keys().map(String::as_str).collect();
    keys.sort_unstable();
    if keys != ["id", "jsonrpc", "result"] || envelope["jsonrpc"] != "2.0" {
        return Err(DeclarationDrift::new(format!(
            "the tools/list response is not a JSON-RPC 2.0 result envelope: {}",
            canonical_json(&Value::Object(
                envelope
                    .iter()
                    .filter(|(k, _)| k.as_str() != "result")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            ))
        )));
    }
    check_tools_list(&envelope["result"], blob_door)
}

/// Asserts [`check_tools_list`], panicking with the drift.
#[track_caller]
pub fn assert_tools_list(result: &Value, blob_door: &BlobDoor) {
    if let Err(drift) = check_tools_list(result, blob_door) {
        panic!("{drift}");
    }
}

/// Asserts [`check_tools_list_response`], panicking with the drift.
#[track_caller]
pub fn assert_tools_list_response(bytes: &[u8], blob_door: &BlobDoor) {
    if let Err(drift) = check_tools_list_response(bytes, blob_door) {
        panic!("{drift}");
    }
}

/// Names what moved: tools added, tools missing, tools whose declaration changed, the tool order,
/// or the list's own fields.
fn describe(actual: &Value, expected: &Value) -> String {
    let index = |v: &Value| -> BTreeMap<String, String> {
        v.get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|t| (tool_name(t).to_string(), canonical_json(t)))
            .collect()
    };
    let (have, want) = (index(actual), index(expected));
    let mut lines = Vec::new();
    let added: Vec<&String> = have.keys().filter(|k| !want.contains_key(*k)).collect();
    let missing: Vec<&String> = want.keys().filter(|k| !have.contains_key(*k)).collect();
    let changed: Vec<&String> = have
        .iter()
        .filter(|(k, v)| want.get(*k).is_some_and(|w| w != *v))
        .map(|(k, _)| k)
        .collect();
    if !added.is_empty() {
        lines.push(format!("advertised but not declared: {added:?}"));
    }
    if !missing.is_empty() {
        lines.push(format!("declared but not advertised: {missing:?}"));
    }
    if !changed.is_empty() {
        lines.push(format!("declaration changed: {changed:?}"));
    }
    if lines.is_empty() {
        let order = |v: &Value| -> Vec<String> {
            v.get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|t| tool_name(t).to_string())
                .collect()
        };
        if order(actual) != order(expected) {
            lines.push("the tool order changed".to_string());
        } else {
            let strip = |v: &Value| {
                let mut v = v.clone();
                if let Some(map) = v.as_object_mut() {
                    map.remove("tools");
                }
                canonical_json(&v)
            };
            lines.push(format!(
                "the list's own fields changed: advertised {}, declared {}",
                strip(actual),
                strip(expected)
            ));
        }
    }
    format!(
        "this host's tools/list differs from the declarations temperkb-mcp {} ships — {}",
        env!("CARGO_PKG_VERSION"),
        lines.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(blob_door_open: bool) -> Value {
        serde_json::to_value(crate::service::list_tools_result(blob_door_open))
            .expect("the list serializes")
    }

    fn door(open: bool) -> BlobDoor {
        if open {
            BlobDoor::Open {
                single_request_max_bytes: 1,
            }
        } else {
            BlobDoor::Closed {
                refusal: "closed".into(),
            }
        }
    }

    /// The fixture IS the router: what `list_tools` answers, door open and door closed, is what
    /// the crate ships. A declaration edit without a regenerated fixture reddens here first.
    #[test]
    fn the_shipped_fixture_is_what_the_router_advertises() {
        assert_tools_list(&served(true), &door(true));
        assert_tools_list(&served(false), &door(false));
        assert_eq!(
            canonical_json(&without_complete_result_type(&served(true))),
            canonical_json(&serde_json::from_str(TOOLS_LIST).unwrap()),
            "the open-door answer is the fixture itself"
        );
    }

    /// The posture is the host's statement: a host that advertises the other door's set is drift,
    /// in both directions, and so is advertising half the blob pair.
    #[test]
    fn the_answer_must_match_the_stated_blob_door() {
        let drift = check_tools_list(&served(true), &door(false))
            .unwrap_err()
            .to_string();
        assert!(drift.contains("advertised but not declared"), "{drift}");
        let drift = check_tools_list(&served(false), &door(true))
            .unwrap_err()
            .to_string();
        assert!(drift.contains("declared but not advertised"), "{drift}");

        let mut half_blob = served(true);
        half_blob["tools"]
            .as_array_mut()
            .unwrap()
            .retain(|t| tool_name(t) != "blob_read");
        assert!(check_tools_list(&half_blob, &door(true)).is_err());
        assert!(check_tools_list(&half_blob, &door(false)).is_err());
    }

    /// `resultType: "complete"` is the absent field's meaning and passes either way, on the
    /// answer's side and the fixture's; any other value is drift.
    #[test]
    fn only_a_complete_result_type_is_equivalent_to_none() {
        let mut complete = served(true);
        complete["resultType"] = Value::from("complete");
        assert_tools_list(&complete, &door(true));
        complete.as_object_mut().unwrap().remove("resultType");
        assert_tools_list(&complete, &door(true));

        let mut other = served(true);
        other["resultType"] = Value::from("input_required");
        let drift = check_tools_list(&other, &door(true))
            .unwrap_err()
            .to_string();
        assert!(drift.contains("own fields"), "{drift}");
    }

    /// Each kind of drift is caught and named, and a passing check is not vacuous.
    #[test]
    fn each_drift_is_caught_and_named() {
        let mut described = served(true);
        described["tools"][0]["description"] = Value::String("a drifted description".into());
        let drift = check_tools_list(&described, &door(true))
            .unwrap_err()
            .to_string();
        assert!(drift.contains("declaration changed"), "{drift}");

        let mut dropped = served(false);
        dropped["tools"].as_array_mut().unwrap().pop();
        let drift = check_tools_list(&dropped, &door(false))
            .unwrap_err()
            .to_string();
        assert!(drift.contains("declared but not advertised"), "{drift}");

        let mut reordered = served(true);
        reordered["tools"].as_array_mut().unwrap().swap(0, 1);
        let drift = check_tools_list(&reordered, &door(true))
            .unwrap_err()
            .to_string();
        assert!(drift.contains("order"), "{drift}");

        let mut ttl = served(true);
        ttl["ttlMs"] = Value::from(1);
        let drift = check_tools_list(&ttl, &door(true)).unwrap_err().to_string();
        assert!(drift.contains("own fields"), "{drift}");
    }

    #[test]
    fn the_response_check_reads_the_envelope() {
        let ok = serde_json::json!({"jsonrpc": "2.0", "id": 7, "result": served(true)});
        check_tools_list_response(&serde_json::to_vec(&ok).unwrap(), &door(true))
            .expect("a clean envelope");

        let err = serde_json::json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -1}});
        assert!(
            check_tools_list_response(&serde_json::to_vec(&err).unwrap(), &door(true)).is_err()
        );
        assert!(check_tools_list_response(b"not json", &door(true)).is_err());
    }
}
