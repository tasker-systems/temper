//! Input types used by operation commands.
//!
//! `BodyUpdate` represents the new body content for an update; `ListFilter`
//! and `SearchQuery` carry list/search inputs. Kept small and serde-friendly.

use serde::{Deserialize, Serialize};

/// New body content for an `UpdateResource` (or `CreateResource`) command.
///
/// Wraps a String so we can extend with body-meta fields (e.g., explicit
/// content hash, encoding) without breaking the command struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyUpdate {
    pub content: String,
}

impl BodyUpdate {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

/// Filter inputs for `ListResources`.
///
/// All fields optional — caller passes the subset they want to filter by.
/// Stage / doctype / context filters mirror what the API surface accepts today.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListFilter {
    pub doctype: Option<String>,
    pub context: Option<String>,
    pub stage: Option<String>,
    pub goal: Option<String>,
    pub limit: Option<u32>,
}

/// Query input for `SearchResources`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub doctype: Option<String>,
    pub context: Option<String>,
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_update_new_wraps_content() {
        let b = BodyUpdate::new("hello");
        assert_eq!(b.content, "hello");
    }

    #[test]
    fn list_filter_default_is_all_none() {
        let f = ListFilter::default();
        assert!(f.doctype.is_none());
        assert!(f.context.is_none());
        assert!(f.stage.is_none());
        assert!(f.goal.is_none());
        assert!(f.limit.is_none());
    }

    #[test]
    fn search_query_round_trips() {
        let q = SearchQuery {
            query: "rust".to_string(),
            doctype: Some("task".to_string()),
            context: None,
            limit: Some(10),
        };
        let s = serde_json::to_string(&q).unwrap();
        let back: SearchQuery = serde_json::from_str(&s).unwrap();
        assert_eq!(q, back);
    }
}
