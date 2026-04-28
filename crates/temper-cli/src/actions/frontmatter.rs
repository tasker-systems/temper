//! Typed builder for the managed_meta a templated resource starts with.
//!
//! Single source of truth — local-mode templated creators serialize the
//! returned struct to YAML for the file write; cloud-mode creators pass
//! it directly to `build_ingest_payload`.

use temper_core::types::ManagedMeta;

pub struct NewResourceArgs<'a> {
    pub doc_type: &'a str,
    pub context: &'a str,
    pub title: &'a str,
    // Task-specific (None for non-task types)
    pub mode: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub seq: Option<i64>,
    // Goal-specific
    pub status: Option<&'a str>,
    // Provenance (LLM-discovered vs user-created)
    pub provenance: Option<&'a str>,
    pub llm_model: Option<&'a str>,
    pub llm_run: Option<&'a str>,
}

pub fn build_managed_meta_for_create(args: NewResourceArgs<'_>) -> ManagedMeta {
    ManagedMeta {
        doc_type: Some(args.doc_type.to_string()),
        context: Some(args.context.to_string()),
        title: Some(args.title.to_string()),
        stage: args.stage.map(str::to_string),
        mode: args.mode.map(str::to_string),
        effort: args.effort.map(str::to_string),
        goal: args.goal.map(str::to_string),
        seq: args.seq,
        status: args.status.map(str::to_string),
        provenance: args.provenance.map(str::to_string),
        llm_model: args.llm_model.map(str::to_string),
        llm_run: args.llm_run.map(str::to_string),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_managed_meta_carries_task_fields() {
        let mm = build_managed_meta_for_create(NewResourceArgs {
            doc_type: "task",
            context: "temper",
            title: "Wire up cloud writes",
            mode: Some("build"),
            effort: Some("medium"),
            goal: Some("temper-cloud-portable-memory"),
            stage: Some("backlog"),
            seq: Some(60),
            status: None,
            provenance: None,
            llm_model: None,
            llm_run: None,
        });
        assert_eq!(mm.doc_type.as_deref(), Some("task"));
        assert_eq!(mm.mode.as_deref(), Some("build"));
        assert_eq!(mm.effort.as_deref(), Some("medium"));
        assert_eq!(mm.goal.as_deref(), Some("temper-cloud-portable-memory"));
        assert_eq!(mm.stage.as_deref(), Some("backlog"));
        assert_eq!(mm.seq, Some(60));
        assert!(mm.status.is_none());
    }

    #[test]
    fn goal_managed_meta_carries_status() {
        let mm = build_managed_meta_for_create(NewResourceArgs {
            doc_type: "goal",
            context: "temper",
            title: "Land cloud-first",
            mode: None,
            effort: None,
            goal: None,
            stage: None,
            seq: None,
            status: Some("active"),
            provenance: None,
            llm_model: None,
            llm_run: None,
        });
        assert_eq!(mm.doc_type.as_deref(), Some("goal"));
        assert_eq!(mm.status.as_deref(), Some("active"));
    }

    #[test]
    fn session_managed_meta_minimal() {
        let mm = build_managed_meta_for_create(NewResourceArgs {
            doc_type: "session",
            context: "temper",
            title: "2026-04-27 Session D",
            mode: None,
            effort: None,
            goal: None,
            stage: None,
            seq: None,
            status: None,
            provenance: None,
            llm_model: None,
            llm_run: None,
        });
        assert_eq!(mm.doc_type.as_deref(), Some("session"));
        assert_eq!(mm.context.as_deref(), Some("temper"));
        assert!(mm.stage.is_none());
        assert!(mm.status.is_none());
    }
}
