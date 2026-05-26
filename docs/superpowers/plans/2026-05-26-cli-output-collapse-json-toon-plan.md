# CLI output collapse to JSON | TOON — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Review cadence:** Per `feedback_subagent_review_cadence`, do **not** chain spec-review or code-quality review subagents after each task. Each implementer task returns when its commit lands; consolidated spec + code review happens after Task 11.

**Goal:** Collapse the `temper` CLI's three output formats (`Pretty`, `NoTty`, `Json`) to two (`Json`, `Toon`), make JSON a strict wire-type passthrough of the cloud API, and delete the bespoke per-command formatters from the local-vault era.

**Architecture:** New `OutputFormat::{Json, Toon}` enum + `render<T: Serialize>` helper in `crates/temper-cli/src/format.rs`. Existing `OutputFormat::resolve(Option<&str>)` continues to be the dispatch entry point — callers pass `format.as_deref()` from clap. The `toon-format = "0.5"` crate is hidden behind the format module — imported only there, never leaked. Foundation lands in Task 1; each command migrates in its own commit (Tasks 2–10); dead code drops in Task 11.

**Tech Stack:** Rust, clap (with `ValueEnum` derive), `serde` / `serde_json`, `toon-format` (v3.0 spec), `std::io::IsTerminal`. Tests via `cargo nextest`.

**Spec:** `docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md`

---

## File Structure

**Modified (rewritten):**
- `crates/temper-cli/Cargo.toml` — add `toon-format = "0.5"`
- `crates/temper-cli/src/format.rs` — new `OutputFormat::{Json, Toon}`, `FormatChoice`, `render<T>()`, `render_resource_show()`

**Modified (per-command sweeps):**
- `crates/temper-cli/src/cli.rs` — clap arg definitions for `--format` flags
- `crates/temper-cli/src/commands/events.rs` (Task 2)
- `crates/temper-cli/src/commands/auth.rs` (Task 3)
- `crates/temper-cli/src/commands/context_cmd.rs` (Task 4)
- `crates/temper-cli/src/commands/status.rs` (Task 5)
- `crates/temper-cli/src/commands/search_cmd.rs` + `crates/temper-cli/src/actions/search.rs` (Task 6)
- `crates/temper-cli/src/commands/resource.rs` (Tasks 7, 8, 9)
- `crates/temper-cli/src/commands/warmup.rs`, `doctor.rs`, `init.rs` (Task 10)

**Deleted at end (Task 11):**
- `crates/temper-cli/src/output/columns.rs` (180 lines)
- `crates/temper-cli/src/output/table.rs` (220 lines)
- The `Pretty` and `NoTty` arms of `OutputFormat`
- The `output(&T, OutputFormat)` helper (replaced by `render`)
- Re-exports of `display_columns`, `extract_row`, `Alignment`, `Column`, `TableRenderer` from `output/mod.rs`

**Kept untouched:**
- `crates/temper-cli/src/output/mod.rs` styling functions (`success`, `error`, `label`, etc. — used by status/init for human-readable Toon-mode output)
- `crates/temper-cli/src/output/styles.rs` (color constants + `clap_styles()`)

---

## Working Conventions

- **One commit per task.** The task's commit message body should reference the design doc.
- **Test-first within each task.** Modify or add the test that asserts the new behavior; run it and see it fail; make the implementation change; run it and see it pass; commit.
- **Toon assertions = contains-checks**, not exact-string. Toon output formatting may evolve; assert that key fields appear in the output, not the whole rendered string.
- **`cargo nextest run -p temper-cli`** is the per-task verification command. Full `cargo make check` runs in Task 11.
- **Branch:** `jct/post-cloud-only-cli-output-json-toon` (already created and on `c3cfc1f`).

---

## Task 1: Foundation — new `OutputFormat`, `render<T>()`, toon-format dep

**Files:**
- Modify: `crates/temper-cli/Cargo.toml`
- Modify: `crates/temper-cli/src/format.rs` (whole file rewritten)

**Goal:** Add `OutputFormat::Toon` variant alongside existing `Pretty`/`NoTty`/`Json` (so per-command tasks can migrate one at a time). Introduce `render<T: Serialize>(&T, OutputFormat) -> Result<String, TemperError>` and `render_resource_show(metadata, body, fmt)`. Old `Pretty`/`NoTty` variants and `output<T>()` keep working — they get deleted in Task 11.

- [ ] **Step 1: Add the toon-format dependency**

Edit `crates/temper-cli/Cargo.toml`. Under `[dependencies]`, add:

```toml
toon-format = "0.5"
```

Keep alphabetical ordering with existing deps.

- [ ] **Step 2: Write the failing tests in `format.rs`**

