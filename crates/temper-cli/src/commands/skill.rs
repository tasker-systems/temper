use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use askama::Template;
use clap::CommandFactory;
use sha2::{Digest, Sha256};

use crate::cli::Cli;
use crate::config::{self, Config};
use crate::error::{Result, TemperError};
use crate::output;
use crate::templates::{
    CommandWrapperTemplate, OutcomeRegistersTemplate, SessionLifecycleTemplate, SkillTemplate,
};

// ── Surfaces ─────────────────────────────────────────────────────────────────

/// The two packagings of the same discipline. `cli` ships to `~/.claude/skills/temper/` and speaks
/// `temper …` commands; `mcp` is the committed `agent-skills/` projection and speaks tool calls.
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
/// compares the render against a file on disk — which is exactly what the `agent-skills` drift gate
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

// ── Static content (compiled into the binary) ────────────────────────────────

/// The MCP skill's router. A plain file, not a template, and deliberately so: it has no CLI twin
/// to share prose with (the CLI router bakes a per-user context list, which is exactly why that one
/// can never be a committed projection) and no values to interpolate. Templating it would buy a
/// seam with nothing on either side of it.
static MCP_SKILL_MD: &str = include_str!("../../skill-content/mcp/SKILL.md");
static SUBAGENT_GUIDANCE_MD: &str = include_str!("../../skill-content/subagent-guidance.md");
static PLAN_VERIFICATION_MD: &str = include_str!("../../skill-content/plan-verification.md");
static IMPLEMENTATION_GROUNDING_MD: &str =
    include_str!("../../skill-content/implementation-grounding.md");
static COGNITIVE_MAPS_MD: &str = include_str!("../../skill-content/cognitive-maps.md");
static TEAMS_MD: &str = include_str!("../../skill-content/teams.md");
static KNOWLEDGE_BASE_MD: &str = include_str!("../../../../agent-skills/knowledge-base.md");
static WF_BUILD_SMALL: &str = include_str!("../../skill-content/workflows/build-small.md");
static WF_BUILD_MEDIUM: &str = include_str!("../../skill-content/workflows/build-medium.md");
static WF_BUILD_LARGE: &str = include_str!("../../skill-content/workflows/build-large.md");
static WF_PLAN_SMALL: &str = include_str!("../../skill-content/workflows/plan-small.md");
static WF_PLAN_MEDIUM: &str = include_str!("../../skill-content/workflows/plan-medium.md");
static WF_PLAN_LARGE: &str = include_str!("../../skill-content/workflows/plan-large.md");

static REFERENCE_FOOTER: &str = r#"
## Resource Types

| Type | Description |
|------|-------------|
| task | Work items with stage, mode, effort tracking |
| goal | High-level objectives that group tasks |
| session | Timestamped work session notes |
| research | Research notes and findings |
| concept | Named ideas, patterns, or domain terms |
| decision | Point-in-time choices with rationale (ADR-like) |

## Task Stages

| Stage | Meaning |
|-------|---------|
| backlog | Not yet started |
| in-progress | Actively being worked |
| done | Completed |
| cancelled | Abandoned or no longer relevant |

## Modes

| Mode | Purpose |
|------|---------|
| plan | Research, design, discovery -- understanding before building |
| build | Implementation -- producing artifacts |

## Effort Levels

| Effort | Scope |
|--------|-------|
| small | Single session, focused deliverable |
| medium | Multi-step, bounded to a clear outcome |
| large | Multi-session, may require decomposition |

## Managed vs Open Frontmatter

`managed_meta` is a **closed, temper-owned vocabulary** -- the `temper-*`
workflow/provenance keys (stage, mode, effort, status, seq, branch, pr,
llm-model, llm-run, provenance). It is optional metadata with smart defaults;
you never *have* to send it. An unknown key under `managed_meta` is rejected --
put caller-defined ("bring-your-own") fields in `open_meta`, the free-form tier.

**Slug:** the slug is always derived from the title -- it is not a caller
input on any surface. Addressing is trailing-UUID-only (a bare UUID or the
decorated `sluggify(title)-<uuid>` ref), so the slug is display decoration
only; there is no CLI `--slug` flag and the MCP/API request schemas no longer
carry a `slug` field. A slug placed in managed frontmatter is likewise inert.

