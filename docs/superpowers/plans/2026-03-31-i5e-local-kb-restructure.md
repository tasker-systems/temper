# I5e — Local KB Restructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify config into `~/.config/temper/config.toml`, invert vault layout to `{context}/{doc_type}/{slug}.md`, and validate with a trial import+sync.

**Architecture:** Single global config replaces the split global+vault config model. `Config` struct loses per-doc-type directory fields; all vault paths are computed from `vault_root/context/doc_type`. The `projects` map and CWD-based context resolution are removed — contexts come from `sync.subscriptions.contexts`.

**Tech Stack:** Rust, toml, serde, sha2, clap

---

## File Structure

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `crates/temper-core/src/types/config.rs` | New unified config types (`SyncAutoConfig`, `SyncSubscriptionsConfig`, revised `SyncConfig`) |
| Modify | `crates/temper-cli/src/config.rs` | New `GlobalConfig` with all sections, simplified `Config`, updated `load()` and `resolve_vault()` |
| Modify | `crates/temper-cli/src/commands/init.rs` | Create `.temper/{manifest.json,events.jsonl}`, write full default global config |
| Modify | `crates/temper-cli/src/commands/skill.rs` | Read from global config, contexts from subscriptions |
| Modify | `crates/temper-cli/src/commands/context_cmd.rs` | Add/remove/list contexts via global config `sync.subscriptions.contexts` |
| Modify | `crates/temper-cli/src/commands/session.rs` | Use `vault_root/context/session/` paths |
| Modify | `crates/temper-cli/src/commands/status.rs` | Scan `vault_root/{context}/` dirs instead of per-type dirs |
| Modify | `crates/temper-cli/src/commands/check.rs` | Check `.temper/` dir and global config instead of `temper.toml` + type dirs |
| Modify | `crates/temper-cli/src/commands/warmup.rs` | Use `vault_root/context/session/` path |
| Modify | `crates/temper-cli/src/commands/note.rs` | Use embedded templates, write to `vault_root/{context}/{note_type}/` |
| Modify | `crates/temper-cli/src/commands/research.rs` | Use `vault_root/context/research/` path |
| Modify | `crates/temper-cli/src/actions/task.rs` | Use `vault_root/context/task/` path |
| Modify | `crates/temper-cli/src/actions/goal.rs` | Use `vault_root/context/goal/` path |
| Modify | `crates/temper-cli/src/actions/normalize.rs` | Scan `vault_root/{context}/{doc_type}/` dirs |
| Modify | `crates/temper-cli/src/actions/ingest.rs` | `build_vault_path` uses slug instead of UUID |
| Modify | `crates/temper-cli/src/vault.rs` | `render_template` uses embedded templates only |
| Modify | `crates/temper-cli/src/main.rs` | Remove `resolve_from_cwd` calls, simplify context resolution |
| Modify | `crates/temper-cli/src/project.rs` | Remove or gut — context resolution no longer path-based |
| Modify | `crates/temper-cli/src/cli.rs` | Update context_cmd subcommands (remove `--path`, `--repo`) |

---

### Task 1: Update temper-core config types

**Files:**
- Modify: `crates/temper-core/src/types/config.rs`

The core config types need to match the new unified TOML shape. The old `SyncSubscription` array-of-tables model is replaced by flat section models.

- [ ] **Step 1: Write tests for new config shape**

Add tests to `crates/temper-core/src/types/config.rs`:

```rust
#[test]
fn test_unified_config_toml_roundtrip() {
    let toml_str = r#"
[vault]
path = "~/projects/kb-vault"

[sync.auto]
doctypes = ["task", "goal", "session"]

[sync.subscriptions]
contexts = ["temper", "storyteller", "tasker", "writing"]

[cli]
progress = "bar"

[skill]
output = "~/.claude/commands/temper.md"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
"#;
    let config: UnifiedConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.vault.path, "~/projects/kb-vault");
    assert_eq!(config.sync.auto.doctypes, vec!["task", "goal", "session"]);
    assert_eq!(config.sync.subscriptions.contexts, vec!["temper", "storyteller", "tasker", "writing"]);
    assert_eq!(config.cli.progress, "bar");
    assert_eq!(config.skill.output, "~/.claude/commands/temper.md");
    assert_eq!(config.skill.framework, "superpowers");
    assert_eq!(config.auth.provider, "auth0");
    assert!(config.auth.providers.contains_key("auth0"));
    let auth0 = &config.auth.providers["auth0"];
    assert_eq!(auth0.client_id, "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF");
    assert_eq!(auth0.scopes, vec!["openid", "profile", "email", "offline_access"]);
}

#[test]
fn test_unified_config_minimal() {
    let toml_str = r#"
[vault]
path = "~/vault"
"#;
    let config: UnifiedConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.vault.path, "~/vault");
    assert!(config.sync.auto.doctypes.is_empty());
    assert!(config.sync.subscriptions.contexts.is_empty());
    assert_eq!(config.cli.progress, "bar");
    assert_eq!(config.skill.output, "~/.claude/commands/temper.md");
    assert_eq!(config.skill.framework, "superpowers");
    assert_eq!(config.auth.provider, "auth0");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo test -p temper-core test_unified_config`
Expected: compilation errors — `UnifiedConfig` doesn't exist yet

- [ ] **Step 3: Implement new config types**

