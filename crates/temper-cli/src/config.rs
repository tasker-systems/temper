use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, TemperError};

// ---------------------------------------------------------------------------
// Raw TOML-deserialized structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VaultConfig {
    #[serde(default = "default_sessions")]
    pub sessions: String,
    #[serde(default = "default_tasks")]
    pub tasks: String,
    #[serde(default = "default_goals")]
    pub goals: String,
    #[serde(default = "default_templates")]
    pub templates: String,
    #[serde(default = "default_state_dir")]
    pub state_dir: String,
}

fn default_sessions() -> String {
    "sessions".to_string()
}
fn default_tasks() -> String {
    "tasks".to_string()
}
fn default_goals() -> String {
    "goals".to_string()
}
fn default_templates() -> String {
    "templates".to_string()
}
fn default_state_dir() -> String {
    ".temper".to_string()
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            sessions: default_sessions(),
            tasks: default_tasks(),
            goals: default_goals(),
            templates: default_templates(),
            state_dir: default_state_dir(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectConfig {
    pub repo: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillConfig {
    #[serde(default = "default_skill_output")]
    pub output: String,
    #[serde(default = "default_skill_framework")]
    pub framework: String,
}

fn default_skill_output() -> String {
    "~/.claude/commands/temper.md".to_string()
}
fn default_skill_framework() -> String {
    "superpowers".to_string()
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            output: default_skill_output(),
            framework: default_skill_framework(),
        }
    }
}

/// The top-level deserialized struct for temper.toml.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemperConfig {
    #[serde(default)]
    pub vault: VaultConfig,
    #[serde(default)]
    pub projects: HashMap<String, ProjectConfig>,
    #[serde(default)]
    pub skill: SkillConfig,
}

impl TemperConfig {
    /// Parse a temper.toml from the given file path.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            TemperError::Config(format!("cannot read {}: {}", path.as_ref().display(), e))
        })?;
        let cfg: TemperConfig = toml::from_str(&content)?;
        Ok(cfg)
    }
}

// ---------------------------------------------------------------------------
// Global config (~/.config/temper/config.toml)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct GlobalConfig {
    pub default_vault: Option<String>,
}

pub fn global_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("TEMPER_GLOBAL_CONFIG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    expand_tilde("~/.config/temper/config.toml")
}

fn load_global_config() -> GlobalConfig {
    let path = global_config_path();
    if !path.exists() {
        return GlobalConfig::default();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return GlobalConfig::default(),
    };
    toml::from_str(&content).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Resolved config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResolvedProject {
    pub name: String,
    pub repo: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub vault_root: PathBuf,
    pub sessions_dir: PathBuf,
    pub tasks_dir: PathBuf,
    pub goals_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub state_dir: PathBuf,
    pub projects: HashMap<String, ResolvedProject>,
    pub skill_output: PathBuf,
    pub skill_framework: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Expand a leading `~/` to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

/// 4-step vault resolution:
///   1. CLI --vault flag
///   2. TEMPER_VAULT env var
///   3. Walk up CWD looking for temper.toml
///   4. GlobalConfig default_vault
pub fn resolve_vault(cli_vault: Option<&str>) -> Result<PathBuf> {
    // Step 1: CLI flag
    if let Some(v) = cli_vault {
        let p = expand_tilde(v);
        return Ok(p);
    }

    // Step 2: env var
    if let Ok(v) = std::env::var("TEMPER_VAULT") {
        if !v.is_empty() {
            return Ok(PathBuf::from(v));
        }
    }

    // Step 3: walk up CWD
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            if dir.join("temper.toml").exists() {
                return Ok(dir.to_path_buf());
            }
            match dir.parent() {
                Some(p) => dir = p,
                None => break,
            }
        }
    }

    // Step 4: global config default_vault
    let global = load_global_config();
    if let Some(v) = global.default_vault {
        if !v.is_empty() {
            return Ok(expand_tilde(&v));
        }
    }

    Err(TemperError::VaultNotFound)
}

/// Resolve vault + parse temper.toml + resolve all paths into a `Config`.
pub fn load(cli_vault: Option<&str>) -> Result<Config> {
    let vault_root = resolve_vault(cli_vault)?;
    let toml_path = vault_root.join("temper.toml");
    let raw = TemperConfig::from_path(&toml_path)?;

    let join = |sub: &str| vault_root.join(sub);

    let projects = raw
        .projects
        .into_iter()
        .map(|(name, pc)| {
            let path = expand_tilde(&pc.path);
            (
                name.clone(),
                ResolvedProject {
                    name,
                    repo: pc.repo,
                    path,
                },
            )
        })
        .collect();

    Ok(Config {
        sessions_dir: join(&raw.vault.sessions),
        tasks_dir: join(&raw.vault.tasks),
        goals_dir: join(&raw.vault.goals),
        templates_dir: join(&raw.vault.templates),
        state_dir: join(&raw.vault.state_dir),
        vault_root,
        projects,
        skill_output: expand_tilde(&raw.skill.output),
        skill_framework: raw.skill.framework,
    })
}

/// Load the device UUID string from `~/.config/temper/device.json`.
///
/// Returns `None` when the file is absent or cannot be parsed.
pub fn load_device_id() -> Option<String> {
    let path = dirs::home_dir()?
        .join(".config")
        .join("temper")
        .join("device.json");
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val.get("client_id")?.as_str().map(String::from)
}

/// Safe-write protocol: read → validate original TOML → apply transform →
/// validate result → atomic write via tmp + rename.
pub fn safe_write<F>(path: &Path, transform: F) -> Result<()>
where
    F: FnOnce(String) -> String,
{
    let original = std::fs::read_to_string(path)
        .map_err(|e| TemperError::Config(format!("safe_write read error: {}", e)))?;

    // Validate original parses as TOML
    toml::from_str::<toml::Value>(&original)?;

    let transformed = transform(original);

    // Validate transformed parses as TOML
    toml::from_str::<toml::Value>(&transformed)?;

    // Atomic write: write to tmp, then rename
    let tmp_path = path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, &transformed)
        .map_err(|e| TemperError::Config(format!("safe_write tmp write error: {}", e)))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|e| TemperError::Config(format!("safe_write rename error: {}", e)))?;

    Ok(())
}
