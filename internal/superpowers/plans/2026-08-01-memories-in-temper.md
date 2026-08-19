# Memories in Temper — Phase 1: the command family and the local gate

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `MEMORY.md` a rendered projection of `memory`-typed Temper resources, with an opt-in config, a read-only status report, and a local drift check — so that Phase 2 can migrate data into a system that already renders and verifies it.

**Architecture:** A new `temper memory` command family in `temper-cli`, backed by a new optional `[memory]` section on `TemperConfig`. The rendering core is a **pure function** over resource rows, unit-tested without I/O or a database; the commands are thin I/O shells around it. Reads use the existing `client.list_meta()` (metadata + `open_meta`, no bodies).

**Tech Stack:** Rust (temper-cli, temper-core), clap, serde/toml, `validator`, cargo-nextest.

## Global Constraints

Copied verbatim from `internal/superpowers/specs/2026-08-01-memories-in-temper-design.md`. Every task's requirements implicitly include these.

- **"No `[memory]` section means the feature is off and `emit` is a no-op that says why."**
- **"`emit` fails loudly on any `memory` resource missing either key, or carrying a value outside its vocabulary."** (`open_meta.status` ∈ {`active`, `superseded`}; `open_meta.verified` an ISO date.) This is the enforcement point that substitutes for write-time validation.
- **`open_meta.source_file` is OPTIONAL and must never be required.** It records the memory file a resource was migrated from, and is how `status` matches local files to Temper resources. A memory authored natively in Temper has none; its absence is ordinary, never a defect. `emit` must not reject on it.
- **"An old date means unexamined, never false, and the render must not blur the two."**
- **"Reach is a property of which configs list a context, never of a field on the memory itself."**
- **"The MCP skill tree is config-free"** — it may describe the convention and the doc type, and must **never** name a user's contexts. Adding a config-derived value removes that tree's ability to be gated.
- Doc type is `memory`, disambiguated from D3 cognitive-map node labels **by home**: context-homed = Claude Code memory, cogmap-homed = map node. `emit` reads only configured contexts, so it can never pick up a map node.

## Two corrections to the spec, found while grounding

Both are settled; the plan below implements the corrected form.

1. **The drift gate cannot be a CI job.** The spec says "the gate re-emits and diffs against the committed file." There is no committed file — the index lives at `~/.claude/projects/<project>/memory/MEMORY.md`, outside the repo, and is per-machine. The skills-drift gate (`.github/scripts/check-skills-drift.sh`) works only because `agent-skills/` is tracked by git. So the gate here is **`temper memory check`**: a local command that re-renders, diffs against the on-disk file, and exits non-zero on drift. Runnable by hand or from a session-start hook. Task 5.
2. **`memory` is a claimed name, not an unclaimed one.** `DocType::Memory` already exists as a spec-D3 cognitive-map node label (`crates/temper-workflow/src/frontmatter/document.rs:29`), and `validate_doctype` is an open tail that accepts it today (`crates/temper-workflow/src/operations/actions.rs:211-219`). Resolved by home, per Global Constraints.

## File Structure

| File | Responsibility |
|---|---|
| `crates/temper-core/src/types/config.rs` **(modify)** | Add `MemoryConfig` + an optional `memory` field on `TemperConfig`. Config shape only — no behaviour. |
| `crates/temper-cli/src/commands/memory/render.rs` **(create)** | **Pure**: resource rows → index markdown, and the validation that rejects a malformed memory. No I/O, no network. This is where the logic and nearly all the tests live. |
| `crates/temper-cli/src/commands/memory/mod.rs` **(create)** | The I/O shell: fetch rows, call `render`, write/compare files. `emit`, `check`, `status`. |
| `crates/temper-cli/src/commands/mod.rs` **(modify)** | Register the `memory` module. |
| `crates/temper-cli/src/cli.rs` **(modify)** | `Commands::Memory { action: MemoryAction }` + the `MemoryAction` enum. |
| `crates/temper-cli/src/main.rs` **(modify)** | Dispatch arms. |
| `crates/temper-cli/templates/` **(modify)** | Skill-side discovery copy, CLI and MCP variants. |

Splitting `render.rs` from `mod.rs` is the load-bearing decision: it puts every rule that can be got wrong (staleness marking, vocabulary validation, grouping, the unexamined-vs-false distinction) behind a pure function that needs no network, no config, and no database to test.

