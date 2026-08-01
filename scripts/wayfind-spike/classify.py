#!/usr/bin/env python3
"""Emit the SQL that classifies returned resources by home anchor, arm, and chunk count.

    python3 classify.py w3_ids.txt > classify.sql
    ./prod-readonly.sh classify.sql > classify_out.txt

Home is a strict partition on this corpus (every resource has exactly one row in
kb_resource_homes), which is what makes arm attribution exact rather than approximate:
cold-start fires ONLY for region-less anchors, so a resource's arm follows from its home
anchor's live-region count.

Pass the UNION of every id set you intend to analyze — rows from an unclassified width come
back 'unknown' in analyze.py.
"""
import sys

PRINCIPAL = '019d4add-f49d-7c43-a87d-dda470e5dd9c'

ids = sorted({l.strip() for l in open(sys.argv[1]) if l.strip()})
if not ids:
    sys.exit("no ids on stdin file")
values = ",".join(f"('{i}'::uuid)" for i in ids)

print(f"""\\set prin '{PRINCIPAL}'
WITH r(id) AS (VALUES {values})
SELECT r.id,
       h.anchor_table,
       COALESCE(cm.name, cx.name) AS anchor,
       (SELECT count(*) FROM kb_cogmap_regions g
         WHERE g.home_anchor_table = h.anchor_table AND g.home_anchor_id = h.anchor_id
           AND NOT g.is_folded) AS anchor_live_regions,
       (SELECT count(*) FROM kb_chunks c
         WHERE c.resource_id = r.id AND c.is_current) AS n_chunks
FROM r
LEFT JOIN kb_resource_homes h ON h.resource_id = r.id
LEFT JOIN kb_cogmaps  cm ON h.anchor_table = 'kb_cogmaps'  AND cm.id = h.anchor_id
LEFT JOIN kb_contexts cx ON h.anchor_table = 'kb_contexts' AND cx.id = h.anchor_id;
""")
