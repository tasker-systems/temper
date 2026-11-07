use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use askama::Template;
use clap::CommandFactory;
use sha2::{Digest, Sha256};

use crate::cli::Cli;
use crate::config::{self, Config};
use crate::error::{Result, TemperError};
use crate::output;
use crate::templates::{
    CommandWrapperTemplate, DataArtifactsTemplate, OutcomeRegistersTemplate,
    SessionLifecycleTemplate, SkillTemplate, WorkingAGoalTemplate,
};

// ── Surfaces ─────────────────────────────────────────────────────────────────

/// The two packagings of the same discipline. `cli` ships to `~/.claude/skills/temper/` and speaks
/// `temper …` commands; `mcp` is the committed `skills/` projection and speaks tool calls.
///
/// They exist as one template set because the discipline is about outcomes and grounding, which are
/// surface-independent by design — only the worked examples differ. Two hand-maintained copies of
/// the same prose is the drift this repo already has an open goal about, in a place where no
/// equivalence test could see it.
pub const SURFACE_CLI: &str = "cli";
pub const SURFACE_MCP: &str = "mcp";

/// Render a markdown template so it ends with exactly one newline.
///
/// askama strips a single trailing newline from the template file, so a template that ends the way
/// every other file in this tree ends renders one byte short. That is invisible until something
/// compares the render against a file on disk — which is exactly what the `skills/` drift gate
/// does, where it would read as permanent, uncloseable drift. Normalising here rather than padding
/// the template keeps the fix where a reader will find it.
fn render_md<T: Template>(template: &T) -> Result<String> {
    let mut rendered = template
        .render()
        .map_err(|e| TemperError::Config(format!("template render error: {}", e)))?;
    while rendered.ends_with('\n') {
        rendered.pop();
    }
    rendered.push('\n');
    Ok(rendered)
}

fn render_session_lifecycle(surface: &str) -> Result<String> {
    render_md(&SessionLifecycleTemplate { surface })
}

fn render_outcome_registers(surface: &str) -> Result<String> {
    render_md(&OutcomeRegistersTemplate { surface })
}

fn render_data_artifacts(surface: &str) -> Result<String> {
    render_md(&DataArtifactsTemplate { surface })
}

fn render_working_a_goal(surface: &str) -> Result<String> {
    render_md(&WorkingAGoalTemplate { surface })
}

// ── Static content (compiled into the binary) ────────────────────────────────

/// The MCP skill's router. A plain file, not a template, and deliberately so: it has no CLI twin
/// to share prose with (the CLI router bakes a per-user context list, which is exactly why that one
/// can never be a committed projection) and no values to interpolate. Templating it would buy a
/// seam with nothing on either side of it.
static MCP_SKILL_MD: &str = include_str!("../../skill-content/mcp/SKILL.md");
/// The memory-discovery copy, CLI surface. Was briefly a shared `{% if %}` template with the MCP
/// copy below; the two paragraphs turned out to share no prose and no interpolated value, so the
/// `surface` parameter was only ever selecting between two fixed strings — a seam with nothing on
/// either side of it, exactly what the `MCP_SKILL_MD` comment above already warns against. Two
/// plain consts buy identical testability (`the_cli_skill_points_at_memory_status`,
/// `the_mcp_skill_describes_the_convention_without_naming_a_context`) with less machinery.
static MEMORIES_CLI_MD: &str = include_str!("../../skill-content/memories-cli.md");
/// The memory-discovery copy, MCP surface. Config-free by construction: names the doc type and the
/// two `open_meta` keys, never a CLI command or a user's context — see `MEMORIES_CLI_MD` above for
/// why this is a plain const rather than the other branch of a shared template.
static MEMORIES_MCP_MD: &str = include_str!("../../skill-content/mcp/memories.md");
/// Grounding, plan verification, subagent briefing and review gates. Names no command on either
/// surface, so it ships to both verbatim.
static IMPLEMENTATION_GROUNDING_MD: &str =
    include_str!("../../skill-content/implementation-grounding.md");
static COGNITIVE_MAPS_MD: &str = include_str!("../../skill-content/cognitive-maps.md");
static QUERYING_MD: &str = include_str!("../../skill-content/querying.md");
static TEAMS_MD: &str = include_str!("../../skill-content/teams.md");
/// CLI-packaging only, like `cognitive-maps.md` and `teams.md`: `/temper init` is a slash
/// command on CLI hosts, and everything the flow touches (the wizard, the skill's `guidance/`
/// directory) exists only there. The MCP tree declares its absence rather than shipping an arm
/// that names doors its client does not have.
static PROJECT_SETUP_MD: &str = include_str!("../../skill-content/project-setup.md");
/// The admin surface, CLI-packaging only behind `skill install --include-admin`. Two reasons it
/// is not in the default tree, one per audience: a non-admin install must carry no signal the
/// set exists (the flag is the disclosure), and the MCP surface deliberately has no admin tools
/// — a tree teaching `temper admin …` to an MCP client points at doors that client cannot open.
/// Gated at install time rather than templated per-user: the content itself is static.
static ADMIN_MD: &str = include_str!("../../skill-content/admin.md");
static KNOWLEDGE_BASE_MD: &str =
    include_str!("../../../../skills/temper-knowledge-base/knowledge-base.md");

static REFERENCE_FOOTER: &str = r#"
## Resource Types

The recognized set, each with a schema (`resource create --type <t> --show-template` prints it). A
type outside it is accepted without a schema; `evidence`, a goal's evidence record, is the one the
skill uses on purpose:

{{ doc_type_table }}

## Task Stages

`backlog` → `in-progress` → `done`, or `cancelled`.

## Modes and Effort

Mode is `plan` (the output is a decision or spec) or `build` (the output is code). Effort is
`small` (one session), `medium` (bounded) or `large` (multi-session; decompose).

## Managed vs Open Frontmatter

`managed_meta` is a closed, temper-owned vocabulary (`temper-*` keys); an unknown key is rejected, so
caller-defined fields go in `open_meta`. The slug is always derived from the title and is display
only — addressing is by trailing UUID.

## Context Requirement

`resource create` requires `--context`; `resource list` takes it as an optional filter;
`show`/`update`/`delete` take a ref and no context. A context is `@me/<slug>`, `+team-slug/<slug>`
or a bare UUID — never a bare name.

## Listing: truncation, sort, and filters

`resource list` returns a page of 20 by default. **Never conclude a resource is absent, or a set
complete, from a default list.** Every response carries `total`, `returned` and `truncated`; when
`truncated` is true there is more, and the CLI also hints on stderr.

| Flag | Use |
|------|-----|
| `--all` | Every matching row. Reach for it before claiming completeness |
| `--limit <n>`, `--page <n>`, `--offset <n>` | A bigger page, or walk the set (`--page` is 1-indexed in units of `--limit`) |
| `--sort <field>[:asc\|desc]` | `updated` (default, desc), `created`, `title`, `stage`, `seq`, `context`, `doctype` |
| `--title-contains <s>` | Narrow instead of paging blind (topical search is `temper search`) |
| `--stage`, `--goal` (task) · `--status` (goal) · `--tag` (repeat narrows) | Filters; a type-specific one on the wrong `--type` is an error |

Omit `--type` to list across every doc type, e.g. a whole tag axis: `temper resource list --tag ci --all`.

## Orientation Projection

`show <ref> --without body` (everything but the body), `list … --with open-meta` (rows plus the open
tier), `--fields a,b,c` (top-level keys; `id` always kept), `show <ref> --edges` (composes with
`--without body`). `temper warmup --context @me/<ctx>` primes a session.

## Body Source

On `resource create`/`update` the body comes from the first that applies:

1. `--body @<path>` — the file; stdin ignored; an empty file is refused.
2. `--body -` — stdin, explicitly; refused on a TTY or empty input.
3. Implicit — stdin only when it is a non-TTY with input ready; an idle or empty pipe means no body.
4. None — the body is untouched; `update` patches frontmatter only.

**Footgun: implicit stdin is a body rewrite.** Inside `while read n ref; do temper resource update
"$ref" --title "…"; done < refs.txt`, every `update` inherits the loop's stdin and overwrites a body
with the leftover lines, with no error. Change frontmatter one call at a time with stdin untouched;
rewrite a body only on purpose, preferably with `--body @file`.

## Block-Grain Ingest & Attribution

A body can be built as distinctly addressable blocks, each with provenance (the sources it was
distilled from) and authorship (confidence, reasoning, model). Reach for it for large or
citation-graded documents. It builds and attributes blocks; it does not edit one block's text in a
finished resource — that is still a whole-body `update`.

- **On any write:** `--sources <refs-or-urls>` (a URL may carry `#L<start>-L<end>`),
  `--content-block <uuid>` (required once a resource has more than one block), and authorship flags,
  where `--confidence` is required if any other is given. `create --from <url|path>` extracts and
  records the source; `--sources-as-edges` also asserts `derived_from` edges.
- **`resource annotate <ref> --sources …`** backfills provenance with no body revise: `body_hash` and
  embeddings are unchanged.
- **Reading:** `resource show <ref> --provenance`.
- **Large bodies:** the CLI segments and resumes automatically. On MCP, `segmented_ingest` exposes the
  lifecycle by `action`: `begin` (creates the resource, returns an opaque `body_hash`), `append`
  (segments from seq 1, idempotent), `blocks` (read back to resume), `finalize` (pass
  `expected_blocks` and echo `body_hash` verbatim; a gap fails). See `knowledge-base.md`.

## Error Handling (JSON mode)

Check the exit code before reading output. On failure stdout carries
`{ "error": { "code", "message", "hint" } }`; branch on `code`, never on `message`. A success may
carry an advisory `diagnostics` array, which is not a failure.

## Skill-Only Commands

Handled by the skill router, not the CLI: `task start <slug>`, `task resume <slug>`, `task create`,
`session start`, `session wrap`. See `SKILL.md`.
"#;

// ── Generated frontmatter reference ──────────────────────────────────────────

/// Prose that frames the generated tables. Kept beside them rather than in a template because it
/// is meaningless without them — it explains what the derivation *is*.
static FRONTMATTER_PREAMBLE: &str = r#"# Frontmatter Reference

> **Generated from `temper_workflow`'s embedded JSON Schemas — the same ones the server validates
> against.** Do not hand-edit: run the emit command and commit the result. It exists in generated
> form for one reason. Its hand-written predecessor described a set of primitives temper had
> already retired, and nothing could notice, because a document describing types is not connected
> to the types. Derived from the validator's own source, this one cannot name a doc type that does
> not exist, nor omit one that does.

A resource's frontmatter has **two tiers**, and the distinction is enforced on write:

| Tier | Vocabulary | Rejects unknown keys? |
|------|-----------|----------------------|
| `managed_meta` | **Closed** — the `temper-*` workflow/provenance keys below | **Yes.** An unknown key is an error |
| `open_meta` | **Open** — anything you like | No. Some keys are *recognized* (below); the rest are stored as-is |

Identity is **not** metadata. `title`, the doc type, and the resource's home (a context or a
cognitive map) are first-class fields on the write call, never `managed_meta` keys. The **slug is
derived from the title** on every surface — there is no way to set one, and a slug placed in
frontmatter is inert.
"#;

