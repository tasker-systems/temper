use std::fs;
use std::path::Path;

use chrono::Local;

use crate::actions::types::NormalizeSummary;
use crate::config::Config;
use crate::discovery;
use crate::error::Result;
use crate::ids;
use crate::vault;

/// Old stage names that should be migrated to "in-progress".
const OLD_STAGES: &[&str] = &["brainstorm", "design", "plan", "implement"];

/// Doc types to scan for normalization.
const ENTITY_DOC_TYPES: &[&str] = &["task", "session", "goal"];

struct NormalizeOptions {
    dry_run: bool,
    fix_slugs: bool,
}

/// Run normalization across the vault and return a summary of changes.
///
/// Does not produce any output — callers are responsible for formatting the summary.
pub fn run(
    config: &Config,
    project: Option<&str>,
    dry_run: bool,
    fix_slugs: bool,
) -> Result<NormalizeSummary> {
    let mut summary = NormalizeSummary {
        ids_backfilled: 0,
        files_moved: 0,
        stages_migrated: 0,
        slugs_fixed: 0,
        frontmatter_fixed: 0,
        tasks_without_effort: 0,
    };

    let opts = NormalizeOptions { dry_run, fix_slugs };

    // For each doc type, scan across contexts
    for doc_type in ENTITY_DOC_TYPES {
        let contexts_to_scan: Vec<String> = if let Some(p) = project {
            vec![p.to_string()]
        } else {
            config.contexts.clone()
        };

        for ctx in &contexts_to_scan {
            let dir = config.doc_type_dir(ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            let md_files: Vec<_> = fs::read_dir(&dir)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
                .collect();

            for file_path in md_files {
                process_file(&opts, doc_type, &file_path, ctx, &mut summary)?;
            }
        }
    }

    // Also scan research directory
    let research_dir = config.vault_root.join("research");
    if research_dir.is_dir() {
        normalize_research_dir(&opts, &research_dir, project, &mut summary)?;
    }

    // Record event (skip in dry-run)
    if !dry_run
        && (summary.ids_backfilled
            + summary.files_moved
            + summary.stages_migrated
            + summary.slugs_fixed
            + summary.frontmatter_fixed
            > 0)
    {
        let event = discovery::Event::Normalize {
            ts: Local::now().to_rfc3339(),
            project: project.map(String::from),
            ids_backfilled: summary.ids_backfilled,
            files_moved: summary.files_moved,
            stages_migrated: summary.stages_migrated,
            slugs_fixed: summary.slugs_fixed,
            frontmatter_fixed: summary.frontmatter_fixed,
        };
        if let Err(e) = discovery::append_event(&config.state_dir, &event) {
            tracing::warn!("Failed to append discovery event: {e}");
        }
    }

    Ok(summary)
}

/// Scan research directory (which has its own subdirectory structure).
fn normalize_research_dir(
    opts: &NormalizeOptions,
    research_dir: &Path,
    filter_project: Option<&str>,
    summary: &mut NormalizeSummary,
) -> Result<()> {
    let proj_dirs: Vec<_> = fs::read_dir(research_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();

    for proj_dir in proj_dirs {
        let dir_context_name = proj_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        if let Some(fp) = filter_project {
            if dir_context_name != fp {
                continue;
            }
        }

        let md_files: Vec<_> = fs::read_dir(&proj_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
            .collect();

        for file_path in md_files {
            process_file(opts, "research", &file_path, &dir_context_name, summary)?;
        }
    }

    Ok(())
}

fn process_file(
    opts: &NormalizeOptions,
    doc_type: &str,
    file_path: &Path,
    dir_context_name: &str,
    summary: &mut NormalizeSummary,
) -> Result<()> {
    let content = fs::read_to_string(file_path)?;
    let mut modified = content.clone();
    let mut changed = false;

    let fm = vault::parse_frontmatter(&modified);

    // --- Check for missing `id` field ---
    let has_id = fm
        .as_ref()
        .and_then(|v| v.get("temper-id").or_else(|| v.get("id")))
        .is_some();

    if !has_id {
        // Derive a date from the slug prefix (YYYY-MM-DD) or frontmatter date field
        let date_str = extract_date_from_file(file_path, &fm);
        let new_id = ids::generate_id_from_date(date_str.as_deref().unwrap_or(""));

        // Insert id: "uuid" right after the opening ---\n
        if let Some(pos) = modified.find("---\n") {
            let insert_pos = pos + 4;
            modified.insert_str(insert_pos, &format!("id: \"{new_id}\"\n"));
            summary.ids_backfilled += 1;
            changed = true;
        }
    }

    // Re-parse frontmatter after potential modification
    let fm = vault::parse_frontmatter(&modified);

    // --- Check for old stage values ---
    if let Some(ref v) = fm {
        // Check temper-stage first, fall back to legacy stage
        let (stage_key, stage_val) = v
            .get("temper-stage")
            .and_then(|s| s.as_str())
            .map(|s| ("temper-stage", s))
            .or_else(|| {
                v.get("stage")
                    .and_then(|s| s.as_str())
                    .map(|s| ("stage", s))
            })
            .unzip();
        if let (Some(key), Some(val)) = (stage_key, stage_val) {
            if OLD_STAGES.contains(&val) {
                modified = vault::set_frontmatter_field(&modified, key, "in-progress");
                summary.stages_migrated += 1;
                changed = true;
            }
        }
    }

    // Re-parse frontmatter after stage migration
    let fm = vault::parse_frontmatter(&modified);

    // --- Check if file is in wrong context directory ---
    if let Some(ref v) = fm {
        let fm_context = v
            .get("temper-context")
            .or_else(|| v.get("context"))
            .and_then(|p| p.as_str());
        if let Some(fm_context) = fm_context {
            if fm_context != dir_context_name {
                // For the new layout, we'd need to know the base dir to move to.
                // Count it but don't auto-move (requires knowing full vault structure).
                summary.files_moved += 1;
            }
        }
    }

    // --- Check slug consistency ---
    if opts.fix_slugs {
        if let Some(ref v) = fm {
            if let Some(title) = v.get("title").and_then(|t| t.as_str()) {
                let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                // Strip date prefix if present (YYYY-MM-DD-)
                let slug_part = if stem.len() > 11 && stem.chars().nth(10) == Some('-') {
                    &stem[11..]
                } else {
                    stem
                };
                let expected_slug = vault::slugify(title);
                if slug_part != expected_slug {
                    summary.slugs_fixed += 1;
                    // Note: we count it but don't rename the file automatically
                    // (renaming would break references)
                }
            }
        }
    }

    // --- Backfill missing effort field on tasks ---
    if doc_type == "task" {
        if let Some(ref v) = fm {
            // Check if effort key exists at all (null counts as existing)
            let has_effort_key = v.get("temper-effort").is_some() || v.get("effort").is_some();
            if !has_effort_key {
                // Insert temper-effort: null after the temper-stage: (or stage:) line
                let mut new_lines = Vec::new();
                let mut in_fm = false;
                for line in modified.lines() {
                    new_lines.push(line.to_string());
                    if line.trim() == "---" {
                        in_fm = !in_fm;
                    } else if in_fm
                        && (line.starts_with("temper-stage:") || line.starts_with("stage:"))
                    {
                        new_lines.push("temper-effort: null".to_string());
                    }
                }
                modified = new_lines.join("\n") + "\n";
                summary.tasks_without_effort += 1;
                changed = true;
            }
        }
    }

    // --- Write changes if not dry-run ---
    if changed && !opts.dry_run {
        fs::write(file_path, &modified)?;
    }

    Ok(())
}

/// Extract a date string from the file path stem (YYYY-MM-DD prefix) or frontmatter.
pub fn extract_date_from_file(file_path: &Path, fm: &Option<serde_yaml::Value>) -> Option<String> {
    // Try frontmatter date field first
    if let Some(v) = fm {
        if let Some(date_str) = v.get("date").and_then(|d| d.as_str()) {
            if date_str.len() >= 10 {
                return Some(date_str[..10].to_string());
            }
        }
    }

    // Fall back to slug prefix YYYY-MM-DD
    let stem = file_path.file_stem()?.to_str()?;
    if stem.len() >= 10 {
        let prefix = &stem[..10];
        // Validate it looks like a date
        if prefix.chars().nth(4) == Some('-') && prefix.chars().nth(7) == Some('-') {
            return Some(prefix.to_string());
        }
    }

    None
}
