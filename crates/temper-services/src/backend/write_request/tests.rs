use super::*;
use temper_core::types::provenance::ProvenanceSource;
use uuid::Uuid;

fn payload(content: &str, context_ref: &str, cogmap: Option<Uuid>) -> IngestPayload {
    serde_json::from_value(serde_json::json!({
        "title": "T",
        "origin_uri": "",
        "context_ref": context_ref,
        "home_cogmap_id": cogmap,
        "doc_type_name": "research",
        "content": content,
    }))
    .expect("minimal IngestPayload")
}

fn refusal(result: Result<(), TemperError>) -> String {
    match result {
        Err(TemperError::BadRequest(m)) => m,
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

#[test]
fn an_ingest_with_sources_and_no_content_is_refused() {
    let mut p = payload("", "@me/ctx", None);
    p.sources = vec![ProvenanceSource::Resource(Uuid::now_v7())];
    assert_eq!(refusal(check_ingest_payload(&p)), SOURCES_WITHOUT_CONTENT);
}

#[test]
fn an_ingest_with_sources_and_content_passes() {
    let mut p = payload("body", "@me/ctx", None);
    p.sources = vec![ProvenanceSource::Resource(Uuid::now_v7())];
    assert!(check_ingest_payload(&p).is_ok());
}

#[test]
fn an_ingest_with_no_body_and_no_sources_passes() {
    assert!(check_ingest_payload(&payload("", "@me/ctx", None)).is_ok());
}

#[test]
fn an_ingest_naming_two_homes_is_refused() {
    let p = payload("", "@me/ctx", Some(Uuid::now_v7()));
    assert_eq!(refusal(check_ingest_payload(&p)), TWO_HOMES);
}

#[test]
fn a_cogmap_home_with_an_empty_context_ref_passes() {
    assert!(check_ingest_payload(&payload("", "", Some(Uuid::now_v7()))).is_ok());
}

#[test]
fn a_patch_with_sources_and_no_content_is_refused() {
    let req = ResourceUpdateRequest {
        sources: vec![ProvenanceSource::Resource(Uuid::now_v7())],
        ..Default::default()
    };
    assert_eq!(refusal(check_update_request(&req)), SOURCES_WITHOUT_CONTENT);
}

#[test]
fn a_patch_with_a_content_block_and_no_content_is_refused() {
    let req = ResourceUpdateRequest {
        content_block: Some(Uuid::now_v7()),
        ..Default::default()
    };
    assert_eq!(
        refusal(check_update_request(&req)),
        CONTENT_BLOCK_WITHOUT_CONTENT
    );
}

#[test]
fn a_patch_with_content_and_its_addressing_passes() {
    let req = ResourceUpdateRequest {
        content: Some("body".into()),
        sources: vec![ProvenanceSource::Resource(Uuid::now_v7())],
        content_block: Some(Uuid::now_v7()),
        ..Default::default()
    };
    assert!(check_update_request(&req).is_ok());
}

#[test]
fn a_meta_only_patch_passes() {
    assert!(check_update_request(&ResourceUpdateRequest::default()).is_ok());
}