Replace the existing `#[cfg(test)] mod tests { ... }` block in `crates/temper-cli/src/format.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    // Parse tests for existing variants are kept until Task 11 to gate the
    // rip-and-replace; new variant + new helpers covered here.

    #[test]
    fn parse_toon_lowercase() {
        assert_eq!(OutputFormat::parse("toon"), OutputFormat::Toon);
    }

    #[test]
    fn resolve_explicit_toon() {
        assert_eq!(OutputFormat::resolve(Some("toon")), OutputFormat::Toon);
    }

    #[derive(Serialize)]
    struct Fixture {
        slug: &'static str,
        score: f32,
    }

    #[test]
    fn render_json_emits_serde_json_pretty() {
        let f = Fixture { slug: "hello", score: 0.5 };
        let out = render(&f, OutputFormat::Json).expect("json render");
        assert!(out.contains("\"slug\": \"hello\""), "json: {out}");
        assert!(out.contains("\"score\": 0.5"), "json: {out}");
    }

    #[test]
    fn render_toon_emits_key_and_value() {
        let f = Fixture { slug: "hello", score: 0.5 };
        let out = render(&f, OutputFormat::Toon).expect("toon render");
        // Contains-check, not exact-string — Toon formatting may evolve.
        assert!(out.contains("slug"), "toon: {out}");
        assert!(out.contains("hello"), "toon: {out}");
    }

    // Existing tests kept for backward compat until Task 11.

    #[test]
    fn parse_pretty_lowercase() {
        assert_eq!(OutputFormat::parse("pretty"), OutputFormat::Pretty);
    }

    #[test]
    fn parse_no_tty_with_dash() {
        assert_eq!(OutputFormat::parse("no-tty"), OutputFormat::NoTty);
    }

    #[test]
    fn parse_json_lowercase() {
        assert_eq!(OutputFormat::parse("json"), OutputFormat::Json);
    }

    #[test]
    fn parse_unknown_defaults_to_auto() {
        let v = OutputFormat::parse("text");
        assert!(matches!(
            v,
            OutputFormat::Pretty | OutputFormat::NoTty | OutputFormat::Toon | OutputFormat::Json
        ));
    }

    #[test]
    fn resolve_explicit_honors_value() {
        assert_eq!(OutputFormat::resolve(Some("json")), OutputFormat::Json);
    }
}
```

- [ ] **Step 3: Run the new tests; they should fail to compile**

Run: `cargo nextest run -p temper-cli format::tests 2>&1 | tail -20`

Expected: compilation errors — `OutputFormat::Toon` not defined, `render` not defined.

- [ ] **Step 4: Implement the new format.rs**

Replace the entire content of `crates/temper-cli/src/format.rs` with:

```rust
//! Output format selector for CLI commands.
//!
//! Strict policy: this module is the **only** place the `toon-format` crate
//! is imported. Callers receive `String` from `render` / `render_resource_show`
//! and never touch toon types directly. Swapping the Toon backend (to
//! `toon-rs`, a hand-rolled implementation, or a successor crate) touches
//! this file only.

use std::io::IsTerminal;

use serde::Serialize;
use temper_core::error::TemperError;

/// CLI output format. Two formats only post-Group-F: `Json` (strict
/// wire-type passthrough of cloud API responses) and `Toon` (human-readable
/// rendering of the same data via the `toon-format` crate, TOON v3.0 spec).
///
/// `Pretty` and `NoTty` are deprecated aliases kept until Task 11 to enable
/// per-command migration without a flag-day cutover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Toon,
    /// Deprecated — removed in Task 11.
    Pretty,
    /// Deprecated — removed in Task 11.
    NoTty,
}

impl OutputFormat {
    /// Parse a `--format` string. Unknown / legacy values auto-detect via TTY.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => Self::Json,
            "toon" => Self::Toon,
            "pretty" => Self::Pretty,
            "no-tty" | "notty" => Self::NoTty,
            _ => Self::auto(),
        }
    }

    /// Resolve the effective format given an optional explicit CLI value.
    pub fn resolve(explicit: Option<&str>) -> Self {
        match explicit {
            Some(s) => Self::parse(s),
            None => Self::auto(),
        }
    }

    /// Auto-pick based on whether stdout is a terminal: TTY → Toon, else Json.
    fn auto() -> Self {
        if std::io::stdout().is_terminal() {
            Self::Toon
        } else {
            Self::Json
        }
    }

    /// Canonical string form for callsites that still pass `&str` (Task 11
    /// removes the surviving callers).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toon => "toon",
            Self::Pretty => "pretty",
            Self::NoTty => "no-tty",
        }
    }
}

/// Render any `Serialize` value in the chosen format. `Json` uses
/// `serde_json::to_string_pretty`; `Toon` uses `toon_format::to_string`.
/// `Pretty` and `NoTty` fall through to JSON until Task 11 deletes them.
pub fn render<T: Serialize>(value: &T, fmt: OutputFormat) -> Result<String, TemperError> {
    match fmt {
        OutputFormat::Json | OutputFormat::Pretty | OutputFormat::NoTty => {
            Ok(serde_json::to_string_pretty(value)?)
        }
        OutputFormat::Toon => toon_format::to_string(value)
            .map_err(|e| TemperError::Api(format!("toon render: {e}"))),
    }
}

/// `temper resource show` exception: Toon emits markdown body with the
/// frontmatter at the top (as today's Pretty/NoTty does); Json emits a
/// composite shape `{ ...metadata, content: "<body>" }`.
pub fn render_resource_show(
    metadata: &serde_json::Value,
    body: &str,
    fmt: OutputFormat,
) -> Result<String, TemperError> {
    match fmt {
        OutputFormat::Toon | OutputFormat::Pretty | OutputFormat::NoTty => {
            // Frontmatter as YAML between `---` fences, then the body.
            let frontmatter = serde_yaml::to_string(metadata)?;
            Ok(format!("---\n{frontmatter}---\n{body}"))
        }
        OutputFormat::Json => {
            let mut composite = metadata.clone();
            if let Some(obj) = composite.as_object_mut() {
                obj.insert(
                    "content".to_string(),
                    serde_json::Value::String(body.to_string()),
                );
            }
            Ok(serde_json::to_string_pretty(&composite)?)
        }
    }
}

/// Legacy helper kept until Task 11; new code uses `render`.
pub fn output<T: Serialize + std::fmt::Display>(value: &T, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(value).unwrap_or_default());
        }
        OutputFormat::Toon => match toon_format::to_string(value) {
            Ok(s) => println!("{s}"),
            Err(_) => println!("{value}"),
        },
        OutputFormat::Pretty | OutputFormat::NoTty => println!("{value}"),
    }
}

/// Resolve an optional explicit format to its canonical string form
/// (auto-detecting the TTY when `None`). Convenience wrapper for dispatch.
pub fn resolve_format_str(explicit: Option<&str>) -> &'static str {
    OutputFormat::resolve(explicit).as_str()
}

#[cfg(test)]
mod tests {
    // (Test block from Step 2.)
}
```

