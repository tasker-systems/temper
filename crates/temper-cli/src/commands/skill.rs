use std::collections::HashMap;
use std::path::Path;

use askama::Template;
use sha2::{Digest, Sha256};

use crate::config::{self, Config};
use crate::error::{Result, TemperError};
use crate::output;
use crate::templates::{CommandWrapperTemplate, SkillTemplate};

// ── Static content (compiled into the binary) ────────────────────────────────

static REFERENCE_MD: &str = include_str!("../../skill-content/reference.md");
static SUBAGENT_GUIDANCE_MD: &str = include_str!("../../skill-content/subagent-guidance.md");
static SESSION_LIFECYCLE_MD: &str = include_str!("../../skill-content/session-lifecycle.md");
static WF_BUILD_SMALL: &str = include_str!("../../skill-content/workflows/build-small.md");
static WF_BUILD_MEDIUM: &str = include_str!("../../skill-content/workflows/build-medium.md");
static WF_BUILD_LARGE: &str = include_str!("../../skill-content/workflows/build-large.md");
static WF_PLAN_SMALL: &str = include_str!("../../skill-content/workflows/plan-small.md");
static WF_PLAN_MEDIUM: &str = include_str!("../../skill-content/workflows/plan-medium.md");
static WF_PLAN_LARGE: &str = include_str!("../../skill-content/workflows/plan-large.md");

// ── Public API ───────────────────────────────────────────────────────────────

/// Generate all skill files as a map of relative_path → content.
pub fn generate_skill_files(config: &Config) -> Result<HashMap<String, String>> {
    let hash = compute_config_hash()?;
    generate_skill_files_with_hash(config, &hash)
}

/// Backward-compatible: returns SKILL.md content for stdout preview.
pub fn generate(config: &Config) -> Result<String> {
    let files = generate_skill_files(config)?;
    Ok(files.get("SKILL.md").cloned().unwrap_or_default())
}

/// Install skill directory and command wrapper.
///
/// 1. Generate all skill files
/// 2. Write skill files (except command-wrapper.md) into `skill_dir`
/// 3. Write command-wrapper.md to `~/.claude/commands/temper.md`
pub fn install(config: &Config, skill_dir: &Path) -> Result<()> {
    let files = generate_skill_files(config)?;

    // Ensure skill_dir and subdirectories exist
    for sub in &["workflows", "guidance"] {
        let dir = skill_dir.join(sub);
        std::fs::create_dir_all(&dir).map_err(|e| {
            TemperError::Config(format!("cannot create directory {}: {}", dir.display(), e))
        })?;
    }

    // Write all files except command-wrapper.md into skill_dir
    for (rel_path, content) in &files {
        if rel_path == "command-wrapper.md" {
            continue;
        }
        let dest = skill_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TemperError::Config(format!(
                    "cannot create parent dir for {}: {}",
                    dest.display(),
                    e
                ))
            })?;
        }
        std::fs::write(&dest, content)
            .map_err(|e| TemperError::Config(format!("cannot write {}: {}", dest.display(), e)))?;
    }

    // Write command-wrapper.md to ~/.claude/commands/temper.md
    if let Some(wrapper_content) = files.get("command-wrapper.md") {
        let home = dirs::home_dir()
            .ok_or_else(|| TemperError::Config("cannot determine home directory".to_string()))?;
        let commands_dir = home.join(".claude/commands");
        std::fs::create_dir_all(&commands_dir).map_err(|e| {
            TemperError::Config(format!("cannot create {}: {}", commands_dir.display(), e))
        })?;
        let wrapper_path = commands_dir.join("temper.md");
        std::fs::write(&wrapper_path, wrapper_content).map_err(|e| {
            TemperError::Config(format!("cannot write {}: {}", wrapper_path.display(), e))
        })?;
    }

    Ok(())
}

