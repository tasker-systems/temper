-- The erasure record names the subject's TEXT held under another governance's terms —
-- the 2026-09-06 ruling's "the team remainder is named, never silent", which the blob arm
-- has honored since 20260911000010 and the text arm has not: its scope computed only
-- governed-home hashes, so prose the subject authored into team homes was simply absent
-- from the record, unnamed (found by the adversarial review of PR #877). The plan gains
-- TWO independent_obligation-shaped targets — one per text grain, mirroring the
-- chunk-content and block-content redaction arms — naming count + hashes of
-- UNGOVERNED-home resources the subject owns or originates (kb_resource_homes'
-- owner/originator halves, the same attribution the blob arm's ungoverned remainder
-- names). The hashes are never admitted to v_hashes: the redaction's reads stay
-- governed-scoped, and the team's copy of the prose keeps its lawful life under the
-- terms-of-use line a contributor crosses by writing into another's personal context.
-- The act consumes the plan, so record and survey gain the naming together; a remainder
-- is a named outcome, never a mutation.
--
-- Additive: CREATE OR REPLACE only, signatures unchanged (the 20260804000020 class).

-- THE shared erasure computation (the act's own scope machinery, 20260913000010; the act
-- consumes the plan computed ONCE per act and the survey door serves the same plan). The
-- 2026-09-12 home-pure ruling widens the blob pass: every LIVE governed-home blob row is a
-- structured would-strike entry (id, hash, pathname, released_would_be), whoever committed
-- it; governed-home struck rows report already-erased; the subject's rows in ungoverned
-- homes keep the act's named-remainder prose. Every other arm is unchanged from
-- 20260913000010.
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

    -- The text remainder (2026-09-06: the team remainder is named, never silent): the
    -- subject's own text in UNGOVERNED homes, per grain, count + hashes (D2 vocabulary).
    -- Named, never struck, never admitted to v_hashes — the redaction's reads stay
    -- governed-scoped and the team's copy keeps its lawful life. Attribution is the same
    -- owner/originator halves the blob arm's ungoverned remainder names; a resource with
    -- ANY governed home is estate text and never a remainder.
    v_text_rem := (
        SELECT 'independent_obligation: home governed by a team or map; '
               || count(DISTINCT c.content_hash) || ' text hash(s) not struck: '
               || array_to_string(array_agg(DISTINCT c.content_hash), ', ')
          FROM kb_chunks c
          JOIN kb_resource_homes h ON h.resource_id = c.resource_id
         WHERE h.anchor_table = 'kb_contexts'
           AND h.anchor_id NOT IN (SELECT unnest(v_governed))
           AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg
                            WHERE hg.resource_id = c.resource_id
                              AND hg.anchor_table = 'kb_contexts'
                              AND hg.anchor_id = ANY(v_governed))
           AND (h.owner_profile_id = p_subject OR h.originator_profile_id = p_subject)
        HAVING count(DISTINCT c.content_hash) > 0);
    IF v_text_rem IS NOT NULL THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_chunk_content.content',
            'outcome', v_text_rem));
    END IF;
    v_text_rem := (
        SELECT 'independent_obligation: home governed by a team or map; '
               || count(DISTINCT bc.content_hash) || ' text hash(s) not struck: '
               || array_to_string(array_agg(DISTINCT bc.content_hash), ', ')
          FROM kb_block_content bc
          JOIN kb_block_revisions br ON br.id = bc.block_revision_id
          JOIN kb_content_blocks b ON b.id = br.block_id
          JOIN kb_resource_homes h ON h.resource_id = b.resource_id
         WHERE h.anchor_table = 'kb_contexts'
           AND h.anchor_id NOT IN (SELECT unnest(v_governed))
           AND NOT EXISTS (SELECT 1 FROM kb_resource_homes hg
                            WHERE hg.resource_id = b.resource_id
                              AND hg.anchor_table = 'kb_contexts'
                              AND hg.anchor_id = ANY(v_governed))
           AND (h.owner_profile_id = p_subject OR h.originator_profile_id = p_subject)
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
'THE shared erasure computation (the act''s scope machinery, moved whole out of
principal_erasure_execute by 20260913000010): governed contexts, home-pure resources, text
hashes, the blob pass, and every per-target outcome arm reading PRE-redaction state, in the
act''s exact order. The blob pass is HOME-PURE (ruled 2026-09-12): every LIVE governed-home
blob row is a structured would-strike entry (id, hash, pathname, released_would_be — the
survey predicts each strike''s verdict under the act''s own sequential refcount: the
blob_delete live-row predicate minus the same-hash rows earlier in the strike set, already
emptied when the act reaches this row), whoever committed it; governed-home struck rows
report already-erased; the subject''s rows in ungoverned homes keep the named remainder.
Read-only: it appends no event, mutates no row. principal_erasure_execute consumes this
plan (computed ONCE per act; strikes by id, in plan order; nothing re-enumerates) and the
survey door serves the same plan — the would-strike verdicts speak for the moment the plan
ran; the wrapper''s strike-time verdict is authoritative.';

