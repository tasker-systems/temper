-- Person erasure runs resource erasure over the subject's own contexts.
--
-- Task 01a12603-db75-74e6-a5c4-1dc9ad508202, under goal 01a04aee-81ec-7f42-a8e9-e5a0c1a3b918. Design:
-- temper-artifacts specs/2026-10-10-person-erasure-runs-resource-erasure-design.md (ruled 2026-10-10),
-- building rulings R1 and R5 of reviews/2026-10-10-erasure-inventory.md.
--
-- Until now the person act emptied its estate's prose by hash with its own arms, and left the estate's
-- titles, properties, edge labels and the rest of its surface, in the projections and on the ledger.
-- Its estate was the subject's @me contexts only, so the personal team's contexts were outside it.
--
--   0. A guard: the act's record gains a required key, and the redaction no longer reads an event
--      without it. No principal has been erased on any deployment (Q5), so this raises if a
--      principal_erased event exists rather than let replay of one fail later.
--   1. _erasure_estate_contexts(subject): the ONE definition of the estate, the subject's @me
--      contexts and their personal team's contexts (R1).
--   2. principal_erasure_survey_plan: reads the estate from (1), gives every estate resource a
--      disposition (erase, complete, skip, charter) in resource-id order, narrows redacted_hashes to
--      the charters' text hashes plus the struck blobs' (D5), and names the charters held and the
--      resources skipped.
--   3. principal_erasure_survey: the read-only door adds per-resource counts from each resource's
--      own survey (D6).
--   4. principal_erasure_execute: runs resource_erasure_execute on each `erase` or `complete`
--      resource in resource-id order under the request reference, strikes the estate's blobs,
--      appends principal_erased with estate_contexts, resource_erasures and charters_held, then
--      runs the redaction (2c, 2d).
--   5. _erasure_apply_redaction: reads the estate from its event's payload instead of re-deriving
--      the @me predicate in each arm; arm 12 marks every anchor whose live regions hold an estate
--      member, and arms 12 and 13 reach every estate context.
--   6. The principal_erased payload_schema gains the three optional keys.
--
-- Bodies (2) and (5) are their latest definitions (20261020100000) except for the lines named above
-- and their comments.
--
-- Additive: one new function and CREATE OR REPLACE with every signature unchanged. A binary without
-- this migration calls the same functions and reads the same keys it read before; the new keys in
-- their results are extra.

-- Section 0. The guard (Q5).
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id
                WHERE et.name = 'principal_erased') THEN
        RAISE EXCEPTION '20261021100000: a principal_erased event exists; its payload has no estate_contexts and the redaction now requires one. Ruling Q5 (2026-10-10) assumed none exists: stop and re-rule before deploying.';
    END IF;
END;
$$;

-- Section 1. The estate.
CREATE OR REPLACE FUNCTION _erasure_estate_contexts(p_subject uuid)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
    -- The subject's own (@me) contexts, and their personal team's contexts. The personal team is
    -- found by kb_teams.personal_of (unique: kb_teams_personal_of_key), never by slug.
    SELECT c.id FROM kb_contexts c
     WHERE (c.owner_table = 'kb_profiles' AND c.owner_id = p_subject)
        OR (c.owner_table = 'kb_teams'
            AND c.owner_id = (SELECT t.id FROM kb_teams t WHERE t.personal_of = p_subject))
     ORDER BY c.id;
$$;

COMMENT ON FUNCTION _erasure_estate_contexts(uuid) IS
    'The person-erasure estate: the subject''s @me contexts and their personal team''s contexts (R1, 2026-10-10). The one definition; the act records its result in principal_erased.estate_contexts.';

-- Section 2. The plan.
CREATE OR REPLACE FUNCTION principal_erasure_survey_plan(p_subject uuid)
RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_exists     uuid;
    v_governed   uuid[] := '{}';
    v_resources  uuid[] := '{}';
    v_hashes     text[] := '{}';
    v_blob_hash  text[] := '{}';
    v_targets    jsonb  := '[]'::jsonb;
    v_strikes    jsonb  := '[]'::jsonb;
    v_row        record;
    v_rel        boolean;
    v_struck     text[] := '{}';
    v_team  record;
    v_n     integer;
    v_text_rem text;
    v_prof  kb_profiles%ROWTYPE;
    v_charters   uuid[] := '{}';
    v_resrows    jsonb  := '[]'::jsonb;
    v_disp       text;
    v_counts     jsonb  := jsonb_build_object('erase', 0, 'complete', 0, 'skip', 0, 'charter', 0);