Note: `serde_yaml` must already be a dep; if not, add it to `Cargo.toml` in this step.

- [ ] **Step 5: Verify serde_yaml is in Cargo.toml**

Run: `grep -n "^serde_yaml" crates/temper-cli/Cargo.toml`

If empty, add `serde_yaml = "0.9"` under `[dependencies]` (alphabetical ordering).

- [ ] **Step 6: Run all `format` tests; they should pass**

Run: `cargo nextest run -p temper-cli -E 'test(format::tests::)'`

Expected: all 9 tests pass (the 4 new + 5 existing).

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/Cargo.toml crates/temper-cli/src/format.rs
git commit -m "$(cat <<'GIT'
feat(cli): foundation for json|toon output collapse

Add OutputFormat::Toon variant + render<T: Serialize>() helper +
FormatChoice + render_resource_show() in crates/temper-cli/src/format.rs.
toon-format = "0.5" added to Cargo.toml; imported only here so the Toon
backend can be swapped behind a one-file boundary.

OutputFormat::Pretty and ::NoTty remain as deprecated aliases routing
through JSON; per-command migrations in subsequent commits move
callers off them, and Task 11 deletes them outright.

Auto-detection now picks Toon (TTY) / Json (non-TTY) — but until each
command moves to render(), the surviving Pretty/NoTty branches still
run the old code paths.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 2: Migrate `temper events`

**Files:**
- Modify: `crates/temper-cli/src/commands/events.rs`

**Goal:** Drop `format_event` and friends; route both Json and Toon through `render(events, fmt)` on `Vec<Event>`. Smallest API-shaped surface; validates the foundation.

- [ ] **Step 1: Locate the current text formatter**

Read `crates/temper-cli/src/commands/events.rs:42-83` to confirm the shape of `format_event`/`format_events` and which calls invoke it.

- [ ] **Step 2: Add or update a test asserting Json passthrough**

Append to `crates/temper-cli/src/commands/events.rs` `#[cfg(test)] mod tests` block:

```rust
#[test]
fn render_events_json_passes_wire_type() {
    use temper_core::discovery::Event;
    // Construct a small Vec<Event> fixture matching whichever variant is
    // simplest in the current Event enum. Read the enum definition before
    // writing this; adjust the constructor to current shape.
    let events: Vec<Event> = vec![/* fixture */];
    let out = crate::format::render(&events, crate::format::OutputFormat::Json)
        .expect("json render");
    assert!(out.starts_with('['), "json should be an array: {out}");
}

#[test]
fn render_events_toon_includes_event_marker() {
    use temper_core::discovery::Event;
    let events: Vec<Event> = vec![/* same fixture */];
    let out = crate::format::render(&events, crate::format::OutputFormat::Toon)
        .expect("toon render");
    // Contains-check on a structurally-stable field name; pick one that
    // every Event variant carries (e.g. "ts" or "event_type").
    assert!(out.contains("ts") || out.contains("event_type"), "toon: {out}");
}
```

Replace `/* fixture */` with a real `Event` value after reading the enum in `temper_core::discovery::Event`. If unsure, read `crates/temper-core/src/discovery.rs` first.

- [ ] **Step 3: Run the new tests; they should fail or compile-fail**

Run: `cargo nextest run -p temper-cli -E 'test(commands::events::tests::render_)'`

Expected: tests don't exist yet OR fail at compile (depending on whether the fixture matches the Event enum). Fix the fixture against the actual Event variants.

- [ ] **Step 4: Replace the formatter call sites**

In `crates/temper-cli/src/commands/events.rs`, locate the function that prints events (where `format_event` is called today). Replace with:

```rust
let rendered = crate::format::render(&events, crate::format::OutputFormat::resolve(format_arg.as_deref()))?;
println!("{rendered}");
```

`format_arg` is the existing `Option<String>` from the clap definition. Drop `format_event` and any private helpers it called (they have no other callers; verify with `grep`).

- [ ] **Step 5: Run the events tests + commands::events tests**

Run: `cargo nextest run -p temper-cli -E 'test(commands::events::)'`

Expected: all events tests pass; the deleted `format_event` tests if any are gone.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/events.rs
git commit -m "$(cat <<'GIT'
refactor(cli): events command emits json|toon via render()

Drop format_event and friends; route through format::render(events, fmt).
JSON: passes Vec<Event> directly. Toon: same data, human-readable.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 3: Migrate `temper auth status / login / logout`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (clap arg additions)
- Modify: `crates/temper-cli/src/commands/auth.rs`

**Goal:** Add `--format` flag to all three auth subcommands. Toon mode renders the `AuthStatus` struct (status) or a confirmation struct (login/logout). Json keeps current shape.

- [ ] **Step 1: Add the --format clap arg to each auth subcommand**

Read `crates/temper-cli/src/cli.rs` to find the `Auth` subcommand enum (likely `enum AuthAction { Status, Login, Logout, ... }`). Add a `format: Option<String>` field to each variant struct, following the pattern used by `Search` / `Resource::List`. Example:

```rust
Status {
    /// Output format: json | toon (default: toon on TTY, json otherwise)
    #[arg(long)]
    format: Option<String>,
},
```

- [ ] **Step 2: Write the failing test**

