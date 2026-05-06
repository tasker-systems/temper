# Schema-Driven Managed-Meta Alignment — Design Spec

**Date:** 2026-05-03
**Context:** `temper`
**Mode:** plan
**Effort:** large (multi-session, controller-driven)
**Branch (separate from Wave 1):** TBD; not to be merged into `jct/wave1-shared-execution-paths-and-cloud-first-reframe`

**Related work:**
- Upstream architectural backbone: `2026-05-01-shared-core-execution-paths-design.md` (the Backend / Surface / pure-actions layer this spec rides on).
- Bug ticket fixed earlier today: `2026-05-03-resource-update-via-cli-strips-yaml-frontmatter-and-glues-h1-to-next-heading` — the surfacing of the show-cache tier-2 dead-code path was the trigger for this work.
- Adjacent (out of scope): `2026-04-29-fix-duplicate-h1-when-temper-resource-create-body-starts-with-h1` (server-side body H1-glue, separate fix), and the `temper-updated` client-vs-server clock disagreement (becomes moot once tier-2 uses hashes).

---

## Problem

`Frontmatter::managed_meta` and the seven JSON schemas in `crates/temper-core/schemas/*.json` disagree about which keys live in the managed tier:

- `ManagedMeta.title` and `ManagedMeta.slug` (in `crates/temper-core/src/types/managed_meta.rs`) have **no** `serde(rename)` — they serialize as bare `title:` / `slug:`. Schemas have them as bare too.
- `TIER1_SYSTEM_FIELDS` (in `crates/temper-core/src/frontmatter/fields.rs`) does **not** include `title` or `slug` — both are part of the canonical-form `managed_hash` computation on the local side.
- The server-side ingest path strips only `IDENTITY_FIELDS` + `TIER1_SYSTEM_FIELDS` from incoming managed_meta JSONB (`crates/temper-api/src/services/ingest_service.rs:100-115`), then extracts `title`/`slug` into the `kb_resources.title` / `kb_resources.slug` columns, dropping them from the JSONB. Server-side `managed_hash` is computed over the JSONB without `title`/`slug`.
- Result: local `managed_hash` includes `title`/`slug`, server `managed_hash` excludes them — **guaranteed mismatch**, every time. Show-cache tier-2 (the hash-based skip) has therefore been dead code since inception. Tier-3 fires every show, which is what surfaced the corruption bug fixed today.

The deeper issue is structural. Different write paths populate different default fields, the field-set is not single-source-of-truth, and the canonical form is not invariant across surfaces. CLAUDE.md's "schema-required-defaults at create/update, not later" rule was added because of this drift. It papered over symptoms rather than fixing the cause.

`SYSTEM_MANAGED_FIELDS` already has bare `slug` at line 86 of fields.rs — a pre-existing inconsistency the rename will resolve.

`date` (used by session/research/decision/concept schemas, all `required: ["date", ...]`) is a doctype-specific field that has lived in managed_meta because the schema declares it. It is **not** a temper-the-system-managed lifecycle field — it is user/agent-supplied content, like body. Its presence in managed_meta is a misclassification.

## The Reframe

Three rules. The whole spec is downstream of these.

1. **Every managed-tier key has the `temper-` prefix.** `title → temper-title`, `slug → temper-slug`. No bare keys in the managed tier. Schemas in `crates/temper-core/schemas/*.json` are the single source of truth — both for the field set and for which fields are `required`.

2. **`date` is open-tier.** It moves out of managed_meta into open_meta, with a one-time DB migration. Schemas are updated: session/research/decision/concept drop `date` from `properties` and `required`. The CLI write paths that currently insert `date` into managed_meta (research, warmup, graph-index materialize) are updated to insert into open_meta instead. The `temper doctor fix` pass rewrites legacy vault files to match.

3. **`kb_resources.title` and `kb_resources.slug` are projections, not the source.** The JSONB managed_meta is canonical; the columns are app-layer-dual-written from the same `ManagedMeta` struct that produces the JSONB. Both go through `apply_managed_meta_partial` so they cannot drift inside a single update. (See "Why dual-write not generated columns" below.)

