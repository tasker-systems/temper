# Resource Body Update: Local-Mode Wiring and Silent-Success Guard

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire `--body @<path>` / `--body -` / auto-stdin through the local-mode `temper resource update` action so it actually rewrites body content (currently silently dropped), and add a guard so explicit `--body @empty.md` and `cat /dev/null | temper resource update --body -` error rather than write empty bodies.

**Architecture:** The CLI flag and `resolve_body_source` already exist and are correctly wired in cloud mode. The local-mode `update` function in `crates/temper-cli/src/commands/resource.rs` parses the file via `Frontmatter::parse_file`, mutates managed/open fields, and calls `Frontmatter::write_to` — but it never consumes `params.body`. The fix: call `resolve_body_source`, and if it yields `Some(body)`, call `fm.set_body(body)` (which already exists at `temper-core/src/frontmatter/document.rs:311`) before `write_to`. The silent-success guard lives inside `resolve_body_source` at the `@<path>` and `-` arms — both currently return `Ok(Some(""))` for empty content, which should become an error. The implicit-stdin empty-content path keeps returning `Ok(None)` (this is the spawned-thread test-harness safeguard documented in the existing tests).

**Tech Stack:** Rust workspace, clap CLI, `Frontmatter` round-trip from `temper-core`, integration tests using `tempfile::TempDir` + `temper_cli::config::Config` (pattern from `crates/temper-cli/tests/actions_task_test.rs`), e2e tests under `tests/e2e/` driving the CLI through the live API.

---

## File Structure

| File | Change |
|------|--------|
| `crates/temper-cli/src/actions/body_source.rs` | Modify — add explicit-empty error in `@<path>` and `-` arms; keep implicit-empty as `Ok(None)`. Extend unit tests. |
| `crates/temper-cli/src/commands/resource.rs` | Modify — `update` (local-mode) calls `resolve_body_source` and applies `fm.set_body` before `write_to`. |
| `crates/temper-cli/src/cli.rs` | Modify — update help text on `Update` subcommand. |
| `crates/temper-cli/tests/resource_body_update_test.rs` | Create — local-mode integration tests across goal/task/session doctypes. |
| `tests/e2e/tests/resource_body_update_e2e_test.rs` | Create — full read → cat-update → read cycle through the live Axum server (cloud mode) plus a local-mode round-trip. |
| `CLAUDE.md` | Modify — clarify that the show-edit-cat flow now works in both modes. |

No new modules, no new types. The fix is purely behavioral wiring + a guard.

---

## Task 1: Tighten `resolve_body_source` so explicit-empty errors

**Files:**
- Modify: `crates/temper-cli/src/actions/body_source.rs`

**Context for implementer:** `resolve_body_source` is the single point where `--body @<path>`, `--body -`, and implicit non-TTY stdin are resolved into `Option<String>`. Currently, an empty file at `@path` and an empty `-` stdin both return `Ok(Some(""))` — which downstream callers happily turn into "PATCH the body to empty string." That's the silent-success bug surface. The fix: those two explicit forms must error. The implicit-stdin empty path stays as `Ok(None)` because the existing test `implicit_returns_none_for_empty_stdin` documents an intentional safeguard against unconnected stdin in spawned-thread test harnesses.

- [ ] **Step 1: Write failing tests for the explicit-empty guards**

Add the following tests inside `mod tests` in `crates/temper-cli/src/actions/body_source.rs`:

```rust
    #[test]
    fn errors_when_at_path_file_is_empty() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "").unwrap();
        let result = resolve_body_source(
            Some(format!("@{}", temp.path().display())),
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
        );
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("empty"),
            "expected empty-body error, got: {msg}"
        );
    }

    #[test]
    fn errors_when_explicit_dash_stdin_is_empty() {
        let result = resolve_body_source(
            Some("-".to_string()),
            /*stdin_is_tty:*/ false,
            Cursor::new(b""),
        );
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("empty"),
            "expected empty-body error, got: {msg}"
        );
    }
```

- [ ] **Step 2: Run the new tests to verify they fail**

Run: `cargo nextest run -p temper-cli --lib body_source::tests::errors_when_at_path_file_is_empty body_source::tests::errors_when_explicit_dash_stdin_is_empty`

Expected: both FAIL — current code returns `Ok(Some(""))` for both inputs.

