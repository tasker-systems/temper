//! Doctor action — walks the vault, validates frontmatter, and reports issues.

use std::fs;
use std::path::Path;

use serde::Serialize;
use temper_core::schema::{
    check_legacy_fields, check_unknown_temper_fields, validate_frontmatter, ValidationIssue,
    ValidationResult,
};

use crate::config::Config;
use crate::error::Result;
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
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept"];

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
}
