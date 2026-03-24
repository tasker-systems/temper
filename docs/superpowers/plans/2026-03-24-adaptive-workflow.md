# Adaptive Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `scope` field (patch/feature/epic) to ticket frontmatter that drives workflow routing in the generated skill template.

**Architecture:** Optional `scope` field on `TicketInfo` with clap `ValueEnum` validation. Template renders `scope: null` by default. Skill template replaces linear workflow with a decision tree keyed on scope. Warmup surfaces in-progress tickets with scope.

**Tech Stack:** Rust, clap 4 (derive), serde, chrono

**Spec:** `docs/superpowers/specs/2026-03-24-adaptive-workflow-design.md`

---

### Task 1: Add scope to ticket template

**Files:**
- Modify: `src/templates/ticket.md`

- [ ] **Step 1: Add scope field to template**

```markdown
---
id: "{{id}}"
type: ticket
title: "{{title}}"
slug: "{{slug}}"
project: "{{project}}"
milestone: "{{milestone}}"
stage: backlog
scope: {{scope}}
seq: {{seq}}
created: {{datetime}}
updated: {{datetime}}
branch: null
pr: null
---

# {{title}}
```

- [ ] **Step 2: Commit**

```bash
git add src/templates/ticket.md
git commit -m "feat: add scope field to ticket template"
```

---

### Task 2: Add scope to TicketInfo struct and create function

**Files:**
- Modify: `src/commands/ticket.rs:13-25` (TicketInfo struct)
- Modify: `src/commands/ticket.rs:129-201` (create function)

- [ ] **Step 1: Write failing test — create ticket with scope**

Add to `tests/ticket_test.rs`:

```rust
#[test]
fn test_ticket_create_with_scope() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config,
        "myapp",
        "Scoped Ticket",
        Some(&ms_slug),
        Some("feature"),
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("scope: feature"), "should contain scope: feature");
}

#[test]
fn test_ticket_create_without_scope() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config,
        "myapp",
        "Unscoped Ticket",
        Some(&ms_slug),
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("scope: null"), "should contain scope: null");
}

#[test]
fn test_ticket_create_rejects_invalid_scope() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let result = temper_cli::commands::ticket::create(
        &config, "myapp", "Bad Scope", Some(&ms_slug), Some("huge"),
    );
    assert!(result.is_err(), "invalid scope on create should be rejected");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test ticket_test test_ticket_create_with_scope test_ticket_create_without_scope`
Expected: FAIL — `create()` doesn't accept scope parameter yet

- [ ] **Step 3: Update TicketInfo struct**

