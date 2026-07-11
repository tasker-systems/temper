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

## Convention v1 (issue #359)

| `open_meta` key | Value shape | Weight | Purpose |
|-----------------|-------------|--------|---------|
| `keywords` | JSON array of strings (space-joined), or a bare JSON string | `C` | Deliberately-attached topical tags. A resource that lists the query term as a keyword ranks above an otherwise-identical resource that does not. |
| `descriptor` | JSON string | `D` | The full section descriptor, for corpora where importers truncate it out of the title under length pressure. Keeps the discriminating words searchable. |

Any key **not** in this table is stored but not indexed — attaching it changes nothing about search.

Example `open_meta`:

```json
{
  "keywords": ["thermocline", "stratification"],
  "descriptor": "Vertical temperature structure of the open ocean"
}
```

## Maintenance & versioning

The vector is rebuilt whenever the title changes, the body is re-blocked, or an indexed `open_meta`
key is set (`property_set`). At create time the block projection rebuilds the vector *before* the
property_set events fire, so the property_set rebuild hook is what actually gets `keywords`/`descriptor`
into the index — for both create and later updates. A superseded value (folded row) drops out on the
next rebuild.

The convention is versioned by migration — the migration *is* the schema version. The indexed set was
introduced by `migrations/20260711000040_index_open_meta_into_fts.sql`. Adding or reweighting a field
is a new additive migration that `CREATE OR REPLACE`s `_rebuild_resource_search_vector` and backfills
the affected resources; it bumps this convention to the next version. Do not edit the introducing
migration in place (shipped migrations are checksum-locked).
