//! Trait impls projecting `Frontmatter` to the typed structs in
//! `crate::types`: `ResourceRelationships`, `ManagedMeta`, `ResourceFrontmatter`.

use crate::error::{Result, TemperError};
use crate::frontmatter::document::Frontmatter;
use crate::types::graph::ResourceRelationships;
use crate::types::managed_meta::ManagedMeta;
use crate::types::vault::ResourceFrontmatter;

impl From<&Frontmatter> for ResourceRelationships {
    /// Project a [`Frontmatter`] to a [`ResourceRelationships`] by
    /// deserializing the open-tier JSON. Infallible because every
    /// field on `ResourceRelationships` is `#[serde(default)]`.
    fn from(fm: &Frontmatter) -> Self {
        let open = fm.open_json();
        serde_json::from_value(open).unwrap_or_default()
    }
}

impl TryFrom<&Frontmatter> for ManagedMeta {
    type Error = TemperError;

    fn try_from(fm: &Frontmatter) -> Result<Self> {
        let managed = fm.managed_json();
        serde_json::from_value(managed)
            .map_err(|e| TemperError::Config(format!("failed to project to ManagedMeta: {e}")))
    }
}

impl TryFrom<&Frontmatter> for ResourceFrontmatter {
    type Error = TemperError;

    fn try_from(fm: &Frontmatter) -> Result<Self> {
        // ResourceFrontmatter's serde mapping is NOT aligned with raw YAML
        // key names for most fields, so we build it field-by-field from the
        // top-level mapping.
        let managed = fm.managed_json();
        let title = managed
            .get("temper-title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TemperError::Config("ResourceFrontmatter requires `temper-title`".to_string())
            })?
            .to_string();
        let doc_type = fm.doc_type().as_str().to_string();

        // Context and created live in the top-level value (tier-1 system fields
        // don't make it into the managed tier).
        let mapping = fm
            .value()
            .as_mapping()
            .ok_or_else(|| TemperError::Config("frontmatter is not a mapping".to_string()))?;
        let context = mapping
            .get(serde_yaml::Value::String("temper-context".into()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TemperError::Config("ResourceFrontmatter requires `temper-context`".to_string())
            })?
            .to_string();
        let created_str = mapping
            .get(serde_yaml::Value::String("temper-created".into()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TemperError::Config("ResourceFrontmatter requires `temper-created`".to_string())
            })?;
        let created = chrono::DateTime::parse_from_rfc3339(created_str)
            .map_err(|e| TemperError::Config(format!("invalid temper-created: {e}")))?
            .with_timezone(&chrono::Utc);
        let temper_id_str = mapping
            .get(serde_yaml::Value::String("temper-id".into()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TemperError::Config("ResourceFrontmatter requires `temper-id`".to_string())
            })?;
        let temper_id = uuid::Uuid::parse_str(temper_id_str)
            .map_err(|e| TemperError::Config(format!("invalid temper-id uuid: {e}")))?;

        Ok(ResourceFrontmatter {
            temper_id,
            title,
            context,
            doc_type,
            ingestion_source: mapping
                .get(serde_yaml::Value::String("temper-source".into()))
                .and_then(|v| v.as_str())
                .map(String::from),
            created,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK_FIXTURE: &str = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
temper-title: My Task
slug: my-task
temper-stage: in-progress
temper-mode: build
temper-effort: small
temper-seq: 42
relates_to: [peer-a, peer-b]
depends_on: [dep-c]
parent: the-parent
tags: [auth, observability]
---
body
"#;

    #[test]
    fn projects_to_resource_relationships() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let rels = ResourceRelationships::from(&fm);
        assert_eq!(rels.relates_to, vec!["peer-a", "peer-b"]);
        assert_eq!(rels.depends_on, vec!["dep-c"]);
        assert_eq!(rels.parent.as_deref(), Some("the-parent"));
        // Tags are metadata, not a relationship — they do NOT land on the
        // typed struct. Session 2 removed `tags` from ResourceRelationships
        // to fix the phantom-edge bug; use `fm.tags()` to read them instead.
        assert_eq!(
            fm.tags(),
            vec!["auth".to_string(), "observability".to_string()]
        );
    }

    #[test]
    fn projects_to_managed_meta() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let mm = ManagedMeta::try_from(&fm).unwrap();
        assert_eq!(mm.title.as_deref(), Some("My Task"));
        assert_eq!(mm.slug.as_deref(), Some("my-task"));
        assert_eq!(mm.stage.as_deref(), Some("in-progress"));
        assert_eq!(mm.mode.as_deref(), Some("build"));
        assert_eq!(mm.seq, Some(42));
    }

    #[test]
    fn projects_to_resource_frontmatter() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let rf = ResourceFrontmatter::try_from(&fm).unwrap();
        assert_eq!(rf.title, "My Task");
        assert_eq!(rf.context, "temper");
        assert_eq!(rf.doc_type, "task");
    }

    #[test]
    fn projects_to_resource_frontmatter_fails_without_required_fields() {
        let input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
slug: t
---
"#;
        // Missing `title`, which ResourceFrontmatter requires.
        let fm = Frontmatter::try_from(input).unwrap();
        assert!(ResourceFrontmatter::try_from(&fm).is_err());
    }

    #[test]
    fn projection_of_empty_relationships_is_default() {
        let input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
slug: t
temper-stage: in-progress
---
"#;
        let fm = Frontmatter::try_from(input).unwrap();
        let rels = ResourceRelationships::from(&fm);
        assert!(rels.is_empty());
    }
}