- [ ] **Step 3: Implement the explicit-empty guards**

In `crates/temper-cli/src/actions/body_source.rs`, change the `@<path>` and `-` match arms in `resolve_body_source`:

```rust
        Some(s) if s.starts_with('@') => {
            let path = &s[1..];
            let content = std::fs::read_to_string(path)
                .map_err(|e| TemperError::Vault(format!("read --body @{path}: {e}")))?;
            if content.is_empty() {
                return Err(TemperError::Project(format!(
                    "--body @{path} resolved to empty content; refusing to write empty body"
                )));
            }
            Ok(Some(content))
        }
        Some("-") => {
            if stdin_is_tty {
                return Err(TemperError::Project(
                    "--body - requires non-TTY stdin".to_string(),
                ));
            }
            let mut buf = String::new();
            stdin_reader
                .read_to_string(&mut buf)
                .map_err(|e| TemperError::Vault(format!("read stdin: {e}")))?;
            if buf.is_empty() {
                return Err(TemperError::Project(
                    "--body - resolved to empty stdin; refusing to write empty body"
                        .to_string(),
                ));
            }
            Ok(Some(buf))
        }
```

Leave the implicit `None` arm unchanged — it must keep returning `Ok(None)` for empty stdin per the safeguard documented in `implicit_returns_none_for_empty_stdin`.

- [ ] **Step 4: Run the new tests and the existing suite to verify**

Run: `cargo nextest run -p temper-cli --lib body_source`

Expected: PASS for all tests in `body_source::tests`, including the existing implicit-empty-returns-none test (which proves the safeguard is preserved).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/body_source.rs
git commit -m "$(cat <<'EOF'
fix(cli): error on explicit empty --body @path or --body -

Previously --body @empty.md and `: | temper update --body -` both
resolved to Ok(Some("")) and silently wrote empty bodies. The implicit
empty-stdin path is unchanged (intentional safeguard for spawned-thread
test harnesses) — only the explicit forms are tightened.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Local-mode `update` consumes `params.body` and rewrites file body

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`update` function ~line 1331)

**Context for implementer:** The local-mode `update` in `crates/temper-cli/src/commands/resource.rs:1331` currently parses the file with `Frontmatter::parse_file`, mutates frontmatter scalars/arrays, optionally moves the file, then calls `fm.write_to(&final_path)`. `params.body` is in scope but never read. `Frontmatter::set_body(body: String)` exists at `crates/temper-core/src/frontmatter/document.rs:311` and `write_to` writes the full file (frontmatter + body), so the wiring is mechanical. Insert the body resolution after the schema-validation block (so frontmatter-validation errors still take precedence) and before the `temper-updated` timestamp is written.

The cloud-mode path (`cloud_mode_update`) at the same file already calls `resolve_body_source` — keep it as-is. Task 1's guard tightens its behavior for explicit empty inputs without further changes here.

- [ ] **Step 1: Write the failing local-mode body-rewrite test**

Create `crates/temper-cli/tests/resource_body_update_test.rs` with the following content:

```rust
//! Local-mode tests for `temper resource update --body @path`.
//!
//! These exercise the wire-through: that --body actually rewrites the
//! file body in local mode (before this task, the flag was silently
//! ignored).

use tempfile::TempDir;

mod common;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    common::init_isolated_auth();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
    }
}

fn write_body_file(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn read_body(file: &std::path::Path) -> String {
    let raw = std::fs::read_to_string(file).unwrap();
    // Strip frontmatter: everything after the second "---\n".
    let after_first = raw.split_once("---\n").map(|(_, r)| r).unwrap_or(&raw);
    let after_second = after_first
        .split_once("---\n")
        .map(|(_, r)| r)
        .unwrap_or(after_first);
    after_second.to_string()
}

#[test]
fn local_mode_update_rewrites_goal_body_via_body_at_path() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let slug =
        temper_cli::commands::goal::create(&config, "myapp", "Sample goal", None, "text").unwrap();
    let goal_dir = dir
        .path()
        .join("@me")
        .join("myapp")
        .join("goal");
    let goal_file = goal_dir.join(format!("{slug}.md"));
    assert!(goal_file.exists(), "goal file should be created");

    let new_body_path = write_body_file(&dir, "new_body.md", "# Rewritten\n\nNew body content.\n");

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &slug,
        doc_type: Some("goal"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: None,
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: Some(format!("@{}", new_body_path.display())),
    };

    temper_cli::commands::resource::update(&config, &params).unwrap();

    let body = read_body(&goal_file);
    assert!(
        body.contains("New body content."),
        "body should be rewritten; got: {body}"
    );
    assert!(
        body.contains("# Rewritten"),
        "body should contain new H1; got: {body}"
    );
}
```