These three rules — and only these three — are what the rename mechanically buys. Everything else is plumbing.

### Why dual-write not generated columns

Two layers of "dual-write" need separating:

1. **DB-level (intentional, retained):** `kb_resources.title`/`slug` columns AND `kb_resource_manifests.managed_meta` JSONB both carry the values. The columns are kept for query ergonomics — search facets, list ordering, sync replay all read them directly, and several SQL views depend on them. Replacing them with `GENERATED ALWAYS AS (managed_meta->>'temper-title') STORED` would close drift by construction but require a re-audit of every read path. Dual-write keeps reads semantically identical and confines the change to writes; revisitable as a one-shot additive change later if drift is observed.

2. **Wire-level (acceptable as ergonomic sugar):** `IngestPayload` and `ResourceUpdateRequest` carry top-level `title`/`slug` fields alongside `managed_meta`. After this work, the canonical source of truth is the JSONB; the top-level fields are a convenience for CLI/MCP callers who already have title and slug as scalars and don't want to re-pack them into JSONB themselves. The shared `ensure_managed_identity_keys` helper makes the wire dual-write impossible to skew: send-side and receive-side both run it, and the values come from a single in-memory source on each side. Wire collapse (dropping the top-level fields entirely) was considered and deferred — its blast radius (~50 sites including ts-rs codegen, MCP JsonSchema input shapes, OpenAPI spec, e2e tests) is disproportionate to the correctness gain when the helper already closes the gap.

## Decisions Locked (this session)

| Decision | Choice | Rationale |
|---|---|---|
| Scope of `temper-` prefix | All managed-tier keys, no exceptions | Consistent rule kills the case-by-case judgment that drives drift |
| Fate of `date` | Move to open_meta, one-time migration | `date` is user content, not temper lifecycle; managed_meta becomes the temper-managed shape only |
| `kb_resources.title/slug` | App-layer dual-write from `ManagedMeta` | Smaller risk surface than generated columns; queries stay natural |
| Subagent dispatch | Allowed only after this spec + the action-level inventory + test plan + validation-agent-pass checklist are agreed | Per-task constraint: controller-driven for architectural decisions |

## The Contract

### Field-set rules (post-alignment)

- **Identity tier** (no hash, fixed display order): `temper-id`, `temper-provisional-id`. *(unchanged)*
- **Tier-1 system tier** (server-authoritative; stripped before hash): `temper-context`, `temper-type`, `temper-created`, `temper-updated`, `temper-owner`, `temper-source`, `temper-legacy-id`. *(unchanged)*
- **Managed tier** (typed, hashed, schema-validated): `temper-title`, `temper-slug`, `temper-stage`, `temper-mode`, `temper-effort`, `temper-goal`, `temper-seq`, `temper-branch`, `temper-pr`, `temper-status`, `temper-provenance`, `temper-llm-model`, `temper-llm-run`. *(was bare `title`, `slug`; rest unchanged)*
- **Open tier** (free-form, hashed separately): `date`, plus all relationship and tag fields, plus user extras. *(date moves here)*

### What the `ManagedMeta` struct emits

```rust
pub struct ManagedMeta {
    #[serde(rename = "temper-type", skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    // ... existing typed fields keep their renames ...

    #[serde(rename = "temper-title", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(rename = "temper-slug", skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,

    // `date` is removed entirely — it's no longer a managed field.
}
```

### What the schemas declare (post-alignment)

- `base.schema.json`: `temper-title` replaces `title` in `properties` and in `required`.
- `task.schema.json`, `goal.schema.json`, `research.schema.json`, `decision.schema.json`, `concept.schema.json`: `temper-slug` replaces `slug` in `properties` and `required`.
- `session.schema.json`, `research.schema.json`, `decision.schema.json`, `concept.schema.json`: `date` removed from `properties` and `required`.
- All seven schemas: `additionalProperties: true` continues to allow open-tier fields (including `date` post-migration).

