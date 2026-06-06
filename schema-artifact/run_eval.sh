#!/usr/bin/env bash
# ============================================================================
# Temper — emergent-region projection: end-to-end falsification runner.
# ----------------------------------------------------------------------------
# Loads the artifact, materializes the onboarding-cogmap regions with the
# temper-next binary, and runs the S6a–h verdicts. Each verdict prints PASS/FAIL
# and the script exits non-zero on any failure — "regions are computable from
# the declared graph (not cosine)" is asserted, not asserted-by-narration.
#
# Requires: the ONNX runtime for temper-ingest's `embed` feature (bge-768) —
# run on a box that has it (matches the Embed CI job).
#
# Usage:  schema-artifact/run_eval.sh
# ============================================================================
set -euo pipefail
DB="${DATABASE_URL:-postgresql://temper:temper@localhost:5437/temper_development}"
cd "$(dirname "$0")/.."

# search_path on the connection so queries need no inline SET (which would print "SET" into captures).
export PGOPTIONS="-c search_path=temper_next,public"
q() { psql "$DB" -tAX -c "$1"; }   # terse, unaligned, no psqlrc
fail=0
check() {  # check "<label>" "<actual>" "<expected>"
  if [ "$2" = "$3" ]; then echo "  PASS  $1"; else echo "  FAIL  $1 (got '$2', want '$3')"; fail=1; fi
}

echo "== load artifact (01 -> 02 -> 03) =="
for f in 01_schema 02_functions 03_seed; do
  psql "$DB" -q -v ON_ERROR_STOP=1 -f "schema-artifact/$f.sql" >/dev/null
done

echo "== materialize telos-default (embed + cluster) =="
DATABASE_URL="$DB" cargo run -q -p temper-next -- onboarding-cogmap >/dev/null