> **Note for implementer:** check that `UpdateParams` is `pub` and that `commands::resource::update` is `pub`. They are referenced from cloud-mode tests but if either is `pub(crate)`, expose them publicly and adjust the test imports accordingly.

- [ ] **Step 2: Run the new test to verify it fails**

Run: `cargo nextest run -p temper-cli --test resource_body_update_test`

Expected: FAIL on the `body.contains("New body content.")` assertion — local mode currently drops the body, so the file still contains the template body from `goal::create`.

- [ ] **Step 3: Wire `params.body` through local-mode `update`**

In `crates/temper-cli/src/commands/resource.rs`, in the `update` function (currently starting at line 1331), insert body resolution **after** the schema-validation loop ends (after the closing `}` of the `for (field_name, value) in &scalar_updates` validation loop, around line 1398) and **before** `Frontmatter::parse_file` is called (~line 1402). Then apply the body **after** all scalar/array/move mutations and **before** `fm.write_to`.

Replace this block:

```rust
    // Parse the file once, apply all mutations to the aggregate, then write
    // exactly once to the (potentially moved) final path.
    let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&path)?;
```

with:

```rust
    // Resolve --body before reading the file so a malformed flag fails fast,
    // before any side effects. None means "no body update requested" — leave
    // the existing on-disk body untouched.
    let resolved_body = {
        use std::io::IsTerminal;
        let stdin_is_tty = std::io::stdin().is_terminal();
        crate::actions::body_source::resolve_body_source(
            params.body.clone(),
            stdin_is_tty,
            std::io::stdin(),
        )?
    };

    // Parse the file once, apply all mutations to the aggregate, then write
    // exactly once to the (potentially moved) final path.
    let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&path)?;
```

Then, immediately before `fm.write_to(&final_path)?;` (currently around line 1488), add:

```rust
    if let Some(new_body) = resolved_body {
        fm.set_body(new_body);
    }

```

Do not modify the `temper-updated` timestamp logic — it already runs before `write_to`, which is correct.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run -p temper-cli --test resource_body_update_test`

Expected: PASS.

- [ ] **Step 5: Run the full CLI test suite to verify no regressions**

Run: `cargo nextest run -p temper-cli`

Expected: PASS for all CLI tests.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/tests/resource_body_update_test.rs
git commit -m "$(cat <<'EOF'
fix(cli): wire --body through local-mode resource update

Local-mode update was silently ignoring params.body — frontmatter
mutations were applied and written via Frontmatter::write_to, but the
existing body was preserved unchanged. Pipe a body to a goal-doc update
in local mode and the CLI prints "Updated" while your content vanishes.

Resolve --body via the same body_source::resolve_body_source path the
cloud-mode update uses, then apply via Frontmatter::set_body before
write_to. None means "no body update requested" — existing behavior is
preserved when no body is supplied.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Mode-parity coverage across goal / task / session doctypes

**Files:**
- Modify: `crates/temper-cli/tests/resource_body_update_test.rs`

**Context for implementer:** Task 2 covered the goal doctype. The acceptance criterion calls for at least three doctypes; this task adds task and session, plus an explicit-stdin (`--body -`) variant via `Cursor`-backed reader. We can't easily simulate `--body -` through the public `update` action because it reads the process's real stdin — instead, stage the body in a file and use `--body @path` for the multi-doctype assertion, and unit-test the `-` path through `body_source.rs` (already covered by `resolves_explicit_dash_reads_stdin`).

- [ ] **Step 1: Add task and session body-rewrite tests**

Append to `crates/temper-cli/tests/resource_body_update_test.rs`:

```rust
#[test]
fn local_mode_update_rewrites_task_body_via_body_at_path() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let goal_slug =
        temper_cli::commands::goal::create(&config, "myapp", "Parent goal", None, "text").unwrap();
    let task_slug = temper_cli::actions::task::create(
        &config,
        "myapp",
        "Sample task",
        Some(&goal_slug),
        None,
        None,
        None,
    )
    .unwrap();
    let task_file = dir
        .path()
        .join("@me")
        .join("myapp")
        .join("task")
        .join(format!("{task_slug}.md"));

    let new_body_path = write_body_file(
        &dir,
        "task_body.md",
        "# Task work log\n\nDay 1: started.\n",
    );

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &task_slug,
        doc_type: Some("task"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: None,
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: Some(format!("@{}", new_body_path.display())),
    };

    temper_cli::commands::resource::update(&config, &params).unwrap();

    let body = read_body(&task_file);
    assert!(
        body.contains("Day 1: started."),
        "task body should be rewritten; got: {body}"
    );
}

