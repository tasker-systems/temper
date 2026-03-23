use std::path::Path;

use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Generate the skill file content as a string.
pub fn generate(config: &Config) -> Result<String> {
    let toml_path = config.vault_root.join("temper.toml");
    let toml_content = std::fs::read_to_string(&toml_path)
        .map_err(|e| TemperError::Config(format!("cannot read temper.toml: {}", e)))?;
    let hash = format!("{:x}", Sha256::digest(toml_content.as_bytes()));

    let vault_path = config.vault_root.display().to_string();

    let mut project_lines = Vec::new();
    let mut sorted_projects: Vec<_> = config.projects.values().collect();
    sorted_projects.sort_by_key(|p| &p.name);
    for project in sorted_projects {
        project_lines.push(format!("- `{}` — {}", project.name, project.path.display()));
    }
    let project_list = if project_lines.is_empty() {
        "(no projects configured)".to_string()
    } else {
        project_lines.join("\n")
    };

    let content = format!(
        r#"<!-- config-hash: {hash} -->
---
name: temper
description: Knowledge vault operations — context lookup, session notes, ticket management, semantic search
---

# Temper — Vault Workflow Tool

Vault: {vault_path}

## Projects
{project_list}

## Commands

- `temper search <query>` — Semantic search across indexed content
- `temper context <topic>` — Show topic with related context
- `temper session save [<title>]` — Create/update session note
- `temper session list` — List recent sessions
- `temper ticket create --title <t> --project <p>` — Create ticket
- `temper ticket list` — List tickets
- `temper ticket board` — Board view
- `temper milestone list` — Roadmap view
- `temper note create <type> <title>` — Create note from template
- `temper index` — Rebuild search index
- `temper status` — Vault overview

## Workflow Integration

When starting a session:
- Check for recent sessions: `temper session list --project <current>`
- Search for relevant context: `temper search "<topic>"`

When ending a session:
- Suggest: `temper session save`

This tool uses the superpowers workflow: brainstorm → design → plan → implement → finish.
"#,
        hash = hash,
        vault_path = vault_path,
        project_list = project_list,
    );

    Ok(content)
}

/// Write the generated skill file to `output_path`, creating parent dirs as needed.
pub fn install(config: &Config, output_path: &Path) -> Result<()> {
    let content = generate(config)?;

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            TemperError::Config(format!(
                "cannot create directories for {}: {}",
                output_path.display(),
                e
            ))
        })?;
    }

    std::fs::write(output_path, &content).map_err(|e| {
        TemperError::Config(format!(
            "cannot write skill file to {}: {}",
            output_path.display(),
            e
        ))
    })?;

    Ok(())
}

/// Check skill installation status.
/// 1. Checks if superpowers plugin is installed.
/// 2. Checks if the skill file exists at the configured output path.
/// 3. If it exists, compares the embedded config hash to detect staleness.
pub fn check(config: &Config) -> Result<()> {
    // Check superpowers installation
    let superpowers_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
        .join(".claude/plugins/cache/claude-plugins-official/superpowers");

    if superpowers_path.exists() {
        println!("Superpowers: OK ({})", superpowers_path.display());
    } else {
        println!("Superpowers: NOT FOUND ({})", superpowers_path.display());
    }

    // Check skill file
    let skill_path = &config.skill_output;
    if !skill_path.exists() {
        println!("Skill file:  NOT FOUND ({})", skill_path.display());
        println!("  Run: temper skill install");
        return Ok(());
    }

    println!("Skill file:  OK ({})", skill_path.display());

    // Check for staleness by comparing hashes
    let existing = std::fs::read_to_string(skill_path)
        .map_err(|e| TemperError::Config(format!("cannot read skill file: {}", e)))?;

    let embedded_hash = extract_config_hash(&existing);

    // Compute current hash
    let toml_path = config.vault_root.join("temper.toml");
    let toml_content = std::fs::read_to_string(&toml_path)
        .map_err(|e| TemperError::Config(format!("cannot read temper.toml: {}", e)))?;
    let current_hash = format!("{:x}", Sha256::digest(toml_content.as_bytes()));

    match embedded_hash {
        Some(h) if h == current_hash => {
            println!("Hash:        OK (up to date)");
        }
        Some(h) => {
            println!("Hash:        STALE");
            println!("  Embedded: {}", h);
            println!("  Current:  {}", current_hash);
            println!("  Run: temper skill install");
        }
        None => {
            println!("Hash:        UNKNOWN (no config-hash comment found)");
        }
    }

    Ok(())
}

fn extract_config_hash(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("<!-- config-hash: ") {
            if let Some(hash) = rest.strip_suffix(" -->") {
                return Some(hash.to_string());
            }
        }
    }
    None
}
