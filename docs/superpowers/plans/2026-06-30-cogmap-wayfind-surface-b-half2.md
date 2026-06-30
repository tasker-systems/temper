# Cogmap Wayfinding Surface B — Half 2 Implementation Plan (`--wayfind` region-salience funnel)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. Follow `~/.claude/skills/temper/subagent-guidance.md` and the project fundamentals verbatim in every subagent prompt.

**Goal:** When the agent does **not** name a single map, `temper search --wayfind [--lens <ref>] [--regions N] <query>` runs the lens-driven discovery pass: visible maps → region-salience trace (`α·salience_norm + β·query_centroid_cosine`) → top-N regions → visibility-gated members → the **existing** `unified_search` blend over that scope. Region-less / thin maps degrade silently to whole-map direct scope. Gated at every stage; green under `cargo make test-artifacts` + e2e.

**Architecture:** Same throughline as Half 1 — the FTS+vector+graph blend (`unified_search`) is scope-agnostic; Surface B is only a *scope-resolution front end*. Half 1 added `p_scope_ids uuid[]` to `unified_search` and `cogmap_scope_ids` for single-map scope. Half 2 adds **one** new scope-resolution path: a `wayfind_scope_ids(...)` SQL function that returns the bounding `resource_id` set, threaded through the same `SearchParams → search_select → UnifiedSearchQuery.scope_ids → p_scope_ids` seam. **No change to `unified_search` itself.** New SQL is limited to scope-resolution functions; the blend, fusion, and ranking are Beat-2 code reused verbatim.