#[test]
fn local_mode_update_rewrites_session_body_via_body_at_path() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let session_slug = temper_cli::commands::session::create(
        &config,
        "myapp",
        "Working session",
        None,
        None,
    )
    .unwrap();
    let session_file = dir
        .path()
        .join("@me")
        .join("myapp")
        .join("session")
        .join(format!("{session_slug}.md"));

    let new_body_path = write_body_file(
        &dir,
        "session_body.md",
        "# Session notes\n\nDecisions: shipped X.\n",
    );

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &session_slug,
        doc_type: Some("session"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: None,
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: Some(format!("@{}", new_body_path.display())),
    };

    temper_cli::commands::resource::update(&config, &params).unwrap();

    let body = read_body(&session_file);
    assert!(
        body.contains("Decisions: shipped X."),
        "session body should be rewritten; got: {body}"
    );
}

#[test]
fn local_mode_update_no_body_flag_preserves_existing_body() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let slug =
        temper_cli::commands::goal::create(&config, "myapp", "Preserved goal", None, "text").unwrap();
    let goal_file = dir
        .path()
        .join("@me")
        .join("myapp")
        .join("goal")
        .join(format!("{slug}.md"));
    let original_body = read_body(&goal_file);

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &slug,
        doc_type: Some("goal"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: Some("Renamed goal"),
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: None,
    };

    temper_cli::commands::resource::update(&config, &params).unwrap();

    let body = read_body(&goal_file);
    assert_eq!(body, original_body, "body must be unchanged when --body is omitted");
}
```

> **Note for implementer:** verify the exact signatures of `goal::create`, `actions::task::create`, and `session::create` in the current code — copy from `actions_task_test.rs` and adjust if any signature differs (e.g. extra `Option<...>` parameter). The contract this test relies on is: each `create` returns the slug.

- [ ] **Step 2: Run the new tests**

Run: `cargo nextest run -p temper-cli --test resource_body_update_test`

Expected: PASS for all four tests in this file (goal, task, session, no-flag-preserves-body).

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/tests/resource_body_update_test.rs
git commit -m "$(cat <<'EOF'
test(cli): cover local-mode body update across goal/task/session and no-flag preservation

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: E2E coverage for cloud-mode body update via `--body @path`

**Files:**
- Create: `tests/e2e/tests/resource_body_update_e2e_test.rs`

**Context for implementer:** Cloud-mode body PATCH is already covered at the API layer by `crates/temper-api/tests/resource_update_body_test.rs`. What's missing is an end-to-end test that drives the **CLI** flag through to the live Axum server, proving that `--body @path` survives the full path: clap → `cloud_mode_update` → `resolve_body_source` → `client.resources().update` → handler → service → DB → response. Use the existing e2e harness pattern from `tests/e2e/tests/resource_crud_test.rs` (the CRUD test) as the template — specifically how it spawns the server, gets an authed client, creates a resource, and exercises the CLI.

- [ ] **Step 1: Read the existing e2e CRUD pattern**

Run: `cat tests/e2e/tests/resource_crud_test.rs | head -120`

Expected: see how the harness boots an Axum server, registers a profile, sets `TEMPER_VAULT_STATE=cloud`, and drives `temper_cli::commands::resource::*` against it. Note the helper modules under `tests/e2e/tests/common/`.

- [ ] **Step 2: Write the failing e2e test**

Create `tests/e2e/tests/resource_body_update_e2e_test.rs`. Model it on the most analogous existing test (`resource_crud_test.rs`). The test must:

1. Boot the e2e harness (server + authed CLI config).
2. Set `TEMPER_VAULT_STATE=cloud`.
3. Create a goal via `temper_cli::commands::resource::create` (or whichever code path the CRUD test uses) with body `# Initial\n`.
4. Stage a body file with `# Updated via flag\n\nNew content.\n`.
5. Call `temper_cli::commands::resource::update` with `body: Some(format!("@{}", body_path.display()))`.
6. Fetch the resource back from the server (via the existing client helper or `resource::show` action).
7. Assert the returned body contains `"New content."`.

