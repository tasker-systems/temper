use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The source of an ingested (non-markdown) resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum IngestionSource {
    /// Local file extracted to markdown
    File { path: String },
    /// URL fetched and extracted to markdown
    Url { url: String },
}

/// YAML frontmatter injected into vault-managed markdown files.
///
/// This is the identity anchor — everything else (tags, behaviors, team
/// associations, access levels) lives in Postgres.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceFrontmatter {
    /// Resource UUID (UUIDv7, globally unique without coordination)
    #[serde(rename = "temper-id")]
    pub temper_id: Uuid,
    pub title: String,
    pub context: String,
    pub doc_type: String,
    /// Present only for ingested resources
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_source: Option<String>,
    pub created: DateTime<Utc>,
}

/// Result of a `temper vault add` operation for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultAddResult {
    pub resource_id: Uuid,
    pub vault_path: String,
    pub was_copied: bool,
    pub was_extracted: bool,
    pub source: Option<IngestionSource>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_ingestion_source_file_serde() {
        let source = IngestionSource::File {
            path: "/home/user/paper.pdf".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"type\":\"file\""));
        let parsed: IngestionSource = serde_json::from_str(&json).unwrap();
        match parsed {
            IngestionSource::File { path } => assert_eq!(path, "/home/user/paper.pdf"),
            _ => panic!("expected File variant"),
        }
    }

    #[test]
    fn test_ingestion_source_url_serde() {
        let source = IngestionSource::Url {
            url: "https://arxiv.org/abs/2401.12345".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"type\":\"url\""));
    }

    #[test]
    fn test_frontmatter_yaml_roundtrip() {
        let fm = ResourceFrontmatter {
            temper_id: Uuid::nil(),
            title: "Test Resource".to_string(),
            context: "temper".to_string(),
            doc_type: "research".to_string(),
            ingestion_source: None,
            created: Utc::now(),
        };
        let yaml = serde_yaml::to_string(&fm).unwrap();
        assert!(yaml.contains("temper-id:"));
        let parsed: ResourceFrontmatter = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.title, "Test Resource");
    }

    #[test]
    fn test_frontmatter_with_ingestion_source() {
        let fm = ResourceFrontmatter {
            temper_id: Uuid::nil(),
            title: "Imported Paper".to_string(),
            context: "temper".to_string(),
            doc_type: "source".to_string(),
            ingestion_source: Some("https://arxiv.org/abs/2401.12345".to_string()),
            created: Utc::now(),
        };
        let yaml = serde_yaml::to_string(&fm).unwrap();
        assert!(yaml.contains("ingestion_source:"));
    }

    #[test]
    fn test_vault_add_result_serde() {
        let result = VaultAddResult {
            resource_id: Uuid::nil(),
            vault_path: "temper/source/paper.md".to_string(),
            was_copied: true,
            was_extracted: true,
            source: Some(IngestionSource::File {
                path: "/tmp/paper.pdf".to_string(),
            }),
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: VaultAddResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.was_copied);
        assert!(parsed.was_extracted);
    }
}
