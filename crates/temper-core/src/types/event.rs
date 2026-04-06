use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ids::{EventId, ProfileId, ResourceId};

/// Query parameters for `GET /api/events`.
///
/// Events are scoped by time-bounded resource visibility: you see events on
/// resources visible to you, but only events that occurred after the resource
/// became visible to you. You always see events you generated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// An event returned from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventResponse {
    pub id: EventId,
    pub profile_id: ProfileId,
    pub device_id: String,
    pub context: Option<String>,
    pub resource_id: Option<ResourceId>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_query_minimal() {
        let query = EventQuery {
            since: None,
            context: None,
            resource_id: None,
            limit: Some(50),
        };
        let json = serde_json::to_string(&query).unwrap();
        assert!(!json.contains("since"));
        assert!(!json.contains("context"));
        assert!(json.contains("\"limit\":50"));
    }

    #[test]
    fn test_event_response_serde() {
        let event = EventResponse {
            id: EventId::from(Uuid::nil()),
            profile_id: ProfileId::from(Uuid::nil()),
            device_id: "device-abc".to_string(),
            context: Some("temper".to_string()),
            resource_id: Some(ResourceId::from(Uuid::nil())),
            event_type: "resource.modified".to_string(),
            payload: serde_json::json!({"content_hash": "sha256:abc"}),
            created: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EventResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, "resource.modified");
        assert_eq!(parsed.context, Some("temper".to_string()));
    }
}
