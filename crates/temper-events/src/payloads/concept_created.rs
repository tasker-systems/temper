use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConceptCreatedPayload {
    pub definition: String,
    pub elaboration: Option<String>,
}
