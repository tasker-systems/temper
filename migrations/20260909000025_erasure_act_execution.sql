-- The erasure act's execution (spec 2026-08-31, temper-artifacts/specs/
-- 2026-08-31-erasure-act-design.md; task 01a0577c-f115, Beat 2). The act runs in ONE
-- transaction in the `principal_standing_apply` shape: SQL commits, the Rust gate decides
-- (20260720000030:15-19 — this file's functions MUST NEVER grow a legality check; the
-- operator gate is `access_service::is_system_admin`, resolved before any mutation).
--
-- THE CONSTRAINTS A LATER EDIT MUST NOT BREAK:
--
--   * THE ONE REDACTION DEFINITION. `_erasure_apply_redaction(p_subject, p_hashes, p_event)`
--     is the text analog of the blobs' D5.2 shape: content emptied, hashes and
--     embedding-provenance columns kept coherent, search vectors emptied, the profile
--     tombstoned, the erased-content set refilled. It is the ONLY home of that shape — the
--     act calls it and the replay redaction pre-pass calls the SAME function with the
--     payload's `redacted_hashes` + the admitting event's id; re-deriving any of it is a
--     second definition that drifts. It is hash-keyed (D2: the hash is the join key every
--     reader shares) and idempotent.
--   * THE REPLAY REDACTION MUST MIRROR ALL OF THIS (all of it lives in
--     _erasure_apply_redaction; the replay arm calls the function — never re-derives it):
--     chunk/block content emptied BY HASH with hashes retained (D3); kb_chunks.embedding AND
--     embedded_with nulled together (the 20260713000040 coherence rule); search vectors
--     emptied wholesale for hash-affected resources; tombstoned_at := the admitting event's
--     occurred_at (NEVER now() — projected timestamps replay-stable by construction);
--     handle/display_name := 'erased-' || subject_id and the sync_personal_team row scrubbed
--     to 'personal-' || that sentinel (the trigger's own derivation,
--     20260624000002:95-98, applied to the sentinel identity); the two no-FK Slack
--     identifier stores deleted via the auth-link principal join; the auth-link identifiers
--     unclaimed AFTER those deletes (email NULLed, auth_provider_user_id sentineled to
--     'erased-' || the row's own id — the Slack deletes resolve their principals THROUGH
--     auth_provider_user_id, so the order is load-bearing; the per-row id is what the
--     (auth_provider, auth_provider_user_id) UNIQUE grain demands of a sentinel); the
--     subject's entity names sentineled to 'erased-' || each entity's own id (the
--     (profile_id, name) UNIQUE grain — one shared sentinel cannot hold); the
--     subject's own data-artifact content emptied on governed resources (kind_owner IS the
--     subject's profile — another principal's kind namespace is the disposition-iii
--     remainder, never struck); kb_erased_content
--     refilled with ON CONFLICT DO NOTHING so FIRST admit keeps attribution — and that
--     attribution assumes live admission order equals ledger id order: concurrent erasures
--     sharing a hash can interleave the set-insert. The divergence fails LOUD via the replay
--     dump (kb_erased_content is in PROJECTION_DUMPS, replay.rs), and it is the register's
--     declared-open concurrency axis. Formation
--     watermarks (shape_materialized_event_id) nulled on the region-member anchors and the
--     subject's governed contexts.
--   * CENTROIDS ARE MARKED, NOT EMPTIED (D5): kb_cogmap_regions.centroid is NOT NULL and
--     Rust-derived (MaterializeCogmapShape); telos_centroid snapshots ride region events.
--     "Marked for recompute" is the formation watermark NULLED: admin events advance no
--     formation count (STRUCTURAL ∪ CONTENT only — replay.rs), so without the null NO
--     materialize would ever re-fire; with it, materialize_delta counts from the beginning
--     (watermark None) and the next cron pass recomputes from surviving members. The stale
--     window is the spec's accepted trade-off.
--   * THE STRIKE ARM IS GOVERNED-HOMES-ONLY (disposition iii, ruled 2026-09-08): blob rows
--     are struck PER ROW through blob_delete('blob_erased', …) — never a bulk path — only
--     where the home is a personal context of the subject. Team- or cogmap-homed rows are
--     NEVER struck: their hashes ride the payload's per-target remainder,
--     independent_obligation-shaped, and never enter redacted_hashes (admitting them to the
--     erased-content set would refuse writes in homes the subject does not govern).
--   * THE PAYLOAD NEVER RE-IDENTIFIES AND NEVER CARRIES A TRAIL JOIN-KEY SHAPE (D2): the
--     redacted set is content hashes alone; per-target outcomes are manifest-identity prose
--     (`table`, `table.column`) — no resource_id/block_id/owner/edge_id keys, no blob ids
--     (the per-row strikes carry those on their own identity-only events). The request
--     reference rides `kb_events."references"` (rel `request`) and the act's correlation id,
--     never the payload.
--
-- Additive: new functions only — no existing column, constraint or function is altered, and
-- an old binary reads every pre-existing row unchanged.

