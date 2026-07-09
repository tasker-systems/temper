# MCP Segmented Ingest Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the MCP surface streaming, resumable, multi-block ingest — `ingest_begin` → N × `ingest_append` → `ingest_finalize`, with the server chunking segments the caller cannot chunk itself.

**Architecture:** Integrity moves to where a non-chunking caller has the information: `append` verifies `sha256(content) == content_hash` per segment (retiring two dead wire fields), and `BlocksResponse` carries a `body_hash` the caller echoes back at finalize. `resource_finalize`'s SQL and every migration are untouched. Two fixes this beat's caller exposes ride along: begin's three-call composition hoists out of the HTTP handler into `Backend::begin_segmented_ingest`, and the hardcoded `Surface::ApiHttp` emitter on append/finalize/list_blocks becomes a threaded parameter.

**Tech Stack:** Rust (axum, rmcp, sqlx, ort/ONNX), PostgreSQL 18 + pgvector, cargo-make, cargo-nextest.

**Spec:** [docs/superpowers/specs/2026-07-09-mcp-segmented-ingest-design.md](../specs/2026-07-09-mcp-segmented-ingest-design.md)

## Global Constraints

- Branch is `jct/mcp-segmented-ingest`, already created, spec committed at `ea6f5e60`.
- **No new migrations.** `resource_finalize` and `block_append` SQL are untouched. If you find yourself writing a migration, stop and escalate — the design depends on not needing one.
- Run `cargo make check` before every commit. A failing check on files you did not touch is a scope-creep signal — escalate, do not "fix" it.
- SQL changes require cache regen, **in this order**: `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, then `cargo make prepare-api`. `cargo make` tasks force `SQLX_OFFLINE=true`, so `cargo make check` is the honest local probe of the committed caches.
- Never inline `sqlx::query!()` in a surface (handler / MCP tool / CLI action). Persistence lives in `temper-substrate` (`writes`/`readback`) and `temper-services`.
- Typed structs over `serde_json::json!()` for data with a known shape.
- Auth before writes, always. Every `DbBackend` write gates independently — do not remove a `check_can_modify_next` call because a caller "already checked."
- Do not soften a contract to make a test pass. If a passing test requires weakening an assertion, **STOP and report BLOCKED.**
- Full workspace test runs belong at branch end (Task 8), not per-task. Per-task: the focused test + that crate's suite.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/temper-ingest/src/chunk.rs` | (read-only) `chunk_markdown_with_prefix` already exists at :386 | 1 |
| `crates/temper-substrate/src/content.rs` | `prepare_block_with_prefix`, `prepare_block_deferred_with_prefix` | 1 |
| `crates/temper-substrate/src/readback/mod.rs` | `trailing_breadcrumb` — the heading path a server-chunked segment resumes from | 1 |
| `crates/temper-core/src/hash.rs` | `sha256_hex` — bare-hex sha256, the twin of the CLI's private `sha256_hex_raw` | 2 |
| `crates/temper-core/src/types/ingest.rs` | `AppendBlockPayload.chunks_packed: Option`, `BlocksResponse.body_hash` | 2, 3 |
| `crates/temper-services/src/backend/db_backend.rs` | append verification, server-chunk branch, `Surface` threading, `begin_segmented_ingest` | 2–5 |
| `crates/temper-workflow/src/operations/backend.rs` | `Backend` trait signatures | 4, 5 |
| `crates/temper-api/src/handlers/ingest.rs` | begin collapses to one dispatch | 5 |
| `crates/temper-api/src/handlers/segments.rs` | pass `Surface::ApiHttp` | 4 |
| `crates/temper-mcp/src/tools/resources.rs` | extract `build_create_command` for reuse by `ingest_begin` | 6 |
| `crates/temper-mcp/src/tools/ingest.rs` | **new** — the four tool handlers | 6 |
| `crates/temper-mcp/src/service.rs` | `#[tool(...)]` registration | 6 |
| `tests/e2e/tests/mcp_segmented_ingest_test.rs` | **new** — the equivalence + resume assertions | 7 |

---

### Task 1: Breadcrumb-carrying block preparation

A server-chunked segment that begins mid-section must still carry its ancestor heading path. `temper_ingest::chunk::chunk_markdown_with_prefix(text, initial_breadcrumb)` already exists (`crates/temper-ingest/src/chunk.rs:386`) and is byte-identical to `chunk_markdown` when the breadcrumb is empty. This task lifts it into the substrate's block layer and adds the readback that supplies the breadcrumb.

**Files:**
- Modify: `crates/temper-substrate/src/content.rs` (near `plan_chunks` at :70, `prepare_block` at :142, `prepare_block_deferred` at :195)
- Modify: `crates/temper-substrate/src/readback/mod.rs`
- Test: `crates/temper-substrate/src/content.rs` (the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `temper_ingest::chunk::chunk_markdown_with_prefix(text: &str, initial_breadcrumb: &[String]) -> Vec<ChunkData>`
- Produces:
  - `content::prepare_block_with_prefix(seq: i32, role: Option<&str>, prose: &str, breadcrumb: &[String]) -> Result<PreparedBlock>`
  - `content::prepare_block_deferred_with_prefix(seq: i32, role: Option<&str>, prose: &str, breadcrumb: &[String]) -> PreparedBlock`
  - `readback::trailing_breadcrumb(pool: &PgPool, resource: ResourceId) -> Result<Vec<String>>`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/temper-substrate/src/content.rs`:

```rust
#[test]
fn prefix_variants_with_empty_breadcrumb_match_the_originals() {
    // The no-regression guard: every existing single-block caller must be bit-for-bit
    // unaffected by the new parameter.
    let prose = "# Title\n\nalpha\n\n## Section\n\nbeta\n";
    let plain = prepare_block_deferred(0, None, prose);
    let prefixed = prepare_block_deferred_with_prefix(0, None, prose, &[]);

    assert_eq!(plain.chunks.len(), prefixed.chunks.len());
    for (a, b) in plain.chunks.iter().zip(prefixed.chunks.iter()) {
        assert_eq!(a.chunk_index, b.chunk_index);
        assert_eq!(a.content_hash, b.content_hash);
        assert_eq!(a.content, b.content);
        assert_eq!(a.header_path, b.header_path);
        assert_eq!(a.heading_depth, b.heading_depth);
    }
}

#[test]
fn prefix_seeds_ancestor_breadcrumb_for_a_mid_section_segment() {
    // A segment cut mid-section carries no heading of its own; without the prefix its chunks
    // would land with a NULL header_path, breaking search breadcrumbs across block boundaries.
    let mid_section_prose = "beta continues here\n";
    let block = prepare_block_deferred_with_prefix(
        1,
        None,
        mid_section_prose,
        &["Title".to_owned(), "Section".to_owned()],
    );

    assert_eq!(block.chunks.len(), 1);
    assert_eq!(
        block.chunks[0].header_path.as_deref(),
        Some("Title > Section"),
        "a mid-section segment must inherit its ancestor path"
    );
}

#[test]
fn prefix_variant_merkle_matches_whole_document_chunking() {
    // Splitting a document at a heading and prefixing the tail must reproduce the same chunk
    // content hashes as chunking the whole document in one pass.
    let whole = "# Title\n\nalpha\n\n## Section\n\nbeta\n";
    let head = "# Title\n\nalpha\n";
    let tail = "## Section\n\nbeta\n";

    let one_pass = prepare_block_deferred(0, None, whole);
    let b0 = prepare_block_deferred_with_prefix(0, None, head, &[]);
    let b1 = prepare_block_deferred_with_prefix(1, None, tail, &["Title".to_owned()]);

    let one_pass_hashes: Vec<&str> = one_pass.chunks.iter().map(|c| c.content_hash.as_str()).collect();
    let split_hashes: Vec<&str> = b0
        .chunks
        .iter()
        .chain(b1.chunks.iter())
        .map(|c| c.content_hash.as_str())
        .collect();

    assert_eq!(one_pass_hashes, split_hashes);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-substrate -E 'test(prefix)'
```

Expected: FAIL — `cannot find function prepare_block_deferred_with_prefix in this scope`.

- [ ] **Step 3: Implement the prefix-aware chunk planner and block builders**

In `crates/temper-substrate/src/content.rs`, generalize the existing private `plan_chunks` (currently at :70) rather than duplicating it:

```rust
/// Plan a block's chunks, seeding the chunker's heading-breadcrumb stack from `breadcrumb`.
/// An empty `breadcrumb` is byte-identical to whole-document chunking — that equivalence is the
/// contract the streaming segment boundary rests on.
fn plan_chunks_with_prefix(
    prose: &str,
    breadcrumb: &[String],
) -> Vec<(i32, String, String, String, u8)> {
    temper_ingest::chunk::chunk_markdown_with_prefix(prose, breadcrumb)
        .into_iter()
        .map(|c| {
            (
                c.chunk_index as i32,
                c.content_hash,
                c.content,
                c.header_path,
                c.heading_depth,
            )
        })
        .collect()
}

fn plan_chunks(prose: &str) -> Vec<(i32, String, String, String, u8)> {
    plan_chunks_with_prefix(prose, &[])
}
```

Now rewrite `prepare_block` and `prepare_block_deferred` as thin delegations, so the heading-mapping rule (`heading_depth == 0 || header_path.is_empty()` ⇒ NULL columns) exists in exactly one place per variant:

```rust
/// Prepare one block: chunk its prose, then embed every chunk in a single batched ONNX call.
pub fn prepare_block(seq: i32, role: Option<&str>, prose: &str) -> Result<PreparedBlock> {
    prepare_block_with_prefix(seq, role, prose, &[])
}

/// [`prepare_block`], seeding the chunker's heading breadcrumb from `breadcrumb` so a segment
/// beginning mid-section still carries its ancestor `header_path`. With an empty `breadcrumb`
/// this is byte-identical to [`prepare_block`].
pub fn prepare_block_with_prefix(
    seq: i32,
    role: Option<&str>,
    prose: &str,
    breadcrumb: &[String],
) -> Result<PreparedBlock> {
    let planned = plan_chunks_with_prefix(prose, breadcrumb);
    let texts: Vec<&str> = planned
        .iter()
        .map(|(_, _, content, _, _)| content.as_str())
        .collect();
    // Empty prose ⇒ no chunks ⇒ no embedding call (embed_texts on an empty slice is undefined).
    let embeddings = if texts.is_empty() {
        Vec::new()
    } else {
        temper_ingest::embed::embed_texts(&texts).context("embed_texts (bge-768) failed")?
    };
    let chunks = planned
        .into_iter()
        .zip(embeddings)
        .map(|((chunk_index, content_hash, content, header_path, heading_depth), embedding)| {
            let (header_path, heading_depth) = map_heading(header_path, heading_depth);
            PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index,
                content_hash,
                content,
                embedding: Some(embedding),
                header_path,
                heading_depth,
            }
        })
        .collect();
    Ok(PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq,
        role: role.map(str::to_owned),
        chunks,
        incorporated: Vec::new(),
    })
}