/// Generate `references/frontmatter.md` from the embedded doc-type schemas.
///
/// Everything here is read from `temper_workflow::schema` — `DOC_TYPE_NAMES` for the type set,
/// `updatable_fields` + `enum_fields` for the per-type keys, `base_schema_value` for the universal
/// ones, and `describe_open_meta` for the open tier. That sourcing *is* the feature: the validator
/// and this document cannot disagree, because there is only one of them.
pub fn generate_frontmatter_reference() -> Result<String> {
    use temper_workflow::frontmatter::fields::SYSTEM_MANAGED_FIELDS;
    use temper_workflow::schema;

    let mut out = String::from(FRONTMATTER_PREAMBLE);

    out.push_str("\n## Doc types\n\n");
    out.push_str(&format!(
        "These {} are the recognized set, each with a schema. A `doc_type_name` outside it is \
         accepted but carries no type schema, so a misspelled type silently becomes a new one \
         rather than an error.\n\n",
        schema::DOC_TYPE_NAMES.len()
    ));
    for name in schema::DOC_TYPE_NAMES {
        out.push_str(&format!("- `{name}`\n"));
    }

    out.push_str("\n## `managed_meta` by doc type\n\n");
    out.push_str(
        "The type-specific half of the closed vocabulary. A key listed against one type is \
         accepted on that type; sending it on another is a validation error, not a silent \
         no-op.\n\n",
    );
    out.push_str("| Doc type | Keys | Must the caller send one? |\n|---|---|---|\n");
    let wire_keys = managed_wire_keys()?;
    for name in schema::DOC_TYPE_NAMES {
        let fields = schema::updatable_fields(name)?;
        let enums = schema::enum_fields(name)?;
        let mut described: Vec<String> = fields
            .iter()
            .filter(|(key, _)| wire_keys.contains(key))
            .map(|(key, prop)| describe_property(key, prop, &enums))
            .collect();
        described.sort();
        let keys = if described.is_empty() {
            "*(none beyond the universal set)*".to_string()
        } else {
            described.join("<br>")
        };
        // A schema-required key the server defaults is not required *of a caller*. Both halves are
        // read from the code that enforces them, so this column cannot drift from either.
        let defaults = defaulted_managed_keys(name);
        let mut required: Vec<String> = schema::required_fields(name)?
            .into_iter()
            .filter(|f| !SYSTEM_MANAGED_FIELDS.contains(&f.as_str()) && wire_keys.contains(f))
            .map(|f| match defaults.get(&f) {
                Some(value) => format!("`{f}` — optional, defaults to `{value}`"),
                None => format!("`{f}` — **required**"),
            })
            .collect();
        required.sort();
        let required = if required.is_empty() {
            "no".to_string()
        } else {
            required.join("<br>")
        };
        out.push_str(&format!("| `{name}` | {keys} | {required} |\n"));
    }

    out.push_str("\n## Universal `managed_meta` keys\n\n");
    out.push_str("Accepted on every doc type.\n\n");
    let base = schema::base_schema_value()?;
    let base_enums = extract_base_enum_fields(&base);
    let mut universal: Vec<String> = base_properties(&base)
        .into_iter()
        .filter(|(key, _)| wire_keys.contains(*key))
        .map(|(key, prop)| format!("- {}\n", describe_property(key, prop, &base_enums)))
        .collect();
    universal.sort();
    for line in universal {
        out.push_str(&line);
    }

    out.push_str("\n## Server-managed fields — never send these\n\n");
    out.push_str(
        "Stamped by the server. They appear on every resource you read and are rejected (or \
         ignored) on write.\n\n",
    );
    for field in SYSTEM_MANAGED_FIELDS {
        out.push_str(&format!("- `{field}`\n"));
    }

    out.push_str("\n## `open_meta` — the open tier\n\n");
    let convention = schema::describe_open_meta()?;
    out.push_str(
        "Any key is accepted and stored. The keys below are *recognized*: they carry a declared \
         shape, and some are indexed for search. An unrecognized key is neither an error nor \
         indexed.\n\n",
    );
    out.push_str("| Key | Shape | Notes |\n|---|---|---|\n");
    if let Some(props) = convention
        .schema
        .get("properties")
        .and_then(|p| p.as_object())
    {
        for (key, prop) in props {
            let shape = json_type_label(prop);
            let notes = prop
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("—");
            out.push_str(&format!("| `{key}` | {shape} | {notes} |\n"));
        }
    }
    if !convention.discouraged_keys.is_empty() {
        out.push_str(
            "\n**Discouraged keys.** Legal, but they shadow a managed field and drift \
                      away from it silently:\n\n",
        );
        for d in &convention.discouraged_keys {
            out.push_str(&format!(
                "- `{}` — the canonical value lives in `{}`\n",
                d.key, d.use_instead
            ));
        }
    }

    Ok(out)
}

/// The `managed_meta` **wire** vocabulary, derived from the type that defines it.
///
/// This is deliberately not "every `temper-*` property in the JSON Schema". The schema describes
/// frontmatter as it rests on disk, which is a *superset*: it carries identity keys like
/// `temper-title` that `ManagedMeta`'s `deny_unknown_fields` rejects at the wire boundary. A
/// reference generated from the schema alone therefore tells an agent to send a key that hard-fails
/// — which is the same genre of defect as documenting a doc type that does not exist, one tier down.
///
/// Built from an **exhaustive struct literal** — no `..Default::default()` — so a field added to
/// `ManagedMeta` stops this file compiling until the reference is taught about it.
fn managed_wire_keys() -> Result<std::collections::BTreeSet<String>> {
    use temper_workflow::types::managed_meta::ManagedMeta;

    let all = ManagedMeta {
        stage: Some(String::new()),
        mode: Some(String::new()),
        effort: Some(String::new()),
        status: Some(String::new()),
        seq: Some(0),
        branch: Some(String::new()),
        pr: Some(String::new()),
        llm_model: Some(String::new()),
        llm_run: Some(String::new()),
        provenance: Some(String::new()),
    };

    let value = serde_json::to_value(&all)
        .map_err(|e| TemperError::Config(format!("ManagedMeta serialization error: {e}")))?;
    let obj = value.as_object().ok_or_else(|| {
        TemperError::Config("ManagedMeta did not serialize to an object".to_string())
    })?;
    Ok(obj.keys().cloned().collect())
}

/// The managed keys a doc type's own defaults fill in, as `key → default value`.
///
/// Read by running the real default pass over an empty object, so "is this actually required of a
/// caller?" is answered by the code that answers it at write time. `temper-stage` is schema-required
/// on `task` and also auto-defaulted to `backlog`; reporting it as flatly *required* would send
/// agents hunting for a value the server supplies.
fn defaulted_managed_keys(doc_type: &str) -> BTreeMap<String, String> {
    let mut meta = serde_json::json!({});
    temper_workflow::defaults::apply_managed_defaults(doc_type, &mut meta);
    meta.as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    let rendered = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                    (k.clone(), rendered)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Base-schema properties as `(name, definition)` pairs, in schema order.
fn base_properties(base: &serde_json::Value) -> Vec<(&str, &serde_json::Value)> {
    base.get("properties")
        .and_then(|p| p.as_object())
        .map(|o| o.iter().map(|(k, v)| (k.as_str(), v)).collect())
        .unwrap_or_default()
}

/// Enum values declared directly on the base schema's properties, keyed by property name.
fn extract_base_enum_fields(base: &serde_json::Value) -> BTreeMap<String, Vec<String>> {
    base_properties(base)
        .into_iter()
        .filter_map(|(key, prop)| {
            let values: Vec<String> = prop
                .get("enum")?
                .as_array()?
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            (!values.is_empty()).then(|| (key.to_string(), values))
        })
        .collect()
}

/// Render one property as `` `key` — description (one of: a, b) ``.
///
/// The enum values come from the caller's already-computed map rather than the property, because
/// `enum_fields` has the null-filtering rule (`temper-mode` is `[plan, build, null]` in the schema
/// and `[plan, build]` to a caller) and re-deriving it here would be a second, divergent copy.
fn describe_property(
    key: &str,
    prop: &serde_json::Value,
    enums: &BTreeMap<String, Vec<String>>,
) -> String {
    let mut rendered = format!("`{key}`");
    if let Some(desc) = prop.get("description").and_then(|d| d.as_str()) {
        rendered.push_str(&format!(" — {desc}"));
    }
    match enums.get(key) {
        Some(values) => rendered.push_str(&format!(
            " (one of: {})",
            values
                .iter()
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        None => {
            if let Some(label) = prop.get("type").and_then(json_scalar_type_label) {
                rendered.push_str(&format!(" ({label})"));
            }
        }
    }
    rendered
}

/// A human label for a JSON Schema `type` node, skipping the uninformative bare `"string"`.
fn json_scalar_type_label(type_node: &serde_json::Value) -> Option<String> {
    match type_node.as_str() {
        Some("string") => None,
        Some(other) => Some(other.to_string()),
        None => None,
    }
}

/// A shape label for an open_meta property, which may use `type`, `oneOf`, or neither.
fn json_type_label(prop: &serde_json::Value) -> String {
    if let Some(t) = prop.get("type").and_then(|t| t.as_str()) {
        return t.to_string();
    }
    if prop.get("oneOf").is_some() || prop.get("anyOf").is_some() {
        return "string or array".to_string();
    }
    "any".to_string()
}

/// Generate the reference.md content from clap's command tree.
///
/// `include_admin` selects whether the `admin` subtree's rows appear. A default install's skill
/// carries no admin content at all, and these rows ARE admin content — every row naming an
/// admin verb is exactly the disclosure the flag is supposed to be — so they are filtered from
/// the default render and restored under `--include-admin`. The rows are still clap-derived on
/// both paths, so the admin render cannot name a verb the binary does not have.
pub fn generate_reference(include_admin: bool) -> String {
    let cmd = Cli::command();
    let mut rows = Vec::new();
    collect_command_rows(&cmd, "", &mut rows);
    if !include_admin {
        rows.retain(|(name, _)| name != "admin" && !name.starts_with("admin "));
    }

    let mut out = String::new();
    out.push_str("# CLI Reference\n\n");
    out.push_str("## Invocation\n\n");
    out.push_str("Run `temper` from PATH (`which temper`). If it is missing, say so and stop: never fall back to\n");
    out.push_str(
        "`cargo run` or a full path, which reach a different binary than the one installed.\n\n",
    );
    out.push_str("## Commands\n\n");
    out.push_str("| Command | Syntax |\n");
    out.push_str("|---------|--------|\n");
    for (name, syntax) in &rows {
        out.push_str(&format!("| {} | `{}` |\n", name, syntax));
    }
    out.push_str("\nPipe content via stdin for `resource create` (all types accept stdin body).\n");
    // The doc-type set is derived from the validator's own enumeration rather than restated,
    // so a type added or retired in temper-workflow cannot leave this table contradicting the
    // same skill's `memories.md` (which teaches creating `memory`-typed resources).
    let doc_type_table = {
        use temper_workflow::schema;
        let mut table = String::from("| Type | Notes |\n|------|-------|\n");
        for name in schema::DOC_TYPE_NAMES {
            let notes = match *name {
                "task" => "Work items with stage, mode, effort tracking",
                "goal" => "High-level objectives that group tasks",
                "session" => "Timestamped work session notes",
                "research" => "Research notes and findings",
                "concept" => "Named ideas, patterns, or domain terms",
                "decision" => "Point-in-time choices with rationale (ADR-like)",
                "memory" => "Durable statements carried across sessions (see `memories.md`)",
                _ => "Per-type schema via `--show-template`",
            };
            table.push_str(&format!("| {name} | {notes} |\n"));
        }
        table
    };
    out.push_str(&REFERENCE_FOOTER.replace("{{ doc_type_table }}", &doc_type_table));
    out
}

/// Recursively collect (command_name, syntax_string) rows from the clap command tree.
fn collect_command_rows(cmd: &clap::Command, prefix: &str, rows: &mut Vec<(String, String)>) {
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        let name = sub.get_name();
        let full_name = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{} {}", prefix, name)
        };

        // If this command has subcommands, recurse into them
        let child_subs: Vec<_> = sub.get_subcommands().filter(|c| !c.is_hide_set()).collect();
        if !child_subs.is_empty() {
            // If subcommand is optional, also emit a row for the parent command
            if !sub.is_subcommand_required_set() {
                let syntax = build_syntax(&full_name, sub);
                rows.push((full_name.clone(), syntax));
            }
            collect_command_rows(sub, &full_name, rows);
        } else {
            let syntax = build_syntax(&full_name, sub);
            rows.push((full_name, syntax));
        }
    }
}

/// Build a syntax string like `temper task create --title <title> [--context @me/<ctx>]`
fn build_syntax(full_name: &str, cmd: &clap::Command) -> String {
    let mut parts = vec![format!("temper {}", full_name)];

    for arg in cmd.get_arguments() {
        // Skip hidden args
        if arg.is_hide_set() {
            continue;
        }
        let id = arg.get_id().as_str();
        // Skip --format (implementation detail)
        if id == "format" {
            continue;
        }
        // Skip global --help and --version
        if id == "help" || id == "version" || id == "vault" {
            continue;
        }

        if arg.is_positional() {
            if arg.is_required_set() {
                parts.push(format!("<{}>", id));
            } else {
                parts.push(format!("[<{}>]", id));
            }
        } else {
            // Flag/option arg
            let long = arg
                .get_long()
                .map(|l| format!("--{}", l))
                .unwrap_or_else(|| format!("--{}", id));

            let takes_value = !matches!(
                arg.get_action(),
                clap::ArgAction::SetTrue | clap::ArgAction::SetFalse | clap::ArgAction::Count
            );
            if takes_value {
                if arg.is_required_set() {
                    parts.push(format!("{} <{}>", long, id));
                } else {
                    parts.push(format!("[{} <{}>]", long, id));
                }
            } else {
                // Boolean flag
                parts.push(format!("[{}]", long));
            }
        }
    }

    parts.join(" ")
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Generate all skill files as a map of relative_path → content (the default, non-admin tree).
pub fn generate_skill_files(config: &Config) -> Result<HashMap<String, String>> {
    let hash = compute_config_hash()?;
    generate_skill_files_with_hash(config, &hash, false)
}

/// Returns the generated reference.md content for stdout preview.
pub fn generate(_config: &Config, include_admin: bool) -> Result<String> {
    Ok(generate_reference(include_admin))
}

// ── The MCP surface: a committed projection ──────────────────────────────────

/// Generate the `skills/` projection tree as a map of relative_path → content.
///
/// **Config-free, and that is the whole reason this tree can be committed.** The CLI skill bakes a
/// per-user context list into its router, so its rendered form is one developer's and could never be
/// checked in; the MCP skill has no such input, so its render is a pure function of this source tree
/// and a drift gate can pin it the way `openapi.json` and the ts-rs trees are pinned.
///
/// Deliberately **not** every file in the bundle. `knowledge-base.md` is hand-written and stays
/// that way — it is the MCP tool reference and has no CLI twin to share with, so emitting over it
/// would be a rewrite. The gate only ever compares what appears here; the tool-NAME half of that
/// gap is covered by a test in temper-mcp-server, against the tool layer's declarations.
pub fn generate_agent_skill_files() -> Result<HashMap<String, String>> {
    let mut files = HashMap::new();

    files.insert("SKILL.md".to_string(), MCP_SKILL_MD.to_string());
    files.insert(
        "references/frontmatter.md".to_string(),
        generate_frontmatter_reference()?,
    );
    files.insert(
        "session-lifecycle.md".to_string(),
        render_session_lifecycle(SURFACE_MCP)?,
    );
    files.insert(
        "outcome-registers.md".to_string(),
        render_outcome_registers(SURFACE_MCP)?,
    );
    files.insert(
        "data-artifacts.md".to_string(),
        render_data_artifacts(SURFACE_MCP)?,
    );
    files.insert(
        "working-a-goal.md".to_string(),
        render_working_a_goal(SURFACE_MCP)?,
    );
    files.insert("memories.md".to_string(), MEMORIES_MCP_MD.to_string());
    // Names no command on either surface, so both trees carry the same bytes.
    files.insert(
        "implementation-grounding.md".to_string(),
        IMPLEMENTATION_GROUNDING_MD.to_string(),
    );

    Ok(files)
}

/// Write the generated `skills/` projection tree into `dir`, returning the relative paths written.
///
/// Every write goes through `write_if_changed`, so re-emitting an up-to-date tree touches no mtimes
/// — which is what lets the drift gate run this unconditionally and read `git status` afterwards.
pub fn emit_agent_skills(dir: &Path) -> Result<Vec<String>> {
    let files = generate_agent_skill_files()?;
    let mut written: Vec<String> = Vec::new();

    for (rel_path, content) in &files {
        let dest = dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TemperError::Config(format!(
                    "cannot create parent dir for {}: {}",
                    dest.display(),
                    e
                ))
            })?;
        }
        write_if_changed(&dest, content)?;
        written.push(rel_path.clone());
    }

    written.sort();
    Ok(written)
}

