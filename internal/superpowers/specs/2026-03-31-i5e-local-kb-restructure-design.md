# I5e — Local KB Restructure and First Import

## Problem

The temper CLI has a split config model (global `~/.config/temper/config.toml` for auth + vault pointer, vault-root `temper.toml` for projects/skill/directories) and a vault layout that puts doc-type above context (`tasks/{context}/`). This needs to unify into a single global config and a context-first vault layout (`{context}/{doc-type}/{slug}.md`) before further sync and cloud work can proceed.

## Design

### 1. Unified Global Config

**Location**: `~/.config/temper/config.toml` (overridable via `TEMPER_GLOBAL_CONFIG`)

```toml
[vault]
path = "~/projects/kb-vault"

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
```

**What changes from current**:
- Auth sections merge in from current global config
- `[vault].path` replaces `default_vault`
- `[sync]` is new — defines auto-sync doctypes and subscribed contexts
- `[skill]` moves here from vault's temper.toml
- `[projects.*]` sections are removed — replaced by `[sync.subscriptions].contexts`

### 2. Vault Resolution (3-step, no CWD walk-up)

1. `--vault` CLI flag
2. `TEMPER_VAULT` env var
3. `[vault].path` from global config

The CWD walk-up (looking for `temper.toml` in parent dirs) is removed.

### 3. Vault Layout

```
~/projects/kb-vault/
├── .temper/
│   ├── manifest.json
│   └── events.jsonl
├── {context}/
│   └── {doc_type}/
│       └── {slug}.md
```

**Key conventions**:
- Canonical URI: `kb://{context}/{doc-type}/{resource-uuid}`
- File path: `{vault-root}/{context}/{doc-type}/{slug}.md`
- Slug derived from title (lowercase, hyphens, strip special chars)
- UUID lives in frontmatter (`temper-id`) and manifest, not filename
- Manifest maps UUID → relative path (`{context}/{doc-type}/{slug}.md`)

**No longer created in vault root**: `temper.toml`, `sessions/`, `tasks/`, `goals/`, `templates/`. Directories are created on-demand when files are written.

### 4. Config Types (Rust)

**`GlobalConfig`** becomes the primary deserialized struct:

```rust
struct GlobalConfig {
    vault: VaultPathConfig,           // { path: String }
    sync: Option<SyncConfig>,         // { auto: SyncAutoConfig, subscriptions: SyncSubscriptionsConfig }
    cli: Option<CliConfig>,           // { progress: String }
    skill: Option<SkillConfig>,       // { output: String, framework: String }
    auth: Option<AuthConfig>,         // { provider: String, providers: HashMap<String, AuthProviderConfig> }
}

struct SyncConfig {
    auto: SyncAutoConfig,             // { doctypes: Vec<String> }
    subscriptions: SyncSubscriptionsConfig, // { contexts: Vec<String> }
}
```

**`VaultConfig`** (old directory-name config) is removed. Directory names are hardcoded as the canonical layout: `{context}/{doc_type}/`.

**`Config`** (resolved runtime) is built from `GlobalConfig` only:
- `vault_root` from `GlobalConfig.vault.path`
- `state_dir` is always `{vault_root}/.temper`
- `contexts` from `sync.subscriptions.contexts`
- `skill_output` / `skill_framework` from `skill` section
- No more `sessions_dir` / `tasks_dir` / `goals_dir` — paths are computed from context + doc_type

### 5. `temper init` Changes

`temper init [path]` (path defaults to CWD):

1. Create `{path}/.temper/` directory
2. Write empty `{path}/.temper/manifest.json` (`{"device_id": null, "last_sync": null, "entries": {}}`)
3. Write empty `{path}/.temper/events.jsonl`
4. If `~/.config/temper/config.toml` does not exist:
   - Create `~/.config/temper/` directory
   - Write default config with `[vault].path` set to the init path, `default` context, full auth0 defaults
5. If it exists: update `[vault].path` to point to the new vault (using `safe_write`)
6. Print success message with next steps

### 6. Skill Generation Changes

- Reads from `GlobalConfig` instead of vault-root `temper.toml`
- Config hash computed from global config file content
- Contexts list comes from `sync.subscriptions.contexts`
- `framework = "superpowers"` includes superpowers workflow guidance; other values omit it and use simpler plan/build guidance
- Superpowers check (`~/.claude/plugins/cache/...`) only runs when framework is "superpowers"

### 7. Downstream Command Updates

All commands that call `config::load()` continue to work — the function signature stays the same, but internally reads from global config. Commands that reference `config.sessions_dir` etc. need updating to compute paths as `vault_root / context / doc_type /`.

Key files to update:
- `config.rs` — new GlobalConfig struct, updated `load()`, remove `resolve_vault` walk-up
- `commands/init.rs` — new init behavior
- `commands/skill.rs` — read from global config
- `actions/ingest.rs` — `build_vault_path` already takes context + doc_type, may just work
- `main.rs` — remove any vault-toml-specific logic
- All commands using `config.sessions_dir` / `config.tasks_dir` / `config.goals_dir`

### 8. Import/Sync Trial

After code changes + `cargo install`:

1. `temper init ~/projects/kb-vault` — verify `.temper/` created, global config written
2. Create a simple markdown file as test content
3. `temper import <file> --context temper --doc-type research` — single file import
4. Verify: file at `~/projects/kb-vault/temper/research/{slug}.md` with frontmatter
5. Verify: manifest entry in `.temper/manifest.json`
6. `temper sync run` — push to cloud
7. Verify round-trip if cloud endpoint is available

### 9. Out of Scope

- Auto-sync on write (I6b)
- Team sync subscriptions
- Merge conflict resolution
- Migrating existing ~/projects/knowledge content (I5g)
- HNSW index / registry.json management
- temper-mcp / vault-skills updates