Append to `crates/temper-cli/src/commands/auth.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn render_auth_status_json_includes_authenticated() {
    let status = temper_client::auth::AuthStatus {
        authenticated: true,
        provider: Some("auth0".to_string()),
        expires_at: None,
        profile_id: None,
    };
    let out = crate::format::render(&status, crate::format::OutputFormat::Json)
        .expect("json render");
    assert!(out.contains("\"authenticated\": true"), "json: {out}");
}

#[test]
fn render_auth_status_toon_contains_provider() {
    let status = temper_client::auth::AuthStatus {
        authenticated: true,
        provider: Some("auth0".to_string()),
        expires_at: None,
        profile_id: None,
    };
    let out = crate::format::render(&status, crate::format::OutputFormat::Toon)
        .expect("toon render");
    assert!(out.contains("auth0"), "toon: {out}");
}
```

If `AuthStatus` field names differ from this fixture, read `crates/temper-client/src/auth.rs` and adjust.

- [ ] **Step 3: Run the new tests; expect fail until the auth handler routes through render()**

Run: `cargo nextest run -p temper-cli -E 'test(commands::auth::tests::render_auth_status)'`

Expected: tests pass (since they only call `render` directly; auth handler changes come in Step 4). If they fail, fix the fixture.

- [ ] **Step 4: Replace println! / serde_json calls in the auth handlers**

In `crates/temper-cli/src/commands/auth.rs`, find the `status()`, `login()`, `logout()` functions. Replace direct `serde_json::to_string_pretty(...)` calls with:

```rust
let fmt = crate::format::OutputFormat::resolve(format.as_deref());
let rendered = crate::format::render(&status, fmt)?;
println!("{rendered}");
```

For `login` and `logout`, define a small action-confirmation struct rather than a raw JSON literal:

```rust
#[derive(serde::Serialize)]
struct AuthAction<'a> {
    status: &'a str,
    profile: Option<String>,
}

let result = AuthAction { status: "logged_in", profile: Some(profile_id.to_string()) };
let rendered = crate::format::render(&result, fmt)?;
println!("{rendered}");
```

When `fmt == OutputFormat::Toon` and the action is a no-data side-effect (e.g. `logout`), the rendered Toon will be a one-line `status: logged_out` — acceptable terse output for an action command.

- [ ] **Step 5: Run all auth command tests**

Run: `cargo nextest run -p temper-cli -E 'test(commands::auth::)'`

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/auth.rs
git commit -m "$(cat <<'GIT'
feat(cli): --format flag on auth status / login / logout

Toon mode renders AuthStatus (status) or an AuthAction confirmation
struct (login/logout). Json keeps current shape; default is auto
(Toon in TTY, Json in non-TTY).

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 4: Migrate `temper context list`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (clap arg)
- Modify: `crates/temper-cli/src/commands/context_cmd.rs`

**Goal:** Add `--format` flag. Json emits `Vec<String>`; Toon renders the same data.

- [ ] **Step 1: Add `--format` to the `Context::List` clap variant**

Pattern as in Task 3 Step 1.

- [ ] **Step 2: Write the failing test**

Append to `crates/temper-cli/src/commands/context_cmd.rs` test module:

```rust
#[test]
fn render_context_list_json_is_array_of_strings() {
    let contexts = vec!["temper".to_string(), "knowledge".to_string()];
    let out = crate::format::render(&contexts, crate::format::OutputFormat::Json)
        .expect("json render");
    assert!(out.contains("\"temper\""), "json: {out}");
    assert!(out.contains("\"knowledge\""), "json: {out}");
    assert!(out.starts_with('['), "json should be an array: {out}");
}

#[test]
fn render_context_list_toon_contains_context_names() {
    let contexts = vec!["temper".to_string(), "knowledge".to_string()];
    let out = crate::format::render(&contexts, crate::format::OutputFormat::Toon)
        .expect("toon render");
    assert!(out.contains("temper"), "toon: {out}");
    assert!(out.contains("knowledge"), "toon: {out}");
}
```

- [ ] **Step 3: Run; tests should pass already**

Run: `cargo nextest run -p temper-cli -E 'test(commands::context_cmd::tests::render_context_list)'`

Expected: pass (render is generic over `Serialize`).

- [ ] **Step 4: Replace the text-formatting print in the list handler**

In `crates/temper-cli/src/commands/context_cmd.rs`, locate the function that today prints the aligned columns of context names. Replace with:

```rust
let fmt = crate::format::OutputFormat::resolve(format.as_deref());
let rendered = crate::format::render(&contexts, fmt)?;
println!("{rendered}");
```

Drop the old aligned-columns helper (verify no other callers via grep).

- [ ] **Step 5: Run context_cmd tests**

Run: `cargo nextest run -p temper-cli -E 'test(commands::context_cmd::)'`

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/context_cmd.rs
git commit -m "$(cat <<'GIT'
feat(cli): --format flag on context list

Drop aligned-column text rendering; emit Vec<String> via format::render().
JSON is the bare array; Toon renders the same data in human-readable form.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 5: Migrate `temper status`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (clap arg)
- Modify: `crates/temper-cli/src/commands/status.rs`

**Goal:** Add `--format` flag. Define a `StatusReport` struct as the new JSON shape: `{ contexts: [{ name, staleness, projected, server }] }`. Toon renders the same.

- [ ] **Step 1: Add `--format` to the `Status` clap definition**

Pattern as in Task 3 Step 1.

- [ ] **Step 2: Define the StatusReport struct**

Add to `crates/temper-cli/src/commands/status.rs` near the top of the file:

```rust
#[derive(Debug, serde::Serialize)]
pub(crate) struct StatusReport {
    pub contexts: Vec<ContextStatus>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ContextStatus {
    pub name: String,
    pub staleness: String,
    pub projected: u64,
    pub server: Option<u64>,
}
```

- [ ] **Step 3: Write the failing test**

Append to `commands/status.rs` test module:

```rust
#[test]
fn render_status_report_json_shape() {
    let report = StatusReport {
        contexts: vec![ContextStatus {
            name: "temper".to_string(),
            staleness: "fresh".to_string(),
            projected: 42,
            server: Some(42),
        }],
    };
    let out = crate::format::render(&report, crate::format::OutputFormat::Json)
        .expect("json render");
    assert!(out.contains("\"contexts\""), "json: {out}");
    assert!(out.contains("\"staleness\": \"fresh\""), "json: {out}");
    assert!(out.contains("\"projected\": 42"), "json: {out}");
}

#[test]
fn render_status_report_toon_includes_context_name() {
    let report = StatusReport {
        contexts: vec![ContextStatus {
            name: "temper".to_string(),
            staleness: "fresh".to_string(),
            projected: 42,
            server: Some(42),
        }],
    };
    let out = crate::format::render(&report, crate::format::OutputFormat::Toon)
        .expect("toon render");
    assert!(out.contains("temper"), "toon: {out}");
    assert!(out.contains("fresh"), "toon: {out}");
}
```

- [ ] **Step 4: Run new tests; expect pass**

Run: `cargo nextest run -p temper-cli -E 'test(commands::status::tests::render_status_report)'`

Expected: pass.

- [ ] **Step 5: Refactor `status` handler to build `StatusReport` then render**

In `crates/temper-cli/src/commands/status.rs`, find the function that today prints the staleness report line-by-line. Replace its print loop with:

1. Construct a `StatusReport` by walking the same context list and mapping each row to a `ContextStatus`.
2. Map the existing staleness outcome enum into the staleness string (e.g. `Fresh → "fresh"`, `Stale → "stale"`, `NotProjected → "not-projected"`, `Skipped → "skipped"`). Use a small private match.
3. Call `render(&report, fmt)` and print.

Keep the human-readable line rendering in Toon mode only if `toon-format` output isn't sufficient for the user's expectation — if the contains-checks in Step 3 pass, the default Toon rendering is acceptable.

- [ ] **Step 6: Run all status tests**

Run: `cargo nextest run -p temper-cli -E 'test(commands::status::)'`

Expected: all pass. (The pre-existing `count_projected_md_files_*` tests in `status.rs:190-241` are unrelated to formatting and should be untouched.)

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/status.rs
git commit -m "$(cat <<'GIT'
feat(cli): --format flag on status; define StatusReport JSON shape

Status now constructs a StatusReport { contexts: [{ name, staleness,
projected, server }] } and routes through format::render(). The old
line-by-line human-readable print is replaced by Toon rendering of the
same struct.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 6: Migrate `temper search`

**Files:**
- Modify: `crates/temper-cli/src/actions/search.rs`
- Modify: `crates/temper-cli/src/commands/search_cmd.rs`

**Goal:** Drop `actions/search.rs::format_text` and its test. JSON + Toon pass `Vec<UnifiedSearchResultRow>` directly via `render`.

- [ ] **Step 1: Locate `format_text` and its caller**

Read `crates/temper-cli/src/actions/search.rs:91-105` and find the caller in `commands/search_cmd.rs` that invokes it.

- [ ] **Step 2: Update the search handler in `commands/search_cmd.rs`**

Replace the current branching call (probably `match format { OutputFormat::Json => ..., OutputFormat::Pretty | NoTty => format_text(...) }`) with:

```rust
let fmt = crate::format::OutputFormat::resolve(format.as_deref());
let rendered = crate::format::render(&results, fmt)?;
println!("{rendered}");
```

`results` is `Vec<UnifiedSearchResultRow>` returned by `client.search().search_with_params(...)`.

- [ ] **Step 3: Delete `format_text` from `actions/search.rs`**

Remove the `pub fn format_text(...) -> String` definition (lines ~91-105).
Remove the `format_text_includes_score_and_origin` test in the same file's `#[cfg(test)]` block.

Keep `build_search_params`, `truncate`, and their tests — they're unrelated to format and used elsewhere.

- [ ] **Step 4: Add a regression test asserting Json passthrough**

Append to `crates/temper-cli/src/actions/search.rs` test module:

```rust
#[test]
fn render_search_results_json_is_passthrough_array() {
    use temper_core::types::api::UnifiedSearchResultRow;
    let rows: Vec<UnifiedSearchResultRow> = vec![/* one fixture row */];
    let out = crate::format::render(&rows, crate::format::OutputFormat::Json)
        .expect("json render");
    assert!(out.starts_with('['), "json should be an array: {out}");
    // If the fixture has a title field, assert it appears verbatim.
}
```

Construct a real `UnifiedSearchResultRow` after reading the struct definition at `crates/temper-core/src/types/api.rs:151-164`.

- [ ] **Step 5: Run search tests**

Run: `cargo nextest run -p temper-cli -E 'test(actions::search::) or test(commands::search_cmd::)'`

Expected: all pass; `format_text_includes_score_and_origin` is gone.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/search.rs crates/temper-cli/src/commands/search_cmd.rs
git commit -m "$(cat <<'GIT'
refactor(cli): search emits wire type via format::render()

Drop actions/search.rs::format_text (numbered text list). JSON and Toon
now both pass Vec<UnifiedSearchResultRow> directly. Score/origin fields
are visible in JSON wire output and in Toon's structural rendering.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 7: Migrate `temper resource list`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`

**Goal:** Drop `render_server_rows`, `row_to_frontmatter_value`, and the call into `extract_row`. JSON + Toon pass `Vec<ResourceRow>` directly. Internal fields (`body_hash`, `managed_hash`, `kb_context_id`, etc.) become visible in JSON — accepted per spec.

- [ ] **Step 1: Locate the list handler and its helpers**

Read `crates/temper-cli/src/commands/resource.rs` to find:
- The handler for `Resource::List` (around line 370-393 per earlier inventory)
- `render_server_rows` (around line 449-478)
- `row_to_frontmatter_value` (somewhere in the same file)

- [ ] **Step 2: Add a test asserting wire-type passthrough**

Append to `commands/resource.rs` test module:

```rust
#[test]
fn render_resource_list_json_includes_wire_fields() {
    use temper_core::types::resource::ResourceRow;
    let rows: Vec<ResourceRow> = vec![/* one fixture row */];
    let out = crate::format::render(&rows, crate::format::OutputFormat::Json)
        .expect("json render");
    // body_hash is an internal field that the old re-shaping dropped;
    // strict passthrough means it must now be present.
    assert!(out.contains("body_hash") || out.contains("\"body_hash\""), "json: {out}");
}
```

Read `crates/temper-core/src/types/resource.rs:18-54` first to construct a valid `ResourceRow` fixture.

- [ ] **Step 3: Run the new test; expect pass against the foundation**

Run: `cargo nextest run -p temper-cli -E 'test(commands::resource::tests::render_resource_list_json)'`

Expected: pass.

- [ ] **Step 4: Replace the list handler print path**

In `commands/resource.rs`, in the `Resource::List` handler:

```rust
let fmt = crate::format::OutputFormat::resolve(format.as_deref());
let rendered = crate::format::render(&rows, fmt)?;
println!("{rendered}");
```

Delete the `render_server_rows` function and any private helpers it called (e.g. `row_to_frontmatter_value`) — verify no other callers via grep before deleting.

- [ ] **Step 5: Run resource list tests**

Run: `cargo nextest run -p temper-cli -E 'test(commands::resource::) and test(list)'`

Expected: tests pass; pre-existing table/columns-style assertions if any will need to be deleted as collateral (verify before removing).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'GIT'
refactor(cli): resource list emits wire type ResourceRow via render()

Drop render_server_rows + row_to_frontmatter_value. JSON output now
includes all wire fields including internal ones (body_hash,
managed_hash, kb_context_id) — strict wire-type passthrough per spec.
Toon renders the same data structurally.

Breaking change: scripts piping `temper resource list --format json`
into jq with the old `temper-*` frontmatter keys must switch to wire
field names. Accepted per feedback_no_premature_backward_compat.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 8: Migrate `temper resource show`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`
- Modify: `crates/temper-cli/src/commands/task.rs` (if it has a show-shaped handler)

**Goal:** Use `render_resource_show()` from the format module. Toon emits `---\n<frontmatter yaml>\n---\n<markdown body>`; Json emits `{ ...metadata, content: "<body>" }`.

- [ ] **Step 1: Locate the current show handlers**

Read `crates/temper-cli/src/commands/resource.rs:615-660` and `crates/temper-cli/src/commands/task.rs:31-40`.

- [ ] **Step 2: Add a test for the show path**

Append to `commands/resource.rs` test module:

```rust
#[test]
fn render_resource_show_toon_emits_frontmatter_then_body() {
    let metadata = serde_json::json!({
        "temper-title": "Hello",
        "temper-slug": "hello",
    });
    let body = "# Hello\n\nBody text.\n";
    let out = crate::format::render_resource_show(
        &metadata,
        body,
        crate::format::OutputFormat::Toon,
    )
    .expect("toon render");
    assert!(out.starts_with("---\n"), "toon: {out}");
    assert!(out.contains("# Hello"), "toon: {out}");
}

#[test]
fn render_resource_show_json_includes_content_key() {
    let metadata = serde_json::json!({
        "temper-title": "Hello",
        "temper-slug": "hello",
    });
    let body = "# Hello\n\nBody text.\n";
    let out = crate::format::render_resource_show(
        &metadata,
        body,
        crate::format::OutputFormat::Json,
    )
    .expect("json render");
    assert!(out.contains("\"content\""), "json: {out}");
    assert!(out.contains("# Hello"), "json: {out}");
}
```

- [ ] **Step 3: Run new tests; expect pass**

Run: `cargo nextest run -p temper-cli -E 'test(commands::resource::tests::render_resource_show)'`

Expected: pass.

- [ ] **Step 4: Replace the show handler**

In the `Resource::Show` handler (and any per-doctype variants like `task.rs::show`):

```rust
let fmt = crate::format::OutputFormat::resolve(format.as_deref());

// Build metadata as JSON Value from ResourceRow's serde derive.
let metadata = serde_json::to_value(&row)
    .map_err(|e| temper_core::error::TemperError::Api(format!("metadata serialize: {e}")))?;

let rendered = crate::format::render_resource_show(&metadata, &body, fmt)?;
println!("{rendered}");
```

`row` is `ResourceRow`, `body` is the fetched markdown content (already a `String` from the existing `client.resources().content(...)` call).

Drop the legacy per-format branching that hand-built JSON wrappers like `{ doc_type, slug, title, context, path, content }`.

- [ ] **Step 5: Run resource show tests**

Run: `cargo nextest run -p temper-cli -E 'test(commands::resource::) and test(show)'`

Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/commands/task.rs
git commit -m "$(cat <<'GIT'
refactor(cli): resource show uses render_resource_show()

Toon mode: emit YAML frontmatter between --- fences followed by raw
markdown body (matches today's Pretty/NoTty rendering for the
show-is-content idiom). Json mode: composite { ...ResourceRow, content }.

Drop the bespoke JSON wrapper { doc_type, slug, title, context, path,
content }; the new shape is the wire ResourceRow with a content field
joined in.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 9: Migrate `temper resource create / update / delete`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`

**Goal:** Drop the 7-variant per-doctype JSON shape map at `commands/resource.rs:72-159`. Default action-command output is a confirmation line; `--format json` emits `{ status: "ok", ...wire-type-fields }`.

- [ ] **Step 1: Locate the per-doctype shape map**

Read `crates/temper-cli/src/commands/resource.rs:72-159`. Confirm the 7 variants (Task, Goal, Session, Research, Concept, Decision, default).

- [ ] **Step 2: Define a CreateActionResult struct near the top of the file**