In `crates/temper-core/src/types/config.rs`, add the new types. Keep the old types for now (they'll be removed when CLI migrates):

```rust
/// Auto-sync configuration — which doc types trigger auto-sync on create/update.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncAutoConfig {
    #[serde(default)]
    pub doctypes: Vec<String>,
}

/// Sync subscriptions — which contexts are synced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSubscriptionsConfig {
    #[serde(default)]
    pub contexts: Vec<String>,
}

/// New sync config with auto + subscriptions sub-sections.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnifiedSyncConfig {
    #[serde(default)]
    pub auto: SyncAutoConfig,
    #[serde(default)]
    pub subscriptions: SyncSubscriptionsConfig,
}

/// Skill generation config.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Auth provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProviderConfig {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub audience: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Auth configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_auth_provider")]
    pub provider: String,
    #[serde(default)]
    pub providers: std::collections::HashMap<String, AuthProviderConfig>,
}

fn default_auth_provider() -> String {
    "auth0".to_string()
}

impl Default for AuthConfig {
    fn default() -> Self {
        let mut providers = std::collections::HashMap::new();
        providers.insert("auth0".to_string(), AuthProviderConfig {
            authorize_url: "https://temperkb.us.auth0.com/authorize".to_string(),
            token_url: "https://temperkb.us.auth0.com/oauth/token".to_string(),
            client_id: "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF".to_string(),
            audience: "https://temperkb.io/api".to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ],
        });
        Self {
            provider: default_auth_provider(),
            providers,
        }
    }
}

/// Unified config — `~/.config/temper/config.toml`.
///
/// Single config file replacing the old split model (global config + vault temper.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedConfig {
    pub vault: CloudVaultConfig,
    #[serde(default)]
    pub sync: UnifiedSyncConfig,
    #[serde(default)]
    pub cli: CliConfig,
    #[serde(default)]
    pub skill: SkillConfig,
    #[serde(default)]
    pub auth: AuthConfig,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo test -p temper-core test_unified_config`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/config.rs
git commit -m "feat(core): add UnifiedConfig types for single-file config model"
```

---

### Task 2: Rewrite config.rs — GlobalConfig and Config

**Files:**
- Modify: `crates/temper-cli/src/config.rs`

Replace the split config model. `GlobalConfig` becomes the primary struct (deserializing the unified TOML). `Config` loses `sessions_dir`/`tasks_dir`/`goals_dir`/`templates_dir`/`projects` and gains `contexts` + a helper method for path computation.

- [ ] **Step 1: Rewrite GlobalConfig and Config structs**

Replace the existing `GlobalConfig`, `TemperConfig`, `VaultConfig`, `ProjectConfig`, `SkillConfig`, and `ResolvedProject` with:

```rust
use temper_core::types::config::UnifiedConfig;

/// Deserialized global config from ~/.config/temper/config.toml.
/// Re-export for convenience — the actual struct lives in temper-core.
pub type GlobalConfig = UnifiedConfig;

/// Resolved runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub vault_root: PathBuf,
    pub state_dir: PathBuf,
    pub contexts: Vec<String>,
    pub skill_output: PathBuf,
    pub skill_framework: String,
}

impl Config {
    /// Compute the directory for a given context + doc_type.
    /// Returns `vault_root/{context}/{doc_type}/`
    pub fn doc_type_dir(&self, context: &str, doc_type: &str) -> PathBuf {
        self.vault_root.join(context).join(doc_type)
    }
}
```

- [ ] **Step 2: Rewrite global_config_path()**

```rust
pub fn global_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("TEMPER_GLOBAL_CONFIG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    expand_tilde("~/.config/temper/config.toml")
}
```

(No change — keep `TEMPER_GLOBAL_CONFIG`.)

- [ ] **Step 3: Rewrite load_global_config()**

```rust
pub fn load_global_config() -> Result<GlobalConfig> {
    let path = global_config_path();
    if !path.exists() {
        return Err(TemperError::Config(format!(
            "global config not found: {}. Run 'temper init' first.",
            path.display()
        )));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| TemperError::Config(format!("cannot read {}: {}", path.display(), e)))?;
    let cfg: GlobalConfig = toml::from_str(&content)?;
    Ok(cfg)
}
```

Make this `pub` — it's needed by `init` and `context_cmd`.

- [ ] **Step 4: Rewrite resolve_vault() — 3-step, no walk-up**

```rust
/// 3-step vault resolution:
///   1. CLI --vault flag
///   2. TEMPER_VAULT env var
///   3. Global config [vault].path
pub fn resolve_vault(cli_vault: Option<&str>) -> Result<PathBuf> {
    if let Some(v) = cli_vault {
        return Ok(expand_tilde(v));
    }

    if let Ok(v) = std::env::var("TEMPER_VAULT") {
        if !v.is_empty() {
            return Ok(expand_tilde(&v));
        }
    }

    let global = load_global_config()?;
    Ok(expand_tilde(&global.vault.path))
}
```

- [ ] **Step 5: Rewrite load()**

```rust
/// Load the resolved Config from the global config file.
pub fn load(cli_vault: Option<&str>) -> Result<Config> {
    let global = load_global_config()?;

    let vault_root = if let Some(v) = cli_vault {
        expand_tilde(v)
    } else if let Ok(v) = std::env::var("TEMPER_VAULT") {
        if !v.is_empty() {
            expand_tilde(&v)
        } else {
            expand_tilde(&global.vault.path)
        }
    } else {
        expand_tilde(&global.vault.path)
    };

    Ok(Config {
        state_dir: vault_root.join(".temper"),
        vault_root,
        contexts: global.sync.subscriptions.contexts.clone(),
        skill_output: expand_tilde(&global.skill.output),
        skill_framework: global.skill.framework.clone(),
    })
}
```

- [ ] **Step 6: Remove old types**

Delete: `VaultConfig`, `ProjectConfig`, `SkillConfig` (the CLI-local one), `TemperConfig`, `ResolvedProject`, and all `default_*` functions for sessions/tasks/goals/templates/state_dir/skill_output/skill_framework.

Keep: `expand_tilde`, `global_config_path`, `load_device_id`, `safe_write`.

- [ ] **Step 7: Verify compilation**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli 2>&1 | head -60`
Expected: Many compilation errors in downstream files — that's expected, we fix them in subsequent tasks.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/config.rs
git commit -m "feat(cli): rewrite config to use unified GlobalConfig from temper-core"
```

---

### Task 3: Update vault.rs — embedded templates only

**Files:**
- Modify: `crates/temper-cli/src/vault.rs`

Templates are now always embedded. Remove the `templates_dir` parameter from `render_template` and `render_template_with_vars`.

- [ ] **Step 1: Simplify render_template**

Replace the current `render_template` function:

```rust
/// Read an embedded template, fill in {{date}} and {{title}}.
pub fn render_template(note_type: &str, title: &str) -> Result<String> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let content = embedded_template(note_type)
        .ok_or_else(|| TemperError::Vault(format!("No template found for note type '{note_type}'")))?;

    Ok(content
        .replace("{{date}}", &today)
        .replace("{{title}}", title))
}
```

- [ ] **Step 2: Simplify render_template_with_vars**

```rust
pub fn render_template_with_vars(
    note_type: &str,
    title: &str,
    vars: &[(&str, &str)],
) -> Result<String> {
    let mut content = render_template(note_type, title)?;
    for (key, value) in vars {
        content = content.replace(&format!("{{{{{key}}}}}"), value);
    }
    Ok(content)
}
```

- [ ] **Step 3: Verify compilation of vault.rs**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli 2>&1 | head -40`
Expected: Errors in callers of `render_template` — fixed in subsequent tasks.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/vault.rs
git commit -m "refactor(cli): simplify template rendering to embedded-only"
```

---

### Task 4: Update actions/ingest.rs — slug-based vault paths

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs`

