# Enhance and Fix temper init — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the broken `temper init` config, clean up production data hygiene (doc_types migration + orphan audit), auto-provision default contexts for new profiles, and enhance the temper skill with resume/start/create flows.

**Architecture:** Three independent workstreams executed sequentially: (A) fix init config template + update `temper add` default doc_type, (B) database migration + server-side auto-provisioning + client-side fallback, (C) skill file updates for new command routing. Each workstream produces a clean commit.

**Tech Stack:** Rust (temper-cli, temper-api, temper-core), SQL (Postgres migrations), Markdown (skill files)

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/temper-cli/src/commands/init.rs` | Modify | Fix config template skill output path and subscriptions default |
| `crates/temper-cli/src/cli.rs:116` | Modify | Change `temper add` default doc_type from `"resource"` to `"research"` |
| `crates/temper-cli/tests/init_test.rs` | Modify | Add test for generated config content |
| `migrations/20260404000001_consolidate_doc_types.sql` | Create | Add `decision`, reclassify orphans, remove stale types |
| `crates/temper-api/src/services/profile_service.rs` | Modify | Auto-create `default` context on new profile |
| `crates/temper-api/tests/auth_test.rs` | Modify | Test that auto-provisioned profile gets default context |
| `~/.claude/skills/temper/SKILL.md` | Modify | Add task resume, session start, task create routing |
| `~/.claude/skills/temper/reference.md` | Modify | Add new commands to reference table |

---

### Task 1: Fix temper init config template

**Files:**
- Modify: `crates/temper-cli/src/commands/init.rs:79-105`
- Modify: `crates/temper-cli/tests/init_test.rs`

- [ ] **Step 1: Write a test for the generated config content**

Add to `crates/temper-cli/tests/init_test.rs`:

```rust
#[test]
fn test_init_config_has_correct_skill_output() {
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("myvault");

    // Create a fake global config dir so register_default_config writes there
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();

    // We can't easily test register_default_config without clobbering the real
    // global config, so test the run function with register_global=false and
    // verify the vault structure. The config content is tested via a unit test
    // in init.rs itself.
    temper_cli::commands::init::run(&vault_path, true, false).unwrap();
    assert!(vault_path.join(".temper/manifest.json").exists());
    assert!(vault_path.join("default").is_dir());
}
```

Since `register_default_config` is private and writes to the real global config path, add a unit test inside `init.rs`:

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn config_template_contains_correct_skill_output() {
        // Reproduce the format string logic with a test path
        let vault_path_str = "/tmp/test-vault";
        let config_content = format!(
            r#"[vault]
path = "{vault_path_str}"

[sync.auto]
doctypes = ["task", "goal", "session"]

# Add contexts to sync: temper context add <name>
[sync.subscriptions]
contexts = []

[cli]
progress = "bar"

[skill]
output = "~/.claude/skills/temper"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
"#
        );
        assert!(config_content.contains(r#"output = "~/.claude/skills/temper""#));
        assert!(config_content.contains("contexts = []"));
        assert!(!config_content.contains("commands/temper.md"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli config_template_contains_correct_skill_output`
Expected: FAIL — the test module doesn't exist yet in init.rs

- [ ] **Step 3: Fix the config template in init.rs**

In `crates/temper-cli/src/commands/init.rs`, replace the `register_default_config` function's format string (lines 79-105):

Change line 87 from:
```rust
contexts = ["default"]
```
to:
```rust
# Add contexts to sync: temper context add <name>
contexts = []
```

Change line 93 from:
```rust
output = "~/.claude/commands/temper.md"
```
to:
```rust
output = "~/.claude/skills/temper"
```

Add the test module at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_template_contains_correct_skill_output() {
        let vault_path_str = "/tmp/test-vault";
        let config_content = format!(
            r#"[vault]
path = "{vault_path_str}"

[sync.auto]
doctypes = ["task", "goal", "session"]

# Add contexts to sync: temper context add <name>
[sync.subscriptions]
contexts = []

[cli]
progress = "bar"

[skill]
output = "~/.claude/skills/temper"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
"#
        );
        assert!(
            config_content.contains(r#"output = "~/.claude/skills/temper""#),
            "skill output must point to skills dir, not commands"
        );
        assert!(
            config_content.contains("contexts = []"),
            "subscriptions should default to empty"
        );
        assert!(
            !config_content.contains("commands/temper.md"),
            "must not contain stale commands path"
        );
    }
}
```

Note: The test duplicates the format string — this is intentional. The test validates the *contract* (correct output path, empty subscriptions). When the template changes, the test breaks and forces review. Do NOT extract the format string into a shared function — that would make the test tautological.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-cli config_template_contains_correct_skill_output`
Expected: PASS

