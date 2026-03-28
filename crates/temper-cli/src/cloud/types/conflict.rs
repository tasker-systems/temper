use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Metadata for an active conflict, stored in `.temper/conflicts/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub resource_id: Uuid,
    /// Path to the local version
    pub local_path: String,
    /// Path to the `.conflict.md` file
    pub conflict_path: String,
    /// Hash of local content at conflict detection time
    pub local_hash: String,
    /// Hash of remote content at conflict detection time
    pub remote_hash: String,
    /// When the conflict was detected
    pub detected_at: DateTime<Utc>,
}

/// A TEMPER-SYSTEM annotation block in a `.conflict.md` file.
///
/// Parsed by `temper merge` to produce the merged document with
/// section-level attribution headers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperSystemAnnotation {
    /// Email of the profile who made the change
    pub author_email: String,
    /// When the change was made
    pub modified_at: DateTime<Utc>,
    /// Event ID for traceability
    pub event_id: Uuid,
    /// The changed content between start and end markers
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_conflict_record_serde() {
        let record = ConflictRecord {
            resource_id: Uuid::nil(),
            local_path: "temper/research/sync.md".to_string(),
            conflict_path: "temper/research/sync.conflict.md".to_string(),
            local_hash: "sha256:aaa".to_string(),
            remote_hash: "sha256:bbb".to_string(),
            detected_at: Utc::now(),
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: ConflictRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.local_path, "temper/research/sync.md");
        assert_eq!(parsed.conflict_path, "temper/research/sync.conflict.md");
    }

    #[test]
    fn test_temper_system_annotation_serde() {
        let annotation = TemperSystemAnnotation {
            author_email: "pete@example.com".to_string(),
            modified_at: Utc::now(),
            event_id: Uuid::nil(),
            content: "The sync protocol uses a three-phase approach...".to_string(),
        };
        let json = serde_json::to_string(&annotation).unwrap();
        let parsed: TemperSystemAnnotation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.author_email, "pete@example.com");
    }
}