---

### Task 1: The `[memory]` config section

**Files:**
- Modify: `crates/temper-core/src/types/config.rs` (add after `SkillConfig`, ~line 97)
- Test: same file, in the existing `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nothing.
- Produces: `MemoryConfig { shared_contexts: Vec<String>, project_contexts: Vec<String>, index_path: String, stale_after_days: u32 }`, and `TemperConfig::memory: Option<MemoryConfig>`. `None` means the feature is off — the distinction later tasks branch on.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn memory_section_is_absent_by_default() {
    let cfg: TemperConfig = toml::from_str(r#"
        [vault]
        path = "~/x"
    "#).expect("parses");
    assert!(cfg.memory.is_none(), "absent [memory] must mean the feature is OFF, not defaulted on");
}

#[test]
fn memory_section_parses_context_lists() {
    let cfg: TemperConfig = toml::from_str(r#"
        [vault]
        path = "~/x"
        [memory]
        shared_contexts = ["@me/working-agreements"]
        project_contexts = ["@me/temper", "@me/knowledge"]
        index_path = "~/.claude/projects/p/memory/MEMORY.md"
    "#).expect("parses");
    let m = cfg.memory.expect("present");
    assert_eq!(m.shared_contexts, vec!["@me/working-agreements"]);
    assert_eq!(m.project_contexts, vec!["@me/temper", "@me/knowledge"]);
    assert_eq!(m.stale_after_days, 90, "default staleness threshold");
}

#[test]
fn memory_section_requires_at_least_one_context() {
    let cfg: TemperConfig = toml::from_str(r#"
        [vault]
        path = "~/x"
        [memory]
        shared_contexts = []
        project_contexts = []
        index_path = "~/x/MEMORY.md"
    "#).expect("parses");
    assert!(cfg.validate().is_err(), "an opted-in section naming no context renders an empty index silently");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p temper-core --lib memory_section`
Expected: FAIL — `no field 'memory' on TemperConfig`.

- [ ] **Step 3: Implement**

```rust
/// Claude Code memory projection. **Absent means the feature is off** — this is
/// `Option` rather than `#[serde(default)]` precisely so "not configured" and
/// "configured empty" stay distinguishable.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct MemoryConfig {
    /// Contexts whose memories reach EVERY project on this machine.
    #[serde(default)]
    pub shared_contexts: Vec<String>,
    /// Contexts for this project. A list — a project may legitimately span contexts.
    #[serde(default)]
    pub project_contexts: Vec<String>,
    /// Where the rendered index is written.
    #[validate(length(min = 1, message = "memory index_path cannot be empty"))]
    pub index_path: String,
    /// Days after which a memory's `verified` date is rendered as UNEXAMINED.
    #[serde(default = "default_stale_after_days")]
    pub stale_after_days: u32,
}

fn default_stale_after_days() -> u32 {
    90
}

impl MemoryConfig {
    /// Every context this machine renders for this project, shared first.
    pub fn all_contexts(&self) -> Vec<&str> {
        self.shared_contexts
            .iter()
            .chain(self.project_contexts.iter())
            .map(String::as_str)
            .collect()
    }
}
```

Add the `validator` rule and the field on `TemperConfig`:

```rust
// inside MemoryConfig, as a struct-level validation
#[validate(schema(function = "validate_has_a_context"))]

fn validate_has_a_context(cfg: &MemoryConfig) -> Result<(), ValidationError> {
    if cfg.shared_contexts.is_empty() && cfg.project_contexts.is_empty() {
        return Err(ValidationError::new("memory_no_contexts"));
    }
    Ok(())
}
```

```rust
// on TemperConfig, after `pub cli: CliSection,`
    /// Claude Code memory projection. `None` = feature off.
    #[serde(default)]
    #[validate(nested)]
    pub memory: Option<MemoryConfig>,