/// Report of what changed during `install`.
///
/// Lets the caller distinguish between a real refresh and a no-op so agents
/// (and humans) can tell whether an install actually did anything.
#[derive(Debug, Default)]
pub struct InstallReport {
    pub total: usize,
    pub changed: Vec<String>,
    /// Installer-era duplicate files removed from `guidance/` (byte-matched against the exact
    /// revisions the old installer shipped — user-modified files are never touched).
    pub removed_legacy: Vec<String>,
    /// Files earlier releases shipped and this one no longer does, removed when their bytes match
    /// a revision the installer shipped (see `RETIRED_SKILL_FILES`).
    pub removed_retired: Vec<String>,
    /// Admin files removed because this install ran WITHOUT `--include-admin` over a tree that
    /// had them. Byte-matched against what the installer ships — a file the user has edited is
    /// theirs, not ours to delete.
    pub removed_admin: Vec<String>,
}

impl InstallReport {
    pub fn is_no_op(&self) -> bool {
        self.changed.is_empty()
            && self.removed_legacy.is_empty()
            && self.removed_retired.is_empty()
            && self.removed_admin.is_empty()
    }
}

// ── Skill targets ────────────────────────────────────────────────────────────

use crate::cli::SkillTarget;

/// The per-target install paths: where the skill directory lives and where the command wrapper
/// goes. Each agent has its own config home; the skill content and the wrapper template are shared
/// (opencode discovers Claude-Code-compat skills and reads the same command frontmatter format).
impl SkillTarget {
    /// The default skill directory under the target's config home.
    pub fn default_skill_dir(&self, home: &Path) -> PathBuf {
        match self {
            // The vendor-neutral location: not any one agent's config home, but the
            // `~/.agents` convention several clients converge on for shared skill content.
            SkillTarget::Agents => home.join(".agents/skills/temper"),
            SkillTarget::Claude => home.join(".claude/skills/temper"),
            SkillTarget::Opencode => home.join(".config/opencode/skills/temper"),
        }
    }

    /// The command-wrapper directory under the target's config home, when the target has one.
    /// `Agents` has none — `~/.agents` is a skill-content location, and `/temper` command
    /// routing stays an individual agent's concern.
    fn command_dir(&self, home: &Path) -> Option<PathBuf> {
        match self {
            SkillTarget::Agents => None,
            SkillTarget::Claude => Some(home.join(".claude/commands")),
            SkillTarget::Opencode => Some(home.join(".config/opencode/command")),
        }
    }
}

/// Install skill directory and command wrapper.
///
/// 1. Generate all skill files
/// 2. Write skill files (except command-wrapper.md) into `skill_dir`
/// 3. Write command-wrapper.md to the target's command directory
///
/// `target` selects the command-wrapper location (Claude vs opencode config home). `skill_dir` is
/// already resolved by the caller (from `--path` or the target's default) — the install function
/// itself is path-agnostic. `include_admin` selects whether the admin skill set rides along; a
/// default install over a tree that has admin files removes the ones the installer itself shipped
/// (byte-matched — see `remove_admin_files`), so the absence stays assertable after an upgrade
/// path that once carried the flag.
///
/// Skips writes when the destination already matches the generated content,
/// returning an `InstallReport` that lists every file whose bytes changed.
pub fn install(
    config: &Config,
    skill_dir: &Path,
    target: SkillTarget,
    include_admin: bool,
) -> Result<InstallReport> {
    let files = generate_skill_files_with_hash(config, &compute_config_hash()?, include_admin)?;
    let mut report = InstallReport {
        total: files.len(),
        changed: Vec::new(),
        removed_legacy: Vec::new(),
        removed_retired: Vec::new(),
        removed_admin: Vec::new(),
    };

    // Ensure skill_dir and its guidance/ subdirectory exist
    let guidance = skill_dir.join("guidance");
    std::fs::create_dir_all(&guidance).map_err(|e| {
        TemperError::Config(format!(
            "cannot create directory {}: {}",
            guidance.display(),
            e
        ))
    })?;

    // Write all files except command-wrapper.md into skill_dir
    for (rel_path, content) in &files {
        if rel_path == "command-wrapper.md" {
            continue;
        }
        let dest = skill_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TemperError::Config(format!(
                    "cannot create parent dir for {}: {}",
                    dest.display(),
                    e
                ))
            })?;
        }
        if write_if_changed(&dest, content)? {
            report.changed.push(rel_path.clone());
        }
    }

    // Write command-wrapper.md to the target's command directory, when the target has one.
    if let Some(wrapper_content) = files.get("command-wrapper.md") {
        if let Some(commands_dir) =
            target.command_dir(&dirs::home_dir().ok_or_else(|| {
                TemperError::Config("cannot determine home directory".to_string())
            })?)
        {
            std::fs::create_dir_all(&commands_dir).map_err(|e| {
                TemperError::Config(format!("cannot create {}: {}", commands_dir.display(), e))
            })?;
            let wrapper_path = commands_dir.join("temper.md");
            if write_if_changed(&wrapper_path, wrapper_content)? {
                report.changed.push("command-wrapper.md".to_string());
            }
        }
    }

    report.removed_legacy = remove_legacy_guidance_duplicates(skill_dir)?;
    report.removed_retired = remove_retired_files(skill_dir, &RETIRED_SKILL_FILES)?;
    if !include_admin {
        report.removed_admin = remove_admin_files(skill_dir)?;
    }

    Ok(report)
}

/// Remove admin files from a tree installed without `--include-admin`, keyed by content hash —
/// the same rule the legacy-guidance cleanup follows and for the same reason: a file whose bytes
/// match what the installer ships is ours to retract (the tree must return to a no-admin state),
/// while a file the user has since edited is theirs and stays.
fn remove_admin_files(skill_dir: &Path) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for (rel_path, content) in admin_skill_files() {
        let path = skill_dir.join(rel_path);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            // Absent is the expected state; unreadable is not ours to judge by hash.
            Err(_) => continue,
        };
        let shipped_earlier =
            ADMIN_SHIPPED_REVISIONS.contains(&hex::encode(Sha256::digest(&bytes)).as_str());
        if bytes == content.as_bytes() || shipped_earlier {
            std::fs::remove_file(&path).map_err(|e| {
                TemperError::Config(format!("cannot remove {}: {}", path.display(), e))
            })?;
            removed.push(rel_path.to_string());
        }
    }
    Ok(removed)
}

/// SHA-256 of every revision of `admin.md` that reached `main`, so an install made by an earlier
/// release is recognized as ours to retract, not only one matching the current bytes.
const ADMIN_SHIPPED_REVISIONS: [&str; 9] = [
    "1f39616f4d83f59be3c9d069a297d1b4d2ea5111b681964e190e258c84e91535",
    "2db09a7648f824d0483087bc6886816b4f1d92ade99680d157bd60b655e9d309",
    "347aa2c00d557547b1fcdafb539c3908e1a27f3f34dc6dd34bf85f3b22f6d6d8",
    "5046dcc0e29c618bf346ecfd91cbbd8f9e40e1bf4ed0b34c67fce0ab06d3446d",
    "59bf151766f6f2bc0971603873c286da051ef061e853bf83af0715823a39acb5",
    "6c052656c1371c210607d53513c2500f0db838c5cee332da44379f612f90a51e",
    "71553a12e9f42b55d11e6672fea86fba75c97ba05fd0025a41fd882b66b0214b",
    "ea83382bad6770ee03a1dc0bf4071227d0cd0e0690bc5c862ce5d7539ab87e91",
    "f7518d65d12d434604a7d72275cb2de2371bfaeb76d0cd7034b3cb85fb57613b",
];

/// The pre-root-shipping installer wrote the grounding pair into `guidance/`. It now ships at the
/// skill root and writes nothing into `guidance/` (the user's namespace), so those copies survive
/// as stale revisions — and the router's "read every file in `guidance/`" step makes an agent read
/// them as current. Remove a legacy copy **only** when its bytes match the exact revision the old
/// installer shipped: a file a user has since edited is theirs, not ours to delete. Keyed by hash
/// rather than by "not present at root", so the check survives even if the root copy is later
/// renamed or the guidance file is regenerated.
const LEGACY_GUIDANCE_DUPLICATES: [(&str, &str); 2] = [
    (
        "guidance/implementation-grounding.md",
        "29ae9493f2e774820ce18d7a6d49c257fcf7999fa5b5ef8c2ef8f75a4705a023",
    ),
    (
        "guidance/plan-verification.md",
        "1d7e2545159019ea56b79159f91f04faa8ea7e3091c481e1270c5bb8167d4790",
    ),
];

/// Remove stale installer-era duplicates from `guidance/`, byte-matched against
/// [`LEGACY_GUIDANCE_DUPLICATES`]. Returns the paths removed.
fn remove_legacy_guidance_duplicates(skill_dir: &Path) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for (rel_path, legacy_hash) in LEGACY_GUIDANCE_DUPLICATES {
        let path = skill_dir.join(rel_path);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            // Absent, or unreadable — nothing to clean up, and an unreadable file is not
            // ours to judge by hash anyway.
            Err(_) => continue,
        };
        let hash = hex::encode(Sha256::digest(&bytes));
        if hash == legacy_hash {
            std::fs::remove_file(&path).map_err(|e| {
                TemperError::Config(format!("cannot remove {}: {}", path.display(), e))
            })?;
            removed.push(rel_path.to_string());
        }
    }
    Ok(removed)
}

