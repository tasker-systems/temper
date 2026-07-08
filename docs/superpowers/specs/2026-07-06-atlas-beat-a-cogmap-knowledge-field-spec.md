# Atlas Beat A — Cogmap Panorama as Knowledge Field (spec)

**Status:** implementation spec, ready for plan. First beat of the Atlas reshape.
**North star:** `docs/superpowers/specs/2026-07-06-atlas-reshape-projection-class-north-star.md` (vault research `019f39ca`).
**Subsumes:** territory-label task `019f38b3` (both problems). **Goal:** `019f28a1`.

---

## 1. Purpose (the act this surface serves)

The cogmap panorama is the **orientation** surface for a knowledge domain: `(cogmap,
perspective, time) → small structured survey`. Its job is to let a viewer (consumer or
builder) grasp *the shape of what this cogmap knows* at a glance — which regions are
salient, how they cluster, where to go next — while preserving attention (the survey
stays small). Every visual decision below is derived from that attention-contract, not
from taste.

Today this surface fails the contract two ways (the territory-label task): regions have
no derived labels at all (raw `reg.label`, usually NULL → `Region · N`), and where
labels exist they truncate to indistinct 6-char uppercase stubs. Beat A replaces the
flat circle-pack with a **salience field** + **gated legible labels** + **hover
metadata**, and fixes the missing derive.

## 2. The visual model (decided against the `/dev/atlas` harness)

- **Glowing discrete bodies, not a continuous field.** Regions stay discrete,
  clickable circles (clear targets + a11y), but salience is drawn as a *field-effect*:
  brighter fill + stronger glow for salient regions, so magnitude reads at a glance.
  (Continuous blur-field and contour-heatmap were considered and **rejected** — too far
  from the neighborhood force-graph language and heavier to render.)
- **Force-separated layout** (deterministic), adopting the Tier-2 neighborhood visual
  language for cross-surface consistency and to create whitespace for below-circle
  labels. Replaces the tight `packTerritories` enclosure for region rendering.
- **Size encodes salience** (bigger = more salient) — required by the orientation
  contract (magnitude visible).
- **Glow/opacity encode salience**, eased with exponent **1.4** so the most salient
  regions hold near the ceiling while the `member_count = 1` tail recedes into the
  ground. Peak intensity stays moderate (legibility over drama).
- **Labels are GATED** to the top-K salient regions (K = 10 default), mixed-case,
  rendered *beneath* the circle, word-wrapped to ≤ 2 lines. Label-all is an anti-goal
  (it destroys the "small survey" contract). Every region — labeled or not — carries its
  full title in a `<title>` for hover/focus reveal.
- **Hover reveals metadata**: resources (`member_count`) · salience · coherence
  (`content_cohesion`). A small region hover card, modeled on `NodeHoverCard`.
- **Accessibility**: an equivalent **list of region links + metadata** is available as a
  non-spatial fallback; all region marks are keyboard-focusable and enterable.

## 3. Backend changes

### 3.1 Migration — extend `graph_cogmap_territories` (new file, DROP + CREATE)

Two changes to the cogmap-scoped territory read:

1. **Derived label (Problem 2).** Port B1's pattern from `graph_region_territories`
   (migration `20260706120300`): `COALESCE(reg.label, top-visible-member title)` via a
   `LEFT JOIN LATERAL` that respects `resources_visible_to(p_profile)` — a private
   member's title is **never** surfaced as a label.
2. **Coherence column.** Add `content_cohesion` to the `RETURNS TABLE`.

Adding an OUT column changes the function's return type, so `CREATE OR REPLACE` is
illegal — this needs **`DROP FUNCTION` + `CREATE FUNCTION` in one new migration file**
(cf. `[[reference_extend_shipped_sql_fn_needs_new_drop_create_migration]]`). This is
**skew-safe** (cf. `[[feedback_drop_function_non_additive_breaks_deploy_skew]]`): the
DROP+CREATE is atomic within the migration, and the only caller (`cogmap_panorama`)
`SELECT`s columns *by name*, so pre-deploy code selecting the old 5 columns keeps working
against the new 6-column function. No data change; additive-on-`main` invariant holds.