```

Add `memory: None` to the `impl Default for TemperConfig` block.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p temper-core --lib memory_section`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/config.rs
git commit -m "feat(memory): opt-in [memory] config section, absent by default"
```

---

### Task 2: The pure renderer and its validation

**Files:**
- Create: `crates/temper-cli/src/commands/memory/render.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` (add `pub mod memory;`)

**Interfaces:**
- Consumes: `MemoryConfig` (Task 1).
- Produces:
  - `pub struct MemoryEntry { pub id: Uuid, pub title: String, pub context_ref: String, pub descriptor: Option<String>, pub status: String, pub verified: NaiveDate }`
  - `pub enum MemoryDefect { MissingStatus { id, title }, MissingVerified { id, title }, BadStatus { id, title, found: String }, BadVerified { id, title, found: String } }`
  - `pub fn parse_entry(d: &ResourceDetail) -> Result<MemoryEntry, MemoryDefect>`
  - `pub fn render_index(entries: &[MemoryEntry], today: NaiveDate, stale_after_days: u32) -> String`

`today` is a **parameter, never `Utc::now()`** — a renderer that reads the clock cannot be tested deterministically.

- [ ] **Step 1: Write the failing tests**

```rust
use chrono::NaiveDate;

fn d(s: &str) -> NaiveDate { NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap() }

fn entry(title: &str, status: &str, verified: &str) -> MemoryEntry {
    MemoryEntry {
        id: uuid::Uuid::nil(),
        title: title.to_string(),
        context_ref: "@me/temper".to_string(),
        descriptor: Some("hook text".to_string()),
        status: status.to_string(),
        verified: d(verified),
    }
}

#[test]
fn superseded_memories_are_absent_from_the_index() {
    let out = render_index(
        &[entry("live one", "active", "2026-08-01"),
          entry("dead one", "superseded", "2026-08-01")],
        d("2026-08-01"), 90,
    );
    assert!(out.contains("live one"));
    assert!(!out.contains("dead one"), "superseded memories must not render");
}

#[test]
fn a_stale_entry_is_marked_unexamined_never_false() {
    let out = render_index(&[entry("old claim", "active", "2026-01-01")], d("2026-08-01"), 90);
    assert!(out.contains("UNEXAMINED"), "an over-threshold entry is marked");
    for word in ["STALE", "WRONG", "FALSE", "OUTDATED"] {
        assert!(!out.contains(word), "must not imply the claim is false, found {word}");
    }
}

#[test]
fn a_fresh_entry_carries_its_verified_date_and_no_marker() {
    let out = render_index(&[entry("fresh", "active", "2026-07-20")], d("2026-08-01"), 90);
    assert!(out.contains("[verified 2026-07-20]"));
    assert!(!out.contains("UNEXAMINED"));
}

#[test]
fn the_index_declares_itself_generated() {
    let out = render_index(&[entry("x", "active", "2026-08-01")], d("2026-08-01"), 90);
    assert!(out.starts_with("<!-- GENERATED by `temper memory emit` — do not edit -->"));
}

#[test]
fn entries_group_under_their_context() {
    let mut a = entry("from temper", "active", "2026-08-01");
    a.context_ref = "@me/temper".into();
    let mut b = entry("from agreements", "active", "2026-08-01");
    b.context_ref = "@me/working-agreements".into();
    let out = render_index(&[a, b], d("2026-08-01"), 90);
    assert!(out.contains("## @me/temper"));
    assert!(out.contains("## @me/working-agreements"));
}

#[test]
fn a_memory_missing_status_is_a_defect_not_a_default() {
    let row = row_with(None, Some("2026-08-01"));
    assert!(matches!(parse_entry(&row), Err(MemoryDefect::MissingStatus { .. })),
        "emit is the enforcement point; a missing key must never silently default to active");
}

#[test]
fn a_memory_with_an_unknown_status_is_a_defect() {
    let row = row_with(Some("activ"), Some("2026-08-01"));
    assert!(matches!(parse_entry(&row), Err(MemoryDefect::BadStatus { .. })));
}