-- The act's COMMENT only — its body is UNCHANGED (it consumes the plan, 20260913000010;
-- nothing re-enumerates): the home-pure ruling lands entirely in the plan above.
COMMENT ON FUNCTION principal_erasure_execute(uuid, uuid, uuid, uuid) IS
'the erasure act (spec 2026-08-31, "The act, end to end" §3; Beat 2; outcome reads
governed-scoped 20260911000000; home-pure scope per the 2026-09-11 scope-of-engagement
ruling; the blob arm HOME-PURE since 20260913000020 — every live governed-home blob row is
struck with the estate, whoever committed it, the 2026-09-12 ruling; SHARES THE COMPUTATION
with principal_erasure_survey_plan since 20260913000010 — the plan is computed ONCE per act
and the strike loop consumes its rows by id, so the act and the survey cannot drift): scope
(every resource homed in a governed personal context), per-row governed-home blob strikes
through blob_delete(''blob_erased'', …) — the wrapper''s verdict authoritative at strike
time — the ONE NULL-anchored principal_erased event with the request reference on
kb_events."references" + correlation, then _erasure_apply_redaction — all one transaction.
The record names no retention the act does not make: every governed-home blob row is either
struck or already-struck, and only the subject''s team/map-homed rows ride the named
remainder (disposition iii). The per-target outcome reads scope to governed homes with the
redaction''s own predicate, so the record never reports "erased" for a row the act
deliberately leaves standing. Custody, not admission (the corrected arm-13 posture,
20260913000020): retiring the governed contexts floors the read/author arms into the estate
— but the estate''s resource rows stay live (D3), kb_erased_content refuses no write
(20260911000000), and a re-commit of identical bytes into a retired home mints a fresh live
row that a later erasure of the same estate strikes again; the estate is guarded by the tombstone and the custody floor, never by
an impossibility of re-admission. The record also names the subject''s ungoverned-home TEXT
per grain (20260913000030, the 2026-09-06 team-remainder clause): named with count and
hashes, never struck, never in the redacted set. Does NOT decide legality (is_system_admin is the Rust
caller''s gate, resolved before any mutation); the unauthorized refusal face is
principal_erasure_refuse.';

SELECT declare_migration(
    20260913000030,
    'additive',
    'The erasure record names the subject''s TEXT held under another governance''s terms — the 2026-09-06 ruling''s "the team remainder is named, never silent", which the blob arm honored since 20260911000010 and the text arm did not: its scope computed only governed-home hashes, so prose the subject authored into team homes was absent from the record, unnamed (found by the adversarial review of PR #877). principal_erasure_survey_plan gains two independent_obligation-shaped targets — one per text grain, mirroring the chunk-content and block-content redaction arms — naming count + hashes of ungoverned-home resources the subject owns or originates (kb_resource_homes owner/originator, the blob arm''s own ungoverned-remainder attribution). The hashes are never admitted to the redacted set: the redaction''s reads stay governed-scoped and the team''s copy keeps its lawful life under the terms-of-use line. The act consumes the plan, so record and survey gain the naming together; a remainder is a named outcome, never a mutation. Additive: CREATE OR REPLACE only, signatures unchanged (the 20260804000020 class).'
);
