\set ON_ERROR_STOP on
\echo '=== are there zero-norm (NaN-producing) chunk embeddings on prod? ==='
SELECT count(*) AS zero_norm_current_chunks
  FROM kb_chunks
 WHERE is_current AND (embedding <=> embedding) <> (embedding <=> embedding);
\echo ''
\echo '=== and zero-norm region centroids (the case wayfind already guards) ==='
SELECT count(*) AS zero_norm_live_centroids
  FROM kb_cogmap_regions
 WHERE NOT is_folded AND centroid IS NOT NULL
   AND (centroid <=> centroid) <> (centroid <=> centroid);
