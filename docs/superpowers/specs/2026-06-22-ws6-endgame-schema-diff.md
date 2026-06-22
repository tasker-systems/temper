# WS6 Endgame — schema diff (temper_next vs public) → pristine-target inventory

First-pass working artifact for the [migration endgame](2026-06-22-ws6-migration-endgame-design.md).
Captured live from prod (2026-06-22). Goal: define the **canonical (pristine) schema** = the
union of *intended* outcomes. This is **not** a strict superset either way — `temper_next` is the
new substrate but `public` carries cross-cutting infra `temper_next` never had.

Classification confidence: ✅ high · 🟡 needs verification · ❓ open design question.

---

## A. Tables only in `public` (17) — classify each: SURVIVES vs SUPERSEDED/DIES

| Table | Disposition | Note |
|---|---|---|
| `kb_backend_selection` | ✅ DIES | the flag itself — gone with the split |
| `kb_resource_edges` | ✅ SUPERSEDED | → `temper_next.kb_edges` |
| `_sqlx_migrations` | ✅ SURVIVES (infra) | rebuilt by the bootstrap migration set |
| `kb_profile_auth_links` | ✅ SURVIVES (infra) | Auth0 identity links — cross-cutting, no temper_next equiv |
| `kb_join_requests` | ✅ SURVIVES (infra) | access requests — cross-cutting |
| `kb_system_settings` | ✅ SURVIVES (infra) | instance settings |
| `kb_scopes` | 🟡 SURVIVES (infra) | RBAC scopes — confirm still used |
| `kb_blob_files` | 🟡 SURVIVES (infra) | blob/upload refs — confirm vs temper_next content_blocks |
| `kb_ingestion_records` | 🟡 SURVIVES (infra) | ingest idempotency — confirm |
| `kb_doc_types` | ❓ OPEN | §7 dissolved the typed id (doc_type as property); PR #159 dropped the cross-namespace lookup. But the *system still needs the doc-type schema set somewhere*. Property-only? Or a surviving registry table? **Decide.** |
| `kb_device_sync_state` | ❓ OPEN | sync state — cloud-only demoted the vault to a read-only projection; does sync state still exist or shrink? |
| `kb_resource_manifests` | ❓ OPEN | per-device manifest ledger — same cloud-only question |
| `kb_resource_revisions` | 🟡 SUPERSEDED | likely → `kb_block_revisions` / event ledger; confirm no unique data |
| `kb_resource_search_index` | ❓ OPEN | FTS index — how does temper_next search? (generated tsvector? different mechanism?) confirm before dropping |
| `kb_team_invitations` | 🟡 SURVIVES (infra) | team mgmt — temper_next has team_* but not invitations |
| `kb_team_resources` | 🟡 SUPERSEDED | → `kb_team_contexts` / `kb_team_cogmaps` / `kb_resource_access`? confirm mapping |
| `kb_transfers` | 🟡 SURVIVES? | ownership transfers — confirm still in product |

## B. Tables only in `temper_next` (17) — the canonical substrate (keep)

`kb_edges`, `kb_properties`, `kb_entities`, `kb_invocations`, `kb_content_blocks`,
`kb_block_provenance`, `kb_block_revisions`, `kb_resource_homes`, `kb_resource_access`,
`kb_cogmaps`, `kb_cogmap_regions`, `kb_cogmap_region_members`, `kb_cogmap_components`,
`kb_cogmap_lenses`, `kb_team_cogmaps`, `kb_team_contexts`, `kb_teams_parents`.

These are the new event-sourced + cognitive-map substrate. All survive as canonical.

## C. Tables in BOTH (11) — reconcile to ONE shape (canonical = temper_next's)

`kb_resources`, `kb_events`, `kb_event_types`, `kb_profiles`, `kb_contexts`, `kb_topics`,
`kb_chunks`, `kb_chunk_content`, `kb_resource_audits`, `kb_teams`, `kb_team_members`.

Same names, **likely different columns** (e.g. `temper_next.kb_resources` has no
`doc_type_name`/`slug` columns — doc type is a property, slug is dissolved). **Action:**
per-table column diff; canonical adopts the temper_next shape; identify any `public`-only column
carrying data that must migrate.

---

## Open design questions this raises (for the endgame/bootstrap specs)

1. **Doc types** — registry table vs pure property? The system needs the *set* of doc-type
   schemas (for defaults/validation). §7 dissolved the typed id but not necessarily the registry.
2. **Sync/manifest tables** — under cloud-only (vault = read-only projection), do
   `kb_device_sync_state` / `kb_resource_manifests` survive, shrink, or die?
3. **Search index** — confirm temper_next's FTS mechanism before dropping
   `kb_resource_search_index` (the §9 harness proved FTS parity, so *something* serves it).
4. **Shared-table column reconciliation** — the 11 both-tables need a real column diff; that is
   the next mechanical step.

