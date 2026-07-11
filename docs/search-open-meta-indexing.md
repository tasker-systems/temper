# open_meta search-indexing convention

Importers enrich resources with `open_meta` — section descriptors, keyword lists, facet values. Most
of it is free-form and never touches ranking. A small, **versioned** set of keys is different: it is
folded into the stored full-text vector so it affects search ranking. This page is that contract, so
an importer can attach a key and rely on it being searchable.

## Where it lives

Each `open_meta` key lands in `kb_properties` as one row (`owner_table = 'kb_resources'`,
`property_key` = the key, `property_value` = its JSON value). The stored FTS vector in
`kb_resource_search_index` is built by `_rebuild_resource_search_vector`, which reads the indexed keys
and `setweight`s them alongside the title and body. `ts_rank` then picks up the extra lexemes with no
query-side change — the same code path serves the CLI, MCP, and API search surfaces.

Weights follow Postgres' four-tier model (`A > B > C > D`). Title is `A`, body is `B`. The indexed
`open_meta` keys sit **below** both, so importer metadata breaks ties and surfaces matches the
title/body missed — without ever outweighing a genuine primary-content match.

## Indexed keys — convention v2

| `open_meta` key | Value shape | Weight | Since | Purpose |
|-----------------|-------------|--------|-------|---------|
| `keywords` | JSON array of strings (space-joined), or a bare JSON string | `C` | v1 (#359) | Deliberately-attached topical tags. A resource that lists the query term as a keyword ranks above an otherwise-identical resource that does not. |
| `tags` | JSON array of strings (space-joined), or a bare JSON string | `C` | v2 | The everyday topical-tag key; ranks identically to `keywords`. Added because production evidence showed the corpus uses `tags`, not `keywords` — so v1's ranking win was unreachable by real data until this key was indexed. |
| `descriptor` | JSON string | `D` | v1 (#359) | The full section descriptor, for corpora where importers truncate it out of the title under length pressure. Keeps the discriminating words searchable. |

Any key **not** in this table is stored but not indexed — attaching it changes nothing about search.

Example `open_meta`:

```json
{
  "tags": ["thermocline", "stratification"],
  "descriptor": "Vertical temperature structure of the open ocean"
}
```

## Recognized shape-only conventions

Beyond the indexed keys, temper *recognizes* a few more `open_meta` keys and constrains their shape
(so a mis-shaped value is caught at write time) without folding them into the vector. These are the
common conventions the corpus already uses:

| `open_meta` key | Value shape | Notes |
|-----------------|-------------|-------|
| `date` | JSON string, `YYYY-MM-DD` | The most common `open_meta` key in production. |
| `relates_to` / `references` / `depends_on` | JSON array of strings (refs) | Soft relationships, parallel to the hard edge model. |
| `derived_from` / `preceded_by` | JSON string or array of strings | Soft provenance / sequence. |

The full, authoritative registry — including which keys are FTS-indexed (and at what weight) versus
shape-only — is the self-describing schema
(`crates/temper-workflow/schemas/open_meta.schema.json`). Dump it any time:

- CLI: `temper resource describe-open-meta`
- MCP: the `describe_open_meta` tool

The open tier stays free-form (`additionalProperties: true`): an **unrecognized** key of any shape is
always stored and never rejected, so a newer convention key reaching an older temper never hard-fails.

## Producers & validation (symmetric defense)

Producers attach these keys via `--open-meta` (CLI) or the `open_meta` field (ingest API / MCP). Both
ends validate against the schema:

- **Send-side** (CLI create/update): a *recognized* key carrying the wrong shape (e.g. `descriptor: 42`,
  a malformed `date`) is a hard error before the request leaves. Discouraged bare keys (`slug`/`title`,
  whose canonical home is the managed `temper-slug`/`temper-title`) surface as a warning.
- **Receive-side** (server, shared across API + MCP): the same shape check runs before the value lands
  as a property row.

A wrong shape on an indexed key is exactly the footgun this catches: it stores but does not index — a
silent search miss.

## Maintenance & versioning

The vector is rebuilt whenever the title changes, the body is re-blocked, or an indexed `open_meta`
key is set (`property_set`). At create time the block projection rebuilds the vector *before* the
property_set events fire, so the property_set rebuild hook is what actually gets `keywords`/`descriptor`
into the index — for both create and later updates. A superseded value (folded row) drops out on the
next rebuild.

The convention is versioned by migration — the migration *is* the schema version. v1 (`keywords`@C,
`descriptor`@D) was introduced by `migrations/20260711000040_index_open_meta_into_fts.sql`; v2 (`tags`@C)
by `migrations/20260711000050_index_open_meta_tags_into_fts.sql`. Adding or reweighting a field is a new
additive migration that `CREATE OR REPLACE`s `_rebuild_resource_search_vector`, extends the
`_project_property_set` rebuild gate, and backfills the affected resources; it bumps this convention to
the next version. Do not edit an introducing migration in place (shipped migrations are
checksum-locked).
