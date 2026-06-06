-- ============================================================================
-- Temper — Arc-1 emergent-region projection: THE FALSIFICATION SUITE (S6a–h)
-- ----------------------------------------------------------------------------
-- Runs AFTER the temper-next binary materializes the telos-default regions
-- (load order: 01 → 02 → 03 → `temper-next onboarding-cogmap` → 04b).
--
-- The hypothesis: a region is a pure projection of the DECLARED graph (edges +
-- facets) under a lens — cosine never FORMS a region, it only reads one out. The
-- cast (03_seed.sql) is authored so declared-structure and content-structure
-- DISAGREE; these verdicts make the disagreement observable and falsifiable.
--
-- NOTE: both telos-default and telos-default-propheavy may have live regions, so
-- every membership lookup is SCOPED to telos-default via the td_member view.
-- ============================================================================

SET search_path = temper_next, public;
\echo '======== REGION SUITE (telos-default, post-materialize) ========'

-- Concept → region for the telos-default lens only (keyed by origin_uri).
DROP VIEW IF EXISTS td_member;
CREATE TEMP VIEW td_member AS
SELECT res.origin_uri, m.region_id
FROM kb_cogmap_region_members m
JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded
JOIN kb_cogmap_lenses  l ON l.id = r.lens_id AND l.name = 'telos-default'
JOIN kb_resources    res ON res.id = m.member_id;

\echo '== S6a: >=2 regions; alpha co-region =='
SELECT (SELECT count(*) FROM kb_cogmap_regions r
          JOIN kb_cogmap_lenses l ON l.id = r.lens_id
          WHERE l.name = 'telos-default' AND NOT r.is_folded) AS region_count,
       (SELECT a.region_id = b.region_id FROM td_member a, td_member b
          WHERE a.origin_uri = 'temper://c/pair'
            AND b.origin_uri = 'temper://c/smallest') AS alpha_together;
-- EXPECT: region_count >= 2, alpha_together = t

\echo '== S6c (HEADLINE): content_cohesion(alpha) > content_cohesion(beta) =='
SELECT round(ca.content_cohesion::numeric, 4) AS alpha_cohesion,
       round(cb.content_cohesion::numeric, 4) AS beta_cohesion,
       ca.content_cohesion > cb.content_cohesion AS surface_gt_relational
FROM kb_cogmap_regions ca, kb_cogmap_regions cb
WHERE ca.id = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/pair')
  AND cb.id = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/staging');
-- EXPECT: surface_gt_relational = t (beta is declared-coherent yet content-divergent — relational surplus)

\echo '== S6d: solo-retro-note stays its OWN region (cosine did NOT form co-membership) =='
SELECT count(*) AS solo_region_size
FROM td_member
WHERE region_id = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/solo');
-- EXPECT: solo_region_size = 1

\echo '== S6e: bridge joins beta via facet_overlap alone (no edge) =='
SELECT (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/checklist')
     = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/staging') AS bridge_in_beta;
-- EXPECT: bridge_in_beta = t

\echo '== S6g: blue-green & big-bang co-region AND internal_tension > 0 =='
SELECT (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/bluegreen')
     = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/bigbang') AS tension_together,
       (SELECT internal_tension FROM kb_cogmap_regions
          WHERE id = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/bluegreen')) > 0
         AS tension_positive;
-- EXPECT: tension_together = t, tension_positive = t

\echo '== SUITE SUMMARY (single PASS/FAIL line per verdict) =='
WITH v AS (
  SELECT
    (SELECT count(*) FROM kb_cogmap_regions r JOIN kb_cogmap_lenses l ON l.id = r.lens_id
       WHERE l.name = 'telos-default' AND NOT r.is_folded) >= 2 AS s6a_count,
    (SELECT a.region_id = b.region_id FROM td_member a, td_member b
       WHERE a.origin_uri = 'temper://c/pair' AND b.origin_uri = 'temper://c/smallest') AS s6a_alpha,
    (SELECT ca.content_cohesion > cb.content_cohesion
       FROM kb_cogmap_regions ca, kb_cogmap_regions cb
       WHERE ca.id = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/pair')
         AND cb.id = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/staging')) AS s6c,
    (SELECT count(*) FROM td_member
       WHERE region_id = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/solo')) = 1 AS s6d,
    (SELECT (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/checklist')
          = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/staging')) AS s6e,
    (SELECT (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/bluegreen')
          = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/bigbang')) AS s6g_together,
    (SELECT (SELECT internal_tension FROM kb_cogmap_regions
               WHERE id = (SELECT region_id FROM td_member WHERE origin_uri = 'temper://c/bluegreen')) > 0)
         AS s6g_tension
)
SELECT format('S6a %s | S6c %s | S6d %s | S6e %s | S6g %s',
              CASE WHEN s6a_count AND s6a_alpha THEN 'PASS' ELSE 'FAIL' END,
              CASE WHEN s6c THEN 'PASS' ELSE 'FAIL' END,
              CASE WHEN s6d THEN 'PASS' ELSE 'FAIL' END,
              CASE WHEN s6e THEN 'PASS' ELSE 'FAIL' END,
              CASE WHEN s6g_together AND s6g_tension THEN 'PASS' ELSE 'FAIL' END) AS verdicts,
       (s6a_count AND s6a_alpha AND s6c AND s6d AND s6e AND s6g_together AND s6g_tension) AS all_pass
FROM v;
-- EXPECT: all verdicts PASS, all_pass = t