### Alias normalization (transition window)

`crates/temper-core/src/frontmatter/parse.rs::normalize_aliases` already implements the alias-rewrite-at-parse pattern for open-field hyphen forms (`relates-to → relates_to`). The same pattern is extended to the managed tier:

| Legacy key | Canonical key |
|---|---|
| `title` | `temper-title` |
| `slug` | `temper-slug` |

Vault files written before the rename keep parsing because the alias normalizer rewrites at parse boundary. The transition window has a defined end: `temper doctor fix` rewrites legacy vault files in-place to canonical form, and the alias entries can be removed in a follow-up release once a vault audit confirms no legacy keys remain. **Until then they stay** — the goal is intelligent recovery, not a sharp cliff.

### Hash invariants (the load-bearing one)

For any resource at rest:
```
local_managed_hash == server_managed_hash
```
This is the invariant that makes show-cache tier-2 functional. After alignment:

- **Local** computes `managed_hash` over the canonical-form JSONB containing `temper-title` and `temper-slug` (after stripping `IDENTITY_FIELDS` + `TIER1_SYSTEM_FIELDS`).
- **Server** computes `managed_hash` over the **same** canonical-form JSONB. The columns are projections; they do not enter the hash.
- The current server-side path that extracts `title`/`slug` out of JSONB into columns and recomputes hash on the stripped form is reversed: extraction is non-destructive (read column-side from JSONB after persisting), and hash is computed pre-extraction.

The hash invariant is the spec's primary acceptance test. Phase 8's restored e2e test (`tier2_hits_when_local_hashes_match_server_hashes`) is the regression gate.

## Surface-Area Inventory

Per the Wave 1 design, four surfaces dispatch to two backends through the `temper-core::operations` layer. **The temper-prefix alignment is enforced exclusively at the operations layer** (typed `ManagedMeta`, pure `merge_managed_meta`, `apply_defaults`, schema validation) — surfaces are thin adapters; backends are responsible for persistence. The table below names, per (surface, verb), the path the field-set invariants flow through.

Legend: `→ ops` means the path goes through `temper-core::operations`. `legacy` means the path is currently in a legacy CLI command module (Wave 1 will retire those; until it does, the legacy path needs to stay aligned).

| Surface ↓ \ Verb → | create | show | update | delete | list |
|---|---|---|---|---|---|
| `CliLocalVault` | `commands/resource.rs::create` `legacy` → `actions/frontmatter::build_managed_meta_for_create` (`→ ops::apply_defaults`); writes vault file via `actions/frontmatter`; tail push to `DbBackend.create_resource` via `temper-client` | `commands/resource.rs::show` `legacy` → reads vault; debounce-cache via `actions/show_cache`; tier-3 reconstructs via `Frontmatter::set_managed_meta` (typed) | `commands/resource.rs::update` `legacy` → `Frontmatter::set_managed_meta` (typed); body via `actions/body_source`; tail push to `DbBackend.update_resource` | `commands/resource.rs::delete` `legacy` → API soft-delete first via `temper-client`; vault rm + manifest entry clear as tail | `commands/resource.rs::list` `legacy` → reads manifest; no managed_meta write |
| `CliCloud` | same `commands/resource.rs::create` flow but cloud branch → `temper-client::ingest::ingest()` → API `/api/ingest` → server `DbBackend.create_resource` `→ ops` | cloud branch → `temper-client::resources::get_by_slug` → API `resource_service::get_by_slug` `→ ops` | cloud branch → `temper-client::resources::patch` → API `resource_service::update` (`→ ops::apply_managed_meta_partial`) | cloud branch → `temper-client::resources::delete` → API `resource_service::delete` `→ ops` | cloud branch → `temper-client::resources::list` → API `resource_service::list_visible` `→ ops` |
| `Mcp` | `temper-mcp::tools::resources::create_resource` → in-process `ingest_service::ingest` (`→ ops::apply_defaults`) | `temper-mcp::tools::resources::get_resource` → in-process `resource_service::get_by_slug`/`get_visible` `→ ops` | `temper-mcp::tools::resources::update_resource` (full) and `update_meta` (partial) → in-process `resource_service::update` / `meta_service::update_meta` (`→ ops::apply_managed_meta_partial`) | `temper-mcp::tools::resources::delete_resource` → in-process `resource_service::delete` `→ ops` | `temper-mcp::tools::resources::list_resources` → in-process `resource_service::list_visible` `→ ops` |
| `ApiHttp` | `handlers::ingest::post` → `ingest_service::ingest` (`→ ops::apply_defaults`) | `handlers::resources::get_by_slug` → `resource_service::get_by_slug` `→ ops` | `handlers::resources::patch` → `resource_service::update` (`→ ops::apply_managed_meta_partial`) | `handlers::resources::delete` → `resource_service::delete` `→ ops` | `handlers::resources::list` → `resource_service::list_visible` `→ ops` |