## Discovery Workflow

1. `temper search "<topic>"` -- find relevant documents and notes
2. `temper context [<name>]` -- understand current context and recent activity
3. Use search results to guide targeted file reads
4. Reach for `--meta-only` / `--fields` on `resource show` and `resource list`
   when you need cheap orientation rather than full bodies (see below)

Search first, read second. Don't guess at file paths.

## Listing: truncation, sort, and filters

`temper resource list` returns a **capped page** — 20 rows by default (50 with
`--meta-only`). **Never conclude a resource is absent, or that a set is
complete, from a default `list`.** A goal/task/session you "don't see" may just
be past the cap.

Every list response carries the signal to catch this:

| Envelope key | Meaning |
|--------------|---------|
| `total` | Count of ALL rows matching the filters (before the page cap). |
| `returned` | Rows in THIS page. |
| `truncated` | `true` when `total` exceeds what this page shows — there is more. |

When `truncated` is `true`, the CLI also prints a stderr hint. Before asserting
absence or completeness, **expand or narrow**:

| Flag | Use |
|------|-----|
| `--all` | Return every matching row (no cap). Reach for this before claiming a set is complete. Conflicts with `--limit`. |
| `--limit <n>` / `--offset <n>` | A bigger page, or page through the set. |
| `--sort <field>[:asc\|desc]` | Order by `updated`, `created`, `title`, `stage`, `seq`, `context`, or `doctype`. Direction defaults per field (time/seq → desc, text → asc); default sort is `updated:desc`. |
| `--title-contains <substr>` | Case-insensitive title filter — narrow instead of paging blind (for topical/semantic search use `temper search`). |
| `--stage <s>` / `--goal <ref>` (task) · `--status <s>` (goal) | Type-specific filters. Naming a `--type` they don't apply to is an error, not a silent no-op. |
| `--tag <t>` (repeatable) | Filter by tag. Repeating **narrows** — `--tag ci --tag security` returns only resources carrying both. Exact match, case-insensitive. Not type-specific: tags span every doc type. |
| `--type <t>` | Optional. Omit it to list across every doc type — which is how you enumerate a whole tag axis in one call instead of once per type. |

```bash
# WRONG: "no maintenance goal exists" — read off a 20-row default page.
temper resource list --type goal --context @me/temper

# RIGHT: narrow, or enumerate fully, before asserting absence.
temper resource list --type goal --context @me/temper --title-contains maintenance
temper resource list --type goal --context @me/temper --all

# Enumerate a program axis across every doc type — no --type.
temper resource list --tag ci --all
temper resource list --tag ci --tag security --all   # carries BOTH
```

## Orientation Projection (cheap reads)

The read-side projection flags let you peek at structure without paying for
full bodies — useful when triaging a context, comparing a few resources, or
deciding whether to read more deeply.

| Pattern | What it returns |
|---------|-----------------|
| `temper resource show <ref> --meta-only` | The full `show` view — the row (title, type, context, owner, stage/seq/mode/effort) plus both the managed and open meta tiers — minus the reconstructed body. |
| `temper resource list --type <t> --context @me/<ctx> --meta-only` | Each row as a full payload **plus** both meta tiers (no bodies). The default `list` carries neither tier, so this triages a whole context on managed/open-meta fields in one call. |
| `--fields <a,b,c>` on either of the above | Subselects top-level response keys. The anchor key `id` is always preserved. Pipe through `jq` for nested projection. |
| `temper resource show <ref> --edges` | Adds the graph edges connected to this resource. Mutually exclusive with `--meta-only`. |

## Referencing Other Resources — full UUIDv7, and link it

**A UUID is not a SHA. Never abbreviate one.** A UUIDv7's leading bits are a
**timestamp**, so resources created near each other share a prefix *by
construction* — a goal and the task written a minute later routinely agree on
their first seven characters:

```
019fbb77-72a3-72e1-bbbd-13eb6aa64982   <- a goal
019fbb78-657b-7380-9063-212727cfe390   <- its task, 62 seconds later
```

A prefix is therefore *systematically* ambiguous between exactly the resources
most likely to be cited together, and resolves to nothing a reader can follow.
Write the full 36 characters everywhere — prose, tables, `open_meta`, commits.

