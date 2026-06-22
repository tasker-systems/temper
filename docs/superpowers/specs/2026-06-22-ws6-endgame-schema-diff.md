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

## Next mechanical step

Column-level diff of the 11 shared tables (B vs C shapes), and confirm the 🟡/❓ dispositions in
A (grep each table's usage in temper-api/cli + check temper_next equivalents). Output: the
canonical schema as draft bootstrap migrations.

---

## References

Endgame: `2026-06-22-ws6-migration-endgame-design.md` · captured from prod main 2026-06-22.