#[test]
fn a_memory_with_an_unparseable_verified_date_is_a_defect() {
    let row = row_with(Some("active"), Some("last tuesday"));
    assert!(matches!(parse_entry(&row), Err(MemoryDefect::BadVerified { .. })));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-cli --lib memory::render`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement `render.rs`**

```rust
//! Pure rendering of the memory index. No I/O, no network, no clock.
//!
//! `emit` is the enforcement point for `open_meta.status` / `open_meta.verified`,
//! because those keys live in the OPEN tier and nothing validates them at write
//! time (design §"The open-tier cost"). A missing or malformed key is therefore a
//! DEFECT that fails the command — never a value that quietly defaults.

use chrono::NaiveDate;
use uuid::Uuid;

pub const GENERATED_HEADER: &str = "<!-- GENERATED by `temper memory emit` — do not edit -->";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntry {
    pub id: Uuid,
    pub title: String,
    pub context_ref: String,
    pub descriptor: Option<String>,
    pub status: String,
    pub verified: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryDefect {
    MissingStatus { id: Uuid, title: String },
    MissingVerified { id: Uuid, title: String },
    BadStatus { id: Uuid, title: String, found: String },
    BadVerified { id: Uuid, title: String, found: String },
}

impl std::fmt::Display for MemoryDefect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingStatus { id, title } =>
                write!(f, "{id} \"{title}\": open_meta.status is missing (expected \"active\" or \"superseded\")"),
            Self::MissingVerified { id, title } =>
                write!(f, "{id} \"{title}\": open_meta.verified is missing (expected an ISO date, YYYY-MM-DD)"),
            Self::BadStatus { id, title, found } =>
                write!(f, "{id} \"{title}\": open_meta.status is {found:?} (expected \"active\" or \"superseded\")"),
            Self::BadVerified { id, title, found } =>
                write!(f, "{id} \"{title}\": open_meta.verified is {found:?} (expected an ISO date, YYYY-MM-DD)"),
        }
    }
}

fn open_str(open_meta: &serde_json::Value, key: &str) -> Option<String> {
    open_meta.get(key)?.as_str().map(str::to_owned)
}

/// Parse one metadata row into an entry, or the specific defect that stops the render.
///
/// `list_meta` returns `ResourceMetaListResponse { rows: Vec<ResourceDetail>, .. }`
/// (`crates/temper-workflow/src/types/managed_meta.rs:143-147`). `ResourceDetail`
/// is a `#[serde(flatten)]`ed `ResourceRow` plus the two meta tiers
/// (`crates/temper-workflow/src/types/resource.rs:187-197`), so identity fields
/// are reached through `.row`.
pub fn parse_entry(d: &temper_workflow::types::resource::ResourceDetail)
    -> Result<MemoryEntry, MemoryDefect>
{
    let id = d.row.id.uuid();
    let title = d.row.title.clone();
    let om = d.open_meta.clone().unwrap_or(serde_json::Value::Null);

    let status = open_str(&om, "status")
        .ok_or_else(|| MemoryDefect::MissingStatus { id, title: title.clone() })?;
    if status != "active" && status != "superseded" {
        return Err(MemoryDefect::BadStatus { id, title, found: status });
    }

    let raw = open_str(&om, "verified")
        .ok_or_else(|| MemoryDefect::MissingVerified { id, title: title.clone() })?;
    let verified = NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
        .map_err(|_| MemoryDefect::BadVerified { id, title: title.clone(), found: raw })?;

    Ok(MemoryEntry {
        id,
        title,
        // `ResourceRow` carries `context_owner_ref` + `context_slug`, not a composed
        // `context_ref` — compose it here rather than assuming a field that is not there.
        context_ref: match (&d.row.context_owner_ref, &d.row.context_slug) {
            (Some(owner), Some(slug)) => format!("{owner}/{slug}"),
            _ => String::new(),
        },
        descriptor: open_str(&om, "descriptor"),
        status,
        verified,
    })
}