/// Files earlier releases installed and this one no longer ships, with the SHA-256 of every
/// revision of each that ever reached `main` (so an install from any release is recognized). Without
/// this, a retired file survives every upgrade and an agent browsing the skill directory reads it as
/// current. A file whose bytes match none of these was edited by its owner and is left alone.
const RETIRED_SKILL_FILES: [(&str, &str); 43] = [
    (
        "plan-verification.md",
        "8869c9972e65a0798e3ae97bd4cbb70a76130387794a67b1f2edd3fc297a5d33",
    ),
    (
        "review-gates.md",
        "fa1fd7581b6afe92a428a361550ee2c09ce6ce67e9564c5ccb9f5a944ccfd5bf",
    ),
    (
        "session-wrap.md",
        "077be522dd65c9597dc4bc2f0b50e443a66b0384c891810add1ef35625b190bc",
    ),
    (
        "session-wrap.md",
        "811b81a3f18c23654be9008fbd41b3296cffbd5515075b67cd967ca3d1339ef8",
    ),
    (
        "session-wrap.md",
        "8c748bcfa6d8ef3a273a9ff9657d71dbf612e82c6ac87e9f58dee1de4956ef39",
    ),
    (
        "session-wrap.md",
        "8d945d1954f1e66acb2b630ad908f277c92aa5e84147819b3b8701944d72c339",
    ),
    (
        "session-wrap.md",
        "be02c96eddea0c66ec43fa0922e25477f9a8578a9f4cab672d47217b72fd0753",
    ),
    (
        "subagent-guidance.md",
        "24d80ae8cba411a9abb6e5d7367f64f61b89519c8a4f99430b64adc631fa9ccc",
    ),
    (
        "workflows/build-large.md",
        "068eabd607aba5b1dd695f37a06f63adeb911cd283aa86230783db64f39f8d82",
    ),
    (
        "workflows/build-large.md",
        "2ecea58a9f4c02fe506b909769ac78a460f719892cb44b414f0595a64add16be",
    ),
    (
        "workflows/build-large.md",
        "309bd05c42c021b91134368023b4fae529aa89597c80a9f98a122323c9041dfc",
    ),
    (
        "workflows/build-large.md",
        "a3955c146e2b14fb7dcbf73d37b04d4544ebbfbc2fd29cb67ade309987369ebc",
    ),
    (
        "workflows/build-large.md",
        "d333a801f1393e5d83228dbdd510d04e27b7f3f31bd1c77576be48f0f6967cc9",
    ),
    (
        "workflows/build-large.md",
        "f4471bd71d510efa6fba2ae2a9ae259dbb7ca09309b43af798c9f4f334f226fb",
    ),
    (
        "workflows/build-medium.md",
        "33d9684da1f66f829b47386bfadb21536933cfceeb43f48d62b3f4c3a7f8678c",
    ),
    (
        "workflows/build-medium.md",
        "9277cae4cec56192ab90187aeb26f0fb8fc2ac3c00d68ecfee17430dddbe978f",
    ),
    (
        "workflows/build-medium.md",
        "99ce3a178db82d96783ab200c314220ef2ea898b6ceaa44c0e1ba914038bdcad",
    ),
    (
        "workflows/build-medium.md",
        "a7fc57587392c240c2fe8aa4a6f4ea7be3def1bb3ad8a3f31b586de079c65cc6",
    ),
    (
        "workflows/build-medium.md",
        "c257ada56a7d265d9b707e9f80ae7606bb9cb930d38ce706503ec7c0d7b0b79e",
    ),
    (
        "workflows/build-medium.md",
        "ef70635b85b8efefd096ded57358c92e7f92fdf1c1fb661fb762ce164b58630b",
    ),
    (
        "workflows/build-small.md",
        "05d305d12a5cfdaee6875c3863616a9d69a1edc63b7d6fbe2c9d1a9dc108bc87",
    ),
    (
        "workflows/build-small.md",
        "3766a251a991a5821e1b0d3f9f37f6f5caeaf81297ef269a324b956fbae9d1bb",
    ),
    (
        "workflows/build-small.md",
        "ccab827499e9110edd86401fb3d5281027b10ae900031b5a03d67c0d3af7cd3a",
    ),
    (
        "workflows/build-small.md",
        "f58062547c827cb4dc676a1b8c624dde83babf7d7dc21bba7712331dc91e3f08",
    ),
    (
        "workflows/build-small.md",
        "feb4c080bdbf98008e91050e080d302d7c9e914ff513ad293737c5f624de45f5",
    ),
    (
        "workflows/plan-large.md",
        "2567dd3a69fc7deef2f8183b14bb5bbde336025177fad7af60de8e50fa2ff789",
    ),
    (
        "workflows/plan-large.md",
        "4b174b6ea16782545ed116bc44b89473dda6c386f8a9df4e1798ccff18919580",
    ),
    (
        "workflows/plan-large.md",
        "563452c6b1e667935a0614d0ac19f0d6338a02d3b06a1fa34fadd683d03978d0",
    ),
    (
        "workflows/plan-large.md",
        "5b96b670d9de4ea967d959177c12bee10b90c81ac6dc19e7fcfe83a1e3617fac",
    ),
    (
        "workflows/plan-large.md",
        "5d144e2d7171deb65ca003ab8511cc70215749ea4a117319dac81550822f464a",
    ),
    (
        "workflows/plan-large.md",
        "6e6d0bfbdbed63e642d91d977e29ff96a37b1fd9f18a35ef972845d1e5927fa8",
    ),
    (
        "workflows/plan-large.md",
        "b44fbd4dfe6cfa99fc4bad579dcf009476c3e6883f52ae8a4d1da9830623c7d2",
    ),
    (
        "workflows/plan-medium.md",
        "438c7ab4391944bdae5ad5be4534227237b32242505369121137c912fb6d0e14",
    ),
    (
        "workflows/plan-medium.md",
        "4c26045182ed173ea7666c1ba12b33a2d354cfc3240a1cdc6786ce35dac00e7e",
    ),
    (
        "workflows/plan-medium.md",
        "5dab6e12d7aa474dfd2917948c2b4872528eec12adfec056ae3fe4a981e9757a",
    ),
    (
        "workflows/plan-medium.md",
        "5e79dd2b271b37aae1439a2df423fec29e93f0e0010420dd070f7ab60a24aa52",
    ),
    (
        "workflows/plan-medium.md",
        "8698afe267bff26dbfade2dff6703d3ad5249ef2fadb8d242121507049b776a7",
    ),
    (
        "workflows/plan-medium.md",
        "e4549f49c0fb43a05b210cb9d41a2c7280fecab8cc6025eff2ab128af4c3a1b8",
    ),
    (
        "workflows/plan-small.md",
        "03c2fc145cc5f41866dfcde0b8298e8a6bda7f267b3128e6ec891df2e9b9df91",
    ),
    (
        "workflows/plan-small.md",
        "177bfcdc26479749323ded5c47b11eff19c1784b32db848be834ad06aed9d4c3",
    ),
    (
        "workflows/plan-small.md",
        "22b12e8f4029c931727ce4c0fda0e7bd4859d1d0380fb427ac686a909bc0cc22",
    ),
    (
        "workflows/plan-small.md",
        "7c03a9b637e4f24f229901da847a79670de8f9737c204727dc9882fd2d2e0728",
    ),
    (
        "workflows/plan-small.md",
        "99cd0b0325b26ce974b01f61caa8aef1665596ab411219dc91441b60d17e8402",
    ),
];

/// Remove each retired file whose bytes hash to one of its shipped revisions in `retired`, then
/// remove a `workflows/` directory left empty. Returns the paths removed.
fn remove_retired_files(skill_dir: &Path, retired: &[(&str, &str)]) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for (rel_path, shipped_hash) in retired {
        let path = skill_dir.join(rel_path);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if hex::encode(Sha256::digest(&bytes)) == *shipped_hash {
            std::fs::remove_file(&path).map_err(|e| {
                TemperError::Config(format!("cannot remove {}: {}", path.display(), e))
            })?;
            removed.push((*rel_path).to_string());
        }
    }
    let workflows = skill_dir.join("workflows");
    if std::fs::read_dir(&workflows).is_ok_and(|mut d| d.next().is_none()) {
        let _ = std::fs::remove_dir(&workflows);
    }
    Ok(removed)
}

/// Write `content` to `dest` only if the on-disk bytes differ. Returns
/// `true` if a write happened (or the file did not previously exist).
fn write_if_changed(dest: &Path, content: &str) -> Result<bool> {
    if let Ok(existing) = std::fs::read_to_string(dest) {
        if existing == content {
            return Ok(false);
        }
    }
    std::fs::write(dest, content)
        .map_err(|e| TemperError::Config(format!("cannot write {}: {}", dest.display(), e)))?;
    Ok(true)
}

/// Check skill installation status.
pub fn check(config: &Config, target: SkillTarget) -> Result<()> {
    // 1. Check skill directory exists — a missing directory short-circuits
    //    the remaining checks.
    let skill_dir = &config.skill_output;
    if !skill_dir.exists() {
        output::status_icon(
            false,
            format!("Skill directory: NOT FOUND ({})", skill_dir.display()),
        );
        output::hint("  Run: temper skill install");
        return Ok(());
    }
    output::status_icon(true, format!("Skill directory: {}", skill_dir.display()));

    check_expected_files(skill_dir);
    check_config_hash_staleness(skill_dir)?;
    check_command_wrapper(target);

    Ok(())
}

/// Report which of the expected skill files are present under `skill_dir`.
fn check_expected_files(skill_dir: &Path) {
    let expected_files = [
        "SKILL.md",
        "reference.md",
        "implementation-grounding.md",
        "outcome-registers.md",
        "working-a-goal.md",
        "data-artifacts.md",
        "session-lifecycle.md",
        "memories.md",
        "cognitive-maps.md",
        "teams.md",
        "querying.md",
        "knowledge-base.md",
        "project-setup.md",
    ];

    let mut all_present = true;
    for file in &expected_files {
        let path = skill_dir.join(file);
        if !path.exists() {
            output::status_icon(false, format!("Missing: {}", file));
            all_present = false;
        }
    }
    if all_present {
        output::status_icon(
            true,
            format!("All {} skill files present", expected_files.len()),
        );
    }
}

/// Compare the config-hash embedded in `SKILL.md` against the current config
/// hash, reporting up-to-date / stale / unknown status.
fn check_config_hash_staleness(skill_dir: &Path) -> Result<()> {
    let skill_md_path = skill_dir.join("SKILL.md");
    if skill_md_path.exists() {
        let existing = std::fs::read_to_string(&skill_md_path)
            .map_err(|e| TemperError::Config(format!("cannot read SKILL.md: {}", e)))?;

        let embedded_hash = extract_config_hash(&existing);
        let current_hash = compute_config_hash()?;

        match embedded_hash {
            Some(h) if h == current_hash => {
                output::status_icon(true, "Hash: up to date");
            }
            Some(h) => {
                output::status_icon(false, "Hash: STALE");
                output::plain(format!("  Embedded: {}", h));
                output::plain(format!("  Current:  {}", current_hash));
                output::hint("  Run: temper skill install");
            }
            None => {
                // Same stream (stdout) and shape (`status_icon`) as the up-to-date
                // and STALE arms above: this is a *report*, and a non-passing outcome
                // routed to stderr would render as silence for a caller reading the
                // stdout report — indistinguishable from "the check did not run",
                // which is the worst reading for UNKNOWN specifically.
                output::status_icon(false, "Hash: UNKNOWN (no config-hash comment found)");
            }
        }
    }
    Ok(())
}

