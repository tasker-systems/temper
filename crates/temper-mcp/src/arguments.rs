//! The door's one rule for tool arguments: a call may carry only the arguments its tool declares.
//!
//! Every tool's input struct is deserialized by serde, and most of them `#[serde(flatten)]` the
//! shared act fields. serde cannot apply `deny_unknown_fields` across a flatten, so without this
//! rule an argument the tool does not declare is dropped without a word. On a conditional write
//! that disarms the guard: `"expects"` for `"expect"` and the write lands unconditionally, then
//! answers success.
//!
//! **The declared set is the published schema.** A tool's arguments are its input schema's
//! top-level `properties`, the same object `tools/list` advertises. Every schema this crate
//! publishes is one flat object (the act fields are flattened in, and an action-discriminated tool
//! carries every action's fields), so the set is complete. `fill_correlation_id` in `service.rs`
//! reads the same object.
//!
//! - [`undeclared`] names the arguments a call carries that its tool does not declare.
//! - [`refusal`] answers them: a tool-execution error result (`isError: true`) naming the keys,
//!   never their values. Not a JSON-RPC error, which rmcp logs at `warn` with its `data` in full.
//!   The call never reaches the tool, so nothing is written.
//! - [`close_schema`] stamps `additionalProperties: false` on a tool's advertised schema, so a
//!   client that validates against the schema refuses the same call before sending it.
//!
//! **The rule is per tool, not per action.** An action-discriminated tool (`relationship`,
//! `context_manage`, `segmented_ingest`, ...) declares every action's fields in one schema, so a
//! key declared for one action passes the gate when sent with another, and that action's handler
//! decides what to do with it. A guard argument added to such a tool needs that handler's own
//! check.
//!
//! The rule holds for every tool, read and write alike: one rule for the whole door. The harm is
//! on writes, but a misspelled read argument (a filter, a limit) silently answers a different
//! question, and a read tool's caller gets the same repair.

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::Value;

/// At most this many unknown keys are named in a refusal; the rest are counted.
const MAX_NAMED: usize = 8;

/// Each named key is cut to this many characters: a key is the caller's own text, so it is
/// bounded the way any echoed input is.
const MAX_KEY_CHARS: usize = 64;

/// The top-level arguments `arguments` carries that `schema` does not declare, sorted (the
/// arguments object's key order is build-config dependent). A schema with no `properties`
/// declares nothing, so every argument is undeclared.
pub(crate) fn undeclared(schema: &JsonObject, arguments: &JsonObject) -> Vec<String> {
    let declared = schema.get("properties").and_then(Value::as_object);
    arguments
        .keys()
        .filter(|key| declared.is_none_or(|props| !props.contains_key(key.as_str())))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The refusal for a call to `tool` carrying `unknown` (non-empty), listing what `schema` declares
/// so the caller can repair the call in one round trip. Key names only: no argument value is read.
pub(crate) fn refusal(tool: &str, schema: &JsonObject, unknown: &[String]) -> CallToolResult {
    let mut named: Vec<String> = unknown
        .iter()
        .take(MAX_NAMED)
        .map(|key| format!("`{}`", bounded(key)))
        .collect();
    if unknown.len() > MAX_NAMED {
        named.push(format!("and {} more", unknown.len() - MAX_NAMED));
    }
    let noun = if unknown.len() == 1 {
        "argument"
    } else {
        "arguments"
    };
    let accepted = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            // Sorted: object key order is build-config dependent and carries no meaning.
            let mut keys: Vec<_> = props.keys().map(|k| format!("`{k}`")).collect();
            keys.sort();
            keys.join(", ")
        })
        .filter(|list| !list.is_empty())
        .unwrap_or_else(|| "none".to_string());
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{tool}: unknown {noun} {}; nothing was done. {tool} takes: {accepted}.",
        named.join(", ")
    ))])
}

/// `tool` with `additionalProperties: false` on its input schema: the published form of the rule.
pub(crate) fn close_schema(mut tool: Tool) -> Tool {
    std::sync::Arc::make_mut(&mut tool.input_schema)
        .insert("additionalProperties".to_string(), Value::Bool(false));
    tool
}

