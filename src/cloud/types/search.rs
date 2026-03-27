use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Search mode — determines the query strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Cosine similarity against chunk embeddings (default)
    #[default]
    Semantic,
    /// Full-text search via Postgres tsvector
    Keyword,
    /// Graph traversal from nearest semantic matches
    Graph,
}

/// Query parameters for `GET /api/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub q: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// A single search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub resource_id: Uuid,
    pub title: String,
    pub context: String,
    pub doc_type: String,
    pub score: f64,
    pub snippet: String,
}

/// Response body for `GET /api/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub mode: SearchMode,
    pub total: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_mode_default() {
        assert_eq!(SearchMode::default(), SearchMode::Semantic);
    }

    #[test]
    fn test_search_mode_serde() {
        assert_eq!(
            serde_json::to_string(&SearchMode::Graph).unwrap(),
            "\"graph\""
        );
        assert_eq!(
            serde_json::to_string(&SearchMode::Keyword).unwrap(),
            "\"keyword\""
        );
    }

    #[test]
    fn test_search_request_minimal() {
        let req = SearchRequest {
            q: "sync protocol".to_string(),
            mode: SearchMode::default(),
            context: None,
            doc_type: None,
            team: None,
            depth: None,
            limit: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("context"));
        assert!(!json.contains("team"));
    }

    #[test]
    fn test_search_result_serde() {
        let result = SearchResult {
            resource_id: Uuid::nil(),
            title: "R5 Design".to_string(),
            context: "temper".to_string(),
            doc_type: "research".to_string(),
            score: 0.92,
            snippet: "The sync protocol uses...".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: SearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "R5 Design");
        assert!((parsed.score - 0.92).abs() < f64::EPSILON);
    }
}