BEGIN
    SELECT id INTO v_exists FROM kb_profiles WHERE id = p_subject;
    IF v_exists IS NULL THEN
        RAISE EXCEPTION 'principal_erasure_execute: subject % not found', p_subject;
    END IF;

    -- The estate contexts (R1, ruled 2026-10-10): the subject's @me contexts and their personal
    -- team's contexts, from _erasure_estate_contexts, the ONE definition. Other team contexts and
    -- map homes are NOT the estate (disposition iii). The act records this set in its event and
    -- the redaction reads it from there, so the arms no longer re-derive it.
    v_governed := ARRAY(SELECT _erasure_estate_contexts(p_subject));

    -- ── Scope: the estate, HOME-PURE (ruled 2026-09-11 with Pete — the "scope of
    -- engagement" ruling) ─────────────────────────────────────────────────────────────
    -- Everything homed in a governed context wipes with the estate, whoever authored,
    -- owns or emitted it: writing into someone's PRIVATE context under a grant declares
    -- the content's scope of engagement — it lives and dies with that estate (the
    -- terms-of-use documentation states this). The 20260909000025 actor halves are
    -- dissolved: the owner/originator OR kept reassigned-in-place authorship in reach
    -- (20260703140000 moves owner_profile_id in place), and the emitted half was already
    -- subsumed by the governed-home EXISTS it carried. Erasure keeps the resource rows
    -- live (D3), so a grantee whose prose dies with the estate holds their remaining
    -- access only against a retired context (arm 13), and their attribution dies by the
    -- pseudonym break, never by edits.
    SELECT coalesce(array_agg(DISTINCT h.resource_id), '{}') INTO v_resources
      FROM kb_resource_homes h
     WHERE h.anchor_table = 'kb_contexts'
       AND h.anchor_id = ANY(v_governed);

    -- ── Each estate resource's disposition, in resource-id order: the order the act erases them
    --    in (2c), which is also the order every person act takes their locks in.
    --      charter  — a cogmap's telos. Resource erasure refuses it; the act empties its text
    --                 by hash below and holds it until map-grain erasure (2a).
    --      erase    — live or soft-deleted: resource erasure runs on it (2b).
    --      complete — erased, and the ledger still holds paths to rewrite (an erasure from
    --                 before cut 2): resource erasure's completion pass runs (2f).
    --      skip     — erased and complete (2f). ──────────────────────────────────────────────
    FOR v_row IN
        SELECT r.id,
               r.erased_at IS NOT NULL AS erased,
               EXISTS (SELECT 1 FROM kb_cogmaps m WHERE m.telos_resource_id = r.id) AS charter
          FROM kb_resources r
         WHERE r.id = ANY(v_resources)
         ORDER BY r.id
    LOOP
        v_disp := CASE
            WHEN v_row.charter THEN 'charter'
            WHEN NOT v_row.erased THEN 'erase'
            WHEN resource_erasure_completion_fields(v_row.id) <> '[]'::jsonb THEN 'complete'
            ELSE 'skip'
        END;
        IF v_disp = 'charter' THEN
            v_charters := v_charters || v_row.id;
        END IF;
        v_resrows := v_resrows || jsonb_build_array(jsonb_build_object(
            'resource_id', v_row.id, 'disposition', v_disp));
        v_counts := jsonb_set(v_counts, ARRAY[v_disp], to_jsonb((v_counts->>v_disp)::integer + 1));
    END LOOP;

    -- The text hashes the act empties BY HASH (D5, ruled 2026-10-10): the charters' chunk and
    -- verbatim block content hashes. Every other estate resource is emptied row by row by
    -- resource erasure, which admits no hash to kb_erased_content.
    SELECT coalesce(array_agg(DISTINCT h), '{}') INTO v_hashes FROM (
        SELECT c.content_hash AS h FROM kb_chunks c
         WHERE c.resource_id = ANY(v_charters)
        UNION
        SELECT bc.content_hash AS h
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
         WHERE b.resource_id = ANY(v_charters)
    ) s;

    -- ── The blob pass: HOME-PURE (the 2026-09-12 ruling), WOULD-STRIKE structured ───────
    -- Scope = every blob row homed in a governed context, WHOEVER committed it, plus the
    -- subject's own rows (owner/originator/emitted) wherever they home — the subject's
    -- team- and map-homed rows stay the named remainder (disposition iii). Governed-home
    -- LIVE rows report would-strike entries; governed-home struck rows report
    -- already-erased; EVERY other home is the named remainder. released_would_be SIMULATES
    -- the act's own sequential refcount: the ONE blob_delete live-row predicate (live rows
    -- with the hash, this one included, <= 1) minus the same-hash rows EARLIER IN THE
    -- PLAN'S OWN STRIKE SET — execute consumes the entries in order, so those siblings are
    -- already emptied when the act reaches this row. Run at survey time WITHOUT the hash's
    -- advisory lock: a PREDICTION, honest about the moment it ran; the wrapper's
    -- strike-time verdict is authoritative in the act. The enumeration is ORDERED BY id —
    -- the act strikes in plan order and the survey must walk the SAME sequence when it
    -- simulates it, so the order cannot be left to the scan.
    FOR v_row IN
        SELECT b.id, b.content_hash, b.blob_pathname, b.content_type,
               (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed)) AS governed_home
          FROM kb_blobs b
         WHERE (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed))
            OR b.owner_profile_id = p_subject
            OR b.originator_profile_id = p_subject
            OR b.asserted_by_event_id IN (
               SELECT e.id FROM kb_events e
                 JOIN kb_entities en ON en.id = e.emitter_entity_id
                WHERE en.profile_id = p_subject)
          ORDER BY b.id
    LOOP
        IF v_row.governed_home AND v_row.content_type IS NOT NULL THEN
            -- The act's own SEQUENTIAL refcount, simulated: the strike loop consumes the
            -- plan's rows by id IN ORDER and each blob_delete counts live rows at ITS
            -- moment, so every same-hash row already in this plan's strike set is emptied
            -- when the act reaches this row — subtract it (accumulated below, in plan
            -- order) from the ONE live-row predicate. Two same-hash rows in the estate
            -- therefore predict released=false then released=true, exactly as the act
            -- strikes them.
            v_rel := ((SELECT count(*) FROM kb_blobs live
                        WHERE live.content_hash = v_row.content_hash
                          AND live.content_type IS NOT NULL)
                      - (SELECT count(*) FROM unnest(v_struck) prior
                          WHERE prior = v_row.content_hash)) <= 1;
            v_struck := v_struck || ARRAY[v_row.content_hash];
            v_blob_hash := v_blob_hash || ARRAY[v_row.content_hash];
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'would_strike', jsonb_build_object(
                    'blob_id',           v_row.id,
                    'content_hash',      v_row.content_hash,
                    'pathname',          v_row.blob_pathname,
                    'released_would_be', v_rel)));
            v_strikes := v_strikes || jsonb_build_array(jsonb_build_object(
                'blob_id',           v_row.id,
                'released_would_be', v_rel));
        ELSIF v_row.governed_home THEN
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'already-erased'));
        ELSE
            -- The named remainder: independent_obligation-shaped, hash named (D2 vocabulary),
            -- never admitted to the erased-content set.
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'independent_obligation: home governed by a team or map; '
                           || 'hash ' || v_row.content_hash || ' not struck'));
        END IF;
    END LOOP;

    v_hashes := v_hashes || v_blob_hash;

    -- ── Per-target outcomes, read against the PRE-redaction state (the mutations live in
    -- _erasure_apply_redaction alone; these reads only report what it will find).
    -- 20260911000000: every read below scopes to GOVERNED homes with the same predicate
    -- the redaction spells — the record reports "erased" only for rows the act actually
    -- redacts; a same-hash row in an ungoverned home is nobody's outcome (it is not the
    -- subject's data and was never a target). ────────────────────────────────────────────
    SELECT * INTO v_prof FROM kb_profiles WHERE id = p_subject;
    IF v_prof.tombstoned_at IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profiles.handle',        'outcome','already-erased'),
            jsonb_build_object('target','kb_profiles.display_name',  'outcome','already-erased'),
            jsonb_build_object('target','kb_profiles.tombstoned_at', 'outcome','already-erased'));
    ELSE
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profiles.handle',        'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_profiles.display_name',  'outcome','sentinel-scrubbed'),
            jsonb_build_object('target','kb_profiles.tombstoned_at', 'outcome','tombstoned'));
        IF v_prof.email IS NOT NULL THEN
            v_targets := v_targets || jsonb_build_array(
                jsonb_build_object('target','kb_profiles.email','outcome','erased'));
        END IF;
        IF v_prof.preferences <> '{}'::jsonb THEN
            v_targets := v_targets || jsonb_build_array(
                jsonb_build_object('target','kb_profiles.preferences','outcome','erased'));
        END IF;
    END IF;

    SELECT * INTO v_team FROM kb_teams t WHERE t.personal_of = p_subject;
    IF FOUND THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_teams.slug',
                'outcome', CASE WHEN v_team.slug = 'personal-erased-' || p_subject::text
                                THEN 'already-erased' ELSE 'sentinel-scrubbed' END),
            jsonb_build_object('target','kb_teams.name',
                'outcome', CASE WHEN v_team.slug = 'personal-erased-' || p_subject::text
                                THEN 'already-erased' ELSE 'sentinel-scrubbed' END));
    END IF;

    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
      JOIN kb_slack_grant_vault v ON l.profile_id = p_subject
       AND l.auth_provider = 'slack' AND v.slack_principal_id = l.auth_provider_user_id;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_slack_grant_vault.slack_principal_id',
                'outcome','deleted'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
      JOIN kb_slack_link_intents i ON l.profile_id = p_subject
       AND l.auth_provider = 'slack' AND i.slack_principal_id = l.auth_provider_user_id;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_slack_link_intents.slack_principal_id',
                'outcome','deleted'));
    END IF;

    -- The auth-link identifiers, the entity names, and the subject's own artifact content —
    -- outcomes read against the PRE-redaction state exactly like every arm above (the
    -- mutations live in _erasure_apply_redaction alone; these reads only report what it
    -- will find). The artifact remainder subset is named with the disposition-iii
    -- vocabulary the blob arm set: another principal's kind namespace on a governed
    -- resource is an independent obligation, re-named on every run because it stands.
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
     WHERE l.profile_id = p_subject AND l.email IS NOT NULL;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profile_auth_links.email',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_profile_auth_links l
     WHERE l.profile_id = p_subject
       AND l.auth_provider_user_id IS DISTINCT FROM 'erased-' || l.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_profile_auth_links.auth_provider_user_id',
                'outcome','sentinel-scrubbed'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_entities en
     WHERE en.profile_id = p_subject
       AND en.name IS DISTINCT FROM 'erased-' || en.id::text;
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_entities.name',
                'outcome','sentinel-scrubbed'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE da.kind_owner_table = 'kb_profiles'
       AND da.kind_owner_id = p_subject
       AND dac.content <> '{}'::jsonb
       AND da.resource_id = ANY(v_charters);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE dac.content <> '{}'::jsonb
       AND (da.kind_owner_table <> 'kb_profiles' OR da.kind_owner_id <> p_subject)
       AND da.resource_id = ANY(v_charters);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content (other-kind)',
                'outcome','independent_obligation: the artifact''s kind is owned by '
                           || 'another principal; content on a governed resource not '
                           || 'struck'));
    END IF;

    IF EXISTS (SELECT 1 FROM kb_chunk_content cc
                 JOIN kb_chunks c ON c.id = cc.chunk_id
                WHERE c.content_hash = ANY(v_hashes)
                  AND c.resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunk_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunk_content cc
                                               JOIN kb_chunks c ON c.id = cc.chunk_id
                                              WHERE c.content_hash = ANY(v_hashes)
                                                AND cc.content <> ''
                                                AND c.resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_block_content
                WHERE content_hash = ANY(v_hashes)
                  AND block_revision_id IN (
                      SELECT br.id FROM kb_block_revisions br
                        JOIN kb_content_blocks b ON b.id = br.block_id
                       WHERE b.resource_id IN (
                             SELECT h.resource_id FROM kb_resource_homes h
                              WHERE h.anchor_table = 'kb_contexts'
                                AND h.anchor_id = ANY(v_governed)))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_block_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_block_content
                                              WHERE content_hash = ANY(v_hashes)
                                                AND content <> ''
                                                AND block_revision_id IN (
                                                    SELECT br.id FROM kb_block_revisions br
                                                      JOIN kb_content_blocks b ON b.id = br.block_id
                                                     WHERE b.resource_id IN (
                                                           SELECT h.resource_id FROM kb_resource_homes h
                                                            WHERE h.anchor_table = 'kb_contexts'
                                                              AND h.anchor_id = ANY(v_governed))))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;

    -- The text remainder (the attribution ruling, 2026-09-13, on the 2026-09-06 clause):
    -- prose ATTRIBUTED to the subject by authorship — content blocks whose genesis event
    -- the subject's entity emitted — in homes outside the governed estate (team and map
    -- alike). Named for audit with count + hashes (capped at 8, "and N more" beyond),
    -- never struck, never admitted to v_hashes: nothing in a shared home is the act's to
    -- strike, and the redaction's reads stay governed-scoped. Attribution is the emitting
    -- entity, the blob arm's own emitted-half shape; direct authorship only — carried
    -- attribution is provenance the pseudonym break already covers.
    v_text_rem := (
        SELECT 'independent_obligation: home governed by a team or map; '
               || count(DISTINCT bc.content_hash) || ' text hash(s) not struck'
               || CASE WHEN count(DISTINCT bc.content_hash) <= 8
                       THEN ': ' || array_to_string(array_agg(DISTINCT bc.content_hash), ', ')
                       ELSE ': ' || array_to_string(ARRAY(
                                   SELECT DISTINCT bc2.content_hash
                                     FROM kb_block_content bc2
                                     JOIN kb_block_revisions br2 ON br2.id = bc2.block_revision_id
                                     JOIN kb_content_blocks b2 ON b2.id = br2.block_id
                                     JOIN kb_events ge2 ON ge2.id = b2.genesis_event_id
                                     JOIN kb_entities ge2_en ON ge2_en.id = ge2.emitter_entity_id
                                    WHERE ge2_en.profile_id = p_subject
                                      AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg2
                                                       WHERE hg2.resource_id = b2.resource_id
                                                         AND hg2.anchor_table = 'kb_contexts'
                                                         AND hg2.anchor_id = ANY(v_governed))
                                    ORDER BY 1 LIMIT 8), ', ')
                            || '; and ' || (count(DISTINCT bc.content_hash) - 8) || ' more'
                  END
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
          JOIN kb_events ge ON ge.id = b.genesis_event_id
          JOIN kb_entities ge_en ON ge_en.id = ge.emitter_entity_id
         WHERE ge_en.profile_id = p_subject
           AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg
                            WHERE hg.resource_id = b.resource_id
                              AND hg.anchor_table = 'kb_contexts'
                              AND hg.anchor_id = ANY(v_governed))
        HAVING count(DISTINCT bc.content_hash) > 0);
    IF v_text_rem IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_block_content.content',
            'outcome', v_text_rem));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_chunks
                WHERE content_hash = ANY(v_hashes)
                  AND resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.embedding',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL
                                                AND resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END),
            jsonb_build_object('target','kb_chunks.embedded_with',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL
                                                AND resource_id IN (
                                                    SELECT h.resource_id FROM kb_resource_homes h
                                                     WHERE h.anchor_table = 'kb_contexts'
                                                       AND h.anchor_id = ANY(v_governed)))
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    -- The heading trail, arm (4a) of the redaction (20261020100000). Claimed only when the act
    -- will null one: a chunk with no heading trail never held one, so "already-erased" would
    -- claim an act that never happened. A re-run of the act therefore omits the target.
    IF EXISTS (SELECT 1 FROM kb_chunks
                WHERE content_hash = ANY(v_hashes)
                  AND header_path IS NOT NULL
                  AND resource_id IN (
                      SELECT h.resource_id FROM kb_resource_homes h
                       WHERE h.anchor_table = 'kb_contexts'
                         AND h.anchor_id = ANY(v_governed))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.header_path', 'outcome','erased'));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_resource_search_index si
                WHERE si.resource_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                          WHERE c.content_hash = ANY(v_hashes)
                                            AND c.resource_id IN (
                                                SELECT h.resource_id FROM kb_resource_homes h
                                                 WHERE h.anchor_table = 'kb_contexts'
                                                   AND h.anchor_id = ANY(v_governed)))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_resource_search_index.search_vector',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_resource_search_index si
                                              WHERE si.resource_id IN (
                                                    SELECT DISTINCT c.resource_id FROM kb_chunks c
                                                     WHERE c.content_hash = ANY(v_hashes)
                                                       AND c.resource_id IN (
                                                           SELECT h.resource_id FROM kb_resource_homes h
                                                            WHERE h.anchor_table = 'kb_contexts'
                                                              AND h.anchor_id = ANY(v_governed)))
                                              AND si.search_vector <> '')
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;

    SELECT count(*) INTO v_n FROM kb_cogmaps m
     WHERE m.shape_materialized_event_id IS NOT NULL
       AND m.id IN (
           SELECT r.home_anchor_id FROM kb_cogmap_regions r
             JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
            WHERE r.home_anchor_table = 'kb_cogmaps' AND NOT r.is_folded
              AND mem.member_table = 'kb_resources'
              AND mem.member_id = ANY(v_resources));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_cogmaps.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_contexts g
      WHERE g.is_active
        AND g.id = ANY(v_governed);
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.is_active','outcome','retired'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_contexts c
     WHERE c.shape_materialized_event_id IS NOT NULL
       AND ( c.id IN (
               SELECT r.home_anchor_id FROM kb_cogmap_regions r
                 JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
                WHERE r.home_anchor_table = 'kb_contexts' AND NOT r.is_folded
                  AND mem.member_table = 'kb_resources'
                  AND mem.member_id = ANY(v_resources))
          -- Every estate context (R1).
          OR c.id = ANY(v_governed) );
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;

    -- The charters held (2a), and the resources already complete (2f), named in the record.
    v_targets := v_targets || coalesce((
        SELECT jsonb_agg(jsonb_build_object(
                   'target',  'kb_resources',
                   'outcome', 'charter ' || ch || '; held until map-grain erasure (task '
                              || '01a0e960-0ca2-7f42-b33e-1ed19b024e6b); text emptied; title, '
                              || 'properties, edges and ledger kept') ORDER BY ch)
          FROM unnest(v_charters) AS ch), '[]'::jsonb);
    IF (v_counts->>'skip')::integer > 0 THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_resources',
            'outcome', (v_counts->>'skip') || ' estate resource(s) already erased and complete; skipped'));
    END IF;

    RETURN jsonb_build_object(
        'estate_contexts', (SELECT coalesce(jsonb_agg(g ORDER BY g), '[]'::jsonb) FROM unnest(v_governed) AS g),
        'estate',          v_counts || jsonb_build_object('total', jsonb_array_length(v_resrows)),
        'resources',       v_resrows,
        'charters',        (SELECT coalesce(jsonb_agg(ch ORDER BY ch), '[]'::jsonb) FROM unnest(v_charters) AS ch),
        'redacted_hashes', to_jsonb(v_hashes),
        'targets',         v_targets,
        'already_erased',  (v_prof.tombstoned_at IS NOT NULL),
        'blob_strikes',    v_strikes);