```sql
-- New migration: 2026070613xxxx_cogmap_territory_derived_label_and_coherence.sql
DROP FUNCTION IF EXISTS graph_cogmap_territories(uuid, uuid, uuid);
CREATE FUNCTION graph_cogmap_territories(p_profile uuid, p_cogmap uuid, p_lens uuid)
RETURNS TABLE(region_id uuid, cogmap_id uuid, label text,
              member_count int, salience double precision, coherence double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id,
           COALESCE(reg.label, rep.title) AS label,
           reg.member_count, reg.salience, reg.content_cohesion
    FROM kb_cogmap_regions reg
    LEFT JOIN LATERAL (
        SELECT r.title
        FROM kb_cogmap_region_members m
        JOIN resources_visible_to(p_profile) v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ORDER BY m.affinity DESC NULLS LAST
        LIMIT 1
    ) rep ON true
    WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, p_cogmap);
$$;
```

> Parity note: `graph_region_territories` already derives the label but does **not**
> return coherence. It is used by the *team* panorama, whose regions leave in Beat C, so
> Beat A does **not** touch it. If a coherence read is wanted at team/region-slice grain
> later, extend it then.

### 3.2 Service — `cogmap_panorama` (`graph_service.rs`)

The read is runtime `sqlx::query_as::<_, (…tuple…)>` (not the compile-time macro), so no
`.sqlx` cache regeneration. Add the sixth tuple column and populate the new field:

```rust
let territories = sqlx::query_as::<_, (Uuid, Uuid, Option<String>, i32, f64, Option<f64>)>(
    "SELECT region_id, cogmap_id, label, member_count, salience, coherence \
         FROM graph_cogmap_territories($1, $2, $3)",
)
// … .map(|(region_id, cogmap_id, label, member_count, salience, coherence)| Territory {
//     id: region_id, kind: Region, label, member_count,
//     salience: Some(salience), coherence, anchor_id: cogmap_id })
```

### 3.3 Type — `Territory` (`temper-core/src/types/graph_territory.rs`)

