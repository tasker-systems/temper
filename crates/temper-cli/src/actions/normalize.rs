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

    let entity_base_dirs = [&config.tasks_dir, &config.sessions_dir, &config.goals_dir];

    for base_dir in entity_base_dirs {
        if !base_dir.is_dir() {
            continue;
        }
        normalize_directory(&opts, base_dir, project, &mut summary)?;
    }

    // Also scan research directory
    let research_dir = config.vault_root.join("research");
    if research_dir.is_dir() {
        normalize_directory(&opts, &research_dir, project, &mut summary)?;
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

fn normalize_directory(
    opts: &NormalizeOptions,
    base_dir: &Path,
    filter_project: Option<&str>,
    summary: &mut NormalizeSummary,
) -> Result<()> {
    // Collect context subdirectories
    let proj_dirs: Vec<_> = fs::read_dir(base_dir)?
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

        // If filtering by context, skip non-matching directories
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
            process_file(opts, base_dir, &file_path, &dir_context_name, summary)?;
        }
    }

    Ok(())
}

fn process_file(
    opts: &NormalizeOptions,
    base_dir: &Path,
    file_path: &Path,
    dir_context_name: &str,
    summary: &mut NormalizeSummary,
) -> Result<()> {
    let content = fs::read_to_string(file_path)?;
    let mut modified = content.clone();
    let mut changed = false;

    let fm = vault::parse_frontmatter(&modified);

    // --- Check for missing `id` field ---
    let has_id = fm.as_ref().and_then(|v| v.get("id")).is_some();

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
        if let Some(stage_val) = v.get("stage").and_then(|s| s.as_str()) {
            if OLD_STAGES.contains(&stage_val) {
                modified = vault::set_frontmatter_field(&modified, "stage", "in-progress");
                summary.stages_migrated += 1;
                changed = true;
            }
        }
    }

    // Re-parse frontmatter after stage migration
    let fm = vault::parse_frontmatter(&modified);

    // --- Check if file is in wrong context directory ---
    if let Some(ref v) = fm {
        if let Some(fm_context) = v.get("context").and_then(|p| p.as_str()) {
            if fm_context != dir_context_name {
                // Move file to correct context directory
                let correct_dir = base_dir.join(fm_context);
                let file_name = file_path.file_name().unwrap_or_default();
                let correct_path = correct_dir.join(file_name);

                if !opts.dry_run {
                    fs::create_dir_all(&correct_dir)?;
                    // Write any pending changes to the correct location
                    if changed {
                        fs::write(&correct_path, &modified)?;
                        // Remove the original
                        fs::remove_file(file_path)?;
                    } else {
                        fs::rename(file_path, &correct_path)?;
                    }
                    // File is now moved, no need to write again below
                    summary.files_moved += 1;
                    return Ok(());
                } else {
                    summary.files_moved += 1;
                }
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
    if base_dir.ends_with("tasks") {
        if let Some(ref v) = fm {
            // Check if effort key exists at all (null counts as existing)
            let has_effort_key = v.get("effort").is_some();
            if !has_effort_key {
                // Insert effort: null after the stage: line so set_frontmatter_field works later
                let mut new_lines = Vec::new();
                let mut in_fm = false;
                for line in modified.lines() {
                    new_lines.push(line.to_string());
                    if line.trim() == "---" {
                        in_fm = !in_fm;
                    } else if in_fm && line.starts_with("stage:") {
                        new_lines.push("effort: null".to_string());
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
