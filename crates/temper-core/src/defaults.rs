//! Doc-type-specific default values for managed and open metadata.
//!
//! Splits responsibility by tier: `apply_managed_defaults` populates
//! managed-tier defaults (`temper-stage`, `temper-status`); `apply_open_defaults`
//! populates open-tier defaults (`date` for session/research). Phase 6's
//! Migration A established that `date` belongs in open_meta — these helpers
//! enforce that split for new ingest writes too.

/// Apply doc-type-specific defaults to managed_meta.
/// Only sets fields that are absent — never overwrites caller-provided values.
pub fn apply_managed_defaults(doc_type: &str, meta: &mut serde_json::Value) {
    use serde_json::json;

    let obj = match meta.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    match doc_type {
        "task" => {
            obj.entry("temper-stage")
                .or_insert_with(|| json!("backlog"));
        }
        "goal" => {
            obj.entry("temper-status")
                .or_insert_with(|| json!("active"));
        }
        _ => {}
    }
}

/// Apply doc-type-specific defaults to open_meta.
/// Only sets fields that are absent — never overwrites caller-provided values.
pub fn apply_open_defaults(doc_type: &str, meta: &mut serde_json::Value) {
    use serde_json::json;

    let obj = match meta.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    let now_date = chrono::Utc::now().format("%Y-%m-%d").to_string();

    if matches!(doc_type, "session" | "research") {
        obj.entry("date").or_insert_with(|| json!(now_date));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn managed_defaults_set_temper_stage_for_task() {
        let mut meta = json!({});
        apply_managed_defaults("task", &mut meta);
        assert_eq!(
            meta.get("temper-stage").and_then(|v| v.as_str()),
            Some("backlog")
        );
    }

    #[test]
    fn managed_defaults_do_not_overwrite_existing_values() {
        let mut meta = json!({"temper-stage": "in-progress"});
        apply_managed_defaults("task", &mut meta);
        assert_eq!(
            meta.get("temper-stage").and_then(|v| v.as_str()),
            Some("in-progress")
        );
    }

    #[test]
    fn managed_defaults_set_temper_status_for_goal() {
        let mut meta = json!({});
        apply_managed_defaults("goal", &mut meta);
        assert_eq!(
            meta.get("temper-status").and_then(|v| v.as_str()),
            Some("active")
        );
    }

    #[test]
    fn managed_defaults_do_not_inject_date_for_session() {
        // Phase 6 / Migration A: `date` belongs in open_meta, not managed_meta.
        let mut meta = json!({});
        apply_managed_defaults("session", &mut meta);
        assert!(
            meta.get("date").is_none(),
            "`date` must not be injected into managed_meta; it's an open-tier default"
        );
    }

    #[test]
    fn managed_defaults_do_not_inject_date_for_research() {
        let mut meta = json!({});
        apply_managed_defaults("research", &mut meta);
        assert!(
            meta.get("date").is_none(),
            "`date` must not be injected into managed_meta; it's an open-tier default"
        );
    }

    #[test]
    fn open_defaults_set_date_for_session() {
        let mut meta = json!({});
        apply_open_defaults("session", &mut meta);
        assert!(
            meta.get("date").and_then(|v| v.as_str()).is_some(),
            "session should get a date default in open_meta"
        );
    }

    #[test]
    fn open_defaults_set_date_for_research() {
        let mut meta = json!({});
        apply_open_defaults("research", &mut meta);
        assert!(
            meta.get("date").and_then(|v| v.as_str()).is_some(),
            "research should get a date default in open_meta"
        );
    }

    #[test]
    fn open_defaults_do_not_overwrite_existing_date() {
        let mut meta = json!({"date": "2024-01-01"});
        apply_open_defaults("session", &mut meta);
        assert_eq!(
            meta.get("date").and_then(|v| v.as_str()),
            Some("2024-01-01")
        );
    }

    #[test]
    fn open_defaults_no_op_for_doctypes_without_open_defaults() {
        let mut meta = json!({});
        apply_open_defaults("task", &mut meta);
        assert!(meta.as_object().unwrap().is_empty());
        let mut meta = json!({});
        apply_open_defaults("goal", &mut meta);
        assert!(meta.as_object().unwrap().is_empty());
    }

    #[test]
    fn defaults_no_panic_on_unknown_type() {
        let mut meta = json!({});
        apply_managed_defaults("unknown-type", &mut meta);
        assert!(meta.as_object().unwrap().is_empty());
        apply_open_defaults("unknown-type", &mut meta);
        assert!(meta.as_object().unwrap().is_empty());
    }

    #[test]
    fn defaults_no_panic_on_non_object() {
        let mut meta = serde_json::Value::Null;
        apply_managed_defaults("task", &mut meta);
        assert!(meta.is_null());
        apply_open_defaults("session", &mut meta);
        assert!(meta.is_null());
    }
}
