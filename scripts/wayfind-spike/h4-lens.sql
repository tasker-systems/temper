-- H4 — which lens rows exist, so the probe holds the SAME lens the caller gets. Read-only.
SELECT id::text, name, home_anchor_table, home_anchor_id::text,
       s_telos, s_ref, s_central
FROM kb_cogmap_lenses
ORDER BY home_anchor_table NULLS FIRST, name;