-- ---------------------------------------------------------------------------
-- The ONE redaction definition (the text analog of D5.2 — see header).
-- Event-free and strike-free BY DESIGN: the replay pre-pass runs this beside the walk;
-- only `principal_erasure_execute` appends events around it.
-- ---------------------------------------------------------------------------
CREATE FUNCTION _erasure_apply_redaction(p_subject uuid, p_hashes text[], p_event uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
    v_sentinel text := 'erased-' || p_subject::text;
    v_team     uuid;
BEGIN
    -- (1) The personal-team denormalization (D5 blind-spot row 1 — the only denormalization
    --     carrier Derivation C found). Captured by the trigger's OWN inverse lookup BEFORE
    --     the handle moves (20260624000002:98); scrubbed to the same derivation applied to
    --     the sentinel identity. The trigger is AFTER INSERT only, so this never re-fires.
    SELECT t.id INTO v_team
      FROM kb_teams t
      JOIN kb_profiles pr ON pr.id = p_subject
     WHERE t.slug = 'personal-' || pr.handle;

    UPDATE kb_teams
       SET slug = 'personal-' || v_sentinel,
           name = v_sentinel || ' (personal)'
     WHERE id = v_team;

    -- (2) Chunk prose emptied BY HASH, hash kept (D3: emptied, not deleted — the CAS
    --     retention rule never fires). Hash-keyed, so a hash shared across homes redacts
    --     everywhere: the erased-content set refuses by hash globally, and the act's set is
    --     governed-scope only, so this never reaches a home the subject does not govern.
    UPDATE kb_chunk_content cc
       SET content = ''
      FROM kb_chunks c
     WHERE cc.chunk_id = c.id
       AND c.content_hash = ANY(p_hashes)
       AND cc.content <> '';

    -- (3) Verbatim block bytes, same shape.
    UPDATE kb_block_content
       SET content = ''
     WHERE content_hash = ANY(p_hashes)
       AND content <> '';

    -- (4) Embeddings + provenance nulled TOGETHER — `embedding IS NULL` and
    --     `embedded_with IS NULL` can never disagree (20260713000040:84-87).
    UPDATE kb_chunks
       SET embedding = NULL,
           embedded_with = NULL
     WHERE content_hash = ANY(p_hashes)
       AND embedding IS NOT NULL;

    -- (5) Search vectors emptied wholesale for every hash-affected resource (the
    --     _rebuild_resource_search_vector site, 20260711000060:74-83): the vector folds
    --     title+body+meta, so a partial redaction would leave redacted terms searchable.
    UPDATE kb_resource_search_index si
       SET search_vector = ''
      WHERE si.resource_id IN (
            SELECT DISTINCT c.resource_id FROM kb_chunks c
             WHERE c.content_hash = ANY(p_hashes))
       AND si.search_vector <> '';

    -- (6) The erased-content set (D4): first admit keeps attribution — ON CONFLICT DO
    --     NOTHING is what makes a ledger-order rebuild byte-identical.
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
    --      kb_erased_content: that set is the prose/binary re-admission refusal, and no
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
              AND h.anchor_id IN (
                  SELECT c.id FROM kb_contexts c
                   WHERE c.owner_table = 'kb_profiles'
                     AND c.owner_id = p_subject))
       AND dac.content <> '{}'::jsonb;

    -- (12) The centroid sites MARKED for recompute (D5): the formation watermark nulled on
    --     every anchor whose live regions hold redacted members, plus the subject's own
    --     governed contexts (their telos aggregates their goals). See the header bullet.
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
              AND mem.member_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                     WHERE c.content_hash = ANY(p_hashes)));

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
                  AND mem.member_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                         WHERE c.content_hash = ANY(p_hashes)))
           -- The personal-context predicate again — the schema's own owner arm
           -- (contexts_readable_by arm 1, 20260712000010:96-97): the subject's own contexts
           -- only; team arms are a different read and a different governance.
           OR (c.owner_table = 'kb_profiles' AND c.owner_id = p_subject) );