**Tech Stack:** Rust (sqlx, axum, clap, rmcp/MCP), PostgreSQL 17/18 + pgvector, sqlx migrations.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-29-search-substrate-beat3-surface-b-wayfinding-design.md` §4 (the wayfind funnel) + §5 (cold-start) + §7 (gating) + §8 (tuning) + §9 (acceptance). Half 1 plan (the pattern to mirror): `docs/superpowers/plans/2026-06-29-cogmap-wayfind-surface-b-half1.md`.
- **Migrations are immutable once shipped.** Never edit `20260624*`–`20260629000005*`. Add NEW files only. Next free name: **`20260629000006_*`**.
- **`--all-features` for every build/clippy/check.** `cargo make check` runs `SQLX_OFFLINE=true` against committed `.sqlx/` caches.
- **sqlx caches:** `wayfind_scope_ids` is called via **runtime `query_as`** (takes a `vector` param — the `::vector` cast forbids the macro), so it needs **no** `.sqlx` cache entry, exactly like `unified_search`. Keep all new vector-bearing calls runtime. If a non-vector `query_scalar!`/`query!` macro is added, run the ritual: `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` → `cargo make prepare-api`. After adding the migration, `cargo clean -p temper-substrate` (and `-p temper-api`) so `sqlx::migrate!()` picks up the new file (memory `project_sqlx_migrate_macro_stale_cache`).
- **Auth before reads-from-nowhere (§7):** every stage gates — map admission via team-cogmap membership, region read excludes folded, member dereference through `resources_visible_to`, and `unified_search` re-gates inside each candidate fn (belt + suspenders).
- **Surfaces dispatch through the service read path** — the wayfind scope resolves in `search_select` (temper-services), the SQL lives in temper-substrate readback; handlers/tools/CLI stay thin and forward `SearchParams` unchanged (the MCP/API surfaces are **zero-touch** beyond the wire fields).
- **Typed structs / parse-don't-validate:** lens ref resolves client-side to a `LensId` (trailing-UUID `parse_ref`), like `--cogmap`. Tuning constants are SQL-resident in one leading CTE (mirroring `unified_search`'s `k` CTE), never API/CLI params.
- **Test tiers:** deterministic funnel tests in `temper-substrate/tests` (artifact-tests, `cargo make test-artifacts`); surface/access-semantics tests in `temper-api/tests` (test-db) driving real `POST /api/search`; **e2e before push** per `feedback_access_semantics_changes_need_e2e_tier` (the deny→zero gating is access-semantics — test-db alone is a false signal). Rebuild/reinstall the `temper` bin before e2e (`project_e2e_stale_temper_bin`).

## In scope this session — the multi-author read RBAC (Task A0, decided 2026-06-30)

The carried-forward boundary from Half 1 — `resources_visible_to` has no cogmap-membership clause, so cogmap search returns only the searcher's **own** contributions on multi-author maps — is **fixed in this session, first** (owner decision: a team joined to a cogmap should confer read access to the resources homed in it, exactly as it already confers read access to the *map*; otherwise multi-author maps are pointless). It is a small, additive, well-understood extension: the resource-grain mirror of the team-owned-context fix already in `20260627000002_team_owned_context_resource_visibility.sql`, membership-flat to match `cogmap_readable_by_profile`. Doing it first means the rest of the suite asserts correct (peer-visible) semantics instead of pinning fail-closed and flipping. See **Task A0**.

## Deferred / out of scope (explicit)

- **The producer-side WRITE predicate for `--cogmap` authorship.** Still deferred to the cogmap-arc RBAC (composes with steward-invocation scoping), per spec §10. Half 1's `cogmap_authorable_by_profile` seam stands. Task A0 touches *reads* only.
- **Versioning α/β in a table (YAGNI, named exit).** Tuning constants stay SQL-resident (spec §4.1 + the `unified_search` `k`-CTE precedent). The "evolve without a fresh code deploy" goal is already met: changing α/β is a new additive migration that `DROP+CREATE`s the function — not a Rust rebuild — so it's already operator-governed, per-target, git-versioned, review-gated, and re-runs the deterministic regression in CI. A mutable table would spend exactly the accountability guardrail (ill-understood tweaks) for marginal speed. **If** the calibration machinery (below) later shows we need *online* experimentation, the home is the **lens row** (already event-sourced via `lens_create`, already carries the salience `s_*` weights) — add blend weights there, not a free table. Build nothing now; decision gated on the eval machinery existing.
- **A centroid HNSW index.** Region counts are clustering outputs (tens, not millions); the per-map centroid seq scan is cheap (spec §4.1, OQ-2). Additive follow-up only if eval shows it's a bottleneck.
- **True production-corpus calibration of α/β.** Defaults are **spec-reasoned and conservative** (see Task A), and the deterministic tests pin the funnel's qualitative behavior (the §9 regressions). Empirical re-tuning against the live temperkb.io corpus is flagged as a measurement follow-up (Task D) — **not silently claimed done** (there is no rich region corpus locally; the L0 kernel is born region-less). Can't begin until the machinery here is in place.

---

## File Structure

**New files:**
- `migrations/20260629000006_cogmap_resource_visibility.sql` — additive cogmap-membership clause on `resources_visible_to` (Task A0).
- `migrations/20260629000007_wayfind_scope.sql` — `cogmap_visible_maps(principal)` + `wayfind_scope_ids(principal, lens, emb, regions_n)`.
- `crates/temper-substrate/tests/cogmap_wayfind_scope.rs` — deterministic funnel tests (region selection, sparse-beats-large regression, normalization, cold-start, deny). Artifact-tests tier.
- `crates/temper-api/tests/cogmap_wayfind_test.rs` — surface tests over `POST /api/search` with `wayfind=true` (scopes into regions, cold-start whole-map, deny→zero, multi-author fail-closed doc test). test-db tier.

**Modified files (by task):**
- Task A: `crates/temper-substrate/src/readback/mod.rs` (new `WayfindScopeQuery` params struct + `wayfind_scope_ids` runtime fn).
- Task B: `crates/temper-core/src/types/api.rs` (`SearchParams.{wayfind,lens_id,regions}` + `Default`), `crates/temper-services/src/backend/substrate_read.rs` (`search_select` third scope path + mutual-exclusion).
- Task C: `crates/temper-cli/src/cli.rs` (`--wayfind`/`--lens`/`--regions`), `crates/temper-cli/src/main.rs` (destructure + forward), `crates/temper-cli/src/actions/search.rs` (`CliSearchArgs` fields + `build_search_params` mapping + mutual-exclusion), `crates/temper-cli/src/commands/search_cmd.rs` (re-bundle). MCP/client: **no change** (forward `SearchParams`).

---

## Task A0 — Multi-author read RBAC: cogmap-membership clause on `resources_visible_to`

**Files:**
- Create: `migrations/20260629000006_cogmap_resource_visibility.sql`
- Modify (flip): `crates/temper-api/tests/cogmap_home_test.rs:746` (`cogmap_search_excludes_unowned_peer_resource_pending_rbac` → asserts inclusion)

**Why first:** doing the RBAC fix before the funnel means every downstream test asserts the correct peer-visible semantics. The change is additive (only *adds* visibility — non-members still see nothing), membership-flat to match `cogmap_readable_by_profile`, and the exact resource-grain mirror of the team-owned-context clause already in `20260627000002` (lines 58-67).

**Interfaces:**
- Produces (SQL): `resources_visible_to(p_profile)` gains a UNION branch — resources homed in a cogmap joined to a team the principal is a member of. Resolution is `kb_team_cogmaps ∩ profile_effective_teams(p_profile)` (NOT the ancestor-expanded `reachable_teams` the context clauses use — membership-flat, so "a map you can read" and "the resources homed in it" agree by construction, per spec §7).

- [ ] **Step 1: Write/flip the failing test.** Rename `cogmap_search_excludes_unowned_peer_resource_pending_rbac` → `cogmap_search_includes_peer_resource_on_shared_map` and invert its assertion: a second profile **also a member of the map's team** now **sees** the peer's cogmap-homed resource via `--cogmap` search. (If the original test's second profile was deliberately a non-member, add a member peer; keep a separate non-member→zero assertion.) Run — expect FAIL (peer still excluded).

  Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test cogmap_home_test peer_resource`