END;
$$

;

-- Section 3. The survey door.
CREATE OR REPLACE FUNCTION principal_erasure_survey(p_subject uuid)
RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_plan      jsonb;
    v_targets   jsonb;
    v_strike    jsonb;
    v_i         integer;
    v_row       jsonb;
    v_rplan     jsonb;
    v_resources jsonb := '[]'::jsonb;
BEGIN
    v_plan := principal_erasure_survey_plan(p_subject);
    v_targets := v_plan->'targets';
    FOR v_i IN 0 .. jsonb_array_length(v_targets) - 1 LOOP
        v_strike := v_targets->v_i->'would_strike';
        IF v_strike IS NOT NULL THEN
            v_targets := jsonb_set(v_targets, ARRAY[v_i::text],
                jsonb_build_object(
                    'target',  'kb_blobs',
                    'outcome', blob_strike_outcome_text(
                                   (v_strike->>'released_would_be')::boolean,
                                   v_strike->>'pathname')));
        END IF;
    END LOOP;

    -- Per-resource counts (D6): each from that resource's OWN survey, the computation its
    -- resource erasure will consume, so the counts are predictions in the act's own terms. Counts
    -- only: the full per-resource plan is the resource survey door's answer, and inlining N of
    -- them would grow the person survey with the estate. Computed here, in the door, and never in
    -- the plan the act consumes, where each resource erasure computes its own.
    FOR v_row IN SELECT r FROM jsonb_array_elements(v_plan->'resources') r LOOP
        IF v_row->>'disposition' = 'erase' THEN
            v_rplan := resource_erasure_survey_plan((v_row->>'resource_id')::uuid);
            v_row := v_row || jsonb_build_object(
                'edges',            jsonb_array_length(v_rplan->'edges'),
                'remainder',        jsonb_array_length(v_rplan->'remainder'),
                'ledger_remainder', jsonb_array_length(v_rplan->'ledger_remainder'),
                'redacted_fields',  jsonb_array_length(v_rplan->'redacted_fields'));
        ELSIF v_row->>'disposition' = 'complete' THEN
            v_row := v_row || jsonb_build_object(
                'edges', 0, 'remainder', 0, 'ledger_remainder', 0,
                'redacted_fields', jsonb_array_length(
                    resource_erasure_completion_fields((v_row->>'resource_id')::uuid)));
        END IF;
        v_resources := v_resources || jsonb_build_array(v_row);
    END LOOP;

    RETURN jsonb_build_object(
        'estate',          v_plan->'estate',
        'estate_contexts', v_plan->'estate_contexts',
        'resources',       v_resources,
        'charters',        v_plan->'charters',
        'redacted_hashes', v_plan->'redacted_hashes',
        'targets',         v_targets,
        'already_erased',  v_plan->'already_erased',
        'blob_strikes',    (SELECT coalesce(jsonb_agg(jsonb_build_object(
                                'blob_id', s->>'blob_id',
                                'released', (s->>'released_would_be')::boolean)
                                ORDER BY ord),
                                '[]'::jsonb)
                             FROM jsonb_array_elements(v_plan->'blob_strikes')
                                  WITH ORDINALITY AS s(s, ord)));