/// The one heading rule, shared by every block builder: depth 0 or an empty breadcrumb means an
/// unheaded preamble, which persists as NULL columns.
fn map_heading(header_path: String, heading_depth: u8) -> (Option<String>, Option<i16>) {
    if heading_depth == 0 || header_path.is_empty() {
        (None, None)
    } else {
        (Some(header_path), Some(heading_depth as i16))
    }
}

pub fn prepare_block_deferred(seq: i32, role: Option<&str>, prose: &str) -> PreparedBlock {
    prepare_block_deferred_with_prefix(seq, role, prose, &[])
}

/// [`prepare_block_deferred`] with a seeded heading breadcrumb. ONNX-free: chunks land with a
/// NULL vector, backfilled off-request by the embed drain (issue #299).
pub fn prepare_block_deferred_with_prefix(
    seq: i32,
    role: Option<&str>,
    prose: &str,
    breadcrumb: &[String],
) -> PreparedBlock {
    let chunks = plan_chunks_with_prefix(prose, breadcrumb)
        .into_iter()
        .map(|(chunk_index, content_hash, content, header_path, heading_depth)| {
            let (header_path, heading_depth) = map_heading(header_path, heading_depth);
            PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index,
                content_hash,
                content,
                embedding: None,
                header_path,
                heading_depth,
            }
        })
        .collect();
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq,
        role: role.map(str::to_owned),
        chunks,
        incorporated: Vec::new(),
    }
}
```

Note `prepare_block_from_chunks` (:107) has its own copy of the heading rule inline. Replace that copy with a `map_heading(c.header_path, c.heading_depth as u8)` call so all three builders share it.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-substrate -E 'test(prefix)'
```

Expected: PASS, 3 tests.

- [ ] **Step 5: Add the `trailing_breadcrumb` readback**

In `crates/temper-substrate/src/readback/mod.rs`:

```rust
/// The heading breadcrumb a newly-appended segment resumes from: the `header_path` of the last
/// chunk of the highest-`seq` live block. A server-chunked segment seeds its chunker with this so
/// its chunks carry ancestor headings across the block boundary (see
/// `content::prepare_block_with_prefix`). An empty vec when the resource has no live blocks, or
/// its trailing chunk is an unheaded preamble (NULL `header_path`).
pub async fn trailing_breadcrumb(pool: &PgPool, resource: ResourceId) -> Result<Vec<String>> {
    let path: Option<String> = sqlx::query_scalar!(
        r#"
        SELECT c.header_path
          FROM kb_content_blocks b
          JOIN kb_chunks c ON c.block_id = b.id AND c.is_current
         WHERE b.resource_id = $1 AND NOT b.is_folded
         ORDER BY b.seq DESC, c.chunk_index DESC
         LIMIT 1
        "#,
        resource.uuid(),
    )
    .fetch_optional(pool)
    .await?
    .flatten();

    Ok(path
        .map(|p| p.split(" > ").map(str::to_owned).collect())
        .unwrap_or_default())
}
```

Verify `kb_chunks` really has an `is_current` column and that `header_path` is nullable before trusting this query — `psql "$DATABASE_URL" -c '\d kb_chunks'`. If `is_current` does not exist, drop that predicate rather than inventing one.

- [ ] **Step 6: Regenerate the sqlx cache and check**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make check
```

Expected: clean. `cargo make check` runs offline against the committed cache, so a missing entry fails here.

- [ ] **Step 7: Run the substrate suite**

```bash
cargo nextest run -p temper-substrate
```

Expected: PASS. The pure-core tests are ungated; scenario write-path tests need `artifact-tests` and are not in scope here.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-substrate/src/content.rs crates/temper-substrate/src/readback/mod.rs .sqlx
git commit -m "feat(substrate): breadcrumb-carrying block preparation + trailing_breadcrumb readback"
```

---

### Task 2: Append verifies its declared content hash

`AppendBlockPayload.content` and `.content_hash` are dead: `db_backend.rs:2214-2216` reads only `chunks_packed` and `seq`. This task makes them the integrity contract. `chunks_packed` stays a required `String` — Task 3 makes it optional.

The CLI already sends the correct values (`crates/temper-cli/src/actions/ingest.rs:314-315`), so it gains a real check for free. Four test files pass the placeholder `"unused-client-text-hash"` and **must be updated to real hashes** — that update is what proves the check bites.

**Files:**
- Modify: `crates/temper-core/src/hash.rs`
- Modify: `crates/temper-core/src/types/ingest.rs` (`AppendBlockPayload` :121, `BlocksResponse` :136)
- Modify: `crates/temper-cli/src/actions/ingest.rs` (:87 private `sha256_hex_raw`)
- Modify: `crates/temper-services/src/backend/db_backend.rs` (`landed_segments` :381, `append_block` :2198, `list_blocks` :2266)
- Test: `crates/temper-services/tests/segmented_backend_test.rs`, `crates/temper-api/tests/segments_handler_test.rs`, `crates/temper-client/tests/segments_client_test.rs`, `tests/e2e/tests/streaming_ingest_test.rs`

