# Annotate-only block provenance + span locators (issue #355)

## Problem

Two related gaps in block provenance:

1. **Backfill required a body revise.** The only way to attach sources to an existing resource was
   `resource update` with a body. For a corpus imported without sources, the cheapest honest repair
   was "re-send the unchanged body + `--sources`" per resource: correct and UUID-preserving, but it
   round-tripped full bodies and triggered the revise pipeline (re-chunk / re-embed) for a
   metadata-only change — O(2 calls × corpus size), and it churned `body_hash` and embeddings.
2. **No span locator.** `Incorporation` carried no byte/line/section range, so provenance could say
   "this block came from that document" but not *where* in it — insufficient for citation-grade
   retrieval over long sources ("part 11 of a 16-chunk series" needs "lines 120–180 of the source").

## Design

### 1. Annotate-only write path — `block_provenance_annotated`

A dedicated event records incorporation rows onto an existing block **without touching chunks**. It is
NOT `block_mutated` with an empty chunk set: `_project_block_mutated` unconditionally supersedes and
re-inserts every chunk (a new generation, re-embedded), and `block_mutate` hard-rejects an empty chunk
set. The annotate path is therefore its own event with its own projector.

The path threads every layer, mirroring the existing write verbs:

| Layer | Addition |
|-------|----------|
| Substrate payload | `BlockProvenanceAnnotated { block_id, incorporated }` (`payloads.rs`); added to `TYPED_EVENT_NAMES` + `verify_ledger_roundtrip`; committed JSON-Schema snapshot |
| Substrate fire | `EventKind::BlockProvenanceAnnotated`, `SeedAction::BlockAnnotate`; `writes::annotate_block_sources{,_with,_in_tx}` + `AnnotateParams`; block resolution shared with the revise path via `resolve_target_block` |
| Replay | payload-only arm → `_project_block_annotated` (no sidecar); the exhaustive `EventKind` match makes a missed arm a compile error |
| SQL (`20260710000001`) | register the event type; `_project_block_annotated(event, payload)` calls only the chunk-independent `_insert_block_provenance`; `block_annotate(payload, emitter, meta, invocation, correlation)` resolves the home to anchor the event and rejects empty `incorporated` |
| Backend / ops | `AnnotateResource` command; `Backend::annotate_resource`; `DbBackend` impl (auth before write via `check_can_modify_next`; `ActContext` → `EventContext`) |
| HTTP | `POST /api/resources/{id}/provenance` → `ResourceAnnotateRequest` → `AnnotateResource` |
| Client | `resources().annotate(id, req)` |
| CLI | `temper resource annotate <ref> --sources <…> [--content-block <id>]` |
| MCP | `annotate_resource` tool (returns the resulting per-block provenance) |

**Invariants.** Because the projector runs only `_insert_block_provenance`, `block_body_hash`, the
`kb_chunks` rows (and their embeddings), and `kb_block_revisions` are all left untouched — verified in
`content_mutation.rs::annotate_records_provenance_without_touching_chunks` and in the e2e
`annotate_backfills_provenance_without_revise_through_cli_api_db`. `_insert_block_provenance` is
idempotent (`ON CONFLICT DO NOTHING` on `(block_id, source_kind, source_id, contributed_by_event_id)`)
and a pure function of `(event, payload)`, so the annotate replays byte-identically.

`accretion_seq` is the source's list index (position → `seq`), the same rule the create/update
`--sources` path uses — annotate and revise derive `seq` identically.

### 2. Span locators — a URI-fragment convention (zero schema change)

A locator is expressed as a **fragment on a `Remote` source URI**: `…/source.md#L120-L180`. This needs
no new column and no payload field:

- `normalize_remote_uri` deliberately does not touch the fragment (it is semantically load-bearing), so
  two locators into the same base document (`…#L120-L180`, `…#L200-L260`) are distinct
  `kb_remote_sources` rows — distinct provenance records, as intended.
- `resource_block_provenance` surfaces `source_uri` **verbatim** (the raw URL as supplied), so the
  locator is visible unchanged in `--provenance`, `get_block_provenance`, and the HTTP provenance read.

So a locator round-trips through both `create --sources` and the new `annotate --sources` with no code
change beyond documentation — verified in `annotate_with_remote_locator_round_trips` and the e2e test.
Line ranges (`#L120-L180`) are the documented default; any fragment the caller chooses rides through
verbatim, so byte ranges or section anchors are equally expressible.

## Acceptance criteria

- **Annotating N resources performs no body revise** (`body_hash` and embeddings unchanged, verified)
  and `--provenance` shows the new rows. ✓
- **A locator round-trips**: written at annotate/create time, visible in `--provenance` and
  `get_block_provenance` output. ✓