END;
$$;

-- Section 4. The act.
CREATE OR REPLACE FUNCTION principal_erasure_execute(p_subject uuid, p_operator uuid, p_emitter uuid, p_request_ref uuid)
RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_plan     jsonb;
    v_targets  jsonb;
    v_hashes   text[] := '{}';
    v_row      jsonb;
    v_bid uuid; v_rel boolean; v_path text;
    v_i        integer;
    v_ev       uuid;
    v_out      jsonb;
    v_erasures jsonb := '[]'::jsonb;
    v_refs     jsonb;
BEGIN
    -- The request reference is the correlation id of every event the act appends: the resource
    -- erasures' events, the strikes and the completion. Replay finds the act's span by it (D7).
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'principal_erasure_execute: p_request_ref is required';
    END IF;

    -- ONE computation per act. The existence RAISE lives in the plan (its message keeps the
    -- act's voice); the Rust gate resolves existence before either door is reached.
    v_plan := principal_erasure_survey_plan(p_subject);
    v_targets := v_plan->'targets';
    -- The plan's hash array comes back out IN ORDER (ORDINALITY makes the order load-bearing,
    -- not incidental): it is the redacted set the event carries.
    v_hashes := (SELECT coalesce(array_agg(h ORDER BY ord), '{}')
                   FROM jsonb_array_elements_text(v_plan->'redacted_hashes')
                        WITH ORDINALITY AS t(h, ord));

    -- ── The resource erasures (R5, 2c): every `erase` and `complete` resource, in the plan's
    --    resource-id order, through resource_erasure_execute itself, so each is that act whole:
    --    its edge folds, its resource_erased record, its body and its ledger rewrite. No blob list
    --    is passed: the strike loop below strikes every live blob homed in the estate. Each takes
    --    its own resource's locks as it reaches it, in id order, so two person acts cannot
    --    deadlock on them. A raise from any of them (a raced fold or remote source) aborts the
    --    whole act; the service retries it. ──────────────────────────────────────────────────
    FOR v_row IN
        SELECT r FROM jsonb_array_elements(v_plan->'resources') r
         WHERE r->>'disposition' IN ('erase', 'complete')
    LOOP
        v_out := resource_erasure_execute((v_row->>'resource_id')::uuid, p_operator, p_emitter,
                                          p_request_ref, '{}'::uuid[]);
        -- Keyed `resource` and `event`, never `resource_id`: the completion payload carries no
        -- key a trail function joins on (the D2 shape), so no resource's trail ever reaches it.
        v_erasures := v_erasures || jsonb_build_array(jsonb_build_object(
            'resource', v_row->>'resource_id',
            'event',    v_out->>'event_id',
            'kind',     CASE v_row->>'disposition' WHEN 'erase' THEN 'erasure' ELSE 'completion' END));
    END LOOP;

    -- ── The strike loop: the plan's would-strike rows, IN ORDER, through the wrapper —
    -- per-row blob_delete('blob_erased', …) events exactly as before (the pairing with the
    -- completion is a fact, D1). Each structured entry is replaced IN PLACE with the prose
    -- built from the WRAPPER'S verdict — authoritative at strike time; the plan's
    -- released_would_be was a prediction and is discarded here.
    FOR v_i IN 0 .. jsonb_array_length(v_targets) - 1 LOOP
        v_row := v_targets->v_i->'would_strike';
        IF v_row IS NOT NULL THEN
            SELECT blob_id, released, pathname
              INTO v_bid, v_rel, v_path
              FROM blob_delete('blob_erased',
                               jsonb_build_object('blob_id', (v_row->>'blob_id')::uuid),
                               p_emitter,
                               p_correlation => p_request_ref);
            v_targets := jsonb_set(v_targets, ARRAY[v_i::text],
                jsonb_build_object(
                    'target',  'kb_blobs',
                    'outcome', blob_strike_outcome_text(v_rel, v_path)));
        END IF;
    END LOOP;

    -- ── The ONE completion event (D1), last in the record (2c, 2d). NULL-anchored (admin);
    -- emitter = the OPERATOR. It names the estate it reached (the redaction reads it back from
    -- here), the resource erasures it ran, in order, and the charters it holds. References carry
    -- the subject, the request, and each resource erasure (rel `erasure`).
    v_refs := jsonb_build_array(
        jsonb_build_object('rel','subject',
            'target', jsonb_build_object('kind','kb_profiles','id', p_subject)),
        jsonb_build_object('rel','request',
            'target', jsonb_build_object('kind','kb_events','id', p_request_ref)))
        || coalesce((SELECT jsonb_agg(jsonb_build_object('rel','erasure',
                         'target', jsonb_build_object('kind','kb_events','id', e->>'event'))
                         ORDER BY ord)
                       FROM jsonb_array_elements(v_erasures) WITH ORDINALITY AS t(e, ord)),
                    '[]'::jsonb);
    v_ev := _event_append(
        'principal_erased', p_emitter, NULL, NULL,
        jsonb_build_object(
            'subject_table',     'kb_profiles',
            'subject_id',        p_subject,
            'actor',             p_operator,
            'redacted_hashes',   to_jsonb(v_hashes),
            'targets',           v_targets,
            'estate_contexts',   v_plan->'estate_contexts',
            'resource_erasures', v_erasures,
            'charters_held',     v_plan->'charters'),
        p_references => v_refs,
        p_correlation => p_request_ref);

    -- ── The identity arms, the charters' text, the watermarks and the custody closure: the ONE
    --    definition, reading the estate from the event just appended.
    PERFORM _erasure_apply_redaction(p_subject, v_hashes, v_ev);

    RETURN jsonb_build_object(
        'event_id',          v_ev,
        'redacted_hashes',   to_jsonb(v_hashes),
        'targets',           v_targets,
        'already_erased',    (v_plan->>'already_erased')::boolean,
        'estate_contexts',   v_plan->'estate_contexts',
        'resource_erasures', v_erasures,
        'charters_held',     v_plan->'charters');
