-- The hash-global erasure grain is retired (ruled 2026-09-10 with Pete — temper vault
-- decision 01a08dc2-684c-7f20-aeac-b1895f57831b, "erasure is offboarding — the authority
-- line is custody, never bytes"; task 01a08dc4-32bc). Three bodies change, all
-- CREATE OR REPLACE with signatures unchanged (the 20260804000020 additive class):
--
--   * _erasure_apply_redaction — the text arms (chunk prose, block bytes, embeddings,
--     search vectors, formation watermarks) scope to the subject's GOVERNED homes
--     (owner_table='kb_profiles' AND owner_id=subject), the same predicate the scope
--     computation and the artifact arm already spell. The 20260909000025 comment claimed
--     governed-scope admission kept the wipe bounded; it did not — the wipe re-expanded
--     hashes across every home, emptying a second principal's identical template prose and
--     vectors. The set's ADMISSION stays governed-scope (unchanged); the redaction now is
--     too. The replay pre-pass calls this same function, and contexts are replay INPUT
--     tables restored verbatim, so the predicate resolves identically in the replay
--     namespace — the byte-identity proof carries unchanged.
--   * block_mutate — the erased-content refusal check (20260909000030) is removed; the
--     five suppression checks and their semantics are restored byte-identical. A hash in
--     kb_erased_content no longer refuses any write anywhere: another principal's lawful
--     write of identical bytes is never an erasure violation (the register's re-keyed
--     negative face). The set remains a projection and the replay refill is untouched.
--   * principal_erasure_execute — the per-target outcome reads scope to governed homes
--     with the same predicate, so the record never reports "erased" for rows the act
--     deliberately leaves standing (never silently partial cuts both ways: never falsely
--     total either).
--
-- kb_erased_content itself is UNTOUCHED: it remains the ledger-derived projection the
-- replay diffs in full. What retires is every write-path consult that read it as an
-- instance-wide oracle.

-- ---------------------------------------------------------------------------
-- The ONE redaction definition — governed-home scoped (see this file's header).
-- Event-free and strike-free BY DESIGN, exactly as 20260909000025 left it: the replay
-- pre-pass runs this beside the walk; only `principal_erasure_execute` appends events
-- around it. The governed-home predicate below is the scope computation's own owner arm
-- (contexts_readable_by arm 1, 20260712000010:96-97) — spelled once per arm, never
-- aliased, so every reader sees the same shape the register's clauses name.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION _erasure_apply_redaction(p_subject uuid, p_hashes text[], p_event uuid)
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
              AND h.anchor_id IN (
                  SELECT g.id FROM kb_contexts g
                   WHERE g.owner_table = 'kb_profiles'
                     AND g.owner_id = p_subject));

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
                      AND h.anchor_id IN (
                          SELECT g.id FROM kb_contexts g
                           WHERE g.owner_table = 'kb_profiles'
                             AND g.owner_id = p_subject)));

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
               AND h.anchor_id IN (
                   SELECT g.id FROM kb_contexts g
                    WHERE g.owner_table = 'kb_profiles'
                      AND g.owner_id = p_subject));

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
                      AND h.anchor_id IN (
                          SELECT g.id FROM kb_contexts g
                           WHERE g.owner_table = 'kb_profiles'
                             AND g.owner_id = p_subject)))
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
               AND h.anchor_id IN (
                   SELECT c.id FROM kb_contexts c
                    WHERE c.owner_table = 'kb_profiles'
                      AND c.owner_id = p_subject))
        AND dac.content <> '{}'::jsonb;

    -- (12) The centroid sites MARKED for recompute (D5): the formation watermark nulled on
    --     every anchor whose live regions hold redacted GOVERNED members, plus the
    --     subject's own governed contexts (their telos aggregates their goals). See the
    --     header bullet.
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
                                      WHERE c.content_hash = ANY(p_hashes)
                                        AND c.resource_id IN (
                                            SELECT h.resource_id
                                              FROM kb_resource_homes h
                                             WHERE h.anchor_table = 'kb_contexts'
                                               AND h.anchor_id IN (
                                                   SELECT g.id FROM kb_contexts g
                                                    WHERE g.owner_table = 'kb_profiles'
                                                      AND g.owner_id = p_subject))));

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
                                          WHERE c.content_hash = ANY(p_hashes)
                                            AND c.resource_id IN (
                                                SELECT h.resource_id
                                                  FROM kb_resource_homes h
                                                 WHERE h.anchor_table = 'kb_contexts'
                                                   AND h.anchor_id IN (
                                                       SELECT g.id FROM kb_contexts g
                                                        WHERE g.owner_table = 'kb_profiles'
                                                          AND g.owner_id = p_subject))))
            -- The personal-context predicate again — the schema's own owner arm
            -- (contexts_readable_by arm 1, 20260712000010:96-97): the subject's own contexts
            -- only; team arms are a different read and a different governance.
            OR (c.owner_table = 'kb_profiles' AND c.owner_id = p_subject) );

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
    UPDATE kb_contexts g
       SET is_active = false
      WHERE g.is_active
        AND g.owner_table = 'kb_profiles'
        AND g.owner_id = p_subject;