**When a document refers to another resource, write it as a markdown link:**

```markdown
[<the resource's exact title>](./<full-uuidv7>)
```

Resources are addressed **flatly**; there is no directory tree to be relative
to, so `./<uuid>` is the entire path and it resolves wherever the body renders.
The reader then sees *what* is cited without a round-trip, and can navigate to
it instead of copying an id into `resource show`.

Take the title from `resource list`/`show`, not from memory — an approximate
title inside a link is a citation that looks precise and is not. Escape any
`[`, `]`, `(` or `)` the title contains, or the link will not render.

To delete a resource server-side, run `temper resource delete <ref> [--force]`
(the `<ref>` is the resource's `ref` field from `list`/`show`).

## Context Requirement

| Verb | `--context` required? |
|------|----------------------|
| `resource list` | optional (omitting lists across all contexts) |
| `resource show` | **required** |
| `resource create` | **required** |
| `resource update` | **required** |
| `resource delete` | **required** |

Omitting `--context` where it is required surfaces the error
`Project error: no context specified — use --context <ref> (e.g. @me/temper, +team/general, or a UUID)`.

A context is addressed by **ref**: `@me/<slug>` for your own contexts, `+team-slug/<slug>` for
team contexts, or a bare UUID. Bare context names are **not accepted**.

## Template Access

Use `--show-template` on `resource create` to display the expected frontmatter and body
structure without creating anything:

```bash
temper resource create --type session --show-template
temper resource create --type task --show-template
temper resource create --type research --show-template
```

## Body Source (`resource create` / `resource update`)

Where the markdown body comes from on a write is resolved by a **first-match-wins
precedence**. Get this wrong and you silently clobber a body — so it is spelled
out here.

| Precedence | Form | Behavior |
|-----------|------|----------|
| 1 | `--body @<path>` | Read the file. **stdin is ignored entirely.** Empty file → error (refuses to write an empty body). |
| 2 | `--body -` | Read stdin explicitly. Blocks until EOF. Errors on a TTY or on empty input. |
| 3 | implicit (no `--body`) | Read stdin **only** when it is a non-TTY that has input *ready* and non-empty — the `cat new.md \| temper resource update <ref>` case. An open-but-idle pipe, or empty stdin, is treated as **no body**. |
| 4 | none of the above | Body is left unchanged; on `update`, only frontmatter is PATCHed. |

> **Footgun — implicit stdin is a body rewrite.** `resource update` treats an
> implicit non-TTY stdin as a full-body replace. Two ways this bites an agent:
>
> - **Redirected loop inheritance.** Inside
>   `while read n ref; do temper resource update "$ref" --title "…"; done < refs.txt`,
>   every `update` inherits the loop's redirected stdin (`refs.txt`) and rewrites
>   the resource body with the *leftover loop lines* — clobbering one resource and
>   consuming the remaining iterations, with no error.
> - **`< /dev/null` "safety".** Redirecting from `/dev/null` used to set the body
>   to empty. (Current CLI treats empty implicit stdin as *no body* — but relying
>   on that is fragile; older installed binaries still clobber.)
>
> **Rules of thumb:**
> - To change **frontmatter only** (e.g. `--title`, `--stage`), invoke `update`
>   **one resource per call with stdin untouched** — never inside a
>   `while read … done < file` loop.
> - To rewrite a **body**, do it with an explicit, intended
>   `cat file.md | temper resource update <ref>` (or `--body @file.md`).
> - `--body @<path>` always wins over stdin — reach for it when you want to be
>   unambiguous.

## Block-Grain Ingest & Attribution

A resource body is not always one opaque blob. The ingest surface lets you build
it as an **enumerated sequence of distinctly addressable, individually
attributable content blocks**, and attach **per-block provenance** (which
sources each block was distilled from) and **per-act authorship** (confidence,
reasoning, the model that wrote it). Reach for this when constructing a large or
citation-graded document instead of defaulting to a whole-body `create`/`update`.

**Boundary — what this is NOT.** These verbs *build* and *attribute* blocks; they
do **not** surgically rewrite one existing block's text in a finalized resource.
An in-place text edit is still a whole-body `resource update`. `annotate` is
**provenance-only** — it never touches block text, `body_hash`, or embeddings.

### Per-block provenance & authorship (on any write)

`resource create` and `resource update` both take:

- `--sources <a,b,…>` — comma-separated **resource refs** (UUID or decorated
  `slug-<uuid>`) and/or **http(s) URLs**. Each becomes a block-provenance record
  on the body block. A URL may carry a `#L<start>-L<end>` span locator (e.g.
  `https://ex.com/doc.md#L120-L180`) — preserved verbatim end-to-end and surfaced
  in provenance reads. On `update`, `--sources` requires a body update (there is
  no block to attribute otherwise).
- `--content-block <block-uuid>` — which block the body revise + `--sources`
  target. Omit to address the resource's sole body block; **required** once a
  resource has more than one block.
- Authorship/correlation flags (`--confidence <tentative|probable|confident>`,
  `--reasoning`, `--rationale`, `--persona`, `--model`, `--invocation`,
  `--correlation`). `--confidence` is **required** whenever any other authorship
  flag is supplied.

`resource create` also has `--from <url|path>` (extract markdown from a source,
which by default seeds a Remote provenance record — `--no-source` opts out) and
`--sources-as-edges` (also assert a `derived_from` edge to each resource-valued
source).

### `resource annotate <ref> --sources … [--content-block <uuid>]`

The **annotate-only backfill**: attach provenance sources to a block **without a
body revise**. `body_hash` and embeddings are unchanged, so a corpus imported
without sources can be made citation-grade cheaply. At least one `--sources`
entry is required; `--content-block` is required for a multi-block resource.
Verify with `resource show --provenance`.

### Reading provenance

- `temper resource show <ref> --provenance` — itemized per-block provenance: the
  sources each of the resource's content blocks was distilled from, in
  (block, accretion) order.

### Segmented ingest lifecycle (large / resumable builds)

For a body too large for one call, the CLI streams it automatically (the
`resource create` path segments internally past ~256 KB and resumes an
interrupted upload). The **MCP** surface exposes the lifecycle explicitly for
programmatic callers:

| Tool | Role |
|------|------|
| `ingest_begin` | Lands segment 0, creates the resource, returns `resource_id` + landed block set + an opaque `body_hash`. Takes every `create_resource` field plus `content_hash`. |
| `ingest_append` | Lands segment N (`seq` starts at **1**, in order). **Idempotent** — a re-append of an already-landed seq is a safe no-op, so retry/resume is safe. Optional per-segment `sources`. |
| `ingest_blocks` | Reads the landed segment set back — how a stateless caller resumes after an interruption. |
| `ingest_finalize` | Declares the session complete: pass `expected_blocks` and **echo the `body_hash` verbatim** (opaque — never parse or recompute it). Fails loudly on a gap. |

See `knowledge-base.md` for the MCP tool details.

## Skill-Only Commands

These commands are handled by the skill routing layer, not the temper CLI directly.
They compose multiple CLI commands into guided workflows.

| Skill Command | What It Does |
|---------------|-------------|
| `task start <slug>` | Shows task, moves to in-progress, routes to workflow |
| `task resume <slug>` | Shows task, reads last session, continues workflow |
| `task create` | Guided interactive task creation with prompts |
| `session start` | Start a session without a predefined task |
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
        "These {} are the complete set. A `doc_type_name` outside it is rejected.\n\n",
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
pub fn generate_reference() -> String {
    let cmd = Cli::command();
    let mut rows = Vec::new();
    collect_command_rows(&cmd, "", &mut rows);

    let mut out = String::new();
    out.push_str("# CLI Reference\n\n");
    out.push_str("## Invocation\n\n");
    out.push_str("**Always run `temper` directly from PATH.** Never use `cargo run -p temper-cli`, `python`,\n");
    out.push_str("full paths, or any indirect invocation method — even when working inside the temper source\n");
    out.push_str(
        "repository. The installed binary may differ from the in-development code, and that is\n",
    );
    out.push_str("intentional: we use the installed CLI to manage our own workflow while evolving the crate.\n\n");
    out.push_str("**Before running any temper command**, verify the binary exists:\n");
    out.push_str("```bash\nwhich temper\n```\n");
    out.push_str("If `temper` is not on PATH, **stop and warn the user**:\n");
    out.push_str("> \"The `temper` binary is not installed or not on PATH. Install it with\n");
    out.push_str("> `cargo install --path crates/temper-cli` or ensure `~/.cargo/bin` is in your PATH.\"\n\n");
    out.push_str("Do not fall back to `cargo run` as a workaround.\n\n");
    out.push_str("## Commands\n\n");
    out.push_str("| Command | Syntax |\n");
    out.push_str("|---------|--------|\n");
    for (name, syntax) in &rows {
        out.push_str(&format!("| {} | `{}` |\n", name, syntax));
    }
    out.push_str("\nPipe content via stdin for `resource create` (all types accept stdin body).\n");
    out.push_str(REFERENCE_FOOTER);
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

/// Generate all skill files as a map of relative_path → content.
pub fn generate_skill_files(config: &Config) -> Result<HashMap<String, String>> {
    let hash = compute_config_hash()?;
    generate_skill_files_with_hash(config, &hash)
}

/// Returns the generated reference.md content for stdout preview.
pub fn generate(_config: &Config) -> Result<String> {
    Ok(generate_reference())
}

// ── The MCP surface: a committed projection ──────────────────────────────────

/// Generate the `agent-skills/` tree as a map of relative_path → content.
///
/// **Config-free, and that is the whole reason this tree can be committed.** The CLI skill bakes a
/// per-user context list into its router, so its rendered form is one developer's and could never be
/// checked in; the MCP skill has no such input, so its render is a pure function of this source tree
/// and a drift gate can pin it the way `openapi.json` and the ts-rs trees are pinned.
///
/// Deliberately **not** every file in `agent-skills/`. `knowledge-base.md` and `claude-desktop.md`
/// are hand-written and stay that way — the first is the MCP tool reference and has no CLI twin to
/// share with, the second is a setup guide. Emitting over them would be a rewrite, and the gate
/// only ever compares what appears here.
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
    // Shipped to both surfaces verbatim: these three name no command on either, so they are the
    // same bytes in both trees rather than two renders of one template.
    files.insert(
        "subagent-guidance.md".to_string(),
        SUBAGENT_GUIDANCE_MD.to_string(),
    );
    files.insert(
        "plan-verification.md".to_string(),
        PLAN_VERIFICATION_MD.to_string(),
    );
    files.insert(
        "implementation-grounding.md".to_string(),
        IMPLEMENTATION_GROUNDING_MD.to_string(),
    );

    Ok(files)
}

/// Write the generated `agent-skills/` tree into `dir`, returning the relative paths written.
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
}