END;
$$;

-- Section 5. The redaction reads the recorded estate.
CREATE OR REPLACE FUNCTION _erasure_apply_redaction(p_subject uuid, p_hashes text[], p_event uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
    v_sentinel text := 'erased-' || p_subject::text;
    v_team     uuid;
    -- The estate the act recorded (D1 of the 2026-10-10 person-erasure design): the subject's
    -- @me contexts and their personal team's contexts, read from THIS event's payload, never
    -- re-derived. Live and replay therefore reach the same set whatever has happened to
    -- personal_of since. Required: no principal was erased before the key existed (Q5, ruled
    -- 2026-10-10, guarded by 20261021100000), so an event without it is a defect, not history.
    v_estate   uuid[] := (SELECT ARRAY(SELECT jsonb_array_elements_text(e.payload->'estate_contexts')::uuid)
                            FROM kb_events e
                           WHERE e.id = p_event AND e.payload ? 'estate_contexts');
BEGIN
    IF v_estate IS NULL THEN
        RAISE EXCEPTION '_erasure_apply_redaction: event % records no estate_contexts', p_event;
    END IF;

    -- (1) The personal-team denormalization (D5 blind-spot row 1 — the only denormalization
    --     carrier Derivation C found). Found by kb_teams.personal_of, never by recomputing the
    --     slug: a personal team whose slug was held at genesis carries a `-N` suffix, and the
    --     team at the bare slug is someone else's (20261013100000). Scrubbed to the same
    --     derivation applied to the sentinel identity. The trigger is AFTER INSERT only, so
    --     this never re-fires.
    SELECT t.id INTO v_team FROM kb_teams t WHERE t.personal_of = p_subject;

    UPDATE kb_teams
       SET slug = 'personal-' || v_sentinel,
           name = v_sentinel || ' (personal)'
      WHERE id = v_team;

    -- Arms (2)-(5), (11) and (12)'s member arm reach by hash. Since 20261021100000 every estate
    -- resource but a charter has already been erased by resource erasure in this transaction,
    -- row by row, before this body runs, and the act's hashes are the charters' and the struck
    -- blobs' alone (D5). So for a husk these arms find nothing left to empty, and what they still
    -- do is empty the charters the act holds (D3).
    --
    -- (2) Chunk prose emptied BY HASH, hash kept (D3: emptied, not deleted — the CAS
    --     retention rule never fires) — GOVERNED HOMES ONLY (ruled 2026-09-10, decision
    --     01a08dc2): the hash is the record key, never the reach. A same-hash chunk in a
    --     home the subject does not govern keeps its prose; erasure never reaches beyond
    --     the estate its subject governs. The predicate is HOME-CUSTODY shaped, not
    --     authorship shaped: a resource homed in the subject's governed context but owned
    --     by a grantee is IN the estate (the ruling — the context wipes with the estate)
    --     and its content redacts; a same-hash row in a home the subject does not govern
    --     is never the subject's to erase.
    UPDATE kb_chunk_content cc
       SET content = ''
      FROM kb_chunks c
     WHERE cc.chunk_id = c.id
       AND c.content_hash = ANY(p_hashes)
       AND cc.content <> ''
       AND c.resource_id IN (
           SELECT h.resource_id
             FROM kb_resource_homes h
            WHERE h.anchor_table = 'kb_contexts'
              AND h.anchor_id = ANY(v_estate));

    -- (3) Verbatim block bytes, same shape, same governed reach (the content row resolves
    --     to its resource through the revision chain).
    UPDATE kb_block_content
       SET content = ''
      WHERE content_hash = ANY(p_hashes)
        AND content <> ''
        AND block_revision_id IN (
            SELECT br.id
              FROM kb_block_revisions br
              JOIN kb_content_blocks b ON b.id = br.block_id
             WHERE b.resource_id IN (
                   SELECT h.resource_id
                     FROM kb_resource_homes h
                    WHERE h.anchor_table = 'kb_contexts'
                      AND h.anchor_id = ANY(v_estate)));

    -- (4) Embeddings + provenance nulled TOGETHER — `embedding IS NULL` and
    --     `embedded_with IS NULL` can never disagree (20260713000040:84-87). Governed
    --     homes only: a same-hash chunk elsewhere keeps its vector and stays searchable.
    UPDATE kb_chunks
       SET embedding = NULL,
           embedded_with = NULL
      WHERE content_hash = ANY(p_hashes)
        AND embedding IS NOT NULL
        AND resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate));

    -- (4a) The heading trail nulled (20261020100000): header_path is authored heading prose,
    --      so a heading naming the subject is the subject's text like the chunk body (2). The
    --      resource act's D2.1 spelling (NULL, not ''). Same hash, same governed reach: a
    --      same-hash chunk elsewhere keeps its own trail.
    UPDATE kb_chunks
       SET header_path = NULL
      WHERE content_hash = ANY(p_hashes)
        AND header_path IS NOT NULL
        AND resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate));

    -- (5) Search vectors emptied wholesale for every hash-affected GOVERNED resource (the
    --     _rebuild_resource_search_vector site, 20260711000060:74-83): the vector folds
    --     title+body+meta, so a partial redaction would leave redacted terms searchable.
    --     Resources outside the governed homes keep their vectors — their content was
    --     never touched.
    UPDATE kb_resource_search_index si
       SET search_vector = ''
      WHERE si.resource_id IN (
            SELECT DISTINCT c.resource_id FROM kb_chunks c
             WHERE c.content_hash = ANY(p_hashes)
               AND c.resource_id IN (
                   SELECT h.resource_id
                     FROM kb_resource_homes h
                    WHERE h.anchor_table = 'kb_contexts'
                      AND h.anchor_id = ANY(v_estate)))
        AND si.search_vector <> '';

    -- (6) The erased-content set (D4): first admit keeps attribution — ON CONFLICT DO
    --     NOTHING is what makes a ledger-order rebuild byte-identical. The set remains the
    --     RECORD of what the act redacted (hash + admitting event); its former role as an
    --     instance-wide write-path oracle is retired (see block_mutate below).
    INSERT INTO kb_erased_content (content_hash, erased_by_event_id)
    SELECT h, p_event FROM unnest(p_hashes) AS h
    ON CONFLICT (content_hash) DO NOTHING;

    -- (7) The tombstone (D5): identifiers nulled/sentinel'd, the UUID KEPT — it is the
    --     pseudonym. tombstoned_at is the admitting event's occurred_at, never now().
    UPDATE kb_profiles pr
       SET handle        = v_sentinel,
           display_name  = v_sentinel,
           email         = NULL,
           preferences   = '{}'::jsonb,
           tombstoned_at = COALESCE(pr.tombstoned_at, v_occurred)
      WHERE pr.id = p_subject;

    -- (8) The no-FK external identifiers (D5 blind-spot row 3): they identify the person
    --     through an EXTERNAL system, so the pseudonym break does not cover them — deleted,
    --     the slack_disconnect_service.rs:168,224 precedent. The principals resolve through
    --     the auth-link rows (auth_provider 'slack' — slack_link_service::SLACK_AUTH_PROVIDER).
    DELETE FROM kb_slack_grant_vault v
     USING kb_profile_auth_links l
      WHERE l.profile_id = p_subject
        AND l.auth_provider = 'slack'
        AND v.slack_principal_id = l.auth_provider_user_id;

    DELETE FROM kb_slack_link_intents i
     USING kb_profile_auth_links l
      WHERE l.profile_id = p_subject
        AND l.auth_provider = 'slack'
        AND i.slack_principal_id = l.auth_provider_user_id;

    -- (9) The auth-link identifiers (the manifest's identifier | full columns on
    --     kb_profile_auth_links): the email and the IdP's subject id identify the person
    --     DIRECTLY — the pseudonym break does not cover them, the same reasoning as the
    --     Slack stores above. They are unclaimed only HERE, after (8): the Slack-store
    --     deletes resolve their principals THROUGH auth_provider_user_id, so nulling first
    --     would leave the external stores standing over a destroyed lookup. email is
    --     nullable and NULLed; auth_provider_user_id is NOT NULL and sentineled — the D5
    --     nulled/sentinel'd split. THE SENTINEL IS THE ROW'S OWN ID ('erased-' || id): the
    --     kb_profiles derivation generalized, and unique by construction under
    --     kb_profile_auth_links_auth_provider_auth_provider_user_id_key, which two rows of
    --     one provider sharing a subject-keyed sentinel would violate. Every provider's
    --     rows, not just Slack's: the manifest declares the columns, not a provider.
    UPDATE kb_profile_auth_links l
       SET email                 = NULL,
           auth_provider_user_id = 'erased-' || l.id::text
      WHERE l.profile_id = p_subject
        AND (l.email IS NOT NULL OR l.auth_provider_user_id IS DISTINCT FROM 'erased-' || l.id::text);

    -- (10) The subject's entities' names (manifest identifier | full: an agent-instance
    --      name may embed the person's name). Sentineled to 'erased-' || the entity's own
    --      id — the display_name sentinel derivation (the row's kept UUID is the
    --      pseudonym), and the ONLY per-row spelling that satisfies the
    --      (profile_id, name) UNIQUE grain: two of the subject's entities cannot share
    --      one subject-keyed sentinel. The entity UUIDs stay.
    UPDATE kb_entities en
       SET name = 'erased-' || en.id::text
      WHERE en.profile_id = p_subject
        AND en.name IS DISTINCT FROM 'erased-' || en.id::text;

    -- (11) The subject's own data-artifact content (manifest content | full). Artifacts are
    --      reached only through what points at them, so the reach is the governed-home join:
    --      the resource is homed in one of the subject's OWN personal contexts (the same
    --      owner arm (12) spells) AND the kind is owned by the subject's profile. Emptied,
    --      never deleted — the artifact id, kb_data_artifacts.content_hash and
    --      kb_data_artifact_content.content_hash all stay, keeping the metadata/bytes split's
    --      hash-retention shape (D3). A team-kind or other-profile-kind artifact on the
    --      subject's governed resource is NOT the subject's data — the disposition-iii
    --      remainder the act's targets name, never struck here. NOT admitted to
    --      kb_erased_content: the set is the record of the act's redacted hashes, and no
    --      artifact consult site exists — the tombstone is what keeps the subject from
    --      re-committing.
    UPDATE kb_data_artifact_content dac
       SET content = '{}'::jsonb
      FROM kb_data_artifacts da
      WHERE dac.artifact_id = da.id
        AND da.kind_owner_table = 'kb_profiles'
        AND da.kind_owner_id = p_subject
        AND da.resource_id IN (
            SELECT h.resource_id
              FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts'
               AND h.anchor_id = ANY(v_estate))
        AND dac.content <> '{}'::jsonb;

    -- (12) The centroid sites MARKED for recompute (D5): the formation watermark nulled on
    --     every anchor whose live regions hold an ESTATE member, plus every estate context
    --     (their telos aggregates their goals). Keyed by home since 20261021100000, not by the
    --     act's hashes, which are now the charters' alone: every estate resource is erased,
    --     and resource erasure's own step (7) marks only a resource's home context and the maps
    --     holding it, never another principal's context whose region holds it.
    UPDATE kb_cogmaps m
       SET shape_materialized_event_id = NULL
      WHERE m.shape_materialized_event_id IS NOT NULL
        AND m.id IN (
            SELECT r.home_anchor_id
              FROM kb_cogmap_regions r
              JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
             WHERE r.home_anchor_table = 'kb_cogmaps'
               AND NOT r.is_folded
               AND mem.member_table = 'kb_resources'
               AND mem.member_id IN (SELECT h.resource_id
                                      FROM kb_resource_homes h
                                     WHERE h.anchor_table = 'kb_contexts'
                                       AND h.anchor_id = ANY(v_estate)));

    UPDATE kb_contexts c
       SET shape_materialized_event_id = NULL
      WHERE c.shape_materialized_event_id IS NOT NULL
        AND ( c.id IN (
                SELECT r.home_anchor_id
                  FROM kb_cogmap_regions r
                  JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
                 WHERE r.home_anchor_table = 'kb_contexts'
                   AND NOT r.is_folded
                   AND mem.member_table = 'kb_resources'
                   AND mem.member_id IN (SELECT h.resource_id
                                      FROM kb_resource_homes h
                                     WHERE h.anchor_table = 'kb_contexts'
                                       AND h.anchor_id = ANY(v_estate)))
            -- Every estate context, the personal team's included (R1).
            OR c.id = ANY(v_estate) );

    -- (13) CUSTODY CLOSURE (the offboarding ruling: grantees lose access to a dead
    --     principal's context). Retiring the governed contexts floors every access
    --     predicate that touches the estate — `can()`'s subject-liveness arm
    --     (20260902000010) checks kb_contexts.is_active, and context_authorable_by_
    --     profile / can_modify_resource consult it — so grants INTO the estate die with
    --     the context's activation, and no live principal can re-admit content into the
    --     wiped homes. UN-EVENTED BY THE SAME PRECEDENT AS ARM (7): kb_contexts is a
    --     replay INPUT table restored verbatim (replay.rs INPUT_TABLES), like kb_profiles;
    --     is_active is not identity-bearing (the 20260826000120 evented-retire defect is
    --     slug-specific — the walk's context_renamed never touches is_active), so the
    --     verbatim restore carries the retired state and the diff stays byte-identical.
    --     Idempotent: replay re-runs this arm against already-retired input rows.
    --     Every estate context is retired, the personal team's included (R1).
    UPDATE kb_contexts g
       SET is_active = false
      WHERE g.is_active
        AND g.id = ANY(v_estate);
END;
$$


;

-- Section 6. The principal_erased payload_schema, re-registered with the three new optional keys
-- (estate_contexts, resource_erasures, charters_held). None is required and the schema sets no
-- additionalProperties, so every payload valid before stays valid. The literal is the committed
-- fixture crates/temper-substrate/tests/fixtures/payloads/principal_erased.v1.schema.json, pasted
-- byte for byte; payload_schema.rs pins the two together.
UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PrincipalErased",
  "description": "`principal_erased` — the ONE admin event of a completed erasure (erasure spec D1).\n\nONE TYPE for identity erasure and content erasure: the distinction rides the per-target\noutcomes as data, never the type boundary — two types would cost the pairing (one request,\none operator, one reference) and double registration for no additional guarantee. The\nsubject is the PSEUDONYM the act itself broke; the event never re-identifies — no name, no\nemail, no case description. The request reference does not ride here: it lives on\n`kb_events.\"references\"`, the apparatus this act owns.\n\nThe redacted set is keyed on CONTENT HASH (D2) — never row ids, which would collide with\nthe trail functions' join-key shapes. The erased-content set (D4) rebuilds from these\npayloads alone.",
  "type": "object",
  "properties": {
    "actor": {
      "description": "The acting operator, distinguishable from the subject — self-serve vs administrative\nis a comparison the auditor needs. `None` only where no actor exists to name; inventing\none would put a fabricated attribution on the ledger (the standing-event precedent).",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "charters_held": {
      "description": "The estate's charters, emptied and held until map-grain erasure (D3).",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "estate_contexts": {
      "description": "The estate the act reached (20261021100000, person-erasure design D1): the subject's @me\ncontexts and their personal team's contexts, in id order. The redaction reads it back from\nhere, so replay reaches the set live reached.",
      "type": "array",
      "items": {
        "type": "string",
        "format": "uuid"
      }
    },
    "redacted_hashes": {
      "description": "The redacted set (D2): bare sha256 hex, exactly as `content_hash` carries it — the key\nevery reader already shares and no trail join-key shape can match.",
      "type": "array",
      "items": {
        "type": "string"
      }
    },
    "resource_erasures": {
      "description": "The resource erasures the act ran over the estate, in the order it ran them (D4).",
      "type": "array",
      "items": {
        "$ref": "#/$defs/EstateResourceErasureRef"
      }
    },
    "subject_id": {
      "description": "The pseudonym — `kb_profiles.id`. It survives; its identifying power does not (D5).",
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "$ref": "#/$defs/AnchorTable"
    },
    "targets": {
      "description": "Per-target outcomes and the named remainder (D6's accepted-in-part arm): anything\nunhonourable is named here. Partial completion is data, never a silent success.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/ErasureTargetOutcome"
      }
    }
  },
  "required": [
    "subject_table",
    "subject_id"
  ],
  "$defs": {
    "AnchorTable": {
      "description": "A polymorphic anchor/endpoint reference. Serializes table names exactly as the DDL spells them.",
      "type": "string",
      "enum": [
        "kb_contexts",
        "kb_cogmaps",
        "kb_resources",
        "kb_edges",
        "kb_content_blocks",
        "kb_teams",
        "kb_profiles",
        "kb_connections",
        "kb_machine_clients",
        "kb_blobs",
        "kb_events"
      ]
    },
    "ErasureTargetOutcome": {
      "description": "One target of a completed erasure and what happened to it (erasure spec, \"per-target\noutcomes\"). The target names itself the way the personal-data manifest does — `table` or\n`table.column`; the outcome is the act's own record of what redaction applied. Deliberately\nopen-textured in v1: ceilings are DATA, not types (D1), and the per-target vocabulary is the\nexecution build's to pin. `unhonourable_scope` outcomes land here, never silent.",
      "type": "object",
      "properties": {
        "outcome": {
          "description": "What the act did to it (erased / sentinel-scrubbed / accepted-in-part / …).",
          "type": "string"
        },
        "target": {
          "description": "Manifest identity of the target (`kb_profiles.display_name`, `kb_teams.slug`, …).",
          "type": "string"
        }
      },
      "required": [
        "target",
        "outcome"
      ]
    },
    "EstateResourceErasureKind": {
      "description": "Whether that resource erasure was a first erasure or a completion pass (D2's `complete`).",
      "type": "string",
      "enum": [
        "erasure",
        "completion"
      ]
    },
    "EstateResourceErasureRef": {
      "description": "One resource erasure a person act ran: its subject and its `resource_erased` event. Keyed\n`resource` and `event`, never `resource_id`: the completion payload carries no key the trail\nfunctions join on.",
      "type": "object",
      "properties": {
        "event": {
          "type": "string",
          "format": "uuid"
        },
        "kind": {
          "$ref": "#/$defs/EstateResourceErasureKind"
        },
        "resource": {
          "type": "string",
          "format": "uuid"
        }
      },
      "required": [
        "resource",
        "event",
        "kind"
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}
$JS$::jsonb
 WHERE name = 'principal_erased';

SELECT declare_migration(
    20261021100000,
    'additive',
    'One new function (_erasure_estate_contexts), the principal_erased payload_schema re-registered with three optional keys (none required; every payload valid before stays valid), and CREATE OR REPLACE of principal_erasure_survey_plan, principal_erasure_survey, principal_erasure_execute and _erasure_apply_redaction with signatures unchanged, plus a guard that raises if a principal_erased event already exists. The person act now runs resource_erasure_execute over every estate resource (the @me and personal-team contexts), and its event gains estate_contexts, resource_erasures and charters_held. A binary without this migration keeps working: no table, column, constraint or grant changes, and the functions are called as before; their results only gain keys.'
);