/// `key` cut to [`MAX_KEY_CHARS`] characters (marked when cut) and escaped, so caller text can
/// neither leave its backtick span nor reshape the sentence: control and invisible characters
/// (newlines, bidi overrides, zero-width marks) and backslashes are escaped the way `{:?}`
/// escapes them, and a backtick becomes `\u{60}`.
fn bounded(key: &str) -> String {
    let mut out = String::new();
    for c in key.chars().take(MAX_KEY_CHARS) {
        match c {
            // A backslash escapes nothing inside a Markdown code span, so no raw backtick is
            // ever echoed: it takes the `\u{..}` form `escape_debug` gives invisible characters.
            '`' => out.push_str("\\u{60}"),
            // Quotes cannot leave the span; echo them as sent.
            '\'' | '"' => out.push(c),
            c => out.extend(c.escape_debug()),
        }
    }
    if key.chars().count() > MAX_KEY_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(value: Value) -> JsonObject {
        value.as_object().expect("an object").clone()
    }

    fn schema() -> JsonObject {
        object(json!({"type": "object", "properties": {"id": {}, "expect": {}}}))
    }

    fn text(result: &CallToolResult) -> String {
        result.content[0]
            .as_text()
            .expect("a text part")
            .text
            .clone()
    }

    #[test]
    fn declared_arguments_pass_and_undeclared_ones_are_named_sorted() {
        assert!(undeclared(&schema(), &object(json!({"id": 1, "expect": {}}))).is_empty());
        assert_eq!(
            undeclared(
                &schema(),
                &object(json!({"zeta": 1, "id": 1, "expects": {}}))
            ),
            ["expects", "zeta"]
        );
    }

    #[test]
    fn a_schema_without_properties_declares_nothing() {
        assert_eq!(
            undeclared(&object(json!({"type": "object"})), &object(json!({"a": 1}))),
            ["a"]
        );
    }

    #[test]
    fn the_refusal_is_a_tool_error_naming_keys_and_never_values() {
        let result = refusal("update_resource_meta", &schema(), &["expects".to_string()]);
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_none());
        let text = text(&result);
        assert!(text.contains("unknown argument `expects`"), "{text}");
        assert!(text.contains("nothing was done"), "{text}");
        assert!(text.contains("takes: `expect`, `id`"), "{text}");
    }

    #[test]
    fn many_or_long_keys_are_bounded() {
        let long = "k".repeat(500);
        let mut keys: Vec<String> = (0..20).map(|i| format!("key{i}")).collect();
        keys.insert(0, long.clone());
        let text = text(&refusal("t", &schema(), &keys));
        assert!(!text.contains(&long), "a long key is cut");
        assert!(
            text.contains(&format!("`{}…`", "k".repeat(MAX_KEY_CHARS))),
            "{text}"
        );
        assert!(text.contains("and 13 more"), "{text}");
        assert!(!text.contains("key19"), "{text}");
    }

    #[test]
    fn caller_text_in_a_key_cannot_leave_its_span() {
        let key = "x`; nothing was done.\n\u{202e}takes: `id`\u{200b}".to_string();
        let escaped = super::bounded(&key);
        assert!(!escaped.contains('`'), "no raw backtick: {escaped}");
        assert!(!escaped.contains('\n'), "{escaped}");
        assert!(
            !escaped.contains('\u{202e}') && !escaped.contains('\u{200b}'),
            "{escaped}"
        );
        assert_eq!(
            escaped,
            "x\\u{60}; nothing was done.\\n\\u{202e}takes: \\u{60}id\\u{60}\\u{200b}"
        );
        let text = text(&refusal("t", &schema(), &[key]));
        assert_eq!(
            text.matches('`').count() % 2,
            0,
            "every span closes: {text}"
        );
        // An ordinary or merely odd key is quoted as sent.
        assert_eq!(super::bounded("open_meta-tag.v2"), "open_meta-tag.v2");
        assert_eq!(super::bounded("it's \"x\""), "it's \"x\"");
    }

    #[test]
    fn close_schema_stamps_additional_properties_false() {
        let tool = Tool::new("t", "d", std::sync::Arc::new(schema()));
        let closed = close_schema(tool);
        assert_eq!(closed.input_schema["additionalProperties"], json!(false));
    }
}