/// Render the index. `today` is a parameter, never the clock — a renderer that
/// reads the clock cannot be tested deterministically.
pub fn render_index(entries: &[MemoryEntry], today: NaiveDate, stale_after_days: u32) -> String {
    let mut out = String::from(GENERATED_HEADER);
    out.push_str("\n\n# Memory index\n");

    let mut contexts: Vec<&str> = entries
        .iter()
        .filter(|e| e.status == "active")
        .map(|e| e.context_ref.as_str())
        .collect();
    contexts.sort_unstable();
    contexts.dedup();

    for ctx in contexts {
        out.push_str(&format!("\n## {ctx}\n\n"));
        for e in entries.iter().filter(|e| e.status == "active" && e.context_ref == ctx) {
            let hook = e.descriptor.as_deref().unwrap_or("");
            let age = (today - e.verified).num_days();
            // UNEXAMINED, never STALE/WRONG: an old date means nobody has checked,
            // which is not evidence the claim is false (design, Global Constraints).
            let mark = if age > i64::from(stale_after_days) {
                format!("[verified {} — UNEXAMINED {}d]", e.verified, age)
            } else {
                format!("[verified {}]", e.verified)
            };
            let sep = if hook.is_empty() { "" } else { " — " };
            out.push_str(&format!("- [{}](temper://{}){}{}  {}\n", e.title, e.id, sep, hook, mark));
        }
    }
    out
}
```

Add a `row_with(status: Option<&str>, verified: Option<&str>) -> ResourceDetail` test helper in the test module: build a `ResourceRow` (its field list is at `crates/temper-workflow/src/types/resource.rs:19-57` — `id`, `title`, `context_owner_ref`, `context_slug`, `doc_type_name` are the ones this code reads), wrap it in `ResourceDetail { row, managed_meta: None, open_meta: Some(json!({..})) }`, and omit the supplied keys when the argument is `None`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-cli --lib memory::render`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/memory/render.rs crates/temper-cli/src/commands/mod.rs
git commit -m "feat(memory): pure index renderer, with defects instead of silent defaults"
```

---

### Task 3: `temper memory status`

**Files:**
- Create: `crates/temper-cli/src/commands/memory/mod.rs`
- Modify: `crates/temper-cli/src/cli.rs`, `crates/temper-cli/src/main.rs`

**Interfaces:**
- Consumes: `MemoryConfig` (Task 1), `render::{parse_entry, MemoryDefect}` (Task 2).
- Produces: `pub struct MemoryStatus { pub opted_in: bool, pub contexts: Vec<String>, pub in_temper: usize, pub defects: Vec<String>, pub local_files: usize, pub local_without_counterpart: Vec<String> }`, and `pub fn status_report(cfg: Option<&MemoryConfig>, rows: &[ResourceDetail], local: &[LocalMemoryFile]) -> MemoryStatus` (pure), plus `pub async fn status(...)` (the I/O shell).

Status is the **discovery** surface: it must work on a machine that has never opted in.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn status_works_when_not_opted_in() {
    let r = status_report(None, &[], &[local("feedback_x.md")]);
    assert!(!r.opted_in);
    assert_eq!(r.local_files, 1);
    assert_eq!(r.local_without_counterpart, vec!["feedback_x.md"],
        "an unadopted machine must still be told what it is carrying");
}

#[test]
fn status_matches_local_files_to_temper_by_slug() {
    let rows = vec![meta_row_titled("feedback_x", "@me/temper")];
    let r = status_report(Some(&cfg()), &rows, &[local("feedback_x.md"), local("feedback_y.md")]);
    assert_eq!(r.in_temper, 1);
    assert_eq!(r.local_without_counterpart, vec!["feedback_y.md"]);
}

#[test]
fn status_reports_defects_without_failing() {
    let rows = vec![meta_row_missing_status("feedback_x", "@me/temper")];
    let r = status_report(Some(&cfg()), &rows, &[]);
    assert_eq!(r.defects.len(), 1, "status REPORTS defects; only emit refuses on them");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-cli --lib memory::status`
Expected: FAIL — `status_report` not found.

- [ ] **Step 3: Implement**

`status_report` is pure over its three inputs. Slug matching strips the `.md` extension from the local filename and compares against the resource's `temper-slug`. The `status` shell reads local files from the directory containing `index_path`, calls `client.list_meta()` once per configured context, and prints via `output::`.

Wire the CLI:

```rust
// cli.rs — alongside `Skill`
    /// Manage the Claude Code memory projection
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },

#[derive(Subcommand, Debug)]
pub enum MemoryAction {
    /// Report this machine's memory state — works whether or not you have opted in
    Status,
}
```