END;
$$;

COMMENT ON FUNCTION _erasure_apply_redaction(uuid, text[], uuid) IS
'the erasure act''s ONE redaction definition (20260909000025, re-scoped 20260911000000 —
the text analog of the blobs'' D5.2 shape): content emptied BY HASH with hashes kept —
GOVERNED HOMES ONLY since the 2026-09-10 offboarding ruling (decision 01a08dc2: the hash
is the record key, never the reach; a same-hash chunk or block in a home the subject does
not govern keeps its prose, vector and search vector) — embeddings+provenance nulled
together, search vectors emptied for governed resources only, the profile tombstoned to
occurred_at, the sync_personal_team denormalization scrubbed to the sentinel derivation,
the two no-FK Slack identifier stores deleted, the auth-link identifiers unclaimed (email
NULL, auth_provider_user_id sentineled to ''erased-'' || the row''s own id — after the
Slack deletes, whose principal join runs through it), the subject''s entity names
sentineled per-row under the (profile_id, name) UNIQUE grain, the subject''s own
data-artifact content emptied on governed resources (kind owned by the subject''s profile;
hashes and ids kept), kb_erased_content first-admit refilled (the set is the RECORD of the
act''s redacted hashes; no write-path consult reads it as an instance-wide oracle any
more), formation watermarks nulled for governed members only (the centroid recompute MARK
— D5). Event-free and idempotent: the erasure act calls it inside its transaction and the
replay redaction pre-pass calls the SAME function with the payload''s hashes — a second
body would be two definitions of erasure that drift; the replay namespace resolves the
governed-home predicate identically because contexts are replay INPUT tables. NEVER grows
a legality check: the Rust gate decides, SQL commits (the principal_standing_apply shape).';

-- ---------------------------------------------------------------------------
-- The act. One transaction: scope → per-row governed-home strikes → the ONE
-- principal_erased event → the redaction. Returns the outcome jsonb the service layer
-- maps onto typed structs. LEGALITY IS THE CALLER'S (is_system_admin) — never checked here.
-- 20260911000000: the per-target outcome reads scope to governed homes with the same
-- predicate the redaction now spells, so the record reports "erased" only for rows the
-- act actually redacts.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION principal_erasure_execute(
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
        'already_erased',  (v_prof.tombstoned_at IS NOT NULL));
END;
$$;