echo "== S6a/S6c/S6d/S6e/S6g (04b suite) =="
psql "$DB" -q -f schema-artifact/04b_region_suite.sql >/dev/null   # printed verbatim below for humans
all_pass=$(q "
  WITH td AS (
    SELECT res.origin_uri, m.region_id
    FROM kb_cogmap_region_members m
    JOIN kb_cogmap_regions r ON r.id=m.region_id AND NOT r.is_folded
    JOIN kb_cogmap_lenses  l ON l.id=r.lens_id AND l.name='telos-default'
    JOIN kb_resources    res ON res.id=m.member_id)
  SELECT ((SELECT count(*) FROM kb_cogmap_regions r JOIN kb_cogmap_lenses l ON l.id=r.lens_id
             WHERE l.name='telos-default' AND NOT r.is_folded) >= 2)
     AND (SELECT a.region_id=b.region_id FROM td a, td b
            WHERE a.origin_uri='temper://c/pair' AND b.origin_uri='temper://c/smallest')
     AND (SELECT ca.content_cohesion > cb.content_cohesion FROM kb_cogmap_regions ca, kb_cogmap_regions cb
            WHERE ca.id=(SELECT region_id FROM td WHERE origin_uri='temper://c/pair')
              AND cb.id=(SELECT region_id FROM td WHERE origin_uri='temper://c/staging'))
     AND (SELECT count(*)=1 FROM td WHERE region_id=(SELECT region_id FROM td WHERE origin_uri='temper://c/solo'))
     AND (SELECT (SELECT region_id FROM td WHERE origin_uri='temper://c/checklist')
                =(SELECT region_id FROM td WHERE origin_uri='temper://c/staging'))
     AND (SELECT (SELECT region_id FROM td WHERE origin_uri='temper://c/bluegreen')
                =(SELECT region_id FROM td WHERE origin_uri='temper://c/bigbang'))
     AND (SELECT (SELECT internal_tension FROM kb_cogmap_regions
                    WHERE id=(SELECT region_id FROM td WHERE origin_uri='temper://c/bluegreen')) > 0);")
check "S6a/c/d/e/g (04b suite all_pass)" "$all_pass" "t"

# membership fingerprint for one lens (md5 over region->member, order-stable)
fp() { q "
  SELECT md5(string_agg(res.origin_uri, ',' ORDER BY r.id, res.origin_uri))
  FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id=m.region_id AND NOT r.is_folded
  JOIN kb_cogmap_lenses  l ON l.id=r.lens_id AND l.name='$1'
  JOIN kb_resources    res ON res.id=m.member_id;"; }

echo "== S6b: reproducible (re-run -> byte-identical membership) =="
A=$(fp telos-default)
DATABASE_URL="$DB" cargo run -q -p temper-next -- onboarding-cogmap >/dev/null
B=$(fp telos-default)
check "S6b membership reproducible" "$([ "$A" = "$B" ] && echo same || echo differ)" "same"

echo "== S6f: plurality (prop-heavy lens -> different region set) =="
DATABASE_URL="$DB" cargo run -q -p temper-next -- onboarding-cogmap telos-default-propheavy >/dev/null
TD=$(fp telos-default)
PH=$(fp telos-default-propheavy)
check "S6f propheavy differs from telos-default" "$([ "$TD" != "$PH" ] && echo differ || echo same)" "differ"
# the concrete delta: setup/first-build are co-region under telos-default, split under prop-heavy
delta=$(q "
  WITH m AS (
    SELECT l.name AS lens, res.origin_uri, mem.region_id
    FROM kb_cogmap_region_members mem
    JOIN kb_cogmap_regions r ON r.id=mem.region_id AND NOT r.is_folded
    JOIN kb_cogmap_lenses  l ON l.id=r.lens_id
    JOIN kb_resources    res ON res.id=mem.member_id)
  SELECT (SELECT region_id FROM m WHERE lens='telos-default' AND origin_uri='temper://c/setup')
        =(SELECT region_id FROM m WHERE lens='telos-default' AND origin_uri='temper://c/firstbuild')
     AND (SELECT region_id FROM m WHERE lens='telos-default-propheavy' AND origin_uri='temper://c/setup')
        <>(SELECT region_id FROM m WHERE lens='telos-default-propheavy' AND origin_uri='temper://c/firstbuild');")
check "S6f setup~build merge(td) yet split(propheavy)" "$delta" "t"

echo "== S6h: functorial update + staleness =="
# solo is a singleton pre-update
solo_pre=$(q "
  SELECT count(*) FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id=m.region_id AND NOT r.is_folded
  JOIN kb_cogmap_lenses  l ON l.id=r.lens_id AND l.name='telos-default'
  WHERE m.region_id=(SELECT m2.region_id FROM kb_cogmap_region_members m2
    JOIN kb_cogmap_regions r2 ON r2.id=m2.region_id AND NOT r2.is_folded
    JOIN kb_cogmap_lenses l2 ON l2.id=r2.lens_id AND l2.name='telos-default'
    WHERE m2.member_id=(SELECT id FROM kb_resources WHERE origin_uri='temper://c/solo'));")
check "S6h solo singleton pre-update" "$solo_pre" "1"

# emit ONE relationship_asserted event with express edges solo -> {pair,smallest,confidence}.
# (average-link dilutes a single edge to a 3-member cluster below resolution, so solo is linked to
#  all three α members under one assertion — a coherent declared change, not a hack.)
psql "$DB" -q -v ON_ERROR_STOP=1 -c "
DO \$h\$
DECLARE v_et uuid; v_ev uuid; v_cog uuid; v_emit uuid; v_solo uuid; m text;
BEGIN
  SELECT id INTO v_et   FROM kb_event_types WHERE name='relationship_asserted';
  SELECT id INTO v_cog  FROM kb_cogmaps     WHERE name='onboarding-cogmap';
  SELECT id INTO v_solo FROM kb_resources   WHERE origin_uri='temper://c/solo';
  SELECT emitter_entity_id INTO v_emit FROM kb_events ORDER BY occurred_at DESC LIMIT 1;
  INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, occurred_at)
    VALUES (v_et, v_emit, 'kb_cogmaps', v_cog, now()) RETURNING id INTO v_ev;
  FOREACH m IN ARRAY ARRAY['temper://c/pair','temper://c/smallest','temper://c/confidence'] LOOP
    INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label,
                          home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
    VALUES ('kb_resources', v_solo, 'kb_resources', (SELECT id FROM kb_resources WHERE origin_uri=m),
            'express', 'related', 'kb_cogmaps', v_cog, v_ev, v_ev);
  END LOOP;
END \$h\$;"

# the watermark is now behind the new declared touch
stale=$(q "SELECT is_stale FROM cogmap_staleness((SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'));")
check "S6h stale after new edge event" "$stale" "t"

# re-materialize -> the projection updates predictably: solo now co-regions with α
DATABASE_URL="$DB" cargo run -q -p temper-next -- onboarding-cogmap >/dev/null
solo_with_alpha=$(q "
  WITH td AS (
    SELECT res.origin_uri, m.region_id
    FROM kb_cogmap_region_members m
    JOIN kb_cogmap_regions r ON r.id=m.region_id AND NOT r.is_folded
    JOIN kb_cogmap_lenses  l ON l.id=r.lens_id AND l.name='telos-default'
    JOIN kb_resources    res ON res.id=m.member_id)
  SELECT (SELECT region_id FROM td WHERE origin_uri='temper://c/solo')
       = (SELECT region_id FROM td WHERE origin_uri='temper://c/pair');")
check "S6h solo joins α after re-materialize" "$solo_with_alpha" "t"

echo "============================================================"
if [ "$fail" -eq 0 ]; then
  echo "ALL S6 VERDICTS PASS — regions are a pure projection of the declared graph."
else
  echo "SOME S6 VERDICTS FAILED."; exit 1
fi