```rust
// main.rs
        Commands::Memory { action } => match action {
            MemoryAction::Status => {
                let config = temper_cli::config::load(cli.vault.as_deref())?;
                temper_cli::commands::memory::status(&config, output_format).await
            }
        },
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-cli --lib memory::status && cargo run -q -p temper-cli -- memory status`
Expected: PASS (3 tests); the command prints a report and exits 0 even with no `[memory]` section.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/memory/mod.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(memory): temper memory status, the discovery surface for unadopted machines"
```

---

### Task 4: `temper memory emit`

**Files:**
- Modify: `crates/temper-cli/src/commands/memory/mod.rs`, `crates/temper-cli/src/cli.rs`, `crates/temper-cli/src/main.rs`

**Interfaces:**
- Consumes: `render::{parse_entry, render_index}` (Task 2), `MemoryConfig` (Task 1).
- Produces: `pub async fn emit(config: &TemperConfig, path_override: Option<&str>) -> Result<PathBuf>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn emit_refuses_when_any_memory_is_malformed() {
    let rows = vec![meta_row_missing_status("feedback_x", "@me/temper")];
    let err = build_index(&cfg(), &rows, d("2026-08-01")).expect_err("must refuse");
    assert!(err.to_string().contains("open_meta.status is missing"));
    assert!(err.to_string().contains("feedback_x"), "the error must name the offending memory");
}

#[test]
fn emit_reports_every_defect_not_just_the_first() {
    let rows = vec![
        meta_row_missing_status("a", "@me/temper"),
        meta_row_missing_verified("b", "@me/temper"),
    ];
    let err = build_index(&cfg(), &rows, d("2026-08-01")).expect_err("must refuse");
    assert!(err.to_string().contains("\"a\"") && err.to_string().contains("\"b\""),
        "one fix-run should not require N emit-runs to discover N defects");
}

