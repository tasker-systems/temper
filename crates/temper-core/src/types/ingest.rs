//! Ingest API types — request body for POST /api/ingest.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for POST /api/ingest — create resource + upload content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    /// Extracted markdown content
    pub content: String,
    pub title: String,
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    /// Resource URI (e.g., "kb://temper/resource/my-doc")
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimetype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Provenance metadata: device_id, original_path, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Context name — resolved to UUID server-side (alternative to kb_context_id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_name: Option<String>,
    /// Doc type name — resolved to UUID server-side (alternative to kb_doc_type_id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_type_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
    }

    fn minimal_request() -> IngestRequest {
        IngestRequest {
            content: "# Hello\n\nWorld".to_owned(),
            title: "Hello Doc".to_owned(),
            kb_context_id: sample_uuid(),
            kb_doc_type_id: sample_uuid(),
            uri: "kb://temper/resource/hello-doc".to_owned(),
            slug: None,
            mimetype: None,
            tags: None,
            metadata: None,
            context_name: None,
            doc_type_name: None,
        }
    }

    #[test]
    fn test_required_fields_serialize_correctly() {
        let req = minimal_request();
        let json = serde_json::to_value(&req).unwrap();

        assert_eq!(json["content"], "# Hello\n\nWorld");
        assert_eq!(json["title"], "Hello Doc");
        assert_eq!(json["uri"], "kb://temper/resource/hello-doc");
        assert_eq!(
            json["kb_context_id"],
            "00000000-0000-0000-0000-000000000001"
        );
        assert_eq!(
            json["kb_doc_type_id"],
            "00000000-0000-0000-0000-000000000001"
        );
    }

    #[test]
    fn test_optional_fields_skipped_when_none() {
        let req = minimal_request();
        let json = serde_json::to_value(&req).unwrap();
        let obj = json.as_object().unwrap();

        assert!(!obj.contains_key("slug"));
        assert!(!obj.contains_key("mimetype"));
        assert!(!obj.contains_key("tags"));
        assert!(!obj.contains_key("metadata"));
        assert!(!obj.contains_key("context_name"));
        assert!(!obj.contains_key("doc_type_name"));
    }

    #[test]
    fn test_context_name_and_doc_type_name_serialize_when_some() {
        let req = IngestRequest {
            context_name: Some("my-context".to_owned()),
            doc_type_name: Some("note".to_owned()),
            ..minimal_request()
        };
        let json = serde_json::to_value(&req).unwrap();

        assert_eq!(json["context_name"], "my-context");
        assert_eq!(json["doc_type_name"], "note");
    }

    #[test]
    fn test_all_optional_fields_serialize_when_some() {
        let req = IngestRequest {
            slug: Some("hello-doc".to_owned()),
            mimetype: Some("text/markdown".to_owned()),
            tags: Some(vec!["rust".to_owned(), "docs".to_owned()]),
            metadata: Some(serde_json::json!({"device_id": "abc123"})),
            context_name: Some("work".to_owned()),
            doc_type_name: Some("note".to_owned()),
            ..minimal_request()
        };
        let json = serde_json::to_value(&req).unwrap();

        assert_eq!(json["slug"], "hello-doc");
        assert_eq!(json["mimetype"], "text/markdown");
        assert_eq!(
            json["tags"],
            Value::Array(vec!["rust".into(), "docs".into()])
        );
        assert_eq!(json["metadata"]["device_id"], "abc123");
    }

    #[test]
    fn test_roundtrip_deserialization() {
        let req = IngestRequest {
            slug: Some("hello".to_owned()),
            ..minimal_request()
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: IngestRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.title, req.title);
        assert_eq!(deserialized.slug, req.slug);
        assert_eq!(deserialized.content, req.content);
    }
}
