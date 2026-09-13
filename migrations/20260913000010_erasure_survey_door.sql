-- The erasure survey door (task 01a09628 item 2, ruled 2026-09-12 with Pete): a read-only
-- preview beside the execute door, sharing the act's ONE computation — a preview that can
-- disagree with the act is worse than no preview. The act's scope computation (governed
-- contexts, home-pure resources, text hashes, the per-target outcome arms reading
-- PRE-redaction state) leaves `principal_erasure_execute` and moves whole into
-- `principal_erasure_survey_plan`, which the act then CONSUMES: computed ONCE per act, the
-- strike loop consumes the plan's rows by id in plan order, nothing re-enumerates — the act
-- and the survey cannot drift, by construction (8656d533's reblock_partition_in_tx). The
-- subject's-own live governed-home blob rows come out as STRUCTURED would-strike entries
-- (id, hash, pathname, released_would_be — the survey predicts each strike's verdict under
-- the act's own sequential refcount: same-hash rows earlier in the strike set are already
-- emptied when the act reaches this row), rendered through the ONE prose template so its
-- targets array is exactly the record the act would write. A refused survey records
-- NOTHING: a non-operator at the survey door gets a silent 404 and no
-- principal_erasure_refused event — a survey attempt is not an erasure request.
--
-- The act's observable record is BYTE-IDENTICAL: the strike loop keeps per-row blob_delete
-- ('blob_erased', …) through the wrapper (its verdict is authoritative at strike time), the
-- ONE principal_erased event keeps its payload keys, references and correlation, and
-- _erasure_apply_redaction runs as before. The strike-outcome prose template — parsed by
-- exact prefix in erasure_fence_service::classify_blob_outcome — is extracted to ONE
-- definition (blob_strike_outcome_text), byte-unchanged from 20260909000025:367 /
-- 20260911000010:116-118.
--
-- Additive: one new function + CREATE OR REPLACE only, signatures unchanged (the
-- 20260804000020 class).

CREATE FUNCTION blob_strike_outcome_text(p_released boolean, p_pathname text)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT 'erased; released=' || p_released::text || '; pathname=' || p_pathname
$$;

COMMENT ON FUNCTION blob_strike_outcome_text(boolean, text) IS
'the ONE strike-outcome prose template (extracted from 20260909000025:367 /
20260911000010:116-118 — task 01a09628 item 2): ''erased; released=<bool>;
pathname=<text>''. The erasure byte-delete fence parses this template by EXACT prefix
(erasure_fence_service::classify_blob_outcome) — the output must stay byte-identical; the
survey renders its would-strike predictions through it so its targets array is the record
the act would write.';

-- THE shared erasure computation — the act's own scope machinery, read-only (task 01a09628
-- item 2; body moved whole from principal_erasure_execute, 20260911000010:47-442). The act
-- consumes the plan (computed ONCE per act; strikes consume its rows by id, in plan order;
-- nothing re-enumerates) and the survey door serves the same plan: same-state, the survey
-- predicts each strike's verdict under the act's own sequential refcount — same-hash rows
-- earlier in the strike set are already emptied when the act reaches this row — and the
-- wrapper's strike-time verdict stays authoritative for writers after the survey. The
-- subject's-own LIVE governed-home blob rows are the only structural difference: the plan
-- reports each as a would_strike object (blob_id, content_hash, pathname, released_would_be)
-- where the act puts the struck prose. Already-erased own rows, both guest arms and every
-- other target keep the act's exact prose, in the act's exact order.
CREATE FUNCTION principal_erasure_survey_plan(p_subject uuid)
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
    v_prof  kb_profiles%ROWTYPE;
