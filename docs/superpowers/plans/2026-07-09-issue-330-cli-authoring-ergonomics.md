# Issue #330 — CLI Authoring Ergonomics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a scripted cognitive-map authoring pass work end-to-end through the CLI, with a JSON contract a naive parser can consume, a managed-metadata tier that actually carries provenance, and agent-facing docs that tell the truth.

**Architecture:** Three sequential PRs, each one story. (a) fixes the CLI's JSON output contract by making multi-document output structurally impossible. (b) makes the managed tier real: the server stamps the provenance trio from the act envelope on every create, and `resource show` returns both metadata tiers via a new `ResourceDetail` wire type. (c) adds the one genuinely missing verb, an opt-in edge-assert flag, a derived `disposition`, and corrects the three false statements in the agent-facing skill content that caused this issue.

**Tech Stack:** Rust workspace (temper-cli, temper-core, temper-workflow, temper-services, temper-api, temper-client, temper-mcp), clap, serde, sqlx, PostgreSQL 18 + pgvector, cargo-make, cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-07-09-issue-330-cli-authoring-ergonomics-design.md`

## Global Constraints

- **Never merge PRs, never run production migrations.** Stop at "PR up + CI green + summary." Pete reviews and merges.
- **Typed structs over inline JSON.** Never `serde_json::json!()` for data with a known structure.
- **Persistence is its own layer.** Never inline `sqlx::query!()` in a surface. Surfaces dispatch through `DbBackend` for writes; reads stay service-direct.
- **Auth before writes.** Authorization checks precede any mutation.
- **No premature backward compat.** This is a young project — remove dead code rather than keep it "for compat."
- **Local testing stays targeted.** Run the adjacent tests named in each task, plus `cargo make check`. Do **not** run `cargo make test-all` locally. CI carries regression coverage.
- **Before any e2e run:** `cargo build -p temper-cli --bin temper` (test-e2e does not rebuild the binary). Never run two e2e suites concurrently.
- **`cargo make check` forces `SQLX_OFFLINE=true`** — it is the honest local probe of the committed `.sqlx/` caches.
- **Branch naming:** `jct/<scope>`. Commit prefixes: `fix(<scope>):` / `refactor(<scope>):` / `feat(<scope>):` / `docs(<scope>):` / `test:`.
- **Every commit ends with:** `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

## Verified Ground Truth

These were checked against the code. **Trust these over the spec's sketches** — the spec contains two signatures that will not compile.

| Fact | Location |
|------|----------|
| `ActContext { invocation: Option<InvocationId>, authorship: Option<AgentAuthorship> }` — **the model is nested**, there is no `act.model` | `temper-core/src/types/authorship.rs:78` |
| `AgentAuthorship { reasoning, confidence: ConfidenceBand, rationale, persona, model: Option<String> }` | `temper-core/src/types/authorship.rs:56` |
| `CreateResource.managed_meta` is a plain `ManagedMeta`, **not** `Option<ManagedMeta>` — `.unwrap_or_default()` will not compile | `temper-workflow/src/operations/commands.rs:27` |
| `base.schema.json` already declares `"temper-provenance": { "enum": ["llm-discovered", "user-created"] }` | `temper-workflow/schemas/base.schema.json:108-111` |
| Every doc-type schema sets `additionalProperties: true`; the strip lists (`IDENTITY_FIELDS`, `TIER1_SYSTEM_FIELDS`) do not contain the trio | `temper-workflow/src/frontmatter/fields.rs:9,14` |
| `ProvenanceSource` has **three** variants: `Event(Uuid)`, `Resource(Uuid)`, `Remote(String)` | `temper-core/src/types/provenance.rs:35` |
| `InvocationCloseAck` **already carries** `disposition: Disposition`. Only `InvocationView` lacks it. | `temper-core/src/types/invocation_requests.rs:51` |
| `Backend::show_resource` returns `CommandOutput<ResourceRow>`; **three** impls exist (DbBackend + two cfg'd CloudBackend) | `temper-workflow/src/operations/backend.rs:60`, `temper-services/.../db_backend.rs:1235`, `temper-cli/src/cloud_backend/backend.rs:145,513` |
| MCP `get_resource` already composes `show_select` + `get_meta_select` — the composition Task 7 formalizes | `temper-mcp/src/tools/resources.rs:614-635` |
| `relationship_assert` is idempotent (upsert on the active-edge invariant) | `migrations/20260624000002_canonical_functions.sql:813-816` |
| `resource create` is **not** idempotent — content dedup was retired (#219) | — |
| `create()` already does a post-create edge assert (`link_session_to_task`) — the house pattern for Task 12 | `temper-cli/src/commands/resource.rs:390-399` |
| `ActInput` derives `Clone` | `temper-core/src/types/authorship.rs:103` |
| `EdgeType::DerivedFrom.legacy_mapping()` → `(EdgeKind::LeadsTo, Polarity::Inverse, "derived_from")` | `temper-workflow/src/types/graph.rs:64` |

### Deliberate divergence from the spec (Task 12)

The spec says a partial `--sources-as-edges` failure should **error**. It should **warn and exit zero** instead. Rationale: `resource create` is not idempotent, so a nonzero exit invites a retry that produces a duplicate node, while `edge assert` *is* idempotent and can be retried freely. The output reports `edges_asserted` and `edges_failed` so the failure is machine-visible. This matches the existing `link_session_to_task` best-effort tail.

---

# PR (a) — JSON output contract

**Branch:** `jct/330a-json-output-contract`
**Story:** `--format json` yields exactly one JSON document per invocation, with a consistent `id` on create-style responses.
**Blast radius:** `temper-cli` + two ack structs in `temper-core`. DB-free.

## File Structure

- Modify: `crates/temper-cli/src/commands/search_cmd.rs` — wrap results in an object
- Modify: `crates/temper-cli/src/actions/search.rs:345-363` — rewrite the array-contract test
- Modify: `crates/temper-cli/src/commands/resource.rs` — `build_show_document`, `fetch_edges`, `fetch_provenance`, single print
- Modify: `crates/temper-core/src/types/invocation_requests.rs:41` — `InvocationAck.id`
- Modify: `crates/temper-core/src/types/facet_requests.rs:40` — `FacetAck.id`
- Modify: `crates/temper-api/src/handlers/invocations.rs`, `crates/temper-api/src/handlers/facets.rs`, `crates/temper-mcp/src/tools/facets.rs` — ack constructors

---

### Task 1: `search` emits a JSON object, not a bare array

**Files:**
- Modify: `crates/temper-cli/src/commands/search_cmd.rs:33-42`
- Modify/Test: `crates/temper-cli/src/actions/search.rs:345-363`

**Interfaces:**
- Produces: `pub(crate) struct SearchResultsResponse { pub results: Vec<serde_json::Value> }` in `commands/search_cmd.rs`

- [ ] **Step 1: Rewrite the failing test**

The existing test at `crates/temper-cli/src/actions/search.rs:356-361` asserts the *old* contract (`out.starts_with('[')`). It is not deleted — it is inverted. Replace the tail of `render_search_results_json_is_passthrough_array` (rename it too):

```rust
    #[test]
    fn render_search_results_json_is_object_with_results_key() {
        let rows = vec![UnifiedSearchResultRow {
            resource_id: uuid::Uuid::nil(),
            slug: "some-slug".to_string(),
            title: "Some Title".to_string(),
            origin_uri: "file:///some/path.md".to_string(),
            context: None,
            doc_type: "task".to_string(),
            fts_score: 0.5,
            vector_score: 0.0,
            graph_score: 0.0,
            combined_score: 0.5,
            origin: "fts".to_string(),
            context_slug: None,
            context_owner_ref: None,
        }];
        let rows_value: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| serde_json::to_value(r).expect("row to value"))
            .collect();
        let doc = crate::commands::search_cmd::SearchResultsResponse { results: rows_value };
        let out =
            crate::format::render(&doc, crate::format::OutputFormat::Json).expect("json render");

        assert!(out.starts_with('{'), "json should be an object: {out}");
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("single json document");
        assert!(parsed["results"].is_array(), "results must be an array: {out}");
        assert!(out.contains("\"slug\""), "json: {out}");
        assert!(out.contains("\"title\""), "json: {out}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-cli -E 'test(render_search_results_json_is_object_with_results_key)'
```
Expected: FAIL — `SearchResultsResponse` does not exist (compile error: `could not find SearchResultsResponse in search_cmd`).

- [ ] **Step 3: Add the struct and use it**

In `crates/temper-cli/src/commands/search_cmd.rs`, add near the top of the file (after the existing `use` block):

```rust
/// Envelope for `temper search --format json`.
///
/// Search previously rendered a bare top-level array, which forced every
/// consumer to special-case it against the object every other command emits.
/// Rows stay `serde_json::Value` because `inject_ref` has already decorated
/// them with a `ref` key that is not on the wire type.
#[derive(Debug, serde::Serialize)]
pub(crate) struct SearchResultsResponse {
    pub results: Vec<serde_json::Value>,
}
```

Then replace `search_cmd.rs:33-42` (the `inject_ref` loop through the `render` call):

```rust
    // Identity-out: every printed search row carries its decorated `ref`
    // (read from `resource_id` for search rows).
    let mut results_value = serde_json::to_value(&results)
        .map_err(|e| crate::error::TemperError::Api(format!("search serialize: {e}")))?;
    if let Some(arr) = results_value.as_array_mut() {
        for row in arr.iter_mut() {
            crate::commands::resource::inject_ref(row);
        }
    }
    let results = match results_value {
        serde_json::Value::Array(rows) => rows,
        other => vec![other],
    };
    let rendered = crate::format::render(&SearchResultsResponse { results }, fmt)?;
    crate::output::plain(rendered);
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo nextest run -p temper-cli -E 'test(render_search_results_json_is_object_with_results_key)'
```
Expected: PASS.

- [ ] **Step 5: Run the crate suite and check**

```bash
cargo nextest run -p temper-cli
cargo make check
```
Expected: both green.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/search_cmd.rs crates/temper-cli/src/actions/search.rs
git commit -m "$(cat <<'EOF'
fix(cli): search --format json emits an object, not a bare array

`json.load(...)["results"]` failed against a top-level array, forcing
consumers to special-case search against every other command. Wraps rows in
a typed SearchResultsResponse. Rewrites the test that encoded the old
contract.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `resource show` emits exactly one JSON document

`show()` currently calls `println!` up to three times: the resource (`resource.rs:934`), `--edges` (`:1031`), `--provenance` (`:1061`). A single `json.load()` raises `Extra data`. This task makes multiple documents *structurally impossible* by folding them in a pure builder printed once.

Toon (the human TTY format) keeps its multi-block layout — the one-document contract is about JSON, the agent surface.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:896-945` (`show`), `:1003-1035` (`show_edges`), `:1042-1064` (`show_provenance`)
- Test: `crates/temper-cli/src/commands/resource.rs` (existing `mod tests` at the file's tail)

**Interfaces:**
- Consumes: `EdgesReport` (`resource.rs:44`), `temper_core::types::provenance::BlockProvenanceRow`
- Produces:
  - `pub(crate) fn build_show_document(metadata: serde_json::Value, body: &str, edges: Option<EdgesReport>, provenance: Option<Vec<BlockProvenanceRow>>) -> Result<serde_json::Value>`
  - `fn fetch_edges(id: ResourceId) -> Result<EdgesReport>`
  - `fn fetch_provenance(id: ResourceId) -> Result<Vec<BlockProvenanceRow>>`

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block at the end of `crates/temper-cli/src/commands/resource.rs`:

```rust
    #[test]
    fn build_show_document_folds_edges_and_provenance_into_one_object() {
        let metadata = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "title": "A Node",
        });
        let edges = EdgesReport {
            outgoing: vec![],
            incoming: vec![],
        };

        let doc = build_show_document(metadata, "# body\n", Some(edges), Some(vec![]))
            .expect("build show document");

        // One document: content, edges, and provenance all hang off the resource object.
        assert_eq!(doc["title"], "A Node");
        assert_eq!(doc["content"], "# body\n");
        assert!(doc["edges"]["outgoing"].is_array(), "edges folded: {doc}");
        assert!(doc["edges"]["incoming"].is_array(), "edges folded: {doc}");
        assert!(doc["provenance"].is_array(), "provenance folded: {doc}");

        // And it round-trips through a single `serde_json::from_str` with no trailing data.
        let rendered = serde_json::to_string_pretty(&doc).expect("render");
        let _: serde_json::Value = serde_json::from_str(&rendered).expect("exactly one document");
    }

    #[test]
    fn build_show_document_omits_absent_sections() {
        let metadata = serde_json::json!({ "id": "11111111-1111-1111-1111-111111111111" });
        let doc = build_show_document(metadata, "b", None, None).expect("build show document");

        assert_eq!(doc["content"], "b");
        assert!(doc.get("edges").is_none(), "no edges key when not requested: {doc}");
        assert!(
            doc.get("provenance").is_none(),
            "no provenance key when not requested: {doc}"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-cli -E 'test(build_show_document)'
```
Expected: FAIL — `cannot find function build_show_document in this scope`.

- [ ] **Step 3: Write the builder and the fetchers**

In `crates/temper-cli/src/commands/resource.rs`, add the pure builder above `pub fn show`:

```rust
/// Fold a resource's metadata, body, and its optional edge/provenance sections into
/// ONE JSON document.
///
/// `show` used to `println!` once per section, so `--edges` emitted two concatenated
/// JSON documents and `--provenance` a third — a single `json.load()` raised
/// `Extra data`. Building the composite here and printing once makes a multi-document
/// JSON response structurally impossible rather than merely test-detectable.
pub(crate) fn build_show_document(
    metadata: serde_json::Value,
    body: &str,
    edges: Option<EdgesReport>,
    provenance: Option<Vec<temper_core::types::provenance::BlockProvenanceRow>>,
) -> Result<serde_json::Value> {
    let mut doc = metadata;
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| TemperError::Api("resource metadata is not a JSON object".to_string()))?;

    obj.insert(
        "content".to_string(),
        serde_json::Value::String(body.to_string()),
    );

    if let Some(edges) = edges {
        obj.insert(
            "edges".to_string(),
            serde_json::to_value(edges)
                .map_err(|e| TemperError::Api(format!("edges serialize: {e}")))?,
        );
    }

    if let Some(provenance) = provenance {
        obj.insert(
            "provenance".to_string(),
            serde_json::to_value(provenance)
                .map_err(|e| TemperError::Api(format!("provenance serialize: {e}")))?,
        );
    }

    Ok(doc)
}
```

Now convert the two printers into fetchers. Replace the body of `show_edges` (`resource.rs:1003-1035`) with a fetcher of the same shape, renaming it:

```rust
/// Fetch a resource's graph edges, grouped by direction.
///
/// Cloud-only and context-free: the id was already resolved from the ref by
/// `show`. Returns data — `show` decides how to render it.
fn fetch_edges(id: temper_core::types::ids::ResourceId) -> Result<EdgesReport> {
    use crate::actions::runtime;

    let edges: Vec<temper_workflow::types::graph::GraphEdgeRow> = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .edges(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let outgoing: Vec<_> = edges
        .iter()
        .filter(|e| e.direction == "outgoing")
        .cloned()
        .collect();
    let incoming: Vec<_> = edges
        .iter()
        .filter(|e| e.direction == "incoming")
        .cloned()
        .collect();

    Ok(EdgesReport { outgoing, incoming })
}
```

And replace `show_provenance` (`resource.rs:1042-1064`):

```rust
/// Fetch the itemized per-block provenance for a resource via the API.
///
/// Hits `GET /api/resources/{id}/provenance` and returns the rows in
/// `(block, accretion)` order. An unreadable resource returns an empty list
/// (access-scoped in SQL).
fn fetch_provenance(
    id: temper_core::types::ids::ResourceId,
) -> Result<Vec<temper_core::types::provenance::BlockProvenanceRow>> {
    use crate::actions::runtime;

    runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .provenance(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })
}
```

- [ ] **Step 4: Rewire `show` to fetch-then-print-once**

Replace `resource.rs:930-944` (from `inject_ref` to the end of `show`):

```rust
    inject_ref(&mut metadata);

    // Fetch every requested section BEFORE rendering: the JSON arm folds them into
    // one document, so nothing may be printed until all of them are in hand.
    let edges = if params.edges {
        Some(fetch_edges(id)?)
    } else {
        None
    };
    let provenance = if params.provenance {
        Some(fetch_provenance(id)?)
    } else {
        None
    };

    match params.format {
        crate::format::OutputFormat::Json => {
            let doc = build_show_document(metadata, &body, edges, provenance)?;
            let rendered = crate::format::render(&doc, params.format)?;
            crate::output::plain(rendered);
        }
        // Toon is the human TTY surface: keep the frontmatter+body document, then append
        // each requested section as its own block. The one-document contract is a JSON
        // (agent-surface) invariant, not a Toon one.
        crate::format::OutputFormat::Toon => {
            let rendered = crate::format::render_resource_show(&metadata, &body, params.format)?;
            crate::output::plain(rendered);
            if let Some(edges) = edges {
                crate::output::plain(crate::format::render(&edges, params.format)?);
            }
            if let Some(provenance) = provenance {
                crate::output::plain(crate::format::render(&provenance, params.format)?);
            }
        }
    }

    Ok(())
}
```

Note `show`'s early `--meta-only` return (`resource.rs:899-901`) and the `_config` parameters on the old `show_edges`/`show_provenance` are gone; drop the now-unused `_config` args at the call sites. If clippy flags `config` as unused in `show`, keep it — `write_resource_file_from_parts` still uses `config_clone`.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-cli -E 'test(build_show_document)'
cargo nextest run -p temper-cli
cargo make check
```
Expected: all green. `cargo make check` must report no `dead_code` warning for a leftover `show_edges`/`show_provenance` — if it does, you missed a rename.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'EOF'
fix(cli): resource show emits exactly one JSON document