- [ ] **Step 5: Change `temper add` default doc_type from "resource" to "research"**

In `crates/temper-cli/src/cli.rs`, line 116, change:
```rust
        #[arg(long, default_value = "resource")]
        doc_type: String,
```
to:
```rust
        #[arg(long, default_value = "research")]
        doc_type: String,
```

- [ ] **Step 6: Run full CLI tests**

Run: `cargo nextest run -p temper-cli`
Expected: All pass

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/init.rs crates/temper-cli/src/cli.rs
git commit -m "fix: update temper init config template — correct skill path, empty subscriptions, default doc_type"
```

---

### Task 2: Write kb_doc_types consolidation migration

**Files:**
- Create: `migrations/20260404000001_consolidate_doc_types.sql`

- [ ] **Step 1: Write the migration**

Create `migrations/20260404000001_consolidate_doc_types.sql`:

```sql
-- =============================================================================
-- Consolidate kb_doc_types to canonical six
-- =============================================================================
-- Target types: task, goal, session, research, decision, concept
-- Removes: ticket, milestone, board, source, resource
-- Reclassifies any orphaned kb_resources to research before deletion.
--
-- Idempotent: uses IF EXISTS / ON CONFLICT guards so this is safe on both
-- fresh databases and production (where types were already manually deleted).

-- ─── 1. Add "decision" type ─────────────────────────────────────────────────
INSERT INTO kb_doc_types (id, name)
VALUES ('00000000-0000-0000-0001-00000000000b', 'decision')
ON CONFLICT (name) DO NOTHING;

-- ─── 2. Reclassify "resource" → "research" ─────────────────────────────────
-- Well-known IDs: resource = 0...0a, research = 0...04
UPDATE kb_resources
   SET kb_doc_type_id = '00000000-0000-0000-0001-000000000004'
 WHERE kb_doc_type_id = '00000000-0000-0000-0001-00000000000a';

-- ─── 3. Reclassify remaining stale types → "research" ──────────────────────
-- ticket (01), milestone (03), board (05), source (07)
UPDATE kb_resources
   SET kb_doc_type_id = '00000000-0000-0000-0001-000000000004'
 WHERE kb_doc_type_id IN (
     '00000000-0000-0000-0001-000000000001',  -- ticket
     '00000000-0000-0000-0001-000000000003',  -- milestone
     '00000000-0000-0000-0001-000000000005',  -- board
     '00000000-0000-0000-0001-000000000007'   -- source
 );

-- ─── 4. Catch-all: reclassify ANY orphan doc_type_id ────────────────────────
-- Safety net for manually-deleted types or future drift.
UPDATE kb_resources
   SET kb_doc_type_id = '00000000-0000-0000-0001-000000000004'
 WHERE kb_doc_type_id NOT IN (SELECT id FROM kb_doc_types);

-- ─── 5. Delete removed types ────────────────────────────────────────────────
DELETE FROM kb_doc_types
 WHERE name IN ('ticket', 'milestone', 'board', 'source', 'resource');
```

- [ ] **Step 2: Verify migration applies cleanly on local dev**

Run:
```bash
cargo make docker-up
sqlx migrate run --database-url postgresql://temper:temper@localhost:5437/temper_development
```
Expected: Migration applied successfully, no errors.

- [ ] **Step 3: Verify the canonical six exist**

Run:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "SELECT name FROM kb_doc_types ORDER BY name;"
```
Expected output:
```
  name
----------
 concept
 decision
 goal
 research
 session
 task
(6 rows)
```

- [ ] **Step 4: Verify no orphaned resources exist**

Run:
```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "
SELECT r.id, r.title, r.kb_doc_type_id
  FROM kb_resources r
  LEFT JOIN kb_doc_types dt ON r.kb_doc_type_id = dt.id
 WHERE dt.id IS NULL;
"
```
Expected: 0 rows

- [ ] **Step 5: Run integration tests to confirm migration doesn't break anything**

Run: `cargo nextest run -p temper-api --features test-db`
Expected: All pass

- [ ] **Step 6: Commit**

```bash
git add migrations/20260404000001_consolidate_doc_types.sql
git commit -m "feat: consolidate kb_doc_types to canonical six (task, goal, session, research, decision, concept)"
```

---

### Task 3: Production orphan audit

**Files:** None — this is a manual query session against Neon prod.

