use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Topic {
    pub id: Uuid,
    pub fqdn: String,
    pub parent_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}
