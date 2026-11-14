//! Conditional meta writes: what a write expects to find, and the refusal when it is not there.
//!
//! A meta write (`PUT /api/resources/{id}/meta`, or a `PATCH /api/resources/{id}` that touches
//! meta) may carry an `expect`: per-key **expected values**, JSON-Patch `test` semantics. The
//! server judges every expectation against the resource's current readable meta, under the
//! resource's writer lock, in the same transaction as the write. If any expectation fails nothing
//! is written, and the caller gets `412 PRECONDITION_FAILED` carrying a [`PreconditionFailure`].
//!
//! **Not a lock.** An expectation is judged once, at the write. A value that changed and changed
//! back (ABA) passes; lease meaning, retry and backoff are the caller's.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Per-key expected values for a meta write, by tier.
///
/// - **A top-level `null` means absent**: the key has no live value, which is also how a key whose
///   newest stored value is a top-level `null` reads. A nested `null` is a value.
/// - **Values are compared as stored**: after the server's write-time normalization (a bare-string
///   `tags` value is stored as a one-element array) and by JSON value equality, so `1` equals
///   `1.0`. Echo back the values you read rather than reformatting them, and guard on strings or
///   on integers within ±2^53.
/// - **A key need not be one the write writes.** Every named key is judged, written or not.
/// - **`open`** names open-tier keys: never `facet`, `doc_type`, a managed key, or an identity key
///   (`temper-title`, `temper-slug`, `temper-id`, `temper-context`, `temper-goal`,
///   `temper-type`). **`managed`** names managed workflow and provenance keys only
///   (`temper-stage`, `temper-status`, …). Anything else is a `400`.
/// - **A key named here may not also be in `open_meta_add`** (`400`): the union is computed
///   from a read taken before the guard is judged.
/// - **An `expect` on a request that writes nothing is a `400`**: a check without a write is a
///   read.
///
/// Unknown members are refused (`422`), so a misspelled tier is never silently dropped.
///
/// - **A retried write can meet its own success.** If a guarded write landed but its answer was
///   lost, the same request sent again is refused with a `412` whose `current` is the value it
///   wrote. Make guarded values unique to the writer (a claim id, a lease token) so that refusal
///   reads as "my earlier attempt landed".
/// - **Only the named keys are guarded.** Other keys the same write sets land as they would without
///   an `expect`; an `open_meta_add` on an unguarded key unions over a read taken before the lock.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Inlined, as `ManagedMeta` is: an MCP tool's declared input schema must be self-contained.
#[cfg_attr(feature = "mcp", schemars(inline))]
pub struct MetaExpect {
    /// Open-tier keys and the value each is expected to hold now (`null`: absent).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "web-api", schema(value_type = Object))]
    pub open: BTreeMap<String, Value>,
    /// Managed-tier keys and the value each is expected to hold now (`null`: absent).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "web-api", schema(value_type = Object))]
    pub managed: BTreeMap<String, Value>,
}

impl MetaExpect {
    /// True when the expectation names no key at all.
    pub fn is_empty(&self) -> bool {
        self.open.is_empty() && self.managed.is_empty()
    }
}

/// Which tier a failed expectation named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "precondition.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub enum PreconditionTier {
    Open,
    Managed,
}

impl PreconditionTier {
    /// The wire spelling, also used in the refusal's message (`open.claim_id`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Managed => "managed",
        }
    }
}

/// One expectation that did not hold.
///
/// `current` is the value the caller could read now; `null` means the key is absent now. Nothing
/// else is ever added: no actor, time, event, position, revision or count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "precondition.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct FailedExpectation {
    pub tier: PreconditionTier,
    /// The key the expectation named. Present on every `open` and `managed` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// What the caller expected, as it sent it (`null`: absent).
    pub expected: Value,
    /// What is there now (`null`: absent).
    pub current: Value,
}

/// Every expectation of a refused write that did not hold. The payload of a
/// `412 PRECONDITION_FAILED`, carried as `error.precondition`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "precondition.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct PreconditionFailure {
    pub failed: Vec<FailedExpectation>,
}

impl PreconditionFailure {
    /// The refusal's sentence: the count and the failed keys, never a value. It is logged, so
    /// values travel only in the structured payload.
    pub fn message(&self) -> String {
        let names = self
            .failed
            .iter()
            .map(|f| match &f.key {
                Some(key) => format!("{}.{key}", f.tier.as_str()),
                None => f.tier.as_str().to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let n = self.failed.len();
        let noun = if n == 1 {
            "precondition"
        } else {
            "preconditions"
        };
        format!("{n} {noun} failed: {names}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unknown_member_is_refused_never_dropped() {
        let refused = serde_json::from_value::<MetaExpect>(json!({"open": {}, "body": "bt1.x"}));
        assert!(
            refused.is_err(),
            "`body` is not a member of a meta expectation"
        );
        let ok: MetaExpect =
            serde_json::from_value(json!({"open": {"claim": null}})).expect("parses");
        assert_eq!(ok.open.get("claim"), Some(&Value::Null));
        assert!(ok.managed.is_empty());
    }

    #[test]
    fn the_refusal_keeps_null_current_on_the_wire_and_names_keys_only_in_its_message() {
        let failure = PreconditionFailure {
            failed: vec![
                FailedExpectation {
                    tier: PreconditionTier::Open,
                    key: Some("claim_id".into()),
                    expected: Value::Null,
                    current: json!("someone-else"),
                },
                FailedExpectation {
                    tier: PreconditionTier::Managed,
                    key: Some("temper-stage".into()),
                    expected: json!("doing"),
                    current: Value::Null,
                },
            ],
        };
        let wire = serde_json::to_value(&failure).unwrap();
        assert_eq!(
            wire,
            json!({"failed": [
                {"tier": "open", "key": "claim_id", "expected": null, "current": "someone-else"},
                {"tier": "managed", "key": "temper-stage", "expected": "doing", "current": null},
            ]})
        );
        let message = failure.message();
        assert_eq!(
            message,
            "2 preconditions failed: open.claim_id, managed.temper-stage"
        );
        assert!(!message.contains("someone-else") && !message.contains("doing"));
    }
}
