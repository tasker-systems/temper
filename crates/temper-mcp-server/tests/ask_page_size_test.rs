//! A full batch-read page, as `ask_data_artifacts` answers it, fits the platform's response cap.
//!
//! The page's byte budget (`temper_services::transport::ARTIFACT_ASK_PAGE_BYTES`) is sized for the
//! MCP relay on two conditions the tool must keep: it writes the page compactly, and it sends it
//! once. The arithmetic is on the constant's doc and held by temper-services' own test; this one
//! measures the tool's real answer to the worst page the budget admits. It lives in the deployed
//! shell because the tool layer depends on neither temper-services nor its constants.

use temper_core::types::artifact_ask::{
    AskOrder, AskResponse, AskedShape, OversizedArtifact, ShapeSuccessor,
};
use temper_core::types::data_artifact::{ArtifactCursor, ArtifactView, MAX_ARTIFACT_PAGE_ROWS};
use temper_core::types::ids::{DataArtifactId, ResourceId, ShapeId};
use temper_core::types::query::ArtifactAskPopulation;
use temper_services::transport::{
    ARTIFACT_ASK_PAGE_BYTES, ARTIFACT_PAGE_ENVELOPE_BYTES, ARTIFACT_ROW_FIXED_BYTES,
    VERCEL_RESPONSE_BODY_CAP_BYTES,
};
use uuid::Uuid;

/// A text field's length as the JSON string a page writes and the page's SQL charges.
fn written(text: &str) -> usize {
    serde_json::to_string(text)
        .expect("a string serializes")
        .len()
}

/// One row of the worst page, before its content: a family name of 1,000 backslashes, which a
/// page writes doubled and the relay doubles again.
fn row(n: u128, family: &str, content: serde_json::Value) -> ArtifactView {
    ArtifactView {
        artifact_id: DataArtifactId::from(Uuid::from_u128(n)),
        resource_id: ResourceId::from(Uuid::max()),
        kind_owner_table: "kb_profiles".to_string(),
        kind_owner_id: Uuid::max(),
        artifact_kind: family.to_string(),
        intent: "member".to_string(),
        precedence: -f64::MIN_POSITIVE,
        content_hash: "f".repeat(64),
        content_bytes: i64::try_from(serde_json::to_string(&content).expect("serializes").len())
            .expect("fits"),
        shape_state: "declared_not_yet_checked".to_string(),
        is_folded: false,
        created: Default::default(),
        content: Some(content),
    }
}

/// What the page's SQL charges a served row: its content and its metadata.
fn charge(r: &ArtifactView) -> usize {
    usize::try_from(r.content_bytes).expect("fits")
        + ARTIFACT_ROW_FIXED_BYTES
        + written(&r.kind_owner_table)
        + written(&r.artifact_kind)
        + written(&r.intent)
        + written(&r.content_hash)
        + written(&r.shape_state)
}

/// **The worst page the budget admits, answered by the tool, fits `VERCEL_RESPONSE_BODY_CAP_BYTES`
/// with the relay's envelope beside it.** `MAX_ARTIFACT_PAGE_ROWS` rows charged up to the budget:
/// half carry a string of backslashes (the content the relay's escaping doubles), half an array of
/// zeros (the content pretty-printing inflates most), every one a long backslashed family name;
/// plus a full envelope. The page is within the budget as the SQL charges it, its API body within
/// the budget and the envelope, and the tool's whole `CallToolResult`, serialized, within the cap.
/// Bite: `to_string_pretty` in `ask_result` — the array rows' indentation alone carries it past the
/// cap.
#[test]
fn the_worst_page_the_budget_admits_fits_the_response_cap_as_the_tool_sends_it() {
    let rows = usize::try_from(MAX_ARTIFACT_PAGE_ROWS).expect("fits");
    let family = "\\".repeat(1000);
    let metadata = charge(&row(0, &family, serde_json::Value::Null))
        - serde_json::to_string(&serde_json::Value::Null)
            .expect("serializes")
            .len();
    let allowance = ARTIFACT_ASK_PAGE_BYTES / rows - metadata;
    let artifacts: Vec<ArtifactView> = (0..rows)
        .map(|i| {
            let content = if i % 2 == 0 {
                // `"` + 2 bytes per backslash + `"`.
                serde_json::Value::String("\\".repeat((allowance - 2) / 2))
            } else {
                // `[` + `0,` per element, the last without its comma, + `]`.
                serde_json::Value::Array(vec![serde_json::Value::from(0); (allowance - 1) / 2])
            };
            row(u128::try_from(i).expect("fits"), &family, content)
        })
        .collect();
    let charged: usize = artifacts.iter().map(charge).sum();
    assert!(
        charged <= ARTIFACT_ASK_PAGE_BYTES,
        "the fixture is a page the budget admits: {charged}"
    );

    let page = AskResponse {
        shape: AskedShape {
            shape_id: ShapeId::from(Uuid::max()),
            shape_version: i32::MAX,
            home_anchor_table: "kb_cogmaps".to_string(),
            home_anchor_id: Uuid::max(),
        },
        artifacts,
        oversized: Vec::<OversizedArtifact>::new(),
        population: Some(ArtifactAskPopulation {
            declared_satisfied: i64::MAX,
            declared_not_satisfied: i64::MAX,
            declared_not_yet_checked: i64::MAX,
            matched: i64::MAX,
        }),
        ordered_by: AskOrder::Created,
        next: Some(ArtifactCursor {
            created: Default::default(),
            artifact_id: DataArtifactId::from(Uuid::max()),
        }),
        amended_since: Some(ShapeSuccessor {
            shape_id: ShapeId::from(Uuid::max()),
            shape_version: i32::MAX,
        }),
    };
    let body = serde_json::to_string(&page)
        .expect("the page serializes")
        .len();
    assert!(
        body <= ARTIFACT_ASK_PAGE_BYTES + ARTIFACT_PAGE_ENVELOPE_BYTES,
        "the API body is {body} bytes"
    );

    let answered = temper_mcp::tools::data_artifacts::ask_result(&page).expect("answered");
    let relayed = serde_json::to_string(&answered)
        .expect("the result serializes")
        .len();
    assert!(
        relayed + ARTIFACT_PAGE_ENVELOPE_BYTES <= VERCEL_RESPONSE_BODY_CAP_BYTES,
        "the tool's answer is {relayed} bytes, before the JSON-RPC envelope"
    );
    assert!(
        relayed > body,
        "the relay escapes the page: {relayed} against {body}"
    );
}
