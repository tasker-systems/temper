//! Conditional writes: what a write expects to find, and the refusal when it is not there.
//!
//! A meta write (`PUT /api/resources/{id}/meta`, or a `PATCH /api/resources/{id}` that touches
//! meta) may carry an `expect`: per-key **expected values**, JSON-Patch `test` semantics. A
//! `PATCH` may also expect the body: a **body token** ([`body_token`]) naming the exact readable
//! bytes the writer read, sent as `expect.body` or as `If-Match`. The server judges every
//! expectation against the resource's current readable state, under the resource's writer lock,
//! in the same transaction as the write. If any expectation fails nothing is written, and the
//! caller gets `412 PRECONDITION_FAILED` carrying a [`PreconditionFailure`].
//!
//! **Not a lock.** An expectation is judged once, at the write. A value that changed and changed
//! back (ABA) passes; lease meaning, retry and backoff are the caller's.

use std::collections::BTreeMap;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The version prefix of every body token. It versions the hash construction only, not the
/// renderer that produced the bytes: a change to how `/content` renders a body moves the token of
/// every body it renders differently, under the same prefix.
pub const BODY_TOKEN_PREFIX: &str = "bt1.";

/// The body token of a readable body: `bt1.` followed by the unpadded base64url (RFC 4648 §5)
/// sha256 of exactly these bytes, 47 characters in all.
///
/// The one encoder. Every door that answers a token computes it here, over the bytes that door
/// returns, so a token always describes the bytes beside it. It is an unsalted content hash and
/// opaque to callers: compare tokens for equality, never parse them.
pub fn body_token(markdown: &str) -> String {
    let digest = Sha256::digest(markdown.as_bytes());
    let mut token = String::with_capacity(47);
    token.push_str(BODY_TOKEN_PREFIX);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode_string(digest, &mut token);
    token
}

/// Strip one pair of enclosing double quotes, if there is one. A token is unquoted in JSON and
/// quoted in a header; either spelling compares equal.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(s)
}

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

/// What a `PATCH /api/resources/{id}` expects to find: the meta tiers of [`MetaExpect`], plus
/// the body.
///
/// - **`open` and `managed`** follow every rule of [`MetaExpect`].
/// - **`body`** is a body token, exactly as a read answered it (`body_token` on
///   `GET /api/resources/{id}/content`, on `?sections=body`, or on an earlier `PATCH` that wrote
///   the body). The write lands only if the resource's readable body is still exactly those bytes.
///   It guards the body only; a title change on the same request is not guarded by it.
/// - **A retried body save that already landed is answered `200`.** When the body expectation
///   fails but the request is a whole-body save carrying nothing but `content` and its bytes
///   already equal the current body, nothing is written and the answer is the current resource
///   and its current token. Any other request whose body expectation fails is a `412`.
///
/// `body` exists only here: `PUT /meta`'s [`MetaExpect`] has no such member, so a meta write
/// carrying one is refused (`422`). `If-Match` on `PATCH` is the same guard as `body`; sending
/// both is refused (`400`) unless the header is the one strong tag `body` names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
pub struct ResourceExpect {
    /// Open-tier keys and the value each is expected to hold now (`null`: absent).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "web-api", schema(value_type = Object))]
    pub open: BTreeMap<String, Value>,
    /// Managed-tier keys and the value each is expected to hold now (`null`: absent).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "web-api", schema(value_type = Object))]
    pub managed: BTreeMap<String, Value>,
    /// The body token of the readable body this write expects to replace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

impl ResourceExpect {
    /// True when the expectation names no key and no body.
    pub fn is_empty(&self) -> bool {
        self.open.is_empty() && self.managed.is_empty() && self.body.is_none()
    }

    /// The meta tiers alone, as a meta write's expectation.
    pub fn meta(&self) -> MetaExpect {
        MetaExpect {
            open: self.open.clone(),
            managed: self.managed.clone(),
        }
    }
}

impl From<MetaExpect> for ResourceExpect {
    fn from(meta: MetaExpect) -> Self {
        Self {
            open: meta.open,
            managed: meta.managed,
            body: None,
        }
    }
}

/// One member of an `If-Match` list, or the one tag `expect.body` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTag {
    /// `W/`: a weak tag. A body token is only ever strong.
    pub weak: bool,
    /// The tag without its quotes.
    pub opaque: String,
}

/// The body condition a `PATCH` carries: the tags any one of which must name the current body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyCondition {
    pub tags: Vec<EntityTag>,
}

/// How a [`BodyCondition`] stands against the current body token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyJudgment {
    /// A strong member names the current body.
    Holds,
    /// No strong member names it, but a weak one's opaque value does. Refused `400`, never `412`:
    /// a client that re-reads and retries on `412` would be refused the same way forever.
    WeakMatch,
    /// No member names it.
    Fails,
}

impl BodyCondition {
    /// Judge against the current body's token.
    pub fn judge(&self, current: &str) -> BodyJudgment {
        if self.tags.iter().any(|t| !t.weak && t.opaque == current) {
            BodyJudgment::Holds
        } else if self.tags.iter().any(|t| t.weak && t.opaque == current) {
            BodyJudgment::WeakMatch
        } else {
            BodyJudgment::Fails
        }
    }

