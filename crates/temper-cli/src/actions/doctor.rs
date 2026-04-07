//! Doctor action — walks the vault, validates frontmatter, and reports issues.

use std::fs;
use std::path::Path;

use serde::Serialize;
use temper_core::schema::{
    check_legacy_fields, check_unknown_temper_fields, validate_frontmatter, ValidationIssue,
    ValidationResult,
};

use crate::actions::doctor_fix::{
    apply_manifest_actions, apply_plan, fix_filename, fix_manifest_for_moves, fix_missing_fields,
    fix_relocation, fix_stale_manifest_entries, ApplyReport, FixPlan,
};
use crate::config::Config;
use crate::error::Result;
use crate::manifest_io;
use crate::vault;

/// Aggregated report from a vault doctor scan.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub files_checked: u32,
    pub total_issues: u32,
    pub auto_fixable: u32,
    pub file_results: Vec<ValidationResult>,
}

/// Doc types whose files live at `{vault_root}/{context}/{doc_type}/`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

/// Scan the vault and validate all markdown frontmatter.
///
/// If `context_filter` is `Some`, only that context is scanned; otherwise all
/// contexts listed in `config.contexts` are scanned.
pub fn scan(config: &Config, context_filter: Option<&str>) -> Result<DoctorReport> {
    let mut file_results: Vec<ValidationResult> = Vec::new();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    // Walk standard entity doc type directories
    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts_to_scan {
            let dir = config.doc_type_dir(ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            scan_directory(&dir, doc_type, &mut file_results)?;
        }
    }

    // Walk research directory: {vault_root}/research/{context}/
    let research_root = config.vault_root.join("research");
    if research_root.is_dir() {
        for ctx in &contexts_to_scan {
            let dir = research_root.join(ctx);
            if !dir.is_dir() {
                continue;
            }
            scan_directory(&dir, "research", &mut file_results)?;
        }
    }

    // Compute summary counts
    let mut total_issues: u32 = 0;
    let mut auto_fixable: u32 = 0;
    for result in &file_results {
        for issue in &result.issues {
            total_issues += 1;
            if issue.auto_fixable {
                auto_fixable += 1;
            }
        }
    }

    Ok(DoctorReport {
        files_checked: file_results.len() as u32,
        total_issues,
        auto_fixable,
        file_results,
    })
}

/// Scan all `.md` files in `dir` using `dir_doc_type` as the fallback doc type.
fn scan_directory(
    dir: &Path,
    dir_doc_type: &str,
    results: &mut Vec<ValidationResult>,
) -> Result<()> {
    let md_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    for file_path in md_files {
        let result = scan_file(&file_path, dir_doc_type)?;
        results.push(result);
    }

    Ok(())
}

/// Validate a single markdown file and return a `ValidationResult`.
fn scan_file(file_path: &Path, dir_doc_type: &str) -> Result<ValidationResult> {
    let file_path_str = file_path.display().to_string();
    let content = fs::read_to_string(file_path)?;

    let fm = vault::parse_frontmatter(&content);

    let mut issues: Vec<ValidationIssue> = Vec::new();

    let Some(ref frontmatter) = fm else {
        issues.push(ValidationIssue {
            path: String::new(),
            message: "No YAML frontmatter found".to_string(),
            auto_fixable: false,
        });
        return Ok(ValidationResult {
            file_path: file_path_str,
            issues,
        });
    };

    // 1. Legacy field check
    issues.extend(check_legacy_fields(frontmatter));

    // 2. Detect effective doc type from frontmatter or directory name
    let effective_doc_type =
        detect_doc_type(frontmatter, dir_doc_type).unwrap_or_else(|| dir_doc_type.to_string());

    // 3. Schema validation (only for known doc types; skip unknown gracefully)
    match validate_frontmatter(&effective_doc_type, frontmatter) {
        Ok(schema_issues) => issues.extend(schema_issues),
        Err(e) => {
            // Unknown doc type or schema load failure — report as an issue
            issues.push(ValidationIssue {
                path: "temper-type".to_string(),
                message: format!("schema validation skipped: {e}"),
                auto_fixable: false,
            });
        }
    }

    // 4. Unknown temper-* fields
    issues.extend(check_unknown_temper_fields(frontmatter));

    Ok(ValidationResult {
        file_path: file_path_str,
        issues,
    })
}

/// Determine the effective doc type from frontmatter fields.
///
/// Priority: `temper-type` → `type` → `doc_type` → `dir_doc_type` fallback.
fn detect_doc_type(fm: &serde_yaml::Value, dir_doc_type: &str) -> Option<String> {
    for field in &["temper-type", "type", "doc_type"] {
        if let Some(val) = fm.get(*field).and_then(|v| v.as_str()) {
            return Some(val.to_string());
        }
    }
    Some(dir_doc_type.to_string())
}

