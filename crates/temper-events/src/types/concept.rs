use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Concept {
    pub id: Uuid,
    pub current_definition: String,
    pub current_elaboration: Option<String>,
    pub scope_id: Uuid,
    pub topic_id: Uuid,
    pub created_by_event_id: Uuid,
    pub last_event_id: Uuid,
    pub latest_event_recorded_at: DateTime<Utc>,
}
