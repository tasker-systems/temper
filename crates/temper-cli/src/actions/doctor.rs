//! Doctor action — walks the vault, validates frontmatter, and reports issues.

use std::fs;
use std::path::Path;

use serde::Serialize;
use temper_core::schema::{
    check_legacy_fields, check_unknown_temper_fields, validate_frontmatter, ValidationIssue,
    ValidationResult,
};
use temper_core::vault::Vault;

use crate::actions::doctor_fix::{
    apply_manifest_actions, apply_plan, fix_filename, fix_manifest_for_moves, fix_missing_fields,
    fix_missing_owner, fix_relocation, fix_stale_manifest_entries, ApplyReport, FixPlan,
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

/// Doc types whose files live at `{vault_root}/{owner}/{context}/{doc_type}/`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

/// Scan the vault and validate all markdown frontmatter.
///
/// If `context_filter` is `Some`, only that context is scanned; otherwise all
/// contexts listed in `config.contexts` are scanned.
pub fn scan(config: &Config, context_filter: Option<&str>) -> Result<DoctorReport> {
    let mut file_results: Vec<ValidationResult> = Vec::new();
    let vault_layout = Vault::new(&config.vault_root);

    // Load the manifest once so scan_directory can look up manifest_owner + is_provisional
    // for each file. If the manifest cannot be loaded (e.g. fresh vault), fall back
    // to treating every file as provisional.
    let temper_dir = config.vault_root.join(".temper");
    let manifest = manifest_io::load_manifest(&temper_dir, "doctor-scan").ok();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    // Walk every entity doc_type directory under its owner-scoped path.
    // research is included in ENTITY_DOC_TYPES — no special case required.
    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts_to_scan {
            let owner = config.owner_for_context(ctx);
            let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            scan_directory(
                &dir,
                doc_type,
                &mut file_results,
                &config.vault_root,
                manifest.as_ref(),
            )?;
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
    vault_root: &Path,
    manifest: Option<&temper_core::types::Manifest>,
) -> Result<()> {
    let md_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    for file_path in md_files {
        let rel = file_path
            .strip_prefix(vault_root)
            .unwrap_or(&file_path)
            .to_string_lossy()
            .to_string();

        let manifest_entry = manifest.and_then(|m| m.entries.values().find(|e| e.path == rel));

        let (manifest_owner, is_provisional) = match manifest_entry {
            Some(entry) => {
                let owner = Vault::parse_rel(&entry.path).map(|p| p.owner.to_string());
                (owner, entry.provisional)
            }
            None => (None, true),
        };

        let result = scan_file(
            &file_path,
            dir_doc_type,
            manifest_owner.as_deref(),
            is_provisional,
        )?;
        results.push(result);
    }

    Ok(())
}

/// Extract the `temper-owner` value from parsed frontmatter, if present.
fn extract_temper_owner(frontmatter: &serde_yaml::Value) -> Option<String> {
    frontmatter
        .get("temper-owner")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Validate owner pattern using shared logic from temper-core.
fn is_valid_owner_pattern(value: &str) -> bool {
    temper_core::validation::validate_owner_pattern(value).is_ok()
}

/// Validate a single markdown file and return a `ValidationResult`.
fn scan_file(
    file_path: &Path,
    dir_doc_type: &str,
    manifest_owner: Option<&str>,
    is_provisional: bool,
) -> Result<ValidationResult> {
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

    // 5. temper-owner validation
    {
        let owner_opt = extract_temper_owner(frontmatter);
        match owner_opt {
            None => {
                if is_provisional || manifest_owner.is_none() {
                    issues.push(ValidationIssue {
                        path: "temper-owner".to_string(),
                        message: "missing temper-owner (will default to @me on next sync)"
                            .to_string(),
                        auto_fixable: true,
                    });
                } else {
                    issues.push(ValidationIssue {
                        path: "temper-owner".to_string(),
                        message: "missing temper-owner on a synced file — run `temper sync run` to reconcile from server".to_string(),
                        auto_fixable: false,
                    });
                }
            }
            Some(ref value) if !is_valid_owner_pattern(value) => {
                issues.push(ValidationIssue {
                    path: "temper-owner".to_string(),
                    message: format!(
                        "invalid temper-owner pattern: {value} (expected @<slug> or +<slug>)"
                    ),
                    auto_fixable: false,
                });
            }
            Some(ref value) => {
                if let Some(expected) = manifest_owner {
                    if value != expected {
                        issues.push(ValidationIssue {
                            path: "temper-owner".to_string(),
                            message: format!(
                                "temper-owner ({value}) disagrees with manifest ({expected}) — ownership transfers require an explicit server action"
                            ),
                            auto_fixable: false,
                        });
                    }
                }
            }
        }
    } // end temper-owner validation block

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
    let vault_layout = Vault::new(&config.vault_root);

    // Load manifest once so the fix plan can determine per-file provisionality
    // (needed for fix_missing_owner) and later reconciliation can consume it.
    let temper_dir = config.vault_root.join(".temper");
    let manifest_result = manifest_io::load_manifest(&temper_dir, "doctor-fix");

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    // Walk every entity doc_type directory under its owner-scoped path.
    // research is included in ENTITY_DOC_TYPES — no special case required.
    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts_to_scan {
            let owner = config.owner_for_context(ctx);
            let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            collect_fixes_for_directory(
                &dir,
                &config.vault_root,
                &owner,
                manifest_result.as_ref().ok(),
                &mut plan,
            )?;
        }
    }

    // F5: Manifest reconciliation — manifest_result binding is still in scope.
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
fn collect_fixes_for_directory(
    dir: &Path,
    vault_root: &Path,
    owner: &str,
    manifest: Option<&temper_core::types::Manifest>,
    plan: &mut FixPlan,
) -> Result<()> {
    let md_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    for file_path in md_files {
        // Compute provisionality the same way scan_directory does.
        let rel = file_path
            .strip_prefix(vault_root)
            .unwrap_or(&file_path)
            .to_string_lossy()
            .to_string();

        let is_provisional = match manifest.and_then(|m| m.entries.values().find(|e| e.path == rel))
        {
            Some(entry) => entry.provisional,
            None => true, // files not in manifest are treated as provisional
        };

        collect_fixes_for_file(&file_path, vault_root, owner, is_provisional, plan)?;
    }

    Ok(())
}

/// Collect fix actions for a single file and add them to the plan.
fn collect_fixes_for_file(
    file_path: &Path,
    vault_root: &Path,
    owner: &str,
    is_provisional: bool,
    plan: &mut FixPlan,
) -> Result<()> {
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
    plan.extend(fix_missing_owner(file_path, &fm, is_provisional));
    plan.extend(fix_relocation(file_path, &fm, vault_root, owner));
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

#[cfg(test)]
mod owner_validation_tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_fixture(dir: &std::path::Path, rel: &str, frontmatter: &str) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = format!("---\n{frontmatter}\n---\n\n# body\n");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn missing_temper_owner_on_provisional_is_auto_fixable() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/p.md",
            "temper-type: task\ntitle: p\nslug: p",
        );
        let result = scan_file(&file, "task", None, true).unwrap();
        let owner_issue = result
            .issues
            .iter()
            .find(|i| i.path == "temper-owner")
            .unwrap();
        assert!(owner_issue.auto_fixable);
        assert!(owner_issue.message.contains("default to @me"));
    }

    #[test]
    fn missing_temper_owner_on_synced_is_warning_not_fixable() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/s.md",
            "temper-type: task\ntitle: s\nslug: s",
        );
        let result = scan_file(&file, "task", Some("@me"), false).unwrap();
        let owner_issue = result
            .issues
            .iter()
            .find(|i| i.path == "temper-owner")
            .unwrap();
        assert!(!owner_issue.auto_fixable);
        assert!(owner_issue.message.contains("run `temper sync run`"));
    }

    #[test]
    fn invalid_temper_owner_pattern_is_error() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/e.md",
            "temper-type: task\ntitle: e\nslug: e\ntemper-owner: \"not-a-sigil\"",
        );
        let result = scan_file(&file, "task", Some("@me"), false).unwrap();
        let owner_issue = result
            .issues
            .iter()
            .find(|i| i.path == "temper-owner")
            .unwrap();
        assert!(!owner_issue.auto_fixable);
        assert!(owner_issue.message.contains("invalid temper-owner pattern"));
    }

    #[test]
    fn directory_mismatch_warns_never_fixes() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/m.md",
            "temper-type: task\ntitle: m\nslug: m\ntemper-owner: \"+team\"",
        );
        let result = scan_file(&file, "task", Some("@me"), false).unwrap();
        let owner_issue = result
            .issues
            .iter()
            .find(|i| i.path == "temper-owner")
            .unwrap();
        assert!(!owner_issue.auto_fixable);
        assert!(owner_issue.message.contains("disagrees with manifest"));
    }

    #[test]
    fn valid_temper_owner_matching_manifest_emits_no_issue() {
        let tmp = TempDir::new().unwrap();
        let file = write_fixture(
            tmp.path(),
            "@me/temper/task/v.md",
            "temper-type: task\ntitle: v\nslug: v\ntemper-owner: \"@me\"",
        );
        let result = scan_file(&file, "task", Some("@me"), false).unwrap();
        let owner_issues: Vec<_> = result
            .issues
            .iter()
            .filter(|i| i.path == "temper-owner")
            .collect();
        assert!(owner_issues.is_empty(), "got {:?}", owner_issues);
    }
}
