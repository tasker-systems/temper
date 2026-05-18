use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "porosity", rename_all = "lowercase")]
pub enum Porosity {
    Access,
    Attention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Scope {
    pub id: Uuid,
    pub name: String,
    pub porosity: Porosity,
    pub created_at: DateTime<Utc>,
}