/// Auto-fix issues in the vault using the FixAction pipeline.
///
/// Collects fix actions for all vault files, applies them in phase order,
/// and updates the manifest to reflect any file moves.
pub fn fix(config: &Config, context_filter: Option<&str>, dry_run: bool) -> Result<ApplyReport> {
    let mut plan = FixPlan::new();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    // Walk standard entity doc type directories
    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts_to_scan {
            let dir = config.doc_type_dir(ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            collect_fixes_for_directory(&dir, &config.vault_root, &mut plan)?;
        }
    }

    // Walk research directory: {vault_root}/research/{context}/
    let research_root = config.vault_root.join("research");
    if research_root.is_dir() {
        for ctx in &contexts_to_scan {
            let dir = research_root.join(ctx);
            if !dir.is_dir() {
                continue;
            }
            collect_fixes_for_directory(&dir, &config.vault_root, &mut plan)?;
        }
    }

    // F5: Manifest reconciliation
    let temper_dir = config.vault_root.join(".temper");
    let manifest_result = manifest_io::load_manifest(&temper_dir, "doctor-fix");
    if let Ok(manifest) = &manifest_result {
        let move_actions: Vec<_> = plan
            .actions
            .iter()
            .filter(|a| a.phase() == 1)
            .cloned()
            .collect();
        plan.extend(fix_manifest_for_moves(
            &move_actions,
            manifest,
            &config.vault_root,
        ));
        plan.extend(fix_stale_manifest_entries(manifest, &config.vault_root));
    }

    // Apply
    let report = apply_plan(&mut plan, dry_run)?;

    // Apply manifest changes, rehash modified files, and save
    if !dry_run {
        if let Ok(mut manifest) = manifest_result {
            apply_manifest_actions(&plan, &mut manifest);
            // Rehash all entries so body/managed/open hashes reflect
            // the content changes doctor fix just made.
            crate::actions::sync::rehash_manifest(&mut manifest, &config.vault_root)?;
            manifest_io::save_manifest(&temper_dir, &manifest)?;
        }
    }

    Ok(report)
}

/// Collect fix actions for all `.md` files in `dir`.
fn collect_fixes_for_directory(dir: &Path, vault_root: &Path, plan: &mut FixPlan) -> Result<()> {
    let md_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    for file_path in md_files {
        collect_fixes_for_file(&file_path, vault_root, plan)?;
    }

    Ok(())
}

/// Collect fix actions for a single file and add them to the plan.
fn collect_fixes_for_file(file_path: &Path, vault_root: &Path, plan: &mut FixPlan) -> Result<()> {
    let mut content = fs::read_to_string(file_path)?;

    // Pre-pass: if frontmatter fails to parse, attempt dedup and rewrite.
    if vault::parse_frontmatter(&content).is_none() {
        if let Some(deduped) = dedup_frontmatter_keys(&content) {
            fs::write(file_path, &deduped)?;
            content = deduped;
        }
    }

    let Some(fm) = vault::parse_frontmatter(&content) else {
        return Ok(());
    };
    plan.extend(fix_missing_fields(file_path, &fm, vault_root));
    plan.extend(fix_relocation(file_path, &fm, vault_root));
    plan.extend(fix_filename(file_path, &fm, vault_root));
    Ok(())
}

/// Deduplicate frontmatter keys, keeping the last occurrence of each key.
///
/// Returns `Some(new_content)` if duplicates were found, `None` if the content
/// has no frontmatter or no duplicates.
fn dedup_frontmatter_keys(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = &trimmed[3..];
    let end_pos = rest.find("---")?;
    let yaml_block = &rest[..end_pos];
    let after_fm = &rest[end_pos..];

    // Parse lines, track seen keys, keep last occurrence
    let lines: Vec<&str> = yaml_block.lines().collect();
    let mut seen_keys: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // First pass: find the last index for each key
    for (i, line) in lines.iter().enumerate() {
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim();
            if !key.is_empty() && !key.starts_with('#') {
                seen_keys.insert(key.to_string(), i);
            }
        }
    }

    // Check if any key appears more than once
    let mut key_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for line in &lines {
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim();
            if !key.is_empty() && !key.starts_with('#') {
                *key_counts.entry(key).or_insert(0) += 1;
            }
        }
    }
    let has_duplicates = key_counts.values().any(|&count| count > 1);
    if !has_duplicates {
        return None;
    }

    // Second pass: keep only lines where the key's last index matches current index
    let mut deduped_lines: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim();
            if !key.is_empty() && !key.starts_with('#') {
                if seen_keys.get(key) == Some(&i) {
                    deduped_lines.push(line);
                }
                continue;
            }
        }
        // Non-key lines (blank, comments) — keep them
        deduped_lines.push(line);
    }

    let new_yaml = deduped_lines.join("\n");
    Some(format!("---\n{new_yaml}\n{after_fm}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;

    fn yaml_val(s: &str) -> Value {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn detect_doc_type_prefers_temper_type() {
        let fm = yaml_val("temper-type: task\ntype: goal");
        assert_eq!(detect_doc_type(&fm, "session").unwrap(), "task");
    }

    #[test]
    fn detect_doc_type_falls_back_to_type() {
        let fm = yaml_val("type: goal");
        assert_eq!(detect_doc_type(&fm, "session").unwrap(), "goal");
    }

    #[test]
    fn detect_doc_type_falls_back_to_dir() {
        let fm = yaml_val("title: no type field here");
        assert_eq!(detect_doc_type(&fm, "session").unwrap(), "session");
    }

    #[test]
    fn dedup_frontmatter_keeps_last_occurrence() {
        let content = "---\ntemper-type: task\ntitle: First\ntemper-type: research\ntitle: Second\n---\nBody\n";
        let result = dedup_frontmatter_keys(content).unwrap();
        assert!(result.contains("temper-type: research"));
        assert!(result.contains("title: Second"));
        assert!(!result.contains("temper-type: task"));
        assert!(!result.contains("title: First"));
        assert!(result.contains("Body"));
    }

    #[test]
    fn dedup_frontmatter_returns_none_when_no_duplicates() {
        let content = "---\ntemper-type: task\ntitle: Only One\n---\nBody\n";
        assert!(dedup_frontmatter_keys(content).is_none());
    }

    #[test]
    fn dedup_frontmatter_returns_none_when_no_frontmatter() {
        let content = "Just some markdown\n";
        assert!(dedup_frontmatter_keys(content).is_none());
    }
}