```rust
#[derive(Debug, serde::Serialize)]
pub(crate) struct CreateActionResult {
    pub status: &'static str,
    #[serde(flatten)]
    pub resource: temper_core::types::resource::ResourceRow,
}
```

For update/delete:

```rust
#[derive(Debug, serde::Serialize)]
pub(crate) struct UpdateActionResult {
    pub status: &'static str,
    pub slug: String,
    pub id: temper_core::types::ids::ResourceId,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct DeleteActionResult {
    pub status: &'static str,
    pub slug: String,
    pub id: temper_core::types::ids::ResourceId,
}
```

- [ ] **Step 3: Write the failing test**

Append to `commands/resource.rs` test module:

```rust
#[test]
fn render_create_action_result_json_is_flat() {
    use temper_core::types::resource::ResourceRow;
    let row: ResourceRow = /* fixture */;
    let result = CreateActionResult { status: "ok", resource: row };
    let out = crate::format::render(&result, crate::format::OutputFormat::Json)
        .expect("json render");
    assert!(out.contains("\"status\": \"ok\""), "json: {out}");
    assert!(out.contains("\"slug\""), "json: {out}");  // flattened ResourceRow field
}
```

- [ ] **Step 4: Run; expect pass**

Run: `cargo nextest run -p temper-cli -E 'test(commands::resource::tests::render_create_action)'`

Expected: pass.

- [ ] **Step 5: Replace the per-doctype shaping in the create handler**

In the Resource::Create handler:

```rust
let row = client.resources().create(payload).await?;
let fmt = crate::format::OutputFormat::resolve(format.as_deref());
match fmt {
    crate::format::OutputFormat::Json | crate::format::OutputFormat::Toon => {
        let result = CreateActionResult { status: "ok", resource: row };
        let rendered = crate::format::render(&result, fmt)?;
        println!("{rendered}");
    }
    // Pretty/NoTty fall through here for now; Task 11 deletes them.
    _ => {
        let result = CreateActionResult { status: "ok", resource: row };
        let rendered = crate::format::render(&result, crate::format::OutputFormat::Toon)?;
        println!("{rendered}");
    }
}
```

Delete the entire 7-variant per-doctype shape map. Verify with grep that the deleted functions have no other callers.

Repeat the same pattern for update and delete handlers using `UpdateActionResult` and `DeleteActionResult`.

- [ ] **Step 6: Run resource tests**

Run: `cargo nextest run -p temper-cli -E 'test(commands::resource::)'`

Expected: all pass; any deleted-helper tests are gone.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'GIT'
refactor(cli): resource create/update/delete return wire action result

Drop the 7-variant per-doctype JSON shape map. Each mutating command
now returns a flat { status: "ok", ...wire-type-fields } struct
(CreateActionResult flattens ResourceRow; Update/Delete return slug+id).

Breaking change: scripts depending on { temper-slug, temper-title } or
similar per-doctype shapes from create output must switch to the
wire-type field names. Accepted per feedback_no_premature_backward_compat.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 10: Migrate `temper warmup`, `doctor`, `init`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (clap arg for doctor)
- Modify: `crates/temper-cli/src/commands/warmup.rs`
- Modify: `crates/temper-cli/src/commands/doctor.rs` (or wherever check lives)
- Modify: `crates/temper-cli/src/commands/init.rs`

**Goal:** Migrate `warmup` to new render path. Add `--format` to `doctor`/`check`. `init` stays interactive in TTY; non-interactive + `--format json` emits the final summary.

- [ ] **Step 1: warmup — replace the existing format dispatch**

Read `crates/temper-cli/src/commands/warmup.rs`. It already accepts `--format`. Replace its internal output call with:

```rust
let fmt = crate::format::OutputFormat::resolve(format.as_deref());
let rendered = crate::format::render(&warmup_result, fmt)?;
println!("{rendered}");
```

Drop any old `output(&warmup_result, format)` call that goes through the legacy helper.

- [ ] **Step 2: doctor — add --format flag and a CheckReport struct**

In `cli.rs`, add `format: Option<String>` to the `Doctor` (or `Check`) clap variant.

In the doctor handler, define:

```rust
#[derive(Debug, serde::Serialize)]
pub(crate) struct CheckReport {
    pub checks: Vec<CheckItem>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct CheckItem {
    pub name: String,
    pub status: String,    // "ok" | "warning" | "error"
    pub message: String,
}
```

Build a `CheckReport` from the current check results and render through `format::render()`.

- [ ] **Step 3: init — interactive in TTY, JSON summary in non-interactive**

In `crates/temper-cli/src/commands/init.rs`, locate the wizard end-state where the summary is printed today. Define:

```rust
#[derive(Debug, serde::Serialize)]
pub(crate) struct InitSummary {
    pub vault_path: String,
    pub contexts: Vec<String>,
    pub auth: String,  // "auth0" | "none"
}
```

After the wizard completes, if `format.as_deref() == Some("json")` (or `--no-interactive` with default-Auto format resolving to Json non-TTY), build an `InitSummary` and render it. Otherwise keep the existing styled `label()` calls.

- [ ] **Step 4: Write a test for CheckReport JSON shape**

Append to `commands/doctor.rs` (or wherever check lives):

```rust
#[test]
fn render_check_report_json_includes_checks_array() {
    let report = CheckReport {
        checks: vec![CheckItem {
            name: "db".to_string(),
            status: "ok".to_string(),
            message: "connected".to_string(),
        }],
    };
    let out = crate::format::render(&report, crate::format::OutputFormat::Json)
        .expect("json render");
    assert!(out.contains("\"checks\""), "json: {out}");
    assert!(out.contains("\"status\": \"ok\""), "json: {out}");
}
```

- [ ] **Step 5: Run tests**