Change `build_vault_path` to use a slug (from title) instead of UUID for filenames.

- [ ] **Step 1: Update build_vault_path signature and implementation**

```rust
/// Canonical vault path for a managed resource.
///
/// `{vault_root}/{context}/{doc_type}/{slug}.md`
pub fn build_vault_path(vault_root: &Path, context: &str, doc_type: &str, slug: &str) -> PathBuf {
    vault_root
        .join(context)
        .join(doc_type)
        .join(format!("{slug}.md"))
}
```

- [ ] **Step 2: Add slugify_title helper**

```rust
/// Generate a slug from a title, suitable for filenames.
pub fn slugify_title(title: &str) -> String {
    crate::vault::slugify(title)
}
```

- [ ] **Step 3: Update write_vault_file_and_register to use slug**

In `write_vault_file_and_register`, change the `build_vault_path` call:

```rust
pub fn write_vault_file_and_register(
    vault_root: &Path,
    context: &str,
    doc_type: &str,
    resource: &temper_core::types::ResourceRow,
    content: &str,
    ingestion_source: Option<&str>,
) -> Result<PathBuf> {
    let slug = slugify_title(&resource.title);
    let vault_path = build_vault_path(vault_root, context, doc_type, &slug);
    // ... rest unchanged
```

- [ ] **Step 4: Update tests for build_vault_path**

Replace the UUID-based tests:

```rust
#[test]
fn build_vault_path_produces_correct_path() {
    let root = Path::new("/vault");
    let path = build_vault_path(root, "work", "note", "my-document");
    assert_eq!(path, PathBuf::from("/vault/work/note/my-document.md"));
}

#[test]
fn build_vault_path_nested_context() {
    let root = Path::new("/home/user/kb");
    let path = build_vault_path(root, "personal", "resource", "research-paper");
    assert_eq!(
        path,
        PathBuf::from("/home/user/kb/personal/resource/research-paper.md")
    );
}
```

- [ ] **Step 5: Run tests**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo test -p temper-cli ingest -- --test-threads=1`
Expected: PASS for pure function tests

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs
git commit -m "refactor(cli): use slug-based vault paths instead of UUIDs"
```

---

### Task 5: Update actions/task.rs — new vault paths

**Files:**
- Modify: `crates/temper-cli/src/actions/task.rs`

Replace all `config.tasks_dir` references with `config.doc_type_dir(context, "task")`. Remove `templates_dir_str` helper. Update `render_template_with_vars` calls to new signature.

- [ ] **Step 1: Update load_tasks**

```rust
pub fn load_tasks(
    config: &Config,
    context: Option<&str>,
    goal_slug: Option<&str>,
) -> Result<Vec<TaskInfo>> {
    let mut tasks = Vec::new();
    let dirs: Vec<_> = if let Some(p) = context {
        let d = config.doc_type_dir(p, "task");
        if d.is_dir() { vec![d] } else { vec![] }
    } else {
        // Scan all contexts
        config.contexts.iter()
            .map(|c| config.doc_type_dir(c, "task"))
            .filter(|d| d.is_dir())
            .collect()
    };
    // ... rest of iteration logic unchanged
```

- [ ] **Step 2: Update create**

Replace `templates_dir_str` usage and `config.tasks_dir` with direct calls:

```rust
pub fn create(
    config: &Config,
    context: &str,
    title: &str,
    goal_slug: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
) -> Result<String> {
    // ... validation unchanged ...

    let mut content =
        vault::render_template_with_vars("task", title, &vars)?;

    if let Some(stdin_content) = vault::read_stdin_if_piped() {
        content.push_str(&stdin_content);
        content.push('\n');
    }

    let dir = config.doc_type_dir(context, "task");
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    let path = dir.join(format!("{slug}.md"));
    vault::write_note(&path, &content)?;
    // ... event logging unchanged
```

- [ ] **Step 3: Update move_task**

Replace `config.tasks_dir.join(&task.context)` with `config.doc_type_dir(&task.context, "task")`:

```rust
    let path = config
        .doc_type_dir(&task.context, "task")
        .join(format!("{}.md", task.slug));
```

- [ ] **Step 4: Update done**

Same pattern:

```rust
    let path = config
        .doc_type_dir(&task.context, "task")
        .join(format!("{}.md", task.slug));
```

- [ ] **Step 5: Delete templates_dir_str helper**

Remove the `templates_dir_str` function entirely.

- [ ] **Step 6: Verify compilation**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli 2>&1 | head -40`

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/task.rs
git commit -m "refactor(cli): update task actions to use context-first vault paths"
```

---

### Task 6: Update actions/goal.rs — new vault paths

**Files:**
- Modify: `crates/temper-cli/src/actions/goal.rs`

Replace all `config.goals_dir` and `config.tasks_dir` references. Update `render_template_with_vars` calls.

- [ ] **Step 1: Update load_goals**

```rust
pub fn load_goals(config: &Config, context: Option<&str>) -> Result<Vec<GoalInfo>> {
    let mut goals = Vec::new();
    let dirs: Vec<_> = if let Some(p) = context {
        let d = config.doc_type_dir(p, "goal");
        if d.is_dir() { vec![d] } else { vec![] }
    } else {
        config.contexts.iter()
            .map(|c| config.doc_type_dir(c, "goal"))
            .filter(|d| d.is_dir())
            .collect()
    };
    // ... rest unchanged
```

- [ ] **Step 2: Update ensure_maintenance**

```rust
pub fn ensure_maintenance(config: &Config, context: &str) -> Result<String> {
    let slug = format!("{context}-maintenance");
    let dir = config.doc_type_dir(context, "goal");
    let path = dir.join(format!("{slug}.md"));
    if path.exists() {
        return Ok(slug);
    }
    let id = crate::ids::generate_id();
    let vars = vec![
        ("slug", slug.as_str()),
        ("context", context),
        ("seq", "0"),
        ("id", id.as_str()),
    ];
    let content = vault::render_template_with_vars("goal", "Maintenance", &vars)?;
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
    // ... event unchanged
```

- [ ] **Step 3: Update create, update, count_tasks_by_stage**

Replace `config.goals_dir.join(context)` → `config.doc_type_dir(context, "goal")` and `config.tasks_dir.join(context)` → `config.doc_type_dir(context, "task")` in all three functions. Update `render_template_with_vars` calls to remove `vault_root` and `templates_dir` args.

- [ ] **Step 4: Verify compilation**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli 2>&1 | head -40`

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/goal.rs
git commit -m "refactor(cli): update goal actions to use context-first vault paths"
```

---

### Task 7: Update commands/session.rs — new vault paths

**Files:**
- Modify: `crates/temper-cli/src/commands/session.rs`

Replace `config.sessions_dir`, `config.templates_dir`, and `project::resolve_from_cwd` usage.

- [ ] **Step 1: Update save function**

Replace project resolution and path computation:

```rust
pub fn save(
    config: &Config,
    title: Option<&str>,
    project: Option<&str>,
    stdin_content: Option<&str>,
    task: Option<&str>,
    state: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();

    let project_name = project.unwrap_or("general").to_string();
    let note_title = title.unwrap_or(&today);

    let filename = format!("{today} \u{2014} {note_title}.md");
    let session_project_dir = config.doc_type_dir(&project_name, "session");
    let note_path = session_project_dir.join(&filename);

    // ... exists check unchanged ...

    let id = crate::ids::generate_id();
    let content = vault::render_template_with_vars(
        "session",
        note_title,
        &[("project", &project_name), ("id", &id)],
    )?;

    // ... rest unchanged except remove templates_dir_str extraction ...
```

- [ ] **Step 2: Update link_session_to_task**

Replace `config.tasks_dir.join(...)` with `config.doc_type_dir(...)`:

```rust
    let task_path = config
        .doc_type_dir(&task_info.context, "task")
        .join(format!("{}.md", task_info.slug));
```

- [ ] **Step 3: Update list function**

Replace `config.sessions_dir` with scanning contexts:

```rust
pub fn list(config: &Config, project: Option<&str>, format: &str) -> Result<()> {
    let mut entries: Vec<SessionEntry> = Vec::new();

    let contexts: Vec<&str> = if let Some(p) = project {
        vec![p]
    } else {
        config.contexts.iter().map(|s| s.as_str()).collect()
    };

    for ctx in contexts {
        let sessions_dir = config.doc_type_dir(ctx, "session");
        if sessions_dir.is_dir() {
            collect_sessions(&sessions_dir, ctx, &mut entries)?;
        }
    }

    // ... sort and display unchanged
```

- [ ] **Step 4: Update session_path helper**

