-- L0 — the kernel "what is temper" cognitive map (cognitive-map agent-invocation architecture,
-- 2026-06-25 spec). The public, root-team-joined system-default cogmap, born deterministically
-- here via the SAME genesis SQL functions every map uses. Born content-light: a minimal empty-charter
-- telos (blocks=[] → _project_blocks no-ops → no embeddings needed in a migration). The rich charter
-- content (prose, facets, edges) is a separate deferred authoring task via the real ingest path.
-- Idempotent: guarded by natural key so a logical re-run is a no-op. Additive (data only).

-- 1. The root team `temper-system`. Canonical triggers/functions REFERENCE
--    `kb_teams WHERE slug = 'temper-system'` (e.g. 20260624000002_canonical_functions.sql:63,102)
--    but production migrations never created it — a latent gap L0 closes. id is defaulted
--    (uuid_generate_v7()). Idempotent on the UNIQUE slug.
INSERT INTO kb_teams (slug, name)
VALUES ('temper-system', 'Temper System')
ON CONFLICT (slug) DO NOTHING;

-- 2. L0 itself, via cogmap_genesis under the system actor. Reserved fixed ids so future migrations
--    and code reference L0 deterministically. Empty telos.blocks (content-light birth). p_content is
--    '{}' (no chunks ⇒ no sidecar needed). Guarded by name so the genesis runs exactly once.
DO $l0$
DECLARE
    v_emitter uuid := (SELECT e.id FROM kb_entities e
                         JOIN kb_profiles p ON p.id = e.profile_id
                        WHERE p.handle = 'system' AND e.name = 'system');
    v_owner   uuid := (SELECT id FROM kb_profiles WHERE handle = 'system');
BEGIN
    IF NOT EXISTS (SELECT 1 FROM kb_cogmaps WHERE id = '00000000-0000-0000-0005-000000000001') THEN
        PERFORM cogmap_genesis(
            jsonb_build_object(
                'cogmap_id',        '00000000-0000-0000-0005-000000000001',
                'name',             'system-default',
                'owner_profile_id', v_owner,
                'telos', jsonb_build_object(
                    'resource_id', '00000000-0000-0000-0005-000000000002',
                    'title',       'What Temper Is',
                    'origin_uri',  'temper://system/what-is-temper',
                    'blocks',      '[]'::jsonb
                )
            ),
            '{}'::jsonb,   -- content sidecar: empty (no chunks)
            v_emitter
        );
    END IF;
END
$l0$;

-- 3. Join L0 to the root team (public cognitive-map home, spec §8). Idempotent on the join's PK.
INSERT INTO kb_team_cogmaps (cogmap_id, team_id)
SELECT '00000000-0000-0000-0005-000000000001', t.id
  FROM kb_teams t
 WHERE t.slug = 'temper-system'
   AND NOT EXISTS (
       SELECT 1 FROM kb_team_cogmaps tc
        WHERE tc.cogmap_id = '00000000-0000-0000-0005-000000000001' AND tc.team_id = t.id
   );