**Interfaces:**
- Produces:
  - `temper_core::hash::sha256_hex(bytes: &[u8]) -> String` — bare lowercase hex, no `sha256:` prefix
  - `BlocksResponse { blocks: Vec<SegmentInfo>, body_hash: String }`
  - `DbBackend::landed_blocks(pool: &PgPool, resource: ResourceId) -> Result<BlocksResponse, TemperError>` (replaces the private `landed_segments`)

- [ ] **Step 1: Write the failing test**

In `crates/temper-services/tests/segmented_backend_test.rs`, add:

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn append_rejects_a_content_hash_that_does_not_match_content(pool: PgPool) {
    let (backend, created) = seed_segmented_resource(&pool).await;

    let err = backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: "second segment".to_string(),
                content_hash: "deadbeef".to_string(), // not sha256("second segment")
                chunks_packed: one_chunk_packed("second segment", "bb"),
            },
        )
        .await
        .expect_err("a mismatched content_hash must be rejected");

    assert!(
        matches!(err, TemperError::BadRequest(ref m) if m.contains("content_hash")),
        "expected BadRequest naming content_hash, got {err:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn append_returns_the_live_body_hash(pool: PgPool) {
    let (backend, created) = seed_segmented_resource(&pool).await;

    let text = "second segment";
    let out = backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: text.to_string(),
                content_hash: temper_core::hash::sha256_hex(text.as_bytes()),
                chunks_packed: one_chunk_packed(text, "bb"),
            },
        )
        .await
        .expect("append with a correct hash succeeds");

    let stored: String = sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
        .bind(created.id.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        out.value.body_hash, stored,
        "BlocksResponse.body_hash must be the value finalize will compare against"
    );
}
```

`seed_segmented_resource` is the existing helper in that file — if it is inlined rather than extracted, extract it from the current `append_lands_a_second_block`-style test first, returning `(DbBackend, ResourceRow)`.

- [ ] **Step 2: Run to verify it fails**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make docker-up
cargo nextest run -p temper-services --features test-db -E 'test(append_rejects) or test(append_returns_the_live_body_hash)'
```

Expected: FAIL — `no function sha256_hex`, and `no field body_hash on BlocksResponse`.

- [ ] **Step 3: Add the shared hash helper**

In `crates/temper-core/src/hash.rs`, beside `compute_body_hash`:

```rust
/// Bare lowercase-hex SHA-256 of raw bytes — **no `sha256:` prefix**, unlike
/// [`compute_body_hash`]. This is the segment-text identity hash the ingest wire carries in
/// `AppendBlockPayload.content_hash`, and the Rust twin of Postgres's
/// `encode(sha256(convert_to(s, 'UTF8')), 'hex')`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
```

In `crates/temper-cli/src/actions/ingest.rs`, delete the private `sha256_hex_raw` (:87-92) and replace its call sites (`:315` and any others — `grep -n sha256_hex_raw`) with `temper_core::hash::sha256_hex`.

- [ ] **Step 4: Add `body_hash` to `BlocksResponse`**

In `crates/temper-core/src/types/ingest.rs`:

```rust
/// Response to append / `GET /api/resources/{id}/blocks`: the currently landed segment set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct BlocksResponse {
    pub blocks: Vec<SegmentInfo>,
    /// The resource's live `kb_resources.body_hash` after the landed set — the value a caller
    /// echoes back as `FinalizePayload.expected_body_hash`. A caller that does not chunk locally
    /// cannot derive this merkle itself, so the server hands it over; finalize's comparison then
    /// asserts "nothing changed between my last append and now."
    pub body_hash: String,
}
```

Update the round-trip test in the same file's `mod tests` (`blocks_response_round_trips`) to construct and assert the new field.

- [ ] **Step 5: Verify the hash and return `body_hash` in the backend**

In `crates/temper-services/src/backend/db_backend.rs`, rename `landed_segments` to `landed_blocks` and have it return the whole `BlocksResponse`, reading `body_hash` in the same call:

```rust
    /// The landed (non-folded) segment set plus the resource's live `body_hash`. `SegmentInfo.
    /// content_hash` is the per-block merkle (`kb_block_revisions.block_body_hash`, latest
    /// revision per block) — see [`SegmentInfo`]'s doc for why that is NOT the hash the client
    /// sends inbound on append. Shared by `append_block` / `list_blocks` / `begin_segmented_ingest`
    /// so the "currently landed set" projection lives in exactly one place.
    async fn landed_blocks(
        pool: &PgPool,
        resource: ResourceId,
    ) -> Result<BlocksResponse, TemperError> {
        let rows = sqlx::query!(
            r#"
            SELECT b.seq AS "seq!", r.block_body_hash AS "block_body_hash!"
              FROM kb_content_blocks b
              JOIN LATERAL (
                     SELECT block_body_hash FROM kb_block_revisions
                      WHERE block_id = b.id ORDER BY created DESC LIMIT 1
                   ) r ON true
             WHERE b.resource_id = $1 AND NOT b.is_folded
             ORDER BY b.seq
            "#,
            resource.uuid(),
        )
        .fetch_all(pool)
        .await
        .map_err(api_err)?;

        let body_hash: String =
            sqlx::query_scalar!("SELECT body_hash FROM kb_resources WHERE id = $1", resource.uuid())
                .fetch_one(pool)
                .await
                .map_err(api_err)?;

        Ok(BlocksResponse {
            blocks: rows
                .into_iter()
                .map(|r| SegmentInfo {
                    seq: r.seq as u32,
                    content_hash: r.block_body_hash,
                })
                .collect(),
            body_hash,
        })
    }
```

If `kb_resources.body_hash` is nullable, `query_scalar!` yields `Option<String>`; in that case use `.ok_or_else(|| TemperError::BadRequest("resource has no body_hash".into()))?` rather than `unwrap`. Confirm with `psql "$DATABASE_URL" -c '\d kb_resources'`.

Add the verification helper and call it first in `append_block`:

```rust
/// The declared segment-text hash must match the segment text. This is the one integrity check a
/// caller that does not chunk locally can honor, so it is the check every caller honors.
fn verify_content_hash(payload: &AppendBlockPayload) -> Result<(), TemperError> {
    let actual = temper_core::hash::sha256_hex(payload.content.as_bytes());
    if actual != payload.content_hash {
        return Err(TemperError::BadRequest(format!(
            "content_hash mismatch for seq {}: declared {}, computed {}",
            payload.seq, payload.content_hash, actual
        )));
    }
    Ok(())
}
```

In `append_block`, immediately after the `check_can_modify_next` gate:

```rust
        verify_content_hash(&payload)?;
```

Then replace the two `Self::landed_segments(...)` call sites in `append_block` and `list_blocks` with `Self::landed_blocks(...)`, returning its `BlocksResponse` directly.

- [ ] **Step 6: Update the four test fixtures that pass a placeholder hash**

Every `AppendBlockPayload` literal that sets `content_hash: "unused-client-text-hash"` (or any non-hash string) must now compute the real value. The sites:

- `crates/temper-services/tests/segmented_backend_test.rs:104, :128, :219`
- `crates/temper-api/tests/segments_handler_test.rs:92, :196, :237`
- `crates/temper-client/tests/segments_client_test.rs:112`
- `tests/e2e/tests/streaming_ingest_test.rs:502, :545`

The mechanical edit at each, keeping the same `content`:

```rust
    let content = "second segment".to_string();
    let append_payload = AppendBlockPayload {
        seq: 1,
        content_hash: temper_core::hash::sha256_hex(content.as_bytes()),
        chunks_packed: one_chunk_packed(&content, "bb"),
        content,
    };
```

Delete the now-false comment at `segmented_backend_test.rs:107` ("Unused server-side … any value is accepted"). `segments_client_test.rs` is a wiremock test asserting the serialized request shape — its assertions on `content_hash` (if any) need the real value too; its stub responses must also grow a `body_hash` field or deserialization will fail.

- [ ] **Step 7: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(append_rejects) or test(append_returns_the_live_body_hash)'
cargo nextest run -p temper-services --features test-db
cargo nextest run -p temper-core
cargo nextest run -p temper-client --test segments_client_test
```

