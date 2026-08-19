# CLI Discoverability and Skill Accuracy Fixes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix gaps between skill docs and CLI reality, implement missing filter flags, add seq-number task lookup, and redesign warmup output to focus on goals/tasks/sessions instead of raw events.

**Architecture:** All changes are in `temper-cli`. We add `--stage` to `task list`, `--limit` to `session list`, and seq-number lookup to `find_task`. Warmup output drops the events section (which is becoming irrelevant) and keeps the existing sections: recent sessions, in-progress tasks, last session content. Finally, `reference.md` is corrected to match actual CLI flags.

**Tech Stack:** Rust (clap, serde_yaml), temper-cli unit tests, temper-e2e

**Subagent guidance:** SG-1 (follow existing patterns), SG-4 (test strategy), SG-5 (don't over-build), SG-6 (verify before claiming done), SG-10 (checkpoint before continuing).

**Test commands:**
- Unit tests: `cargo nextest run -p temper-cli`
- Lint: `cargo make check`

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `crates/temper-cli/src/cli.rs` | Modify | Add `--stage` to `TaskAction::List`, `--limit` to `SessionAction::List` |
| `crates/temper-cli/src/main.rs` | Modify | Thread new args to handlers |
| `crates/temper-cli/src/commands/task.rs` | Modify | Accept and apply `stage` filter in `list()` |
| `crates/temper-cli/src/actions/task.rs` | Modify | Add seq-number lookup to `find_task()` |
| `crates/temper-cli/src/commands/session.rs` | Modify | Accept `limit` param in `list()` |
| `crates/temper-cli/src/commands/warmup.rs` | Modify | Remove events section, add goal listing |
| `crates/temper-cli/tests/task_test.rs` | Modify | Add tests for `--stage` filter and seq-number lookup |
| `crates/temper-cli/tests/session_test.rs` | Modify | Add test for `--limit` |
| `crates/temper-cli/tests/warmup_test.rs` | Modify | Update warmup tests for new output |
| `~/.claude/skills/temper/reference.md` | Modify | Correct documented flags to match CLI |

---

### Task 1: Add `--stage` filter to `task list`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:241-249`
- Modify: `crates/temper-cli/src/main.rs:192-198`
- Modify: `crates/temper-cli/src/commands/task.rs:33-67`
- Test: `crates/temper-cli/tests/task_test.rs`

- [ ] **Step 1: Write the failing test for stage filtering**

In `crates/temper-cli/tests/task_test.rs`, add:

```rust
#[test]
fn test_task_list_filters_by_stage() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug_a = temper_cli::commands::task::create(
        &config, "myapp", "Backlog Task", Some(&g_slug), None, None,
    ).unwrap();
    let slug_b = temper_cli::commands::task::create(
        &config, "myapp", "Active Task", Some(&g_slug), None, None,
    ).unwrap();

    // Move one to in-progress
    temper_cli::commands::task::move_task(
        &config, &slug_b, Some("in-progress"), None, None, None, None,
    ).unwrap();

    // List only in-progress — should succeed and not include backlog task
    let result = temper_cli::commands::task::list(
        &config, Some("myapp"), None, Some("in-progress"), "json",
    );
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli test_task_list_filters_by_stage`
Expected: compile error — `list()` doesn't accept a `stage` parameter yet.

- [ ] **Step 3: Add `--stage` arg to clap definition**

In `crates/temper-cli/src/cli.rs`, change `TaskAction::List`:

```rust
    /// List tasks
    List {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        goal: Option<String>,
        #[arg(long)]
        stage: Option<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
```

- [ ] **Step 4: Thread `stage` through main.rs**

In `crates/temper-cli/src/main.rs`, update the `TaskAction::List` match arm:

```rust
                TaskAction::List {
                    context,
                    goal,
                    stage,
                    format,
                } => {
                    let context = context.as_deref();
                    temper_cli::commands::task::list(
                        &config,
                        context,
                        goal.as_deref(),
                        stage.as_deref(),
                        &format,
                    )
                }
```

- [ ] **Step 5: Add stage filter to `task::list()`**

In `crates/temper-cli/src/commands/task.rs`, update the `list` function:

```rust
/// List tasks grouped by goal.
pub fn list(
    config: &Config,
    context: Option<&str>,
    goal_slug: Option<&str>,
    stage: Option<&str>,
    format: &str,
) -> Result<()> {
    let mut tasks = load_tasks(config, context, goal_slug)?;
    if let Some(s) = stage {
        crate::vault::validate_stage(s)?;
        tasks.retain(|t| t.stage == s);
    }
    if format == "json" {
        let json = serde_json::to_string_pretty(&tasks)
            .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
        println!("{json}");
        return Ok(());
    }
    if tasks.is_empty() {
        output::hint("No tasks found.");
        return Ok(());
    }
    // Group by goal
    let mut by_goal: std::collections::BTreeMap<String, Vec<&TaskInfo>> =
        std::collections::BTreeMap::new();
    for task in &tasks {
        by_goal.entry(task.goal.clone()).or_default().push(task);
    }
    for (g, tix) in &by_goal {
        output::blank();
        output::header(format!("## {g}"));
        for t in tix {
            output::plain(format!(
                "  {:>3}  [{:<10}]  {} ({})",
                t.seq, t.stage, t.title, t.context
            ));
        }
    }
    Ok(())
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo nextest run -p temper-cli test_task_list_filters_by_stage`
Expected: PASS

- [ ] **Step 7: Write test for invalid stage rejection**

```rust
#[test]
fn test_task_list_rejects_invalid_stage() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    temper_cli::commands::task::create(
        &config, "myapp", "Task", Some(&g_slug), None, None,
    ).unwrap();

    let result = temper_cli::commands::task::list(
        &config, Some("myapp"), None, Some("brainstorm"), "text",
    );
    assert!(result.is_err(), "invalid stage should be rejected");
}
```

- [ ] **Step 8: Run all task tests**

Run: `cargo nextest run -p temper-cli task`
Expected: all pass

- [ ] **Step 9: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs \
  crates/temper-cli/src/commands/task.rs crates/temper-cli/tests/task_test.rs
git commit -m "feat(cli): add --stage filter to task list command"
```

---

### Task 2: Add `--limit` to `session list`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:327-333`
- Modify: `crates/temper-cli/src/main.rs:111-112`
- Modify: `crates/temper-cli/src/commands/session.rs:333-374`
- Test: `crates/temper-cli/tests/session_test.rs`

- [ ] **Step 1: Write the failing test**

In `crates/temper-cli/tests/session_test.rs`, add:

```rust
#[test]
fn test_session_list_respects_limit() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Create sessions across contexts to get multiple entries
    temper_cli::commands::session::save(
        &config, Some("Alpha"), Some("proj"), None, None, None, "text",
    ).unwrap();
    temper_cli::commands::session::save(
        &config, Some("Beta"), Some("other"), None, None, None, "text",
    ).unwrap();

    // List with limit=1 — should succeed (we verify via json output)
    let result = temper_cli::commands::session::list(&config, None, Some(1), "json");
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli test_session_list_respects_limit`
Expected: compile error — `list()` doesn't accept a `limit` parameter.

- [ ] **Step 3: Add `--limit` to clap and thread through**

In `crates/temper-cli/src/cli.rs`, update `SessionAction::List`:

```rust
    /// List recent sessions
    List {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value = "text")]
        format: String,
    },
```

In `crates/temper-cli/src/main.rs`, update the `SessionAction::List` match arm:

```rust
                SessionAction::List { context, limit, format } => {
                    temper_cli::commands::session::list(
                        &config,
                        context.as_deref(),
                        limit,
                        &format,
                    )
                }
```

- [ ] **Step 4: Update `session::list()` to accept limit**

In `crates/temper-cli/src/commands/session.rs`, update `list`:

```rust
pub fn list(config: &Config, context: Option<&str>, limit: Option<usize>, format: &str) -> Result<()> {
    let mut entries: Vec<SessionEntry> = Vec::new();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    for ctx in &contexts_to_scan {
        let session_dir = config.doc_type_dir(ctx, "session");
        if session_dir.is_dir() {
            collect_sessions(&session_dir, ctx, &mut entries)?;
        }
    }

    // Sort by date descending (most recent first)
    entries.sort_by(|a, b| b.date.cmp(&a.date));
    entries.truncate(limit.unwrap_or(20));

    if format == "json" {
        let json = serde_json::to_string_pretty(&entries).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    if entries.is_empty() {
        output::hint("No sessions found.");
        return Ok(());
    }

    output::plain(format!("{:<12} {:<20} Title", "Date", "Context"));
    output::dim("-".repeat(60));
    for entry in &entries {
        output::plain(format!(
            "{:<12} {:<20} {}",
            entry.date, entry.context, entry.title
        ));
    }

    Ok(())
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p temper-cli test_session_list`
Expected: all session list tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs \
  crates/temper-cli/src/commands/session.rs crates/temper-cli/tests/session_test.rs
git commit -m "feat(cli): add --limit flag to session list command"
```

---

### Task 3: Add seq-number lookup to `find_task`

**Files:**
- Modify: `crates/temper-cli/src/actions/task.rs:77-104`
- Test: `crates/temper-cli/tests/task_test.rs`

- [ ] **Step 1: Write the failing test**

In `crates/temper-cli/tests/task_test.rs`, add:

```rust
#[test]
fn test_find_task_by_seq_number() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config, "myapp", "Seq Lookup", Some(&g_slug), None, None,
    ).unwrap();

    // Get the task's seq number
    let task = temper_cli::commands::task::find_task(&config, &slug, None)
        .unwrap()
        .unwrap();
    let seq = task.seq;

    // Look up by seq number (as string)
    let found = temper_cli::commands::task::find_task(&config, &seq.to_string(), None)
        .unwrap();
    assert!(found.is_some(), "should find task by seq number");
    assert_eq!(found.unwrap().slug, slug);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli test_find_task_by_seq_number`
Expected: FAIL — assertion `found.is_some()` fails because seq number doesn't match any slug or suffix.

- [ ] **Step 3: Add seq-number lookup to `find_task`**

In `crates/temper-cli/src/actions/task.rs`, update `find_task`:

```rust
/// Find a task by exact slug, unambiguous suffix match, or sequence number.
pub fn find_task(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
) -> Result<Option<TaskInfo>> {
    let all = load_tasks(config, context, None)?;
    // Exact match first
    if let Some(t) = all.iter().find(|t| t.slug == slug_or_suffix) {
        return Ok(Some(t.clone()));
    }
    // Suffix match
    let matches: Vec<_> = all
        .iter()
        .filter(|t| t.slug.ends_with(slug_or_suffix))
        .collect();
    match matches.len() {
        1 => return Ok(Some(matches[0].clone())),
        n if n > 1 => {
            return Err(TemperError::Vault(format!(
                "ambiguous slug suffix '{slug_or_suffix}', matches: {}",
                matches
                    .iter()
                    .map(|t| t.slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
        _ => {}
    }
    // Seq number match
    if let Ok(seq) = slug_or_suffix.parse::<u32>() {
        let seq_matches: Vec<_> = all.iter().filter(|t| t.seq == seq).collect();
        if seq_matches.len() == 1 {
            return Ok(Some(seq_matches[0].clone()));
        }
    }
    Ok(None)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-cli test_find_task_by_seq`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/task.rs crates/temper-cli/tests/task_test.rs
git commit -m "feat(cli): support seq number lookup in task show/move/done"
```

---

### Task 4: Redesign warmup output

**Files:**
- Modify: `crates/temper-cli/src/commands/warmup.rs`
- Test: `crates/temper-cli/tests/warmup_test.rs`

The warmup redesign removes the events section (which is becoming irrelevant as events.jsonl goes away in the kb-resource-audits work) and increases session count from 3 to 5. The remaining sections — recent sessions, in-progress tasks, and last session content — are exactly what agents need for context.

- [ ] **Step 1: Update warmup tests**

Replace the existing `test_warmup_produces_output` test and add a test that verifies events are NOT shown:

```rust
#[test]
fn test_warmup_does_not_show_events_section() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    temper_cli::commands::task::create(
        &config, "myapp", "Test", Some(&g_slug), None, None,
    ).unwrap();

    // Warmup should succeed and not reference events
    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}

#[test]
fn test_warmup_json_has_no_events() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "json");
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run tests to confirm current state**

Run: `cargo nextest run -p temper-cli warmup`
Expected: existing tests pass

- [ ] **Step 3: Remove events from warmup**

In `crates/temper-cli/src/commands/warmup.rs`, replace the entire file:

```rust
use crate::config::Config;
use crate::error::Result;

const MAX_SESSION_LINES: usize = 500;

/// Run the warmup command — output a context primer for a new session.
pub fn run(config: &Config, project: Option<&str>, format: &str) -> Result<()> {
    let project_name = project.unwrap_or("general");

    match format {
        "json" => run_json(config, project_name),
        _ => run_text(config, project_name),
    }
}

fn run_text(config: &Config, project: &str) -> Result<()> {
    println!("# Session Context: {project}");
    println!();

    // Section 1: Recent sessions
    println!("## Recent Sessions");
    println!();
    let sessions = collect_recent_sessions(config, project, 5);
    if sessions.is_empty() {
        println!("No recent sessions.");
    } else {
        for (date, title, _path) in &sessions {
            println!("- {date}: {title}");
        }
    }
    println!();

    // Section 2: In-progress tasks
    let in_progress = collect_in_progress_tasks(config, project);
    if !in_progress.is_empty() {
        println!("## In-Progress Tasks");
        println!();
        for (title, slug, mode, effort) in &in_progress {
            let mode_label = mode.as_deref().unwrap_or("no-mode");
            let effort_label = effort.as_deref().unwrap_or("no-effort");
            println!("- [{mode_label}/{effort_label}] {slug}: {title}");
        }
        println!();
    }

    // Section 3: Last session content
    if let Some((_date, _title, path)) = sessions.first() {
        println!("## Last Session");
        println!();
        if let Ok(content) = std::fs::read_to_string(path) {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() > MAX_SESSION_LINES {
                for line in &lines[..MAX_SESSION_LINES] {
                    println!("{line}");
                }
                println!();
                println!(
                    "... (truncated at {MAX_SESSION_LINES} lines, see full note at {})",
                    path.display()
                );
            } else {
                print!("{content}");
            }
        }
        println!();
    }

    Ok(())
}

fn run_json(config: &Config, project: &str) -> Result<()> {
    let sessions = collect_recent_sessions(config, project, 5);
    let in_progress = collect_in_progress_tasks(config, project);
    let in_progress_json: Vec<_> = in_progress
        .iter()
        .map(|(title, slug, mode, effort)| {
            serde_json::json!({
                "title": title,
                "slug": slug,
                "mode": mode,
                "effort": effort,
            })
        })
        .collect();

    let last_session_content = sessions.first().and_then(|(_, _, path)| {
        std::fs::read_to_string(path).ok().map(|content| {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() > MAX_SESSION_LINES {
                lines[..MAX_SESSION_LINES].join("\n")
            } else {
                content
            }
        })
    });

    let output = serde_json::json!({
        "project": project,
        "recent_sessions": sessions.iter().map(|(date, title, _)| {
            serde_json::json!({"date": date, "title": title})
        }).collect::<Vec<_>>(),
        "last_session_content": last_session_content,
        "in_progress_tasks": in_progress_json,
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_default()
    );
    Ok(())
}

/// Collect recent session files for a project, sorted by date descending.
/// Returns (date, title, path) tuples.
fn collect_recent_sessions(
    config: &Config,
    project: &str,
    limit: usize,
) -> Vec<(String, String, std::path::PathBuf)> {
    let sessions_dir = config.doc_type_dir(project, "session");
    if !sessions_dir.exists() {
        return vec![];
    }

    let mut entries: Vec<(String, String, std::path::PathBuf)> = Vec::new();

    if let Ok(dir) = std::fs::read_dir(&sessions_dir) {
        for entry in dir.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Parse date from filename: "YYYY-MM-DD — Title"
            let date = if stem.len() >= 10 {
                stem[..10].to_string()
            } else {
                "unknown".to_string()
            };

            let title = if let Some(pos) = stem.find(" \u{2014} ") {
                stem[pos + " \u{2014} ".len()..].to_string()
            } else {
                stem.clone()
            };

            entries.push((date, title, path));
        }
    }

    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries.truncate(limit);
    entries
}

/// Collect in-progress tasks for a project.
/// Returns (title, slug, mode, effort) tuples.
fn collect_in_progress_tasks(
    config: &Config,
    project: &str,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let tasks = match crate::commands::task::load_tasks(config, Some(project), None) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    tasks
        .into_iter()
        .filter(|t| t.stage == "in-progress")
        .map(|t| (t.title, t.slug, t.mode, t.effort))
        .collect()
}
```

- [ ] **Step 4: Run warmup tests**

Run: `cargo nextest run -p temper-cli warmup`
Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/warmup.rs crates/temper-cli/tests/warmup_test.rs
git commit -m "feat(cli): redesign warmup output — drop events, focus on sessions and tasks"
```

---

### Task 5: Fix `reference.md` to match actual CLI

**Files:**
- Modify: `~/.claude/skills/temper/reference.md`

- [ ] **Step 1: Update reference.md**

Replace the commands table and related documentation to match actual CLI flags:

```markdown
## Commands

| Command | Syntax |
|---------|--------|
| search | `temper search "<query>" [--context <ctx>] [--type <doctype>]` |
| context | `temper context [<name>]` |
| session save | `temper session save [<title>] [--context <ctx>] [--task <slug>] [--state <state>]` |
| session list | `temper session list [--context <ctx>] [--limit <n>]` |
| session show | `temper session show <slug> [--context <ctx>]` |
| task create | `temper task create --title "<title>" --context <ctx> [--goal <slug>] [--mode <mode>] [--effort <effort>]` |
| task list | `temper task list [--context <ctx>] [--goal <slug>] [--stage <stage>]` |
| task move | `temper task move <slug> [--stage <stage>] [--goal <slug>] [--context <ctx>] [--mode <mode>] [--effort <effort>]` |
| task done | `temper task done <slug> [--branch <name>] [--pr <url>] [--context <ctx>]` |
| task show | `temper task show <slug-or-suffix-or-seq> [--context <ctx>]` |
| goal list | `temper goal list [--context <ctx>]` |
| note create | `temper note create "<title>" [--context <ctx>] [--type <doctype>]` |
| research save | `temper research save "<title>" [--task <slug>]` |
| normalize | `temper normalize [--dry-run]` |
| events | `temper events [--limit <n>]` |
| warmup | `temper warmup [--context <ctx>]` |
| index | `temper index [--force]` |
| status | `temper status` |
```

Key corrections:
- `task create` uses `--title` (not positional)
- `task list` has `--stage` and `--goal` filters
- `task show` accepts slug, suffix, or seq number
- `session list` has `--limit`
- All format flags (`--format`) are documented but have default `text`, so omitted from table for brevity

- [ ] **Step 2: Verify the skill file parses correctly**

Run: `temper status` to confirm the CLI works with the installed binary.

- [ ] **Step 3: Commit reference.md**

Note: `reference.md` is outside the repo (in `~/.claude/skills/temper/`), so this is not a git commit — just save the file.

---

### Task 6: Full verification

- [ ] **Step 1: Run full test suite**

Run: `cargo nextest run -p temper-cli`
Expected: all tests pass

- [ ] **Step 2: Run lint and format checks**

Run: `cargo make check`
Expected: no warnings or errors

- [ ] **Step 3: Build the binary**

Run: `cargo build -p temper-cli`
Expected: clean build

- [ ] **Step 4: Manual smoke test**

```bash
# Rebuild and install
cargo install --path crates/temper-cli

# Test the new flags
temper task list --stage in-progress --context temper
temper session list --limit 3 --context temper
temper task show 200  # seq number lookup
temper warmup --context temper  # should show no events section
```