COMMENT ON FUNCTION principal_erasure_execute(uuid, uuid, uuid, uuid) IS
'the erasure act (spec 2026-08-31, "The act, end to end" §3; Beat 2; outcome reads
governed-scoped 20260911000000): scope (the two indexed halves filtered to governed
personal contexts), per-row governed-home blob strikes through blob_delete(''blob_erased'',
…), the ONE NULL-anchored principal_erased event with the request reference on
kb_events."references" + correlation, then _erasure_apply_redaction — all one transaction.
The per-target outcome reads scope to governed homes with the redaction''s own predicate,
so the record never reports "erased" for a row the act deliberately leaves standing. Does
NOT decide legality (is_system_admin is the Rust caller''s gate, resolved before any
mutation); the unauthorized refusal face is principal_erasure_refuse.';

-- ---------------------------------------------------------------------------
-- block_mutate — the erased-content refusal is RETIRED (20260909000030 removed).
-- The refusal keyed on hash membership in kb_erased_content: any revise, in any home,
-- by any actor, refused when a payload hash appeared in the set — the instance-wide
-- oracle the ruling retires. Its stated target (a stale client re-admitting erased
-- prose) is custody-closed: the wiped estate sits in the tombstoned subject's contexts,
-- and no live principal holds standing to reconcile into them. The five no-op
-- suppression checks from 20260726000030 — including check 3's `AND NOT v_unembedded`,
-- never swallow an embed repair — are restored byte-identical, and the whole-body
-- semantics from 20260906000080 / 20260908000010 stand untouched. The embed path keeps
-- its own row-anchored erased exclusion (embed.rs, re-keyed in the same change): a
-- wiped chunk is not embed work; a same-hash chunk with live prose elsewhere embeds.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION block_mutate(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                        p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL,
                                        p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_block uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_anchor_tbl text; v_anchor uuid;
        v_incoming_hash text; v_existing_hash text;
        v_incoming_bytes text; v_existing_bytes text;
        v_live_chunks int; v_unembedded boolean; v_new_model boolean;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION 'block_mutate: block % not found', v_block;
    END IF;
    -- An empty chunk set would supersede the block's current chunks and insert none, silently dropping
    -- the member from its region centroid and diverging body_hash from create-path semantics (which has
    -- no empty-body block). Reject before appending an event — a revise must carry content.
    IF p_payload->'chunks' IS NULL OR jsonb_array_length(p_payload->'chunks') = 0 THEN
        RAISE EXCEPTION 'block_mutate: empty chunk set for block % (a revise with no content would drop the block)', v_block;
    END IF;

    -- ── No-op suppression (20260726000030; the erased-content refusal that once sat here
    -- was retired by 20260911000000) ── check 5: provenance is event-anchored, so a write
    -- carrying sources is never a no-op, whatever its bytes did.
    IF p_payload->'incorporated' IS NULL
       OR jsonb_typeof(p_payload->'incorporated') <> 'array'
       OR jsonb_array_length(p_payload->'incorporated') = 0 THEN

        -- Check 1. ARRAY order, matching the projector that writes the stored hash (trap 2).
        SELECT encode(sha256(convert_to(
                   coalesce(string_agg(c.elem->>'content_hash', '' ORDER BY c.ord), ''), 'UTF8')), 'hex')
          INTO v_incoming_hash
          FROM jsonb_array_elements(p_payload->'chunks') WITH ORDINALITY AS c(elem, ord);

        -- By pointer, not `ORDER BY created DESC LIMIT 1` — two revisions can share an `occurred_at`.
        SELECT rv.block_body_hash INTO v_existing_hash
          FROM kb_content_blocks b
          JOIN kb_block_revisions rv ON rv.id = b.current_revision_id
         WHERE b.id = v_block;

        -- Check 2 (trap 1). Only when the caller supplies bytes: if it does not, the projector would
        -- write none, so suppressing preserves what is stored rather than erasing it.
        v_incoming_bytes := p_content->'__blocks'->(v_block::text)->>'content_hash';
        IF v_incoming_bytes IS NOT NULL THEN
            SELECT bc.content_hash INTO v_existing_bytes
              FROM kb_content_blocks b
              JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
             WHERE b.id = v_block;
        END IF;

        -- Checks 3 and 4. Never swallow an embed repair or a model rotation. A caller bringing no
        -- vectors is not blocked — the projector would only write NULLs over good ones.
        SELECT count(*), coalesce(bool_or(ch.embedding IS NULL), false)
          INTO v_live_chunks, v_unembedded
          FROM kb_chunks ch
         WHERE ch.block_id = v_block AND ch.is_current;

        SELECT coalesce(bool_or(
                   (p_content->(c->>'chunk_id')->>'embedded_with') IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM kb_chunks ch
                        WHERE ch.block_id = v_block AND ch.is_current
                          AND ch.embedded_with = (p_content->(c->>'chunk_id')->>'embedded_with'))
               ), false)
          INTO v_new_model
          FROM jsonb_array_elements(p_payload->'chunks') c;

        IF v_existing_hash IS NOT NULL
           AND v_existing_hash = v_incoming_hash
           AND (v_incoming_bytes IS NULL OR v_existing_bytes = v_incoming_bytes)
           AND v_live_chunks > 0
           AND NOT v_unembedded
           AND NOT v_new_model THEN
            RETURN v_block;  -- no-op: the projector would write nothing new
        END IF;
    END IF;

    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'block_mutate: resource % has no home to anchor the event', v_resource;
    END IF;
    v_ev := _event_append('block_mutated', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_block_mutated(v_ev, p_payload, p_content);
END;
$$;

COMMENT ON FUNCTION block_mutate(jsonb, jsonb, uuid, jsonb, uuid, uuid) IS
    'Revise a block in place. Suppresses a no-op revise at the write path — no event appended, '
    'so the tier-1 staleness cursor does not move and audited findings are not re-offered. '
    'Suppresses only when the projector would write nothing new: same merkle, same stored bytes, '
    'no chunk awaiting a vector, no new embedding model, nothing incorporated. The erased-content '
    'refusal that sat before the suppression (20260909000030) is RETIRED (20260911000000, the '
    'offboarding ruling): no content-admitting path consults kb_erased_content as an '
    'instance-wide oracle any more (the embed drain keeps its row-anchored exclusion) — another '
    'principal''s lawful write of identical bytes is never an erasure violation, and the wiped '
    'estate is custody-closed. See 20260726000030''s header for the two suppression traps.';

SELECT declare_migration(
    20260911000000,
    'additive',
    'The hash-global erasure grain is retired (ruled 2026-09-10 with Pete — decision 01a08dc2-684c-7f20-aeac-b1895f57831b, erasure is offboarding, the authority line is custody never bytes; task 01a08dc4). _erasure_apply_redaction''s text arms (chunk prose, block bytes, embeddings+provenance, search vectors, formation watermarks) scope to the subject''s governed homes with the scope computation''s own owner arm — 20260909000025''s claim that governed-scope set admission kept the wipe bounded was backwards: the wipe re-expanded hashes across every home, emptying a second principal''s identical template prose and vectors; the set''s admission stays governed-scope and the redaction now is too, and the replay pre-pass resolves the same predicate identically because contexts are replay INPUT tables. The redaction also RETIRES the governed contexts (is_active=false) — closing custody over the estate, so grants into it die with the activation floor and no live principal can re-admit content into the wiped homes; un-evented per the profile-tombstone''s own INPUT-table precedent, is_active being non-identity-bearing. block_mutate loses the erased-content refusal (20260909000030), restoring the five suppression checks byte-identical — no content-admitting path consults kb_erased_content as an instance-wide oracle any more; the set remains the ledger-derived projection the replay diffs in full. principal_erasure_execute''s scope is HOME-PURE (ruled 2026-09-11 with Pete, the scope-of-engagement ruling: content created under grant inside a private context dies with that estate) — every resource homed in a governed context, the 20260909000025 actor halves dissolved; per-target outcome reads scope to governed homes with the same predicate, and the retirement rides the record as its own per-target outcome. propagated_to_clients stays false permanently — client propagation is out of enforcement scope, never "not yet". Additive: CREATE OR REPLACE only, signatures unchanged.'
);