**Convergence reading:**
- For Mcp / ApiHttp / CliCloud, **every CRUD verb already converges on the operations layer.** The temper-prefix work touches one place (the typed `ManagedMeta` + the schemas + the pure actions) and all three surfaces inherit the new contract.
- For CliLocalVault, the legacy command modules (`commands/resource.rs`) are mid-migration to `VaultBackend` (Wave 1 Phase 3). Until that lands, `commands/resource.rs` is a parallel write path that has its own copy of the temper-prefix invariants. Phase 4 of this work updates the legacy module's templates and write helpers; Wave 1 Phase 3 retires the duplication.

**Confirmed today (verified by code reading):**
- `temper-core::operations::actions::merge_managed_meta` (line 107) covers all typed fields including `title` and `slug`. Renaming the `ManagedMeta` struct fields' serde renames is one edit; the merge function does not need separate updates.
- `apply_defaults` (operations/actions.rs:34-42) delegates to `temper_core::defaults::apply_doc_type_defaults`. Single shared defaults point.
- `validate_create` (operations/actions.rs:193) requires `cmd.title` (a positional command field, not the `ManagedMeta.title`). This is a command-shape concern; renaming the wire format does not change command-shape requirements.

**Not yet verified (open in section "Open questions"):**
- Whether `crates/temper-cli/src/templates/*.md` (askama templates) emit bare `title:` / `slug:` or already use `temper-` keys. Phase 4 read-and-edit pass.
- Whether `temper-mcp::tools::resources` parameter shapes need updating (managed_meta is typed, so probably no) or whether tool descriptions reference bare field names in user-facing strings.
- Where in the server SQL the `kb_resources.title/slug` column extraction currently happens — needed to write Phase 5/6 with confidence.

## Migration Phase Plan

Each phase ships in its own commit. Each phase has an explicit testable success criterion. **Phases marked `[controller]` are user-in-the-loop, no subagent dispatch. Phases marked `[subagent-OK]` may use a subagent with a byte-equivalent spec, only after this design doc is approved.**