/// Check the command wrapper at the target's command path. `Agents` has no wrapper by design;
/// that is stated rather than left to be inferred from a missing line.
fn check_command_wrapper(target: SkillTarget) {
    let Some(commands_dir) =
        target.command_dir(&dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("~")))
    else {
        output::plain("Command wrapper: none (this target installs skill content only)");
        return;
    };

    let wrapper_path = commands_dir.join("temper.md");

    if wrapper_path.exists() {
        output::status_icon(true, format!("Command wrapper: {}", wrapper_path.display()));
    } else {
        output::status_icon(
            false,
            format!("Command wrapper: NOT FOUND ({})", wrapper_path.display()),
        );
        output::hint("  Run: temper skill install");
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// The files `--include-admin` adds to the tree, as `(path, content)` — the single source for
/// both the install-with-flag write and the install-without-flag cleanup, so they cannot
/// disagree about what an admin set is.
fn admin_skill_files() -> Vec<(&'static str, &'static str)> {
    vec![("admin.md", ADMIN_MD)]
}

/// Generate all skill files with a pre-computed config hash (for testability).
///
/// `include_admin` selects whether the admin skill set rides along. The default (`false`) is the
/// install a non-admin gets: `reference.md` carries no admin rows, no admin file is shipped, and
/// the router names none — the flag's help text is the only disclosure that the set exists.
pub fn generate_skill_files_with_hash(
    config: &Config,
    hash: &str,
    include_admin: bool,
) -> Result<HashMap<String, String>> {
    let context_list = format_context_list(&config.contexts);

    let skill_template = SkillTemplate {
        config_hash: hash,
        context_list: &context_list,
        include_admin,
    };

    let wrapper_template = CommandWrapperTemplate { config_hash: hash };

    let mut files = HashMap::new();

    files.insert(
        "SKILL.md".to_string(),
        skill_template
            .render()
            .map_err(|e| TemperError::Config(format!("template render error: {}", e)))?,
    );

    files.insert(
        "command-wrapper.md".to_string(),
        wrapper_template
            .render()
            .map_err(|e| TemperError::Config(format!("template render error: {}", e)))?,
    );

    files.insert(
        "reference.md".to_string(),
        generate_reference(include_admin),
    );
    // Shipped at the skill root, never into `guidance/`, which is the user's namespace (install
    // only creates it; writing there would clobber user files on every regen).
    files.insert(
        "implementation-grounding.md".to_string(),
        IMPLEMENTATION_GROUNDING_MD.to_string(),
    );
    // The outcome-register discipline. Root, not `guidance/`, for the same reason as the grounding
    // pair: `guidance/` is the project's namespace and this is universal, repo-agnostic discipline.
    files.insert(
        "outcome-registers.md".to_string(),
        render_outcome_registers(SURFACE_CLI)?,
    );
    files.insert(
        "data-artifacts.md".to_string(),
        render_data_artifacts(SURFACE_CLI)?,
    );
    files.insert(
        "session-lifecycle.md".to_string(),
        render_session_lifecycle(SURFACE_CLI)?,
    );
    files.insert(
        "working-a-goal.md".to_string(),
        render_working_a_goal(SURFACE_CLI)?,
    );
    files.insert("memories.md".to_string(), MEMORIES_CLI_MD.to_string());
    files.insert(
        "cognitive-maps.md".to_string(),
        COGNITIVE_MAPS_MD.to_string(),
    );
    files.insert("teams.md".to_string(), TEAMS_MD.to_string());
    files.insert("project-setup.md".to_string(), PROJECT_SETUP_MD.to_string());
    // CLI-only, and declared absent from the MCP tree rather than shipped wrong: MCP has no query
    // tool yet (spec [2] defers it), so an MCP reader routed here would be sent to a door that is
    // not on their surface. Same reason `cognitive-maps.md` and `teams.md` are CLI-only.
    files.insert("querying.md".to_string(), QUERYING_MD.to_string());

    files.insert(
        "knowledge-base.md".to_string(),
        KNOWLEDGE_BASE_MD.to_string(),
    );

    // The admin set rides LAST and only under the flag: a default render's file map never
    // carries an admin path, which is what makes "a default install ships no admin files"
    // assertable from the map alone.
    if include_admin {
        for (path, content) in admin_skill_files() {
            files.insert(path.to_string(), content.to_string());
        }
    }

    Ok(files)
}

/// Compute SHA256 hash of the global config file.
fn compute_config_hash() -> Result<String> {
    let config_path = config::global_config_path();
    let config_content = std::fs::read_to_string(&config_path)
        .map_err(|e| TemperError::Config(format!("cannot read config: {}", e)))?;
    Ok(hex::encode(Sha256::digest(config_content.as_bytes())))
}

/// Format contexts as sorted markdown list items.
pub fn format_context_list(contexts: &[String]) -> String {
    if contexts.is_empty() {
        return "(no contexts configured)".to_string();
    }
    let mut sorted = contexts.to_vec();
    sorted.sort();
    sorted
        .iter()
        .map(|ctx| format!("- `{ctx}`"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the config hash from a `<!-- config-hash: ... -->` comment.
pub fn extract_config_hash(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("<!-- config-hash: ") {
            if let Some(hash) = rest.strip_suffix(" -->") {
                return Some(hash.to_string());
            }
        }
    }
    None
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    fn test_config() -> Config {
        Config {
            vault_root: PathBuf::from("/tmp/test-vault"),
            state_dir: PathBuf::from("/tmp/test-vault/.temper"),
            contexts: vec!["alpha".to_string(), "beta".to_string()],
            skill_output: PathBuf::from("/tmp/test-skill-output"),
            profile_slug: None,
        }
    }

    /// **Shipping a guidance file is not the same as making it reachable.**
    ///
    /// `implementation-grounding.md` existed and was correct for weeks while SKILL.md's procedural
    /// steps named only `guidance/fundamentals.md` — so no session ever loaded it, and plans kept
    /// shipping invention laundered as grounding. The router is what makes a supporting file real:
    /// agents execute the numbered steps, not the directory listing.
    ///
    /// So this pins the wiring, not the file's existence. A supporting file the router never names
    /// is dead content, and it fails silently — which is exactly how it rotted the first time.
    #[test]
    fn every_shipped_guidance_file_is_named_by_the_router() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash", false).unwrap();
        let skill_md = files.get("SKILL.md").expect("SKILL.md");

        // Being named is the bar, not being named in a *numbered step*: `outcome-registers.md` is
        // reached on demand from the routing table (plus a one-line stanza in the always-read
        // steps), which is deliberate — it is ~200 lines that only a goal-authoring session needs.
        // Unreachable is the failure this pins; expensive-on-every-task is a separate judgment.
        //
        // **Derived from what is actually shipped, not a list restated here.** This was four
        // hand-written names, which meant the guard covered the four files that had already rotted
        // and no future one — a new supporting file could be added, shipped, and never routed to,
        // which is precisely the silent failure the test exists to catch. `querying.md` was the
        // file that made that gap concrete. Deriving the set means adding a file to the emit is
        // enough to bring it under the guard.
        let unrouted: Vec<&String> = files
            .keys()
            .filter(|name| {
                // Two exclusions:
                //
                // - `SKILL.md` is the router; it cannot route to itself.
                // - `command-wrapper.md` is not a supporting file at all. `install` writes it to
                //   `~/.claude/commands/temper.md` and excludes it from the skill dir (see the
                //   write loop above). It is the ENTRY POINT — the `/temper` slash command — so a
                //   router naming it would point backwards, at the thing that invoked the router.
                name.as_str() != "SKILL.md" && name.as_str() != "command-wrapper.md"
            })
            .filter(|name| !skill_md.contains(name.as_str()))
            .collect();

        assert!(
            unrouted.is_empty(),
            "SKILL.md never mentions {unrouted:?}, so no session will read them — name them in the \
             router's Read-when table or do not ship them at all"
        );
    }

    #[test]
    fn test_generate_skill_files_contains_expected_keys() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash", false).unwrap();

        assert!(files.contains_key("SKILL.md"));
        assert!(files.contains_key("reference.md"));
        assert!(files.contains_key("implementation-grounding.md"));
        assert!(files.contains_key("outcome-registers.md"));
        assert!(files.contains_key("data-artifacts.md"));
        assert!(files.contains_key("session-lifecycle.md"));
        assert!(files.contains_key("working-a-goal.md"));
        assert!(files.contains_key("memories.md"));
        assert!(files.contains_key("cognitive-maps.md"));
        assert!(files.contains_key("teams.md"));
        assert!(files.contains_key("project-setup.md"));
        assert!(files.contains_key("knowledge-base.md"));
        assert!(files.contains_key("command-wrapper.md"));
        // None of the retired files ships again: a path in both lists would be installed and then
        // deleted by the same install.
        for (retired, _) in RETIRED_SKILL_FILES {
            assert!(
                !files.contains_key(retired),
                "{retired} is retired but still shipped"
            );
        }
    }

    #[test]
    fn test_generate_skill_md_has_contexts_and_no_local_vault_path() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash", false).unwrap();
        let skill_md = &files["SKILL.md"];

        assert!(skill_md.contains("alpha"));
        assert!(skill_md.contains("beta"));
        assert!(skill_md.contains("config-hash: testhash"));

        // The skill no longer advertises a local vault directory. Writes are cloud-only and the
        // on-disk projection is a power-user affordance, so naming a path here taught every agent
        // to reach for the wrong surface. Asserted as an ABSENCE because the failure mode is a
        // re-add, not a removal.
        assert!(
            !skill_md.contains(&config.vault_root.display().to_string()),
            "SKILL.md must not name a local vault path"
        );
    }

    /// **The frontmatter must start at byte 0 — nothing else may precede it.**
    ///
    /// opencode and Claude Code parse skill frontmatter with a strict `---` at
    /// byte 0; any leading line (a `<!-- config-hash -->` comment, an HTML
    /// preamble, anything) causes the parser to treat the whole frontmatter
    /// block as body text. The `description` field is then never read, the
    /// skill is filtered out of the registry, and the model is never told the
    /// skill exists — silent and total. This pins the invariant the
    /// leading-comment bug broke.
    #[test]
    fn skill_md_frontmatter_starts_at_byte_zero() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash", false).unwrap();
        let skill_md = &files["SKILL.md"];

        assert!(
            skill_md.starts_with("---\n"),
            "SKILL.md must begin with `---\\n` at byte 0 so frontmatter parsers read it; \
             got: {:?}",
            skill_md.chars().take(40).collect::<String>()
        );
    }

    /// The referencing convention is the reason sessions stop writing unresolvable id prefixes.
    /// It lives in SKILL.md, the one file every session reads; a silent drop is invisible until
    /// documents start citing nothing again.
    #[test]
    fn test_reference_convention_is_stated_in_the_router() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash", false).unwrap();
        let skill_md = &files["SKILL.md"];
        assert!(
            skill_md.contains("Never abbreviate a UUID"),
            "SKILL.md must state that UUIDs are never abbreviated"
        );
        assert!(
            skill_md.contains("](./<full-uuid>)"),
            "SKILL.md must show the [title](./uuid) reference form"
        );
    }

    // ── The MCP projection ───────────────────────────────────────────────────

    #[test]
    fn agent_skill_files_are_the_expected_set() {
        let files = generate_agent_skill_files().unwrap();
        let mut keys: Vec<&str> = files.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "SKILL.md",
                "data-artifacts.md",
                "implementation-grounding.md",
                "memories.md",
                "outcome-registers.md",
                "references/frontmatter.md",
                "session-lifecycle.md",
                "working-a-goal.md",
            ],
            "the emitted set moved — the drift gate only ever compares what appears here, so a file \
             dropped from this map silently stops being checked"
        );
        // The hand-written tool reference is NOT emitted. Asserted as an ABSENCE because the
        // failure mode is someone folding it in, which would overwrite maintained prose on every
        // regeneration.
        assert!(
            !files.contains_key("knowledge-base.md"),
            "knowledge-base.md is hand-maintained; emitting it would clobber it"
        );
    }

    // ── Memory-convention discovery (both audiences) ─────────────────────────
    //
    // These tests read `MEMORIES_CLI_MD`/`MEMORIES_MCP_MD` directly — not the whole rendered
    // tree. The full MCP tree already contains `@me/<ctx>` placeholders throughout
    // (`session-lifecycle.md`, `outcome-registers.md`, `SKILL.md` all use it as generic
    // addressing syntax), so a whole-tree render would fail
    // `the_mcp_skill_describes_the_convention_without_naming_a_context`'s negative assertions for
    // reasons that have nothing to do with the memory copy itself. Scoping to the one const under
    // test is what makes the assertion mean what it says.

    #[test]
    fn the_cli_skill_points_at_memory_status() {
        let rendered = MEMORIES_CLI_MD;
        assert!(
            rendered.contains("temper memory status"),
            "an unadopted machine learns the feature exists from the CLI skill"
        );
    }

    /// **The populate half.** Phase 1 shipped discovery of `status`/`emit`/`check` — read and
    /// verify — and named neither command that puts anything *into* the store, so a session on a
    /// machine full of un-migrated files could learn the convention exists and still never learn it
    /// had something to move. Asserted per command: dropping either of the two write commands
    /// leaves a workflow that cannot be run from what is shipped.
    #[test]
    fn the_cli_skill_names_every_command_of_the_workflow() {
        let rendered = MEMORIES_CLI_MD;
        for cmd in [
            "temper memory status",
            "temper memory harvest",
            "temper memory migrate",
            "temper memory emit",
            "temper memory check",
        ] {
            assert!(
                rendered.contains(cmd),
                "the CLI memory guidance never names `{cmd}`, so no session will find it"
            );
        }
    }

    /// `harvest` before the `emit` takeover is not a preference. The takeover destroys the only
    /// copy of every curated title, and with it `migrate`'s ability to move the remaining files at
    /// all — a one-way loss for whoever takes them in the other order.
    ///
    /// Asserted as **document order**, not as a phrase. A prose assertion passes on any sentence
    /// that happens to contain the words; what a reader actually follows is the sequence the file
    /// presents, so that is what this pins.
    #[test]
    fn the_cli_skill_puts_harvest_before_migrate_and_the_takeover() {
        let rendered = MEMORIES_CLI_MD;
        let harvest = rendered
            .find("temper memory harvest")
            .expect("harvest is named — see the_cli_skill_names_every_command_of_the_workflow");
        let migrate = rendered
            .find("temper memory migrate")
            .expect("migrate is named — see the_cli_skill_names_every_command_of_the_workflow");
        let emit_section = rendered
            .find("## Rendering and gating the index")
            .expect("the takeover is its own section");
        assert!(
            harvest < migrate,
            "harvest must be presented before migrate — a file whose title was never stamped is \
             skipped by migrate, so the reverse order silently strands the tail"
        );
        assert!(
            migrate < emit_section,
            "the emit takeover must come last — taken first it destroys the titles harvest exists \
             to preserve"
        );
    }

    /// A reader who takes `migrate` for a bulk import runs it unattended and is surprised by a
    /// refusal — or, worse, reads the surviving batch confirmation as a duplicate check. It is
    /// one: the pre-write search was deleted, so `migrate`'s whole reconciliation is the
    /// `source_file` skip, and `--unattended` authorizes the batch rather than skipping any check.
    /// Each of these carries a consequence a session acts on.
    #[test]
    fn the_cli_skill_says_migrate_reconciles_rather_than_bulk_imports() {
        let rendered = MEMORIES_CLI_MD;
        for claim in [
            "reconciles rather than bulk-imports",
            "interactive by default",
            "refuses to write",
            "--unattended",
            "--dry-run",
            "source_file",
            "detects nothing about near-duplicates",
        ] {
            assert!(
                rendered.contains(claim),
                "the CLI memory guidance never states `{claim}`, so a session will expect a bulk \
                 import and be wrong about what a run does"
            );
        }
    }

    /// The MCP audience has no local files and no migration to run, but it populates the store the
    /// most ordinary way there is — by creating a resource. Guidance that only says *read the
    /// active ones* leaves that surface able to consume memories and never contribute one.
    #[test]
    fn the_mcp_skill_teaches_authoring_a_memory_not_only_reading_one() {
        let rendered = MEMORIES_MCP_MD;
        assert!(
            rendered.contains("create_resource"),
            "the MCP memory guidance never names a write tool, so it teaches reading only"
        );
        assert!(
            rendered.contains("superseded"),
            "correcting by supersession is how a wrong memory is fixed without destroying the \
             account of having been wrong"
        );
        assert!(
            rendered.contains("`verified` is a judgment"),
            "nothing validates the open tier at write time, so the one rule that keeps `verified` \
             meaningful has to be stated where the write is taught"
        );
    }

    #[test]
    fn the_mcp_skill_describes_the_convention_without_naming_a_context() {
        let rendered = MEMORIES_MCP_MD;
        assert!(
            rendered.contains("type `memory`"),
            "Desktop needs to know the doc type"
        );
        assert!(
            !rendered.contains("@me/"),
            "the MCP tree is config-free; naming a user context removes its ability to be gated"
        );
        assert!(
            !rendered.contains("temper memory"),
            "Desktop has no CLI; pointing it at a CLI command is a dead end"
        );
    }

    /// The pre-write collision search is gone — measured at 54 surfaced / 3 real on a 184-memory
    /// store `[2026-08-02]`, ~94% of it matching on shared project vocabulary rather than on
    /// claim. The **rule** it enforced is not gone, so the discipline now lives in the prose on
    /// both surfaces, and prose is the only thing carrying it: nothing compiled in will fire if
    /// this is dropped.
    ///
    /// Asserted on the load-bearing terms, not on sentences. `duplicate` alone would pass on any
    /// paragraph containing the word, which is precisely the protection this needs not to be:
    ///
    /// - `the claim, not the wording` — what to compare. Without it a reader falls back to
    ///   comparing text, which is the failure the deleted search embodied.
    /// - `older, richer` / `strictly richer` / `both stale` — the three observed outcome kinds,
    ///   one term each. They are asserted separately because they resolve *differently*: the first
    ///   keeps the **older** account, and the third is the case where recency arbitrates nothing.
    ///   A reader who learns only "duplicate" mis-handles the other two.
    ///
    /// Lower-cased before matching so a heading, a bolded label or a sentence-initial capital is
    /// not a failure — the test is about the content being present, not its typography.
    #[test]
    fn both_memory_surfaces_teach_judging_a_near_duplicate_by_claim() {
        for (surface, rendered) in [("CLI", MEMORIES_CLI_MD), ("MCP", MEMORIES_MCP_MD)] {
            let lowered = rendered.to_lowercase();
            for claim in [
                "the claim, not the wording",
                "older, richer",
                "strictly richer",
                "both stale",
            ] {
                assert!(
                    lowered.contains(claim),
                    "the {surface} memory guidance never states `{claim}` — with the collision \
                     search deleted, an author who is not told to compare the claim will compare \
                     wording, and one who is told only about duplicates will mis-resolve a \
                     supersession or a pair that are both stale"
                );
            }
        }
    }

    /// `memories.md` is shipped in both trees but is dead content unless each surface's router
    /// names it — the same failure mode `every_shipped_guidance_file_is_named_by_the_router` and
    /// `the_mcp_router_names_every_file_it_ships` guard against for every other supporting file.
    #[test]
    fn memories_md_is_named_by_both_routers() {
        let cli_config = test_config();
        let cli_files = generate_skill_files_with_hash(&cli_config, "testhash", false).unwrap();
        assert!(
            cli_files["SKILL.md"].contains("memories.md"),
            "the CLI SKILL.md never mentions memories.md, so no session will read it"
        );

        let mcp_files = generate_agent_skill_files().unwrap();
        assert!(
            mcp_files["SKILL.md"].contains("memories.md"),
            "the MCP SKILL.md never mentions memories.md, so no session will read it"
        );
    }

    /// Both routers must warn that a body write REPLACES, and must say to verify before sending.
    ///
    /// The two `SKILL.md` files have no shared template — the CLI one is an askama render carrying
    /// a per-user context list, the MCP one is static, and the shared supporting files
    /// (`reference.md` and friends) are CLI-only or MCP-only. So a rule that belongs on both is
    /// written twice, and nothing but this test keeps the second copy from being forgotten when
    /// the first is edited.
    ///
    /// The rule earns a guard because it was learned twice in one session `[2026-08-02]`: a
    /// concurrent edit from a second machine was nearly overwritten, and then a single-section
    /// splice truncated everything after its anchor — the second with no concurrency involved at
    /// all, which is why the guidance is about *verifying the write* rather than about locking.
    #[test]
    fn both_routers_warn_that_a_body_write_replaces_and_must_be_verified() {
        let cli = generate_skill_files_with_hash(&test_config(), "testhash", false).unwrap();
        let mcp = generate_agent_skill_files().unwrap();

        for (surface, rendered) in [("CLI", &cli["SKILL.md"]), ("MCP", &mcp["SKILL.md"])] {
            assert!(
                rendered.contains("Re-read immediately before writing"),
                "the {surface} SKILL.md does not tell a session to re-read before writing, so the \
                 window between reading a body and replacing it goes unmentioned"
            );
            assert!(
                rendered.contains("BOTH ends of a splice"),
                "the {surface} SKILL.md does not say to assert at both ends of a splice — \
                 confirming new text landed says nothing about what it displaced"
            );
            assert!(
                rendered.contains("not a lock"),
                "the {surface} SKILL.md must state this is discipline rather than mutexing, or a \
                 reader will file the absence of locking as the defect"
            );
        }
    }

    /// The projection must be a pure function of the source tree — no config, no environment, no
    /// home directory. That is the whole reason it can be committed and gated, where the CLI skill
    /// (which bakes a per-user context list) cannot.
    #[test]
    fn agent_skill_files_are_config_free_and_deterministic() {
        let first = generate_agent_skill_files().unwrap();
        let second = generate_agent_skill_files().unwrap();
        assert_eq!(first, second, "the emit is not deterministic");
    }

    /// **The specific thing that stops Ticket/Milestone recurring.**
    ///
    /// The predecessor to this tree documented `Ticket` and `Milestone` schemas long after temper
    /// stopped having either. This asserts the generated reference names every real doc type and
    /// that the tree names no retired one — the failure is a re-add, so both halves are needed.
    #[test]
    fn the_mcp_tree_names_every_real_doc_type_and_no_retired_one() {
        let files = generate_agent_skill_files().unwrap();
        let frontmatter = files.get("references/frontmatter.md").unwrap();

        for doc_type in temper_workflow::schema::DOC_TYPE_NAMES {
            assert!(
                frontmatter.contains(&format!("`{doc_type}`")),
                "the generated frontmatter reference never names the `{doc_type}` doc type"
            );
        }

        // The primitives the hand-written tree taught, none of which temper has ever shipped.
        let whole_tree: String = files.values().cloned().collect::<Vec<_>>().join("\n");
        for retired in [
            "ticket",
            "Ticket",
            "milestone",
            "Milestone",
            "Epic Workflow",
        ] {
            assert!(
                !whole_tree.contains(retired),
                "the MCP skill names `{retired}`, which is not a temper primitive"
            );
        }
    }

    /// The MCP surface has its own field names, and getting one wrong is a rejected write rather
    /// than a cosmetic slip. `context_name` is an OUTPUT field on a read; every write and list
    /// input spells it `context_ref` — and the shipped MCP prose got this wrong once.
    #[test]
    fn the_mcp_tree_addresses_contexts_by_ref_never_by_name() {
        let files = generate_agent_skill_files().unwrap();
        for (path, body) in &files {
            assert!(
                !body.contains("\"context_name\""),
                "{path} sends `context_name` as an input; the input field is `context_ref`, and a \
                 bare name is rejected"
            );
        }
    }

    /// Both surfaces render, and each speaks its own dialect rather than the other's. A template
    /// whose conditional silently stopped switching would otherwise ship CLI commands to a client
    /// that has no CLI.
    #[test]
    fn shared_templates_render_a_distinct_dialect_per_surface() {
        /// One shared template's renderer, paired with the filename it lands as.
        type SurfaceRenderer = (fn(&str) -> Result<String>, &'static str);

        let renderers: [SurfaceRenderer; 3] = [
            (render_session_lifecycle, "session-lifecycle.md"),
            (render_outcome_registers, "outcome-registers.md"),
            (render_working_a_goal, "working-a-goal.md"),
        ];
        for (render, name) in renderers {
            let cli = render(SURFACE_CLI).unwrap();
            let mcp = render(SURFACE_MCP).unwrap();

            assert_ne!(cli, mcp, "{name} renders identically on both surfaces");
            assert!(
                cli.contains("temper resource"),
                "{name} on the CLI surface names no `temper resource` command"
            );
            assert!(
                !mcp.contains("temper resource"),
                "{name} on the MCP surface names a CLI command; that client has no CLI"
            );
            assert!(
                mcp.contains("Tool:"),
                "{name} on the MCP surface shows no tool call"
            );
            // Both must end with exactly one newline, or a gate diffing renders against committed
            // files reads as permanent, uncloseable drift.
            for (surface, body) in [("cli", &cli), ("mcp", &mcp)] {
                assert!(
                    body.ends_with('\n') && !body.ends_with("\n\n"),
                    "{name} ({surface}) does not end with exactly one newline"
                );
            }
        }
    }

    /// The referencing convention is why documents stop citing unresolvable id prefixes. It has to
    /// reach the MCP surface too — that surface writes the same reference-dense session notes.
    #[test]
    fn the_mcp_skill_carries_the_reference_convention() {
        let files = generate_agent_skill_files().unwrap();
        let skill_md = files.get("SKILL.md").unwrap();
        assert!(
            skill_md.contains("Never abbreviate a UUID"),
            "the MCP SKILL.md must state that UUIDs are never abbreviated"
        );
        assert!(
            skill_md.contains("](./<full-uuid>)"),
            "the MCP SKILL.md must show the [title](./uuid) reference form"
        );
    }

    /// The MCP router must name every file the emit ships, for the same reason the CLI one must:
    /// a supporting file the router never mentions is dead content, and it fails silently.
    #[test]
    fn the_mcp_router_names_every_file_it_ships() {
        let files = generate_agent_skill_files().unwrap();
        let skill_md = files.get("SKILL.md").unwrap();
        for path in files.keys() {
            if path == "SKILL.md" {
                continue;
            }
            assert!(
                skill_md.contains(path.as_str()),
                "the MCP SKILL.md never mentions `{path}`, so no session will read it"
            );
        }
    }

    /// `managed_meta`'s wire vocabulary is `ManagedMeta`, not "every `temper-*` key in the JSON
    /// Schema" — the schema describes frontmatter on disk, which carries identity keys that
    /// `deny_unknown_fields` rejects on the wire. Documenting the schema's set would tell an agent
    /// to send a key that hard-fails.
    #[test]
    fn the_frontmatter_reference_documents_the_wire_vocabulary_not_the_disk_schema() {
        let reference = generate_frontmatter_reference().unwrap();

        // Present: a real Property key from the closed vocabulary.
        assert!(reference.contains("`temper-stage`"));
        // Absent: schema properties that are NOT sendable managed keys.
        for rejected in ["`temper-title`", "`temper-id`", "`temper-context`"] {
            let in_managed_section = reference
                .split("## Server-managed fields")
                .next()
                .unwrap()
                .contains(rejected);
            assert!(
                !in_managed_section,
                "{rejected} is presented as a settable managed key, but the wire type rejects it"
            );
        }
    }

    /// A schema-required key the server defaults is not required *of a caller*. Reporting it as
    /// flatly required sends agents hunting for a value the server supplies.
    #[test]
    fn a_defaulted_required_key_is_reported_as_optional() {
        let reference = generate_frontmatter_reference().unwrap();
        assert!(
            reference.contains("`temper-stage` — optional, defaults to `backlog`"),
            "temper-stage is schema-required AND auto-defaulted; the reference must say so"
        );
    }

    #[test]
    fn test_generate_command_wrapper_contains_hash() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash", false).unwrap();
        let wrapper = &files["command-wrapper.md"];

        assert!(wrapper.contains("config-hash: testhash"));
        assert!(wrapper.contains("Invoke the temper skill"));
    }

    #[test]
    fn test_format_context_list_sorted() {
        let contexts = vec![
            "zebra".to_string(),
            "alpha".to_string(),
            "middle".to_string(),
        ];
        let result = format_context_list(&contexts);
        assert!(result.starts_with("- `alpha`"));
        assert!(result.contains("- `middle`"));
        assert!(result.ends_with("- `zebra`"));
    }

    #[test]
    fn test_format_context_list_empty() {
        let result = format_context_list(&[]);
        assert_eq!(result, "(no contexts configured)");
    }

    #[test]
    fn test_extract_config_hash_found() {
        // The config-hash comment lives AFTER the frontmatter block (see templates/skill.md).
        // Frontmatter parsers in opencode and Claude Code require `---` at byte 0; a leading
        // comment line silently drops the skill from the loader's registry. The extractor scans
        // line-by-line, so it finds the hash wherever it sits.
        let content = "---\nname: temper\ndescription: x\n---\n\n<!-- config-hash: abc123 -->\n";
        assert_eq!(extract_config_hash(content), Some("abc123".to_string()));
    }

    #[test]
    fn test_extract_config_hash_not_found() {
        let content = "---\nname: temper\n---";
        assert_eq!(extract_config_hash(content), None);
    }

    #[test]
    fn test_generate_reference_contains_all_commands() {
        let reference = generate_reference(false);
        assert!(
            reference.contains("| init |"),
            "should contain init command"
        );
        assert!(
            reference.contains("| resource create |"),
            "should contain resource create"
        );
        assert!(
            reference.contains("| resource list |"),
            "should contain resource list"
        );
        assert!(
            reference.contains("| resource show |"),
            "should contain resource show"
        );
        assert!(reference.contains("| warmup |"), "should contain warmup");
        assert!(reference.contains("| search |"), "should contain search");
    }

    #[test]
    fn test_generate_reference_shows_actual_flags() {
        let reference = generate_reference(false);
        // These flags were recently added - they MUST appear
        assert!(
            reference.contains("--stage"),
            "should contain --stage flag from task list"
        );
        assert!(
            reference.contains("--limit"),
            "should contain --limit flag from session list"
        );
    }

    #[test]
    fn test_generate_reference_excludes_hidden_args() {
        let reference = generate_reference(false);
        // --stdin is hidden in clap definitions
        assert!(
            !reference.contains("--stdin"),
            "should NOT contain hidden --stdin flag"
        );
    }

    #[test]
    fn test_generate_reference_excludes_format_flag() {
        let reference = generate_reference(false);
        // --format is an implementation detail, should not appear in syntax column
        assert!(
            !reference.contains("--format"),
            "should NOT contain --format flag in syntax column"
        );
    }

    #[test]
    fn test_generate_reference_has_footer_sections() {
        let reference = generate_reference(false);
        assert!(
            reference.contains("## Task Stages"),
            "should have Task Stages section"
        );
        assert!(reference.contains("## Modes"), "should have Modes section");
        assert!(
            reference.contains("## Skill-Only Commands"),
            "should have Skill-Only Commands"
        );
        assert!(
            reference.contains("## Orientation Projection"),
            "should have Orientation Projection section"
        );
        assert!(
            reference.contains("## Context Requirement"),
            "should have Context Requirement section"
        );
        assert!(
            reference.contains("## Managed vs Open Frontmatter"),
            "should have Managed vs Open Frontmatter section"
        );
        assert!(
            reference.contains("closed, temper-owned vocabulary"),
            "managed-meta section must state the closed-vocabulary contract"
        );
    }

    /// The Resource Types table is DERIVED from `DOC_TYPE_NAMES`, not restated — the hand list
    /// once named 6 of 14 types while the same skill's `memories.md` taught creating a
    /// `memory`-typed resource. This pins the wiring: every real type appears, the
    /// placeholder is actually substituted (an unsubstituted `{{ doc_type_table }}` would
    /// ship as literal text), and no retired type is named.
    #[test]
    fn reference_doc_type_table_is_derived_from_the_validator() {
        let reference = generate_reference(false);
        assert!(
            !reference.contains("{{ doc_type_table }}"),
            "the doc-type placeholder leaked into the rendered reference"
        );
        for doc_type in temper_workflow::schema::DOC_TYPE_NAMES {
            assert!(
                reference.contains(&format!("| {doc_type} |")),
                "the reference's Resource Types table never names `{doc_type}`"
            );
        }
        for retired in ["ticket", "milestone", "epic"] {
            assert!(
                !reference.contains(&format!("| {retired} |")),
                "the reference's Resource Types table names `{retired}`, which is not a temper type"
            );
        }
    }

    #[test]
    fn test_generate_skill_files_uses_generated_reference() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash", false).unwrap();
        let reference = &files["reference.md"];
        // Should contain generated commands, not stale static content
        assert!(
            reference.contains("--stage"),
            "installed reference.md should have --stage"
        );
    }

    // ── Install targets ─────────────────────────────────────────────────────

    /// `agents` is the default `--target`, and its default directory is the vendor-neutral
    /// `~/.agents/skills/temper` — a default is only a default if the flag's absence resolves
    /// to it, so the parse path is exercised rather than assumed.
    #[test]
    fn skill_target_agents_is_the_parsed_default_and_resolves_to_the_neutral_home() {
        use crate::cli::{Cli, Commands, SkillAction};
        use clap::Parser as _;

        let cli = Cli::try_parse_from(["temper", "skill", "install"]).unwrap();
        let Commands::Skill {
            action: SkillAction::Install { target, .. },
        } = cli.command
        else {
            panic!("expected `temper skill install`");
        };
        assert_eq!(target, SkillTarget::Agents);

        let home = Path::new("/home/test");
        assert_eq!(
            target.default_skill_dir(home),
            PathBuf::from("/home/test/.agents/skills/temper"),
            "the agents target must resolve to the vendor-neutral ~/.agents location"
        );
    }

    /// The agents target installs skill content and nothing else — no command wrapper, because
    /// `~/.agents` is a content location and `/temper` routing is an individual agent's concern.
    /// Witnessed through a real install with `HOME` pointed at the tempdir (the wrapper path is
    /// resolved from the process home, like the integration tests do): the claude target writes
    /// the wrapper beside its config home; the agents target writes none.
    #[test]
    fn agents_target_installs_content_and_no_command_wrapper() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let config = test_config();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        // install() hashes the global config (it bakes the hash into SKILL.md), so point
        // TEMPER_GLOBAL_CONFIG at a minimal file, and HOME at the tempdir so the wrapper
        // path resolves inside the test's isolated space.
        let global_config = dir.path().join("global-config.toml");
        std::fs::write(&global_config, "[vault]\npath = \"/tmp/test-vault\"\n").unwrap();
        let env = vec![
            (
                "TEMPER_GLOBAL_CONFIG".to_string(),
                Some(global_config.to_string_lossy().into_owned()),
            ),
            (
                "HOME".to_string(),
                Some(home.to_string_lossy().into_owned()),
            ),
        ];

        temp_env::with_vars(env, || {
            let skill_dir = SkillTarget::Agents.default_skill_dir(&home);
            install(&config, &skill_dir, SkillTarget::Agents, false).unwrap();

            assert!(
                skill_dir.join("SKILL.md").exists(),
                "skill content installed"
            );
            assert!(
                !home.join(".claude/commands/temper.md").exists()
                    && !home.join(".config/opencode/command/temper.md").exists(),
                "the agents target must not write a command wrapper to any agent's config home"
            );

            // The comparison arm: the claude target does write its wrapper.
            let claude_dir = SkillTarget::Claude.default_skill_dir(&home);
            install(&config, &claude_dir, SkillTarget::Claude, false).unwrap();
            assert!(home.join(".claude/commands/temper.md").exists());
        });
    }

    /// **The router is the only file every session loads, so its size is a budget, not a
    /// property.** SKILL.md once carried full sections duplicating `reference.md` and grew to
    /// 22.4KB rendered; the rework moved the detail behind pointers and shipped ~17KB. These
    /// gates sit just above what the rework ships (well below the old size) so the duplication
    /// cannot quietly grow back — a regression here costs every session on every machine.
    #[test]
    fn skill_md_files_stay_under_their_size_budgets() {
        let cli = generate_skill_files_with_hash(&test_config(), "testhash", false).unwrap();
        let cli_skill_md = cli.get("SKILL.md").expect("SKILL.md").len();
        // A router that models read only the top of: ~4.9KB at the 2026-11 slimming (from ~17KB).
        assert!(
            cli_skill_md <= 6_144,
            "the CLI SKILL.md render is {cli_skill_md} bytes; the budget is 6,144. Grow it \
             deliberately, with a reason, never by accretion."
        );

        let mcp = generate_agent_skill_files().unwrap();
        let mcp_skill_md = mcp.get("SKILL.md").expect("SKILL.md").len();
        assert!(
            mcp_skill_md <= 5_120,
            "the MCP SKILL.md is {mcp_skill_md} bytes; the budget is 5,120 (~4.0KB at the \
             2026-11 slimming). Grow it deliberately, with a reason, never by accretion."
        );
    }

    /// **Legacy `guidance/` duplicates are removed only when their bytes match the exact
    /// revisions the old installer shipped.** A file the user has since edited is theirs; a
    /// file the old installer left behind is a stale duplicate that the router's "read every
    /// file in `guidance/`" step feeds to agents as current. Both arms are witnessed here.
    #[test]
    fn install_removes_legacy_guidance_duplicates_only_when_unmodified() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let config = test_config();
        let skill_dir = dir.path().join("skill-output");

        // install() hashes the global config (it bakes the hash into SKILL.md), so point
        // TEMPER_GLOBAL_CONFIG at a minimal file and HOME at the tempdir. Without these the
        // test passes only on machines that happen to have a global config — which is exactly
        // how it reached CI green-local/red-remote.
        let global_config = dir.path().join("global-config.toml");
        std::fs::write(&global_config, "[vault]\npath = \"/tmp/test-vault\"\n").unwrap();
        let env = vec![
            (
                "TEMPER_GLOBAL_CONFIG".to_string(),
                Some(global_config.to_string_lossy().into_owned()),
            ),
            (
                "HOME".to_string(),
                Some(dir.path().to_string_lossy().into_owned()),
            ),
        ];

        let guidance = skill_dir.join("guidance");
        std::fs::create_dir_all(&guidance).unwrap();

        // One file exactly as the old installer shipped it; one user-edited sibling.
        let stale = guidance.join("plan-verification.md");
        std::fs::write(&stale, LEGACY_STALE_BYTES).unwrap();
        let edited = guidance.join("implementation-grounding.md");
        std::fs::write(
            &edited,
            format!("{}\n\nmy local edits\n", LEGACY_STALE_BYTES),
        )
        .unwrap();

        temp_env::with_vars(env, || {
            install(&config, &skill_dir, SkillTarget::Agents, false).unwrap();

            assert!(
                !stale.exists(),
                "the byte-identical installer-era duplicate must be removed"
            );
            assert!(
                edited.exists(),
                "a guidance file the user has modified is theirs — install must not touch it"
            );

            // Idempotent: a second install neither re-creates nor re-reports the removal.
            let second = install(&config, &skill_dir, SkillTarget::Agents, false).unwrap();
            assert!(second.removed_legacy.is_empty());
            assert!(!stale.exists());
        });
    }

    // ── The admin set behind --include-admin ────────────────────────────────

    /// **A default install carries no admin content, and absence is assertable.** No admin file
    /// in the map, no admin row in the generated reference (those rows ARE admin content — a
    /// table naming every admin verb is exactly the disclosure the flag is supposed to be), and
    /// no "admin" in the router at all. Each half is asserted separately because each fails
    /// differently: a stray file is shipping, a stray row is disclosure, a stray mention is a
    /// leaked hint.
    ///
    /// Scope note, deliberate: the BODY scan across every shipped file asserts the absence of
    /// admin *invocations* (`temper admin …`) — the surface this gating owns. Two pre-existing
    /// capability labels in the default tree are NOT admin-set disclosure and are allowed:
    /// `knowledge-base.md`'s declared-off-MCP list names `admin_ledger` (a statement that MCP
    /// does NOT carry it), and `teams.md` names the deployment's administrator role. Reconciled, not
    /// overlooked: widening this to any "admin" substring would put those hand-written files
    /// under a gate this change does not own.
    #[test]
    fn a_default_render_carries_no_admin_content() {
        let files = generate_skill_files_with_hash(&test_config(), "testhash", false).unwrap();

        for name in files.keys() {
            assert!(
                !name.contains("admin"),
                "the default render ships admin file `{name}` — the flag's help text is the \
                 only disclosure, not the tree"
            );
        }
        let skill_md = files.get("SKILL.md").expect("SKILL.md");
        assert!(
            !skill_md.contains("admin"),
            "the default router names admin content; a non-admin install gets no signal the \
             set exists"
        );
        for (name, body) in &files {
            assert!(
                !body.contains("temper admin"),
                "{name} teaches a `temper admin` invocation in a default install — admin \
                 surface teaching is gated behind --include-admin"
            );
        }
        let reference = files.get("reference.md").expect("reference.md");
        assert!(
            !reference.contains("| admin"),
            "the default reference carries admin rows; they are gated behind --include-admin. \
             (The `skill install [--include-admin]` syntax row stays — the flag's own row IS \
             the allowed disclosure.)"
        );
    }

    /// The admin render is the complete tree: the file rides, the router routes to it (both the
    /// Supporting Files entry and the routing-table row — a file named but never routed is the
    /// dead-content failure `every_shipped_guidance_file_is_named_by_the_router` guards for all
    /// files, asserted here at the routing row because that is the arm an admin question takes),
    /// and the reference carries the admin rows again.
    #[test]
    fn the_admin_render_ships_routes_and_restores_the_reference_rows() {
        let files = generate_skill_files_with_hash(&test_config(), "testhash", true).unwrap();

        assert!(
            files.contains_key("admin.md"),
            "the admin render must ship admin.md"
        );
        let skill_md = files.get("SKILL.md").expect("SKILL.md");
        assert!(
            skill_md.contains("admin.md"),
            "the admin router never names admin.md"
        );
        assert!(
            skill_md.contains("| `admin.md` | Administering the deployment"),
            "the admin router has no routing row sending an admin question to admin.md"
        );
        let reference = files.get("reference.md").expect("reference.md");
        for row in [
            "admin machine provision",
            "admin connection",
            "admin reblock",
        ] {
            assert!(
                reference.contains(row),
                "the admin reference never names `{row}` — the clap-derived rows must be \
                 restored under the flag"
            );
        }
    }

    /// **Every verb the admin content names must resolve against the real command tree.**
    ///
    /// Prose cannot be type-checked; its referents can (issue #330's lesson, applied here by
    /// derivation rather than a hand list). Scanned set: every backtick-quoted span starting
    /// with `admin` and built only of command-name characters — table cells and prose alike —
    /// with EVERY segment resolved against the clap tree, not just the first. A renamed or
    /// removed verb at any depth fails here pointing at the sentence that now lies.
    #[test]
    fn admin_content_names_only_verbs_the_binary_has() {
        use clap::CommandFactory;

        for line in ADMIN_MD.lines() {
            let mut rest = line;
            while let Some(start) = rest.find('`') {
                let Some(end) = rest[start + 1..].find('`') else {
                    break;
                };
                let span = &rest[start + 1..start + 1 + end];
                rest = &rest[start + 1 + end + 1..];

                // Command spans only: `admin` alone or a whitespace-separated chain of
                // lowercase name tokens. Anything else that merely starts with "admin"
                // (`admin.md`, a path) is prose, not an invocation, and is skipped.
                let name_token = |s: &str| {
                    !s.is_empty()
                        && s.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                };
                if !(span == "admin"
                    || (span.starts_with("admin ") && span.split_whitespace().all(name_token)))
                {
                    continue;
                }

                let mut cmd = Cli::command();
                let mut resolved: Vec<&str> = Vec::new();
                for segment in span.split_whitespace() {
                    match cmd.find_subcommand(segment) {
                        Some(sub) => {
                            resolved.push(segment);
                            cmd = sub.clone();
                        }
                        // Do NOT break: an unresolved segment at any depth is the defect
                        // (breaking here would verify only the prefix).
                        None => break,
                    }
                }
                assert_eq!(
                    resolved.join(" "),
                    span,
                    "admin.md names `{span}`, which does not fully resolve in the command \
                     tree — the content invented or misnamed a verb"
                );
            }
        }
    }

    /// **The floor is coverage: the content names every admin command group the binary ships.**
    ///
    /// The group set is derived from the clap tree, not restated — a group added to the binary
    /// fails here until the content teaches it, which is the "teach what ships" acceptance
    /// criterion enforced where it can actually fail.
    #[test]
    fn admin_content_names_every_shipped_command_group() {
        use clap::CommandFactory;

        let root = Cli::command();
        let admin = root
            .find_subcommand("admin")
            .expect("the admin group exists");
        for group in admin.get_subcommands() {
            // Hidden commands are not on the dogfooded surface (`--help` omits them), so the
            // content cannot teach what the binary does not show — the same rule the generated
            // reference applies (`collect_command_rows`).
            if group.get_name() == "help" || group.is_hide_set() {
                continue;
            }
            assert!(
                ADMIN_MD.contains(&format!("admin {}", group.get_name())),
                "admin.md never names the shipped admin group `{}` — the content is thinner \
                 than the surface it teaches",
                group.get_name()
            );
        }
    }

    /// A default install's tree must return to a no-admin state even on a machine that once ran
    /// `--include-admin`. Both arms: a byte-identical admin file is retracted by the later
    /// default install; a user-edited one is theirs and stays. Without the first arm the
    /// absence criterion degrades to "absent unless you ever used the flag".
    #[test]
    fn a_default_install_retracts_its_own_admin_files_but_not_edited_ones() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let config = test_config();
        let skill_dir = dir.path().join("skill-output");

        let global_config = dir.path().join("global-config.toml");
        std::fs::write(&global_config, "[vault]\npath = \"/tmp/test-vault\"\n").unwrap();
        let env = vec![
            (
                "TEMPER_GLOBAL_CONFIG".to_string(),
                Some(global_config.to_string_lossy().into_owned()),
            ),
            (
                "HOME".to_string(),
                Some(dir.path().to_string_lossy().into_owned()),
            ),
        ];

        temp_env::with_vars(env, || {
            // Arm 1: flag install, then default install — the shipped bytes go away.
            install(&config, &skill_dir, SkillTarget::Agents, true).unwrap();
            assert!(skill_dir.join("admin.md").exists());
            let report = install(&config, &skill_dir, SkillTarget::Agents, false).unwrap();
            assert!(
                !skill_dir.join("admin.md").exists(),
                "a default install must retract the admin file the installer itself shipped"
            );
            assert_eq!(report.removed_admin, vec!["admin.md".to_string()]);

            // Arm 2: flag install, the user edits admin.md, default install — it stays.
            install(&config, &skill_dir, SkillTarget::Agents, true).unwrap();
            let admin_path = skill_dir.join("admin.md");
            std::fs::write(&admin_path, format!("{}\n\nmy local notes\n", ADMIN_MD)).unwrap();
            let report = install(&config, &skill_dir, SkillTarget::Agents, false).unwrap();
            assert!(
                admin_path.exists(),
                "an admin file the user has modified is theirs — install must not touch it"
            );
            assert!(report.removed_admin.is_empty());
        });
    }

    /// **A fresh default install ships no admin file, and `guidance/` ships empty** — the
    /// fundamentals index is machine-local user data the installer never writes.
    #[test]
    fn a_fresh_default_install_ships_no_admin_file_and_no_guidance_content() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let config = test_config();
        let skill_dir = dir.path().join("skill-output");

        let global_config = dir.path().join("global-config.toml");
        std::fs::write(&global_config, "[vault]\npath = \"/tmp/test-vault\"\n").unwrap();
        let env = vec![
            (
                "TEMPER_GLOBAL_CONFIG".to_string(),
                Some(global_config.to_string_lossy().into_owned()),
            ),
            (
                "HOME".to_string(),
                Some(dir.path().to_string_lossy().into_owned()),
            ),
        ];

        temp_env::with_vars(env, || {
            install(&config, &skill_dir, SkillTarget::Agents, false).unwrap();
            assert!(
                !skill_dir.join("admin.md").exists(),
                "a fresh default install shipped an admin file"
            );
            assert!(
                skill_dir.join("guidance").is_dir(),
                "the guidance directory is created (the router's first hop)"
            );
            let shipped: Vec<_> = std::fs::read_dir(skill_dir.join("guidance"))
                .unwrap()
                .collect();
            assert!(
                shipped.is_empty(),
                "install wrote into guidance/ — per-project fundamentals are user data, not \
                 shipped content"
            );
        });
    }

    /// The fundamentals index must be described where it is used: the router names the index file
    /// and says the repo it points at is the authority. A cold machine cannot resolve a scheme the
    /// router does not describe.
    #[test]
    fn the_router_describes_the_fundamentals_index() {
        let skill_md = generate_skill_files_with_hash(&test_config(), "testhash", false)
            .unwrap()
            .remove("SKILL.md")
            .unwrap();

        for fragment in ["guidance/fundamentals.md", "the repo wins", "/temper init"] {
            assert!(
                skill_md.contains(fragment),
                "the router's fundamentals prose never states `{fragment}`"
            );
        }
    }

    /// A retired file is removed only when its bytes match a revision the installer shipped; an
    /// edited copy is the user's and stays. An emptied `workflows/` directory goes with its files.
    #[test]
    fn retired_files_are_removed_only_when_unedited() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let shipped = "shipped bytes\n";
        let hash = hex::encode(Sha256::digest(shipped.as_bytes()));
        let table = [
            ("workflows/build-small.md", hash.as_str()),
            ("subagent-guidance.md", hash.as_str()),
        ];
        std::fs::create_dir_all(dir.path().join("workflows")).unwrap();
        std::fs::write(dir.path().join("workflows/build-small.md"), shipped).unwrap();
        std::fs::write(
            dir.path().join("subagent-guidance.md"),
            "edited by its owner\n",
        )
        .unwrap();

        let removed = remove_retired_files(dir.path(), &table).unwrap();

        assert_eq!(removed, vec!["workflows/build-small.md".to_string()]);
        assert!(
            !dir.path().join("workflows").exists(),
            "an emptied workflows/ stays behind"
        );
        assert!(
            dir.path().join("subagent-guidance.md").exists(),
            "an edited retired file was deleted"
        );
    }

    /// Every hash in the retired table is a well-formed SHA-256, and no path repeats a hash: a typo
    /// here would silently stop recognizing a shipped revision.
    #[test]
    fn the_retired_table_is_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for (path, hash) in RETIRED_SKILL_FILES {
            assert_eq!(hash.len(), 64, "{path}: {hash} is not a SHA-256");
            assert!(
                hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "{path}: {hash}"
            );
            assert!(seen.insert((path, hash)), "{path} lists {hash} twice");
        }
        for hash in ADMIN_SHIPPED_REVISIONS {
            assert_eq!(hash.len(), 64, "admin.md: {hash} is not a SHA-256");
            assert!(
                hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "admin.md: {hash}"
            );
        }
    }

    /// `--include-admin` defaults to false at the parse layer: the default install is the
    /// non-admin one, so a default that failed open would ship admin content to everyone.
    #[test]
    fn include_admin_defaults_to_false() {
        use crate::cli::{Cli, Commands, SkillAction};
        use clap::Parser as _;

        let cli = Cli::try_parse_from(["temper", "skill", "install"]).unwrap();
        let Commands::Skill {
            action: SkillAction::Install { include_admin, .. },
        } = cli.command
        else {
            panic!("expected `temper skill install`");
        };
        assert!(
            !include_admin,
            "the default install must not include the admin set"
        );
    }

    /// The exact bytes the old installer wrote into `guidance/` for `plan-verification.md` —
    /// the revision this machine carried, hashed in `LEGACY_GUIDANCE_DUPLICATES`. Embedded so
    /// the test witnesses the removal against the same bytes the cleanup matches, with no
    /// round-trip through the user's home directory.
    const LEGACY_STALE_BYTES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/skill-content/legacy-guidance/plan-verification.md"
    ));
}