    /// What the caller sent, as a refusal reports it: the one token, or every member's token in
    /// the order sent.
    pub fn expected(&self) -> Value {
        match self.tags.as_slice() {
            [one] => Value::String(one.opaque.clone()),
            many => Value::Array(
                many.iter()
                    .map(|t| Value::String(t.opaque.clone()))
                    .collect(),
            ),
        }
    }
}

/// Parse an `If-Match` field value into its entity-tags.
///
/// A comma-separated list; each member is `"…"` or `W/"…"`, and one pair of quotes is stripped.
/// A bare unquoted member is read as a strong tag. `*` is refused: a body token always names
/// specific bytes, and "any current body" guards nothing. An empty list, or a quote left open, is
/// refused rather than read as "no condition". The `Err` is the refusal's sentence, which never
/// quotes the value.
pub fn parse_if_match(raw: &str) -> Result<BodyCondition, String> {
    let mut tags = Vec::new();
    let mut rest = raw;
    loop {
        rest = rest.trim_start_matches([',', ' ', '\t']);
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('*') {
            return Err(
                "If-Match: * is not honoured on this route; send the body_token you \
                        read, so the write lands only on those bytes"
                    .into(),
            );
        }
        let (weak, after) = match rest.strip_prefix("W/") {
            Some(after) => (true, after),
            None => (false, rest),
        };
        let (opaque, next) = if let Some(quoted) = after.strip_prefix('"') {
            let Some(end) = quoted.find('"') else {
                return Err("If-Match holds an entity-tag whose quote is never closed".into());
            };
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = after.find(',').unwrap_or(after.len());
            (after[..end].trim_end(), &after[end..])
        };
        let next = next.trim_start_matches([' ', '\t']);
        if !(next.is_empty() || next.starts_with(',')) {
            return Err("If-Match is not a comma-separated list of entity-tags".into());
        }
        if opaque.is_empty() {
            return Err("If-Match holds an empty entity-tag".into());
        }
        tags.push(EntityTag {
            weak,
            opaque: opaque.to_owned(),
        });
        rest = next;
    }
    if tags.is_empty() {
        return Err("If-Match names no entity-tag".into());
    }
    Ok(BodyCondition { tags })
}