--edges printed a second JSON document and --provenance a third, so a single
json.load() raised "Extra data". Folds both under `edges`/`provenance` keys on
the resource object via a pure build_show_document(), printed once. Toon keeps
its multi-block layout — the one-document contract is a JSON invariant.

Multiple documents are now structurally impossible, not merely test-detectable.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: create-style acks carry a consistent `id`

A generic "create X, capture its id" helper must know `id` for `resource create`, `invocation_id` for `invocation open`, and `property_id` for `facet_set`. Add a plain `id` alongside each specific alias. Because these are wire types in `temper-core`, the MCP surface inherits the field.

`resource create` already emits `id` (flattened `ResourceRow`) — nothing to do there.
`InvocationCloseAck` already emits `disposition` — nothing to do there.

**Files:**
- Modify: `crates/temper-core/src/types/invocation_requests.rs:41-43`
- Modify: `crates/temper-core/src/types/facet_requests.rs:40-42`
- Modify: `crates/temper-api/src/handlers/invocations.rs` (the `open` handler's `InvocationAck` construction)
- Modify: `crates/temper-api/src/handlers/facets.rs:29-45` (the `set_facet` handler's `FacetAck` construction)
- Modify: `crates/temper-mcp/src/tools/facets.rs:92-94`

**Interfaces:**
- Produces: `InvocationAck { id: Uuid, invocation_id: Uuid }`, `FacetAck { id: Uuid, property_id: Uuid }`

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-core/src/types/facet_requests.rs`'s existing `mod tests`:

```rust
    #[test]
    fn facet_ack_carries_both_id_and_property_id() {
        let pid = uuid::Uuid::nil();
        let ack = FacetAck {
            id: pid,
            property_id: pid,
        };
        let v = serde_json::to_value(&ack).expect("serialize");
        assert_eq!(v["id"], v["property_id"], "id must alias property_id");
        assert!(v.get("id").is_some(), "generic `id` key present: {v}");
    }
```

And add a `mod tests` to `crates/temper-core/src/types/invocation_requests.rs` (or extend it if present):

```rust
#[cfg(test)]
mod ack_tests {
    use super::*;

    #[test]
    fn invocation_ack_carries_both_id_and_invocation_id() {
        let iid = uuid::Uuid::nil();
        let ack = InvocationAck {
            id: iid,
            invocation_id: iid,
        };
        let v = serde_json::to_value(&ack).expect("serialize");
        assert_eq!(v["id"], v["invocation_id"], "id must alias invocation_id");
        assert!(v.get("id").is_some(), "generic `id` key present: {v}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-core -E 'test(ack_carries_both)'
```
Expected: FAIL — `struct InvocationAck has no field named id` / same for `FacetAck`.

- [ ] **Step 3: Add the fields**

`crates/temper-core/src/types/invocation_requests.rs:39-43`:

```rust
/// Acknowledgement returned by the open endpoint — carries the minted
/// invocation id, fed back into the close call.
///
/// `id` is the generic create-response key (a duplicate of `invocation_id`), so a
/// caller's "create X, capture its id" helper reads `id` from every create-style ack
/// without knowing the per-command alias. Both surfaces (CLI + MCP) inherit it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct InvocationAck {
    pub id: Uuid,
    pub invocation_id: Uuid,
}
```

`crates/temper-core/src/types/facet_requests.rs:37-42`:

```rust
/// Acknowledgement returned by the facet write endpoint.
///
/// `id` duplicates `property_id` — see `InvocationAck::id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FacetAck {
    pub id: Uuid,
    pub property_id: Uuid,
}
```

- [ ] **Step 4: Fix every constructor**

Compile to find them — the type system enumerates the call sites for you:

```bash
cargo check -p temper-api -p temper-mcp -p temper-cli 2>&1 | rg "missing field"
```

Expected sites (each gets `id` set to the same value as its alias):

- `crates/temper-mcp/src/tools/facets.rs:92-94` — `FacetAck { id: Uuid::from(out.value), property_id: Uuid::from(out.value) }`
- `crates/temper-api/src/handlers/facets.rs` — same shape
- `crates/temper-api/src/handlers/invocations.rs` — `InvocationAck { id: <minted>, invocation_id: <minted> }`

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-core -E 'test(ack_carries_both)'
cargo nextest run -p temper-core -p temper-cli
cargo make check
```
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/invocation_requests.rs crates/temper-core/src/types/facet_requests.rs crates/temper-api/src/handlers crates/temper-mcp/src/tools/facets.rs
git commit -m "$(cat <<'EOF'
feat(core): create-style acks carry a consistent `id`

`resource create` returned {"id"}, `invocation open` {"invocation_id"}, and
`facet_set` {"property_id"}, so a generic "create X, capture its id" helper had
to know each command's key. Adds a plain `id` alongside each alias on the wire
type, so the MCP surface inherits it too.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: PR (a) prep

- [ ] **Step 1: Full CLI + core suites and check**

```bash
cargo nextest run -p temper-cli -p temper-core
cargo make check
```
Expected: green. If `cargo make check` fails on files you did not touch, that is a scope-creep signal — stop and report, do not fix.

- [ ] **Step 2: Push and open the PR**

```bash
git push -u origin jct/330a-json-output-contract
gh pr create --base main --title "fix(cli): one JSON document per invocation, consistent create-response id" --body "$(cat <<'EOF'
## What

PR 1 of 3 for #330. Fixes the `--format json` output contract.

- `search` emits `{"results": [...]}` instead of a bare top-level array.
- `resource show --edges --provenance` emits **one** JSON document instead of three concatenated ones. Folded via a pure `build_show_document()` printed once, so multiple documents are structurally impossible rather than test-detectable. Toon keeps its multi-block layout — the one-document contract is a JSON (agent-surface) invariant.
- `InvocationAck` and `FacetAck` carry a plain `id` alongside their specific alias. MCP inherits it.

## Why

A scripted authoring pass could not use a naive `json.load()`: `--edges` raised `Extra data: line N`, `search` broke `["results"]`, and a generic id-capturing helper needed per-command key names.

## Notes

`actions/search.rs`'s `render_search_results_json_is_passthrough_array` encoded the contract this PR deliberately breaks. It is rewritten, not deleted.

Design: `docs/superpowers/specs/2026-07-09-issue-330-cli-authoring-ergonomics-design.md`

Refs #330

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Report CI status**

Wait for CI. Report "PR up + CI green" with the URL. **Do not merge.**

---

# PR (b) — the managed tier becomes real

**Branch:** `jct/330b-managed-tier` (cut from `main` after PR (a) merges)
**Story:** The provenance trio is stamped server-side on every create, and `resource show` returns both metadata tiers.
**Blast radius:** widest. Wire types, Backend trait, three impls, ts-rs regeneration.

## File Structure

- Modify: `crates/temper-workflow/src/types/managed_meta.rs` — provenance constants
- Create: stamp fn in `crates/temper-workflow/src/operations/actions.rs`
- Modify: `crates/temper-services/src/backend/db_backend.rs:1076+` — call the stamp
- Modify: `crates/temper-workflow/src/types/resource.rs` — `ResourceDetail`
- Modify: `crates/temper-services/src/backend/substrate_read.rs:220,250` — `show_detail_select`, drop hashes, rename anchor
- Modify: `crates/temper-workflow/src/operations/backend.rs:60` + 3 impls — `show_resource` return type
- Modify: `crates/temper-api/src/handlers/resources.rs:94-109`, `crates/temper-client/src/resources.rs:60`
- Modify: `crates/temper-cli/src/commands/resource.rs:980` — `--fields` anchor
- Modify: `crates/temper-mcp/src/tools/resources.rs:614-635` — reuse the composition
- Test: `tests/e2e/tests/` — new differential test
- Docs: `packages/agent-workflows/steward/agent/instructions.md:51-52`, `.../skills/map-stewardship.md:105-112`, `crates/temper-cli/skill-content/cognitive-maps.md:143-158`

---

### Task 5: the pure provenance stamp

Keep it pure and DB-free so it unit-tests without Postgres. The `DbBackend` wiring is Task 6.

**Files:**
- Modify: `crates/temper-workflow/src/types/managed_meta.rs` (constants)
- Modify: `crates/temper-workflow/src/operations/actions.rs` (the fn + tests)

**Interfaces:**
- Produces:
  - `pub const PROVENANCE_LLM_DISCOVERED: &str = "llm-discovered";`
  - `pub const PROVENANCE_USER_CREATED: &str = "user-created";`
  - `pub fn stamp_provenance(meta: &mut ManagedMeta, act: &ActContext)`

⚠️ **Plan/reality gap:** the spec sketches `act.model` and `act.invocation_id`. Neither exists. `ActContext` is `{ invocation: Option<InvocationId>, authorship: Option<AgentAuthorship> }` and the model lives at `act.authorship.as_ref().and_then(|a| a.model.as_deref())`. Use the real shape.

- [ ] **Step 1: Write the failing tests**

Add to `crates/temper-workflow/src/operations/actions.rs`'s `mod tests`:

```rust
    use temper_core::types::authorship::{ActContext, AgentAuthorship, ConfidenceBand};
    use temper_core::types::ids::InvocationId;

    fn authored(model: Option<&str>) -> AgentAuthorship {
        AgentAuthorship {
            reasoning: None,
            confidence: ConfidenceBand::Confident,
            rationale: None,
            persona: None,
            model: model.map(String::from),
        }
    }

    #[test]
    fn stamp_provenance_fills_trio_from_llm_act() {
        let inv = InvocationId::from(uuid::Uuid::nil());
        let act = ActContext {
            invocation: Some(inv),
            authorship: Some(authored(Some("claude-opus-4-8"))),
        };
        let mut meta = ManagedMeta::default();

        stamp_provenance(&mut meta, &act);

        assert_eq!(meta.llm_model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(meta.llm_run.as_deref(), Some(uuid::Uuid::nil().to_string().as_str()));
        assert_eq!(meta.provenance.as_deref(), Some(PROVENANCE_LLM_DISCOVERED));
    }

    #[test]
    fn stamp_provenance_marks_non_llm_act_user_created() {
        let act = ActContext::default();
        let mut meta = ManagedMeta::default();

        stamp_provenance(&mut meta, &act);

        assert_eq!(meta.provenance.as_deref(), Some(PROVENANCE_USER_CREATED));
        assert!(meta.llm_model.is_none(), "no model to record");
        assert!(meta.llm_run.is_none(), "no invocation to record");
    }

    #[test]
    fn stamp_provenance_never_overwrites_caller_values() {
        let act = ActContext {
            invocation: Some(InvocationId::from(uuid::Uuid::nil())),
            authorship: Some(authored(Some("claude-opus-4-8"))),
        };
        let mut meta = ManagedMeta {
            llm_model: Some("caller-model".to_string()),
            llm_run: Some("caller-run".to_string()),
            provenance: Some(PROVENANCE_USER_CREATED.to_string()),
            ..ManagedMeta::default()
        };

        stamp_provenance(&mut meta, &act);

        assert_eq!(meta.llm_model.as_deref(), Some("caller-model"));
        assert_eq!(meta.llm_run.as_deref(), Some("caller-run"));
        assert_eq!(meta.provenance.as_deref(), Some(PROVENANCE_USER_CREATED));
    }

    #[test]
    fn stamp_provenance_authored_without_model_is_user_created() {
        // Authorship present (confidence supplied) but no model: a human act inside an
        // invocation. Provenance must not claim llm-discovered.
        let act = ActContext {
            invocation: Some(InvocationId::from(uuid::Uuid::nil())),
            authorship: Some(authored(None)),
        };
        let mut meta = ManagedMeta::default();

        stamp_provenance(&mut meta, &act);

        assert_eq!(meta.provenance.as_deref(), Some(PROVENANCE_USER_CREATED));
        assert!(meta.llm_model.is_none());
        assert_eq!(meta.llm_run.as_deref(), Some(uuid::Uuid::nil().to_string().as_str()));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-workflow -E 'test(stamp_provenance)'
```
Expected: FAIL — `cannot find function stamp_provenance`.

- [ ] **Step 3: Add the constants**

In `crates/temper-workflow/src/types/managed_meta.rs`, above `pub struct ManagedMeta`:

```rust
/// `temper-provenance` value for a resource authored by a model.
///
/// The two values below are the closed vocabulary declared by
/// `schemas/base.schema.json` (`"enum": ["llm-discovered", "user-created"]`).
/// `provenance` stays a `String` rather than a typed enum: `ManagedMeta` is a
/// `deny_unknown_fields` deserialization target for rows already in the database,
/// and a closed enum would turn any historical value outside the pair into a hard
/// readback failure.
pub const PROVENANCE_LLM_DISCOVERED: &str = "llm-discovered";

/// `temper-provenance` value for a resource authored by a person.
pub const PROVENANCE_USER_CREATED: &str = "user-created";
```

- [ ] **Step 4: Write the stamp**

In `crates/temper-workflow/src/operations/actions.rs`:

```rust
/// Fill the managed-tier provenance trio from the act envelope, never overwriting a
/// value the caller supplied.
///
/// The CLI's `--model` / `--invocation` / `--confidence` flags populate the *act event*
/// (the accountability chain), but historically wrote nothing to the resource's own
/// `managed_meta` — so a node's provenance stamp read back empty while three separate
/// stewardship docs insisted every authored node carry it. Deriving it here, on the
/// shared write path, means every surface (CLI, MCP, API) is stamped uniformly and the
/// contract cannot drift. This is the receive-side symmetric-defense pattern that
/// `ensure_managed_identity_keys` uses.
///
/// Fill-missing, never overwrite: an MCP agent that already passes an explicit
/// `managed_meta` keeps its values.
pub fn stamp_provenance(meta: &mut ManagedMeta, act: &ActContext) {
    let model = act.authorship.as_ref().and_then(|a| a.model.as_deref());

    if meta.llm_model.is_none() {
        meta.llm_model = model.map(String::from);
    }
    if meta.llm_run.is_none() {
        meta.llm_run = act.invocation.map(|i| uuid::Uuid::from(i).to_string());
    }
    if meta.provenance.is_none() {
        meta.provenance = Some(
            if model.is_some() {
                PROVENANCE_LLM_DISCOVERED
            } else {
                PROVENANCE_USER_CREATED
            }
            .to_string(),
        );
    }
}
```

Add the imports this needs at the top of `actions.rs`: `use crate::types::managed_meta::{ManagedMeta, PROVENANCE_LLM_DISCOVERED, PROVENANCE_USER_CREATED};` and `use temper_core::types::authorship::ActContext;` (merge into existing `use` groups rather than duplicating). Re-export `stamp_provenance` from `crates/temper-workflow/src/operations/mod.rs` alongside the other `actions::` re-exports.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-workflow -E 'test(stamp_provenance)'
cargo nextest run -p temper-workflow
cargo make check
```
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-workflow/src
git commit -m "$(cat <<'EOF'
feat(workflow): stamp_provenance — derive the managed-tier trio from the act

Pure, DB-free fill-missing derivation of temper-llm-model / temper-llm-run /
temper-provenance from the ActContext. `user-created` and `llm-discovered` are
the closed vocabulary base.schema.json has always declared; nothing produced
`user-created` until now.

Wiring into DbBackend::create_resource is the next commit.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: wire the stamp into `create_resource`

The stamp must run **before** `validate_managed_meta_pipeline`, which serializes `cmd.managed_meta` to a `Value` (`db_backend.rs:1133-1136`). Verified safe: `base.schema.json:108-118` declares all three keys, every doc-type schema sets `additionalProperties: true`, and the strip lists do not contain them.

**Files:**
- Modify: `crates/temper-services/src/backend/db_backend.rs:1076-1140` (inside `create_resource`)

**Interfaces:**
- Consumes: `temper_workflow::operations::stamp_provenance` (Task 5)

- [ ] **Step 1: Apply the stamp before validation**

In `crates/temper-services/src/backend/db_backend.rs`, inside `async fn create_resource`, immediately before the `let managed = validate_managed_meta_pipeline(ManagedValidationParams {` block (currently `db_backend.rs:1133`):

```rust
        // Fill the managed-tier provenance trio from the act envelope before validation.
        // Must precede `validate_managed_meta_pipeline`, which serializes `cmd.managed_meta`.
        // Fill-missing: an explicit caller value (e.g. an MCP agent passing managed_meta)
        // always wins. Safe against the pipeline — base.schema.json declares all three keys
        // and every doc-type schema is additionalProperties: true.
        let mut managed_meta = cmd.managed_meta.clone();
        temper_workflow::operations::stamp_provenance(&mut managed_meta, &cmd.act);
```

Then change the `raw_managed` field of `ManagedValidationParams` from `serde_json::to_value(&cmd.managed_meta)` to `serde_json::to_value(&managed_meta)`.

⚠️ `cmd.managed_meta` is a plain `ManagedMeta`, **not** an `Option` — do not call `.unwrap_or_default()` (the spec's sketch is wrong here).

- [ ] **Step 2: Run the required adjacent integration test**

A `DbBackend` command behavior change requires the `temper-api` integration target directly — e2e alone will not catch a regression here.

```bash
cargo make docker-up
cargo nextest run -p temper-api --features test-db --test genesis_cogmap_test
```
Expected: PASS. If it fails on an assertion about created-resource `managed_meta`, the L0 genesis fixture now carries a `user-created` stamp — update the fixture's expectation, do not weaken the stamp.

- [ ] **Step 3: Run the drift guard**

The stamp must write only keys inside the guarded ten.

```bash
cargo nextest run -p temper-services --features test-db -E 'test(managed_meta_property_drift)'
```
Expected: PASS.

- [ ] **Step 4: Check**

```bash
cargo make check
```
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/src/backend/db_backend.rs
git commit -m "$(cat <<'EOF'
feat(services): auto-stamp the provenance trio on every create

DbBackend::create_resource fills managed_meta's provenance trio from the
ActContext it already receives, before the validation pipeline. Every surface
(CLI, MCP, API) is stamped uniformly; explicit caller values always win.

Closes the gap where `resource create --model M --invocation INV` recorded the
act in the accountability chain but left the node's managed_meta empty.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: `ResourceDetail` — full `show` carries both meta tiers

Today `GET /api/resources/{id}` returns `ResourceRow`, which has **neither** meta tier — only flat projections (`stage`, `seq`, `mode`, `effort`, `body_hash`). `list` shares the type, so nothing may be added to it without cost on every row.

**Files:**
- Modify: `crates/temper-workflow/src/types/resource.rs` — add `ResourceDetail`
- Modify: `crates/temper-services/src/backend/substrate_read.rs:220-228` — add `show_detail_select`
- Modify: `crates/temper-workflow/src/operations/backend.rs:60` — `show_resource` return type
- Modify: `crates/temper-services/src/backend/db_backend.rs:1235`, `crates/temper-cli/src/cloud_backend/backend.rs:145,513`
- Modify: `crates/temper-api/src/handlers/resources.rs:94-109`
- Modify: `crates/temper-client/src/resources.rs:60-67`
- Modify: `crates/temper-mcp/src/tools/resources.rs:614-635` — reuse the composition

**Interfaces:**
- Produces:
  - `pub struct ResourceDetail { #[serde(flatten)] row: ResourceRow, managed_meta: Option<ManagedMeta>, open_meta: Option<Value> }`
  - `pub async fn show_detail_select(pool, profile_id, id) -> ApiResult<ResourceDetail>`
  - `Backend::show_resource(&self, cmd: ShowResource) -> Result<CommandOutput<ResourceDetail>, TemperError>`

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-workflow/src/types/resource.rs`'s `mod tests`:

```rust
    #[test]
    fn resource_detail_flattens_row_and_carries_both_meta_tiers() {
        let detail = ResourceDetail {
            row: sample_resource_row(),
            managed_meta: Some(ManagedMeta {
                mode: Some("build".to_string()),
                ..ManagedMeta::default()
            }),
            open_meta: Some(serde_json::json!({ "custom": "value" })),
        };

        let v = serde_json::to_value(&detail).expect("serialize");

        // ResourceRow's fields are flattened to the top level, not nested under `row`.
        assert!(v.get("row").is_none(), "row must be flattened: {v}");
        assert!(v.get("id").is_some(), "flattened id: {v}");
        assert_eq!(v["managed_meta"]["temper-mode"], "build");
        assert_eq!(v["open_meta"]["custom"], "value");
    }

    #[test]
    fn resource_detail_omits_absent_meta_tiers() {
        let detail = ResourceDetail {
            row: sample_resource_row(),
            managed_meta: None,
            open_meta: None,
        };
        let v = serde_json::to_value(&detail).expect("serialize");
        assert!(v.get("managed_meta").is_none(), "{v}");
        assert!(v.get("open_meta").is_none(), "{v}");
    }
```

If `sample_resource_row()` does not exist in that test module, write it as a local helper constructing a `ResourceRow` with `uuid::Uuid::nil()` ids, empty strings, `is_active: true`, and `None` for every `Option` — matching the field list at `resource.rs:18-58`.

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-workflow -E 'test(resource_detail)'
```
Expected: FAIL — `cannot find type ResourceDetail`.

- [ ] **Step 3: Add the type**

In `crates/temper-workflow/src/types/resource.rs`, after `ResourceRow`:

```rust
/// The single-resource read projection: a `ResourceRow` plus both metadata tiers.
///
/// `show` used to return a bare `ResourceRow`, which carries only the flat managed
/// projections (`stage`/`seq`/`mode`/`effort`) — so the "full" view silently omitted
/// both `managed_meta` and `open_meta`, and a script reading `open_meta` from it got
/// `None`. `list` keeps returning `ResourceRow`, so a 200-row list pays nothing for
/// the tiers.
///
/// The two meta fields carry serde attributes identical to `ResourceMetaResponse`'s,
/// so `--meta-only` is a literal strict subset of this shape.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceDetail {
    #[serde(flatten)]
    pub row: ResourceRow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<crate::types::managed_meta::ManagedMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
}
```

- [ ] **Step 4: Add the composing read**

In `crates/temper-services/src/backend/substrate_read.rs`, after `show_select` (`:220-228`):

```rust
/// `show_detail` — one resource with both metadata tiers.
///
/// Composes the two existing readbacks rather than introducing a joined query: that
/// keeps this free of a new `sqlx::query!` macro (and therefore of the `.sqlx` cache
/// regeneration ritual). Two round-trips for one resource is not an N+1.
///
/// This is the composition `temper-mcp`'s `get_resource` already performed inline.
pub async fn show_detail_select(
    pool: &PgPool,
    profile_id: ProfileId,
    id: ResourceId,
) -> ApiResult<ResourceDetail> {
    let row = native_resource_row(pool, profile_id, id)
        .await
        .map_err(ApiError::from)?;
    let meta = get_meta_select(pool, profile_id, id).await?;

    Ok(ResourceDetail {
        row,
        managed_meta: meta.managed_meta,
        open_meta: meta.open_meta,
    })
}
```

- [ ] **Step 5: Widen the Backend trait and its three impls**

`crates/temper-workflow/src/operations/backend.rs:60` — change the return type:

```rust
    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<crate::types::resource::ResourceDetail>, TemperError>;
```

`crates/temper-services/src/backend/db_backend.rs:1235` — call `show_detail_select` instead of `show_select`.

`crates/temper-cli/src/cloud_backend/backend.rs:145` and `:513` — both delegate to `client.resources().get(...)`, whose return type changes in Step 7. Update their signatures to `CommandOutput<ResourceDetail>`.

- [ ] **Step 6: Update the API handler**

`crates/temper-api/src/handlers/resources.rs:94-109` — change `ApiResult<Json<ResourceRow>>` to `ApiResult<Json<ResourceDetail>>`, and the utoipa `body = ResourceRow` at `:88`-ish to `body = ResourceDetail`. Register `ResourceDetail` in the OpenAPI components list wherever `ResourceRow` is registered.

- [ ] **Step 7: Update the client**

`crates/temper-client/src/resources.rs:60-67`:

```rust
    /// GET /api/resources/{id} — one resource plus both metadata tiers.
    pub async fn get(&self, id: Uuid) -> Result<ResourceDetail> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
```

- [ ] **Step 8: Fix the CLI and MCP consumers**

`crates/temper-cli/src/commands/resource.rs:906-926` (`show`) — `row` is now a `ResourceDetail`. `write_resource_file_from_parts` expects a `ResourceRow`; pass `&row.row`. `serde_json::to_value(&row)` now yields both tiers, which is the point.

`crates/temper-mcp/src/tools/resources.rs:614-635` — replace the inline `show_select` + `get_meta_select` composition with a single `substrate_read::show_detail_select(...)` call.

Compile-drive the rest:

```bash
cargo check --workspace --exclude temper-cloud 2>&1 | rg "^error" | head -20
```

- [ ] **Step 9: Run the tests**

```bash
cargo nextest run -p temper-workflow -E 'test(resource_detail)'
cargo nextest run -p temper-workflow -p temper-cli
cargo nextest run -p temper-api --features test-db --test genesis_cogmap_test
cargo make check
```
Expected: all green.

- [ ] **Step 10: Commit**

```bash
git add crates/
git commit -m "$(cat <<'EOF'
feat(workflow): ResourceDetail — full `resource show` carries both meta tiers

`show` returned a bare ResourceRow, which has neither managed_meta nor
open_meta — only flat projections. The "full" view silently omitted a field the
narrower --meta-only projection returned.

Adds ResourceDetail (flattened row + both tiers) on GET /api/resources/{id}.
`list` keeps the lean ResourceRow so it pays nothing. Composes the two existing
readbacks rather than adding a joined query, so no new sqlx macro and no cache
regeneration. Formalizes the composition temper-mcp's get_resource already did
inline.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: `--meta-only` becomes a literal strict subset

`ResourceMetaResponse` anchors on `resource_id` while `ResourceRow` anchors on `id`, so the subset relation is currently unachievable. And `managed_hash`/`open_hash` have been hardcoded `String::new()` since the §7 dissolve (`substrate_read.rs:265-266`).

**Files:**
- Modify: `crates/temper-workflow/src/types/managed_meta.rs:83-104`
- Modify: `crates/temper-services/src/backend/substrate_read.rs:250-268`
- Modify: `crates/temper-cli/src/commands/resource.rs:980` (the `--fields` anchor)

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-workflow/src/types/managed_meta.rs`'s `mod tests`:

```rust
    #[test]
    fn meta_response_anchors_on_id_and_has_no_hashes() {
        let resp = ResourceMetaResponse {
            id: ResourceId::from(uuid::Uuid::nil()),
            managed_meta: Some(ManagedMeta::default()),
            open_meta: Some(serde_json::json!({})),
        };
        let v = serde_json::to_value(&resp).expect("serialize");

        // Anchors on `id`, matching ResourceRow — this is what makes --meta-only a
        // literal strict subset of the full `show` object.
        assert!(v.get("id").is_some(), "anchor is `id`: {v}");
        assert!(v.get("resource_id").is_none(), "old anchor gone: {v}");

        // The §7-dissolved hashes are removed, not emitted empty.
        assert!(v.get("managed_hash").is_none(), "{v}");
        assert!(v.get("open_hash").is_none(), "{v}");
    }
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo nextest run -p temper-workflow -E 'test(meta_response_anchors_on_id)'
```
Expected: FAIL — `struct ResourceMetaResponse has no field named id` (it has `resource_id`).

- [ ] **Step 3: Rename the anchor and delete the hashes**

`crates/temper-workflow/src/types/managed_meta.rs:83-104`:

```rust
pub struct ResourceMetaResponse {
    /// UUID of the resource. Named `id` (not `resource_id`) so this response is a
    /// literal strict subset of `ResourceDetail` — `--meta-only` returns the same keys
    /// the full `show` does, and nothing else.
    pub id: ResourceId,
    /// Typed managed (temper-*) frontmatter from the manifest — the closed
    /// Property vocabulary. Only the named `temper-*` keys are represented;
    /// there is no catch-all (a stored non-Property key is not surfaced here).
    /// `None` only if the manifest row predates meta population.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    /// Open (user-defined) frontmatter fields from the manifest.
    /// Intentionally untyped — open_meta is the free-form tier. Typed
    /// extraction of relationship fields lives in `ResourceRelationships`
    /// (see `temper-core::types::graph`), which parses this value on
    /// demand and ignores anything it doesn't recognize.
    /// `None` only if the manifest row predates meta population.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<Value>,
}
```

(`managed_hash` and `open_hash` are deleted. They were §7-dissolved: always `""`, never usable.)

`crates/temper-services/src/backend/substrate_read.rs:249-268` — drop the two empty-string fields, rename the anchor, and fix the stale doc comment:

```rust
/// `get_meta` — managed/open frontmatter for one resource (`readback::meta`, the §7 inverse fate).
pub async fn get_meta_select(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
) -> ApiResult<ResourceMetaResponse> {
    let new_id = Uuid::from(resource_id);
    let rb = readback::meta(pool, profile_id, resource_id)
        .await
        .map_err(|e| ApiError::from(map_readback_err(e)))?;
    let managed: ManagedMeta =
        serde_json::from_value(serde_json::Value::Object(rb.managed)).map_err(api_err)?;
    Ok(ResourceMetaResponse {
        id: ResourceId::from(new_id),
        managed_meta: Some(managed),
        open_meta: Some(serde_json::Value::Object(rb.open)),
    })
}
```

- [ ] **Step 4: Retarget the `--fields` anchor**

`crates/temper-cli/src/commands/resource.rs:975-981` — the projection anchor for the meta response is now `"id"`:

```rust
    // Inject `ref` before the `--fields` filter (parity with `list`): the
    // anchor `id` is always preserved, and `ref` is kept only when
    // requested — so `--fields` controls its visibility consistently.
    inject_ref(&mut value);
    let filtered = temper_core::projection::apply_top_level_filter(value, &fields_inner, "id")
        .map_err(map_projection_error)?;
```

Leave `inject_ref` (`resource.rs:59-62`) reading **both** `id` and `resource_id` — `UnifiedSearchResultRow` still anchors on `resource_id`, and search rows flow through the same helper. Update its doc comment at `:52-53` to say so.

Update the two assertions at `resource.rs:2060` and `:2114` that check `row.get("resource_id")` for the meta path — they now assert `id`. Leave the list-meta anchor at `:709` alone unless the compiler says otherwise; `list --meta-only` returns `ResourceMetaListResponse`, a different type (verify with `rg -n "ResourceMetaListResponse" crates/temper-workflow/src/types/managed_meta.rs`; if its rows are `ResourceMetaResponse`, retarget `:709` to `"id"` too).

- [ ] **Step 5: Compile-drive the remaining consumers**

```bash
cargo check --workspace --exclude temper-cloud 2>&1 | rg "^error" | head -20
```
Fix each `resource_id` / `managed_hash` / `open_hash` reference. Expect hits in `temper-mcp/src/tools/resources.rs` and `temper-client`.

- [ ] **Step 6: Regenerate TypeScript types**

Wire types changed.

```bash
cargo make generate-ts-types
```
Then check whether `packages/temper-ui` reads `resource_id`/`managed_hash`/`open_hash` off the meta response:

```bash
rg -n "managed_hash|open_hash" packages/temper-ui/src packages/temper-cloud/src
```
Fix any hits.

- [ ] **Step 7: Run the tests**

```bash
cargo nextest run -p temper-workflow -p temper-cli
cargo make check
cd packages/temper-ui && bun run check && cd -
```
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add crates/ packages/
git commit -m "$(cat <<'EOF'
refactor(workflow): --meta-only is a literal strict subset of `show`

Renames ResourceMetaResponse.resource_id -> id so the meta projection anchors on
the same key ResourceRow does; without this the subset relation is unachievable.
Retargets the --fields anchor. inject_ref still reads both keys because search
rows anchor on resource_id.

Deletes managed_hash/open_hash. They have been hardcoded String::new() since the
§7 dissolve and no consumer can use them — the real body_hash on ResourceRow
stays. Removing dead fields rather than documenting two permanently-empty strings.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: the differential e2e — subset + stamp in one test

This is the acceptance criterion, encoded as a differential test: the two read paths check each other, so no hand-written expected shape can bake in an author's misunderstanding.

**Files:**
- Create: `tests/e2e/tests/resource_meta_tiers_test.rs`

- [ ] **Step 1: Write the failing test**

```rust
#![cfg(feature = "test-db")]
//! Issue #330: full `resource show` carries both metadata tiers, `--meta-only` is a
//! literal strict subset of it, and the provenance trio is stamped server-side.
//!
//! Differential by construction: the assertion compares the two read paths against
//! each other rather than against a typed-out expected shape.

mod common;

use common::TestHarness;

#[tokio::test]
async fn meta_only_is_a_strict_subset_of_full_show_and_carries_the_stamp() {
    let harness = TestHarness::new().await;
    let ctx = harness.create_context("subset-ctx").await;

    // Create through the real CLI code path, with an LLM authorship envelope.
    let created = harness
        .cli(&[
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Subset Probe",
            "--context",
            &ctx.r#ref,
            "--model",
            "claude-opus-4-8",
            "--confidence",
            "confident",
            "--open-meta",
            r#"{"custom":"value"}"#,
            "--format",
            "json",
        ])
        .await
        .expect("create");

    let created: serde_json::Value =
        serde_json::from_str(&created).expect("create emits exactly one JSON document");
    let r#ref = created["ref"].as_str().unwrap_or_else(|| {
        created["id"]
            .as_str()
            .expect("create response carries an id")
    });

    // Full show.
    let full_out = harness
        .cli(&["resource", "show", r#ref, "--format", "json"])
        .await
        .expect("show");
    let full: serde_json::Value =
        serde_json::from_str(&full_out).expect("show emits exactly one JSON document");

    // The server-side stamp landed in the managed tier.
    assert_eq!(
        full["managed_meta"]["temper-provenance"], "llm-discovered",
        "provenance stamped: {full}"
    );
    assert_eq!(
        full["managed_meta"]["temper-llm-model"], "claude-opus-4-8",
        "model stamped: {full}"
    );
    // open_meta is present on the FULL view (it used to be absent entirely).
    assert_eq!(full["open_meta"]["custom"], "value", "open_meta on full show: {full}");

    // Meta-only.
    let meta_out = harness
        .cli(&["resource", "show", r#ref, "--meta-only", "--format", "json"])
        .await
        .expect("show --meta-only");
    let meta: serde_json::Value =
        serde_json::from_str(&meta_out).expect("meta-only emits exactly one JSON document");

    // THE differential assertion: every key --meta-only returns is present, with an
    // equal value, in the full show object. No expected shape is typed out.
    let meta_obj = meta.as_object().expect("meta-only is an object");
    assert!(!meta_obj.is_empty(), "meta-only returned nothing");
    for (key, meta_value) in meta_obj {
        let full_value = full
            .get(key)
            .unwrap_or_else(|| panic!("full show is missing `{key}` that --meta-only returned"));
        assert_eq!(
            full_value, meta_value,
            "`{key}` disagrees between full show and --meta-only"
        );
    }

    // And the dissolved hashes are gone from the wire entirely.
    assert!(meta_obj.get("managed_hash").is_none(), "{meta}");
    assert!(meta_obj.get("open_hash").is_none(), "{meta}");
}

#[tokio::test]
async fn non_llm_create_is_stamped_user_created() {
    let harness = TestHarness::new().await;
    let ctx = harness.create_context("user-created-ctx").await;

    let created = harness
        .cli(&[
            "resource", "create", "--type", "concept", "--title", "Human Node",
            "--context", &ctx.r#ref, "--format", "json",
        ])
        .await
        .expect("create");
    let created: serde_json::Value = serde_json::from_str(&created).expect("one document");
    let r#ref = created["id"].as_str().expect("id");

    let full_out = harness
        .cli(&["resource", "show", r#ref, "--format", "json"])
        .await
        .expect("show");
    let full: serde_json::Value = serde_json::from_str(&full_out).expect("one document");

    assert_eq!(
        full["managed_meta"]["temper-provenance"], "user-created",
        "no --model means a human act: {full}"
    );
    assert!(
        full["managed_meta"].get("temper-llm-model").is_none(),
        "no model to record: {full}"
    );
}
```

⚠️ Match the harness API to what `tests/e2e/tests/common/` actually exposes. Read `tests/e2e/tests/common/mod.rs` first and adapt `TestHarness::new()`, `create_context`, and `cli(...)` to the real names — the shapes above are illustrative of the *assertions*, not of the harness. The differential loop is the part that must survive adaptation.

- [ ] **Step 2: Rebuild the binary, then run it**

`test-e2e` does **not** rebuild the CLI binary.

```bash
cargo build -p temper-cli --bin temper
cargo make docker-up
cargo nextest run -p temper-e2e --features test-db --test resource_meta_tiers_test
```
Expected: FAIL first if run before Tasks 6-8 land; PASS after. Never run two e2e suites concurrently.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/resource_meta_tiers_test.rs
git commit -m "$(cat <<'EOF'
test(e2e): --meta-only is a strict subset of full show; provenance is stamped

Differential rather than hand-written: asserts every key --meta-only returns is
present with an equal value in the full show object, so the two read paths check
each other instead of a typed-out expectation reproducing the author's
misunderstanding.

Also pins llm-discovered vs user-created on the create path.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: retire the manual-trio instruction, then PR (b) prep

The auto-stamp obsoletes three docs that tell agents to pass the trio by hand on every `create_resource`. They ship here, alongside the behavior that obsoletes them.

**Files:**
- Modify: `packages/agent-workflows/steward/agent/instructions.md:51-52`
- Modify: `packages/agent-workflows/steward/agent/skills/map-stewardship.md:105-112`
- Modify: `crates/temper-cli/skill-content/cognitive-maps.md:143-158`

- [ ] **Step 1: Rewrite the three passages**

Read each passage first. Each currently instructs the agent to supply `temper-provenance` / `temper-llm-model` / `temper-llm-run` in `managed_meta` on every create. Replace with the new truth, in each file's own voice:

> Provenance is stamped for you. The server fills `temper-provenance`,
> `temper-llm-model`, and `temper-llm-run` into `managed_meta` from your act
> envelope (`--model` + `--invocation` + `--confidence`) on every create, across
> every surface. Pass them explicitly only to override a derived value — an
> explicit `managed_meta` always wins.
>
> Provenance still belongs in `managed_meta`, never in an ad-hoc `open_meta` blob.

Keep the surrounding "which band when" confidence rubric untouched.

- [ ] **Step 2: Verify no doc still demands the manual trio**

```bash
rg -n "temper-llm-run" packages/agent-workflows crates/temper-cli/skill-content
```
Expected: only the new "stamped for you" passages and any reference table entry. No imperative "pass this on every create".

- [ ] **Step 3: Full PR (b) verification**

```bash
cargo make check
cargo nextest run -p temper-workflow -p temper-cli -p temper-core
cargo nextest run -p temper-api --features test-db --test genesis_cogmap_test
cargo nextest run -p temper-services --features test-db -E 'test(managed_meta_property_drift)'
cargo build -p temper-cli --bin temper
cargo nextest run -p temper-e2e --features test-db --test resource_meta_tiers_test
```
Expected: all green. Do **not** run `cargo make test-all`.

- [ ] **Step 4: Commit, push, open the PR**

```bash
git add packages/agent-workflows crates/temper-cli/skill-content
git commit -m "$(cat <<'EOF'
docs(steward): provenance is stamped server-side, not passed by hand

Three docs instructed agents to supply the provenance trio on every
create_resource. The auto-stamp obsoletes that instruction; they now say an
explicit managed_meta overrides a derived value, and nothing more.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"

git push -u origin jct/330b-managed-tier
gh pr create --base main --title "feat: the managed tier becomes real — server-side provenance stamp + ResourceDetail" --body "$(cat <<'EOF'
## What

PR 2 of 3 for #330.

- `DbBackend::create_resource` stamps `temper-provenance` / `temper-llm-model` / `temper-llm-run` into `managed_meta` from the `ActContext`, fill-missing, on **every** surface. `llm-discovered` when a model is present, `user-created` when not.
- New `ResourceDetail` wire type on `GET /api/resources/{id}`: a flattened `ResourceRow` plus both meta tiers. `list` keeps the lean `ResourceRow`.
- `ResourceMetaResponse.resource_id` → `id`, making `--meta-only` a **literal** strict subset of full `show`.
- `managed_hash` / `open_hash` deleted — hardcoded `""` since the §7 dissolve.
- The stewardship docs stop telling agents to pass the trio by hand.

## Why

`resource create --model M --invocation INV` recorded the act in the accountability chain but left the node's `managed_meta` empty — so item 4's reported "`managed_meta: {}`" was item 3's write gap observed downstream, not a read bug. Fixing the stamp without fixing the read leaves you unable to verify the stamp; fixing the read without the stamp gives a correct view of nothing. One story, one PR.

`user-created` is not invented here: `base.schema.json:108-111` has always declared `"enum": ["llm-discovered", "user-created"]`. Nothing produced the second value until now.

## Notes

- `provenance` stays `Option<String>`, not a typed enum: `ManagedMeta` is a `deny_unknown_fields` deserialization target for rows already in the database, and a closed enum would turn any historical value outside the pair into a hard readback failure.
- `show_detail_select` composes the two existing readbacks instead of adding a joined query — **no new `sqlx::query!` macro, no `.sqlx` cache regeneration.**
- The e2e is differential: it asserts `--meta-only`'s keys are a subset of full `show`'s, so the two paths check each other rather than a typed-out expectation.

Design: `docs/superpowers/specs/2026-07-09-issue-330-cli-authoring-ergonomics-design.md`

Refs #330

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 5: Report CI status.** Wait for CI, report the URL. **Do not merge.**

---

# PR (c) — authoring ergonomics + description truth

**Branch:** `jct/330c-authoring-ergonomics` (cut from `main` after PR (b) merges — it rebases on (b)'s `cognitive-maps.md` edits)
**Story:** Add the one missing verb, reduce the fan-out boilerplate, make `disposition` readable, and stop the skill content from lying.

## File Structure

- Modify: `crates/temper-cli/src/cli.rs` — `CogmapCmd::Materialize`, `Create.sources_as_edges`, `--disposition` long_help
- Modify: `crates/temper-cli/src/main.rs:775-818` — dispatch arms
- Modify: `crates/temper-cli/src/commands/cogmap.rs`, `crates/temper-cli/src/actions/cogmap.rs`
- Modify: `crates/temper-cli/src/commands/resource.rs` — `--sources-as-edges`
- Modify: `crates/temper-core/src/types/invocation.rs` — `Disposition::try_from`, `InvocationView.disposition`
- Modify: `crates/temper-services/src/backend/substrate_read.rs:666` — derive it
- Modify: `crates/temper-cli/skill-content/cognitive-maps.md`, `.../reference.md`
- Modify: `crates/temper-mcp/src/tools/facets.rs`, `.../cognitive_maps.rs` — descriptions
- Test: `crates/temper-cli/src/cli.rs` — clap-introspection guard

---

### Task 11: `temper cogmap materialize`

The only genuinely missing verb. Every layer beneath it exists: client (`cognitive_maps.rs:104`), route (`routes.rs:185`), handler, `Backend::materialize_on_threshold`, `DbBackend` impl. This is CLI-layer only.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:977-983` (after `Analytics`)
- Modify: `crates/temper-cli/src/main.rs:793` (after the `Analytics` arm)
- Modify: `crates/temper-cli/src/actions/cogmap.rs` (after `analytics_api`, `:66`)
- Modify: `crates/temper-cli/src/commands/cogmap.rs` (after `analytics`)

**Interfaces:**
- Produces: `CogmapCmd::Materialize { cogmap: String, threshold: Option<i64> }`, `actions::cogmap::materialize_api`, `commands::cogmap::materialize`

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-cli/src/cli.rs`'s `mod tests`:

```rust
    #[test]
    fn cogmap_materialize_parses() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "temper",
            "cogmap",
            "materialize",
            "my-map-00000000-0000-0000-0000-000000000001",
            "--threshold",
            "25",
        ])
        .expect("cogmap materialize should parse");

        match cli.command {
            Commands::Cogmap {
                cmd: CogmapCmd::Materialize { cogmap, threshold },
            } => {
                assert_eq!(cogmap, "my-map-00000000-0000-0000-0000-000000000001");
                assert_eq!(threshold, Some(25));
            }
            other => panic!("expected cogmap materialize, got {other:?}"),
        }
    }

    #[test]
    fn cogmap_materialize_threshold_is_optional() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["temper", "cogmap", "materialize", "some-ref"])
            .expect("threshold is optional");
        match cli.command {
            Commands::Cogmap {
                cmd: CogmapCmd::Materialize { threshold, .. },
            } => assert_eq!(threshold, None),
            other => panic!("expected cogmap materialize, got {other:?}"),
        }
    }
```

If `Commands` does not derive `Debug`, use `_ => panic!("expected cogmap materialize")` instead of `{other:?}`.

- [ ] **Step 2: Run to verify failure**

```bash
cargo nextest run -p temper-cli -E 'test(cogmap_materialize)'
```
Expected: FAIL — `no variant named Materialize`.

- [ ] **Step 3: Add the clap variant**

In `crates/temper-cli/src/cli.rs`, inside `enum CogmapCmd`, immediately after the `Analytics` variant (`:977-981`):

```rust
    /// Re-materialize a cognitive map's regions when its event delta clears the threshold.
    ///
    /// Regions only exist *after* a materialize. A map below the threshold is a no-op
    /// (`materialized: false`), not an error.
    Materialize {
        /// The cognitive map, by ref (UUID or `slug-<uuid>`).
        cogmap: String,
        /// Minimum unmaterialized-event count required to trigger. Server default when omitted.
        #[arg(long)]
        threshold: Option<i64>,
    },
```

- [ ] **Step 4: Add the action wrapper**

In `crates/temper-cli/src/actions/cogmap.rs`, after `analytics_api` (`:66`), matching the file's existing wrapper shape:

```rust
/// POST /api/cognitive-maps/{id}/materialize — recompute the map's regions.
pub async fn materialize_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    threshold: Option<i64>,
) -> crate::error::Result<temper_core::types::materialize::MaterializeAck> {
    client
        .cognitive_maps()
        .materialize(cogmap_id, threshold)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}