impl InstallReport {
    pub fn is_no_op(&self) -> bool {
        self.changed.is_empty()
    }
}

/// Install skill directory and command wrapper.
///
/// 1. Generate all skill files
/// 2. Write skill files (except command-wrapper.md) into `skill_dir`
/// 3. Write command-wrapper.md to `~/.claude/commands/temper.md`
///
/// Skips writes when the destination already matches the generated content,
/// returning an `InstallReport` that lists every file whose bytes changed.
pub fn install(config: &Config, skill_dir: &Path) -> Result<InstallReport> {
    let files = generate_skill_files(config)?;
    let mut report = InstallReport {
        total: files.len(),
        changed: Vec::new(),
    };

    // Ensure skill_dir and subdirectories exist
    for sub in &["workflows", "guidance"] {
        let dir = skill_dir.join(sub);
        std::fs::create_dir_all(&dir).map_err(|e| {
            TemperError::Config(format!("cannot create directory {}: {}", dir.display(), e))
        })?;
    }

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

    // Write command-wrapper.md to ~/.claude/commands/temper.md
    if let Some(wrapper_content) = files.get("command-wrapper.md") {
        let home = dirs::home_dir()
            .ok_or_else(|| TemperError::Config("cannot determine home directory".to_string()))?;
        let commands_dir = home.join(".claude/commands");
        std::fs::create_dir_all(&commands_dir).map_err(|e| {
            TemperError::Config(format!("cannot create {}: {}", commands_dir.display(), e))
        })?;
        let wrapper_path = commands_dir.join("temper.md");
        if write_if_changed(&wrapper_path, wrapper_content)? {
            report.changed.push("command-wrapper.md".to_string());
        }
    }

    Ok(report)
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
pub fn check(config: &Config) -> Result<()> {
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
    check_command_wrapper();

    Ok(())
}

/// Report which of the expected skill files are present under `skill_dir`.
fn check_expected_files(skill_dir: &Path) {
    let expected_files = [
        "SKILL.md",
        "reference.md",
        "subagent-guidance.md",
        "plan-verification.md",
        "implementation-grounding.md",
        "outcome-registers.md",
        "session-lifecycle.md",
        "cognitive-maps.md",
        "teams.md",
        "knowledge-base.md",
        "workflows/build-small.md",
        "workflows/build-medium.md",
        "workflows/build-large.md",
        "workflows/plan-small.md",
        "workflows/plan-medium.md",
        "workflows/plan-large.md",
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

/// Check the command wrapper at `~/.claude/commands/temper.md`.
fn check_command_wrapper() {
    let wrapper_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~"))
        .join(".claude/commands/temper.md");

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

/// Generate all skill files with a pre-computed config hash (for testability).
pub fn generate_skill_files_with_hash(
    config: &Config,
    hash: &str,
) -> Result<HashMap<String, String>> {
    let context_list = format_context_list(&config.contexts);

    let skill_template = SkillTemplate {
        config_hash: hash,
        context_list: &context_list,
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

    files.insert("reference.md".to_string(), generate_reference());
    files.insert(
        "subagent-guidance.md".to_string(),
        SUBAGENT_GUIDANCE_MD.to_string(),
    );
    // The grounding pair. Shipped at the skill root beside `subagent-guidance.md` — NOT into
    // `guidance/`, which is the user's namespace (install only creates it; it writes nothing
    // there, and shipping into it would clobber user files on every regen).
    files.insert(
        "plan-verification.md".to_string(),
        PLAN_VERIFICATION_MD.to_string(),
    );
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
        "session-lifecycle.md".to_string(),
        render_session_lifecycle(SURFACE_CLI)?,
    );
    files.insert(
        "cognitive-maps.md".to_string(),
        COGNITIVE_MAPS_MD.to_string(),
    );
    files.insert("teams.md".to_string(), TEAMS_MD.to_string());

    files.insert(
        "knowledge-base.md".to_string(),
        KNOWLEDGE_BASE_MD.to_string(),
    );

    files.insert(
        "workflows/build-small.md".to_string(),
        WF_BUILD_SMALL.to_string(),
    );
    files.insert(
        "workflows/build-medium.md".to_string(),
        WF_BUILD_MEDIUM.to_string(),
    );
    files.insert(
        "workflows/build-large.md".to_string(),
        WF_BUILD_LARGE.to_string(),
    );
    files.insert(
        "workflows/plan-small.md".to_string(),
        WF_PLAN_SMALL.to_string(),
    );
    files.insert(
        "workflows/plan-medium.md".to_string(),
        WF_PLAN_MEDIUM.to_string(),
    );
    files.insert(
        "workflows/plan-large.md".to_string(),
        WF_PLAN_LARGE.to_string(),
    );

    Ok(files)
}

/// Compute SHA256 hash of the global config file.
fn compute_config_hash() -> Result<String> {
    let config_path = config::global_config_path();
    let config_content = std::fs::read_to_string(&config_path)
        .map_err(|e| TemperError::Config(format!("cannot read config: {}", e)))?;
    Ok(format!("{:x}", Sha256::digest(config_content.as_bytes())))
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
            subscriptions: Vec::new(),
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
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
        let skill_md = files.get("SKILL.md").expect("SKILL.md");

        // Being named is the bar, not being named in a *numbered step*: `outcome-registers.md` is
        // reached on demand from the routing table (plus a one-line stanza in the always-read
        // steps), which is deliberate — it is ~200 lines that only a goal-authoring session needs.
        // Unreachable is the failure this pins; expensive-on-every-task is a separate judgment.
        for guidance in [
            "subagent-guidance.md",
            "plan-verification.md",
            "implementation-grounding.md",
            "outcome-registers.md",
        ] {
            assert!(
                skill_md.contains(guidance),
                "SKILL.md never mentions `{guidance}`, so no session will read it — ship it in the \
                 router's steps or do not ship it at all"
            );
        }
    }

    #[test]
    fn test_generate_skill_files_contains_expected_keys() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();

        assert!(files.contains_key("SKILL.md"));
        assert!(files.contains_key("reference.md"));
        assert!(files.contains_key("subagent-guidance.md"));
        assert!(files.contains_key("plan-verification.md"));
        assert!(files.contains_key("implementation-grounding.md"));
        assert!(files.contains_key("outcome-registers.md"));
        assert!(files.contains_key("session-lifecycle.md"));
        assert!(files.contains_key("cognitive-maps.md"));
        assert!(files.contains_key("teams.md"));
        assert!(files.contains_key("knowledge-base.md"));
        assert!(files.contains_key("workflows/build-small.md"));
        assert!(files.contains_key("workflows/build-medium.md"));
        assert!(files.contains_key("workflows/build-large.md"));
        assert!(files.contains_key("workflows/plan-small.md"));
        assert!(files.contains_key("workflows/plan-medium.md"));
        assert!(files.contains_key("workflows/plan-large.md"));
        assert!(files.contains_key("command-wrapper.md"));
    }

    #[test]
    fn test_generate_skill_md_has_contexts_and_no_local_vault_path() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
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

    /// The referencing convention is the reason sessions stop writing unresolvable id prefixes.
    /// It lives in two places — SKILL.md (read every session) and reference.md — and a silent
    /// drop from either is invisible until documents start citing nothing again.
    #[test]
    fn test_reference_convention_is_stated_in_skill_and_reference() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();

        for name in ["SKILL.md", "reference.md"] {
            let body = files.get(name).unwrap_or_else(|| panic!("{name} missing"));
            assert!(
                body.contains("A UUID is not a SHA"),
                "{name} must state that UUIDs are never abbreviated"
            );
            assert!(
                body.contains("](./<full-uuidv7>)"),
                "{name} must show the [title](./uuid) reference form"
            );
        }
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
                "implementation-grounding.md",
                "outcome-registers.md",
                "plan-verification.md",
                "references/frontmatter.md",
                "session-lifecycle.md",
                "subagent-guidance.md",
            ],
            "the emitted set moved — the drift gate only ever compares what appears here, so a file \
             dropped from this map silently stops being checked"
        );
        // Hand-written siblings are NOT emitted. Asserted as an ABSENCE because the failure mode is
        // someone folding them in, which would overwrite maintained prose on every regeneration.
        for hand_written in ["knowledge-base.md", "claude-desktop.md"] {
            assert!(
                !files.contains_key(hand_written),
                "{hand_written} is hand-maintained; emitting it would clobber it"
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

        let renderers: [SurfaceRenderer; 2] = [
            (render_session_lifecycle, "session-lifecycle.md"),
            (render_outcome_registers, "outcome-registers.md"),
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
            skill_md.contains("A UUID is not a SHA"),
            "the MCP SKILL.md must state that UUIDs are never abbreviated"
        );
        assert!(
            skill_md.contains("](./<full-uuidv7>)"),
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
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
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
        let content = "<!-- config-hash: abc123 -->\n---\nname: temper\n---";
        assert_eq!(extract_config_hash(content), Some("abc123".to_string()));
    }

    #[test]
    fn test_extract_config_hash_not_found() {
        let content = "---\nname: temper\n---";
        assert_eq!(extract_config_hash(content), None);
    }

    #[test]
    fn test_generate_reference_contains_all_commands() {
        let reference = generate_reference();
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
        let reference = generate_reference();
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
        let reference = generate_reference();
        // --stdin is hidden in clap definitions
        assert!(
            !reference.contains("--stdin"),
            "should NOT contain hidden --stdin flag"
        );
    }

    #[test]
    fn test_generate_reference_excludes_format_flag() {
        let reference = generate_reference();
        // --format is an implementation detail, should not appear in syntax column
        assert!(
            !reference.contains("--format"),
            "should NOT contain --format flag in syntax column"
        );
    }

    #[test]
    fn test_generate_reference_has_footer_sections() {
        let reference = generate_reference();
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

    #[test]
    fn test_generate_skill_files_uses_generated_reference() {
        let config = test_config();
        let files = generate_skill_files_with_hash(&config, "testhash").unwrap();
        let reference = &files["reference.md"];
        // Should contain generated commands, not stale static content
        assert!(
            reference.contains("--stage"),
            "installed reference.md should have --stage"
        );
    }
}