#[test]
fn emit_is_a_noop_that_explains_itself_when_not_opted_in() {
    let outcome = emit_outcome(None);
    assert!(matches!(outcome, EmitOutcome::NotConfigured { .. }),
        "absent [memory] means OFF, and the command must say why rather than erroring");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-cli --lib memory::emit`
Expected: FAIL — `build_index` not found.

- [ ] **Step 3: Implement**

`build_index(cfg, rows, today) -> Result<String>` maps `parse_entry` over rows, **collects all defects** rather than short-circuiting, and returns an error listing every one. On success it calls `render_index`. `emit` writes the result to `index_path` (or the override), creating parent directories.

Add to `MemoryAction`:

```rust
    /// Render the index from Temper and write it
    Emit {
        /// Override the configured index_path (for a machine mid-adoption)
        #[arg(long)]
        path: Option<String>,
    },
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-cli --lib memory::emit`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/memory/mod.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(memory): temper memory emit, refusing on malformed open-tier keys"
```

---

### Task 5: `temper memory check` — the local drift gate

**Files:**
- Modify: `crates/temper-cli/src/commands/memory/mod.rs`, `crates/temper-cli/src/cli.rs`, `crates/temper-cli/src/main.rs`

**Interfaces:**
- Consumes: `build_index` (Task 4).
- Produces: `pub enum DriftVerdict { Match, Drifted { diff: String }, Absent }` and `pub async fn check(config: &TemperConfig) -> Result<DriftVerdict>`. Exit code 1 on `Drifted`.

**Why local, not CI:** the index lives outside the repo and is per-machine (see *Two corrections*). Nothing in git can diff it.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn check_reports_match_when_the_file_equals_a_fresh_render() {
    let rendered = "…";
    assert_eq!(compare_index(rendered, Some(rendered)), DriftVerdict::Match);
}

#[test]
fn check_reports_drift_when_the_file_was_hand_edited() {
    let v = compare_index("generated", Some("generated + a human edit"));
    assert!(matches!(v, DriftVerdict::Drifted { .. }));
}

#[test]
fn check_reports_absent_rather_than_drifted_when_there_is_no_file() {
    assert_eq!(compare_index("generated", None), DriftVerdict::Absent,
        "a machine that has never emitted has not DRIFTED; the two must not be conflated");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-cli --lib memory::check`
Expected: FAIL — `compare_index` not found.

- [ ] **Step 3: Implement**

`compare_index(rendered: &str, on_disk: Option<&str>) -> DriftVerdict` is pure. `check` reads the file, calls it, prints a unified diff on `Drifted`, and the `main.rs` arm maps `Drifted` to `std::process::exit(1)` so a hook can gate on it.

Add `Check` to `MemoryAction`.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-cli --lib memory::check`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/memory/mod.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(memory): temper memory check, the local drift gate"
```

---

### Task 6: Skill-side discovery, for two audiences

**Files:**
- Modify: `crates/temper-cli/templates/` — the CLI skill body and the shared/MCP template set
- Test: `crates/temper-cli/tests/` (skill render tests), plus the existing skills-drift gate

**Interfaces:**
- Consumes: nothing at runtime — this is copy.
- Produces: skill text that makes the convention discoverable.

**The constraint that must not break:** the MCP tree is config-free. It describes the doc type and the convention; it never names a user's contexts.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_cli_skill_points_at_memory_status() {
    let rendered = render_skill_for(Surface::Cli, &fixture_config());
    assert!(rendered.contains("temper memory status"),
        "an unadopted machine learns the feature exists from the CLI skill");
}

#[test]
fn the_mcp_skill_describes_the_convention_without_naming_a_context() {
    let rendered = render_skill_for(Surface::Mcp, &fixture_config());
    assert!(rendered.contains("type `memory`"), "Desktop needs to know the doc type");
    assert!(!rendered.contains("@me/"),
        "the MCP tree is config-free; naming a user context removes its ability to be gated");
    assert!(!rendered.contains("temper memory"),
        "Desktop has no CLI; pointing it at a CLI command is a dead end");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-cli --lib skill`
Expected: FAIL — the copy does not exist yet.

- [ ] **Step 3: Implement**

CLI-surface copy (may reference config and commands):

> **Memories.** This machine's `MEMORY.md` is generated from Temper by `temper memory emit`; do not hand-edit it. Run `temper memory status` to see what this machine carries and whether it has adopted the convention. `temper memory check` fails if the index has drifted.

MCP-surface copy (config-free, no CLI, no contexts):

> **Memories are resources.** Durable working knowledge is stored as resources of type `memory`, carrying `open_meta.status` (`active` / `superseded`) and `open_meta.verified` (the date the claim was last checked). Read the active ones and honour them. A `verified` date far in the past means **nobody has re-checked the claim** — not that it is false.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-cli --lib skill && cargo make check && bash .github/scripts/check-skills-drift.sh`
Expected: PASS; the skills-drift gate is green after re-emitting the committed tree.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/templates crates/temper-cli/tests agent-skills/
git commit -m "docs(memory): skill-side discovery for the CLI and MCP audiences"
```

---

## Out of scope — Phase 2

Deliberately **not** in this plan; each needs Phase 1 to exist first.

- **`temper memory migrate`** — the reconciling migration, interactive-by-default on collisions, refusing unattended runs without an explicit flag.
- **Migrating the 69 `feedback` memories** into `@me/working-agreements` as one reviewed batch.
- **Replacing the largest files with pointers** (`project_context_regions_goal.md`, 23KB, and siblings) rather than converting them.
- **The lazy tail** — the 107 `project` memories moving on next touch.

Phase 1's deliverable is that a machine can render, verify, and report on memories that exist. Phase 2 puts memories there.

## Self-review notes

- **Spec coverage:** config → T1; contract/validation → T2; staleness rendering → T2; status/discovery → T3; emit → T4; gate → T5 (form corrected); skill discovery, both audiences → T6. Migration and reconciliation are explicitly deferred to Phase 2 above, not dropped.
- **Type consistency:** `MemoryEntry`, `MemoryDefect`, `parse_entry`, `render_index`, `build_index`, `compare_index`, `DriftVerdict`, `MemoryConfig::all_contexts` are used with the same names and signatures in every task that references them.
- **One phantom type was caught and fixed before shipping this plan.** The first draft called the row type `ResourceMetaRow`; **no such type exists.** `list_meta` returns `ResourceMetaListResponse { rows: Vec<ResourceDetail>, total, facets }`, and `ResourceDetail` is a flattened `ResourceRow` plus `managed_meta`/`open_meta`. `ResourceRow` also has **no `context_ref` field** — it carries `context_owner_ref` and `context_slug`, which the renderer composes. Both are corrected above. Recording it because a plan that names a type which does not exist costs a full implementer round-trip, and this one nearly shipped.
- **Known gap, stated:** the tests in T3–T6 reference helper constructors (`meta_row_titled`, `meta_row_missing_status`, `local`, `cfg`, `fixture_config`, `render_skill_for`). Implementers build these in the test module of the task that first uses them, against the verified field list above.
