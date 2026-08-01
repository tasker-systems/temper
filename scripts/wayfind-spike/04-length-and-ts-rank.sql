-- P3 — the Stage-2 length lever, measured on the two things Stage-2 actually reads:
--   vec arm: vec_norm = best-of-N current chunks  -> chunk COUNT matters
--   fts arm: ts_rank(search_vector, q, 32)        -> lexeme COUNT / term frequency matters,
--            and flag 32 is rank/(rank+1) — monotone, so it applies NO length normalization.
\set prin '019d4add-f49d-7c43-a87d-dda470e5dd9c'

\echo '=== C. chunks + FTS lexemes per visible resource, by homing class ==='
WITH vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state = 'complete'),
cls AS (
  SELECT v.resource_id,
         CASE WHEN bool_or(hh.anchor_table = 'kb_cogmaps') THEN 'distilled (cogmap)'
              ELSE 'raw (context)' END AS class
  FROM vis v LEFT JOIN kb_resource_homes hh ON hh.resource_id = v.resource_id
  GROUP BY v.resource_id)
SELECT cls.class,
       count(*) AS resources,
       round(avg(x.nc)::numeric, 2)                          AS mean_chunks,
       percentile_disc(0.5) WITHIN GROUP (ORDER BY x.nc)     AS p50_chunks,
       max(x.nc)                                             AS max_chunks,
       round(avg(x.lex)::numeric, 1)                         AS mean_lexemes,
       percentile_disc(0.5) WITHIN GROUP (ORDER BY x.lex)    AS p50_lexemes,
       percentile_disc(0.9) WITHIN GROUP (ORDER BY x.lex)    AS p90_lexemes,
       max(x.lex)                                            AS max_lexemes
FROM cls
CROSS JOIN LATERAL (
  SELECT (SELECT count(*) FROM kb_chunks c
           WHERE c.resource_id = cls.resource_id AND c.is_current) AS nc,
         COALESCE((SELECT length(si.search_vector) FROM kb_resource_search_index si
                    WHERE si.resource_id = cls.resource_id), 0)    AS lex) x
GROUP BY 1 ORDER BY 2 DESC;

\echo ''
\echo '=== D. does ts_rank track lexeme count? correlation, over resources matching a common term ==='
WITH vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state = 'complete'),
cls AS (
  SELECT v.resource_id,
         CASE WHEN bool_or(hh.anchor_table = 'kb_cogmaps') THEN 'distilled (cogmap)'
              ELSE 'raw (context)' END AS class
  FROM vis v LEFT JOIN kb_resource_homes hh ON hh.resource_id = v.resource_id
  GROUP BY v.resource_id),
m AS (
  SELECT cls.class,
         length(si.search_vector) AS lex,
         ts_rank(si.search_vector, websearch_to_tsquery('english','cognitive map'), 32) AS rank32
  FROM cls JOIN kb_resource_search_index si ON si.resource_id = cls.resource_id
  WHERE si.search_vector @@ websearch_to_tsquery('english','cognitive map'))
SELECT class, count(*) AS matching,
       round(avg(lex)::numeric,1) AS mean_lexemes,
       round(avg(rank32)::numeric,4) AS mean_ts_rank,
       round(max(rank32)::numeric,4) AS max_ts_rank,
       round(corr(lex, rank32)::numeric,3) AS corr_lexemes_vs_rank
FROM m GROUP BY 1 ORDER BY 1;