Expected: PASS throughout. If `segments_handler_test` fails on a hash you did not update, you missed a site — `grep -rn 'unused-client-text-hash' crates/ tests/` must return nothing.

- [ ] **Step 8: Regenerate caches and check**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make check
```

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(ingest): append verifies its declared content_hash; BlocksResponse carries body_hash

AppendBlockPayload.content/.content_hash were dead fields — the server read only
chunks_packed and seq. They become the per-segment transit-integrity contract, the
one check a caller that does not chunk locally can honor. BlocksResponse now carries
the live kb_resources.body_hash for echo-back at finalize."
```

---

### Task 3: `chunks_packed` becomes optional; the server chunks what the caller cannot

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs` (`AppendBlockPayload.chunks_packed`)
- Modify: `crates/temper-services/src/backend/db_backend.rs` (`append_block`)
- Modify: `crates/temper-cli/src/actions/ingest.rs` (wrap in `Some`)
- Test: `crates/temper-services/tests/segmented_backend_test.rs`

**Interfaces:**
- Consumes: `content::prepare_block_with_prefix`, `content::prepare_block_deferred_with_prefix`, `readback::trailing_breadcrumb` (Task 1); `verify_content_hash` (Task 2)
- Produces: `AppendBlockPayload.chunks_packed: Option<String>`

- [ ] **Step 1: Write the failing test**

In `crates/temper-services/tests/segmented_backend_test.rs`:

```rust
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn server_chunks_an_append_with_no_packed_chunks_and_carries_the_breadcrumb(pool: PgPool) {
    // Block 0 ends inside "## Section", so block 1's chunks must inherit "Title > Section".
    let (backend, created) = seed_resource_with_body(&pool, "# Title\n\nalpha\n\n## Section\n\nbeta\n").await;

    let text = "beta continues here\n";
    backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content_hash: temper_core::hash::sha256_hex(text.as_bytes()),
                content: text.to_string(),
                chunks_packed: None, // the MCP caller: no embedder, no chunker
            },
        )
        .await
        .expect("server-side chunking lands the block");

    let paths: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT c.header_path FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
          WHERE b.resource_id = $1 AND b.seq = 1 ORDER BY c.chunk_index",
    )
    .bind(created.id.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(!paths.is_empty(), "the appended block must have chunks");
    assert_eq!(
        paths[0].as_deref(),
        Some("Title > Section"),
        "a server-chunked segment inherits the prior block's trailing breadcrumb"
    );
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-services --features "test-db,test-embed" -E 'test(server_chunks_an_append)'
```

Expected: FAIL — `expected String, found Option`.

- [ ] **Step 3: Make the field optional**

In `crates/temper-core/src/types/ingest.rs`:

```rust
    /// Base64-encoded MessagePack of `Vec<PackedChunk>` — this segment's pre-chunked,
    /// pre-embedded content (same wire shape as `IngestPayload::chunks_packed`). `None` when the
    /// caller has no chunker or embedder (the MCP surface): the server then chunks `content`
    /// itself, seeding the heading breadcrumb from the prior block so `header_path` stays
    /// continuous across the boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
```

Add a round-trip test in that file's `mod tests`:

```rust
#[test]
fn append_payload_round_trips_without_packed_chunks() {
    let json = r#"{"seq":2,"content":"x","content_hash":"h"}"#;
    let p: AppendBlockPayload = serde_json::from_str(json).unwrap();
    assert!(p.chunks_packed.is_none());
    assert!(!serde_json::to_string(&p).unwrap().contains("chunks_packed"));
}
```

- [ ] **Step 4: Branch on it in `append_block`**

Replace the block-building lines in `db_backend.rs::append_block` (currently :2214-2216):

```rust
        // Bring-your-own chunks (CLI) ride through verbatim — ONNX-free, `seq` authoritative.
        // Absent chunks (MCP, steward) are chunked server-side, seeding the heading breadcrumb
        // from the prior block so `header_path` is continuous across the boundary. The embed
        // decision uses the create path's predicate verbatim: defer only for the server-computed
        // case, and only where the drain is enabled.
        let block = match payload.chunks_packed.as_deref() {
            Some(packed) => {
                let chunks = unpack_incoming_chunks(packed)?;
                temper_substrate::content::prepare_block_from_chunks(payload.seq as i32, None, chunks)
            }
            None => {
                let breadcrumb = temper_substrate::readback::trailing_breadcrumb(&self.pool, resource)
                    .await
                    .map_err(api_err)?;
                let defer = !payload.content.is_empty()
                    && crate::services::embed_service::async_embed_enabled();
                if defer {
                    temper_substrate::content::prepare_block_deferred_with_prefix(
                        payload.seq as i32,
                        None,
                        &payload.content,
                        &breadcrumb,
                    )
                } else {
                    temper_substrate::content::prepare_block_with_prefix(
                        payload.seq as i32,
                        None,
                        &payload.content,
                        &breadcrumb,
                    )
                    .map_err(api_err)?
                }
            }
        };
```

An empty `content` with no chunks would produce a zero-chunk block, which `block_append` rejects with a raw `empty chunk set` database exception (migration `…0012:48`). Reject it earlier, with a caller-legible message.

Rename Task 2's `verify_content_hash` to `validate_append`, add the emptiness guard, and **update its single call site in `append_block`** from `verify_content_hash(&payload)?` to `validate_append(&payload)?`:

```rust
/// Everything an append must satisfy before any write. The declared segment-text hash must match
/// the segment text — the one integrity check a caller that does not chunk locally can honor, so
/// it is the check every caller honors. And a server-chunked append needs prose to chunk.
fn validate_append(payload: &AppendBlockPayload) -> Result<(), TemperError> {
    let actual = temper_core::hash::sha256_hex(payload.content.as_bytes());
    if actual != payload.content_hash {
        return Err(TemperError::BadRequest(format!(
            "content_hash mismatch for seq {}: declared {}, computed {}",
            payload.seq, payload.content_hash, actual
        )));
    }
    if payload.chunks_packed.is_none() && payload.content.is_empty() {
        return Err(TemperError::BadRequest(
            "append with no chunks_packed requires non-empty content".to_owned(),
        ));
    }
    Ok(())
}
```

- [ ] **Step 5: Update the CLI call site**

`crates/temper-cli/src/actions/ingest.rs:312-316` — wrap the packed blob:

```rust
        let append_payload = AppendBlockPayload {
            seq: idx as u32,
            content: segments[idx].text.clone(),
            content_hash: temper_core::hash::sha256_hex(segments[idx].text.as_bytes()),
            chunks_packed: Some(packed),
        };
```

Fix any remaining `chunks_packed:` literals the compiler flags across `crates/` and `tests/e2e/`.

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-services --features "test-db,test-embed" -E 'test(server_chunks_an_append)'
cargo nextest run -p temper-services --features test-db
cargo nextest run -p temper-core
cargo nextest run -p temper-cli
```

Expected: PASS. The embed-gated test needs ONNX; if ONNX Runtime is not installed locally, note it and let the Embed CI job cover it — do **not** delete the test.

- [ ] **Step 7: Check and commit**

```bash
cargo make check
git add -A
git commit -m "feat(ingest): optional chunks_packed on append — server chunks with a carried breadcrumb"
```

---

### Task 4: Thread `Surface` through append / finalize / list_blocks

`db_backend.rs:2208` and `:2247` hardcode `surface_marker(Surface::ApiHttp)` while every other `Backend` write threads `cmd.origin`. Nothing notices today because the API is the only caller; the moment MCP appends, the block is attributed to the `web` emitter. Pre-existing bug, exposed by this beat's caller, bundled here per the repo's convention.

**Files:**
- Modify: `crates/temper-workflow/src/operations/backend.rs` (:181-203)
- Modify: `crates/temper-services/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/handlers/segments.rs`
- Modify: `crates/temper-cli/src/cloud_backend/backend.rs` (:307, :626 — the CLI's `Backend` impl arms)
- Test: `crates/temper-services/tests/segmented_backend_test.rs`

**Interfaces:**
- Produces:
  - `Backend::append_block(&self, resource: ResourceId, payload: AppendBlockPayload, origin: Surface)`
  - `Backend::finalize_ingest(&self, resource: ResourceId, payload: FinalizePayload, origin: Surface)`
  - `Backend::list_blocks(&self, resource: ResourceId)` — **unchanged**. It emits no event, so it has
    no emitter to attribute; a parameter nothing consumes would be a lie.

- [ ] **Step 1: Write the failing test**

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_mcp_append_is_attributed_to_the_mcp_emitter(pool: PgPool) {
    let (backend, created) = seed_segmented_resource(&pool).await;
    let text = "second segment";

    backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content_hash: temper_core::hash::sha256_hex(text.as_bytes()),
                content: text.to_string(),
                chunks_packed: Some(one_chunk_packed(text, "bb")),
            },
            Surface::Mcp,
        )
        .await
        .expect("append succeeds");

    // The block_created event's emitter must carry the mcp surface marker, not "web".
    let surface: String = sqlx::query_scalar(
        "SELECT e.surface FROM kb_events ev \
           JOIN kb_entities e ON e.id = ev.emitter_id \
          WHERE ev.event_type = 'block_created' \
          ORDER BY ev.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(surface, "mcp", "an MCP append must not be attributed to web");
}
```

Confirm the emitter/entity column names against the live schema before running: `psql "$DATABASE_URL" -c '\d kb_entities'` and `-c '\d kb_events'`. `resolve_emitter`'s own SQL in `crates/temper-substrate/src/writes.rs` is the authority for how the surface marker is stored — read it and mirror it. If the marker is not a `surface` column, adapt the assertion; do not adapt the production code to fit a guessed schema.

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(an_mcp_append_is_attributed)'
```

Expected: FAIL to compile — `append_block` takes 2 arguments, 3 supplied.

- [ ] **Step 3: Add the parameter to the trait**

In `crates/temper-workflow/src/operations/backend.rs`, add `origin: Surface` to all three signatures, documenting why:

```rust
    /// Append one segment to a resource whose block 0 already landed. Idempotent in the substrate
    /// on `(resource, seq, block merkle)`. `origin` attributes the emitted `block_created` event to
    /// the calling surface — it is a parameter rather than a constant because CLI, API, and MCP all
    /// reach this path.
    async fn append_block(
        &self,
        resource: ResourceId,
        payload: AppendBlockPayload,
        origin: Surface,
    ) -> Result<CommandOutput<BlocksResponse>, TemperError>;
```

Do the same for `finalize_ingest`. **Leave `list_blocks` alone** — it emits no event, so it resolves no emitter, and a parameter nothing consumes is a lie. Only `append_block` and `finalize_ingest` take `origin`.

- [ ] **Step 4: Use it in the backend**

In `db_backend.rs`, replace both `surface_marker(Surface::ApiHttp)` occurrences with `surface_marker(origin)`.

- [ ] **Step 5: Update the callers**

- `crates/temper-api/src/handlers/segments.rs`: pass `Surface::ApiHttp` in `append_block_handler` and `finalize_handler`. Add the import.
- `crates/temper-cli/src/cloud_backend/backend.rs:307` and `:626`: add the parameter to the impl arms. The `:626` arm binds `_payload` — it is an unimplemented local arm; add `_origin: Surface` to match.

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(an_mcp_append_is_attributed)'
cargo nextest run -p temper-services --features test-db
cargo nextest run -p temper-api --features test-db --test segments_handler_test
```

Expected: PASS. (Never run a bare `cargo nextest run -p temper-api` — it hangs at test-list enumeration on the bin target. Always scope to `--test <target>`.)

- [ ] **Step 7: Check and commit**

```bash
cargo make check
git add -A
git commit -m "fix(ingest): thread Surface through append/finalize instead of hardcoding ApiHttp

Every other Backend write threads cmd.origin; these two hardcoded Surface::ApiHttp.
Harmless while the API was the only caller — wrong the moment MCP appends a block."
```

---

### Task 5: Hoist begin's composition into `Backend::begin_segmented_ingest`

`crates/temper-api/src/handlers/ingest.rs:150-182` makes three backend calls and assembles `SegmentedBeginResponse` in the handler, using `record_ingestion_source` — a `DbBackend` inherent method, not on the trait. An MCP `ingest_begin` would duplicate all of it, and a surface is supposed to dispatch **one** operations command per inbound call.

**Files:**
- Modify: `crates/temper-workflow/src/operations/backend.rs`
- Modify: `crates/temper-services/src/backend/db_backend.rs` (`record_ingestion_source` :419 becomes private)
- Modify: `crates/temper-api/src/handlers/ingest.rs` (:150-182)
- Test: `crates/temper-services/tests/segmented_backend_test.rs`

**Interfaces:**
- Produces: `Backend::begin_segmented_ingest(&self, cmd: CreateResource, seg: SegmentedBegin) -> Result<CommandOutput<SegmentedBeginResponse>, TemperError>`

- [ ] **Step 1: Write the failing test**

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn begin_segmented_ingest_lands_block_zero_and_records_the_source(pool: PgPool) {
    let (backend, ctx, profile) = seed_backend(&pool).await;
    let cmd = create_command(ctx, profile, "Big Doc", "# Title\n\nalpha\n");

    let out = backend
        .begin_segmented_ingest(
            cmd,
            SegmentedBegin {
                total_blocks_hint: Some(2),
                block_budget: 262_144,
                source_hash: Some("sha256:abc".to_owned()),
            },
        )
        .await
        .expect("begin succeeds");

    assert_eq!(out.value.blocks.len(), 1, "block 0 landed");
    assert_eq!(out.value.blocks[0].seq, 0);
    assert!(!out.value.body_hash.is_empty(), "begin returns the live body_hash");

    let source_hash: Option<String> =
        sqlx::query_scalar("SELECT source_hash FROM kb_ingestion_records WHERE resource_id = $1")
            .bind(out.value.resource_id)
            .fetch_one(&pool)
            .await
            .expect("begin wrote the ingestion record");
    assert_eq!(source_hash.as_deref(), Some("sha256:abc"));
}
```

Note this asserts `SegmentedBeginResponse` also gains `body_hash`. Add that field in Step 3 — it is what an MCP caller echoes at finalize when it appends nothing (a one-block segmented session).

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(begin_segmented_ingest_lands)'
```

Expected: FAIL — `no method named begin_segmented_ingest`.

- [ ] **Step 3: Add the trait method and the response field**

In `crates/temper-core/src/types/ingest.rs`, add to `SegmentedBeginResponse`:

```rust
    /// The live `body_hash` after block 0 — the echo-back value for a session that appends nothing.
    pub body_hash: String,
```

In `crates/temper-workflow/src/operations/backend.rs`:

```rust
    /// Begin a segmented (multi-block) ingest: create the resource with segment 0 as its body
    /// block, record the per-resource source-provenance row, and return the landed set. One
    /// command per inbound call — the surfaces do not compose these three steps themselves.
    async fn begin_segmented_ingest(
        &self,
        cmd: CreateResource,
        seg: SegmentedBegin,
    ) -> Result<CommandOutput<SegmentedBeginResponse>, TemperError>;
```

In `db_backend.rs`, implement it by moving the handler's body verbatim, and change `pub async fn record_ingestion_source` to `async fn record_ingestion_source` (private — it now has exactly one caller):

```rust
    async fn begin_segmented_ingest(
        &self,
        cmd: CreateResource,
        seg: SegmentedBegin,
    ) -> Result<CommandOutput<SegmentedBeginResponse>, TemperError> {
        // `origin_uri` is consumed by the ingestion record below, so clone before the cmd moves.
        let origin_uri = cmd.origin_uri.clone().unwrap_or_default();
        let out = self.create_resource(cmd).await?;
        let resource_id = out.value.id;

        self.record_ingestion_source(resource_id, &origin_uri, seg.source_hash.as_deref())
            .await?;

        let landed = Self::landed_blocks(&self.pool, resource_id).await?;
        Ok(CommandOutput::new(SegmentedBeginResponse {
            resource_id: resource_id.uuid(),
            // Client-side ingest-session id — see `SegmentedBeginResponse::correlation_id`'s doc
            // for why this is not yet threaded onto the server's event ledger.
            correlation_id: uuid::Uuid::now_v7(),
            blocks: landed.blocks,
            body_hash: landed.body_hash,
        }))
    }
```

`seg.total_blocks_hint` and `seg.block_budget` are informational and recorded by the caller's manifest, not validated here — the spec's §7 says the budget is recorded, never enforced. Do not add a check.

- [ ] **Step 4: Collapse the handler to one dispatch**

In `crates/temper-api/src/handlers/ingest.rs`, replace lines 149-182. Everything above (home resolution, managed_meta parse, `CreateResource` assembly) stays; only the tail changes:

```rust
    let backend = DbBackend::new(state.pool.clone(), profile_id);

    let Some(seg) = segmented else {
        // Unchanged one-shot path — no new round-trips, no regression (design §5/§13).
        let out = backend.create_resource(cmd).await.map_err(ApiError::from)?;
        return Ok(IngestCreateResponse::OneShot(Box::new(out.value)));
    };

    let out = backend
        .begin_segmented_ingest(cmd, seg)
        .await
        .map_err(ApiError::from)?;
    Ok(IngestCreateResponse::Segmented(out.value))
```

The `origin_uri_for_ingestion` local (:62) is now dead — `begin_segmented_ingest` reads it off the cmd. Delete it.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(begin_segmented_ingest_lands)'
cargo nextest run -p temper-services --features test-db
cargo nextest run -p temper-api --features test-db --test segments_handler_test
```

Expected: PASS. `segments_handler_test`'s begin assertions must still hold — the response gained a field but lost nothing.

- [ ] **Step 6: Check and commit**

```bash
cargo make check
git add -A
git commit -m "refactor(ingest): hoist segmented-begin composition into Backend::begin_segmented_ingest

The HTTP handler was composing create + record_ingestion_source + list_blocks and
assembling the response itself, which MCP would have had to duplicate. One command
per inbound call; record_ingestion_source is private again."
```

---

### Task 6: The four MCP tools

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (extract `build_create_command` from `create_resource` :424)
- Create: `crates/temper-mcp/src/tools/ingest.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`
- Modify: `crates/temper-mcp/src/service.rs` (register beside `invocation_open` :455)
- Test: `crates/temper-mcp/tests/ingest_input_test.rs` (new — schema/shape unit tests, mirroring the existing `create_input_open_meta_test.rs`)

**Interfaces:**
- Consumes: `Backend::begin_segmented_ingest`, `append_block`, `finalize_ingest`, `list_blocks`
- Produces: MCP tools `ingest_begin`, `ingest_append`, `ingest_finalize`, `ingest_blocks`

- [ ] **Step 1: Extract the create-command builder**

`create_resource` (`resources.rs:424`) resolves the home anchor, derives the slug, defaults `origin_uri`, assembles `act`, and resolves `goal` — roughly :430-550. `ingest_begin` needs all of it. Extract, changing no behavior:

```rust
/// Build the shared `CreateResource` command from an MCP create input: resolve the home anchor
/// (running the cogmap producer gate before any write), derive the slug from the title, default
/// `origin_uri`, and resolve the optional goal ref. Shared by `create_resource` and `ingest_begin`
/// so the two cannot drift.
pub(crate) async fn build_create_command(
    svc: &TemperMcpService,
    profile_id: ProfileId,
    input: CreateResourceInput,
) -> Result<CreateResource, rmcp::ErrorData> {
    // ... the existing body of create_resource from the owner-format check through `let cmd = ...`,
    // returning `cmd` instead of dispatching it.
}
```

`create_resource` then becomes: `let cmd = build_create_command(svc, profile_id, input).await?;` followed by its existing `DbBackend::new(...).create_resource(cmd)` dispatch and `enrich_resource` response assembly. Run `cargo nextest run -p temper-mcp` — the existing tests must pass unchanged. **Commit this refactor separately** before adding tools, so a reviewer can see it moved nothing.

```bash
cargo make check
git add -A
git commit -m "refactor(mcp): extract build_create_command for reuse by ingest_begin"
```

- [ ] **Step 2: Write the failing input-shape test**

`crates/temper-mcp/tests/ingest_input_test.rs`:

```rust
//! The ingest tool inputs are LLM-facing: their JSON schemas are what an agent reads to decide
//! how to call them. These tests pin the shape, not the behavior.

use temper_mcp::tools::ingest::{IngestAppendInput, IngestBeginInput, IngestFinalizeInput};

#[test]
fn append_input_accepts_a_caller_with_no_chunker() {
    let json = r#"{"resource":"doc-019f4498-a5e1-7383-96f7-c8362b0e8daa","seq":1,"content":"beta","content_hash":"abc"}"#;
    let input: IngestAppendInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.seq, 1);
    assert_eq!(input.content, "beta");
}

#[test]
fn begin_input_carries_the_segment_budget_fields() {
    let json = r#"{"context_ref":"@me/temper","doc_type_name":"research","title":"Big","content":"# T\n\nalpha","content_hash":"abc","block_budget":262144,"total_blocks_hint":3}"#;
    let input: IngestBeginInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.block_budget, Some(262_144));
    assert_eq!(input.total_blocks_hint, Some(3));
    assert!(input.source_hash.is_none());
}

#[test]
fn finalize_input_requires_the_echoed_body_hash() {
    let json = r#"{"resource":"doc-019f4498-a5e1-7383-96f7-c8362b0e8daa","expected_blocks":3,"expected_body_hash":"sha256:deadbeef"}"#;
    let input: IngestFinalizeInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.expected_blocks, 3);
    assert_eq!(input.expected_body_hash, "sha256:deadbeef");
}
```

- [ ] **Step 3: Run to verify it fails**

```bash
cargo nextest run -p temper-mcp --test ingest_input_test
```

Expected: FAIL — `unresolved import temper_mcp::tools::ingest`.

- [ ] **Step 4: Write the tool module**

`crates/temper-mcp/src/tools/ingest.rs`:

```rust
//! Segmented (multi-block) ingest tools — the MCP surface's answer to a body that exceeds one
//! request. `ingest_begin` creates the resource with segment 0; `ingest_append` lands each further
//! segment; `ingest_finalize` declares the session complete; `ingest_blocks` reads the landed set
//! back, which is how a stateless caller resumes after an interruption.
//!
//! Unlike the CLI, an MCP caller has no chunker and no embedder: it omits `chunks_packed` and the
//! server chunks `content` itself, carrying the heading breadcrumb across the block boundary.
//! Integrity is per-segment (`content_hash`) on the way in, plus the opaque `body_hash` echoed
//! back at finalize.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_core::error::TemperError;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::ingest::{AppendBlockPayload, FinalizePayload, SegmentedBegin};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, Surface};

use crate::service::TemperMcpService;
use crate::tools::resources::{build_create_command, CreateResourceInput};

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn map_err(e: TemperError, action: &str) -> rmcp::ErrorData {
    match e {
        TemperError::NotFound(_) => {
            rmcp::ErrorData::invalid_params(format!("{action}: resource not found"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        TemperError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot modify this resource"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

fn parse_resource(s: &str) -> Result<ResourceId, rmcp::ErrorData> {
    let uuid = temper_workflow::operations::parse_ref(s)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))?
        .0;
    Ok(ResourceId::from(uuid))
}

// ── Inputs ─────────────────────────────────────────────────────────────────────

/// MCP input for `ingest_begin` — every `create_resource` field, plus the segmented-session hints.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestBeginInput {
    #[serde(flatten)]
    pub create: CreateResourceInput,
    /// sha256 (bare hex) of `content` — this segment's transit-integrity check.
    pub content_hash: String,
    /// Bytes of text this session's segment boundaries were cut at. Recorded so a resume re-derives
    /// identical boundaries; never enforced server-side.
    #[serde(default)]
    pub block_budget: Option<u32>,
    /// Best-effort total segment count, if known. Informational.
    #[serde(default)]
    pub total_blocks_hint: Option<u32>,
    /// sha256 of the whole source, when the source has a stable identity (a file). Omit when
    /// composing content in-context.
    #[serde(default)]
    pub source_hash: Option<String>,
}

/// MCP input for `ingest_append`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestAppendInput {
    /// Resource ref returned by `ingest_begin` (UUID or decorated `slug-<uuid>`).
    pub resource: String,
    /// Zero-based segment index. Segment 0 landed at begin, so appends start at 1.
    pub seq: u32,
    /// This segment's markdown text.
    pub content: String,
    /// sha256 (bare hex) of `content`. A mismatch is rejected.
    pub content_hash: String,
}

/// MCP input for `ingest_finalize`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestFinalizeInput {
    pub resource: String,
    /// Total landed segments, counting segment 0.
    pub expected_blocks: u32,
    /// The `body_hash` from your most recent `ingest_append` / `ingest_blocks` response. Opaque —
    /// echo it back verbatim.
    pub expected_body_hash: String,
}

/// MCP input for `ingest_blocks`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestBlocksInput {
    pub resource: String,
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn ingest_begin(
    svc: &TemperMcpService,
    input: IngestBeginInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);
    let pool = &svc.api_state.pool;

    if temper_core::hash::sha256_hex(input.create.content.as_deref().unwrap_or_default().as_bytes())
        != input.content_hash
    {
        return Err(rmcp::ErrorData::invalid_params(
            "content_hash does not match content".to_owned(),
            None,
        ));
    }

    let seg = SegmentedBegin {
        total_blocks_hint: input.total_blocks_hint,
        block_budget: input.block_budget.unwrap_or(262_144),
        source_hash: input.source_hash,
    };
    let cmd = build_create_command(svc, profile_id, input.create).await?;

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .begin_segmented_ingest(cmd, seg)
        .await
        .map_err(|e| map_err(e, "ingest_begin"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out.value),
    )]))
}

pub async fn ingest_append(
    svc: &TemperMcpService,
    input: IngestAppendInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);
    let resource = parse_resource(&input.resource)?;

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    let out = backend
        .append_block(
            resource,
            AppendBlockPayload {
                seq: input.seq,
                content: input.content,
                content_hash: input.content_hash,
                // No chunker, no embedder on this surface: the server chunks `content`.
                chunks_packed: None,
            },
            Surface::Mcp,
        )
        .await
        .map_err(|e| map_err(e, "ingest_append"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out.value),
    )]))
}

pub async fn ingest_finalize(
    svc: &TemperMcpService,
    input: IngestFinalizeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);
    let resource = parse_resource(&input.resource)?;

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    backend
        .finalize_ingest(
            resource,
            FinalizePayload {
                expected_blocks: input.expected_blocks,
                expected_body_hash: input.expected_body_hash,
            },
            Surface::Mcp,
        )
        .await
        .map_err(|e| map_err(e, "ingest_finalize"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        format!("Finalized {} ({} blocks).", input.resource, input.expected_blocks),
    )]))
}

pub async fn ingest_blocks(
    svc: &TemperMcpService,
    input: IngestBlocksInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);
    let resource = parse_resource(&input.resource)?;

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    let out = backend
        .list_blocks(resource)
        .await
        .map_err(|e| map_err(e, "ingest_blocks"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out.value),
    )]))
}
```

`CreateResourceInput` and `build_create_command` must be `pub`/`pub(crate)` for this import. Add `pub mod ingest;` to `crates/temper-mcp/src/tools/mod.rs`.

- [ ] **Step 5: Register the tools**

In `crates/temper-mcp/src/service.rs`, beside `invocation_open` (:455). The descriptions are the agent's only guidance — they must steer away from the tool when it is the wrong one:

```rust
    #[tool(
        description = "Begin a segmented (multi-block) ingest for a body too large to send in one call, and land its first segment. Prefer create_resource for anything that fits in a single call — segmented ingest costs extra round-trips. Returns resource_id, the landed block set, and an opaque body_hash. Follow with ingest_append for each further segment, then ingest_finalize."
    )]
    async fn ingest_begin(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestBeginInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_begin(self, input).await
    }

    #[tool(
        description = "Append one segment to an in-progress segmented ingest. Segments are zero-indexed and segment 0 landed at ingest_begin, so start at seq=1 and send them in order. content_hash is the bare-hex sha256 of content; a mismatch is rejected. Re-appending an already-landed segment is a safe no-op, which is what makes retry and resume safe. Returns the landed block set and the body_hash to echo at finalize."
    )]
    async fn ingest_append(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestAppendInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_append(self, input).await
    }

    #[tool(
        description = "Declare a segmented ingest complete. expected_blocks is the total segment count including segment 0; expected_body_hash is the opaque body_hash from your most recent ingest_append or ingest_blocks response, echoed back verbatim. Fails if the landed set does not match, which means a segment is missing — call ingest_blocks to see which."
    )]
    async fn ingest_finalize(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestFinalizeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_finalize(self, input).await
    }

    #[tool(
        description = "Read back the segments that have landed for an in-progress segmented ingest. This is how you resume after an interruption: compare the returned seq set against your segments, re-send only the missing ones with ingest_append, then ingest_finalize."
    )]
    async fn ingest_blocks(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestBlocksInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_blocks(self, input).await
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-mcp
```

Expected: PASS, including the three new input-shape tests.

- [ ] **Step 7: Check and commit**

```bash
cargo make check
git add -A
git commit -m "feat(mcp): ingest_begin/append/finalize/blocks — segmented ingest on the agent surface"
```

---

### Task 7: End-to-end equivalence and resume

The load-bearing assertion: **a segmented, server-chunked ingest of document D produces the same body and chunk set as a one-shot create of D.** That single equivalence covers breadcrumb continuity, segment reassembly, and merkle agreement at once.

**Files:**
- Create: `tests/e2e/tests/mcp_segmented_ingest_test.rs`

**Interfaces:**
- Consumes: everything above, through the real Axum server + Postgres harness in `tests/e2e/tests/common/`

**Fixture warning:** the corpus must use **short sections**. A single heading whose section is long enough to split across chunks triggers the known body-readback heading-duplication bug, and the equivalence assertion will fail for a reason unrelated to this work. Keep every section under one chunk.

- [ ] **Step 1: Write the failing test**

```rust
#![cfg(all(feature = "test-db", feature = "test-embed"))]
//! Segmented ingest driven the way an MCP caller drives it: no client-side chunks, no `.temper/`
//! manifest, resume from the server's landed set alone.

mod common;

/// Short sections on purpose — see the fixture warning in the plan. A section long enough to split
/// across chunks would trip the known heading-duplication readback bug.
fn corpus() -> Vec<&'static str> {
    vec![
        "# Manual\n\nIntro line.\n\n## Setup\n\nInstall it.\n",
        "## Usage\n\nRun it.\n\n## Caveats\n\nMind the gap.\n",
        "## Appendix\n\nReferences follow.\n",
    ]
}

#[tokio::test]
async fn segmented_server_chunked_ingest_equals_a_one_shot_create() {
    let h = common::spawn_harness().await;
    let whole: String = corpus().concat();

    // 1. One-shot create of the whole document — the reference.
    let one_shot = h.create_resource("Reference", &whole).await;

    // 2. Segmented, server-chunked ingest of the same bytes, one segment per corpus entry.
    let begin = h.ingest_begin("Segmented", corpus()[0]).await;
    let mut body_hash = begin.body_hash;
    for (i, segment) in corpus().iter().enumerate().skip(1) {
        let resp = h.ingest_append(begin.resource_id, i as u32, segment).await;
        body_hash = resp.body_hash;
    }
    h.ingest_finalize(begin.resource_id, corpus().len() as u32, &body_hash)
        .await;

    // 3. The two resources must be indistinguishable in body and chunk structure.
    assert_eq!(
        h.body_text(begin.resource_id).await,
        h.body_text(one_shot).await,
        "a segmented body must reassemble to the one-shot body"
    );
    assert_eq!(
        h.chunk_header_paths(begin.resource_id).await,
        h.chunk_header_paths(one_shot).await,
        "breadcrumbs must be continuous across block boundaries"
    );
    assert_eq!(
        h.body_hash(begin.resource_id).await,
        h.body_hash(one_shot).await,
        "the block merkle must agree with whole-document chunking"
    );
}

#[tokio::test]
async fn an_interrupted_segmented_ingest_resumes_from_the_server_alone() {
    let h = common::spawn_harness().await;

    let begin = h.ingest_begin("Interrupted", corpus()[0]).await;
    h.ingest_append(begin.resource_id, 1, corpus()[1]).await;
    // Segment 2 never lands — the process died here.

    // Resume with no local manifest: ask the server what it has.
    let landed = h.ingest_blocks(begin.resource_id).await;
    let have: Vec<u32> = landed.blocks.iter().map(|b| b.seq).collect();
    assert_eq!(have, vec![0, 1], "the server knows exactly what landed");

    let missing: Vec<usize> = (0..corpus().len()).filter(|i| !have.contains(&(*i as u32))).collect();
    assert_eq!(missing, vec![2]);

    let mut body_hash = landed.body_hash;
    for i in missing {
        body_hash = h.ingest_append(begin.resource_id, i as u32, corpus()[i]).await.body_hash;
    }
    h.ingest_finalize(begin.resource_id, corpus().len() as u32, &body_hash).await;

    assert_eq!(h.body_text(begin.resource_id).await, corpus().concat());
}

#[tokio::test]
async fn re_appending_a_landed_segment_is_an_idempotent_no_op() {
    let h = common::spawn_harness().await;
    let begin = h.ingest_begin("Idempotent", corpus()[0]).await;

    let first = h.ingest_append(begin.resource_id, 1, corpus()[1]).await;
    let again = h.ingest_append(begin.resource_id, 1, corpus()[1]).await;

    assert_eq!(first.blocks.len(), again.blocks.len(), "no duplicate block landed");
    assert_eq!(first.body_hash, again.body_hash, "body_hash is unchanged by a replay");
}
```

The `h.*` helpers do not exist yet. Model them on the existing harness in `tests/e2e/tests/common/` and on `tests/e2e/tests/streaming_ingest_test.rs`, which already drives begin/append/finalize over HTTP. `ingest_append` there sends `chunks_packed: Some(...)`; these helpers send `None` and let the server chunk. Add the helpers to `common/` only if `streaming_ingest_test.rs` has no equivalent to reuse — prefer reuse.

- [ ] **Step 2: Run to verify it fails**

```bash
cargo make test-e2e-embed 2>&1 | tee /tmp/e2e.log
```

Expected: FAIL — the harness helpers do not exist. **Never pipe a long run to `tail`**; redirect to a log and read that. This suite takes minutes (real ONNX); run it in the background and poll rather than blind-waiting.

Note: e2e runs are not safely concurrent. Do not start a second `test-e2e` while one is running.

- [ ] **Step 3: Implement the harness helpers**

Add the `ingest_begin` / `ingest_append` / `ingest_finalize` / `ingest_blocks` / `body_text` / `chunk_header_paths` / `body_hash` helpers to `tests/e2e/tests/common/`, driving the real HTTP endpoints (`POST /api/ingest` with `segmented`, `POST /api/resources/{id}/blocks`, `POST /api/resources/{id}/finalize`, `GET /api/resources/{id}/blocks`). `content_hash` on each append is `temper_core::hash::sha256_hex(segment.as_bytes())`.

`body_text` reads through the substrate's `resource_body_text` SQL function; `chunk_header_paths` selects `header_path` from `kb_chunks` joined to live blocks, ordered by `(b.seq, c.chunk_index)`.

- [ ] **Step 4: Run to verify they pass**

```bash
cargo make test-e2e-embed 2>&1 | tee /tmp/e2e.log
grep -E 'error: test run failed|Summary' /tmp/e2e.log
```

Expected: PASS. Trust the exit code, not nextest's per-binary Summary line.

If `segmented_server_chunked_ingest_equals_a_one_shot_create` fails on `body_text` with a duplicated heading, check the fixture's section lengths before suspecting the chunker.

- [ ] **Step 5: Regenerate the e2e sqlx cache if you added macro queries**

```bash
cargo make prepare-e2e
```

- [ ] **Step 6: Check and commit**

```bash
cargo make check
git add -A
git commit -m "test(e2e): segmented server-chunked ingest equals a one-shot create; resume; idempotent replay"
```

---

### Task 8: Branch verification and PR

- [ ] **Step 1: Full workspace suite**

```bash
cargo make test-all 2>&1 | tee /tmp/test-all.log
echo "exit=$?"
```

`--workspace` unifies features and pulls ort into `temper-cloud` — if that bites, exclude it (`--exclude temper-cloud`) and say so in the PR body.

- [ ] **Step 2: Embed-gated suites**

```bash
cargo make test-e2e-embed 2>&1 | tee /tmp/e2e-final.log
```

- [ ] **Step 3: Confirm no migration was added**

```bash
git diff --stat main...HEAD -- migrations/
```

Expected: empty. A non-empty diff means the design was violated — escalate rather than committing it.

- [ ] **Step 4: Confirm no placeholder hashes survive**

```bash
grep -rn 'unused-client-text-hash' crates/ tests/
```

Expected: no matches.

- [ ] **Step 5: Rebuild the CLI binary before any manual e2e check**

```bash
cargo build -p temper-cli --bin temper
```

`test-e2e` does not rebuild the binary; a stale `temper` will silently test old code.

- [ ] **Step 6: Open the PR**

```bash
gh pr create --title "MCP segmented ingest — begin/append/finalize on the agent surface" --body "$(cat <<'EOF'
Delivers the streaming-resumable-ingestion design's §12 MCP deferral (PR #327), closing its
final acceptance criterion.

## What

Four MCP tools — `ingest_begin`, `ingest_append`, `ingest_finalize`, `ingest_blocks` — giving the
agent surface the multi-block, resumable ingest the CLI and API got in #327. An MCP caller has no
chunker and no embedder, so it sends raw segment text and the server chunks it, carrying the
heading breadcrumb across block boundaries so `header_path` is continuous.

## The design decision

`resource_finalize` compares `expected_body_hash` against a merkle over chunk hashes a
non-chunking caller can never compute. Rather than exempt that caller, integrity moved to where
it does have information:

- **Per-segment, inbound:** `append` now verifies `sha256(content) == content_hash`. These two
  fields were dead — the server read only `chunks_packed` and `seq`, and the fixtures said so
  (`content_hash: "unused-client-text-hash"`). They are now the transit-integrity contract, and
  the CLI gains the check for free.
- **Whole-body, at finalize:** `BlocksResponse` carries the live `body_hash`; the caller echoes it
  back. `expected_body_hash` stays required and `resource_finalize`'s SQL is untouched.

**No new migrations.**

## Bundled fixes this beat's caller exposed

- `append_block` / `finalize_ingest` hardcoded `Surface::ApiHttp`, so an MCP append would have been
  attributed to the `web` emitter. `Surface` is now threaded, as it is on every other write.
- Segmented-begin's three-call composition lived in the HTTP handler, which MCP would have had to
  duplicate. Hoisted into `Backend::begin_segmented_ingest` — one command per inbound call.

## Testing

The load-bearing assertion is e2e: a segmented, server-chunked ingest of document D produces the
same body, chunk breadcrumbs, and `body_hash` as a one-shot create of D. Plus resume-from-server
(no client manifest) and idempotent replay. Embed-gated; runs in the Embed CI job.

Spec: `docs/superpowers/specs/2026-07-09-mcp-segmented-ingest-design.md`
Plan: `docs/superpowers/plans/2026-07-09-mcp-segmented-ingest.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 7: Stop**

Report "PR up + CI green + summary" and stop. **Do not merge.** Do not run production migrations. Pete's review gates the merge.