```

Confirm the `MaterializeAck` path with `rg -n "pub struct MaterializeAck" crates/temper-core/src` and the `client_err_to_temper` name against the neighbouring `analytics_api` — copy whatever it uses.

- [ ] **Step 5: Add the command body**

In `crates/temper-cli/src/commands/cogmap.rs`, after `analytics`:

```rust
/// `temper cogmap materialize <cogmap_ref> [--threshold N]` — recompute the map's regions.
///
/// The write-side counterpart to `shape`/`region-metrics`/`analytics`: regions only exist
/// after a materialize, so an authoring pass that creates nodes and asserts edges must
/// materialize before the read tier reflects them.
pub fn materialize(cogmap_ref: &str, threshold: Option<i64>, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let ack = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::cogmap::materialize_api(client, cogmap_id, threshold).await })
    })?;

    let rendered = crate::format::render(&ack, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}
```

- [ ] **Step 6: Dispatch it**

In `crates/temper-cli/src/main.rs`, after the `CogmapCmd::Analytics` arm (`:793`):

```rust
            CogmapCmd::Materialize { cogmap, threshold } => {
                commands::cogmap::materialize(&cogmap, threshold, output_format)
            }
```

- [ ] **Step 7: Verify**

```bash
cargo nextest run -p temper-cli -E 'test(cogmap_materialize)'
cargo nextest run -p temper-cli
cargo make check
```
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src
git commit -m "$(cat <<'EOF'
feat(cli): temper cogmap materialize

The write-side counterpart to cogmap shape/region-metrics/analytics. Every layer
beneath it already existed (client, route, handler, Backend trait, DbBackend) —
only the CLI verb was missing, forcing a scripted authoring pass to switch to MCP
mid-run.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: `--sources-as-edges`

`--sources` records block provenance; it never creates a graph edge. Citing N sources still required N `edge assert` calls. This adds an opt-in flag that asserts one `derived_from` edge per **resource-valued** source.

⚠️ `ProvenanceSource` has **three** variants — `Event(Uuid)`, `Resource(Uuid)`, `Remote(String)`. Only `Resource` can become an edge; `Event` and `Remote` have no resource target and are skipped.

⚠️ **Divergence from the spec, deliberate.** The spec says a partial failure should error. It must **warn and exit zero**: `resource create` is not idempotent (dedup retired, #219), so a nonzero exit invites a retry that creates a duplicate node — whereas `relationship_assert` *is* idempotent (`canonical_functions.sql:813-816`) and can be retried freely. Follow the existing best-effort tail at `resource.rs:390-399` (`link_session_to_task`).

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:348-352` (near `--sources`)
- Modify: `crates/temper-cli/src/main.rs` (the `Resource::Create` dispatch arm)
- Modify: `crates/temper-cli/src/commands/resource.rs:14-19` (`CreateActionResult`), `:160-199` (`CreateResourceArgs`), `:380-407` (`create`'s tail)

**Interfaces:**
- Produces:
  - `CreateResourceArgs.sources_as_edges: bool`
  - `CreateActionResult { status, resource, edges_asserted: Vec<Uuid>, edges_failed: Vec<Uuid> }`
  - `fn assert_source_edges(client, source: ResourceId, sources: &[ProvenanceSource], act: ActInput) -> (Vec<Uuid>, Vec<Uuid>)`

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-cli/src/commands/resource.rs`'s `mod tests`:

```rust
    #[test]
    fn source_edge_targets_selects_only_resource_sources() {
        use temper_core::types::provenance::ProvenanceSource;

        let a = uuid::Uuid::from_u128(1);
        let b = uuid::Uuid::from_u128(2);
        let sources = vec![
            ProvenanceSource::Resource(a),
            ProvenanceSource::Remote("https://example.com/post".to_string()),
            ProvenanceSource::Resource(b),
            ProvenanceSource::Event(uuid::Uuid::from_u128(3)),
        ];

        let targets = source_edge_targets(&sources);

        // Remote URLs and event ids have no resource target — they cannot become edges.
        assert_eq!(targets, vec![a, b]);
    }

    #[test]
    fn source_edge_targets_is_empty_without_resource_sources() {
        use temper_core::types::provenance::ProvenanceSource;
        let sources = vec![ProvenanceSource::Remote("https://x.test".to_string())];
        assert!(source_edge_targets(&sources).is_empty());
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo nextest run -p temper-cli -E 'test(source_edge_targets)'
```
Expected: FAIL — `cannot find function source_edge_targets`.

- [ ] **Step 3: Write the pure selector**

In `crates/temper-cli/src/commands/resource.rs`, next to `resolve_provenance_sources`:

```rust
/// The subset of `--sources` that can become `derived_from` graph edges.
///
/// Only `ProvenanceSource::Resource` has a resource target. `Remote` (an external URL)
/// and `Event` (a kb_events id) are recorded as block provenance but have no node to
/// point an edge at, so they are silently skipped rather than erroring — citing a URL
/// alongside two resources is a normal thing to do.
fn source_edge_targets(
    sources: &[temper_core::types::provenance::ProvenanceSource],
) -> Vec<uuid::Uuid> {
    use temper_core::types::provenance::ProvenanceSource;
    sources
        .iter()
        .filter_map(|s| match s {
            ProvenanceSource::Resource(id) => Some(*id),
            ProvenanceSource::Remote(_) | ProvenanceSource::Event(_) => None,
        })
        .collect()
}
```

- [ ] **Step 4: Add the flag**

`crates/temper-cli/src/cli.rs`, immediately after the `sources` arg (`:348-352`):

```rust
        /// Also assert a `derived_from` edge from the new resource to each
        /// resource-valued `--sources` entry. Remote URLs are skipped (no edge target).
        ///
        /// Not atomic: the edges are asserted after the create commits. A failed edge
        /// warns rather than failing the command — `edge assert` is idempotent, so
        /// re-asserting is safe, while re-running a create is not.
        #[arg(long, requires = "sources")]
        sources_as_edges: bool,
```

Thread it through `CreateResourceArgs` (add `pub sources_as_edges: bool,`) and the `main.rs` dispatch arm.

- [ ] **Step 5: Extend the create result**

`crates/temper-cli/src/commands/resource.rs:14-19`:

```rust
#[derive(Debug, serde::Serialize)]
pub(crate) struct CreateActionResult {
    pub status: &'static str,
    #[serde(flatten)]
    pub resource: temper_workflow::types::resource::ResourceRow,
    /// Targets of the `derived_from` edges asserted by `--sources-as-edges`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edges_asserted: Vec<uuid::Uuid>,
    /// Sources whose edge assert failed. The resource exists; re-assert with
    /// `temper edge assert` (idempotent) rather than re-running the create.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edges_failed: Vec<uuid::Uuid>,
}
```

Fix the other `CreateActionResult` construction sites the compiler names (default both vectors to `Vec::new()`).

- [ ] **Step 6: Assert the edges in `create`'s tail**

`create()` already keeps `client` and `runtime` in scope for `link_session_to_task` (`resource.rs:390-399`). Immediately after that block, before building `CreateActionResult`:

```rust
    // `--sources-as-edges`: one `derived_from` edge per resource-valued source.
    //
    // Deliberately NOT atomic and deliberately NOT fatal. The create has already
    // committed and is not idempotent (content dedup was retired), so failing here
    // would push an author toward re-running the create and duplicating the node.
    // `relationship_assert` upserts on the active-edge invariant, so a failed edge is
    // safely re-assertable with `temper edge assert`. Mirrors `link_session_to_task`.
    let (edges_asserted, edges_failed) = if sources_as_edges {
        let targets = source_edge_targets(&resolved_sources);
        let mut asserted = Vec::new();
        let mut failed = Vec::new();

        for target in targets {
            let req = temper_core::types::relationship_requests::AssertRelationshipRequest {
                source: created_resource.id,
                target: temper_core::types::ids::ResourceId::from(target),
                edge_kind: temper_core::types::graph::EdgeKind::LeadsTo,
                polarity: temper_core::types::graph::Polarity::Inverse,
                label: "derived_from".to_string(),
                weight: 1.0,
                act: act.clone(),
            };
            let outcome = runtime.block_on(async { client.relationships().assert(&req).await });
            match outcome {
                Ok(_) => asserted.push(target),
                Err(e) => {
                    output::warning(format!(
                        "could not assert derived_from edge to {target}: {e} \
                         (resource created; re-run `temper edge assert` — it is idempotent)"
                    ));
                    failed.push(target);
                }
            }
        }
        (asserted, failed)
    } else {
        (Vec::new(), Vec::new())
    };

    let result = CreateActionResult {
        status: "ok",
        resource: created_resource,
        edges_asserted,
        edges_failed,
    };
```

⚠️ `resolved_sources` is moved into the `BodyUpdate` at `resource.rs:315`. Clone it before that (`let sources_for_edges = resolved_sources.clone();`) and select from the clone. `act` is an `ActInput` and derives `Clone`, but it is consumed by `act.into_act_context()?` at `:329` — clone it before that line too. Verify `runtime.block_on` is the right call by reading how `link_session_to_task` drives the client; copy that idiom exactly rather than inventing one. The `(EdgeKind::LeadsTo, Polarity::Inverse, "derived_from")` triple comes from `EdgeType::DerivedFrom.legacy_mapping()` (`graph.rs:64`) — prefer calling `legacy_mapping()` over restating the triple if it is reachable from the CLI.

- [ ] **Step 7: Verify**

```bash
cargo nextest run -p temper-cli -E 'test(source_edge_targets)'
cargo nextest run -p temper-cli
cargo make check
```
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src
git commit -m "$(cat <<'EOF'
feat(cli): --sources-as-edges asserts derived_from per resource source

Citing N sources on create required N separate `edge assert` calls to make the
provenance visible as graph edges. The opt-in flag asserts one derived_from edge
per resource-valued source, skipping remote URLs and event ids (no edge target).

Not atomic, and deliberately non-fatal: the create has committed and is NOT
idempotent, so erroring would push authors toward duplicating the node.
relationship_assert IS idempotent, so a failed edge is safely re-assertable.
Successes and failures are reported in edges_asserted / edges_failed.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 13: `invocation show` reports a derived `disposition`

`kb_invocations` has no `disposition` column — the close function writes the disposition **into `status`** (`canonical_functions.sql:1285-1290`). `InvocationCloseAck` already carries it; only `InvocationView` does not. Derive it rather than denormalizing a column.

**Files:**
- Modify: `crates/temper-core/src/types/invocation.rs:23-26` (`TryFrom`), `:64-87` (the field)
- Modify: `crates/temper-services/src/backend/substrate_read.rs:666-676`

**Interfaces:**
- Produces: `impl TryFrom<&str> for Disposition`, `InvocationView.disposition: Option<Disposition>`

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-core/src/types/invocation.rs`'s `mod tests` (create one if absent):

```rust
#[cfg(test)]
mod disposition_tests {
    use super::*;

    #[test]
    fn disposition_parses_every_terminal_status() {
        assert_eq!(Disposition::try_from("completed").unwrap(), Disposition::Completed);
        assert_eq!(Disposition::try_from("failed").unwrap(), Disposition::Failed);
        assert_eq!(Disposition::try_from("abandoned").unwrap(), Disposition::Abandoned);
    }

    #[test]
    fn disposition_rejects_open_and_unknown() {
        // `open` is not a disposition — it is the absence of one.
        assert!(Disposition::try_from("open").is_err());
        // An unknown status means the DB CHECK was violated: escalate, never silently degrade.
        assert!(Disposition::try_from("cancelled").is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo nextest run -p temper-core -E 'test(disposition_)'
```
Expected: FAIL — `TryFrom<&str> is not implemented for Disposition`.

- [ ] **Step 3: Implement `TryFrom`**

In `crates/temper-core/src/types/invocation.rs`, after the `Disposition` enum:

```rust
impl TryFrom<&str> for Disposition {
    type Error = String;

    /// Parse a terminal `kb_invocations.status` into its disposition.
    ///
    /// `open` and any unknown value are errors, not `None`: the column's CHECK
    /// constraint admits exactly `open|completed|failed|abandoned`, so an unparseable
    /// terminal status means an invariant broke and must be loud. Callers map `open`
    /// to `None` before calling.
    fn try_from(status: &str) -> Result<Self, Self::Error> {
        match status {
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(format!(
                "not a terminal disposition: `{other}` (expected completed|failed|abandoned)"
            )),
        }
    }
}
```

- [ ] **Step 4: Add the field**

`crates/temper-core/src/types/invocation.rs`, in `InvocationView` right after `status`:

```rust
    /// The terminal disposition, derived from `status`. `None` while the invocation is open.
    ///
    /// There is no `disposition` column: `invocation close --disposition X` writes X into
    /// `status`. Surfacing it under its own name here makes "did the close take?" answerable
    /// without knowing that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<Disposition>,
```

- [ ] **Step 5: Derive it at readback**

`crates/temper-services/src/backend/substrate_read.rs:666-676` — inside `Ok(Some(InvocationView { ... }))`, add after `status`:

```rust
        disposition: match row.status.as_str() {
            "open" => None,
            terminal => Some(
                Disposition::try_from(terminal).map_err(|e| ApiError::Internal(e.to_string()))?,
            ),
        },
        status: row.status,
```

⚠️ `row.status` is moved by `status: row.status`. Compute `disposition` **before** that line (as written above), or clone. Match `ApiError`'s real internal-error constructor by reading a neighbouring `map_err` in the same file — do not guess `ApiError::Internal` if it is named otherwise. Add `Disposition` to the file's `temper_core::types::invocation::{...}` import at `:32`.

- [ ] **Step 6: Verify, including the required adjacent target**

```bash
cargo nextest run -p temper-core -E 'test(disposition_)'
cargo make docker-up
cargo nextest run -p temper-api --features test-db --test invocation_handler_test
cargo make check
```
Confirm the target name first: `ls crates/temper-api/tests/`. Run whichever integration target covers invocations.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/types/invocation.rs crates/temper-services/src/backend/substrate_read.rs
git commit -m "$(cat <<'EOF'
feat(core): invocation show reports a derived disposition

`invocation close --disposition completed` then `invocation show` reported
`disposition: None` — because kb_invocations has no disposition column. The close
function writes the disposition INTO status. Nothing was broken; nothing existed.

Derives Option<Disposition> from status on InvocationView (None while open) via a
TryFrom<&str> that propagates an unknown status as an error rather than degrading
to None — the DB CHECK admits four values, so an unknown one means an invariant
broke and must be loud.

No migration: two columns that must never disagree, with no reader needing it.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 14: the `cancelled` papercut

`--disposition cancelled` produces clap's bare `[possible values: completed, failed, abandoned]`. No alias is added — two words mapping to one disposition muddies a vocabulary the schema `CHECK`, the SQL function, and the core enum all agree on. Say which value to reach for instead.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:1073-1074`

- [ ] **Step 1: Add the long_help**

Replace the `disposition` arg on `InvocationCmd::Close` (`cli.rs:1072-1074`):

```rust
        /// Terminal disposition: completed | failed | abandoned.
        #[arg(
            long,
            value_enum,
            long_help = "Terminal disposition for the invocation.\n\n\
                         completed  — the run achieved its purpose\n\
                         failed     — the run errored or produced an unusable result\n\
                         abandoned  — the run was cancelled, aborted, or superseded\n\n\
                         There is no `cancelled` value: use `abandoned`."
        )]
        disposition: DispositionArg,
```

- [ ] **Step 2: Verify the help renders**

```bash
cargo run -p temper-cli --bin temper -- invocation close --help
```
Expected: the long_help block appears, naming `abandoned` for cancelled runs.

- [ ] **Step 3: Check and commit**

```bash
cargo make check
git add crates/temper-cli/src/cli.rs
git commit -m "$(cat <<'EOF'
docs(cli): --disposition long_help names `abandoned` for cancelled runs

`cancelled` is a natural word to try and produced a bare clap "invalid value".
No alias is added — two words for one disposition would muddy a vocabulary the
schema CHECK, the SQL close function, and the core enum all agree on. The help
now says which value to reach for.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 15: stop the skill content from lying

This is the deliverable, not a footnote. The issue was filed by a competent agent with the skill content loaded; three of its five items are direct consequences of false statements in one file. The agent behaved correctly given what it was told.

**Files:**
- Modify: `crates/temper-cli/skill-content/cognitive-maps.md:96-101`, `:126`, `:229`
- Modify: `crates/temper-cli/skill-content/reference.md`
- Modify: `crates/temper-mcp/src/tools/facets.rs`, `crates/temper-mcp/src/tools/cognitive_maps.rs:265`
- Modify: `packages/agent-workflows/steward/agent/skills/map-stewardship.md`

- [ ] **Step 1: Fix the authored-4 paragraph**

`crates/temper-cli/skill-content/cognitive-maps.md:96-101` currently reads:

> The **authored-4** are `create_resource` · `assert_relationship` · `facet_set` ·
> `fold_relationship`. On the CLI these are `resource create --cogmap`, `edge assert`, and
> `edge fold`; **`facet_set` is agent-surface only** (the `facet_set` MCP tool) — use it for a
> node's *semantic* properties (a resolved question, a stance), never for provenance.
> **Materialize** (recompute regions) is likewise agent-surface: `cogmap_materialize` /
> `cogmap_materialize_delta`. Regions only exist *after* a materialize.

Both bolded claims are false (`facet_set`) or now false (materialize). Replace with:

```markdown
The **authored-4** are `create_resource` · `assert_relationship` · `facet_set` ·
`fold_relationship`. Every one of them is on the CLI: `resource create --cogmap`,
`edge assert`, `resource facet`, and `edge fold`. Use `resource facet` for a node's
*semantic* properties (a resolved question, a stance), never for provenance —
provenance is stamped for you in `managed_meta`.

**Materialize** (recompute regions) is `temper cogmap materialize <MAP> [--threshold N]`
on the CLI, or `cogmap_materialize` / `cogmap_materialize_delta` on the agent surface.
Regions only exist *after* a materialize.
```

- [ ] **Step 2: Fix the `--sources` parenthetical**

`cognitive-maps.md:125-128` currently implies `--sources` creates the edges:

> When **two sources both assert one concept**, distill **one** node that cites **both** in
> `--sources` (and one `derived_from` edge per source) — not two near-duplicate nodes.

Replace the parenthetical:

```markdown
When **two sources both assert one concept**, distill **one** node that cites **both** in
`--sources` — not two near-duplicate nodes. `--sources` records *block provenance*, not
graph edges; pass `--sources-as-edges` to also assert one `derived_from` edge per
resource-valued source (remote URLs are skipped — they have no node to point at). Match
the source count to what the node honestly distills, not to its label (a `decision`
synthesized from two sources is fine).
```

- [ ] **Step 3: Make the worked example a single-surface script**

`cognitive-maps.md:227-229` ends the worked example with a comment telling the reader to leave the CLI:

> `#    then cogmap_materialize (MCP) on <MAP>`

Replace with the real verb so the example runs end-to-end as one script:

```bash
# 7. Close, then materialize so regions pick up the change.
temper invocation close "$inv" --disposition completed --outcome '{"nodes":1,"edges":1,"folds":1}'
temper cogmap materialize <MAP>
```

- [ ] **Step 4: List both verbs in the CLI reference**

`crates/temper-cli/skill-content/reference.md` mentions neither `facet` nor `materialize`. Add them to its command table, matching the file's existing row format:

```markdown
| `temper resource facet <ref> --values '<json>' [--weight N]` | Set a node's semantic facets |
| `temper cogmap materialize <ref> [--threshold N]` | Recompute a map's regions |
```

- [ ] **Step 5: Name the CLI equivalent in the MCP descriptions**

Only one MCP description anywhere names a CLI verb (`cognitive_maps.rs:170`). Add the same courtesy to the two tools this issue is about, so an agent on the MCP surface learns the CLI verb exists. Append one sentence to each tool's rmcp doc comment:

- `crates/temper-mcp/src/tools/facets.rs` (the `set_facet` tool): `/// CLI equivalent: `temper resource facet <ref> --values '<json>'`.`
- `crates/temper-mcp/src/tools/cognitive_maps.rs:265` (the `cogmap_materialize` tool): `/// CLI equivalent: `temper cogmap materialize <ref> [--threshold N]`.`

- [ ] **Step 6: Correct the steward's copy of the authored-4 claim**

`packages/agent-workflows/steward/agent/skills/map-stewardship.md` — apply the Step 1 correction wherever it restates that `facet_set` or materialize is agent-surface-only. (The *manual-trio* passage was already fixed in PR (b) — do not touch it again.)

```bash
rg -n "agent-surface|facet_set is|likewise agent-surface" packages/agent-workflows crates/temper-cli/skill-content
```
Expected after this task: **no hits** claiming CLI absence.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/skill-content crates/temper-mcp/src/tools packages/agent-workflows
git commit -m "$(cat <<'EOF'
docs(skill): stop telling agents the CLI verbs don't exist

Three of issue #330's five items were caused by this file:

  :98  "facet_set is agent-surface only" -> it is `temper resource facet`
  :100 "materialize is likewise agent-surface" -> now `temper cogmap materialize`
  :126 "--sources (and one derived_from edge per source)" -> --sources never
       created an edge; --sources-as-edges does

The filing agent behaved correctly given what it was told. Also makes the worked
example a runnable single-surface script, lists both verbs in reference.md, and
names the CLI equivalent in the two MCP tool descriptions.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 16: the guard — pin every verb the skill content names

Prose cannot be type-checked, but its *referents* can. This is the class of error that produced the issue: a doc naming a CLI verb that does not exist (or, worse, denying one that does).

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (`mod tests`)

- [ ] **Step 1: Write the guard test**

```rust
    /// Every CLI verb the installable skill content names must resolve against the clap
    /// command tree.
    ///
    /// Issue #330 was filed because `skill-content/cognitive-maps.md` told an agent that
    /// `facet_set` was "agent-surface only" when `temper resource facet` had existed all
    /// along. Prose cannot be type-checked; its referents can. If a verb is renamed or
    /// removed, this fails and points at the doc that now lies.
    #[test]
    fn every_verb_named_by_the_skill_content_resolves() {
        use clap::CommandFactory;

        // The verb paths asserted by crates/temper-cli/skill-content/*.md.
        const DOCUMENTED_VERBS: &[&[&str]] = &[
            &["resource", "create"],
            &["resource", "show"],
            &["resource", "update"],
            &["resource", "facet"],
            &["edge", "assert"],
            &["edge", "fold"],
            &["cogmap", "materialize"],
            &["cogmap", "shape"],
            &["invocation", "open"],
            &["invocation", "close"],
            &["invocation", "show"],
            &["search"],
        ];

        let root = Cli::command();
        for path in DOCUMENTED_VERBS {
            let mut node = &root;
            for (depth, segment) in path.iter().enumerate() {
                node = node.find_subcommand(segment).unwrap_or_else(|| {
                    panic!(
                        "skill content names `temper {}`, but `{segment}` does not resolve \
                         (depth {depth}). Either restore the verb or fix the docs.",
                        path.join(" ")
                    )
                });
            }
        }
    }
```

- [ ] **Step 2: Run it**

```bash
cargo nextest run -p temper-cli -E 'test(every_verb_named_by_the_skill_content_resolves)'
```
Expected: PASS (Task 11 added `cogmap materialize`; `resource facet` already existed). If `edge fold` or any other path fails, the doc names a verb that does not exist — fix the doc, and report it, because that is another instance of this issue's root cause.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/cli.rs
git commit -m "$(cat <<'EOF'
test(cli): pin every CLI verb the skill content names

Introspects the clap command tree and asserts each verb the installable skill
content references actually resolves. Does not verify prose claims — nothing
cheap does — but pins the existence claims the prose depends on, which is exactly
the class of error that produced #330.

Refs #330

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 17: PR (c) prep

- [ ] **Step 1: Targeted verification**

```bash
cargo make check
cargo nextest run -p temper-cli -p temper-core
cargo make docker-up
cargo nextest run -p temper-api --features test-db --test invocation_handler_test
cargo build -p temper-cli --bin temper
```
Expected: green. Do **not** run `cargo make test-all`.

- [ ] **Step 2: Dogfood the fix**

The issue exists because a real authoring pass hit friction. Prove the friction is gone — run the corrected worked example from `cognitive-maps.md` end to end against a scratch cogmap, entirely through the CLI, parsing every response with a single `json.loads`. If any step still requires MCP or a streaming decoder, the PR is not done.

- [ ] **Step 3: Push and open the PR**

```bash
git push -u origin jct/330c-authoring-ergonomics
gh pr create --base main --title "feat(cli): cogmap materialize, --sources-as-edges, derived disposition — and stop the skill content lying" --body "$(cat <<'EOF'
## What

PR 3 of 3 for #330.

- `temper cogmap materialize <ref> [--threshold N]` — the one genuinely missing verb. CLI-layer only; every layer beneath it already existed.
- `--sources-as-edges` on `resource create` — asserts one `derived_from` edge per resource-valued source, skipping remote URLs and event ids.
- `invocation show` reports a derived `disposition` (`None` while open).
- `--disposition` long_help names `abandoned` for cancelled runs.
- **The skill content stops lying**, plus a test that pins every CLI verb it names.

## Why the docs are the headline

The issue was filed by a competent agent with the skill content loaded. Three of its five items are direct consequences of false statements in `skill-content/cognitive-maps.md`:

| Line | Claim | Reality |
|------|-------|---------|
| `:98` | "`facet_set` is agent-surface only" | it is `temper resource facet` |
| `:100` | "materialize is likewise agent-surface" | now `temper cogmap materialize` |
| `:126` | "`--sources` (and one `derived_from` edge per source)" | `--sources` never created an edge |

The agent didn't fail to discover `resource facet` — it was told the verb didn't exist and reached for MCP as instructed. `every_verb_named_by_the_skill_content_resolves` makes that class of error a test failure.

## Notes

- **`--sources-as-edges` is not atomic, and deliberately non-fatal.** The create has committed and is *not* idempotent (dedup retired, #219), so erroring would push authors toward duplicating the node. `relationship_assert` *is* idempotent, so a failed edge is safely re-assertable. Successes/failures surface as `edges_asserted` / `edges_failed`.
- **No `disposition` column.** `kb_invocations` never had one — `close` writes the disposition into `status`. Deriving it beats denormalizing two columns that must never disagree. An unknown status escalates rather than degrading to `None`.
- **No `cancelled` alias.** Two words for one disposition would muddy a vocabulary the schema CHECK, the SQL function, and the core enum all agree on.

Design: `docs/superpowers/specs/2026-07-09-issue-330-cli-authoring-ergonomics-design.md`

Closes #330

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Report CI status.** Wait for CI, report the URL. **Do not merge.**

---

## Self-Review

**Spec coverage.** Every acceptance criterion maps to a task:

| Spec criterion | Task |
|---|---|
| One JSON doc per invocation; `search` object; `--edges` folded; consistent `id` | 1, 2, 3 |
| `cogmap materialize` exists; `resource facet` documented | 11, 15 |
| Provenance trio stamped server-side, fill-missing, every surface | 5, 6 |
| Full `show` has both tiers; `--meta-only` a strict subset; hashes deleted | 7, 8, 9 |
| `--sources-as-edges`; derived `disposition`; friendlier `--disposition` help | 12, 13, 14 |
| Every agent-facing surface tells the truth; verb-existence guard | 15, 16 |

**Divergences from the spec, both deliberate and flagged inline:**
1. Task 12 warns instead of erroring on partial edge failure (create is not idempotent; edge assert is).
2. Task 5 uses the real `ActContext` shape (`act.authorship.and_then(|a| a.model)`), not the spec's non-existent `act.model`; and `cmd.managed_meta` is not an `Option`.

**Type consistency.** `stamp_provenance` (5) → called in 6. `ResourceDetail` (7) → consumed by 8's subset test and 9's e2e. `source_edge_targets` (12) → used in 12's tail. `Disposition::try_from` (13) → used at `substrate_read.rs:666`. `SearchResultsResponse` (1) is `pub(crate)` in `commands::search_cmd` and referenced from `actions::search`'s test by full path.

**Known soft spots**, called out rather than hidden: the e2e harness API in Task 9 must be adapted to `tests/e2e/tests/common/mod.rs` (the assertions are the contract, not the harness calls); `ApiError`'s internal-error constructor in Task 13 must be read from a neighbour rather than guessed; the `temper-api` invocation integration target name must be confirmed with `ls crates/temper-api/tests/`.