- [ ] **Step 2: Write the migration** `migrations/20260629000006_cogmap_resource_visibility.sql` — `CREATE OR REPLACE FUNCTION resources_visible_to(p_profile uuid)` copying the **entire current body** from `20260627000002_team_owned_context_resource_visibility.sql:31-68` verbatim, then append one UNION branch before the final `;`:

  ```sql
  UNION
  -- cogmap membership: resources homed in a cognitive map joined to a team the
  -- principal is a member of (resource-grain mirror of cogmap_readable_by_profile —
  -- membership-flat, so map-read and resource-read agree by construction).
  -- Additive false-negative fix: non-members still see nothing.
  SELECT h.resource_id
  FROM kb_team_cogmaps tc
  JOIN profile_effective_teams(p_profile) e ON e.team_id = tc.team_id
  JOIN kb_resource_homes h
    ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
  ```
  > Keep `LANGUAGE sql STABLE` (compile-checked `sqlx::query!` callers depend on it). Transcribe the existing branches exactly — only the new branch is added.

- [ ] **Step 3: Verify `resources_readable_by` composes (no twin gap).** Read `resources_readable_by` (`migrations/20260624000002_canonical_functions.sql:244+`): its **profile** arm must delegate to / union `resources_visible_to` so this fix propagates to cogmap-principal read paths. If the profile arm inlines its own predicate set (not calling `resources_visible_to`), it has the same gap — add the parallel cogmap branch there too in this migration. (The cogmap **principal** arm is a separate concern — a cogmap reading its own members — and is out of scope.) Record which case held.