| Phase | Title | Mode | Success criterion |
|---|---|---|---|
| 0 | Spec (this doc) | `[controller]` | Approved by user; merged to main as the durable artifact for the work |
| 1 | Schemas + alias entries | `[subagent-OK]` | All 7 schemas updated; `LEGACY_FIELDS` includes `(title, temper-title)`, `(slug, temper-slug)`; alias-normalizer extended for managed tier; new fixture-vault test verifies bare-title/bare-slug files still parse to typed `ManagedMeta` |
| 2 | `ManagedMeta` serde renames | `[controller]` | `title` and `slug` get `serde(rename = "temper-title"/"temper-slug")`; `TIER1_SYSTEM_FIELDS` and `KNOWN_TEMPER_FIELDS` updated; `SYSTEM_MANAGED_FIELDS` corrects `slug → temper-slug`; existing unit tests pass with new keys; `cargo make check` clean. **Phases 1+2 may need to ship together** — if Phase 1 schemas update `required` to `temper-title` but Phase 2's typed struct still emits bare `title`, server validation fails. Confirm this in Phase 1's local test run; if so, merge phases. |
| 3 | Canonical-form display + hash | `[controller]` | `frontmatter/canonical.rs` lines 61-66 updated; doc-type-schema-property-order pass merges in `temper-title`/`temper-slug` from base.schema.json (or stays explicit, decided in this phase); hash-determinism tests still pass; existing canonicalize-idempotent tests still green |
| 4 | CLI write paths + askama templates | `[subagent-OK]` | Templates emit `temper-title:` / `temper-slug:` / no `date:` in managed-tier; `actions/frontmatter::build_managed_meta_for_create` and friends emit canonical keys; `commands/resource.rs::validate_doc_type` removed (defer to `validate_doctype` in operations); golden-file CLI test verifies emitted frontmatter |
| 5 | Server-side: canonical projection-key injection | `[controller]` | `temper-core::operations::ensure_managed_identity_keys` injects `temper-title`/`temper-slug` into `managed_meta` JSONB from the top-level identity fields. Called on both send-side (CLI/MCP build paths) and receive-side (`ingest_service::ingest`, `resource_service::update`) for defense in depth. New service test verifies `managed_meta` JSONB contains `temper-title`/`temper-slug` post-ingest; new integration test asserts `local_managed_hash == server.managed_hash` for any newly-ingested resource. `kb_resources.title`/`slug` columns continue to be populated from top-level fields. |
| 6 | DB migration: rename keys + recompute hashes | `[controller]` | New migration in `migrations/`: existing rows have managed_meta JSONB rewritten to use `temper-title`/`temper-slug` keys, `date` extracted from managed_meta into open_meta, `managed_hash` and `open_hash` recomputed for all rows. Migration is hash-invalidating and explicit. Manifest replay test passes against migrated DB. |
| 7 | Read-side cleanup | `[controller]` | Every consumer reads canonical keys (alias normalization at parse still tolerates legacy on read); `commands/session.rs:454`, `actions/ingest.rs:73`, `materialize.rs`, `research.rs`, `warmup.rs` all read `date` from open_meta. Service tests verify each path. |
| 8 | Re-enable hash-based tier-2 | `[controller]` | `show_cache.rs::attempt_remote` tier-2 hash compare is re-enabled; the previously-removed e2e regression `tier2_hits_when_local_hashes_match_server_hashes` is restored and passes against a real server. This is the spec's primary acceptance gate. |
| 9 | Vault doctor fix | `[subagent-OK]` | `temper doctor fix` rewrites legacy `title:`/`slug:` keys in user vault files to canonical form; `date:` in session/research/decision/concept managed-tier moves to open-tier in the rewrite. Fixture-vault test covers all four legacy patterns. |

**Working tests at every phase boundary.** If a phase doesn't compile until the next phase lands, the boundary moved — merge them. The phases above assume the boundaries hold; Phase 1+2 is flagged as the most likely consolidation.

**Dogfood gate before merge.** Run the DB migration against a fresh-ingested vault, verify `show_cache` tier-2 actually hits — both via the e2e test and via a manual show-show round-trip on a real resource.

### Phase 5 helper contract

`ensure_managed_identity_keys(meta: &mut serde_json::Value, title: &str, slug: &str)`:

- Coerces `meta` to a JSON object if it is not one already (replacing it with `{}` on a non-object value, since the alternative is silently dropping the data).
- Inserts or overwrites `meta["temper-title"] = title` and `meta["temper-slug"] = slug`.
- Idempotent: running twice with the same inputs produces the same output.
- Pure: no I/O, no dependencies beyond `serde_json`.

Callers: send-side runs it after serializing `ManagedMeta → Value` and before `compute_managed_hash` (so the local hash sees what the server will see); receive-side runs it after `strip_system_managed_fields` / `apply_managed_meta_partial` and before the server's own `compute_managed_hash` (so a non-CLI client that skipped injection still produces a canonical row).