- [ ] **Step 1: Run orphan query against production**

The user will provide the Neon connection string or run this themselves:

```sql
-- Find kb_resources whose doc_type_id no longer exists in kb_doc_types
SELECT r.id, r.uri, r.title, r.kb_doc_type_id,
       r.created, r.is_active
  FROM kb_resources r
  LEFT JOIN kb_doc_types dt ON r.kb_doc_type_id = dt.id
 WHERE dt.id IS NULL
 ORDER BY r.created DESC;
```

- [ ] **Step 2: Review results with user**

Present findings. If orphans exist, propose reclassification:

```sql
-- Reclassify orphans to research
UPDATE kb_resources
   SET kb_doc_type_id = '00000000-0000-0000-0001-000000000004'
 WHERE kb_doc_type_id NOT IN (SELECT id FROM kb_doc_types);
```

- [ ] **Step 3: Run the migration against production**

After orphan audit is clean, apply the consolidation migration to prod. The migration is already idempotent and handles the case where types were already deleted.

- [ ] **Step 4: Verify prod state**

```sql
SELECT name FROM kb_doc_types ORDER BY name;
-- Expected: concept, decision, goal, research, session, task

SELECT count(*) FROM kb_resources r
  LEFT JOIN kb_doc_types dt ON r.kb_doc_type_id = dt.id
 WHERE dt.id IS NULL;
-- Expected: 0
```

---

### Task 4: Auto-provision default context on profile creation

**Files:**
- Modify: `crates/temper-api/src/services/profile_service.rs:94-127`
- Modify: `crates/temper-api/tests/auth_test.rs`

- [ ] **Step 1: Write a test for default context auto-provisioning**

Add to `crates/temper-api/tests/auth_test.rs`:

```rust
/// Auto-provisioned profile must have a "default" context.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_auto_provisioned_profile_has_default_context(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let sub = format!("test-sub-{}", uuid::Uuid::new_v4());
    let email = format!("defaultctx-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    // Trigger auto-provisioning
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);

    // Check that the "default" context was created
    let resp = app
        .client
        .get(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("contexts request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let body: Vec<Value> = resp.json().await.expect("expected JSON array");
    let has_default = body.iter().any(|c| c["name"] == "default");
    assert!(
        has_default,
        "auto-provisioned profile must have a 'default' context; got: {:?}",
        body.iter().map(|c| c["name"].clone()).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db test_auto_provisioned_profile_has_default_context`
Expected: FAIL — no default context created yet

- [ ] **Step 3: Add default context creation to profile_service.rs**

In `crates/temper-api/src/services/profile_service.rs`, after the new profile + auth link insertion (after line 125, before the `get_by_id` call on line 127), add:

```rust
    // Auto-provision a "default" context for the new profile.
    // Ignore conflict — if the profile somehow already has one, that's fine.
    sqlx::query(
        r#"
        INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
        VALUES ($1, 'default', 'kb_profiles', $2)
        ON CONFLICT ON CONSTRAINT kb_contexts_owner_name_unique DO NOTHING
        "#,
    )
    .bind(Uuid::now_v7())
    .bind(profile_id)
    .execute(pool)
    .await?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db test_auto_provisioned_profile_has_default_context`
Expected: PASS

- [ ] **Step 5: Run all API tests**

Run: `cargo nextest run -p temper-api --features test-db`
Expected: All pass

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/profile_service.rs crates/temper-api/tests/auth_test.rs
git commit -m "feat: auto-provision default context when creating new profiles"
```

---

### Task 5: Client-side context fallback with warning

**Files:**
- Modify: `crates/temper-cli/src/commands/task.rs` (or whichever commands resolve context)

The spec calls for client-side graceful degradation: when the CLI encounters a context that doesn't exist locally in the vault, warn and fall back to `default` rather than crashing.

- [ ] **Step 1: Identify where context resolution happens in CLI commands**

Search for context resolution patterns in `crates/temper-cli/src/commands/` — look for where `--context` is resolved to a vault directory path and where errors occur when the context directory doesn't exist.

- [ ] **Step 2: Add fallback logic**

Where context resolution would fail (directory not found), add a fallback:

```rust
let ctx_dir = config.vault_root.join(context);
if !ctx_dir.exists() {
    output::warning(format!(
        "Context \"{}\" not found in vault. Using \"default\" context.\n  \
         To create this context locally: temper context add {}",
        context, context
    ));
    context = "default";
}
```

Apply this pattern to: `task create`, `task list`, `task show`, `task move`, `session save`, `session list`, `research save`, `note create`, `goal create`, `goal list`.

Rather than duplicating across all commands, extract a helper in the commands module:

```rust
/// Resolve a context name, falling back to "default" with a warning if the
/// context directory doesn't exist in the vault.
pub fn resolve_context_with_fallback<'a>(config: &Config, context: &'a str) -> &'a str {
    let ctx_dir = config.vault_root.join(context);
    if ctx_dir.exists() {
        context
    } else {
        output::warning(format!(
            "Context \"{context}\" not found in vault. Using \"default\" context.\n  \
             To create this context locally: temper context add {context}"
        ));
        "default"
    }
}
```

Note: This returns a `&str` — since "default" is a static string, the lifetime works if we use `Cow<'a, str>` instead. Alternatively, return a `String`. Match existing patterns in the codebase.