```rust
pub fn session_path(config: &Config, project: &str, title: &str) -> PathBuf {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let filename = format!("{today} \u{2014} {title}.md");
    config.doc_type_dir(project, "session").join(filename)
}
```

- [ ] **Step 5: Remove project import**

Remove `use crate::project;` from the imports.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/session.rs
git commit -m "refactor(cli): update session commands to use context-first vault paths"
```

---

### Task 8: Update remaining commands

**Files:**
- Modify: `crates/temper-cli/src/commands/note.rs`
- Modify: `crates/temper-cli/src/commands/research.rs`
- Modify: `crates/temper-cli/src/commands/status.rs`
- Modify: `crates/temper-cli/src/commands/check.rs`
- Modify: `crates/temper-cli/src/commands/warmup.rs`
- Modify: `crates/temper-cli/src/actions/normalize.rs`

- [ ] **Step 1: Update note.rs**

Replace templates path computation. The `note create` command needs a `--context` flag to determine where to put the file. Use `config.doc_type_dir(context, note_type)` for the output path:

```rust
pub fn create(
    config: &Config,
    note_type: &str,
    title: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    let mut content = vault::render_template(note_type, title)?;

    let ctx = context.unwrap_or("default");
    let slug = vault::slugify(title);
    let note_dir = config.doc_type_dir(ctx, note_type);
    let note_path = note_dir.join(format!("{slug}.md"));
    // ... rest similar but remove templates_dir references
```

- [ ] **Step 2: Update research.rs**

Replace `project::resolve_from_cwd` and templates path:

```rust
pub fn save(
    config: &Config,
    title: &str,
    context: Option<&str>,
    stdin_content: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let project_name = context.unwrap_or("general").to_string();

    let slug = format!("{today}-{}", vault::slugify(title));
    let filename = format!("{today} \u{2014} {title}.md");
    let research_dir = config.doc_type_dir(&project_name, "research");
    let note_path = research_dir.join(&filename);

    // ... rest unchanged except use vault::render_template_with_vars("research", title, &vars)
```

Remove `use crate::project;`.

- [ ] **Step 3: Update status.rs**

Replace per-type directory counting with context-based scanning:

```rust
pub fn run(config: &Config, verbose: bool) -> Result<()> {
    output::header("Temper Vault");
    output::label("Root", config.vault_root.display());
    output::blank();

    // Count files across all contexts
    let mut total_files = 0;
    for ctx in &config.contexts {
        let ctx_dir = config.vault_root.join(ctx);
        if ctx_dir.is_dir() {
            total_files += count_md_files(&ctx_dir);
        }
    }

    output::header("Files");
    output::label("Total", total_files);
    output::blank();

    output::header("Contexts");
    if config.contexts.is_empty() {
        output::hint("  (none configured)");
    } else {
        for ctx in &config.contexts {
            let ctx_dir = config.vault_root.join(ctx);
            let count = if ctx_dir.is_dir() { count_md_files(&ctx_dir) } else { 0 };
            output::plain(format!("  {} ({} files)", ctx, count));
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Update check.rs**

Check `.temper/` dir and global config instead of `temper.toml` + type dirs:

```rust
fn check_vault(config: &Config) -> std::result::Result<(), String> {
    if !config.vault_root.exists() {
        return Err(format!(
            "vault root does not exist: {}",
            config.vault_root.display()
        ));
    }
    Ok(())
}

fn check_dirs(_config: &Config) -> std::result::Result<(), String> {
    // With context-first layout, dirs are created on-demand.
    // Just verify the vault root exists (already done in check_vault).
    Ok(())
}

fn check_state(config: &Config) -> std::result::Result<(), String> {
    if !config.state_dir.exists() {
        return Err(format!(
            "not initialized — run 'temper init' ({})",
            config.state_dir.display()
        ));
    }
    Ok(())
}
```

Update the `run` function's success message for dirs to say "Layout: context-first" instead of listing type dirs.

- [ ] **Step 5: Update warmup.rs**

Replace `config.sessions_dir.join(project)` with `config.doc_type_dir(project, "session")`:

```rust
fn collect_recent_sessions(
    config: &Config,
    project: &str,
    limit: usize,
) -> Vec<(String, String, std::path::PathBuf)> {
    let sessions_dir = config.doc_type_dir(project, "session");
    if !sessions_dir.exists() {
        return vec![];
    }
    // ... rest unchanged
```

- [ ] **Step 6: Update normalize.rs**

Replace `entity_base_dirs` with context-based scanning:

```rust
pub fn run(
    config: &Config,
    context: Option<&str>,
    dry_run: bool,
    fix_slugs: bool,
) -> Result<NormalizeSummary> {
    let mut summary = NormalizeSummary { /* ... */ };
    let opts = NormalizeOptions { dry_run, fix_slugs };

    let contexts: Vec<&str> = if let Some(c) = context {
        vec![c]
    } else {
        config.contexts.iter().map(|s| s.as_str()).collect()
    };

    for ctx in &contexts {
        for doc_type in &["task", "session", "goal", "research"] {
            let dir = config.doc_type_dir(ctx, doc_type);
            if dir.is_dir() {
                normalize_flat_dir(&opts, &dir, ctx, doc_type, &mut summary)?;
            }
        }
    }
    // ... event logging unchanged
```

Update `normalize_directory` to `normalize_flat_dir` — files are directly in the dir now (no context subdirectory nesting within the doc_type dir):

```rust
fn normalize_flat_dir(
    opts: &NormalizeOptions,
    dir: &Path,
    context: &str,
    doc_type: &str,
    summary: &mut NormalizeSummary,
) -> Result<()> {
    let md_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    for file_path in md_files {
        process_file(opts, dir, &file_path, context, summary)?;
    }

    Ok(())
}
```

The `base_dir.ends_with("tasks")` check in `process_file` changes to check `doc_type == "task"` — pass doc_type through or detect from path.

- [ ] **Step 7: Verify compilation**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli 2>&1 | head -40`

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/commands/note.rs crates/temper-cli/src/commands/research.rs \
       crates/temper-cli/src/commands/status.rs crates/temper-cli/src/commands/check.rs \
       crates/temper-cli/src/commands/warmup.rs crates/temper-cli/src/actions/normalize.rs
git commit -m "refactor(cli): update remaining commands to context-first vault layout"
```

---

### Task 9: Update init.rs — new vault initialization

**Files:**
- Modify: `crates/temper-cli/src/commands/init.rs`

Complete rewrite. Create `.temper/{manifest.json,events.jsonl}`. Write full default global config with `default` context and auth0 defaults.

- [ ] **Step 1: Rewrite init.rs**

```rust
use std::path::Path;

use crate::config::global_config_path;
use crate::error::Result;
use crate::output;

const DEFAULT_CONFIG_TOML: &str = r#"[vault]
path = "VAULT_PATH_PLACEHOLDER"

[sync.auto]
doctypes = ["task", "goal", "session"]

[sync.subscriptions]
contexts = ["default"]

[cli]
progress = "bar"

[skill]
output = "~/.claude/commands/temper.md"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
"#;

const EMPTY_MANIFEST: &str = r#"{"device_id":null,"last_sync":null,"entries":{}}"#;

pub fn run(path: &Path, no_interactive: bool, register_global: bool) -> Result<()> {
    output::dim(format!("Creating vault at {}", path.display()));
    std::fs::create_dir_all(path)?;

    // Create .temper/ state directory
    let temper_dir = path.join(".temper");
    std::fs::create_dir_all(&temper_dir)?;

    // Write empty manifest.json
    let manifest_path = temper_dir.join("manifest.json");
    if !manifest_path.exists() {
        std::fs::write(&manifest_path, EMPTY_MANIFEST)?;
        output::item("Created .temper/manifest.json");
    }

    // Write empty events.jsonl
    let events_path = temper_dir.join("events.jsonl");
    if !events_path.exists() {
        std::fs::write(&events_path, "")?;
        output::item("Created .temper/events.jsonl");
    }

    // Write or update global config
    if register_global {
        write_global_config(path)?;
    }

    if !no_interactive {
        output::blank();
        output::success("Vault initialized successfully");
        output::blank();
        output::header("Next steps");
        output::hint("  temper check          — verify vault and tool health");
        output::hint("  temper task create --title \"First Task\" --context default");
        output::blank();
    }

    Ok(())
}

fn write_global_config(vault_path: &Path) -> Result<()> {
    let config_path = global_config_path();

    let canonical = vault_path
        .canonicalize()
        .unwrap_or_else(|_| vault_path.to_path_buf());
    let vault_str = canonical.to_string_lossy();

    if config_path.exists() {
        // Update vault.path in existing config
        let content = std::fs::read_to_string(&config_path).unwrap_or_default();
        // Simple replacement of the path value
        let updated = if content.contains("path = ") {
            // Find and replace the vault path line
            let mut lines: Vec<String> = content.lines().map(String::from).collect();
            let mut in_vault_section = false;
            for line in &mut lines {
                let trimmed = line.trim();
                if trimmed == "[vault]" {
                    in_vault_section = true;
                } else if trimmed.starts_with('[') {
                    in_vault_section = false;
                } else if in_vault_section && trimmed.starts_with("path") {
                    *line = format!("path = \"{}\"", vault_str);
                }
            }
            lines.join("\n") + "\n"
        } else {
            content
        };
        std::fs::write(&config_path, updated)?;
        output::dim(format!("Updated vault path in {}", config_path.display()));
    } else {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = DEFAULT_CONFIG_TOML.replace("VAULT_PATH_PLACEHOLDER", &vault_str);
        std::fs::write(&config_path, &content)?;
        output::dim(format!("Created global config: {}", config_path.display()));
    }

    Ok(())
}
```

- [ ] **Step 2: Verify compilation**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli 2>&1 | head -20`

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/init.rs
git commit -m "feat(cli): rewrite init to create .temper/ state and full global config"
```

---

### Task 10: Update context_cmd.rs — contexts via global config

**Files:**
- Modify: `crates/temper-cli/src/commands/context_cmd.rs`
- Modify: `crates/temper-cli/src/cli.rs` (update context subcommand args)

Contexts are now `sync.subscriptions.contexts` in the global config. Add/remove edits the global config TOML. List reads from `Config.contexts`.

- [ ] **Step 1: Rewrite context_cmd.rs**

```rust
use crate::config::{self, Config};
use crate::error::{Result, TemperError};
use crate::output;

/// Add a context to sync.subscriptions.contexts in the global config.
pub fn add(name: &str) -> Result<()> {
    let config_path = config::global_config_path();
    config::safe_write(&config_path, |content| {
        // Parse, add context, re-serialize
        let mut cfg: toml::Value = toml::from_str(&content).unwrap_or(toml::Value::Table(Default::default()));
        let contexts = cfg
            .get_mut("sync")
            .and_then(|s| s.get_mut("subscriptions"))
            .and_then(|s| s.get_mut("contexts"))
            .and_then(|c| c.as_array_mut());

        if let Some(arr) = contexts {
            let val = toml::Value::String(name.to_string());
            if !arr.contains(&val) {
                arr.push(val);
            }
        }
        toml::to_string_pretty(&cfg).unwrap_or(content)
    })?;

    output::success(format!("Added context '{name}'"));
    Ok(())
}

/// Remove a context from sync.subscriptions.contexts in the global config.
pub fn remove(name: &str) -> Result<()> {
    let config_path = config::global_config_path();
    config::safe_write(&config_path, |content| {
        let mut cfg: toml::Value = toml::from_str(&content).unwrap_or(toml::Value::Table(Default::default()));
        let contexts = cfg
            .get_mut("sync")
            .and_then(|s| s.get_mut("subscriptions"))
            .and_then(|s| s.get_mut("contexts"))
            .and_then(|c| c.as_array_mut());

        if let Some(arr) = contexts {
            arr.retain(|v| v.as_str() != Some(name));
        }
        toml::to_string_pretty(&cfg).unwrap_or(content)
    })?;

    output::success(format!("Removed context '{name}'"));
    Ok(())
}

/// List configured contexts.
pub fn list(config: &Config) -> Result<()> {
    if config.contexts.is_empty() {
        output::hint("No contexts configured.");
        return Ok(());
    }

    for ctx in &config.contexts {
        output::plain(format!("  {ctx}"));
    }

    Ok(())
}
```

- [ ] **Step 2: Update cli.rs context subcommand**

The `ContextAction::Add` no longer needs `path` and `repo` arguments — just `name`. Update the clap enum:

```rust
// In cli.rs, update ContextAction:
#[derive(Subcommand)]
pub enum ContextAction {
    Add { name: String },
    Remove { name: String },
    List,
}
```

- [ ] **Step 3: Update main.rs context dispatch**

```rust
Commands::Context { action } => match action {
    ContextAction::Add { name } => {
        temper_cli::commands::context_cmd::add(&name)
    }
    ContextAction::Remove { name } => {
        temper_cli::commands::context_cmd::remove(&name)
    }
    ContextAction::List => {
        let config = temper_cli::config::load(cli.vault.as_deref())?;
        temper_cli::commands::context_cmd::list(&config)
    }
},
```

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/commands/context_cmd.rs crates/temper-cli/src/cli.rs
git commit -m "refactor(cli): contexts managed via global config subscriptions"
```

---

### Task 11: Update skill.rs — read from global config

**Files:**
- Modify: `crates/temper-cli/src/commands/skill.rs`

Read config hash from global config file. Contexts from `config.contexts`. Only check superpowers when framework is "superpowers".

- [ ] **Step 1: Update generate function**

```rust
pub fn generate(config: &Config) -> Result<String> {
    let config_path = crate::config::global_config_path();
    let config_content = std::fs::read_to_string(&config_path)
        .map_err(|e| TemperError::Config(format!("cannot read global config: {}", e)))?;
    let hash = format!("{:x}", Sha256::digest(config_content.as_bytes()));

    let vault_path = config.vault_root.display().to_string();

    let mut context_lines = Vec::new();
    let mut sorted_contexts = config.contexts.clone();
    sorted_contexts.sort();
    for ctx in &sorted_contexts {
        context_lines.push(format!("- `{ctx}`"));
    }
    let context_list = if context_lines.is_empty() {
        "(no contexts configured)".to_string()
    } else {
        context_lines.join("\n")
    };

    // ... rest of format string same but replace {project_list} with {context_list}
    // and replace variable name project_list → context_list
```

- [ ] **Step 2: Update check function**

Only check superpowers when `config.skill_framework == "superpowers"`:

```rust
pub fn check(config: &Config) -> Result<()> {
    if config.skill_framework == "superpowers" {
        let superpowers_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~"))
            .join(".claude/plugins/cache/claude-plugins-official/superpowers");

        if superpowers_path.exists() {
            output::status_icon(true, format!("Superpowers: {}", superpowers_path.display()));
        } else {
            output::status_icon(false, format!("Superpowers: NOT FOUND ({})", superpowers_path.display()));
        }
    }

    // Check skill file — read hash from global config instead of vault temper.toml
    let skill_path = &config.skill_output;
    // ... rest unchanged except hash comparison reads from global config:
    let config_path = crate::config::global_config_path();
    let config_content = std::fs::read_to_string(&config_path)
        .map_err(|e| TemperError::Config(format!("cannot read global config: {}", e)))?;
    let current_hash = format!("{:x}", Sha256::digest(config_content.as_bytes()));
    // ... hash comparison unchanged
```

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/skill.rs
git commit -m "refactor(cli): skill generation reads from global config"
```

---

### Task 12: Update main.rs and remove project.rs

**Files:**
- Modify: `crates/temper-cli/src/main.rs`
- Modify: `crates/temper-cli/src/project.rs`
- Modify: `crates/temper-cli/src/lib.rs` (remove `pub mod project;`)

- [ ] **Step 1: Remove all resolve_from_cwd calls in main.rs**

Every place that does:
```rust
let cwd = std::env::current_dir().unwrap_or_default();
let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
let context = context.as_deref().or_else(|| resolved.map(|r| r.name.as_str()));
```

Replace with just:
```rust
let context = context.as_deref();
```

This affects: `Events`, `Task` (all actions), `Goal` (list, update), `Warmup`, `Research`.

For commands that require a context (Task::Create, Goal::Create, Goal::List) and currently use `.ok_or_else(...)`, keep the error but remove the CWD fallback.

- [ ] **Step 2: Delete project.rs content**

Replace with a stub or delete the file entirely. If other modules import it, remove those imports.

```rust
// project.rs — deprecated, contexts are now from global config
```

- [ ] **Step 3: Remove pub mod project from lib.rs**

Find and remove the `pub mod project;` line.

- [ ] **Step 4: Full compilation check**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo check -p temper-cli`
Expected: PASS (no errors)

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/main.rs crates/temper-cli/src/project.rs crates/temper-cli/src/lib.rs
git commit -m "refactor(cli): remove project.rs and CWD-based context resolution"
```

---

### Task 13: Full build + test pass

**Files:** None new — verification only.

- [ ] **Step 1: Run cargo clippy**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo clippy --all-features -p temper-cli -p temper-core -- -D warnings`
Expected: PASS (fix any warnings)

- [ ] **Step 2: Run unit tests**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo test -p temper-cli -p temper-core`
Expected: PASS (some tests may need updating for new Config shape — fix inline)

- [ ] **Step 3: Fix any test failures**

Tests that construct `Config` directly will need updating to remove `sessions_dir`/`tasks_dir`/etc. and add `contexts`. Fix each one.

- [ ] **Step 4: Commit fixes**

```bash
git add -A
git commit -m "fix(cli): update tests for new config and vault layout"
```

---

### Task 14: Manual migration + cargo install

**Files:**
- Modify: `~/.config/temper/config.toml` (manual edit on disk)

- [ ] **Step 1: Write the new unified config.toml**

Manually write `~/.config/temper/config.toml`:

```toml
[vault]
path = "~/projects/kb-vault"

[sync.auto]
doctypes = ["task", "goal", "session"]

[sync.subscriptions]
contexts = ["temper", "storyteller", "tasker", "writing"]

[cli]
progress = "bar"

[skill]
output = "~/.claude/commands/temper.md"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
```

- [ ] **Step 2: Initialize the new vault**

Run: `temper init ~/projects/kb-vault`
Expected: `.temper/manifest.json` and `.temper/events.jsonl` created (vault path already set)

- [ ] **Step 3: Install the updated CLI**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && cargo install --path crates/temper-cli`
Expected: Binary installed to PATH

- [ ] **Step 4: Verify basic commands**

Run these in sequence:
```bash
temper check
temper status
temper context list
temper skill check
```
Expected: All pass without errors

---

### Task 15: Trial import and sync

- [ ] **Step 1: Create a test markdown file**

```bash
echo "# Test Document\n\nThis is a test file for import validation." > /tmp/test-import.md
```

- [ ] **Step 2: Import the file**

Run: `temper import /tmp/test-import.md --context temper --doc-type research`
Expected: File written to `~/projects/kb-vault/temper/research/test-import.md` with frontmatter. Manifest entry created.

- [ ] **Step 3: Verify vault file**

Run: `cat ~/projects/kb-vault/temper/research/test-import.md`
Expected: YAML frontmatter with `temper-id`, `title`, `context: temper`, `doc_type: research`, followed by content.

Run: `cat ~/projects/kb-vault/.temper/manifest.json | python3 -m json.tool`
Expected: One entry with the resource UUID, path, content_hash, remote_hash, state.

- [ ] **Step 4: Try sync (if cloud endpoint available)**

Run: `temper sync run --context temper`
Expected: Push succeeds (or shows connectivity error if cloud not available — either is acceptable for this trial).

- [ ] **Step 5: Regenerate and install skill**

Run: `temper skill install`
Expected: Skill file written to `~/.claude/commands/temper.md` with updated contexts list.

- [ ] **Step 6: Commit**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
git add -A
git commit -m "chore: complete I5e local KB restructure and first import"
```

---

### Task 16: Update Claude memories

- [ ] **Step 1: Update memory files**

Update any Claude memory files that reference `~/projects/knowledge` or the old vault layout:
- Check `/Users/petetaylor/.claude/projects/-Users-petetaylor-projects-tasker-systems-temper/memory/MEMORY.md`
- Update paths and descriptions to reference `~/projects/kb-vault` and the new config structure

- [ ] **Step 2: Session save**

Pipe session content to temper:

```bash
cat <<'EOF' | temper session save "I5e — Local KB Restructure" --task 2026-03-31-i5e-local-kb-restructure-and-first-import --state done
## Goal
Restructure temper-cli to use a single unified config at ~/.config/temper/config.toml and a context-first vault layout at ~/projects/kb-vault.

## What Happened
- Unified global config replaces split global+vault model
- Vault layout inverted to {context}/{doc_type}/{slug}.md
- temper init creates .temper/{manifest.json,events.jsonl} and full global config
- Contexts managed via sync.subscriptions.contexts instead of projects.*
- Slug-based filenames replace UUID-based filenames
- CWD-based context resolution removed
- Trial import+sync validated

## Decisions
- Kept TEMPER_GLOBAL_CONFIG env var (no rename)
- Auth0 defaults baked into init template for all new vaults
- Templates are embedded-only, no more vault-root templates/ dir

## Connections
- I5f: handler refactoring depends on new config types
- I5g: knowledge base migration uses new vault layout
- I6b: auto-sync uses sync.auto.doctypes from new config

## Next Steps
- I5f: refactor Rust/Axum handlers for context CRUD
- I5g: migrate ~/projects/knowledge to ~/projects/kb-vault
EOF
```