## Test Plan

Coverage matrix. Every (surface, verb) row in the inventory has at least one row here.

### Unit tests (no DB)

Located in the relevant `temper-core` modules. Run via `cargo make test`.

| Concern | File | Test names |
|---|---|---|
| `ManagedMeta` round-trip with new keys | `types/managed_meta.rs` | `managed_meta_serde_roundtrip` and `managed_meta_yaml_roundtrip` updated to assert `temper-title`/`temper-slug` |
| Alias normalization at parse | `frontmatter/parse.rs` | New: `normalize_aliases_rewrites_managed_tier_legacy_keys` covering `title → temper-title`, `slug → temper-slug` |
| Canonical-form ordering | `frontmatter/canonical.rs` | Existing `title_comes_before_slug_in_managed_tier` updated to use new keys; new test covers backward-compat parse-then-canonicalize chain |
| `apply_defaults` + new schema set | `operations/actions.rs` | Existing `apply_defaults_task_sets_stage_when_missing` covers; add session-doctype test confirming `date` no longer auto-populates managed |
| `merge_managed_meta` covers `temper-title` and `temper-slug` | `operations/actions.rs` | Existing `merge_managed_meta_covers_all_typed_fields` updated |

### Integration tests (DB-backed)

Located in `crates/temper-api/tests/`. Run via `cargo make test-db` (requires `cargo make docker-up`).

| Concern | File (existing or new) | Test names |
|---|---|---|
| Ingest stores `temper-title`/`temper-slug` in JSONB and projects to columns | `crates/temper-api/tests/resources_test.rs` | New: `ingest_stores_temper_title_in_managed_meta_jsonb_and_column_projection_matches` |
| `apply_managed_meta_partial` preserves `temper-title` and `temper-slug` across partial update | `resource_update_merge_test.rs` | New: `partial_update_with_only_temper_stage_preserves_temper_title_and_temper_slug` |
| Hash invariant under round-trip | new file: `crates/temper-api/tests/managed_hash_invariant_test.rs` | `managed_hash_is_byte_identical_to_local_canonical_form` (test fixture: insert resource, fetch back via meta_service, compute local hash from response, assert equality) |
| `date` ingest path puts date in open_meta | `resources_test.rs` | New: `session_ingest_with_date_routes_date_to_open_meta_not_managed` |

### E2E tests

Located in `tests/e2e/tests/`. Run via `cargo make test-e2e`. These are the trustworthy cross-surface contracts.

| Concern | File (existing or new) | Test names |
|---|---|---|
| **The primary acceptance gate** | `tests/e2e/tests/show_cache_e2e_test.rs` (existing) | Restore: `tier2_hits_when_local_hashes_match_server_hashes` |
| Cloud-create round-trips to local-clone with byte-identical canonical form | `cloud_writes_test.rs` (existing) | New: `cloud_create_then_local_show_emits_temper_title_and_temper_slug` |
| MCP create + API show emit identical managed_meta | `mcp_round_trip_test.rs` (existing) | New: `mcp_create_resource_round_trip_yields_temper_prefixed_keys` |
| Migration replay against fresh-ingested vault | new file: `tests/e2e/tests/managed_meta_migration_test.rs` | `legacy_managed_meta_jsonb_rewrites_correctly` (seeds DB with old shape, runs migration, asserts JSONB shape + hashes) |
| Vault doctor fix on a fixture vault with legacy keys | new file: `tests/e2e/tests/doctor_fix_managed_alignment_test.rs` | `doctor_fix_rewrites_bare_title_and_slug_to_temper_prefix` and `doctor_fix_moves_session_date_from_managed_to_open` |

## Validation-Agent-Pass Checklist

A subagent dispatched against any phase MUST be told to run this exact checklist before reporting completion. Failure of any step → BLOCKED, not workaround.