- [ ] **Step 3: Add a test for the fallback**

```rust
#[test]
fn test_context_fallback_for_missing_context() {
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("vault");
    std::fs::create_dir_all(vault_path.join("default")).unwrap();

    // "missing" context directory doesn't exist
    let config = Config {
        vault_root: vault_path.clone(),
        state_dir: vault_path.join(".temper"),
        contexts: vec![],
        skill_output: PathBuf::from("/tmp/skill"),
        skill_framework: "superpowers".to_string(),
    };

    let resolved = resolve_context_with_fallback(&config, "missing");
    assert_eq!(resolved, "default");

    let resolved = resolve_context_with_fallback(&config, "default");
    assert_eq!(resolved, "default");
}
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p temper-cli`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/
git commit -m "feat: graceful context fallback — warn and use 'default' when context dir missing"
```

---

### Task 6: Update temper skill with resume, session start, and guided task create

**Files:**
- Modify: `~/.claude/skills/temper/SKILL.md`
- Modify: `~/.claude/skills/temper/reference.md`

- [ ] **Step 1: Add task resume routing to SKILL.md**

After the "On Task Start" section (after line 53), add:

```markdown
## On Task Resume

> **CLI sequence**: To resume a task from a previous session:
> 1. `temper task show <slug>` — reload the task content
> 2. `temper session list --context <ctx>` — find the most recent session
> 3. Read the last session's "Next Steps" section
> 4. Continue from the workflow file for this task's mode/effort

1. Read the task content via `temper task show <slug>` — extract mode, effort, and context
2. List recent sessions: `temper session list --context <ctx>`
3. Read the most recent session note linked to this task — look for "Next Steps"
4. If the task is not already in-progress, move it: `temper task move <slug> --stage in-progress`
5. Check for `guidance/fundamentals.md` — read if it exists
6. Check auto-memory for user plugin preferences
7. Scan for installed skills: check `~/.claude/skills/` and plugins cache
8. Ask: "Resuming from last session. Found these skills: [list]. Want subagents to use any? Any other quality gates?"
9. Read `workflows/{mode}-{effort}.md` and continue from where the last session left off
```

- [ ] **Step 2: Add session start routing to SKILL.md**

After the "On Task Resume" section, add:

```markdown
## On Session Start

> Start a working session without a predefined task. Useful for exploration,
> ad-hoc work, or when a task hasn't been created yet.

1. If `--context <ctx>` provided, use it. Otherwise ask which context to work in.
2. List in-progress tasks: `temper task list --context <ctx>`
3. If tasks exist, ask: "Working on one of these, or something new?"
   - If existing task: pivot to **On Task Resume** with that slug
   - If new: continue as open session
4. Check for `guidance/fundamentals.md` — read if it exists
5. Check auto-memory for user plugin preferences
6. Scan for installed skills
7. Proceed with the user's request. At session end, save via:
   ```bash
   cat <<'EOF' | temper session save "<title>" --context <ctx> --state done
   ## Goal
   ...
   EOF
   ```
```

- [ ] **Step 3: Add guided task create routing to SKILL.md**

After the "On Session Start" section, add:

```markdown
## On Task Create

> Guided interactive task creation. Gathers context, title, mode, effort,
> goal linkage, and acceptance criteria through conversation.

1. If `--context <ctx>` provided, use it. Otherwise list available contexts and ask.
2. Ask: "What's the title or problem statement for this task?"
3. Infer or ask mode:
   - "Is this (a) research/design/discovery (plan) or (b) implementation/building (build)?"
4. Infer or ask effort:
   - "How big is this? (a) small — single session, (b) medium — multi-step but bounded, (c) large — multi-session, may need decomposition"