END;
$$;

COMMENT ON FUNCTION _erasure_apply_redaction(uuid, text[], uuid) IS
'the erasure act''s ONE redaction definition (20260909000025 — the text analog of the blobs''
D5.2 shape): content emptied BY HASH with hashes kept, embeddings+provenance nulled
together, search vectors emptied, the profile tombstoned to occurred_at, the
sync_personal_team denormalization scrubbed to the sentinel derivation, the two no-FK
Slack identifier stores deleted, the auth-link identifiers unclaimed (email NULL,
auth_provider_user_id sentineled to ''erased-'' || the row''s own id — after the Slack
deletes, whose principal join runs through it), the subject''s entity names sentineled
per-row under the (profile_id, name) UNIQUE grain, the subject''s own data-artifact
content emptied on governed resources (kind owned by the subject''s profile; hashes and
ids kept), kb_erased_content first-admit refilled, formation
watermarks nulled (the centroid recompute MARK — D5). Event-free and idempotent: the
erasure act calls it inside its transaction and the replay redaction pre-pass (Beat 3)
must call the SAME function with the payload''s hashes — a second body would be two
definitions of erasure that drift. NEVER grows a legality check: the Rust gate decides,
SQL commits (the principal_standing_apply shape).';

-- ---------------------------------------------------------------------------
-- The act. One transaction: scope → per-row governed-home strikes → the ONE
-- principal_erased event → the redaction. Returns the outcome jsonb the service layer
-- maps onto typed structs. LEGALITY IS THE CALLER'S (is_system_admin) — never checked here.
-- ---------------------------------------------------------------------------
CREATE FUNCTION principal_erasure_execute(
    p_subject     uuid,
    p_operator    uuid,
    p_emitter     uuid,
    p_request_ref uuid
) RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    v_exists     uuid;
    v_governed   uuid[] := '{}';
    v_resources  uuid[] := '{}';
    v_hashes     text[] := '{}';
    v_blob_hash  text[] := '{}';
    v_targets    jsonb  := '[]'::jsonb;
    v_row        record;
    v_bid uuid; v_rel boolean; v_path text;
    v_ev    uuid;
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
    -- are NOT governed — disposition iii.
    SELECT coalesce(array_agg(id), '{}') INTO v_governed
      FROM kb_contexts
     WHERE owner_table = 'kb_profiles'
       AND owner_id = p_subject;

    -- ── Scope, the two indexed halves (spec §2), FILTERED to governed homes ──────────────
    -- own half: kb_resource_homes.owner_profile_id / originator_profile_id
    -- emitted half: content blocks whose genesis event the subject's entities emitted
    --               (idx_kb_events_emitter). Third-party mentions are not computed here.
    WITH scope AS (
        SELECT h.resource_id
          FROM kb_resource_homes h
         WHERE h.anchor_table = 'kb_contexts'
           AND h.anchor_id = ANY(v_governed)
           AND (h.owner_profile_id = p_subject OR h.originator_profile_id = p_subject)
        UNION
        SELECT b.resource_id
          FROM kb_content_blocks b
          JOIN kb_events e ON e.id = b.genesis_event_id
          JOIN kb_entities en ON en.id = e.emitter_entity_id
         WHERE en.profile_id = p_subject
           AND EXISTS (SELECT 1 FROM kb_resource_homes h
                        WHERE h.resource_id = b.resource_id
                          AND h.anchor_table = 'kb_contexts'
                          AND h.anchor_id = ANY(v_governed))
    )
    SELECT coalesce(array_agg(DISTINCT resource_id), '{}') INTO v_resources FROM scope;

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

    -- ── The blob pre-pass, IN-ACT (ruled: through the wrapper, PER ROW) ──────────────────
    -- Scope = the subject's blob rows (own half: owner/originator; emitted half: the commit
    -- event's emitter). Governed-home LIVE rows are struck; governed-home struck rows report
    -- already-erased; EVERY other home is the named remainder (disposition iii).
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
    LOOP
        IF v_row.governed_home AND v_row.content_type IS NOT NULL THEN
            SELECT blob_id, released, pathname
              INTO v_bid, v_rel, v_path
              FROM blob_delete('blob_erased',
                               jsonb_build_object('blob_id', v_row.id),
                               p_emitter,
                               p_correlation => p_request_ref);
            v_blob_hash := v_blob_hash || ARRAY[v_row.content_hash];
            v_targets := v_targets || jsonb_build_array(jsonb_build_object(
                'target',  'kb_blobs',
                'outcome', 'erased; released=' || v_rel::text || '; pathname=' || v_path));
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
    -- _erasure_apply_redaction alone; these reads only report what it will find). ─────────
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

    IF EXISTS (SELECT 1 FROM kb_chunk_content cc JOIN kb_chunks c ON c.id = cc.chunk_id
                WHERE c.content_hash = ANY(v_hashes)) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunk_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunk_content cc
                                               JOIN kb_chunks c ON c.id = cc.chunk_id
                                              WHERE c.content_hash = ANY(v_hashes)
                                                AND cc.content <> '')
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_block_content WHERE content_hash = ANY(v_hashes)) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_block_content.content',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_block_content
                                              WHERE content_hash = ANY(v_hashes)
                                                AND content <> '')
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_chunks WHERE content_hash = ANY(v_hashes)) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_chunks.embedding',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL)
                                THEN 'erased' ELSE 'already-erased' END),
            jsonb_build_object('target','kb_chunks.embedded_with',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_chunks
                                              WHERE content_hash = ANY(v_hashes)
                                                AND embedding IS NOT NULL)
                                THEN 'erased' ELSE 'already-erased' END));
    END IF;
    IF EXISTS (SELECT 1 FROM kb_resource_search_index si
                WHERE si.resource_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                          WHERE c.content_hash = ANY(v_hashes))) THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_resource_search_index.search_vector',
                'outcome', CASE WHEN EXISTS (SELECT 1 FROM kb_resource_search_index si
                                              WHERE si.resource_id IN (
                                                    SELECT DISTINCT c.resource_id FROM kb_chunks c
                                                     WHERE c.content_hash = ANY(v_hashes))
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
                                     WHERE c.content_hash = ANY(v_hashes)));
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_cogmaps.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;
    SELECT count(*) INTO v_n FROM kb_contexts c
     WHERE c.shape_materialized_event_id IS NOT NULL
       AND ( c.id IN (
               SELECT r.home_anchor_id FROM kb_cogmap_regions r
                 JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
                WHERE r.home_anchor_table = 'kb_contexts' AND NOT r.is_folded
                  AND mem.member_table = 'kb_resources'
                  AND mem.member_id IN (SELECT DISTINCT c.resource_id FROM kb_chunks c
                                         WHERE c.content_hash = ANY(v_hashes)))
          -- The personal-context predicate again — the schema's own owner arm
          -- (contexts_readable_by arm 1, 20260712000010:96-97); team-owned and team-shared
          -- contexts (arms 2-3) are NOT governed — disposition iii.
          OR (c.owner_table = 'kb_profiles' AND c.owner_id = p_subject) );
    IF v_n > 0 THEN
        v_targets := v_targets || jsonb_build_array(
            jsonb_build_object('target','kb_contexts.shape_materialized_event_id',
                'outcome','recompute-marked'));
    END IF;

    -- ── The ONE completion event (D1). NULL-anchored (admin); emitter = the OPERATOR;
    -- references carry the subject (the precedent) and the request reference (rel
    -- `request`); the correlation id is the request reference — the pairing of this event
    -- with its per-row strikes is a FACT, not a convention (D1). propagated_to_clients is
    -- false: "gone from the server" only, until the propagation protocol exists (D3).
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
        'already_erased',  (v_prof.tombstoned_at IS NOT NULL));