Add `pub coherence: Option<f64>` (doc: "mean member-to-centroid cosine; region cohesion,
`content_cohesion`. Sizes nothing — surfaced in the hover card."). Regenerate TS types
(`cargo make generate-ts-types`). `territory_overview` (team panorama) sets
`coherence: None` for its regions/contexts (Beat A does not read it there).

## 4. Frontend changes (`packages/temper-ui`)

> `TierPanorama` is shared by the team and cogmap panoramas. Beat A's rendering changes
> apply to **both**; that is intentional (a rendering improvement everywhere). Beat C
> separately changes what the *team* panorama is fed (contexts, not regions). The two
> compose; Beat A does not need to branch on scope.

1. **`lib/graph/atlas/layout/forceTerritories.ts`** (new) — deterministic force layout:
   deterministic ring init, fixed tick count, no `Math.random` (mirrors
   `forceNeighborhood`). Radius from salience (regions) / member_count (contexts) on a
   sqrt scale with legible variance; `forceCollide` padded to reserve a below-circle
   label band + separation; weak center/x/y containment so it stays in the box. Pure,
   unit-tested for stable positions. `TierPanorama` uses it in place of
   `packTerritories` for territory positions. (`packTerritories` stays for now; remove
   only when no caller remains.)
2. **`lib/graph/atlas/labels.ts`** — add `wrapLabel(text, cap, maxLines=2)` (greedy
   word-wrap, final line ellipsis-truncated). Unit-tested.
3. **`marks/TerritoryCircle.svelte`** — new `intensity` (0..1) and `showLabel` props.
   Salience field-effect: `fill-opacity = 0.05 + intensity·0.30`, `stroke-opacity =
   0.25 + intensity·0.50`, `filter: drop-shadow(0 0 (1 + intensity·11)px tint)`. Label
   **beneath** the circle, mixed-case (drop `uppercase` + `letter-spacing`), `wrapLabel`
   to ≤ 2 lines, drawn only when `showLabel`. `<title>` full label always. Ghost
   territories stay faint (unchanged semantics).
4. **`TierPanorama.svelte`** — gate labels to the top-K (K = 10) regions by salience;
   `intensityOf(salience) = (salience / maxSalience) ^ 1.4`; pass `intensity` +
   `showLabel` per territory. Contexts get a fixed mid-high intensity.
5. **Sparse cogmap-territory idiom** — the region-less-cogmap blob (the
   `TEMPER — SELF-COGNITION · N FACETS` path in `TierPanorama`) still renders uppercase +
   tracking. Align it to the mixed-case treatment for coherence. (Facet dots unchanged.)
6. **`marks/RegionHoverCard.svelte`** (new) — on hover/focus of a region, a small card:
   region label · `member_count` resources · salience · coherence, with an "enter"
   affordance. Modeled on `NodeHoverCard` (viewport-flip + keyboard from Beat-2b minors).
7. **Accessibility fallback** — a list of region links (label + resources/salience/
   coherence) as the non-spatial equivalent of the field; ensure every region mark is
   keyboard-focusable and enterable (the `atlas-focusable` role/tabindex already exists
   on drillable territories — verify small/low-salience regions are not excluded).
8. **Regenerated TS `Territory`** rides along (coherence).

## 5. Testing

- **`forceTerritories`** — determinism unit test (same input → same positions; no
  `Math.random`), and a no-overlap/containment assertion.
- **`wrapLabel`** — unit tests (≤ cap one line; long → 2 lines; single over-long word;
  ellipsis on final line).
- **`territory.test.ts` / palette** — update for intensity + gated labels.
- **Backend e2e** (`test-db`) — the cogmap panorama read returns derived labels +
  coherence; and a **deny-direction** test: a region whose only member is not visible to
  the caller surfaces **no** member title as its label (cf.
  `[[feedback_read_gate_must_match_full_canonical_visibility]]`). Run e2e (`test-e2e`);
  the derive touches visibility, so run the access-sensitive tier
  (`[[feedback_access_semantics_changes_need_e2e_tier]]`).
- **Harness fixtures** — the committed synthetic bundle (`atlas-fixtures.json`) predates
  B1 and carries NULL labels + no coherence. Regenerate/enrich it (and the sanitize
  script) so the harness and any fixture-driven tests exercise real labels + coherence.
  (The local capture is a personal gitignored file; do not commit it.)

## 6. Open decisions to confirm in review

1. **Orphan facets in the cogmap field.** Region-less cogmap resources currently render
   as a labeled blob with facet dots. Proposed: mixed-case label only (item 4.5); keep
   facet dots as-is. Alternative (deferred): render them as individual faint nodes in
   the field. **Confirm: mixed-case-only for Beat A.**
2. **Telos-charter anchor.** The north star says the cogmap panorama is "anchored by its
   telos-charter." Proposed for Beat A: **minimal** — the cogmap name in the crumb is
   enough; deep charter surfacing (a header/aside showing the charter statement) is
   **deferred** to a later beat. **Confirm defer.**
3. **Gate-K = 10** and **hover fields = resources · salience · coherence** exactly (the
   region row also carries `telos_alignment`, `reference_standing`, `centrality`,
   `internal_tension` — richer signals **deferred**). **Confirm.**

## 7. Out of scope / deferred / rejected

- **Rejected:** continuous blur-field and contour/heatmap rendering (§2).
- **Deferred to later beats:** team panorama = contexts (Beat C), home reframe (Beat B),
  region→resources drill enrichment (Beat D), context view re-imagining / Chunk D
  (Beat E), team-grain aggregate knowledge view, richer region signals in the hover,
  deep telos-charter integration.

## 8. Connections

- North star `019f39ca`; goal `019f28a1`; subsumed task `019f38b3`.
- Framework: projection-class docs `019e54b9` / `019e54bb` / `019e5530` / `019e552c`
  (orientation attention-contract is the source of the gating + size + field decisions).
- `[[feedback_local_proddata_render_harness_for_ui]]`, `[[feedback_local_test_e2e_green_false_signal_for_embed]]`,
  `[[feedback_read_gate_must_match_full_canonical_visibility]]`,
  `[[reference_extend_shipped_sql_fn_needs_new_drop_create_migration]]`,
  `[[feedback_drop_function_non_additive_breaks_deploy_skew]]`,
  `[[feedback_nextest_does_not_rebuild_spawned_temper_bin]]` (if e2e spawns the CLI).