BEGIN
    SELECT id INTO v_exists FROM kb_profiles WHERE id = p_subject;
    IF v_exists IS NULL THEN
        RAISE EXCEPTION 'principal_erasure_execute: subject % not found', p_subject;
    END IF;

    -- Governed (personal) contexts — the schema's own owner arm, exactly as the read model
    -- spells it (contexts_readable_by arm 1, 20260712000010:96-97): owner_table =
    -- 'kb_profiles' AND owner_id = subject. Team-owned and team-shared contexts (arms 2-3)
    -- are NOT governed — disposition iii. SNAPSHOT INVARIANT: nothing between this read and
    -- the redaction may mutate kb_contexts or kb_resource_homes — verified for this
    -- transaction's other bodies (blob_delete, _event_append touch neither) — so the
    -- redaction's per-arm re-derivation of the same predicate always agrees with it.
    SELECT coalesce(array_agg(id), '{}') INTO v_governed
      FROM kb_contexts
     WHERE owner_table = 'kb_profiles'
       AND owner_id = p_subject;

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

    -- The text hashes: chunk content hashes + verbatim block content hashes.
    SELECT coalesce(array_agg(DISTINCT h), '{}') INTO v_hashes FROM (
        SELECT c.content_hash AS h FROM kb_chunks c
         WHERE c.resource_id = ANY(v_resources)
        UNION
        SELECT bc.content_hash AS h
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
         WHERE b.resource_id = ANY(v_resources)
    ) s;

    -- ── The blob pass: the subject's OWN rows, WOULD-STRIKE structured (the survey form of
    -- the act's in-act pre-pass, 20260911000010:97-131) ─────────────────────────────────
    -- Scope = the subject's blob rows (own half: owner/originator; emitted half: the commit
    -- event's emitter). Governed-home LIVE rows report would-strike entries; governed-home
    -- struck rows report already-erased; EVERY other home is the named remainder
    -- (disposition iii). released_would_be SIMULATES the act's own sequential refcount: the
    -- ONE blob_delete live-row predicate (live rows with the hash, this one included, <= 1)
    -- minus the same-hash rows EARLIER IN THE PLAN'S OWN STRIKE SET — execute consumes the
    -- entries in order, so those siblings are already emptied when the act reaches this row.
    -- Run at survey time WITHOUT the hash's advisory lock: a PREDICTION, honest about the
    -- moment it ran; the wrapper's strike-time verdict is authoritative in the act. The
    -- enumeration is ORDERED BY id — the act strikes in plan order and the survey must walk
    -- the SAME sequence when it simulates it, so the order cannot be left to the scan.
    FOR v_row IN
        SELECT b.id, b.content_hash, b.blob_pathname, b.content_type,
               (b.home_table = 'kb_contexts' AND b.home_id = ANY(v_governed)) AS governed_home
          FROM kb_blobs b
         WHERE b.owner_profile_id = p_subject
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
            -- order) from the ONE live-row predicate. A subject holding the same hash in
            -- two of their own governed homes therefore predicts released=false then
            -- released=true, exactly as the act strikes them.
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

    -- ── The guest rows: NAMED, never struck (the blob-arm ruling, 2026-09-11) ────────────
    -- The arm above walks the subject's OWN rows; a guest's committed row homed in a
    -- governed context is the guest's own content — accepted as surviving, but never
    -- silent. These are enumerated BY HOME — every live governed-home row whose actor
    -- halves are not the subject's — and named independent_obligation-shaped with blob id
    -- + content hash, exactly as team-held rows are named (the prefix is the fence's known
    -- non-delete-target shape; never prose the fence cannot parse). The retention is named
    -- WITHOUT a release path for BOTH arms: the act itself retires the governed contexts,
    -- and the context floor (contexts_readable_by's is_active arms) closes the delete
    -- door's gate read for every caller, guest included — custody is never consulted
    -- post-act. The arms differ only in provenance: ATTACHED — a live relation to an
    -- ESTATE RESOURCE (the delete act's own attached shape; non-resource peers fail its
    -- custody closed) — the relation outlives the act; UNATTACHED, there is no relation
    -- and the home owner is the tombstoned subject. Not admitted to kb_erased_content:
    -- only strikes enter the set.
    FOR v_row IN
        SELECT b.id, b.content_hash,
               EXISTS (SELECT 1 FROM kb_edges e
                        WHERE NOT e.is_folded
                          AND ((e.source_table = 'kb_blobs' AND e.source_id = b.id
                                 AND e.target_table = 'kb_resources')
                            OR (e.target_table = 'kb_blobs' AND e.target_id = b.id
                                 AND e.source_table = 'kb_resources'))) AS attached
          FROM kb_blobs b
         WHERE b.home_table = 'kb_contexts'
           AND b.home_id = ANY(v_governed)
           AND b.content_type IS NOT NULL
           AND b.owner_profile_id <> p_subject
           AND b.originator_profile_id <> p_subject
           AND NOT EXISTS (
               SELECT 1 FROM kb_events e
                 JOIN kb_entities en ON en.id = e.emitter_entity_id
                WHERE en.profile_id = p_subject
                  AND e.id = b.asserted_by_event_id)
    LOOP
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_blobs',
            'outcome', 'independent_obligation: committed by a guest of the erased principal; '
                       || 'blob ' || v_row.id::text || '; hash ' || v_row.content_hash
                       || '; retained with no release path — '
                       || CASE WHEN v_row.attached
                               THEN 'a live relation to an estate resource outlives the act, '
                                    || 'but the context floor closes every read into the '
                                    || 'retired home, so no custodian resolves'
                               ELSE 'unattached, and the home''s owner is the erased '
                                    || 'subject, so no custodian resolves'
                          END));
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

    SELECT * INTO v_team FROM kb_teams t
      JOIN kb_profiles pr ON pr.id = p_subject
     WHERE t.slug = 'personal-' || pr.handle;
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
       AND da.resource_id IN (
           SELECT h.resource_id FROM kb_resource_homes h
            WHERE h.anchor_table = 'kb_contexts'
              AND h.anchor_id IN (
                  SELECT c.id FROM kb_contexts c
                   WHERE c.owner_table = 'kb_profiles'
                     AND c.owner_id = p_subject));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_data_artifact_content.content',
                'outcome','erased'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_data_artifacts da
      JOIN kb_data_artifact_content dac ON dac.artifact_id = da.id
     WHERE dac.content <> '{}'::jsonb
       AND (da.kind_owner_table <> 'kb_profiles' OR da.kind_owner_id <> p_subject)
       AND da.resource_id IN (
           SELECT h.resource_id FROM kb_resource_homes h
            WHERE h.anchor_table = 'kb_contexts'
              AND h.anchor_id IN (
                  SELECT c.id FROM kb_contexts c
                   WHERE c.owner_table = 'kb_profiles'
                     AND c.owner_id = p_subject));
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
              AND mem.member_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                     WHERE c.content_hash = ANY(v_hashes)
                                       AND c.resource_id IN (
                                           SELECT h.resource_id FROM kb_resource_homes h
                                            WHERE h.anchor_table = 'kb_contexts'
                                              AND h.anchor_id = ANY(v_governed))));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_cogmaps.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_contexts g
      WHERE g.is_active
        AND g.owner_table = 'kb_profiles'
        AND g.owner_id = p_subject;
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
                  AND mem.member_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                         WHERE c.content_hash = ANY(v_hashes)
                                           AND c.resource_id IN (
                                               SELECT h.resource_id FROM kb_resource_homes h
                                                WHERE h.anchor_table = 'kb_contexts'
                                                  AND h.anchor_id = ANY(v_governed))))
          -- The personal-context predicate again — the schema's own owner arm
          -- (contexts_readable_by arm 1, 20260712000010:96-97); team-owned and team-shared
          -- contexts (arms 2-3) are NOT governed — disposition iii.
          OR (c.owner_table = 'kb_profiles' AND c.owner_id = p_subject) );
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;

    RETURN jsonb_build_object(
        'redacted_hashes', to_jsonb(v_hashes),
        'targets',         v_targets,
        'already_erased',  (v_prof.tombstoned_at IS NOT NULL),
        'blob_strikes',    v_strikes);