```
1. cargo make check                                                      # fmt + clippy + machete + ts-typecheck + biome
2. cargo make test                                                       # unit tests, no DB
3. cargo make docker-up && cargo make test-db                            # integration tests against real Postgres
4. cargo make test-e2e                                                   # e2e tests including restored tier-2 regression
5. grep -rn '"title":\|"slug":\|set_managed_field("title"\|set_managed_field("slug"' crates/  # → expected zero hits in production code (test fixtures and aliases module exempted)
6. grep -rn 'set_managed.*"date"\|"date".*managed' crates/               # → expected zero hits (date moved to open_meta)
7. cargo sqlx prepare --workspace -- --all-features                      # regenerate offline cache; commit if dirty
8. Run `temper resource show <slug>` against a real cloud resource; verify show-cache tier-2 hits on the second show (no remote call). Capture stderr proof.
9. Read the diff for the phase. Confirm: NO "for now" comments. NO "until X reconciled" comments. NO new TODOs without ticket links. (See `feedback_no_ship_for_now_workarounds.md`.)
```

The checklist is mechanical. If a step fails, the subagent reports BLOCKED with the failing output verbatim. It does NOT silently soften, refactor, or skip — that pattern is what `feedback_subagent_escalate_not_soften.md` exists to prevent.

## Open Questions

Flagged here so future-me can resolve before each phase rather than mid-phase.

1. **Phase 1+2 consolidation:** if the schemas declare `required: [temper-title, temper-slug]` but the typed struct still emits bare keys for one commit, server validation fails. Confirm in a local-only test that the boundaries hold; if not, merge to one commit.

2. **Canonical-form schema-property-order pass:** `canonical.rs::schema_property_order` only reads the doc-type schema's top-level `properties`, not the merged closure across `allOf → base.schema.json`. After moving `temper-title`/`temper-slug` into base, the schema-order pass would skip them. Two options: (a) keep the explicit pre-list of `temper-title`/`temper-slug` (today's pattern, lines 61-66, just renamed), or (b) teach `schema_property_order` to follow `allOf`. Decision in Phase 3.

3. **Server-side title/slug column extraction site (RESOLVED 2026-05-04):** Investigation showed `kb_resources.title`/`slug` are NOT extracted from `managed_meta` JSONB — they are populated from top-level `IngestPayload.title`/`slug` and `ResourceUpdateRequest.title`/`slug` fields. The asymmetry is the actual hash-invariant gap: the JSONB never had `temper-title`/`temper-slug` server-side, while the local canonical form does. Resolution: a shared `temper-core::operations::ensure_managed_identity_keys` helper injects the keys into the JSONB from the top-level fields, run on both the send side (CLI / MCP) and the receive side (`ingest_service::ingest`, `resource_service::update`) for defense in depth. See Phase 5 plan: `docs/superpowers/plans/2026-05-04-managed-meta-phase5-canonical-projection-injection.md`.

4. **MCP tool description strings:** several tool descriptions reference bare field names in user-facing copy. Audit during Phase 4 — schema-validated wire format is already typed, but agent-readable strings may need updating to match the canonical contract.

5. **Alias-normalizer end-of-life:** the legacy `(title, temper-title)` and `(slug, temper-slug)` alias entries can stay for one or more releases until vault audits show no legacy keys remaining in user vaults. Define the audit (probably a `temper doctor scan` mode) and the removal trigger as a follow-up issue, not part of this work.

## Out of Scope

- The H1-glue body bug (`# Title## Section` server-stored data). Same-family but separate ticket: `2026-04-29-fix-duplicate-h1-when-temper-resource-create-body-starts-with-h1`.
- The `temper-updated` client-vs-server clock disagreement. Becomes moot for show-cache once tier-2 uses hashes; the cosmetic cross-clock issue stays, separate ticket if user-facing.
- Wave 1 Phase 3 (DBBackend implementation). This work runs alongside; legacy CLI command paths kept aligned during the transition; Wave 1 retires them.
- Generated columns for `title`/`slug` (revisitable after this work lands if dual-write drift is observed).