END;
$$;

COMMENT ON FUNCTION principal_erasure_execute(uuid, uuid, uuid, uuid) IS
'the erasure act (spec 2026-08-31, "The act, end to end" §3; Beat 2): scope (the two
indexed halves filtered to governed personal contexts), per-row governed-home blob strikes
through blob_delete(''blob_erased'', …), the ONE NULL-anchored principal_erased event with
the request reference on kb_events."references" + correlation, then
_erasure_apply_redaction — all one transaction. Does NOT decide legality
(is_system_admin is the Rust caller''s gate, resolved before any mutation); the
unauthorized refusal face is principal_erasure_refuse.';

-- ---------------------------------------------------------------------------
-- The refusal face (D6): one event, reason code, NOTHING else mutated. Same
-- NULL-anchored shape; subject spelled as subject_table/subject_id; the request
-- reference on references + correlation exactly as the completion's.
-- ---------------------------------------------------------------------------
CREATE FUNCTION principal_erasure_refuse(
    p_subject      uuid,
    p_attempted_by uuid,
    p_emitter      uuid,
    p_request_ref  uuid,
    p_reason       text,
    p_detail       text DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    IF p_reason NOT IN ('unauthorized','unhonourable_scope','independent_obligation') THEN
        RAISE EXCEPTION 'principal_erasure_refuse: % is not an erasure refusal reason',
                        p_reason;
    END IF;

    v_ev := _event_append(
        'principal_erasure_refused', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'subject_table', 'kb_profiles',
            'subject_id',    p_subject,
            'actor',         p_attempted_by,
            'reason',        p_reason,
            'detail',        p_detail)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_profiles','id', p_subject)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    RETURN v_ev;
END;
$$;

COMMENT ON FUNCTION principal_erasure_refuse(uuid, uuid, uuid, uuid, text, text) IS
'the erasure act''s negative face (spec D6; Beat 2): records principal_erasure_refused —
one event with the closed reason vocabulary (unauthorized | unhonourable_scope |
independent_obligation) — and mutates NOTHING else. Accepted-in-part is NOT this event:
a completion with a named remainder is principal_erasure_execute''s payload data.';

SELECT declare_migration(
    20260909000025,
    'additive',
    'The erasure act''s execution (spec 2026-08-31, Beat 2 of task 01a0577c): principal_erasure_execute (scope via the two indexed halves filtered to governed personal contexts — owner_table=''kb_profiles'' AND owner_id=subject, the read model''s own owner arm; per-row governed-home blob strikes through the substrate''s blob_delete(''blob_erased'', …) wrapper, team/cogmap homes named as the independent_obligation remainder; the ONE NULL-anchored principal_erased event whose references carry the subject + the opaque request reference and whose correlation id pairs it with its strikes; then _erasure_apply_redaction) and principal_erasure_refuse (D6''s negative face — reason code, nothing else mutated). _erasure_apply_redaction is the ONE text-redaction definition (content emptied by hash, hashes kept, embeddings+provenance nulled together, search vectors emptied, profile tombstoned to occurred_at, personal-team denormalization scrubbed to the sentinel derivation, no-FK Slack identifiers deleted, kb_erased_content first-admit refilled, formation watermarks nulled as the centroid recompute MARK, the manifest-declared identifier columns reached — auth-link email NULLed and auth_provider_user_id sentineled to ''erased-'' || the row''s own id (after the Slack deletes whose principal join runs through it), the subject''s entity names sentineled per-row under the (profile_id, name) UNIQUE grain, the subject''s own data-artifact content emptied on governed personal-context resources with hashes and ids kept (another principal''s kind namespace is the disposition-iii remainder, never struck)) — the replay pre-pass (Beat 3) must call the same function, never a second body. Legality is the Rust caller''s (is_system_admin); SQL commits, it never decides. Additive: new functions only.'
);