END;
$$;

COMMENT ON FUNCTION principal_erasure_survey_plan(uuid) IS
'THE shared erasure computation (task 01a09628 item 2; the act''s scope machinery moved
whole out of principal_erasure_execute, 20260911000010): governed contexts, home-pure
resources, text hashes, the subject''s-own blob pass as STRUCTURED would-strike entries
(id, hash, pathname, released_would_be — the survey predicts each strike''s verdict under
the act''s own sequential refcount: the blob_delete live-row predicate minus the same-hash
rows earlier in the strike set, already emptied when the act reaches this row), the guest
naming pass, and every per-target outcome arm reading PRE-redaction state, in the act''s
exact order. Read-only: it appends no event, mutates no row. principal_erasure_execute
consumes this plan (computed ONCE per act; strikes by id, in plan order; nothing
re-enumerates) and the survey door serves the same plan — the would-strike verdicts speak
for the moment the plan ran; the wrapper''s strike-time verdict is authoritative.';

-- The survey door's read: the plan + prose — every would-strike entry rendered through the
-- ONE template, so the targets array is EXACTLY the record the act would write (task
-- 01a09628 item 2). blob_strikes stays structured ({blob_id, released}) for the typed Rust
-- verdicts; released there is the same PREDICTION the plan computed. Records nothing:
-- no event, no mutation — a refused survey records nothing either (a survey attempt is not
-- an erasure request, ruled 2026-09-12).
CREATE FUNCTION principal_erasure_survey(p_subject uuid)
RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_plan    jsonb;
    v_targets jsonb;
    v_strike  jsonb;
    v_i       integer;
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
    RETURN jsonb_build_object(
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

COMMENT ON FUNCTION principal_erasure_survey(uuid) IS
'the read-only survey door''s answer (task 01a09628 item 2): principal_erasure_survey_plan
plus prose — every would-strike entry rendered through blob_strike_outcome_text so targets
is exactly the record the act would write, and blob_strikes kept structured ({blob_id,
released}) for the typed caller. Predictions, honest about the moment they ran; records
nothing (no event, no mutation — a non-operator at the survey door gets the silent 404 the
Rust gate renders, never a principal_erasure_refused event).';

-- The act, re-expressed as a CONSUMER of the plan (task 01a09628 item 2; body was
-- 20260911000010:27-475, whose computation moved whole into principal_erasure_survey_plan).
-- Signature unchanged. THE INVARIANT: the plan is computed ONCE per act; the strike loop
-- consumes the plan's rows by id, in plan order; NOTHING re-enumerates. The plan order is
-- load-bearing for the verdicts too: each blob_delete counts live rows at ITS moment, so
-- the survey's released_would_be predicts each strike's verdict under the act's own
-- sequential refcount — same-hash rows earlier in the strike set are already emptied when
-- the act reaches this row.
CREATE OR REPLACE FUNCTION principal_erasure_execute(
    p_subject     uuid,
    p_operator    uuid,
    p_emitter     uuid,
    p_request_ref uuid
) RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_plan    jsonb;
    v_targets jsonb;
    v_hashes  text[] := '{}';
    v_row     jsonb;
    v_bid uuid; v_rel boolean; v_path text;
    v_i    integer;
    v_ev   uuid;
BEGIN
    -- ONE computation per act. The existence RAISE lives in the plan (its message keeps the
    -- act's voice); the Rust gate resolves existence before either door is reached.
    v_plan := principal_erasure_survey_plan(p_subject);
    v_targets := v_plan->'targets';
    -- The plan's hash array comes back out IN ORDER (ORDINALITY makes the order load-bearing,
    -- not incidental): it is the redacted set the event carries, byte-identical to the
    -- pre-survey act's v_hashes assembly.
    v_hashes := (SELECT coalesce(array_agg(h ORDER BY ord), '{}')
                   FROM jsonb_array_elements_text(v_plan->'redacted_hashes')
                        WITH ORDINALITY AS t(h, ord));

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

    -- ── The ONE completion event (D1). NULL-anchored (admin); emitter = the OPERATOR;
    -- references carry the subject (the precedent) and the request reference (rel
    -- `request`); the correlation id is the request reference — the pairing of this event
    -- with its per-row strikes is a FACT, not a convention (D1). propagated_to_clients is
    -- false: client propagation is OUT OF ENFORCEMENT SCOPE (ruled 2026-09-10, decision
    -- 01a08dc2) — the value never reads as "not yet".
    v_ev := _event_append(
        'principal_erased', p_emitter, NULL, NULL,
        jsonb_build_object(
            'subject_table',        'kb_profiles',
            'subject_id',           p_subject,
            'actor',                p_operator,
            'redacted_hashes',      to_jsonb(v_hashes),
            'targets',              v_targets,
            'propagated_to_clients', false),
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_profiles','id', p_subject)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    -- ── The tombstone machinery, the ONE definition ─────────────────────────────────────
    PERFORM _erasure_apply_redaction(p_subject, v_hashes, v_ev);

    RETURN jsonb_build_object(
        'event_id',        v_ev,
        'redacted_hashes', to_jsonb(v_hashes),
        'targets',         v_targets,
        'already_erased',  (v_plan->>'already_erased')::boolean);
END;
$$;

COMMENT ON FUNCTION principal_erasure_execute(uuid, uuid, uuid, uuid) IS
'the erasure act (spec 2026-08-31, "The act, end to end" §3; Beat 2; outcome reads
governed-scoped 20260911000000; guest rows NAMED by 20260911000010 — home-pure scope per
the 2026-09-11 scope-of-engagement ruling; SHARES THE COMPUTATION with
principal_erasure_survey_plan since 20260913000010 — the plan is computed ONCE per act and
the strike loop consumes its rows by id, so the act and the survey cannot drift): scope
(every resource homed in a governed personal context), per-row governed-home blob strikes of
the SUBJECT''S OWN rows through blob_delete(''blob_erased'', …) — the wrapper''s verdict
authoritative at strike time — every LIVE guest-committed row homed in a governed context
named in the targets independent_obligation-shaped (blob id + content hash — never struck,
never silent; both outcomes name the retention with no release path, the act''s context
retirement closing the delete gate''s read for every caller, guest included), the ONE
NULL-anchored principal_erased event with the request reference on kb_events."references"
+ correlation, then _erasure_apply_redaction — all one transaction. The per-target outcome
reads scope to governed homes with the redaction''s own predicate, so the record never
reports "erased" for a row the act deliberately leaves standing, and names every
governed-home blob row it does not strike. Does NOT decide legality (is_system_admin is
the Rust caller''s gate, resolved before any mutation); the unauthorized refusal face is
principal_erasure_refuse.';

SELECT declare_migration(
    20260913000010,
    'additive',
    'The erasure survey door (task 01a09628 item 2, ruled 2026-09-12 with Pete): a read-only preview beside the execute door that shares the act''s ONE computation — a preview that can disagree with the act is worse than no preview (the reblock precedent, 8656d533). The act''s scope machinery — existence raise, governed contexts, home-pure resources, text hashes, the subject''s-own blob pass, the guest naming pass, and every per-target outcome arm reading PRE-redaction state — moves whole out of principal_erasure_execute into principal_erasure_survey_plan, which the act then CONSUMES: computed ONCE per act, strikes by id, nothing re-enumerates, so the act and the survey cannot drift by construction. The subject''s-own live governed-home blob rows come out of the plan as structured would-strike entries (id, hash, pathname, released_would_be — the survey predicts each strike''s verdict under the act''s own sequential refcount: the blob_delete live-row predicate minus the same-hash rows earlier in the strike set, already emptied when the act reaches this row — a prediction honest about the moment it ran, with the wrapper''s strike-time verdict staying authoritative); principal_erasure_survey renders them through blob_strike_outcome_text so its targets array is exactly the record the act would write. The strike-outcome prose template is extracted to that ONE definition, byte-unchanged (the fence parses it by exact prefix). The act''s observable record is byte-identical: same per-row blob_erased events in the same order, same ONE principal_erased payload, same redaction. A refused survey records NOTHING — a non-operator at the survey door gets the silent 404 the Rust gate renders and no principal_erasure_refused event: a survey attempt is not an erasure request. Verified: no per-target outcome arm reads kb_blobs or blob events, and blob_delete touches only kb_blobs and its own event, so computing every outcome arm PRE-strike yields identical values, and the plan''s already_erased reads the tombstone state the same way the act does. Additive: CREATE FUNCTION + CREATE OR REPLACE only, signatures unchanged.'
);
