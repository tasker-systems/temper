//! Doc-type-specific default values for managed metadata.

/// Apply doc-type-specific defaults to managed_meta.
/// Only sets fields that are absent — never overwrites caller-provided values.
pub fn apply_doc_type_defaults(doc_type: &str, meta: &mut serde_json::Value) {
    use serde_json::json;

    let obj = match meta.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    let now_date = chrono::Utc::now().format("%Y-%m-%d").to_string();

    match doc_type {
        "task" => {
            obj.entry("temper-stage")
                .or_insert_with(|| json!("backlog"));
        }
        "goal" => {
            obj.entry("temper-status")
                .or_insert_with(|| json!("active"));
        }
        "session" => {
            obj.entry("date").or_insert_with(|| json!(now_date));
        }
        "research" => {
            obj.entry("date").or_insert_with(|| json!(now_date));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_apply_doc_type_defaults_task() {
        let mut meta = json!({});
        apply_doc_type_defaults("task", &mut meta);
        assert_eq!(
            meta.get("temper-stage").and_then(|v| v.as_str()),
            Some("backlog")
        );
    }

    #[test]
    fn test_apply_doc_type_defaults_does_not_overwrite() {
        let mut meta = json!({"temper-stage": "in-progress"});
        apply_doc_type_defaults("task", &mut meta);
        assert_eq!(
            meta.get("temper-stage").and_then(|v| v.as_str()),
            Some("in-progress")
        );
    }

    #[test]
    fn test_apply_doc_type_defaults_goal() {
        let mut meta = json!({});
        apply_doc_type_defaults("goal", &mut meta);
        assert_eq!(
            meta.get("temper-status").and_then(|v| v.as_str()),
            Some("active")
        );
    }

    #[test]
    fn test_apply_doc_type_defaults_session() {
        let mut meta = json!({});
        apply_doc_type_defaults("session", &mut meta);
        assert!(
            meta.get("date").and_then(|v| v.as_str()).is_some(),
            "session should get a date default"
        );
    }

    #[test]
    fn test_apply_doc_type_defaults_research() {
        let mut meta = json!({});
        apply_doc_type_defaults("research", &mut meta);
        assert!(
            meta.get("date").and_then(|v| v.as_str()).is_some(),
            "research should get a date default"
        );
    }

    #[test]
    fn test_apply_doc_type_defaults_unknown_type_no_panic() {
        let mut meta = json!({});
        apply_doc_type_defaults("unknown-type", &mut meta);
        // should be unchanged
        assert!(meta.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_apply_doc_type_defaults_non_object_no_panic() {
        let mut meta = serde_json::Value::Null;
        apply_doc_type_defaults("task", &mut meta);
        // should not panic, meta remains Null
        assert!(meta.is_null());
    }
}
