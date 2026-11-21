-- Edge erasure: one ledger-derivation loop for resource and edge trails, and an edge's trail.
--
-- Person erasure erases the edges homed in the erased person's estate that no resource erasure
-- reached. Rewriting such an edge's ledger needs what resource erasure already has: the events
-- that are the edge's own, and each one's payload and metadata at the class sentinels.
--
--   1. _erasure_trail_payload_redaction(uuid[], jsonb) is the per-event loop of
--      _resource_erasure_payload_redaction (20261010100000), over a caller-supplied trail and
--      remote-source numbering. _resource_erasure_payload_redaction keeps its signature, its husk
--      check and its remote-source numbering, and returns the shared loop over
--      _resource_erasure_trail_scope, so its output is unchanged. The loop leaves every event
--      correlated with a resource_erased or edge_erased other than itself: that is an act's record.
--   2. _edge_erasure_trail_scope(uuid) is an edge's own events: those naming it by edge_id, the
--      property events it owns, and its re-assertions recorded under a minted edge_id
--      (_edge_phantom_reassertions, 20261117000030), domain events only, never a received webhook.
--      _edge_erasure_payload_redaction(uuid) is the shared loop over it; an edge cites no remote
--      source, so its numbering is empty.
--
-- A later edit must not break:
--   * one loop: a sentinel rule changed in _erasure_trail_payload_redaction changes both acts;
--     a second copy of the loop would let them drift;
--   * the shared loop filters nothing by category or type: the trail scopes do, and every caller
--     passes ids a trail scope returned;
--   * the phantom arm reads the edge's row and its asserting event, so the edge trail is read
--     while the kb_edges row exists.

-- ---------------------------------------------------------------------------
-- Section 1. One derivation loop over a trail.
-- ---------------------------------------------------------------------------
-- Each event of p_trail whose allowlisted paths hold something other than their class sentinel,
-- with its payload and metadata rewritten to the sentinels and the paths it changed, in the dotted
-- form redacted_fields records. A path already at its sentinel is not listed, so an event with
-- nothing left to rewrite is not returned. p_remote is the remote-source numbering,
-- '<block id>|<normalized URI>' → n. A location with no sentinel number raises: the derivation
-- never guesses.
CREATE FUNCTION _erasure_trail_payload_redaction(p_trail uuid[], p_remote jsonb)
RETURNS TABLE(event_id uuid, new_payload jsonb, new_metadata jsonb, paths jsonb)
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    -- Each numbering is computed once per owner or endpoint pair and kept here, keyed
    -- '<owner or pair>|<text>': the per-location lookups read the cache, so the cost is linear in
    -- the trail, not in the trail times its owners' families.
    v_keys     jsonb := '{}'::jsonb;
    v_facets   jsonb := '{}'::jsonb;
    v_labels   jsonb := '{}'::jsonb;
    v_done     jsonb := '{}'::jsonb;
    v_ev       record;
    v_path     text;
    v_loc      record;
    v_class    text;
    v_meta     boolean;
    v_payload  jsonb;
    v_metadata jsonb;
    v_paths    jsonb;
    v_new      jsonb;
    v_block    uuid;
    v_owner    text;
    v_n        integer;