5. List goals in context: `temper goal list --context <ctx>`
   - If goals exist, ask: "Link to a goal? [list] or (none)"
6. Ask: "Any specific acceptance criteria or outcomes?" (optional — user can skip)
7. Create the task:
   ```bash
   temper task create --title "<title>" --context <ctx> --mode <mode> --effort <effort> [--goal <slug>]
   ```
8. Ask: "Task created. Want to start working on it now?"
   - If yes: pivot to **On Task Start** with the new slug
```

- [ ] **Step 4: Update the argument parsing in SKILL.md**

Replace the "On Other Commands" section (lines 56-58) with an expanded routing table:

```markdown
## Command Routing

| Invocation Pattern | Route To |
|-------------------|----------|
| `task start <slug>` | On Task Start |
| `task resume <slug>` | On Task Resume |
| `task create [--context <ctx>]` | On Task Create |
| `session start [--context <ctx>]` | On Session Start |
| Other commands (search, session save, etc.) | Read `reference.md` for syntax |
```

- [ ] **Step 5: Update reference.md with new skill commands**

In `~/.claude/skills/temper/reference.md`, add to the Commands table:

```markdown
| task resume | `temper task show <slug>` + session lookup (skill-side, not CLI) |
| session start | Skill-guided session start (not a CLI command) |
```

And add a new section after the Commands table:

```markdown
## Skill-Only Commands

These commands are handled by the skill routing layer, not the temper CLI directly.
They compose multiple CLI commands into guided workflows.

| Skill Command | What It Does |
|---------------|-------------|
| `task start <slug>` | Shows task, moves to in-progress, routes to workflow |
| `task resume <slug>` | Shows task, reads last session, continues workflow |
| `task create` | Guided interactive task creation with prompts |
| `session start` | Start a session without a predefined task |
```

- [ ] **Step 6: Verify skill files parse correctly**

Read both files back and check for markdown formatting issues, broken tables, or section ordering problems.

- [ ] **Step 7: Commit**

```bash
git -C ~/.claude/skills/temper add SKILL.md reference.md
git -C ~/.claude/skills/temper commit -m "feat: add task resume, session start, and guided task create to temper skill"
```

Note: The skill files live outside the temper repo. If they're tracked in the temper repo via `temper skill install`, the commit should be in the temper repo instead. Check with the user.

---

### Task 7: Full verification and final commit

- [ ] **Step 1: Run cargo make check**

Run: `cargo make check`
Expected: All checks pass (fmt, clippy, docs, machete, TS typecheck, biome)

- [ ] **Step 2: Run full test suite**

Run: `cargo make test`
Expected: All unit tests pass

- [ ] **Step 3: Run integration tests**

Run: `cargo make test-db`
Expected: All integration tests pass (requires Docker Postgres running)

- [ ] **Step 4: Verify the skill files are installed**

Run: `temper skill install`
Expected: Skill installed successfully. Verify the SKILL.md at `~/.claude/skills/temper/SKILL.md` contains the new sections.

- [ ] **Step 5: Save session**

```bash
cat <<'EOF' | temper session save "Fix temper init, data hygiene, skill enhancements" --task 2026-04-03-enhance-and-fix-temper-init --state in-progress --context temper
## Goal
Fix broken temper init config, consolidate kb_doc_types, auto-provision default contexts, and add skill command routing for task resume/session start/task create.

## What Happened
- Fixed config.toml skill output path from ~/.claude/commands/temper.md to ~/.claude/skills/temper
- Changed default subscriptions from ["default"] to [] with guidance comment
- Changed temper add default doc_type from "resource" to "research"
- Created migration to consolidate kb_doc_types to canonical six
- Ran orphan audit against production
- Added default context auto-provisioning in profile_service.rs
- Added task resume, session start, and guided task create to SKILL.md

## Decisions
- Empty subscriptions by default — users add explicitly via temper context add
- resource doc_type reclassified to research (aligns with temper's taxonomy)
- Orphan catch-all in migration reclassifies to research as safe default
- Skill commands are routing-layer concepts, not new CLI commands

## Connections
- Design spec: docs/superpowers/specs/2026-04-04-enhance-and-fix-temper-init-design.md
- Future: temper doctor (workstream d), frontmatter schemas (workstream e)
- Future: interactive init flow (workstream g), temper move (workstream i)

## Next Steps
- Workstream d: Unify normalize/check into temper doctor
- Workstream e: Frontmatter JSON schemas + Obsidian alignment
- Workstream f: Auth login auto-provision profile hook
- Workstream g: Interactive temper init flow with guided prompts
EOF
```