/// Check skill installation status.
pub fn check(config: &Config) -> Result<()> {
    // 1. Check superpowers plugin (if framework == "superpowers")
    if config.skill_framework == "superpowers" {
        let superpowers_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~"))
            .join(".claude/plugins/cache/claude-plugins-official/superpowers");

        if superpowers_path.exists() {
            output::status_icon(true, format!("Superpowers: {}", superpowers_path.display()));
        } else {
            output::status_icon(
                false,
                format!("Superpowers: NOT FOUND ({})", superpowers_path.display()),
            );
        }
    }

    // 2. Check skill directory exists
    let skill_dir = &config.skill_output;
    if !skill_dir.exists() {
        output::status_icon(
            false,
            format!("Skill directory: NOT FOUND ({})", skill_dir.display()),
        );
        output::hint("  Run: temper skill install");
        return Ok(());
    }

    output::status_icon(true, format!("Skill directory: {}", skill_dir.display()));

    // 3. Check expected files
    let expected_files = [
        "SKILL.md",
        "reference.md",
        "subagent-guidance.md",
        "session-lifecycle.md",
        "workflows/build-small.md",
        "workflows/build-medium.md",
        "workflows/build-large.md",
        "workflows/plan-small.md",
        "workflows/plan-medium.md",
        "workflows/plan-large.md",
    ];

    let mut all_present = true;
    for file in &expected_files {
        let path = skill_dir.join(file);
        if !path.exists() {
            output::status_icon(false, format!("Missing: {}", file));
            all_present = false;
        }
    }
    if all_present {
        output::status_icon(
            true,
            format!("All {} skill files present", expected_files.len()),
        );
    }

    // 4. Check config hash staleness in SKILL.md
    let skill_md_path = skill_dir.join("SKILL.md");
    if skill_md_path.exists() {
        let existing = std::fs::read_to_string(&skill_md_path)
            .map_err(|e| TemperError::Config(format!("cannot read SKILL.md: {}", e)))?;

        let embedded_hash = extract_config_hash(&existing);
        let current_hash = compute_config_hash()?;

        match embedded_hash {
            Some(h) if h == current_hash => {
                output::status_icon(true, "Hash: up to date");
            }
            Some(h) => {
                output::status_icon(false, "Hash: STALE");
                output::plain(format!("  Embedded: {}", h));
                output::plain(format!("  Current:  {}", current_hash));
                output::hint("  Run: temper skill install");
            }
            None => {
                output::warning("Hash: UNKNOWN (no config-hash comment found)");
            }
        }
    }

    // 5. Check command wrapper at ~/.claude/commands/temper.md
    let wrapper_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
        .join(".claude/commands/temper.md");

    if wrapper_path.exists() {
        output::status_icon(true, format!("Command wrapper: {}", wrapper_path.display()));
    } else {
        output::status_icon(
            false,
            format!("Command wrapper: NOT FOUND ({})", wrapper_path.display()),
        );
        output::hint("  Run: temper skill install");
    }

    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Generate all skill files with a pre-computed config hash (for testability).
pub fn generate_skill_files_with_hash(
    config: &Config,
    hash: &str,
) -> Result<HashMap<String, String>> {
    let vault_path = config.vault_root.display().to_string();
    let context_list = format_context_list(&config.contexts);

    let skill_template = SkillTemplate {
        config_hash: hash,
        vault_path: &vault_path,
        context_list: &context_list,
    };

    let wrapper_template = CommandWrapperTemplate { config_hash: hash };

    let mut files = HashMap::new();

    files.insert(
        "SKILL.md".to_string(),
        skill_template
            .render()
            .map_err(|e| TemperError::Config(format!("template render error: {}", e)))?,
    );

    files.insert(
        "command-wrapper.md".to_string(),
        wrapper_template
            .render()
            .map_err(|e| TemperError::Config(format!("template render error: {}", e)))?,
    );

    files.insert("reference.md".to_string(), REFERENCE_MD.to_string());
    files.insert(
        "subagent-guidance.md".to_string(),
        SUBAGENT_GUIDANCE_MD.to_string(),
    );
    files.insert(
        "session-lifecycle.md".to_string(),
        SESSION_LIFECYCLE_MD.to_string(),
    );

    files.insert(
        "workflows/build-small.md".to_string(),
        WF_BUILD_SMALL.to_string(),
    );
    files.insert(
        "workflows/build-medium.md".to_string(),
        WF_BUILD_MEDIUM.to_string(),
    );
    files.insert(
        "workflows/build-large.md".to_string(),
        WF_BUILD_LARGE.to_string(),
    );
    files.insert(
        "workflows/plan-small.md".to_string(),
        WF_PLAN_SMALL.to_string(),
    );
    files.insert(
        "workflows/plan-medium.md".to_string(),
        WF_PLAN_MEDIUM.to_string(),
    );
    files.insert(
        "workflows/plan-large.md".to_string(),
        WF_PLAN_LARGE.to_string(),
    );

    Ok(files)
}

/// Compute SHA256 hash of the global config file.
fn compute_config_hash() -> Result<String> {
    let config_path = config::global_config_path();
    let config_content = std::fs::read_to_string(&config_path)
        .map_err(|e| TemperError::Config(format!("cannot read config: {}", e)))?;
    Ok(format!("{:x}", Sha256::digest(config_content.as_bytes())))
}

/// Format contexts as sorted markdown list items.
pub fn format_context_list(contexts: &[String]) -> String {
    if contexts.is_empty() {
        return "(no contexts configured)".to_string();
    }
    let mut sorted = contexts.to_vec();
    sorted.sort();
    sorted
        .iter()
        .map(|ctx| format!("- `{ctx}`"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the config hash from a `<!-- config-hash: ... -->` comment.
pub fn extract_config_hash(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("<!-- config-hash: ") {
            if let Some(hash) = rest.strip_suffix(" -->") {
                return Some(hash.to_string());
            }
        }
    }
    None
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    fn test_config() -> Config {
        Config {
            vault_root: PathBuf::from("/tmp/test-vault"),
            state_dir: PathBuf::from("/tmp/test-vault/.temper"),
            contexts: vec!["alpha".to_string(), "beta".to_string()],
            skill_output: PathBuf::from("/tmp/test-skill-output"),
            skill_framework: "superpowers".to_string(),
        }
    }

    #[test]
    fn test_generate_skill_files_contains_expected_keys() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();

        assert!(files.contains_key("SKILL.md"));
        assert!(files.contains_key("reference.md"));
        assert!(files.contains_key("subagent-guidance.md"));
        assert!(files.contains_key("session-lifecycle.md"));
        assert!(files.contains_key("workflows/build-small.md"));
        assert!(files.contains_key("workflows/build-medium.md"));
        assert!(files.contains_key("workflows/build-large.md"));
        assert!(files.contains_key("workflows/plan-small.md"));
        assert!(files.contains_key("workflows/plan-medium.md"));
        assert!(files.contains_key("workflows/plan-large.md"));
        assert!(files.contains_key("command-wrapper.md"));
    }

    #[test]
    fn test_generate_skill_md_contains_vault_and_contexts() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
        let skill_md = &files["SKILL.md"];

        assert!(skill_md.contains("/tmp/test-vault"));
        assert!(skill_md.contains("alpha"));
        assert!(skill_md.contains("beta"));
        assert!(skill_md.contains("config-hash: testhash"));
    }

    #[test]
    fn test_generate_command_wrapper_contains_hash() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
        let wrapper = &files["command-wrapper.md"];

        assert!(wrapper.contains("config-hash: testhash"));
        assert!(wrapper.contains("Invoke the temper skill"));
    }

    #[test]
    fn test_format_context_list_sorted() {
        let contexts = vec![
            "zebra".to_string(),
            "alpha".to_string(),
            "middle".to_string(),
        ];
        let result = format_context_list(&contexts);
        assert!(result.starts_with("- `alpha`"));
        assert!(result.contains("- `middle`"));
        assert!(result.ends_with("- `zebra`"));
    }

    #[test]
    fn test_format_context_list_empty() {
        let result = format_context_list(&[]);
        assert_eq!(result, "(no contexts configured)");
    }

    #[test]
    fn test_extract_config_hash_found() {
        let content = "<!-- config-hash: abc123 -->\n---\nname: temper\n---";
        assert_eq!(extract_config_hash(content), Some("abc123".to_string()));
    }

    #[test]
    fn test_extract_config_hash_not_found() {
        let content = "---\nname: temper\n---";
        assert_eq!(extract_config_hash(content), None);
    }
}
