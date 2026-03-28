use std::path::Path;
use std::process::Command;

use crate::config::{safe_write, Config};
use crate::error::{Result, TemperError};
use crate::output;

/// Add a project to temper.toml.
/// If `repo` is None, infer from `git -C <path> remote get-url origin`.
pub fn add(vault_root: &Path, name: &str, path: &str, repo: Option<&str>) -> Result<()> {
    let toml_path = vault_root.join("temper.toml");

    let resolved_repo = match repo {
        Some(r) => r.to_string(),
        None => {
            let output = Command::new("git")
                .args(["-C", path, "remote", "get-url", "origin"])
                .output()
                .map_err(|e| TemperError::Config(format!("failed to run git: {}", e)))?;
            if !output.status.success() {
                return Err(TemperError::Config(format!(
                    "could not infer repo from git remote: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };

    let block = format!(
        "\n[projects.{}]\nrepo = \"{}\"\npath = \"{}\"\n",
        name, resolved_repo, path
    );

    safe_write(&toml_path, |content| format!("{}{}", content, block))?;

    output::success(format!(
        "Added project '{}' (path={}, repo={})",
        name, path, resolved_repo
    ));
    Ok(())
}

/// Remove a project section from temper.toml.
pub fn remove(vault_root: &Path, name: &str) -> Result<()> {
    let toml_path = vault_root.join("temper.toml");
    let header = format!("[projects.{}]", name);

    safe_write(&toml_path, |content| {
        let mut result = String::new();
        let mut skip = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == header {
                skip = true;
                continue;
            }
            if skip && trimmed.starts_with('[') {
                // Next section starts — stop skipping
                skip = false;
            }
            if !skip {
                result.push_str(line);
                result.push('\n');
            }
        }

        result
    })?;

    output::success(format!("Removed project '{}'", name));
    Ok(())
}

/// List configured projects.
pub fn list(config: &Config) -> Result<()> {
    if config.projects.is_empty() {
        output::hint("No projects configured.");
        return Ok(());
    }

    let mut names: Vec<&String> = config.projects.keys().collect();
    names.sort();

    output::plain(format!("{:<20} {:<40} REPO", "NAME", "PATH"));
    output::dim("-".repeat(80));
    for name in names {
        let p = &config.projects[name];
        output::plain(format!("{:<20} {:<40} {}", name, p.path.display(), p.repo));
    }

    Ok(())
}
