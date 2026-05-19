use serde::{Deserialize, Serialize};

/// Each field is optional: `None` means "no change on this field."
/// `Some("")` is a deliberate update to an empty string and is preserved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConceptMutatedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elaboration: Option<String>,
}