- [ ] **Step 4: `cargo clean -p temper-api` then run the flipped test + the L0-visibility ripple.** The L0 kernel telos (`…0005-000000000002`) is homed in the L0 cogmap, which is bound to the auto-join `temper-system` team — so **every approved profile now sees the L0 telos at the resource grain** (correct: it's the public kernel; this also closes a latent gap). Surface A context search is unaffected (it filters `anchor_table='kb_contexts'`), but any test asserting a fresh profile's *exact* visible-resource set or count may shift — find and fix those (don't weaken assertions; update the expected set to include the now-correctly-visible L0 telos).

  Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test cogmap_home_test` then a broad `cargo nextest run -p temper-api --features test-db` to catch ripple. Expected: flipped test PASSES; any count-shift failures updated to the correct expectation.

- [ ] **Step 5: Commit**

  ```bash
  git add migrations/20260629000006_cogmap_resource_visibility.sql crates/temper-api/tests/cogmap_home_test.rs
  git commit -m "Surface B Half 2 Beat A0: cogmap-membership read clause on resources_visible_to (multi-author maps)"
  ```

---

## Task A — Scope-resolution SQL: `cogmap_visible_maps` + `wayfind_scope_ids` (the funnel)

**Files:**
- Create: `migrations/20260629000007_wayfind_scope.sql`
- Modify: `crates/temper-substrate/src/readback/mod.rs` (add `WayfindScopeQuery<'a>` near `UnifiedSearchQuery` ~1091; add `wayfind_scope_ids` runtime fn near `unified_search` ~1110)
- Test: `crates/temper-substrate/tests/cogmap_wayfind_scope.rs` (new)

**Interfaces:**
- Produces (SQL):
  - `cogmap_visible_maps(p_principal uuid) RETURNS SETOF uuid` — maps the principal can read (the set form of `cogmap_readable_by_profile`: `kb_team_cogmaps ∩ profile_effective_teams(principal)`).
  - `wayfind_scope_ids(p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int) RETURNS SETOF uuid` — the bounding resource-id set: (members of the top-N pooled regions across visible maps) ∪ (direct homed participants of region-less/thin maps), all visibility-gated. Tuning constants (`α`, `β`, default/max N, thin threshold, recall-floor, normalization) live in the leading `k` CTE. `NULL` lens → use each region's memoized `salience`; non-null lens → recompute salience from stored components under the override's `s_*`. `NULL` emb → β term zeroed (salience-only). `NULL`/over-ceiling N → clamped to CTE defaults.
- Produces (Rust): `WayfindScopeQuery<'a> { principal: Uuid, lens_id: Option<Uuid>, embedding: Option<&'a [f32]>, regions: Option<i32> }`; `pub async fn wayfind_scope_ids(pool, q) -> Result<Vec<Uuid>>` binding `p_emb` as text into a `::vector` cast (the `unified_search` runtime pattern at readback/mod.rs:1110-1132, `format_pgvector` at 734-745).
- Consumes: `kb_cogmap_regions` (centroid/salience/components/is_folded — schema 725-744), `kb_cogmap_region_members` (member_table='kb_resources' — 748-755), `kb_cogmap_lenses.s_*` (694-696), `resources_visible_to`, `kb_resource_homes`, `profile_effective_teams`.

- [ ] **Step 1: Write the failing deterministic tests** `crates/temper-substrate/tests/cogmap_wayfind_scope.rs` (`#![cfg(feature = "artifact-tests")]`, `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`). For determinism, **insert region rows directly** with controlled `centroid`/`salience`/components (do NOT depend on `materialize_cogmap` or the ONNX model for cosine values) — seed a profile, a team, `kb_team_members` + `kb_team_cogmaps`, a cogmap (genesis helper), an event id (for the NOT NULL `asserted_by_event_id`/`last_event_id`), the global `telos-default` lens id, and `kb_cogmap_regions` + `kb_cogmap_region_members` rows with hand-chosen 768-d centroids. Helper to build a unit-ish vector pointing "toward" a query so cosine is controllable (e.g. query=`[1,0,0,…]`, region-A centroid≈`[0.95,…]` high cosine, region-B centroid≈`[0.1,…]` low cosine).

  Tests:
  ```
  // 1. top-N selection: 3 regions, regions=2 → only the 2 top-scoring regions' members in scope.
  async fn wayfind_selects_top_n_regions()
  // 2. THE §9 REGRESSION: region B is thin (1 member, low salience) but high query-cosine;
  //    region A is large (many members, high salience) but low query-cosine. With regions=1,
  //    B's member is in scope and A's members are NOT. (Proves relevance buys a top-N slot.)
  async fn sparse_high_cosine_region_beats_large_low_cosine()
  // 3. cold-start: a region-less map in the visible set contributes its direct homed participants
  //    (the cogmap_scope_ids fallback), never errors; regions=N is a silent no-op for it.
  async fn region_less_map_degrades_to_direct_scope()
  // 4. deny: a principal NOT in the map's team gets zero ids (no view from nowhere).
  async fn wayfind_excludes_unreadable_maps()
  // 5. lens override recompute: same regions, an override lens with s_central heavily weighted
  //    reorders selection vs the default lens (proves recompute-from-components, not lens_id filter).
  async fn lens_override_recomputes_salience_from_components()
  ```

- [ ] **Step 2: Run — expect compile failure** (`WayfindScopeQuery`/`wayfind_scope_ids` don't exist).

  Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-substrate --features artifact-tests wayfind`
  Expected: FAIL — unresolved `wayfind_scope_ids`.

- [ ] **Step 3: Write the migration** `migrations/20260629000007_wayfind_scope.sql`:

  ```sql
  -- Surface B Half 2: the wayfind region-salience scope-resolution funnel (spec §4/§5/§7).
  -- New scope-resolution SQL only; the unified_search blend is unchanged and consumes the
  -- returned id set via p_scope_ids. All tuning constants live in the k CTE (single home,
  -- mirroring unified_search's k CTE — calibrate on the corpus, see spec §8).

  -- The set form of cogmap_readable_by_profile (membership-flat map admission).
  CREATE FUNCTION cogmap_visible_maps(p_principal uuid)
  RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
      SELECT tc.cogmap_id
      FROM kb_team_cogmaps tc
      JOIN profile_effective_teams(p_principal) e ON e.team_id = tc.team_id;
  $$;

  CREATE FUNCTION wayfind_scope_ids(
      p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int)
  RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
    WITH
    k AS (SELECT 0.4::float8 AS alpha,        -- salience weight
                 0.6::float8 AS beta,         -- query-cosine weight (β≥α so relevance can buy a slot, §4.1)
                 3   AS default_n,            -- default --regions
                 20  AS max_n,                -- per-call ceiling
                 0   AS thin_threshold,       -- region-count <= this ⇒ bypass to direct scope (region-less)
                 false AS recall_floor),      -- always admit best-cosine region (default OFF, §4.2)
    n AS (SELECT LEAST(COALESCE(p_regions_n, (SELECT default_n FROM k)),
                       (SELECT max_n FROM k)) AS regions_n),
    vmaps AS (SELECT cogmap_visible_maps(p_principal) AS cogmap_id),
    lens AS (SELECT s_telos, s_ref, s_central FROM kb_cogmap_lenses WHERE id = p_lens),
    -- candidate regions in visible maps; salience = memoized (default) or recomputed (override lens)
    cand AS (
      SELECT r.id, r.centroid,
             CASE WHEN p_lens IS NULL THEN r.salience
                  ELSE (SELECT s_telos FROM lens)   * COALESCE(r.telos_alignment, 0)
                     + (SELECT s_ref FROM lens)     * COALESCE(r.reference_standing, 0)
                     + (SELECT s_central FROM lens) * COALESCE(r.centrality, 0)
             END AS sal_eff
      FROM kb_cogmap_regions r
      WHERE r.cogmap_id IN (SELECT cogmap_id FROM vmaps) AND NOT r.is_folded
    ),
    bounds AS (SELECT min(sal_eff) AS lo, max(sal_eff) AS hi FROM cand),
    scored AS (
      SELECT c.id,
             CASE WHEN (SELECT hi FROM bounds) = (SELECT lo FROM bounds) THEN 1.0
                  ELSE (c.sal_eff - (SELECT lo FROM bounds))
                     / NULLIF((SELECT hi FROM bounds) - (SELECT lo FROM bounds), 0)
             END AS sal_norm,
             CASE WHEN p_emb IS NULL THEN 0.0 ELSE 1 - (c.centroid <=> p_emb) END AS query_cos
      FROM cand c
    ),
    ranked AS (
      SELECT id, query_cos,
             (SELECT alpha FROM k) * sal_norm + (SELECT beta FROM k) * query_cos AS region_score
      FROM scored
    ),
    top_regions AS (
      (SELECT id FROM ranked ORDER BY region_score DESC LIMIT (SELECT regions_n FROM n))
      UNION
      (SELECT id FROM ranked WHERE (SELECT recall_floor FROM k) ORDER BY query_cos DESC LIMIT 1)
    ),
    region_ids AS (
      SELECT m.member_id AS resource_id
      FROM kb_cogmap_region_members m
      WHERE m.region_id IN (SELECT id FROM top_regions)
        AND m.member_table = 'kb_resources'
        AND m.member_id IN (SELECT resource_id FROM resources_visible_to(p_principal))
    ),
    -- cold-start (§5): region-less / thin maps contribute their direct homed participants.
    thin_maps AS (
      SELECT v.cogmap_id FROM vmaps v
      WHERE (SELECT count(*) FROM kb_cogmap_regions r
             WHERE r.cogmap_id = v.cogmap_id AND NOT r.is_folded) <= (SELECT thin_threshold FROM k)
    ),
    direct_ids AS (
      SELECT h.resource_id
      FROM kb_resource_homes h
      WHERE h.anchor_table = 'kb_cogmaps'
        AND h.anchor_id IN (SELECT cogmap_id FROM thin_maps)
        AND h.resource_id IN (SELECT resource_id FROM resources_visible_to(p_principal))
    )
    SELECT resource_id FROM region_ids
    UNION
    SELECT resource_id FROM direct_ids;
  $$;
  ```

  > **Implementer notes:** (a) `<=>` is pgvector cosine **distance**, so cosine similarity = `1 - (centroid <=> p_emb)`. (b) the `recall_floor` UNION branch is `WHERE false` when the knob is off → empty, so it's inert by default; `query_cos` is carried in `ranked` so the branch's `ORDER BY` resolves. (c) confirm `profile_effective_teams` / `resources_visible_to` signatures against `migrations/20260624000002_canonical_functions.sql` before finalizing. (d) verify the `vmaps AS (SELECT cogmap_visible_maps(...) AS cogmap_id)` set-returning-in-SELECT form behaves as a set; if the planner objects, use `SELECT * FROM cogmap_visible_maps(p_principal)`.

- [ ] **Step 4: Add the Rust readback wrapper** in `crates/temper-substrate/src/readback/mod.rs` (next to `unified_search`):

  ```rust
  pub struct WayfindScopeQuery<'a> {
      pub principal: Uuid,
      pub lens_id: Option<Uuid>,
      pub embedding: Option<&'a [f32]>,
      pub regions: Option<i32>,
  }

  /// Surface B Half 2: resolve the wayfind bounding resource-id set (spec §4). Runtime `query_as`
  /// — the `::vector` cast forbids the macro (same exception as `unified_search`). All tuning lives
  /// in the SQL function's `k` CTE, not here.
  pub async fn wayfind_scope_ids(pool: &PgPool, q: WayfindScopeQuery<'_>) -> Result<Vec<Uuid>> {
      let emb_text = q.embedding.map(format_pgvector);
      let ids: Vec<(Uuid,)> = sqlx::query_as(
          "SELECT wayfind_scope_ids($1, $2, $3::vector, $4)",
      )
      .bind(q.principal)
      .bind(q.lens_id)
      .bind(emb_text)
      .bind(q.regions)
      .fetch_all(pool)
      .await?;
      Ok(ids.into_iter().map(|(id,)| id).collect())
  }
  ```
  (Adjust the row-decode to the module's convention — a `SETOF uuid` returns one `uuid` column per row.)

- [ ] **Step 5: `cargo clean -p temper-substrate` then run the tests — expect PASS.**

  Run: `cargo clean -p temper-substrate && DATABASE_URL=… cargo nextest run -p temper-substrate --features artifact-tests wayfind`
  Expected: all five PASS. (If `sparse_high_cosine_region_beats_large_low_cosine` is knife-edge, widen the fixture cosine gap — defaults α=0.4/β=0.6 give a clear margin at cos 0.95 vs 0.1; do NOT weaken the assertion.)

- [ ] **Step 6: Commit**

  ```bash
  git add migrations/20260629000007_wayfind_scope.sql crates/temper-substrate/src/readback/mod.rs crates/temper-substrate/tests/cogmap_wayfind_scope.rs
  git commit -m "Surface B Half 2 Beat A: wayfind region-salience scope-resolution SQL + readback"
  ```

---

## Task B — Wire `--wayfind`/`--lens`/`--regions` onto `SearchParams` + the third scope path in `search_select`

**Files:**
- Modify: `crates/temper-core/src/types/api.rs` (`SearchParams` fields after `cogmap_id` line 78 + `Default` 81-98)
- Modify: `crates/temper-services/src/backend/substrate_read.rs` (`search_select` 325-425: third scope path + mutual-exclusion)
- Test: `crates/temper-api/tests/cogmap_wayfind_test.rs` (new)

**Interfaces:**
- Produces (wire): `SearchParams.wayfind: bool` (`#[serde(default)]`), `SearchParams.lens_id: Option<Uuid>` (`#[serde(default)]`, resolved client-side), `SearchParams.regions: Option<i64>` (`#[serde(default)]`). MCP auto-exposes them via the existing `schemars::JsonSchema` derive (no enums → zero MCP work).
- Consumes: `readback::wayfind_scope_ids` (Task A), `unified_search` `p_scope_ids` (Half 1). `wayfind` is a **third** scope path, mutually exclusive with `context_ref` and `cogmap_id`.

- [ ] **Step 1: Write the failing surface tests** `crates/temper-api/tests/cogmap_wayfind_test.rs` (`#![cfg(feature = "test-db")]`, `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`). Model setup on Half 1's `cogmap_home_test.rs` (cogmap genesis + direct `kb_team_*` seeding + `POST` helpers). Tests:
  ```
  // 1. --wayfind scopes into the principal's visible maps' regions and ranks within.
  //    Create a cogmap-homed resource with FTS term "zwayword"; materialize (or insert) a region
  //    containing it; POST /api/search {wayfind:true, query:"zwayword", embedding:<toward it>} → contains it.
  async fn wayfind_scopes_into_regions()
  // 2. cold-start: --wayfind against a region-less owned map returns the map's homed resources,
  //    never errors. POST {wayfind:true,...} with no regions present → 200, contains the homed resource.
  async fn wayfind_cold_start_returns_whole_map()
  // 3. deny: a principal does NOT see a PRIVATE peer map's content via wayfind. NOTE: in open
  //    mode every profile is auto-joined to temper-system and the region-less L0 kernel is always
  //    in scope (cold-start), so "zero visible maps" is unreachable — assert instead that profile B
  //    (not a member of A's private map's team) gets EMPTY results when wayfinding for A's
  //    private-map-specific FTS term (the L0 telos doesn't contain it). 200, never error.
  async fn wayfind_excludes_private_peer_map_content()
  // 4. mutual exclusion: {wayfind:true, context_ref:Some|cogmap_id:Some} → 400 BadRequest.
  async fn wayfind_with_context_or_cogmap_is_bad_request()
  // 5. multi-author (post-A0): a peer's resource on a shared map the searcher is also a member of
  //    IS returned (proves the A0 cogmap-membership read clause flows through wayfind). A
  //    non-member still gets zero (covered by test 3).
  async fn wayfind_includes_peer_resource_on_shared_map()
  ```

- [ ] **Step 2: Run — expect failure** (`wayfind` field missing / path not wired).

  Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test cogmap_wayfind_test`
  Expected: FAIL (compile: no `wayfind`).

- [ ] **Step 3: Add the wire fields** in `crates/temper-core/src/types/api.rs` after `cogmap_id` (line 78):
  ```rust
      /// Wayfind scope (Surface B Half 2): lens-driven region-salience discovery across the
      /// principal's visible maps. Mutually exclusive with `context_ref` and `cogmap_id`.
      #[serde(default)]
      pub wayfind: bool,
      /// Optional lens override for wayfind region selection (resolved client-side, trailing-UUID).
      /// `None` ⇒ each region's memoized salience under its own lens.
      #[serde(default)]
      pub lens_id: Option<Uuid>,
      /// Top-N regions to scope into for wayfind (default/ceiling are SQL-resident). Ignored unless `wayfind`.
      #[serde(default)]
      pub regions: Option<i64>,
  ```
  Add `wayfind: false, lens_id: None, regions: None` to the `Default` impl (81-98).

- [ ] **Step 4: Add the third scope path** in `crates/temper-services/src/backend/substrate_read.rs` `search_select`. After the existing `context_ref`→`context_id` resolution and *before* the `cogmap_id` arm, branch on `params.wayfind`. Resolve `scope_ids` once from whichever path is active; enforce that **at most one** of `{context_ref, cogmap_id, wayfind}` is set (else `ApiError::BadRequest`). For the wayfind arm:
  ```rust
  // params.wayfind is mutually exclusive with context_ref and cogmap_id (checked above).
  let scope_ids: Option<Vec<Uuid>> = if params.wayfind {
      Some(readback::wayfind_scope_ids(pool, readback::WayfindScopeQuery {
          principal: profile_id.uuid(),
          lens_id: params.lens_id,
          embedding: params.embedding.as_deref(),
          regions: params.regions.map(|n| n as i32),
      }).await?)
  } else if let Some(map) = params.cogmap_id {
      // … existing single-cogmap arm (cogmap_scope_ids) …
  } else { None };
  ```
  Keep the existing empty-`Some(vec![])`→zero-rows semantics (deny / no visible maps). Reuse the same `scope_ids.as_deref()` into `UnifiedSearchQuery` (line ~393). Note: the wayfind embedding is used both for region selection here *and* the blend inside `unified_search` (params.embedding flows to both) — correct and intended.

  > **Mutual-exclusion shape:** the cleanest form is to count the set scope selectors and `return Err(ApiError::BadRequest("…"))` if `> 1`, then dispatch. Match the existing arm's exact error constructor and pool/principal accessors — read the function first.

- [ ] **Step 5: Run the surface tests — expect PASS.**

  Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test cogmap_wayfind_test`
  Expected: all PASS. (No sqlx cache change — `wayfind_scope_ids` is runtime; `search_select` adds no new macro.)

- [ ] **Step 6: Commit**

  ```bash
  cargo fmt
  git add crates/temper-core/src/types/api.rs crates/temper-services/src/backend/substrate_read.rs crates/temper-api/tests/cogmap_wayfind_test.rs
  git commit -m "Surface B Half 2 Beat B: wayfind/lens/regions on SearchParams + third scope path"
  ```

---

## Task C — CLI `--wayfind` / `--lens` / `--regions` (MCP + client zero-touch)

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:221-252` (clap args), `crates/temper-cli/src/main.rs:461-496` (destructure + forward), `crates/temper-cli/src/actions/search.rs` (`CliSearchArgs` 26-38, `build_search_params` 41-79, mutual-exclusion 45-49), `crates/temper-cli/src/commands/search_cmd.rs:10-42` (re-bundle)

**Interfaces:**
- Produces: `--wayfind` (bool flag), `--lens <ref>` (`Option<String>` → `LensId` via `parse_ref`), `--regions <N>` (`Option<i64>`). Client-side mutual-exclusion: exactly one of `--context` / `--cogmap` / `--wayfind`; `--lens`/`--regions` require `--wayfind` (warn or error if given without it). MCP tool + temper-client need **no change** (they forward `SearchParams` whole — confirmed in the surface map).

- [ ] **Step 1: Write/extend the CLI unit tests** in `crates/temper-cli/src/actions/search.rs` tests (mirror `test_build_search_params_cogmap_uuid` ~176-217):
  ```
  // --wayfind sets wayfind=true; --lens parses to lens_id; --regions sets regions.
  fn test_build_search_params_wayfind()
  // mutual exclusion: --wayfind + --context (or + --cogmap) → Err.
  fn test_wayfind_context_mutually_exclusive()
  ```

- [ ] **Step 2: Run — expect failure** (fields don't exist).
  Run: `cargo nextest run -p temper-cli build_search_params_wayfind`

- [ ] **Step 3: Add clap args** in `cli.rs` `Commands::Search` (after `--cogmap` ~230):
  ```rust
      /// Wayfind: lens-driven region-salience search across your visible maps.
      /// Mutually exclusive with --context / --cogmap.
      #[arg(long)]
      wayfind: bool,
      /// Lens ref overriding wayfind region selection (requires --wayfind).
      #[arg(long)]
      lens: Option<String>,
      /// Top-N regions to scope into for --wayfind (default/ceiling are server-side).
      #[arg(long)]
      regions: Option<i64>,
  ```

- [ ] **Step 4: Thread through** `main.rs` (destructure the three new fields in the `Commands::Search { .. }` arm 461-472, forward into `CliSearchArgs` 482-493 as `wayfind`, `lens: lens.as_deref()`, `regions`), `actions/search.rs` (`CliSearchArgs` fields 26-38; `commands/search_cmd.rs` re-bundle 13-24), and `build_search_params` (41-79): set `wayfind`, parse `--lens` via `temper_workflow::operations::parse_ref(r).map(|id| id.0)` → `lens_id`, set `regions`. Extend the mutual-exclusion guard (45-49) to reject more than one of context/cogmap/wayfind, and reject `--lens`/`--regions` without `--wayfind`.

- [ ] **Step 5: Run CLI tests + crate suite — expect PASS.**
  Run: `cargo nextest run -p temper-cli search`
  Expected: new + existing search tests PASS.

- [ ] **Step 6: Commit**
  ```bash
  cargo fmt
  git add crates/temper-cli
  git commit -m "Surface B Half 2 Beat C: --wayfind/--lens/--regions CLI flags (MCP+client zero-touch)"
  ```

---

## Task D — Calibration note, consolidated review, full verification, PR

- [ ] **Tuning-constant validation (honest).** Confirm the spec-reasoned defaults (α=0.4, β=0.6, default_n=3, max_n=20, thin_threshold=0, recall_floor=off, min-max norm) against any region data reachable locally. There is no rich region corpus on `main` (the L0 kernel is born region-less), so true production-corpus calibration is a **flagged measurement follow-up**, not claimed-done. Record the defaults + the open questions resolved (multi-map = pooled-region grain; lens override = recompute-from-components) in the session note and as a CLAUDE.md-worthy pointer if it tripped anything up. Do **not** invent calibration numbers (`feedback_no_ship_for_now_workarounds`).
- [ ] **Consolidated spec/code review** (deferred per hybrid-execution): 2 parallel reviewers — (1) spec/security against §4/§5/§7/§9 + the gating invariants (every stage gated, member dereference visibility-gated, deny→zero never error); (2) code-quality lens (CQ-*). Verify findings adversarially before applying.
- [ ] **Full verification** (evidence before claims):
  - `cargo make check` (fmt + clippy -D warnings + machete, all-features) — PASS.
  - `cargo make test-artifacts` (the wayfind funnel tier) — PASS.
  - `cargo nextest run -p temper-api --features test-db --test cogmap_wayfind_test --test cogmap_home_test` — PASS.
  - Rebuild + reinstall the bin (`cargo build -p temper-cli --bin temper`; reinstall) before e2e (`project_e2e_stale_temper_bin`).
  - **e2e tier** (`cargo make test-e2e`, and `-embed` if any ingest/embedding fixtures touched) per `feedback_access_semantics_changes_need_e2e_tier` — the deny→zero gating is access-semantics.
- [ ] **Push + open PR** (merge `origin/main` first; regular merge, not squash, per `feedback_prefer_regular_merge_not_squash`). Title: `Surface B Half 2: --wayfind region-salience funnel + cold-start`.
- [ ] **Note the remaining RBAC boundary** on the parent task `019f15b1`: the multi-author *read* clause is now landed (Task A0); only the producer-side **write** predicate for `--cogmap` authorship stays deferred to the cogmap-arc RBAC. Save the session note + mark this sub-task `--stage done`.

---

## Self-Review

**Spec coverage (Half 2, §4/§5/§7/§9):**
- §4 funnel (lens → authz → region-salience trace → top-N → members → blend) → Task A `wayfind_scope_ids` + Task B third path feeding `p_scope_ids`. ✅
- §4.1 `region_score = α·salience_norm + β·query_centroid_cosine`, weighted (not lexicographic), SQL-resident constants, in-pool min-max norm, recompute-under-override-lens → Task A `k`/`bounds`/`scored`/`ranked` CTEs. ✅
- §4.2 sparsity defense + recall-floor knob (wired, default off) → Task A `top_regions` UNION + the §9 regression test. ✅
- §5 cold-start: region-less/thin maps bypass to direct homed scope, `--regions` widens silently, never errors → Task A `thin_maps`/`direct_ids` UNION + Task B cold-start test. ✅
- §7 gating at every stage (map admission `cogmap_visible_maps`; folded excluded; member dereference `resources_visible_to`; blend re-gates) → Task A. ✅
- §8 tuning constants single-home + open questions resolved (pooled-region grain; recompute-from-components) → Task A `k` CTE + Task D note. ✅
- §9 ACs: scopes into top-N + sparse-beats-large regression (Task A test 2), cold-start whole-map never errors (Task A test 3 + Task B test 2), all scope SQL gated + member dereference gated (Task A tests 4/5 + Task B test 3), green under test-artifacts (Task A tier). ✅
- §6 surface shape: one verb, three scope paths, MCP/API zero-touch beyond wire fields → Task B/C. ✅

**Type/seam consistency:** `SearchParams.{wayfind,lens_id,regions}` (B) → `WayfindScopeQuery` (A) → SQL `wayfind_scope_ids(p_lens, p_emb, p_regions_n)` (A) → returned ids → `UnifiedSearchQuery.scope_ids` → `p_scope_ids` (Half 1, reused). `--lens` ref→`LensId` mirrors `--cogmap`→`CogmapId`. No new index, no `unified_search` change. ✅

**Multi-author read RBAC (§3 boundary):** fixed in Task A0 as an additive, membership-flat cogmap clause on `resources_visible_to` (resource-grain mirror of the existing team-owned-context clause); Half 1's documenting test flips to assert inclusion; the L0 public-kernel-telos visibility gap closes as a correct side effect. Only the producer **write** predicate stays deferred. ✅

**Honesty / no-fabrication:** corpus calibration is explicitly a flagged follow-up (no invented numbers); recall-floor is wired-but-off (YAGNI per spec); α/β stay SQL-resident with a named event-sourced/lens-row exit rather than a premature mutable table. ✅

**Placeholder scan:** the SQL is given in full (not "paste from elsewhere"); the one transcription dependency is verifying `profile_effective_teams`/`resources_visible_to` signatures (flagged in Task A note). No TODO/TBD left as behavior. ✅