Run: `cargo nextest run -p temper-cli -E 'test(commands::warmup::) or test(commands::doctor::) or test(commands::init::)'`

Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/warmup.rs crates/temper-cli/src/commands/doctor.rs crates/temper-cli/src/commands/init.rs
git commit -m "$(cat <<'GIT'
feat(cli): --format on doctor + init; warmup moves to render()

Doctor/check now emits { checks: [{ name, status, message }] } via
format::render(). Init stays interactive in TTY; non-interactive mode
with --format json emits { vault_path, contexts, auth }. Warmup
migrates to format::render() from the legacy output() helper.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Task 11: Cleanup — delete `Pretty` / `NoTty`, columns.rs, table.rs

**Files:**
- Modify: `crates/temper-cli/src/format.rs` (delete Pretty/NoTty arms + legacy `output()`)
- Modify: `crates/temper-cli/src/output/mod.rs` (drop re-exports)
- Delete: `crates/temper-cli/src/output/columns.rs`
- Delete: `crates/temper-cli/src/output/table.rs`
- Modify: `CLAUDE.md` (Build & Test section; help-text mentions)

**Goal:** Final dead-code drop. Project compiles clean against only `Json` / `Toon`.

- [ ] **Step 1: Audit for remaining `Pretty` / `NoTty` references**

Run: `grep -rn "OutputFormat::Pretty\|OutputFormat::NoTty\|::Pretty\|::NoTty" crates/temper-cli/`

For each hit outside `format.rs` itself, verify the callsite was migrated in Tasks 2–10. If any remain, this task isn't ready — fix the missed callsite in a fix-up commit on the previous task's logical scope or extend Task 10.

- [ ] **Step 2: Rewrite `format.rs` to drop Pretty/NoTty**

Open `crates/temper-cli/src/format.rs` and:

- Delete the `Pretty` and `NoTty` variants from `OutputFormat`.
- Delete `parse_pretty_lowercase` and `parse_no_tty_with_dash` tests.
- Update `OutputFormat::parse` to remove the `"pretty"` and `"no-tty"` arms — unknown values now still fall through to `auto()` which picks Json or Toon.
- Delete the legacy `output<T>(value, format)` helper entirely.
- Update `as_str` to only return `"json"` or `"toon"`.

Final `OutputFormat` enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Toon,
}
```

- [ ] **Step 3: Delete columns.rs and table.rs**

```bash
git rm crates/temper-cli/src/output/columns.rs
git rm crates/temper-cli/src/output/table.rs
```

- [ ] **Step 4: Update `output/mod.rs`**

Remove these lines:

```rust
pub mod columns;
pub mod table;
pub use columns::{display_columns, extract_row};
pub use table::{Alignment, Column, TableRenderer};
```

Keep all the styling helpers (`success`, `error`, `label`, `header`, `dim`, etc.) and `pub use styles::clap_styles`.

- [ ] **Step 5: Update CLAUDE.md if it references pretty/no-tty**

Run: `grep -n "pretty\|no-tty\|NoTty\|Pretty" CLAUDE.md`

For any matches in the project CLAUDE.md, update the documentation to reflect json/toon. Particularly in the "Build & Test Commands" or any user-facing format references.

- [ ] **Step 6: Run the full check + test suite**

Run: `cargo make check 2>&1 | tail -30`

Expected: clippy clean, format clean, docs clean, machete clean, biome clean.

Run: `cargo nextest run -p temper-cli 2>&1 | tail -20`

Expected: all temper-cli tests pass.

Run: `cargo nextest run --workspace --features test-db 2>&1 | tail -20`

Expected: full workspace passes (any cross-crate dependent on dropped types would error).

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/format.rs crates/temper-cli/src/output/mod.rs CLAUDE.md
git rm crates/temper-cli/src/output/columns.rs crates/temper-cli/src/output/table.rs
git commit -m "$(cat <<'GIT'
refactor(cli): drop Pretty/NoTty + columns.rs + table.rs (Group F cleanup)

Final dead-code drop for the json|toon collapse. OutputFormat is now
just { Json, Toon }; the legacy output() helper and the
columns/table renderers (400 lines of display code) are gone. CLAUDE.md
updated to reflect json/toon-only.

Spec: docs/superpowers/specs/2026-05-26-cli-output-collapse-json-toon-design.md

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
GIT
)"
```

---

## Post-plan: review + PR

After Task 11 lands locally:

1. Push: `git push -u origin jct/post-cloud-only-cli-output-json-toon`
2. Dispatch one consolidated code-review subagent against the full branch diff (per `feedback_subagent_review_cadence`). Use `superpowers:requesting-code-review`.
3. Address review feedback inline (any new commits added to the branch).
4. Open the PR via `gh pr create`. Title: `feat(cli): collapse output to json|toon (Group F)`.
5. Update memory `project_cli_output_format_simplification` status to "landed" after merge.

---

## Spec coverage check

- [x] **Strict wire-type passthrough** — Tasks 6, 7, 9 (search, list, create).
- [x] **Auto-default Toon (TTY) / Json (non-TTY)** — Task 1 (`OutputFormat::auto()`).
- [x] **toon-format crate v0.5 (v3.0 spec)** — Task 1 step 1.
- [x] **Encapsulation invariant** — Task 1 (`format.rs` is the only importer).
- [x] **resource show exception** — Task 8 (`render_resource_show`).
- [x] **All data-emitting commands grow `--format`** — Tasks 3, 4, 5, 10 (auth, context list, status, doctor).
- [x] **Action commands stay terse with --format json shape** — Tasks 3 (auth login/logout), 9 (create/update/delete).
- [x] **Rip-and-replace, no deprecation shim** — Task 11 (deletes Pretty/NoTty entirely).
- [x] **11 discrete commits, bisectable** — Tasks 1-11.