Keep the test under 80 lines — copy and adapt from the CRUD test's harness setup, don't rebuild it.

- [ ] **Step 3: Run the new test**

Run: `cargo make test-e2e` (or, for faster iteration, `cargo nextest run -p temper-e2e --test resource_body_update_e2e_test --features test-db`).

Expected: PASS. If it fails because of harness setup (auth/test-db features), look for a comparable pattern in `resource_crud_test.rs` — do not invent new harness scaffolding.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/resource_body_update_e2e_test.rs
git commit -m "$(cat <<'EOF'
test(e2e): cover cloud-mode --body @path end-to-end

Drives the CLI flag through the live Axum server: clap → resolve_body_source
→ client → handler → service → DB → response.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Update `Update` subcommand help text

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:283-288`

- [ ] **Step 1: Replace the help text**

In `crates/temper-cli/src/cli.rs`, replace lines 283-288:

```rust
    /// Update a resource's frontmatter fields and push to server
    ///
    /// Update mutates frontmatter from args and pushes the whole file
    /// (including manual body edits) to the server in one operation. Make
    /// body edits before running update. For body-only changes use
    /// `temper push`.
    Update {
```

with:

```rust
    /// Update a resource's frontmatter and/or body
    ///
    /// Mutates frontmatter from flag args. Optionally rewrites the body
    /// via `--body @<path>` (file), `--body -` (explicit stdin), or
    /// implicit non-TTY stdin (e.g. `cat new.md | temper resource update <slug>`).
    /// Works in both local and cloud mode; in local mode the file is
    /// rewritten and best-effort published; in cloud mode the body trio
    /// (content + content_hash + chunks_packed) is PATCHed in one call.
    Update {
```

- [ ] **Step 2: Verify help text in built CLI**

Run: `cargo run -p temper-cli --quiet -- resource update --help | head -25`

Expected: the new help text appears under `temper resource update --help`.

- [ ] **Step 3: Run lint to confirm no formatting drift**

Run: `cargo make check`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/cli.rs
git commit -m "$(cat <<'EOF'
docs(cli): update `resource update --help` to describe --body across modes

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Update CLAUDE.md to reflect mode parity

**Files:**
- Modify: `CLAUDE.md` (the show-edit-cat paragraph, currently around line 116)

**Context for implementer:** The current paragraph is mostly accurate but conflates the show-edit-cat idiom with cloud mode specifically. Now that local mode supports the same flag and pipe behaviors, the paragraph should describe both modes uniformly.

- [ ] **Step 1: Replace the paragraph**

In `CLAUDE.md`, find the paragraph that begins:

> "**Cloud mode operations** — When `TEMPER_VAULT_STATE=cloud`, write paths route directly through the API: ..."

Replace the sentences from "Body edits use the show-edit-cat idiom:" through the end of the paragraph with:

```
Body edits work uniformly in both modes via three forms: `--body @<path>` reads from a file, `--body -` reads from stdin explicitly, and implicit stdin is auto-detected when stdin is non-TTY (e.g. `cat tmpfile.md | temper resource update <slug>`). Explicit empty input (`--body @empty.md` or piping no bytes via `--body -`) errors rather than writing an empty body; implicit empty stdin is treated as "no body update requested" so frontmatter-only updates work without piping. The show-edit-cat idiom — `temper resource show <slug>` writes the current body to a temp path, modify it, then `cat tmpfile.md | temper resource update <slug> --stage done` — works in both local and cloud modes; in local mode the vault file is rewritten and best-effort published, in cloud mode the body trio (content + content_hash + chunks_packed) is PATCHed in one call alongside any frontmatter flags.
```

Leave the sentence about "Do not invoke `temper sync run` in cloud mode" untouched — that is independent of the body-update path.

- [ ] **Step 2: Verify the paragraph reads coherently**

Run: `grep -A 6 "Cloud mode operations" CLAUDE.md`

Expected: the paragraph flows correctly with the new body-edits sentences.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "$(cat <<'EOF'
docs(claude.md): reflect that --body flag works uniformly in both modes

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Full verification before claiming done

**Files:** none

**Context for implementer:** This is the verification gate. Do not skip — the original bug was a verification gap.

- [ ] **Step 1: Run the full quality gate**

Run: `cargo make check`

Expected: PASS (fmt + clippy + docs + machete + biome).

- [ ] **Step 2: Run all Rust unit and integration tests (no DB)**

Run: `cargo make test`

Expected: PASS.

- [ ] **Step 3: Run integration tests (DB required)**

```bash
cargo make docker-up
cargo make test-db
```

Expected: PASS, including the existing `crates/temper-api/tests/resource_update_body_test.rs` cloud-mode body trio coverage.

- [ ] **Step 4: Run the e2e test from Task 4**

Run: `cargo make test-e2e`

Expected: PASS, including the new `resource_body_update_e2e_test`.

- [ ] **Step 5: Manually verify the bug fix**

```bash
# Local mode
mkdir -p /tmp/temper-bug-repro && cd /tmp/temper-bug-repro
TEMPER_VAULT_STATE= temper init --context test
TEMPER_VAULT_STATE= temper resource create --type goal --title "Smoke test goal" --context test <<EOF
# Smoke test goal

Original body.
EOF

echo "# Replaced

This body came in via cat-pipe." | TEMPER_VAULT_STATE= temper resource update smoke-test-goal --type goal --context test

TEMPER_VAULT_STATE= temper resource show smoke-test-goal --type goal --context test
```

Expected: the displayed body contains "This body came in via cat-pipe." — confirming the fix.

```bash
# Explicit empty guard
echo -n "" | TEMPER_VAULT_STATE= temper resource update smoke-test-goal --type goal --context test --body -
```

Expected: error — "--body - resolved to empty stdin; refusing to write empty body".

- [ ] **Step 6: Mark the task complete and save the session**

This step is delegated back to the controller — do not commit a session note from inside the implementation. Hand control back with a summary of what was built, what tests pass, and any deviations from the plan.

---

## Self-Review Notes

**Spec coverage check:**
- [x] `--body @path` in local mode → Task 2.
- [x] `--body -` in local mode → relies on Task 2 (path goes through `resolve_body_source` which already handles `-`); explicit-dash unit coverage already exists in `body_source.rs`.
- [x] Auto-stdin in local mode → relies on Task 2 (same `resolve_body_source` path).
- [x] Silent-success guard → Task 1 (covers `@empty.md` and `cat /dev/null | --body -`).
- [x] Mode parity → Tasks 2 + 4 (local integration + cloud e2e); cloud-mode trio already covered by `resource_update_body_test.rs`.
- [x] Doctype coverage (3+ doctypes) → Task 3 (goal, task, session).
- [x] Help text update → Task 5.
- [x] CLAUDE.md update → Task 6.
- [x] Tests for stdin auto-detect → already exist in `body_source.rs::tests::implicit_uses_stdin_when_non_tty`; the local-mode wire-through is covered by Task 2.
- [x] No-body-flag preserves existing body → Task 3 explicit test.

**Out of scope (not promoted to tasks):**
- Memory doctype support (gated on goal #9).
- `--clear-meta` PUT semantics (goal #6).
- Cross-context/doctype moves (goal #5).
- `temper resource create` body source — already works in cloud mode and is unchanged here.
- Server-side empty-body validation — guard is at the CLI seam; server keeps accepting empty content for callers that legitimately want it (which today is none, but that's not this task's call to make).

**Type/signature consistency:**
- Task 2 introduces no new types. Tests in Task 3 reuse the same `UpdateParams` shape from Task 2.
- `Frontmatter::set_body(body: String)` confirmed at `temper-core/src/frontmatter/document.rs:311`.
- `resolve_body_source` signature confirmed at `temper-cli/src/actions/body_source.rs:14-18`.
- Test helper `common::init_isolated_auth()` confirmed at `crates/temper-cli/tests/common/mod.rs` (used by `actions_task_test.rs`).
