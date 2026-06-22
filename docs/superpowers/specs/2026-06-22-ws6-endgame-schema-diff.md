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
| `kb_doc_types` | ✅ DIES → Rust-interiority | *(Resolved — see findings #1 below.)* §7 dissolved the typed id (doc_type as property); PR #159 dropped the cross-namespace lookup. The doc-type *set* becomes Rust-side (`temper-core/types/schemas`); the table dies. |
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

### ✅ FLAG 1 — RESOLVED: identity union pinned (5 profiles + 5 auth_links); NO resource-data loss

Investigated 2026-06-22 against the held `flip-rollback-2026-06-22` snapshot (read-only). The
identity layer is a **reconciliation, not a superset** — and both halves are now concrete.

**Data half — no loss.** `public.kb_profiles` has 5 rows: 2 sentinels (`system`, `anonymous`,
ids `…0004…0001/0002`) + 3 humans (`j-cole-taylor`, `gm-anirudh`, `lohjishan`). **Only
`j-cole-taylor` owns resources** (1239; 1236 active); every other profile owns **0**. So
single-owner synthesis (which built `temper_next` for the corpus owner) lost **no** resource
data. The non-owner humans participate in nothing else either: 0 team_members, 0 join_requests,
0 invitations, 0 transfers. They are pure identity + `auth_link` rows. `temper_next.kb_profiles`
confirmed = **1 row** (owner, `system_access='none'` — the schema default, not a real level).

**The identity union to carry into canonical** (substrate has only the owner; these must be
INSERTed):
- **5 `kb_profiles`** — the 4 missing rows (`system`, `anonymous`, `gm-anirudh`, `lohjishan`)
  + the owner. Map `public.slug → handle`. Re-add `email` + `preferences`. **Drop**
  `avatar_url`, `vault_config` (vestigial under cloud-only), `is_active`, `updated`.
- **5 `kb_profile_auth_links`** — `neon_auth` for the 3 humans + `system`/`anonymous` sentinel
  links. Carried verbatim (table is substrate-absent).
- **`system_access` must be set explicitly** per profile (the synth default `'none'` is wrong for
  the owner). Recommendation: owner → `admin`; the 2 registered humans → `approved` (preserves
  their `access_mode='open'` + `is_active=true` status quo); sentinels → `none`. Adjustable.

**Infra half — seed-level, fully enumerated.** Carrying real data: `kb_system_settings`=1
(`access_mode='open'`, no instance_name/terms). Empty (schema-only): `kb_ingestion_records`,
`kb_blob_files`, `kb_join_requests`, `kb_team_invitations`, `kb_transfers`. The lone
`public.kb_teams` row is the `temper-system` system team (id `…0002`) — a seed row reconciled in
the 11-shared set, not an identity-layer concern.

> **Correction (2026-06-22, while grounding the canonical draft): `kb_scopes` does NOT survive.**
> The schema-diff first marked it 🟡 SURVIVES; verification against the authoritative substrate
> install (`install_temper_next.sql`, generated from `01_schema.sql`) shows the substrate
> **dissolved** scopes: `kb_cogmaps` is *"Was kb_scopes; renamed, porosity dropped"* (01_schema
> §kb_cogmaps) and `kb_events.scope_id` became the producing-anchor (`[PHASE-2 DECISION]`). The
> `porosity` enum is explicitly RETIRED ("visibility is teams:RBAC"). So `kb_scopes` (1 row
> `public`/`access`) is **superseded**, not carried. `temper-events`' `kb_scopes`/`porosity` code
> (`ledger.rs:50`, `types/scope.rs`) targets the *old* event model — it is **scaffolding to
> retire** in the substrate/scaffolding disentanglement (endgame step 2), not substrate to keep.
>
> **✅ AFFIRMED (user, 2026-06-22):** the model moved to cogmaps away from scopes; `kb_scopes` is
> vestigial and is **dropped** (not grafted). The `temper-events` scope/porosity code is confirmed
> scaffolding to retire in the disentanglement step.

The §9 harness never validated identity completeness — it proved resource/edge/property/content
parity for one owner. This investigation closes that gap: the "union of intended outcomes" is
**temper_next substrate (resource/content/cogmap, single-owner) ∪ {5 profiles, 5 auth_links, 1
system_settings}** (`kb_scopes` superseded — see correction above).

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

**Both 🔴 flags now resolved** (Flag 1 identity-union pinned above; Flag 2 revision-history safe to
drop). Unblocked: draft the canonical schema as bootstrap migrations = **temper_next substrate
(`01_schema.sql` shape) + the enumerated identity/auth/infra layer**:

- **Shared `kb_profiles`** adopts substrate shape (`handle`, `system_access`) **+** re-added
  `email`, `preferences`; drops `avatar_url`/`vault_config`/`is_active`/`updated`.
- **Graft the 7 substrate-absent infra tables** from `migrations/` DDL: `kb_profile_auth_links`,
  `kb_system_settings`, `kb_join_requests`, `kb_team_invitations`, `kb_transfers`,
  `kb_ingestion_records`, `kb_blob_files`. (`kb_scopes` is superseded — see Flag-1 correction.)
  All FKs target substrate-present ids (`kb_profiles`/`kb_teams`/`kb_resources`).
- **Data carry-over migration** inserts the 4 missing profiles + 5 auth_links + the 1 seed row
  (system_settings), setting `system_access` explicitly per profile.

Remaining pre-drop verification (gated, execution-phase): the 2 Flag-2 content-hash mismatches
(0.17%) spot-checked benign before dropping `kb_resource_revisions`.

> **✅ Canonical-layer draft VALIDATED (2026-06-22).** The graft/reconcile/carry-over draft
> (`2026-06-22-ws6-canonical-layer-draft.sql`) was diffed read-only against the live
> `flip-rollback-2026-06-22` snapshot (pg_dump of both schemas). GREEN, no blockers: all 7 grafted
> CREATE TABLEs are byte-faithful to `public`'s real DDL; the 3 enums match label sets + order;
> the `kb_profiles` ADD COLUMNs match and don't collide; all 3 carry-over INSERT…SELECT column
> lists resolve on both ends, and the `system_access` CASE covers exactly the 5 real slugs.
> Execution-checklist item satisfied: substrate owner `handle` already equals legacy `slug`
> (`j-cole-taylor`), so the profiles `ON CONFLICT (id) DO UPDATE` not updating `handle` is
> drift-free. The two tables the audit flagged as unaddressed (`kb_device_sync_state`,
> `kb_doc_types`) are decided DROPs (Flags #1/#2) — the additive graft layer is silent on them by
> design, now cross-referenced in the draft's NOT-GRAFTED note.

---

## Shared-table column reconciliation (§C — RESOLVED, 2026-06-22)

The 11 both-tables are **already in `temper_next`** (synthesized), so canonical = the `temper_next`
shape by construction. The reconciliation question is not "what to carry" but: do the P-only
columns and the row-count gaps represent intentional dissolution/supersession, or silent loss?
Verified read-only vs the live substrate (flip-rollback snapshot). `kb_profiles` resolved in Flag
1; the other 10:

| Table | rows P→N | P-only columns → disposition | Row gap verdict |
|---|---|---|---|
| `kb_resources` | 1239→1237 | `kb_context_id`→homes, `kb_doc_type_id`→property, `owner/originator_profile_id`→homes/access, `slug`→homes (+`body_hash` new) | the 3 absent are all `is_active='f'` (soft-deleted) — correctly excluded. **Benign** |
| `kb_chunks` / `kb_chunk_content` | 77248→14711 | `first/superseded_revision_id` → block-revisions model (+`block_id` new) | gap = superseded revision trail (Flag 2). **Benign** |
| `kb_event_types` | 15→27 | `description`, `is_deprecated` dropped (+`payload_schema`,`schema_version` new) | registry re-seeded, richer in next. **Benign** (note: `description` was doc-only) |
| `kb_resource_audits` | 5916→**0** | `action`/`body_hash`/`managed_hash`/`open_hash`/`profile_id`/`device_id`/`event_id` dropped | **superseded by `kb_events`** — action counts mirror event types (`update_meta`=3124 ↔ `managed_meta_updated`=3124, `create`=574 ↔ `resource_created`). History not carried |
| `kb_contexts` | 11→6 | `kb_owner_id`/`kb_owner_table`→`owner_id`/`owner_table` (rename), `updated` dropped (+`slug` new) | 5 dropped are all **empty** (`default`×3, a `general` dupe, `writing`); the 6 kept are exactly those with resources. Contexts re-materialize on first use. **Benign** |
| `kb_topics` | 4→**0** | same shape | 4 taxonomy seeds (`temper.bootstrap`,`declaration`,`deformation`,`judgment`), 0 event refs → **re-seed** the taxonomy in canonical if live; else benign |
| `kb_teams` | 1→1 | `created_by_profile_id`/`description`/`is_active`/`metadata`/`updated` dropped | the `temper-system` seed team; confirm the 1 row carried at collapse |
| `kb_team_members` | 0→1 | `id`/`invited_by_profile_id`/`joined_at`→`created` | public empty; next has owner's derived membership. **Benign** |
| `kb_events` | 8825→5262 | direct-FKs (`profile_id`/`resource_id`/`kb_context_id`/`device_id`/`scope_id`) → entity/anchor/invocation model | **NOT a 1:1 migration** — see below |

### 🟠 Load-bearing finding — pre-flip mutation history is NOT preserved

`kb_events` is **not** a replay of the historical log. The vocabularies are disjoint: public =
`body_updated`(4478)/`managed_meta_updated`(3124)/… (the mutation trail); next =
`property_asserted`(3175)/`resource_created`(1237)/`relationship_asserted`(848) — a
**synthesis-minted genesis stream representing current state**, minted fresh at synthesis.

So **three history tables converge on one disposition** — none of the pre-flip mutation history is
carried: `kb_resource_revisions` (3342, Flag 2), `kb_resource_audits` (5916, superseded), and the
historical `kb_events` (8825). The canonical preserves **current state** (resources, properties,
content, edges — all §9-proven) + a **genesis event stream** + **forward event-sourcing**, but the
granular pre-flip "who-changed-what-when" trail is gone. This is a legitimate genesis-from-state
design, and it extends the already-accepted Flag-2 verdict ("drop revisions; history is the event
ledger's job") — with the clarification that **the event ledger itself starts at genesis, it does
not backfill the old trail.**

> **✅ AFFIRMED (user, 2026-06-22):** intended posture. The legacy events were high-noise;
> synthesis-mint a genesis stream and run forward event-sourcing from there. No backfill of the
> pre-flip revisions/audits/events trail. This is no longer an open question — the legacy drop may
> proceed on this basis (subject to the gated execution-phase snapshot + the 2 Flag-2 spot-checks).

### Mechanical notes for the bootstrap-export spec
- Column **renames** to encode (current state already lives under the new names in the substrate;
  documenting the mapping): `kb_contexts.kb_owner_*` → `owner_*`. No data action.
- `kb_resource_audits` survives as a (currently empty) substrate table with the hash columns
  dropped — decide whether it is repurposed forward or retired (0 post-flip rows ⇒ currently
  unused; the event ledger is the live audit path).
- `kb_topics` taxonomy seed: include in the canonical system-seed if the topic feature is live.

---

## References

Endgame: `2026-06-22-ws6-migration-endgame-design.md` · captured from prod main 2026-06-22.