/// The one body condition a `PATCH` carries, from `expect.body` and the `If-Match` header.
///
/// `None` when it carries neither. `expect.body` must be one token: empty, `*`, a weak tag, a
/// stray quote, a comma or whitespace is refused. Both may be sent only when they agree: the header is exactly
/// one strong tag, equal to `expect.body` (one pair of quotes stripped from each). Any other pair
/// is refused, as is an `If-Match` that does not parse ([`parse_if_match`]). `Err` is the
/// refusal's sentence, which quotes no value.
pub fn body_condition(
    expect_body: Option<&str>,
    if_match: Option<&str>,
) -> Result<Option<BodyCondition>, String> {
    let header = if_match.map(parse_if_match).transpose()?;
    let body = expect_body
        .map(|b| {
            let opaque = unquote(b);
            // The same shape rules a header member meets: `expect.body` is one strong token.
            let malformed = opaque.is_empty()
                || opaque == "*"
                || opaque.starts_with("W/")
                || b.starts_with("W/")
                || opaque.contains(['"', ',', ' ', '\t']);
            if malformed {
                return Err(
                    "expect.body is not a body token; send the body_token you read, as it was \
                     answered"
                        .to_string(),
                );
            }
            Ok(BodyCondition {
                tags: vec![EntityTag {
                    weak: false,
                    opaque: opaque.to_owned(),
                }],
            })
        })
        .transpose()?;
    match (body, header) {
        (None, None) => Ok(None),
        (Some(c), None) | (None, Some(c)) => Ok(Some(c)),
        (Some(body), Some(header)) => {
            if header == body {
                Ok(Some(body))
            } else {
                Err(
                    "If-Match and expect.body name different body conditions; send one of them \
                     (If-Match is a synonym for expect.body on this route)"
                        .into(),
                )
            }
        }
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
    /// The body. Its entry carries no `key`; `expected` and `current` are body tokens.
    Body,
}

impl PreconditionTier {
    /// The wire spelling, also used in the refusal's message (`open.claim_id`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Managed => "managed",
            Self::Body => "body",
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
    /// The key the expectation named. Present on every `open` and `managed` entry; never on a
    /// `body` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// What the caller expected, as it sent it (`null`: absent). On a `body` entry, the token
    /// sent, or every token of an `If-Match` list in the order sent.
    pub expected: Value,
    /// What is there now (`null`: absent). On a `body` entry, the current body's token.
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

    // FAILS IF: the encoder's construction drifts from `bt1.` + base64url-no-pad(sha256(bytes)):
    // every token any client holds would stop matching, under the same prefix.
    #[test]
    fn the_body_token_is_bt1_and_unpadded_base64url_sha256_of_the_bytes() {
        // sha256("") = e3b0c442…; its unpadded base64url is the 43 characters below.
        assert_eq!(
            body_token(""),
            "bt1.47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU"
        );
        assert_eq!(body_token("# A\n\nbody\n").len(), 47);
        assert_ne!(body_token("a\n"), body_token("a\r\n"));
        assert!(body_token("x")
            .chars()
            .skip(4)
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    fn strong(opaque: &str) -> EntityTag {
        EntityTag {
            weak: false,
            opaque: opaque.into(),
        }
    }

    // FAILS IF: a header member keeps its quotes, a list stops at its first member, a weak tag
    // reads as strong, or `*`, an empty list or an open quote is read as "no condition".
    #[test]
    fn if_match_parses_lists_weak_tags_and_one_pair_of_quotes() {
        let parsed = parse_if_match(r#""bt1.a", W/"bt1.b" ,"bt1.c""#).expect("parses");
        assert_eq!(
            parsed.tags,
            vec![
                strong("bt1.a"),
                EntityTag {
                    weak: true,
                    opaque: "bt1.b".into()
                },
                strong("bt1.c"),
            ]
        );
        assert_eq!(
            parse_if_match("bt1.bare").expect("bare").tags,
            vec![strong("bt1.bare")]
        );
        for refused in ["*", " * ", "", " , ", "\"bt1.a", "\"a\" junk", "\"\""] {
            assert!(parse_if_match(refused).is_err(), "{refused:?}");
        }
        assert!(!parse_if_match("*").unwrap_err().contains("bt1"));
    }

    // FAILS IF: a list does not match on any member, a weak tag equal to the current token is
    // judged a plain failure (a 412 a re-reading client would loop on), or a weak tag satisfies.
    #[test]
    fn a_body_condition_matches_any_strong_member_and_flags_a_weak_match() {
        let list = parse_if_match(r#""bt1.x", "bt1.now""#).unwrap();
        assert_eq!(list.judge("bt1.now"), BodyJudgment::Holds);
        assert_eq!(list.judge("bt1.other"), BodyJudgment::Fails);
        assert_eq!(list.expected(), json!(["bt1.x", "bt1.now"]));
        let weak = parse_if_match(r#"W/"bt1.now""#).unwrap();
        assert_eq!(weak.judge("bt1.now"), BodyJudgment::WeakMatch);
        assert_eq!(weak.judge("bt1.other"), BodyJudgment::Fails);
        assert_eq!(weak.expected(), json!("bt1.now"));
    }

    // FAILS IF: `If-Match` and `expect.body` may disagree and one silently wins, or agreeing
    // spellings (quoted header, unquoted body) are refused.
    #[test]
    fn if_match_and_expect_body_must_agree() {
        assert_eq!(body_condition(None, None), Ok(None));
        let agreed = body_condition(Some("bt1.a"), Some("\"bt1.a\""))
            .unwrap()
            .unwrap();
        assert_eq!(agreed.tags, vec![strong("bt1.a")]);
        assert!(body_condition(Some("bt1.a"), Some("\"bt1.b\"")).is_err());
        assert!(body_condition(Some("bt1.a"), Some("W/\"bt1.a\"")).is_err());
        assert!(body_condition(Some("bt1.a"), Some("\"bt1.a\", \"bt1.b\"")).is_err());
        assert!(body_condition(None, Some("*")).is_err());
        for refused in [
            "",
            "*",
            "\"\"",
            "W/\"bt1.a\"",
            "\"W/x\"",
            "bt1.\"a",
            "bt1.a, bt1.b",
            " bt1.a",
        ] {
            assert!(body_condition(Some(refused), None).is_err(), "{refused:?}");
        }
        assert!(body_condition(Some("\"bt1.q\""), None).unwrap().is_some());
        let header_only = body_condition(None, Some("\"bt1.h\"")).unwrap().unwrap();
        assert_eq!(header_only.tags, vec![strong("bt1.h")]);
    }

    // FAILS IF: `expect.body` reaches `PUT /meta`'s type, or `PATCH`'s type drops an unknown
    // member silently.
    #[test]
    fn body_is_a_member_of_the_patch_expectation_only() {
        let patch: ResourceExpect =
            serde_json::from_value(json!({"body": "bt1.x", "open": {"k": 1}})).expect("parses");
        assert_eq!(patch.body.as_deref(), Some("bt1.x"));
        assert!(!patch.is_empty());
        assert!(serde_json::from_value::<ResourceExpect>(json!({"bodyy": "bt1.x"})).is_err());
        assert!(ResourceExpect::default().is_empty());
    }

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

    // FAILS IF: a body entry gains a `key`, or the message quotes a token.
    #[test]
    fn a_body_entry_carries_no_key_and_its_message_names_only_the_tier() {
        let failure = PreconditionFailure {
            failed: vec![FailedExpectation {
                tier: PreconditionTier::Body,
                key: None,
                expected: json!("bt1.old"),
                current: json!("bt1.new"),
            }],
        };
        assert_eq!(
            serde_json::to_value(&failure).unwrap(),
            json!({"failed": [{"tier": "body", "expected": "bt1.old", "current": "bt1.new"}]})
        );
        assert_eq!(failure.message(), "1 precondition failed: body");
    }
}