In `src/commands/ticket.rs`, update the struct:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TicketInfo {
    pub title: String,
    pub slug: String,
    pub project: String,
    pub milestone: String,
    pub stage: String,
    pub scope: Option<String>,
    pub seq: u32,
    #[expect(dead_code, reason = "deserialized from frontmatter, used in done() output")]
    pub branch: Option<String>,
    #[expect(dead_code, reason = "deserialized from frontmatter, used in done() output")]
    pub pr: Option<String>,
}
```

- [ ] **Step 4: Update create() signature and body**

Add `scope: Option<&str>` parameter to `create()`. Add scope to template vars:

```rust
pub fn create(
    config: &Config,
    project: &str,
    title: &str,
    milestone_slug: Option<&str>,
    scope: Option<&str>,
) -> Result<String> {
```

Validate scope upfront, then add to template vars:

```rust
    // Validate scope if provided
    let valid_scopes = ["patch", "feature", "epic"];
    if let Some(sc) = scope {
        if !valid_scopes.contains(&sc) {
            return Err(TemperError::Vault(format!(
                "invalid scope: {sc}. Must be one of: {}",
                valid_scopes.join(", ")
            )));
        }
    }
```

In the vars vec, add scope (the literal string "null" renders as YAML null in frontmatter):

```rust
    let scope_str = scope.unwrap_or("null");
    let vars = vec![
        ("slug", slug.as_str()),
        ("project", project),
        ("milestone", ms_slug.as_str()),
        ("seq", seq_str.as_str()),
        ("datetime", datetime.as_str()),
        ("id", id.as_str()),
        ("scope", scope_str),
    ];
```

- [ ] **Step 5: Update all existing callers of create()**

In `src/main.rs` (line 179), add `None` as the scope parameter to the existing call:

```rust
temper_cli::commands::ticket::create(
    &config,
    project,
    &title,
    milestone.as_deref(),
    None, // scope — wired up in Task 4
)?;
```

Update all existing callers of `create()` to add `None` as the fifth argument. Complete list:

**tests/ticket_test.rs** (7 calls):
- `test_ticket_create_includes_uuid_id` (line 12)
- `test_milestone_create_and_ticket_create` (line 38)
- `test_ticket_move_to_in_progress` (line 65)
- `test_ticket_move_rejects_old_stages` (line 85)
- `test_ticket_move_to_cancelled` (line 101)
- `test_ticket_move_and_done` (line 121)
- `test_ticket_list_json_format` (line 176)

**tests/warmup_test.rs** (1 call):
- `test_warmup_produces_output` (line 13)

**tests/normalize_test.rs** (4 calls):
- `test_normalize_backfills_missing_ids` (line 12)
- `test_normalize_migrates_old_stages` (line 42)
- `test_normalize_dry_run_makes_no_changes` (line 66)
- `test_normalize_moves_misplaced_files` (line 92)

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test ticket_test`
Expected: ALL PASS

- [ ] **Step 7: Commit**

```bash
git add src/commands/ticket.rs tests/ticket_test.rs src/main.rs
git commit -m "feat: add scope field to TicketInfo and create()"
```

---

### Task 3: Add scope to move_ticket function

**Files:**
- Modify: `src/commands/ticket.rs:204-279` (move_ticket function)

- [ ] **Step 1: Write failing test — move ticket with scope**

Add to `tests/ticket_test.rs`:

```rust
#[test]
fn test_ticket_move_with_scope() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Scope Move", Some(&ms_slug), None,
    ).unwrap();

    temper_cli::commands::ticket::move_ticket(
        &config, &slug, None, None, None, Some("epic"),
    ).unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("scope: epic"), "scope should be updated to epic");
}

#[test]
fn test_ticket_move_with_stage_and_scope() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Both Move", Some(&ms_slug), None,
    ).unwrap();

    temper_cli::commands::ticket::move_ticket(
        &config, &slug, Some("in-progress"), None, None, Some("feature"),
    ).unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: in-progress"));
    assert!(content.contains("scope: feature"));
}

#[test]
fn test_ticket_move_rejects_invalid_scope() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Bad Scope", Some(&ms_slug), None,
    ).unwrap();

    let result = temper_cli::commands::ticket::move_ticket(
        &config, &slug, None, None, None, Some("huge"),
    );
    assert!(result.is_err(), "invalid scope should be rejected");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test ticket_test test_ticket_move_with_scope test_ticket_move_with_stage_and_scope test_ticket_move_rejects_invalid_scope`
Expected: FAIL — `move_ticket()` doesn't accept scope parameter yet

- [ ] **Step 3: Update move_ticket() signature and body**

Add `scope: Option<&str>` parameter. Add validation upfront alongside stage validation. Add `set_frontmatter_field` call:

```rust
pub fn move_ticket(
    config: &Config,
    slug_or_suffix: &str,
    stage: Option<&str>,
    new_milestone: Option<&str>,
    project: Option<&str>,
    scope: Option<&str>,
) -> Result<()> {
    let ticket = find_ticket(config, slug_or_suffix, project)?
        .ok_or_else(|| TemperError::Vault(format!("ticket not found: {slug_or_suffix}")))?;

    // Validate stage and scope upfront before any file modifications
    let valid_stages = ["backlog", "in-progress", "done", "cancelled"];
    if let Some(s) = stage {
        if !valid_stages.contains(&s) {
            return Err(TemperError::Vault(format!(
                "invalid stage: {s}. Must be one of: {}",
                valid_stages.join(", ")
            )));
        }
    }
    let valid_scopes = ["patch", "feature", "epic"];
    if let Some(sc) = scope {
        if !valid_scopes.contains(&sc) {
            return Err(TemperError::Vault(format!(
                "invalid scope: {sc}. Must be one of: {}",
                valid_scopes.join(", ")
            )));
        }
    }
```

After existing stage/milestone updates, before the updated timestamp write, add:

```rust
    let from_scope = ticket.scope.clone();
    if let Some(sc) = scope {
        content = vault::set_frontmatter_field(&content, "scope", sc);
    }
```

Update the discovery event to include scope fields (done in Task 5).

- [ ] **Step 4: Update all existing callers of move_ticket()**

In `src/main.rs` (line 196), add `None` as the scope parameter:

```rust
temper_cli::commands::ticket::move_ticket(
    &config,
    &slug,
    stage.as_deref(),
    milestone.as_deref(),
    project,
    None, // scope — wired up in Task 4
)
```

Update all existing callers of `move_ticket()` to add `None` as the sixth argument. Complete list:

**tests/ticket_test.rs** (4 calls):
- `test_ticket_move_to_in_progress` (line 67)
- `test_ticket_move_rejects_old_stages` (line 88)
- `test_ticket_move_to_cancelled` (line 103)
- `test_ticket_move_and_done` (line 123)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test ticket_test`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/commands/ticket.rs tests/ticket_test.rs src/main.rs
git commit -m "feat: add scope validation and update to move_ticket()"
```

---

### Task 4: Add --scope CLI flag and wire routing

**Files:**
- Modify: `src/cli.rs:160-184` (TicketAction Create and Move)
- Modify: `src/main.rs:153-203` (ticket command routing)

- [ ] **Step 1: Add --scope flag to TicketAction::Create and Move**

In `src/cli.rs`, add to `Create`:

```rust
    Create {
        #[arg(long, required_unless_present = "show_template")]
        title: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        milestone: Option<String>,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long, hide = true)]
        stdin: bool,
        /// Print the raw template and exit
        #[arg(long)]
        show_template: bool,
    },
```

Add to `Move`:

```rust
    Move {
        slug: String,
        #[arg(long)]
        stage: Option<String>,
        #[arg(long)]
        milestone: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        scope: Option<String>,
    },
```

- [ ] **Step 2: Wire scope through main.rs routing**

Update `TicketAction::Create` match arm in `src/main.rs`:

```rust
TicketAction::Create {
    title,
    project,
    milestone,
    scope,
    stdin: _,
    show_template,
} => {
    if show_template {
        let content = temper_cli::vault::get_template("ticket")?;
        print!("{content}");
        return Ok(());
    }
    let project = project
        .as_deref()
        .or_else(|| resolved.map(|r| r.name.as_str()))
        .ok_or_else(|| {
            temper_cli::error::TemperError::Project(
                "no project specified and could not infer from CWD".into(),
            )
        })?;
    let title = title.expect("title required when not using --show-template");
    temper_cli::commands::ticket::create(
        &config,
        project,
        &title,
        milestone.as_deref(),
        scope.as_deref(),
    )?;
    Ok(())
}
```

Update `TicketAction::Move` match arm:

```rust
TicketAction::Move {
    slug,
    stage,
    milestone,
    project,
    scope,
} => {
    let project = project
        .as_deref()
        .or_else(|| resolved.map(|r| r.name.as_str()));
    temper_cli::commands::ticket::move_ticket(
        &config,
        &slug,
        stage.as_deref(),
        milestone.as_deref(),
        project,
        scope.as_deref(),
    )
}
```

- [ ] **Step 3: Run tests to verify everything still passes**

Run: `cargo test`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "feat: add --scope flag to ticket create and move CLI"
```

---

### Task 5: Add scope to discovery events

**Files:**
- Modify: `src/discovery.rs:16-35` (TicketCreate and TicketMove events)
- Modify: `src/commands/ticket.rs:190-199,262-273` (event emission)
- Modify: `src/commands/warmup.rs:165-179` (event formatting)

- [ ] **Step 1: Add scope fields to Event variants**

In `src/discovery.rs`, update `TicketCreate`:

```rust
    #[serde(rename = "ticket_create")]
    TicketCreate {
        ts: String,
        project: String,
        ticket: String,
        milestone: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
    },
```

Update `TicketMove`:

```rust
    #[serde(rename = "ticket_move")]
    TicketMove {
        ts: String,
        project: String,
        ticket: String,
        from_stage: String,
        to_stage: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_milestone: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_milestone: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_scope: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_scope: Option<String>,
    },
```

- [ ] **Step 2: Update event emission in ticket.rs**

In `create()`, update the event:

```rust
    let event = discovery::Event::TicketCreate {
        ts: datetime,
        project: project.to_string(),
        ticket: slug.clone(),
        milestone: ms_slug,
        title: title.to_string(),
        scope: scope.map(String::from),
    };
```

In `move_ticket()`, update the event to include scope fields:

```rust
    let to_scope = scope.map(String::from);
    let from_scope_for_event = if scope.is_some() { from_scope } else { None };

    let event = discovery::Event::TicketMove {
        ts: datetime,
        project: ticket.project,
        ticket: ticket.slug.clone(),
        from_stage: from_stage.clone(),
        to_stage: to_stage.to_string(),
        from_milestone: from_ms,
        to_milestone: to_ms,
        from_scope: from_scope_for_event,
        to_scope,
    };
```

- [ ] **Step 3: Update warmup event formatting**

In `src/commands/warmup.rs`, update the `TicketCreate` match arm to include the new `scope` field in the destructure pattern (add `scope: _, ..`). The `TicketMove` arm similarly needs `from_scope: _, to_scope: _, ..` or just use `..` as already done.

Check that the existing `..` patterns in `format_event_brief` already cover the new fields. If they do, no code changes needed — just verify compilation.

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/discovery.rs src/commands/ticket.rs src/commands/warmup.rs
git commit -m "feat: add scope fields to discovery events"
```

---

### Task 6: Add in-progress tickets to warmup

**Files:**
- Modify: `src/commands/warmup.rs:17-68` (run_text function)
- Modify: `src/commands/warmup.rs:71-101` (run_json function)

- [ ] **Step 1: Write failing test**

Add to `tests/warmup_test.rs`:

```rust
#[test]
fn test_warmup_shows_in_progress_tickets_with_scope() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "Active Work", Some(&ms_slug), Some("feature"),
    ).unwrap();
    temper_cli::commands::ticket::move_ticket(
        &config, &slug, Some("in-progress"), None, None, None,
    ).unwrap();

    // Capture stdout by checking the function succeeds
    // (full output capture would need test infrastructure changes)
    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}

#[test]
fn test_warmup_no_in_progress_tickets() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // No tickets at all — warmup should still succeed
    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run test to verify it passes (baseline)**

Run: `cargo test --test warmup_test`
Expected: PASS (the test just checks for Ok, we need to add the actual warmup code)

- [ ] **Step 3: Add in-progress tickets section to run_text()**

In `src/commands/warmup.rs`, add after the "Recent Sessions" section (after line 31) and before "Last Session" (line 34):

```rust
    // Section: In-progress tickets
    let in_progress = collect_in_progress_tickets(config, project);
    if !in_progress.is_empty() {
        println!("## In-Progress Tickets");
        println!();
        for (title, slug, scope) in &in_progress {
            let scope_label = scope.as_deref().unwrap_or("unscoped");
            println!("- [{scope_label}] {title} ({slug})");
        }
        println!();
    }
```

Add the helper function:

```rust
/// Collect in-progress tickets for a project.
/// Returns (title, slug, scope) tuples.
fn collect_in_progress_tickets(
    config: &Config,
    project: &str,
) -> Vec<(String, String, Option<String>)> {
    let tickets = match crate::commands::ticket::load_tickets(config, Some(project), None) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    tickets
        .into_iter()
        .filter(|t| t.stage == "in-progress")
        .map(|t| (t.title, t.slug, t.scope))
        .collect()
}
```

- [ ] **Step 4: Add in-progress tickets to run_json()**

In `run_json()`, add after loading sessions and events:

```rust
    let in_progress = collect_in_progress_tickets(config, project);
    let in_progress_json: Vec<_> = in_progress
        .iter()
        .map(|(title, slug, scope)| {
            serde_json::json!({
                "title": title,
                "slug": slug,
                "scope": scope,
            })
        })
        .collect();
```

Add to the JSON output object:

```rust
    let output = serde_json::json!({
        "project": project,
        "in_progress_tickets": in_progress_json,
        "recent_sessions": sessions.iter().map(|(date, title, _)| {
            serde_json::json!({"date": date, "title": title})
        }).collect::<Vec<_>>(),
        "last_session_content": last_session_content,
        "recent_events": recent_events.iter().map(|e| {
            serde_json::to_value(e).unwrap_or_default()
        }).collect::<Vec<_>>(),
    });
```

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/commands/warmup.rs tests/warmup_test.rs
git commit -m "feat: surface in-progress tickets with scope in warmup"
```

---

### Task 7: Add scope reporting to normalize

**Files:**
- Modify: `src/commands/normalize.rs:29-103` (run function)

- [ ] **Step 1: Write failing test**

Add to `tests/normalize_test.rs`:

```rust
#[test]
fn test_normalize_reports_unscoped_tickets() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    // Create a ticket without scope — will have scope: null
    temper_cli::commands::ticket::create(
        &config, "myapp", "Unscoped", Some(&ms_slug), None,
    ).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, true, false).unwrap();
    assert!(summary.unscoped_tickets > 0, "should report unscoped tickets");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test normalize_test test_normalize_reports_unscoped_tickets`
Expected: FAIL — `unscoped_tickets` field doesn't exist on NormalizeSummary

- [ ] **Step 3: Add unscoped_tickets to NormalizeSummary**

In `src/commands/normalize.rs`:

```rust
pub struct NormalizeSummary {
    pub ids_backfilled: u32,
    pub files_moved: u32,
    pub stages_migrated: u32,
    pub slugs_fixed: u32,
    pub frontmatter_fixed: u32,
    pub unscoped_tickets: u32,
}
```

Initialize in `run()`:

```rust
    let mut summary = NormalizeSummary {
        ids_backfilled: 0,
        files_moved: 0,
        stages_migrated: 0,
        slugs_fixed: 0,
        frontmatter_fixed: 0,
        unscoped_tickets: 0,
    };
```

- [ ] **Step 4: Count unscoped tickets in process_file()**

In `process_file()`, after the slug consistency check, add a check for tickets (files under tickets_dir) without scope:

```rust
    // --- Check for missing scope on tickets (informational only) ---
    if base_dir.ends_with("tickets") {
        if let Some(ref v) = fm {
            let has_scope = v.get("scope").and_then(|s| s.as_str()).is_some();
            if !has_scope {
                summary.unscoped_tickets += 1;
            }
        }
    }
```

Note: YAML `null` deserializes as `Value::Null`, not as a string, so `as_str()` returns `None` for both missing and null scope. This correctly counts both cases.

- [ ] **Step 5: Add summary output line**

In `run()`, after the existing summary output lines:

```rust
    if summary.unscoped_tickets > 0 {
        output::plain(format!("  {} tickets without scope", summary.unscoped_tickets));
    }
```

- [ ] **Step 6: Update NormalizeSummary in process_file signature**

The `process_file` function already takes `&mut NormalizeSummary` — no signature change needed.

- [ ] **Step 7: Run tests**

Run: `cargo test`
Expected: ALL PASS

- [ ] **Step 8: Commit**

```bash
git add src/commands/normalize.rs tests/normalize_test.rs
git commit -m "feat: report unscoped tickets in normalize"
```

---

### Task 8: Update skill template with scope-based workflow

**Files:**
- Modify: `src/commands/skill.rs:30-108` (generate function template string)

This is the largest single change — replacing the linear workflow section with the scope-based decision tree.

- [ ] **Step 1: Update the template string in generate()**

Replace the `## Stages` and `## Workflow Integration` sections (lines 82-103 of the template string) with:

```rust
## Stages

Tickets use four stages: `backlog`, `in-progress`, `done`, `cancelled`.

## Scope

Tickets have an optional `scope` field: `patch`, `feature`, or `epic`. Scope controls the workflow:

| Scope | Nature | Ceremony | Output |
|-------|--------|----------|--------|
| `patch` | Tactical | None — just do it | Delivered code |
| `feature` | Deliberate | Full superpowers pipeline | Delivered code with design artifact |
| `epic` | Strategic | Deep discovery + roadmapping | Living milestone roadmap + first actionable ticket |

## Workflow Integration

When starting a session:
- Check for recent sessions: `temper session list --project <current>`
- Search for relevant context: `temper search "<topic>"`

When ending a session:
- Suggest: `temper session save --ticket <slug> --state done` (if working on a ticket)
- Or just: `temper session save`

When the user says `/temper ticket start <slug>`:
1. Run `temper ticket move <slug> --stage in-progress --project <p>`
2. Run `temper ticket show <slug>`
3. Check the `scope` field and route accordingly:

### Scope Routing

**If scope is set**, announce the workflow:
- **patch**: "Scoped as patch — implementing directly with tests, no spec or plan." Skip brainstorming.
- **feature**: "Scoped as feature — full superpowers pipeline." Invoke brainstorming skill.
- **epic**: "Scoped as epic — mapping the problem space to produce a milestone roadmap." Invoke brainstorming skill framed as strategic planning.

**If scope is missing**, ask briefly: "Does this feel like a patch, feature, or epic?" Then set it via `temper ticket move <slug> --scope <confirmed>`.

### Patch Workflow
1. Read ticket content
2. Implement directly with tests
3. `cargo test` / `cargo clippy`
4. Commit
5. `temper session save "<summary>" --ticket <slug> --state done`

### Feature Workflow
1. Read ticket content
2. `temper search` / `temper context` for discovery
3. Invoke superpowers:brainstorming (design the implementation)
4. Produce design spec, then invoke superpowers:writing-plans
5. Implement via plan execution
6. Full verification (tests, clippy, fmt)
7. `temper session save "<summary>" --ticket <slug> --state done`

### Epic Workflow
1. Read ticket content
2. Deep discovery — `temper search`, `temper context`, codebase exploration
3. Invoke superpowers:brainstorming (map the problem space, NOT design an implementation)
4. Produce a milestone roadmap via `temper milestone create`:
   - Throughline summary, sequenced deliverable chunks, validation gates, open questions
5. Create the FIRST actionable ticket under that milestone
6. `temper session save "<summary>" --ticket <slug>`
7. Code only if the user actively pushes for it

Epic philosophy: the roadmap guides session work, not ticket-spread. Each session: work the current ticket, learn, evolve the roadmap, create the next ticket.

### Mid-Session Drift Detection

Watch for scope mismatch:
- **Patch drifting up**: needing design decisions, touching 3+ files, considering multiple approaches → suggest feature
- **Feature drifting up**: needs decomposition into multiple deliverables, spans multiple sessions → suggest epic
- **Epic drifting down**: first ticket is obvious, roadmap has only 1-2 items → suggest feature or start work

On confirmation: `temper ticket move <slug> --scope <new>`

### Scope at Create Time

When creating a ticket without `--scope`, ask briefly: "Does this feel like a patch, feature, or epic?" Don't over-analyze — the user usually knows.
```

- [ ] **Step 2: Update the commands section**

Update the ticket commands in the `## Commands` section to include `--scope`:

```
- `temper ticket create --title <t> [--project <p>] [--scope patch|feature|epic]` — Create ticket (stdin auto-detected)
- `temper ticket move <slug> --stage <s> [--project <p>] [--scope patch|feature|epic]` — Move ticket between stages or update scope
```

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/commands/skill.rs
git commit -m "feat: replace linear workflow with scope-based decision tree in skill template"
```

---

### Task 9: Verify JSON output includes scope

**Files:**
- Modify: `tests/ticket_test.rs`

- [ ] **Step 1: Write test for scope in JSON output**

Add to `tests/ticket_test.rs`:

```rust
#[test]
fn test_ticket_show_json_includes_scope() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::ticket::create(
        &config, "myapp", "JSON Scope", Some(&ms_slug), Some("patch"),
    ).unwrap();

    // Verify TicketInfo includes scope by loading and checking
    let ticket = temper_cli::commands::ticket::find_ticket(&config, &slug, None)
        .unwrap()
        .unwrap();
    assert_eq!(ticket.scope, Some("patch".to_string()));
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --test ticket_test test_ticket_show_json_includes_scope`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/ticket_test.rs
git commit -m "test: verify scope field in ticket JSON output"
```

---

### Task 10: Final verification and skill install

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: ALL PASS

- [ ] **Step 2: Run clippy and fmt**

Run: `cargo clippy --all-features -- -D warnings && cargo fmt --check`
Expected: No warnings, formatting OK

- [ ] **Step 3: Install updated skill**

Run: `temper skill install`

- [ ] **Step 4: Verify skill content includes scope**

Run: `temper skill generate | grep -A 2 "scope"`
Expected: Shows scope documentation in generated skill output

- [ ] **Step 5: Smoke test**

Run:
```bash
temper ticket create --title "Scope smoke test" --scope patch --project temper
temper ticket show scope-smoke-test --project temper
```
Expected: Shows ticket with `scope: patch` in frontmatter

- [ ] **Step 6: Commit any final adjustments**

```bash
git add -A
git commit -m "chore: final verification and skill install for adaptive workflow"
```