---

## Investigation findings (2026-06-22) — dispositions confirmed + two new flags

**Row counts (public-only tables):** `kb_backend_selection`=1 (DIES), `kb_resource_edges`=563
(SUPERSEDED→`kb_edges`), `_sqlx_migrations`=45 (infra), `kb_profile_auth_links`=5 (SURVIVES),
`kb_system_settings`=1 (SURVIVES), `kb_scopes`=1 (SURVIVES), `kb_doc_types`=6 (→ Rust-interiority,
table dies), `kb_device_sync_state`=3 (DIES), `kb_resource_manifests`=1239 (DIES),
`kb_resource_revisions`=3342 (see flag), `kb_resource_search_index`=1239 (SUPERSEDED, see #3),
empty (0): `kb_blob_files`, `kb_ingestion_records`, `kb_join_requests` (schema survives),
`kb_team_invitations`, `kb_team_resources`, `kb_transfers`.

**Column diff (11 shared):** `temper_next` is a more-normalized model, not public-plus-extras.
`kb_resources` shed `kb_context_id`/`kb_doc_type_id`/profile-FKs/`slug` (→ homes/properties/
access) + added `body_hash`; `kb_events` moved direct-FKs → entity/anchor/invocation model;
`kb_resource_audits` dropped the hash columns. Canonical = the temper_next shape for these.

**#1 doc types — CONFIRMED table dies → Rust-interiority.** 6 rows; read by
`doc_type_service`, `ingest_service`, `resource_service`, and `sync_service` (the last dies).
Per decision: the doc-type *set* becomes Rust-side (temper-core/types/schemas already holds the
schemas); rewire the doc_type_service/ingest id↔name resolution off the table.

**#2 sync/manifest — CONFIRMED die (cloud-only).** `kb_device_sync_state`,
`kb_resource_manifests`, and `sync_service` all retire; vault is a read-only `pull` projection.
*Deferred-but-noted:* managed/open meta + the three hashes (`body_hash`/`managed_hash`/
`open_hash`) were YAML-frontmatter-era change-detection; mostly vestigial now that content lives
in `kb_properties` and the vault is pull-only. Revisit the meta/hash model in a later pass.

**#3 search — CONFIRMED mechanism.** `temper_next` has **no stored tsvector**; readback builds
it **at query time** (`setweight(to_tsvector(title),'A') || setweight(to_tsvector(body),'B')`,
`readback/mod.rs:650`). So `kb_resource_search_index` (stored tsvector) is droppable — but this
is **unindexed at scale**. Flag for the search followup (which also wants graph-nearness +
cogmap-region salience): the canonical schema likely wants a stored/generated tsvector.
Embeddings: `kb_chunks.embedding` (vector) exists in both; `kb_cogmap_regions.centroid` is new.

### 🔴 NEW FLAG 1 — identity/profile layer is NOT in temper_next (reconciliation, not superset)

`temper_next.kb_profiles` has **1 row** (the corpus owner) and dropped
`email`/`avatar_url`/`preferences`/`vault_config`/`is_active`. `public` has multiple profiles +
5 `kb_profile_auth_links`. **The canonical schema = temper_next substrate ∪ public's
identity/auth/infra layer** (full profiles, auth_links, system_settings, scopes, join_requests).
The §9 harness never validated profile/identity completeness — it proved resource/edge/property/
content parity for one owner. This is the concrete shape of "union of intended outcomes."

### ✅ FLAG 2 — RESOLVED: no current-content loss; dropped revisions are mostly noise

Investigated 2026-06-22. The 3342→1252 delta is historical revision *trail*, not live content:
- **Current content aligns:** public *current* (non-superseded) chunks = **14,713** vs
  `temper_next` = **14,750** (~0.25%); content-hash of current chunk text matched **1,165 / 1,167**
  resources (99.83%); §9 body floor was already 0-mismatch over the full corpus.
- **Revisions are mostly noise:** of 3342, **912 (27%) are no-op duplicate-hash bumps**, and 89
  resources have >1 revision but a single content state (pure flapping). ~2,430 distinct content
  states is the real signal (~2 genuine versions/resource).

**Verdict:** safe to drop `kb_resource_revisions` — current content is preserved; granular
prior-version history is the **event ledger's** job (`resource-lifecycle-event-sourcing`), not
this table's. **Spot-check residue:** 2 resources had a current content-hash mismatch (0.17%) —
likely the chunk-ordering / date-anomaly class; verify they're benign before the drop.

## Next mechanical step

Draft the canonical schema as bootstrap migrations: temper_next substrate + public identity/auth/
infra layer; resolve the two 🔴 flags (identity reconciliation; revision-history retention) first
since they change what tables/data the canonical must hold.

---

## References

Endgame: `2026-06-22-ws6-migration-endgame-design.md` · captured from prod main 2026-06-22.