BEGIN
    -- An erasure act's own events (its relationship_folded, correlated with its resource_erased or
    -- edge_erased) are that act's record, not the erased text: a later erasure of an edge, or of
    -- an edge's other end, leaves them.
    FOR v_ev IN
        SELECT ev.id, et.name AS type, ev.payload, ev.metadata
          FROM (SELECT DISTINCT u.id FROM unnest(p_trail) AS u(id)) t
          JOIN kb_events ev ON ev.id = t.id
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE NOT EXISTS (
                   SELECT 1 FROM kb_events er JOIN kb_event_types ert ON ert.id = er.event_type_id
                    WHERE ert.name IN ('resource_erased', 'edge_erased')
                      AND er.correlation_id = ev.correlation_id AND er.id <> ev.id)
         ORDER BY ev.id
    LOOP
        v_payload  := v_ev.payload;
        v_metadata := v_ev.metadata;
        v_paths    := '[]'::jsonb;
        FOR v_path IN
            SELECT DISTINCT r.path FROM _erasure_redact_paths() r
             WHERE r.event_type = v_ev.type OR r.event_type IS NULL
             ORDER BY r.path
        LOOP
            v_class := _erasure_path_class(v_ev.type, v_path, v_ev.payload);
            CONTINUE WHEN v_class IS NULL OR v_class = 'keep';
            v_meta := v_path LIKE 'metadata.%';
            FOR v_loc IN
                SELECT x.at, x.val
                  FROM _erasure_expand(CASE WHEN v_meta THEN v_ev.metadata ELSE v_ev.payload END,
                                       CASE WHEN v_meta THEN substr(v_path, 10) ELSE v_path END) x
            LOOP
                -- A JSON null carries no text. Rewriting it would make replay project a value
                -- live never held (a doc_type row, and every later key number with it).
                CONTINUE WHEN jsonb_typeof(v_loc.val) = 'null';
                v_new := _erasure_sentinel_exact(v_class, v_ev.id, v_ev.payload);
                IF v_class = 'remote-source-url' THEN
                    IF coalesce(_erasure_location_is_remote(v_ev.payload, v_loc.at), false) THEN
                        v_block := _erasure_location_block(v_ev.payload, v_loc.at);
                        v_n := (p_remote ->> (v_block::text || '|'
                                              || normalize_remote_uri(v_loc.val #>> '{}')))::integer;
                        IF v_n IS NULL THEN
                            RAISE EXCEPTION '_erasure_trail_payload_redaction: no sentinel number for a remote source of block % in event %',
                                v_block, v_ev.id;
                        END IF;
                        v_new := to_jsonb('erased:' || v_block::text || ':' || v_n::text);
                    ELSE
                        v_new := v_loc.val;
                    END IF;
                ELSIF v_class = 'edge-label' THEN
                    IF _erasure_label_is_kept(v_loc.val) THEN
                        v_new := v_loc.val;
                    ELSE
                        v_owner := concat_ws('|', v_ev.payload #>> '{source,table}', v_ev.payload #>> '{source,id}',
                                             v_ev.payload #>> '{target,table}', v_ev.payload #>> '{target,id}');
                        IF NOT v_done ? ('label|' || v_owner) THEN
                            SELECT v_labels || coalesce(jsonb_object_agg(v_owner || '|' || l.label, l.n), '{}'::jsonb)
                              INTO v_labels
                              FROM _resource_erasure_label_numbers(
                                       v_ev.payload #>> '{source,table}', (v_ev.payload #>> '{source,id}')::uuid,
                                       v_ev.payload #>> '{target,table}', (v_ev.payload #>> '{target,id}')::uuid) l;
                            v_done := v_done || jsonb_build_object('label|' || v_owner, true);
                        END IF;
                        v_n := (v_labels ->> (v_owner || '|' || (v_loc.val #>> '{}')))::integer;
                        IF v_n IS NULL THEN
                            RAISE EXCEPTION '_erasure_trail_payload_redaction: no label number for event %', v_ev.id;
                        END IF;
                        v_new := to_jsonb('erased-label-' || v_n::text);
                    END IF;
                ELSIF v_class IN ('property-key', 'facet-value') THEN
                    v_owner := (v_ev.payload #>> '{owner,table}') || '|' || (v_ev.payload #>> '{owner,id}');
                    IF NOT v_done ? ('owner|' || v_owner) THEN
                        SELECT v_keys || coalesce(jsonb_object_agg(v_owner || '|' || k.property_key, k.n), '{}'::jsonb)
                          INTO v_keys
                          FROM _resource_erasure_ledger_key_numbers(v_ev.payload #>> '{owner,table}',
                                                                    (v_ev.payload #>> '{owner,id}')::uuid) k;
                        SELECT v_facets || coalesce(jsonb_object_agg(v_owner || '|' || f.inner_key, f.m), '{}'::jsonb)
                          INTO v_facets
                          FROM _resource_erasure_facet_numbers(v_ev.payload #>> '{owner,table}',
                                                               (v_ev.payload #>> '{owner,id}')::uuid) f;
                        v_done := v_done || jsonb_build_object('owner|' || v_owner, true);
                    END IF;
                    IF v_class = 'property-key' THEN
                        v_n := (v_keys ->> (v_owner || '|' || (v_loc.val #>> '{}')))::integer;
                        IF v_n IS NULL THEN
                            RAISE EXCEPTION '_erasure_trail_payload_redaction: no key number for event %', v_ev.id;
                        END IF;
                        v_new := to_jsonb('erased-key-' || v_n::text);
                    ELSIF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm WHERE fm.inner_key IS NULL) THEN
                        v_new := to_jsonb('erased:' || v_ev.id::text);
                    ELSIF NOT EXISTS (SELECT 1 FROM _facet_marks(v_loc.val)) THEN
                        -- An empty facet value names no mark: nothing to redact. Raising here
                        -- would refuse the act over a value that carries no text.
                        v_new := v_loc.val;
                    ELSE
                        IF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm
                                    WHERE NOT v_facets ? (v_owner || '|' || fm.inner_key)) THEN
                            RAISE EXCEPTION '_erasure_trail_payload_redaction: no mark number for a facet of event %', v_ev.id;
                        END IF;
                        SELECT jsonb_object_agg('erased-facet-' || (v_facets ->> (v_owner || '|' || fm.inner_key)), 'erased')
                          INTO v_new
                          FROM _facet_marks(v_loc.val) fm;
                    END IF;
                END IF;
                IF v_new IS NULL THEN
                    -- jsonb_set with a NULL value returns NULL: never let a missing number empty
                    -- a whole payload.
                    RAISE EXCEPTION '_erasure_trail_payload_redaction: no sentinel for % of event %', v_path, v_ev.id;
                END IF;
                IF v_new IS DISTINCT FROM v_loc.val THEN
                    IF v_meta THEN
                        v_metadata := jsonb_set(v_metadata, v_loc.at, v_new, false);
                    ELSE
                        v_payload := jsonb_set(v_payload, v_loc.at, v_new, false);
                    END IF;
                    IF NOT v_paths @> to_jsonb(ARRAY[v_path]) THEN
                        v_paths := v_paths || to_jsonb(v_path);
                    END IF;
                END IF;
            END LOOP;
        END LOOP;
        IF jsonb_array_length(v_paths) > 0 THEN
            event_id     := v_ev.id;
            new_payload  := v_payload;
            new_metadata := v_metadata;
            paths        := v_paths;
            RETURN NEXT;
        END IF;
    END LOOP;
END;
$$;

COMMENT ON FUNCTION _erasure_trail_payload_redaction(uuid[], jsonb) IS
$c$The ledger derivation of one erasure trail: for each event of p_trail (each id once, in event id
order), except an event correlated with a resource_erased or edge_erased other than itself, the
payload and metadata with every allowlisted path (_erasure_redact_paths) at its class sentinel, and
the paths that changed; an event with nothing to rewrite is not returned. Remote sources are
numbered from p_remote ('<block id>|<normalized URI>' → n), property keys from
_resource_erasure_ledger_key_numbers, facet marks from _resource_erasure_facet_numbers, edge labels
from _resource_erasure_label_numbers (per endpoint pair). Raises when a location has no sentinel
number. Called by _resource_erasure_payload_redaction and _edge_erasure_payload_redaction, whose
trail scopes choose the events.$c$;

-- The husk check and the remote-source numbering are 20261010100000's; the loop is Section 1's.
CREATE OR REPLACE FUNCTION _resource_erasure_payload_redaction(p_resource uuid)
RETURNS TABLE(event_id uuid, new_payload jsonb, new_metadata jsonb, paths jsonb)
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_remote   jsonb;
BEGIN
    -- The remote-source numbering, captured once: (block, normalized URI) → n. On a live resource,
    -- from the ONE capture step (9e) re-points by (_resource_erasure_remote_originals). On a husk,
    -- (9e) has already re-pointed it, so the numbering is read from the ledger in (9e)'s order
    -- (ruling 14), and must name exactly the sentinel rows the husk's provenance cites.
    IF EXISTS (SELECT 1 FROM kb_resources r WHERE r.id = p_resource AND r.erased_at IS NOT NULL) THEN
        -- The check is per provenance row, not per set: a set comparison would pass a
        -- permutation, the ledger giving one URL the n another URL's row cites, and the first
        -- rewrite of a path is final (kb_event_field_redactions' key), so a wrong n could never be
        -- corrected. Each remote row of the husk was inserted for the first element its
        -- contributing event lists for the block (ON CONFLICT DO NOTHING per event), at that
        -- element's seq, and (9e) re-pointed it to that element's n; so some element of that event,
        -- block and seq must carry a URL the ledger numbers to exactly the sentinel the row cites.
        -- Within one (event, seq), two URLs are ordered by their position in the list on both sides.
        -- And every number the ledger gives a block must be cited by some row of it, or the
        -- ledger names a source (9e) never re-pointed.
        IF EXISTS (
            SELECT 1
              FROM kb_block_provenance bp
              JOIN kb_content_blocks b ON b.id = bp.block_id
              JOIN kb_remote_sources rs ON rs.id = bp.source_id
             WHERE b.resource_id = p_resource AND bp.source_kind = 'remote'
               AND NOT EXISTS (
                   SELECT 1
                     FROM _resource_erasure_ledger_remote_elements(p_resource) el
                     JOIN _resource_erasure_ledger_remote_numbers(p_resource) ln
                       ON ln.block_id = el.block_id AND ln.uri_normalized = el.uri_normalized
                    WHERE el.block_id = bp.block_id
                      AND el.event_id = bp.contributed_by_event_id
                      AND el.seq = bp.accretion_seq
                      AND normalize_remote_uri('erased:' || ln.block_id::text || ':' || ln.n::text)
                          = rs.uri_normalized))
           OR EXISTS (
            SELECT 1
              FROM _resource_erasure_ledger_remote_numbers(p_resource) ln
             WHERE NOT EXISTS (
                   SELECT 1
                     FROM kb_block_provenance bp
                     JOIN kb_remote_sources rs ON rs.id = bp.source_id
                    WHERE bp.block_id = ln.block_id AND bp.source_kind = 'remote'
                      AND rs.uri_normalized
                          = normalize_remote_uri('erased:' || ln.block_id::text || ':' || ln.n::text))) THEN
            RAISE EXCEPTION '_resource_erasure_payload_redaction: the ledger''s remote-source numbering of erased resource % does not match the sentinels its provenance cites',
                p_resource;
        END IF;
        SELECT coalesce(jsonb_object_agg(l.block_id::text || '|' || l.uri_normalized, l.n), '{}'::jsonb)
          INTO v_remote
          FROM _resource_erasure_ledger_remote_numbers(p_resource) l;
    ELSE
        SELECT coalesce(jsonb_object_agg(o.block_id::text || '|' || rs.uri_normalized, o.n), '{}'::jsonb)
          INTO v_remote
          FROM _resource_erasure_remote_originals(p_resource) o
          JOIN kb_remote_sources rs ON rs.id = o.source_id;
    END IF;

    RETURN QUERY
    SELECT * FROM _erasure_trail_payload_redaction(
        ARRAY(SELECT ts.event_id FROM _resource_erasure_trail_scope(p_resource) ts), v_remote);
END;
$$;

COMMENT ON FUNCTION _resource_erasure_payload_redaction(uuid) IS
$c$The cut-2 ledger redaction of one resource (spec 2026-09-28 D3, D4): _erasure_trail_payload_redaction
over its trail (_resource_erasure_trail_scope), with the remote-source numbering from
_resource_erasure_remote_originals (on a husk, from _resource_erasure_ledger_remote_numbers, checked
against the sentinels its provenance cites; a mismatch raises). Property keys, facet marks and edge
labels are numbered by the shared loop. The survey plan reports its paths as redacted_fields; the
act, and on a husk the completion pass, writes its payloads.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. An edge's trail and its derivation.
-- ---------------------------------------------------------------------------
-- Each arm reads an expression an index of kb_events carries, as element_trail_edge does.
CREATE FUNCTION _edge_erasure_trail_scope(p_edge uuid)
RETURNS TABLE(event_id uuid, event_type text)
LANGUAGE sql STABLE AS $$
    WITH ids AS (
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'edge_id')::uuid = p_edge
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ((ev.payload -> 'owner') ->> 'id')::uuid = p_edge
           AND (ev.payload -> 'owner') ->> 'table' = 'kb_edges'
        UNION
        SELECT ph.event_id FROM _edge_phantom_reassertions(p_edge) ph
    )
    SELECT ev.id, et.name
      FROM ids
      JOIN kb_events ev ON ev.id = ids.id
      JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE et.category = 'domain'
       AND et.name <> 'webhook_received';
$$;

COMMENT ON FUNCTION _edge_erasure_trail_scope(uuid) IS
$c$An edge's own ledger events: those naming it by payload edge_id, the property events whose owner
is the edge (kb_edges), and the re-assertions recorded under a minted edge_id that never became a
row (_edge_phantom_reassertions), with each event's type. Domain events only; a received webhook is
never one of them, whatever its body carries. The phantom arm reads the edge's kb_edges row and its
asserting event, so it answers only while the row exists. The scope _edge_erasure_payload_redaction
derives over.$c$;

CREATE FUNCTION _edge_erasure_payload_redaction(p_edge uuid)
RETURNS TABLE(event_id uuid, new_payload jsonb, new_metadata jsonb, paths jsonb)
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT d.event_id, d.new_payload, d.new_metadata, d.paths
      FROM _erasure_trail_payload_redaction(
               ARRAY(SELECT s.event_id FROM _edge_erasure_trail_scope(p_edge) s), '{}'::jsonb) d;
$$;

COMMENT ON FUNCTION _edge_erasure_payload_redaction(uuid) IS
$c$The ledger redaction of one edge: _erasure_trail_payload_redaction over its trail
(_edge_erasure_trail_scope) with an empty remote-source numbering, since an edge cites no remote
source. Rewrites the edge's labels to erased-label-<n> (numbered per endpoint pair), its facet marks
to erased-facet-<m>, its other property keys and values to their sentinels, and the authorship
metadata of each event to "erased". Raises when a location has no sentinel number.$c$;

SELECT declare_migration(
    20261201000050,
    'additive',
    'Adds the functions _erasure_trail_payload_redaction(uuid[], jsonb), '
    '_edge_erasure_trail_scope(uuid) and _edge_erasure_payload_redaction(uuid), which nothing calls '
    'yet but _resource_erasure_payload_redaction. CREATE OR REPLACE of '
    '_resource_erasure_payload_redaction with its signature, return type and search_path unchanged: '
    'it returns the shared loop over the same trail with the same remote-source numbering. The '
    'loop also leaves events correlated with an edge_erased, an event type not yet registered, so '
    'its rows are unchanged. No table, column, constraint, grant or event type changes.'
);
