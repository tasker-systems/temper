-- Resource erasure, cut 2 PR 2: the ledger exception (build order 4).
--
-- Spec: temper-artifacts specs/2026-09-28-resource-erasure-design.md, D3, D4 (the payload side) and
-- D12, and the rulings from the cut-2 property-half design (2026-10-03). Plan:
-- plans/2026-10-08-erasure-cut2-pr2-ledger-exception.md.
--
-- An erasure run from now on rewrites the free text of the resource's own trail events in kb_events
-- to each path's class sentinel (D4), and the append-only trigger becomes a verifier that admits
-- exactly that rewrite and nothing else (D3). An erasure run under cut 1 is not touched here: the
-- completion pass is the next build.
--
--   1. _erasure_redact_paths(): the allowlist, the manifest's redact lines (and its qualified
--      structural lines, as `keep`) in the dotted path form redacted_fields uses. A function, not a
--      table: changing what the verifier admits takes DDL, the same privilege that replacing the
--      trigger takes. resource_erasure_surface_test holds it equal to the manifest.
--   2. The path helpers and the per-class sentinels, shared by the derivation and the verifier.
--   3. kb_event_field_redactions(event_id, path, redacted_by): append-only, projected from
--      resource_erased.redacted_fields by _project_resource_erased_redactions, which refuses a
--      path the allowlist does not hold.
--   4. _resource_erasure_payload_redaction(resource): each trail event's redacted payload and
--      metadata, and the paths it changed.
--   5. kb_events_append_only() becomes the verifier.
--   6. The trail scope, rewritten as a UNION of indexed arms (ruled 2026-10-08).
--   7. The survey plan reports `redacted_fields` from the derivation; its `ledger_remainder` keeps
--      only what the exception cannot reach (the telos copies).
--   8. The act appends redacted_fields, projects the redaction rows, and rewrites the events before
--      the redaction body runs.

-- ---------------------------------------------------------------------------
-- Section 1. The allowlist.
-- ---------------------------------------------------------------------------
-- One row per (event type, path, qualifier) the manifest classes `redact:<class>`, plus each
-- qualified `structural` line as class `keep`, so the most specific line wins. event_type NULL is
-- an authorship key on kb_events.metadata, path `metadata.<key>`, on any event. Paths are the dotted
-- form of the manifest's JSON pointers, `[*]` for an array's elements.
CREATE FUNCTION _erasure_redact_paths()
RETURNS TABLE(event_type text, path text, key_qualifier text, owner_qualifier text, class text)
LANGUAGE sql IMMUTABLE AS $$
    VALUES
        ('resource_created',           'title',                                    NULL, NULL, 'title'),
        ('resource_created',           'origin_uri',                               NULL, NULL, 'origin-uri'),
        ('resource_created',           'doc_type',                                 NULL, NULL, 'doc-type'),
        ('resource_created',           'blocks[*].incorporated[*].source.value',   NULL, NULL, 'remote-source-url'),
        ('resource_updated',           'title',                                    NULL, NULL, 'title'),
        ('resource_updated',           'origin_uri',                               NULL, NULL, 'origin-uri'),
        ('block_created',              'block.incorporated[*].source.value',       NULL, NULL, 'remote-source-url'),
        ('block_mutated',              'incorporated[*].source.value',             NULL, NULL, 'remote-source-url'),
        ('block_folded',               'reason',                                   NULL, NULL, 'reason'),
        ('block_provenance_annotated', 'incorporated[*].source.value',             NULL, NULL, 'remote-source-url'),
        ('block_provenance_corrected', 'scar',                                     NULL, NULL, 'scar'),
        ('block_provenance_corrected', 'source.value',                             NULL, NULL, 'remote-source-url'),
        ('resource_reblocked',         'created[*].attribution[*].source.value',   NULL, NULL, 'remote-source-url'),
        ('resource_reblocked',         'kept[*].attribution[*].source.value',      NULL, NULL, 'remote-source-url'),
        ('citation_audited',           'reason',                                   NULL, NULL, 'reason'),
        ('data_artifact_committed',    'artifact_kind',                            NULL, NULL, 'artifact-family'),
        ('property_set',               'property_key',                             NULL, NULL, 'property-key'),
        ('property_set',               'property_key',                             'doc_type', 'kb_resources', 'keep'),
        ('property_set',               'property_key',                             'facet', NULL, 'keep'),
        ('property_set',               'value',                                    NULL, NULL, 'property-value'),
        ('property_set',               'value',                                    'doc_type', 'kb_resources', 'doc-type'),
        ('property_set',               'value',                                    'facet', NULL, 'facet-value'),
        ('property_asserted',          'property_key',                             NULL, NULL, 'property-key'),
        ('property_asserted',          'property_key',                             'doc_type', 'kb_resources', 'keep'),
        ('property_asserted',          'property_key',                             'facet', NULL, 'keep'),
        ('property_asserted',          'value',                                    NULL, NULL, 'property-value'),
        ('property_asserted',          'value',                                    'doc_type', 'kb_resources', 'doc-type'),
        ('property_asserted',          'value',                                    'facet', NULL, 'facet-value'),
        ('property_unset',             'property_key',                             NULL, NULL, 'property-key'),
        ('property_unset',             'property_key',                             'doc_type', 'kb_resources', 'keep'),
        ('property_unset',             'property_key',                             'facet', NULL, 'keep'),
        ('relationship_asserted',      'label',                                    NULL, NULL, 'edge-label'),
        ('relationship_folded',        'reason',                                   NULL, NULL, 'reason'),
        ('relationship_corrected',     'scar',                                     NULL, NULL, 'scar'),
        (NULL,                         'metadata.reasoning',                       NULL, NULL, 'authorship'),
        (NULL,                         'metadata.rationale',                       NULL, NULL, 'authorship'),
        (NULL,                         'metadata.persona',                         NULL, NULL, 'authorship'),
        (NULL,                         'metadata.model',                           NULL, NULL, 'authorship')
$$;

COMMENT ON FUNCTION _erasure_redact_paths() IS
$c$The ledger exception's allowlist (spec 2026-09-28 D3, D9): every (event type, path, qualifier) a
resource_erased event may name in redacted_fields, with its D4 sentinel class; `keep` marks a
qualified line that refines a redact line back to structural (the literal keys doc_type and facet).
Generated from scripts/resource-erasure-surface.txt [payload] and held equal to it by
resource_erasure_surface_test. A function, so widening what the verifier admits takes DDL.$c$;

-- The class one path of one event takes: the most specific matching line, or NULL when the path is
-- not on the allowlist. A qualifier reads the event's ORIGINAL payload, its property_key and owner.
CREATE FUNCTION _erasure_path_class(p_event_type text, p_path text, p_payload jsonb)
RETURNS text
LANGUAGE sql IMMUTABLE AS $$
    SELECT r.class
      FROM _erasure_redact_paths() r
     WHERE r.path = p_path
       AND (r.event_type = p_event_type OR (r.event_type IS NULL AND p_path LIKE 'metadata.%'))
       AND (r.key_qualifier IS NULL OR r.key_qualifier = p_payload ->> 'property_key')
       AND (r.owner_qualifier IS NULL OR r.owner_qualifier = p_payload #>> '{owner,table}')
     ORDER BY (r.key_qualifier IS NOT NULL) DESC, (r.owner_qualifier IS NOT NULL) DESC
     LIMIT 1;
$$;

-- ---------------------------------------------------------------------------
-- Section 2. Paths and sentinels, one definition for the derivation and the verifier.
-- ---------------------------------------------------------------------------

-- Every concrete location a dotted path names in a document, with the value there. `[*]` expands
-- an array's elements by index; a missing key or a non-array under `[*]` names nothing. A
-- `metadata.` path is expanded by the caller against kb_events.metadata, without the prefix.
CREATE FUNCTION _erasure_expand(p_doc jsonb, p_path text)
RETURNS TABLE(at text[], val jsonb)
LANGUAGE sql IMMUTABLE AS $$
    WITH RECURSIVE tok AS (
        SELECT regexp_split_to_array(replace(p_path, '[*]', '.*'), '\.') AS t
    ), w(at, val, i) AS (
        SELECT ARRAY[]::text[], p_doc, 1
        UNION ALL
        SELECT w.at || s.k, s.v, w.i + 1
          FROM w, tok
         CROSS JOIN LATERAL (
            SELECT (e.ord - 1)::text AS k, e.v
              FROM jsonb_array_elements(
                     CASE WHEN tok.t[w.i] = '*' AND jsonb_typeof(w.val) = 'array'
                          THEN w.val ELSE '[]'::jsonb END) WITH ORDINALITY AS e(v, ord)
            UNION ALL
            SELECT tok.t[w.i], w.val -> tok.t[w.i]
             WHERE tok.t[w.i] <> '*' AND jsonb_typeof(w.val) = 'object' AND w.val ? tok.t[w.i]
         ) s
         WHERE w.i <= cardinality(tok.t)
    )
    SELECT w.at, w.val FROM w, tok WHERE w.i = cardinality(tok.t) + 1;
$$;

-- The block a remote-source location belongs to: the element's block in a list of blocks
-- (resource_created's `blocks`, resource_reblocked's `created` and `kept`), block_created's
-- `block`, otherwise the event's own `block_id`.
CREATE FUNCTION _erasure_location_block(p_payload jsonb, p_at text[])
RETURNS uuid
LANGUAGE sql IMMUTABLE AS $$
    SELECT (CASE
                WHEN p_at[1] IN ('blocks', 'created', 'kept')
                    THEN p_payload #>> ARRAY[p_at[1], p_at[2], 'block_id']
                WHEN p_at[1] = 'block' THEN p_payload #>> '{block,block_id}'
                ELSE p_payload ->> 'block_id'
            END)::uuid;
$$;

-- A remote-source location is redacted only when its source is remote: the same path holds a
-- resource or event id when the kind is `resource` or `event`, which is structural.
CREATE FUNCTION _erasure_location_is_remote(p_payload jsonb, p_at text[])
RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT p_payload #>> (p_at[1:cardinality(p_at) - 1] || 'kind'::text) = 'remote';
$$;

-- The sentinel of the classes D4 derives from the event alone. NULL for the classes that need a
-- numbering (property-key, remote-source-url, facet-value, edge-label) and for `keep`.
CREATE FUNCTION _erasure_sentinel_exact(p_class text, p_event uuid, p_payload jsonb)
RETURNS jsonb
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE p_class
        WHEN 'title'           THEN to_jsonb('erased-' || (p_payload ->> 'resource_id'))
        WHEN 'origin-uri'      THEN to_jsonb('erased:' || (p_payload ->> 'resource_id'))
        WHEN 'property-value'  THEN to_jsonb('erased:' || p_event::text)
        WHEN 'doc-type'        THEN to_jsonb('erased:' || p_event::text)
        WHEN 'artifact-family' THEN to_jsonb('erased:' || p_event::text)
        WHEN 'reason'          THEN 'null'::jsonb
        WHEN 'scar'            THEN '"erased"'::jsonb
        WHEN 'authorship'      THEN '"erased"'::jsonb
    END;
$$;

-- An edge label that redaction leaves as it is: none, or empty (the edge identity's COALESCE
-- reads both as ''). Every other label is text to number, a label typed in a sentinel's shape
-- included: treating one as already redacted let a real label be numbered onto the same text and
-- merge two edges on replay (found by the security review, 2026-10-08).
CREATE FUNCTION _erasure_label_is_kept(p_label jsonb)
RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT p_label IS NULL OR jsonb_typeof(p_label) <> 'string' OR p_label #>> '{}' = '';
$$;

-- Whether p_new is the sentinel the verifier admits at one location (D3 condition 3). Exact for
-- every class D4 derives from the event; by pattern for a property key (its n needs the owner's
-- whole family; Witness 25 checks it) and a remote source (its n needs the block's provenance,
-- which the act's own rewrite is changing as this runs), with the block id exact; a facet value
-- by shape and by the original's mark count.
CREATE FUNCTION _erasure_sentinel_admits(p_class text, p_event uuid, p_payload jsonb, p_at text[],
                                         p_old jsonb, p_new jsonb)
RETURNS boolean
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v_marks integer;
    v_scalar boolean;
BEGIN
    IF p_new IS NULL THEN
        RETURN false;
    END IF;
    CASE p_class
        WHEN 'keep' THEN
            RETURN p_new = p_old;
        WHEN 'property-key' THEN
            RETURN jsonb_typeof(p_new) = 'string' AND p_new #>> '{}' ~ '^erased-key-[1-9][0-9]*$';
        WHEN 'edge-label' THEN
            IF _erasure_label_is_kept(p_old) THEN
                RETURN p_new = p_old;
            END IF;
            RETURN jsonb_typeof(p_new) = 'string' AND p_new #>> '{}' ~ '^erased-label-[1-9][0-9]*$';
        WHEN 'remote-source-url' THEN
            IF NOT coalesce(_erasure_location_is_remote(p_payload, p_at), false) THEN
                RETURN p_new = p_old;
            END IF;
            RETURN jsonb_typeof(p_new) = 'string'
               AND p_new #>> '{}' ~ ('^erased:' || _erasure_location_block(p_payload, p_at)::text
                                     || ':[1-9][0-9]*$');
        WHEN 'facet-value' THEN
            SELECT count(*), bool_or(m.inner_key IS NULL)
              INTO v_marks, v_scalar
              FROM _facet_marks(p_old) m;
            IF v_scalar THEN
                RETURN p_new = to_jsonb('erased:' || p_event::text);
            END IF;
            RETURN jsonb_typeof(p_new) = 'object'
               AND (SELECT count(*) FROM jsonb_object_keys(p_new)) = v_marks
               AND NOT EXISTS (SELECT 1 FROM jsonb_each(p_new) kv
                                WHERE kv.key !~ '^erased-facet-[1-9][0-9]*$'
                                   OR kv.value <> '"erased"'::jsonb);
        ELSE
            RETURN p_new = _erasure_sentinel_exact(p_class, p_event, p_payload);
    END CASE;
END;
$$;

-- ---------------------------------------------------------------------------
-- Section 3. kb_event_field_redactions: the ledger's own authorization of each rewrite.
-- ---------------------------------------------------------------------------
-- A projection of resource_erased.redacted_fields: rebuildable from the ledger alone (replay's
-- ResourceErased arm calls the projector), dumped by replay, and append-only. A row is the only
-- thing the verifier (Section 5) accepts as leave to change a path of an event, and only when
-- redacted_by names a resource_erased event.
CREATE TABLE kb_event_field_redactions (
    event_id    uuid NOT NULL REFERENCES kb_events(id),
    path        text NOT NULL,
    redacted_by uuid NOT NULL REFERENCES kb_events(id),
    PRIMARY KEY (event_id, path)
);
CREATE INDEX idx_kb_event_field_redactions_redacted_by ON kb_event_field_redactions (redacted_by);

CREATE FUNCTION kb_event_field_redactions_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'kb_event_field_redactions is append-only';
END;
$$;

CREATE TRIGGER kb_event_field_redactions_append_only
    BEFORE UPDATE OR DELETE ON kb_event_field_redactions
    FOR EACH ROW EXECUTE FUNCTION kb_event_field_redactions_append_only();

COMMENT ON TABLE kb_event_field_redactions IS
$c$Which path of which event a resource erasure rewrote (spec 2026-09-28 D3): one row per (event, path)
of resource_erased.redacted_fields, redacted_by the erasure event. The ledger authorizes its own
redaction: kb_events_append_only admits an UPDATE only at paths these rows name. Append-only; a
projection, rebuilt by replay.$c$;

-- The projector. It refuses an event outside the subject's trail (the exception never crosses the
-- resource boundary, Q1) and a path the allowlist does not hold for that event's type, so the
-- fence (D9), not the event, decides what an erasure may name.
CREATE FUNCTION _project_resource_erased_redactions(p_event uuid, p_payload jsonb)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_entry jsonb;
    v_path  text;
    v_type  text;
    v_trail jsonb;
BEGIN
    -- The subject's trail, once: event id → true.
    SELECT coalesce(jsonb_object_agg(ts.event_id::text, true), '{}'::jsonb)
      INTO v_trail
      FROM _resource_erasure_trail_scope((p_payload ->> 'subject_id')::uuid) ts
     WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    FOR v_entry IN
        SELECT e FROM jsonb_array_elements(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) e
    LOOP
        SELECT et.name INTO v_type
          FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE ev.id = (v_entry ->> 'event')::uuid;
        IF v_type IS NULL THEN
            RAISE EXCEPTION '_project_resource_erased_redactions: event % names a missing event %',
                p_event, v_entry ->> 'event';
        END IF;
        -- The replay walk projects the record as the ledger holds it, and a D14 order inversion
        -- can put a trail event's own block or edge after the act: checking the trail there would
        -- abort the whole replay. Live, kb_events_redaction_in_trail checks it again at the
        -- rewrite, so this check is the earlier of two.
        IF NOT v_trail ? (v_entry ->> 'event')
           AND coalesce(current_setting('temper.replaying', true), '') <> 'on' THEN
            RAISE EXCEPTION '_project_resource_erased_redactions: event % names event %, which is not in its subject''s trail',
                p_event, v_entry ->> 'event';
        END IF;
        FOR v_path IN SELECT jsonb_array_elements_text(v_entry -> 'paths') LOOP
            IF NOT EXISTS (
                SELECT 1 FROM _erasure_redact_paths() r
                 WHERE r.path = v_path AND r.class <> 'keep'
                   AND (r.event_type = v_type
                        OR (r.event_type IS NULL AND v_path LIKE 'metadata.%'))) THEN
                RAISE EXCEPTION '_project_resource_erased_redactions: event % names path % of a % event, which the allowlist does not hold',
                    p_event, v_path, v_type;
            END IF;
            INSERT INTO kb_event_field_redactions (event_id, path, redacted_by)
            VALUES ((v_entry ->> 'event')::uuid, v_path, p_event);
        END LOOP;
    END LOOP;
END;
$$;

-- ---------------------------------------------------------------------------
-- Section 4. The derivation: each trail event's redacted payload and metadata.
-- ---------------------------------------------------------------------------

-- The payload-side key numbering (D4): each ORIGINAL key an owner's property events name, with the
-- n of erased-key-<n>. A key some property_set or property_asserted asserted takes the n that
-- _resource_erasure_key_numbers gives the row that event asserted (the ONE projection-side
-- numbering, so a payload key always equals its row's key: Witness 25). A key with no asserted row
-- (only ever unset, ruled 2026-10-03, or named by an event no projector turned into a row) is
-- numbered after the asserted range, in ledger order of the first event naming it, so no asserted
-- n moves. Reads original key text from the payloads and n through asserted_by_event_id,
-- so it gives the same answer before step 9 rewrites the projection's keys and after.
CREATE FUNCTION _resource_erasure_ledger_key_numbers(p_owner_table text, p_owner_id uuid)
RETURNS TABLE(property_key text, n integer)
LANGUAGE sql STABLE AS $$
    WITH owned AS (
        SELECT ev.id, ev.occurred_at, et.name, ev.payload ->> 'property_key' AS key
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE ((ev.payload -> 'owner') ->> 'id')::uuid = p_owner_id
           AND (ev.payload -> 'owner') ->> 'table' = p_owner_table
           AND et.name IN ('property_set', 'property_asserted', 'property_unset')
    ), numbered AS (
        SELECT k.property_key, k.n FROM _resource_erasure_key_numbers(p_owner_table, p_owner_id) k
    ), asserted AS (
        SELECT DISTINCT ON (o.key) o.key, nb.n
          FROM owned o
          JOIN kb_properties p ON p.asserted_by_event_id = o.id
                              AND p.owner_table = p_owner_table AND p.owner_id = p_owner_id
          JOIN numbered nb ON nb.property_key = p.property_key
         WHERE o.name IN ('property_set', 'property_asserted')
         ORDER BY o.key, o.id
    ), unnumbered AS (
        SELECT DISTINCT ON (o.key) o.key, o.occurred_at, o.id
          FROM owned o
         WHERE NOT EXISTS (SELECT 1 FROM asserted a WHERE a.key = o.key)
         ORDER BY o.key, o.occurred_at, o.id
    )
    SELECT a.key, a.n FROM asserted a
    UNION ALL
    SELECT u.key,
           ((SELECT coalesce(max(nb.n), 0) FROM numbered nb)
             + row_number() OVER (ORDER BY u.occurred_at, u.id))::integer
      FROM unnumbered u;
$$;

-- The facet inner-key numbering (D4, ruled 2026-10-03): each inner key an owner's facet events
-- name, with the m of erased-facet-<m>, in ledger order of the inner key's first appearance (its
-- first event's occurred_at, then the event id, then its position in the value as stored).
CREATE FUNCTION _resource_erasure_facet_numbers(p_owner_table text, p_owner_id uuid)
RETURNS TABLE(inner_key text, m integer)
LANGUAGE sql STABLE AS $$
    WITH marks AS (
        SELECT DISTINCT ON (fm.inner_key) fm.inner_key, ev.occurred_at, ev.id, fm.ord
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         CROSS JOIN LATERAL _facet_marks(ev.payload -> 'value') WITH ORDINALITY AS fm(inner_key, inner_value, ord)
         WHERE ((ev.payload -> 'owner') ->> 'id')::uuid = p_owner_id
           AND (ev.payload -> 'owner') ->> 'table' = p_owner_table
           AND et.name IN ('property_set', 'property_asserted')
           AND ev.payload ->> 'property_key' = 'facet'
           AND fm.inner_key IS NOT NULL
         ORDER BY fm.inner_key, ev.occurred_at, ev.id, fm.ord
    )
    SELECT mk.inner_key, (row_number() OVER (ORDER BY mk.occurred_at, mk.id, mk.ord))::integer
      FROM marks mk;
$$;

-- The edge-label numbering (ruled 2026-10-08): each distinct label asserted from one endpoint to
-- another, with the n of erased-label-<n>, in ledger order of its first assertion (occurred_at,
-- then event id; never the text). A label is part of an edge's identity (uq_kb_edges_assertion
-- covers source, target, kind, home and label), so two assertions can collide only between the
-- same ordered pair of endpoints: a numbering injective within the pair keeps every equality
-- replay's ON CONFLICT decides on. Numbering per pair, not per erased resource, makes it the same
-- whichever endpoint is erased, so erasing the other end later finds every label already at its
-- sentinel and renumbers nothing; it also tells a reader of one pair nothing about another.
-- Reads the asserting events by their payload endpoints, so a re-assertion (whose edge_id never
-- became a row) is numbered with the rest.
CREATE FUNCTION _resource_erasure_label_numbers(p_source_table text, p_source_id uuid,
                                                p_target_table text, p_target_id uuid)
RETURNS TABLE(label text, n integer)
LANGUAGE sql STABLE AS $$
    WITH firsts AS (
        SELECT DISTINCT ON (ev.payload ->> 'label') ev.payload ->> 'label' AS label, ev.occurred_at, ev.id
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE (ev.payload -> 'source') ->> 'id' = p_source_id::text
           AND (ev.payload -> 'source') ->> 'table' = p_source_table
           AND (ev.payload -> 'target') ->> 'id' = p_target_id::text
           AND (ev.payload -> 'target') ->> 'table' = p_target_table
           AND et.name = 'relationship_asserted'
           AND NOT _erasure_label_is_kept(ev.payload -> 'label')
         ORDER BY ev.payload ->> 'label', ev.occurred_at, ev.id
    )
    SELECT f.label, (row_number() OVER (ORDER BY f.occurred_at, f.id))::integer FROM firsts f;
$$;

-- Each event of the resource's trail whose allowlisted paths hold something other than their class
-- sentinel, with its payload and metadata rewritten to the sentinels (D4) and the paths it changed,
-- in the dotted form redacted_fields records. A path already at its sentinel is not listed (D12:
-- the completion pass never re-redacts), so an event with nothing left to rewrite is not returned.
-- Reads the ledger and the projection as they stand; it must run before step 9 of the act's body
-- only in that the act's own fold events must not exist yet, which the caller guarantees by
-- running it first. A location with no sentinel number raises: the derivation never guesses.
CREATE FUNCTION _resource_erasure_payload_redaction(p_resource uuid)
RETURNS TABLE(event_id uuid, new_payload jsonb, new_metadata jsonb, paths jsonb)
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_remote   jsonb;
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
    -- The remote-source numbering, captured once: (block, normalized URI) → n, from the ONE
    -- capture step (9e) re-points by (_resource_erasure_remote_originals).
    SELECT coalesce(jsonb_object_agg(o.block_id::text || '|' || rs.uri_normalized, o.n), '{}'::jsonb)
      INTO v_remote
      FROM _resource_erasure_remote_originals(p_resource) o
      JOIN kb_remote_sources rs ON rs.id = o.source_id;

    -- An erasure act's own events (its relationship_folded, correlated with its resource_erased)
    -- are that act's record, not R's text: a later erasure of an edge's other end leaves them.
    FOR v_ev IN
        SELECT s.event_id AS id, s.event_type AS type, ev.payload, ev.metadata
          FROM _resource_erasure_trail_scope(p_resource) s
          JOIN kb_events ev ON ev.id = s.event_id
         WHERE NOT EXISTS (
                   SELECT 1 FROM kb_events er JOIN kb_event_types ert ON ert.id = er.event_type_id
                    WHERE ert.name = 'resource_erased'
                      AND er.correlation_id = ev.correlation_id AND er.id <> ev.id)
         ORDER BY s.event_id
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
                        v_n := (v_remote ->> (v_block::text || '|'
                                              || normalize_remote_uri(v_loc.val #>> '{}')))::integer;
                        IF v_n IS NULL THEN
                            RAISE EXCEPTION '_resource_erasure_payload_redaction: no sentinel number for a remote source of block % in event %',
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
                            RAISE EXCEPTION '_resource_erasure_payload_redaction: no label number for event %', v_ev.id;
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
                            RAISE EXCEPTION '_resource_erasure_payload_redaction: no key number for event %', v_ev.id;
                        END IF;
                        v_new := to_jsonb('erased-key-' || v_n::text);
                    ELSIF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm WHERE fm.inner_key IS NULL) THEN
                        v_new := to_jsonb('erased:' || v_ev.id::text);
                    ELSIF NOT EXISTS (SELECT 1 FROM _facet_marks(v_loc.val)) THEN
                        -- An empty facet value names no mark: nothing to redact (found by the
                        -- code review, 2026-10-08: it raised, and blocked the act).
                        v_new := v_loc.val;
                    ELSE
                        IF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm
                                    WHERE NOT v_facets ? (v_owner || '|' || fm.inner_key)) THEN
                            RAISE EXCEPTION '_resource_erasure_payload_redaction: no mark number for a facet of event %', v_ev.id;
                        END IF;
                        SELECT jsonb_object_agg('erased-facet-' || (v_facets ->> (v_owner || '|' || fm.inner_key)), 'erased')
                          INTO v_new
                          FROM _facet_marks(v_loc.val) fm;
                    END IF;
                END IF;
                IF v_new IS NULL THEN
                    -- jsonb_set with a NULL value returns NULL: never let a missing number empty
                    -- a whole payload.
                    RAISE EXCEPTION '_resource_erasure_payload_redaction: no sentinel for % of event %', v_path, v_ev.id;
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

COMMENT ON FUNCTION _resource_erasure_payload_redaction(uuid) IS
$c$The cut-2 ledger redaction of one resource (spec 2026-09-28 D3, D4): for each event of its trail
(_resource_erasure_trail_scope), the payload and metadata with every allowlisted path
(_erasure_redact_paths) at its class sentinel, and the paths that changed. Sentinel numbers come
from the ONE numberings: remote sources from _resource_erasure_remote_originals, property keys from
_resource_erasure_ledger_key_numbers (through _resource_erasure_key_numbers), facet marks from
_resource_erasure_facet_numbers, edge labels from _resource_erasure_label_numbers. The survey plan
reports its paths as redacted_fields; the act
writes its payloads.$c$;

-- ---------------------------------------------------------------------------
-- Section 5. The verifier (D3).
-- ---------------------------------------------------------------------------
-- DELETE always raises. An UPDATE is admitted only when (1) no column but payload and metadata
-- changes; (2) every location that changes lies under a path a kb_event_field_redactions row names
-- for the event, and that row's resource_erased event was written in THIS transaction, lists the
-- event and the path in its own redacted_fields, and names a subject that is already erased; (3)
-- each changed location holds its class sentinel; and (4) with every named location set aside on
-- both sides, nothing else changed. The trail check, that the event is in the erasure's subject's
-- trail, runs once per statement in kb_events_redaction_in_trail below. Every refusal raises the
-- message the trigger has always raised. No GUC, role or session flag reaches it.
--
-- "This transaction" makes an authorization single-use: the act's own transaction, never a later
-- UPDATE that renumbers a sentinel the pattern checks would admit (found by both reviews,
-- 2026-10-08). It is read as the authorizing event's `created` equal to now(), the transaction's
-- start time, which kb_events.created defaults to and which a savepoint shares. xmin was the first
-- choice and would refuse an act run inside a savepoint, whose rows carry the subtransaction's id.
-- A later transaction cannot match an earlier event's stamp, and a concurrent one's uncommitted
-- event is invisible here.
CREATE OR REPLACE FUNCTION kb_events_append_only() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_type   text;
    v_row    record;
    v_loc    record;
    v_class  text;
    v_meta   boolean;
    v_rows   integer := 0;
    v_new    jsonb;
    v_old_p  jsonb := OLD.payload;
    v_new_p  jsonb;
    v_old_m  jsonb := OLD.metadata;
    v_new_m  jsonb;
BEGIN
    IF TG_OP <> 'UPDATE' THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    v_new_p := NEW.payload;
    v_new_m := NEW.metadata;
    IF (to_jsonb(OLD) - 'payload' - 'metadata') IS DISTINCT FROM (to_jsonb(NEW) - 'payload' - 'metadata') THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    SELECT et.name INTO v_type FROM kb_event_types et WHERE et.id = OLD.event_type_id;
    FOR v_row IN
        SELECT r.path, r.redacted_by FROM kb_event_field_redactions r WHERE r.event_id = OLD.id
    LOOP
        v_rows := v_rows + 1;
        v_class := _erasure_path_class(v_type, v_row.path, OLD.payload);
        IF v_class IS NULL THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        v_meta := v_row.path LIKE 'metadata.%';
        FOR v_loc IN
            SELECT x.at, x.val
              FROM _erasure_expand(CASE WHEN v_meta THEN OLD.metadata ELSE OLD.payload END,
                                   CASE WHEN v_meta THEN substr(v_row.path, 10) ELSE v_row.path END) x
        LOOP
            v_new := (CASE WHEN v_meta THEN NEW.metadata ELSE NEW.payload END) #> v_loc.at;
            IF v_new IS DISTINCT FROM v_loc.val THEN
                IF NOT EXISTS (
                    SELECT 1 FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id
                     WHERE e.id = v_row.redacted_by AND t.name = 'resource_erased'
                       AND e.created = now()
                       AND e.payload -> 'redacted_fields' @> jsonb_build_array(jsonb_build_object(
                               'event', OLD.id::text, 'paths', jsonb_build_array(v_row.path)))
                       AND EXISTS (SELECT 1 FROM kb_resources r
                                    WHERE r.id = (e.payload ->> 'subject_id')::uuid
                                      AND r.erased_at IS NOT NULL))
                   OR NOT _erasure_sentinel_admits(v_class, OLD.id, OLD.payload, v_loc.at, v_loc.val, v_new) THEN
                    RAISE EXCEPTION 'event ledger is append-only';
                END IF;
            END IF;
            IF v_meta THEN
                v_old_m := jsonb_set(v_old_m, v_loc.at, 'null'::jsonb, false);
                v_new_m := jsonb_set(v_new_m, v_loc.at, 'null'::jsonb, false);
            ELSE
                v_old_p := jsonb_set(v_old_p, v_loc.at, 'null'::jsonb, false);
                v_new_p := jsonb_set(v_new_p, v_loc.at, 'null'::jsonb, false);
            END IF;
        END LOOP;
    END LOOP;
    IF v_rows = 0
       OR v_old_p IS DISTINCT FROM v_new_p
       OR v_old_m IS DISTINCT FROM v_new_m THEN
        RAISE EXCEPTION 'event ledger is append-only';
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION kb_events_append_only() IS
$c$The ledger's guard (spec 2026-09-28 D3). DELETE always raises. An UPDATE is admitted only as a
resource erasure's redaction in the erasure's own transaction: no column but payload and metadata
changes; every changed location lies under a path a kb_event_field_redactions row names, whose
resource_erased event was written in this transaction, lists the event and the path, and names an
erased subject; each changed location holds its class sentinel (_erasure_sentinel_admits); nothing
else changed. kb_events_redaction_in_trail adds, per statement, that each changed event is in the
erasure's subject's trail. Anything else raises 'event ledger is append-only', as it always has.$c$;

-- The trail check, once per statement and once per erasure subject rather than once per row: every
-- event the statement changed under this transaction's erasure must be in that erasure's subject's
-- trail. The exception never reaches another resource's events (Q1), whatever a row inserted by
-- hand or a forged record says.
CREATE FUNCTION kb_events_redaction_in_trail() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_subject uuid;
BEGIN
    FOR v_subject IN
        SELECT DISTINCT (e.payload ->> 'subject_id')::uuid
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        IF EXISTS (
            WITH trail AS MATERIALIZED (
                SELECT ts.event_id FROM _resource_erasure_trail_scope(v_subject) ts
            )
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r ON r.event_id = n.id
              JOIN kb_events e ON e.id = r.redacted_by
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND e.created = now()
               AND (e.payload ->> 'subject_id')::uuid = v_subject
               AND NOT EXISTS (SELECT 1 FROM trail t WHERE t.event_id = n.id)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;
    RETURN NULL;
END;
$$;

CREATE TRIGGER kb_events_redaction_in_trail
    AFTER UPDATE ON kb_events
    REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION kb_events_redaction_in_trail();

-- ---------------------------------------------------------------------------
-- Section 6. The trail scope as a UNION of indexed arms (ruled 2026-10-08).
-- ---------------------------------------------------------------------------
-- A relationship_asserted that re-asserts a live edge carries a freshly minted edge_id, and
-- relationship_assert's ON CONFLICT answers with the existing edge, so that id never becomes a
-- row and the edge_id arm cannot reach the event. Production held 243 such events on
-- 2026-10-08, every one labelled and touching a resource. They are reached by their payload
-- endpoints instead, through these two indexes (text, not uuid: a received webhook's version-1
-- body sits at the payload root and may carry a non-uuid `source.id`, which a uuid cast in an
-- index expression would refuse to store).
CREATE INDEX idx_kb_events_payload_source_id ON kb_events (((payload -> 'source') ->> 'id'));
CREATE INDEX idx_kb_events_payload_target_id ON kb_events (((payload -> 'target') ->> 'id'));

-- 20261008100000's single predicate, arm for arm, each on the expression an index of kb_events
-- carries (payload->>'resource_id', ->'owner'->>'id', ->>'block_id', ->>'edge_id'), the
-- element_trail_node form; the OR form read every ledger row on every call. Plus the two
-- endpoint arms above, which reach a re-assertion the edge_id arm cannot.
CREATE OR REPLACE FUNCTION public._resource_erasure_trail_scope(p_resource uuid)
 RETURNS TABLE(event_id uuid, event_type text)
 LANGUAGE sql
 STABLE
AS $function$
    WITH blocks AS MATERIALIZED (
        SELECT b.id FROM kb_content_blocks b WHERE b.resource_id = p_resource
    ), touching AS MATERIALIZED (
        SELECT e.id FROM kb_edges e
         WHERE (e.source_table = 'kb_resources' AND e.source_id = p_resource)
            OR (e.target_table = 'kb_resources' AND e.target_id = p_resource)
    ), ids AS (
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'resource_id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ((ev.payload -> 'owner') ->> 'id')::uuid = p_resource
           AND (ev.payload -> 'owner') ->> 'table' = 'kb_resources'
        UNION
        SELECT ev.id FROM blocks b
          JOIN kb_events ev ON (ev.payload ->> 'block_id')::uuid = b.id
        UNION
        SELECT ev.id FROM touching t
          JOIN kb_events ev ON (ev.payload ->> 'edge_id')::uuid = t.id
        UNION
        SELECT ev.id FROM blocks b
          JOIN kb_events ev ON ((ev.payload -> 'owner') ->> 'id')::uuid = b.id
         WHERE (ev.payload -> 'owner') ->> 'table' = 'kb_content_blocks'
        UNION
        SELECT ev.id FROM touching t
          JOIN kb_events ev ON ((ev.payload -> 'owner') ->> 'id')::uuid = t.id
         WHERE (ev.payload -> 'owner') ->> 'table' = 'kb_edges'
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload -> 'source') ->> 'id' = p_resource::text
           AND (ev.payload -> 'source') ->> 'table' = 'kb_resources'
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload -> 'target') ->> 'id' = p_resource::text
           AND (ev.payload -> 'target') ->> 'table' = 'kb_resources'
    )
    SELECT ev.id, et.name
      FROM ids
      JOIN kb_events ev ON ev.id = ids.id
      JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE et.category = 'domain'
       AND et.name <> 'webhook_received';
$function$
;

-- The sweep attributes a ledger finding to the resource whose trail holds its event, and its own
-- witness holds that attribution to the trail scope: the endpoint arm joins it, last, after the
-- edge arms, so a projected edge still answers through its row.
CREATE OR REPLACE FUNCTION sensitivity.event_resource(p_type text, p_category text, p_payload jsonb) RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN p_category <> 'domain' OR p_type = 'webhook_received' THEN NULL ELSE coalesce(
        sensitivity.as_uuid(p_payload ->> 'resource_id'),
        CASE WHEN p_payload #>> '{owner,table}' = 'kb_resources'
             THEN sensitivity.as_uuid(p_payload #>> '{owner,id}') END,
        (SELECT b.resource_id FROM kb_content_blocks b
          WHERE b.id = sensitivity.as_uuid(p_payload ->> 'block_id')),
        (SELECT b.resource_id FROM kb_content_blocks b
          WHERE p_payload #>> '{owner,table}' = 'kb_content_blocks'
            AND b.id = sensitivity.as_uuid(p_payload #>> '{owner,id}')),
        (SELECT CASE WHEN e.source_table = 'kb_resources' THEN e.source_id
                     WHEN e.target_table = 'kb_resources' THEN e.target_id END
           FROM kb_edges e
          WHERE e.id = coalesce(sensitivity.as_uuid(p_payload ->> 'edge_id'),
                                CASE WHEN p_payload #>> '{owner,table}' = 'kb_edges'
                                     THEN sensitivity.as_uuid(p_payload #>> '{owner,id}') END)),
        CASE WHEN p_payload #>> '{source,table}' = 'kb_resources'
             THEN sensitivity.as_uuid(p_payload #>> '{source,id}')
             WHEN p_payload #>> '{target,table}' = 'kb_resources'
             THEN sensitivity.as_uuid(p_payload #>> '{target,id}') END)
    END;
$$;

-- ---------------------------------------------------------------------------
-- Section 7. The survey plan: redacted_fields from the derivation; ledger_remainder keeps the rest.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.resource_erasure_survey_plan(p_resource uuid)
 RETURNS jsonb
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_exists        uuid;
    v_n_blocks      integer;
    v_n_revisions   integer;
    v_n_chunks      integer;
    v_n_artifacts   integer;
    v_n_edges       integer;
    v_edges         jsonb := '[]'::jsonb;
    v_charter_of    uuid;
    v_ingest        text;
    v_ledger        jsonb := '[]'::jsonb;
    v_redacted      jsonb := '[]'::jsonb;
    v_genesis       uuid;
    v_erased        uuid;
    v_home_ctx      uuid;
    v_is_goal       boolean;
    v_remainder     jsonb := '[]'::jsonb;
    v_row           record;
    v_targets       jsonb := '[]'::jsonb;
    v_a             bigint;
    v_b             bigint;
    v_c             bigint;
BEGIN
    SELECT id INTO v_exists FROM kb_resources WHERE id = p_resource;
    IF v_exists IS NULL THEN
        RAISE EXCEPTION 'resource_erasure_survey_plan: resource % not found', p_resource;
    END IF;

    -- ── Scope counts, PRE-redaction state (the same discipline the principal plan's per-target
    --    arms hold: the reads only report what the act will find). ────────────────────────────
    SELECT count(*) INTO v_n_blocks FROM kb_content_blocks WHERE resource_id = p_resource;
    SELECT count(*) INTO v_n_revisions FROM kb_block_revisions br
      JOIN kb_content_blocks b ON b.id = br.block_id
     WHERE b.resource_id = p_resource;
    SELECT count(*) INTO v_n_chunks FROM kb_chunks WHERE resource_id = p_resource;
    SELECT count(*) INTO v_n_artifacts FROM kb_data_artifacts WHERE resource_id = p_resource;
    SELECT count(*) INTO v_n_edges FROM kb_edges e
      WHERE NOT e.is_folded
        AND ((e.source_table = 'kb_resources' AND e.source_id = p_resource)
          OR (e.target_table = 'kb_resources' AND e.target_id = p_resource));

    -- ── The edges the act folds, each listed so the record's `folded_edges` and the per-edge
    --     events agree. LIVE edges only: an edge already folded by history is the fold's
    --     business, not this act's — enumerating it would abort execute against a lawful
    --     state. `folded_edges` lists only what THIS act folds; a pre-existing fold already
    --     carries its own event. ────────────────────────────────────────────────────────
    FOR v_row IN
        SELECT e.id
          FROM kb_edges e
          WHERE NOT e.is_folded
            AND ((e.source_table = 'kb_resources' AND e.source_id = p_resource)
              OR (e.target_table = 'kb_resources' AND e.target_id = p_resource))
          ORDER BY e.id
    LOOP
        v_edges := v_edges || to_jsonb(v_row.id);
    END LOOP;

    -- ── THE TARGETS (D8): what the act will reach, one {target, outcome} per D2 step that
    --    reaches something, counted PRE-act here because the resource_erased payload is
    --    appended before the redaction body runs and the act never computes twice (D10). Each
    --    count reads the row set its step in _resource_erasure_apply_redaction updates, by the
    --    same predicate. The outcome text is counts and verbs only: never a title, URL, key,
    --    value or any other content (goal §8 — the record never repeats what it erased). A
    --    table the act reaches nothing in is not claimed; the husk is always reached. execute
    --    appends the blob strikes and the ended ingest to this list. ─────────────────────────
    -- D2 step 1: chunk prose and heading trails.
    SELECT count(*) INTO v_a FROM kb_chunk_content cc
      JOIN kb_chunks c ON c.id = cc.chunk_id
     WHERE c.resource_id = p_resource AND cc.content <> '';
    SELECT count(*) INTO v_b FROM kb_chunks
     WHERE resource_id = p_resource AND header_path IS NOT NULL;
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_chunk_content',
            'outcome', v_a || ' chunk bodies emptied, hashes kept; '
                       || v_b || ' heading paths nulled');
    END IF;
    -- D2 step 2: every revision's bytes.
    SELECT count(*) INTO v_a FROM kb_block_content bc
     WHERE bc.block_revision_id IN (
           SELECT br.id FROM kb_block_revisions br
             JOIN kb_content_blocks b ON b.id = br.block_id
            WHERE b.resource_id = p_resource)
       AND bc.content <> '';
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_block_content',
            'outcome', v_a || ' block revision bodies emptied, hashes kept');
    END IF;
    -- D2 step 3: embeddings with their provenance.
    SELECT count(*) INTO v_a FROM kb_chunks
     WHERE resource_id = p_resource AND embedding IS NOT NULL;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_chunks.embedding',
            'outcome', v_a || ' embeddings nulled with embedded_with');
    END IF;
    -- D2 step 4: the search vector.
    SELECT count(*) INTO v_a FROM kb_resource_search_index
     WHERE resource_id = p_resource AND search_vector <> '';
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_resource_search_index',
            'outcome', v_a || ' search vector emptied');
    END IF;
    -- D2 step 5: every artifact's content.
    SELECT count(*) INTO v_a FROM kb_data_artifact_content dac
     WHERE dac.artifact_id IN (
           SELECT da.id FROM kb_data_artifacts da WHERE da.resource_id = p_resource)
       AND dac.content <> '{}'::jsonb;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_data_artifact_content',
            'outcome', v_a || ' artifact bodies emptied to {}, hashes kept');
    END IF;
    -- D2 step 6: formation watermarks.
    SELECT count(*) INTO v_a FROM kb_contexts c
     WHERE c.shape_materialized_event_id IS NOT NULL
       AND c.id = (SELECT h.anchor_id FROM kb_resource_homes h
                    WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts');
    SELECT count(*) INTO v_b FROM kb_cogmaps m
     WHERE m.shape_materialized_event_id IS NOT NULL
       AND m.id IN (
           SELECT r.home_anchor_id FROM kb_cogmap_regions r
             JOIN kb_cogmap_region_members mem ON mem.region_id = r.id
            WHERE r.home_anchor_table = 'kb_cogmaps' AND NOT r.is_folded
              AND mem.member_table = 'kb_resources' AND mem.member_id = p_resource);
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'formation watermarks',
            'outcome', v_a || ' context and ' || v_b
                       || ' cogmap shape_materialized_event_id nulled');
    END IF;
    -- D2 step 6 (7c, 7d): region centroids holding R — live ones recomputed from the remaining
    --    members (the zero vector when R was alone), folded ones zeroed. Each count is its act
    --    statement's predicate, read before the act (see the migration header on concurrent
    --    materializes). Neither statement skips an already-cleared row, so on an erased resource
    --    this row still claims the folded regions its member rows point at.
    SELECT count(*) FILTER (WHERE NOT r.is_folded), count(*) FILTER (WHERE r.is_folded)
      INTO v_a, v_b
      FROM kb_cogmap_regions r
     WHERE EXISTS (SELECT 1 FROM kb_cogmap_region_members mem
                    WHERE mem.region_id = r.id
                      AND mem.member_table = 'kb_resources' AND mem.member_id = p_resource);
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_cogmap_regions.centroid',
            'outcome', v_a || ' live region centroids recomputed from the remaining members; '
                       || v_b || ' folded region centroids zeroed');
    END IF;
    -- D2 step 6 (7b): the home context's telos snapshot.
    SELECT count(*) INTO v_a FROM kb_contexts c
     WHERE c.telos_centroid IS NOT NULL
       AND c.id = (SELECT h.anchor_id FROM kb_resource_homes h
                    WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts');
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_contexts.telos_centroid',
            'outcome', v_a || ' context telos snapshot nulled');
    END IF;
    -- D2 step 7: workflow jobs, every status.
    SELECT count(*) FILTER (WHERE j.payload <> '{}'::jsonb OR j.last_error IS NOT NULL),
           count(*) FILTER (WHERE j.status IN ('pending', 'waiting_for_retry', 'in_progress'))
      INTO v_a, v_b
      FROM kb_workflow_jobs j WHERE j.resource_id = p_resource;
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_workflow_jobs',
            'outcome', v_a || ' jobs emptied of payload and last_error; '
                       || v_b || ' unfinished jobs cancelled to dead');
    END IF;
    -- D2 step 7a: the ingestion record and the verdicts.
    SELECT count(*) INTO v_a FROM kb_ingestion_records ir
     WHERE ir.resource_id = p_resource AND ir.source_uri <> 'erased:' || p_resource::text;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_ingestion_records.source_uri',
            'outcome', v_a || ' ingestion record source_uri sentineled, source_hash kept');
    END IF;
    SELECT count(*) INTO v_a FROM kb_data_artifact_verdicts v
     WHERE v.artifact_id IN (
           SELECT da.id FROM kb_data_artifacts da WHERE da.resource_id = p_resource)
       AND v.detail IS NOT NULL;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_data_artifact_verdicts.detail',
            'outcome', v_a || ' verdict details nulled');
    END IF;
    -- D2 step 8: projected citation-audit reasons.
    SELECT count(*) INTO v_a FROM kb_citation_audits ca
     WHERE ca.block_id IN (
           SELECT b.id FROM kb_content_blocks b WHERE b.resource_id = p_resource)
       AND ca.reason IS NOT NULL;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_citation_audits.reason',
            'outcome', v_a || ' audit reasons nulled');
    END IF;
    -- D2 step 9: the husk (always), the property sentinels, the edges, the remote sources.
    v_targets := v_targets || jsonb_build_object('target', 'kb_resources',
        'outcome', '1 husk: title and origin_uri sentineled, is_active cleared, erased_at set');
    SELECT count(*) INTO v_a FROM kb_properties p
      JOIN _resource_erasure_key_numbers('kb_resources', p_resource) k
        ON k.property_key = p.property_key
     WHERE p.owner_table = 'kb_resources' AND p.owner_id = p_resource;
    SELECT count(*) INTO v_b
      FROM kb_edges e
     CROSS JOIN LATERAL _resource_erasure_key_numbers('kb_edges', e.id) k
      JOIN kb_properties p
        ON p.owner_table = 'kb_edges' AND p.owner_id = e.id AND p.property_key = k.property_key
     WHERE (e.source_table = 'kb_resources' AND e.source_id = p_resource)
        OR (e.target_table = 'kb_resources' AND e.target_id = p_resource);
    SELECT count(*) INTO v_c
      FROM kb_content_blocks b
     CROSS JOIN LATERAL _resource_erasure_key_numbers('kb_content_blocks', b.id) k
      JOIN kb_properties p
        ON p.owner_table = 'kb_content_blocks' AND p.owner_id = b.id
       AND p.property_key = k.property_key
     WHERE b.resource_id = p_resource;
    IF v_a + v_b + v_c > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_properties',
            'outcome', v_a || ' resource-owned, ' || v_b || ' edge-owned and ' || v_c
                       || ' block-owned rows: keys erased-key-<n>, values erased, folded');
    END IF;
    SELECT count(*) INTO v_b FROM kb_edges e
     WHERE e.label IS NOT NULL
       AND ((e.source_table = 'kb_resources' AND e.source_id = p_resource)
         OR (e.target_table = 'kb_resources' AND e.target_id = p_resource));
    IF v_n_edges + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_edges',
            'outcome', v_n_edges || ' edges folded by this act; ' || v_b || ' labels nulled');
    END IF;
    --    The remote sources, from the ONE capture step (9e) reads
    --    (_resource_erasure_remote_originals): the provenance rows it re-points, the exclusive
    --    originals it deletes, the shared ones it keeps. An exclusive original that is itself
    --    some captured (block, n)'s sentinel row is re-pointed onto, so it stays cited and is
    --    not deleted; the count leaves it out for that reason.
    WITH o AS MATERIALIZED (
        SELECT * FROM _resource_erasure_remote_originals(p_resource)
    )
    SELECT (SELECT count(*) FROM kb_block_provenance bp
              JOIN o ON o.block_id = bp.block_id AND o.source_id = bp.source_id
             WHERE bp.source_kind = 'remote'),
           (SELECT count(DISTINCT o1.source_id) FROM o o1
              JOIN kb_remote_sources rs ON rs.id = o1.source_id
             WHERE NOT o1.shared
               AND NOT EXISTS (
                   SELECT 1 FROM o o2
                    WHERE normalize_remote_uri('erased:' || o2.block_id::text || ':' || o2.n::text)
                          = rs.uri_normalized)),
           (SELECT count(DISTINCT o1.source_id) FROM o o1 WHERE o1.shared)
      INTO v_c, v_a, v_b;
    IF v_c > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_block_provenance',
            'outcome', v_c || ' remote provenance rows re-pointed to erased:<block_id>:<n> sentinels');
    END IF;
    IF v_a + v_b > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_remote_sources',
            'outcome', v_a || ' exclusive remote sources deleted; '
                       || v_b || ' shared remote sources kept, named in the remainder');
    END IF;
    -- D2 step (9f): the artifact families.
    SELECT count(*) INTO v_a FROM kb_data_artifacts da
     WHERE da.resource_id = p_resource
       AND da.artifact_kind <> 'erased:' || da.asserted_by_event_id::text;
    IF v_a > 0 THEN
        v_targets := v_targets || jsonb_build_object('target', 'kb_data_artifacts.artifact_kind',
            'outcome', v_a || ' artifact families set to erased:<asserted_by_event_id>');
    END IF;

    -- ── THE REFUSAL FACE (D5), computed here so execute consumes the verdict rather than
    -- re-deriving it: a charter resource (Q2 — the map-grain act is another task, named), an
    -- already-erased resource. The ingest state is reported, not refused: an in-flight ingest
    -- ends with the erasure and execute names it in `targets`. ─────────────────────────────
    SELECT c.telos_resource_id INTO v_charter_of FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource;
    SELECT r.ingest_state INTO v_ingest FROM kb_resources r WHERE r.id = p_resource;

    -- ── THE REMAINDER (D8). Each entry is the named, never-struck part. The four shapes: ─────

    -- 1. Blobs related to the resource, live or struck, through a relation edge. The act folds
    --    their edges and names each blob; it strikes ONLY what the operator listed
    --    (also_strike_blobs), per row through blob_delete('blob_erased', …) and the byte-delete
    --    fence. It never infers a strike from a relation.
    FOR v_row IN
        SELECT DISTINCT b.id, b.content_hash, b.blob_pathname,
               EXISTS (SELECT 1 FROM kb_blobs live
                        WHERE live.id = b.id AND live.content_type IS NOT NULL) AS is_live
          FROM kb_blobs b
          JOIN kb_edges e ON NOT e.is_folded
               AND ((e.source_table = 'kb_blobs' AND e.source_id = b.id
                      AND e.target_table = 'kb_resources' AND e.target_id = p_resource)
                 OR (e.target_table = 'kb_blobs' AND e.target_id = b.id
                      AND e.source_table = 'kb_resources' AND e.source_id = p_resource))
         ORDER BY b.id
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'kb_blobs',
            'outcome', 'related blob ' || v_row.id::text || '; hash ' || v_row.content_hash
                       || '; ' || CASE WHEN v_row.is_live THEN 'live; struck only when the operator lists it'
                                            ELSE 'already struck' END);
    END LOOP;

    -- 2. Derivers: resources with a live `derived_from` edge INTO R, or block provenance citing R.
    --    Named by id, never touched; their citing provenance rows are ids only (no text).
    FOR v_row IN
        SELECT DISTINCT e.source_id AS deriver_id
          FROM kb_edges e
         WHERE e.is_folded = false AND e.edge_kind = 'leads_to'
           AND e.label = 'derived_from'
           AND e.target_table = 'kb_resources' AND e.target_id = p_resource
           AND e.source_table = 'kb_resources'
        UNION
        SELECT DISTINCT b.resource_id
          FROM kb_block_provenance q
          JOIN kb_content_blocks b ON b.id = q.block_id
         WHERE q.source_kind = 'resource' AND q.source_id = p_resource
     LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'deriver',
            'outcome', 'resource ' || v_row.deriver_id::text
                       || ' holds a structural lead (' ||
                       CASE WHEN EXISTS (SELECT 1 FROM kb_edges e
                                          WHERE e.is_folded = false AND e.edge_kind = 'leads_to'
                                            AND e.label = 'derived_from'
                                            AND e.target_table = 'kb_resources' AND e.target_id = p_resource
                                            AND e.source_table = 'kb_resources' AND e.source_id = v_row.deriver_id)
                            THEN 'derived_from edge'
                            ELSE 'provenance citation' END
                       || '); never touched; discovery-bound');
    END LOOP;

    -- 3. Cross-resource ledger text that quotes R (Q1, listed only): events of a quoting type
    --    whose payload text names R's id AND whose subject arm is NOT R's own. A
    --    citation_audited event ON one of R's blocks is R's own trail (step (6) reached its
    --    projected column; its ledger reason rides R's ledger remainder below) — this arm is
    --    only the events anchored elsewhere. Structurally: the payload text contains R's id,
    --    and no block of R carries the payload's block_id. Named by event id, never redacted —
    --    the exception never crosses the resource boundary (Q1).
    FOR v_row IN
        SELECT DISTINCT ev.id, et.name
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE et.name IN ('citation_audited', 'subscription_delivery_disposed',
                           'invocation_closed')
           AND _resource_erasure_quotes(ev.payload::text, p_resource)
           AND NOT EXISTS (
               SELECT 1 FROM kb_content_blocks b
                WHERE b.resource_id = p_resource
                  AND b.id = (ev.payload->>'block_id')::uuid)
           AND NOT EXISTS (
               SELECT 1
                 FROM _resource_erasure_trail_scope(p_resource) t
                WHERE t.event_id = ev.id)
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'cross-resource ledger text',
            'outcome', 'event ' || v_row.id::text || ' (' || v_row.name
                       || ') may quote the resource; listed only (Q1), never redacted');
    END LOOP;

    -- 4. Shared remote sources: kb_remote_sources rows R's blocks cite that ANOTHER
    --    resource's block also cites (live or folded — the row is shared either way), read from
    --    the ONE capture the act's step (9e) re-points from (_resource_erasure_remote_originals).
    --    A source R exclusively cites is NOT here: step (9e) deletes it, and replay of the
    --    redacted payloads never mints it, so the projection agrees by construction. A shared
    --    one stays and is named by its kb_remote_sources id, never by its URL (D4, D8): the
    --    record is an admin event outside the trail scope, so nothing could ever redact a URL
    --    written into it.
    FOR v_row IN
        SELECT DISTINCT o.source_id
          FROM _resource_erasure_remote_originals(p_resource) o
         WHERE o.shared
         ORDER BY o.source_id
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'kb_remote_sources.id',
            'outcome', 'shared remote source ' || v_row.source_id::text
                       || '; another resource''s block still cites it; named, kept');
    END LOOP;

    -- 5. Subscription deliveries that may quote R (Q1, listed only; ruled 2026-10-07). A delivery
    --    is projected only by webhook intake, so it carries three texts: the remote's body (its
    --    event's payload, bare at version 1, under `body` at 2), `rationale` (copied onto
    --    subscription_delivery_disposed, which arm 3 lists by event) and `scope_reason` (a
    --    caller's text, never on the ledger, so no event id can name it). A delivery is named,
    --    by its id, when any of the three quotes R's id. Never redacted.
    FOR v_row IN
        SELECT d.id
          FROM kb_subscription_deliveries d
          JOIN kb_events ev ON ev.id = d.event_id
         WHERE _resource_erasure_quotes(d.rationale, p_resource)
            OR _resource_erasure_quotes(d.scope_reason, p_resource)
            OR _resource_erasure_quotes(ev.payload::text, p_resource)
         ORDER BY d.id
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'kb_subscription_deliveries',
            'outcome', 'delivery ' || v_row.id::text
                       || ' may quote the resource in its webhook body, rationale or scope_reason; listed only (Q1), never redacted');
    END LOOP;

    -- 6. Shapes in R's home for a family R's artifacts used (D4, ruled 2026-10-03, ruling 5;
    --    20261008100000). A shape's artifact_kind is the home's declaration of a family, not R's
    --    instance, so the act never touches it (Q1), and it may still name what R's artifacts
    --    were. Named by shape id, live shapes only. Matched on (home, kind owner, family), the
    --    family read from the artifact's row OR from the data_artifact_committed event that
    --    asserted it: step (9f) erases the row's family, so a plan computed on the husk (the
    --    survey after the act, a cut-1 husk the backfill below rewrote, cut 2's completion pass
    --    re-deriving) still finds the shapes through the ledger until cut 2 redacts that payload.
    --    Only R's current home: a shape in a home R was rehomed out of is not named.
    FOR v_row IN
        SELECT DISTINCT s.id
          FROM kb_data_artifacts da
          JOIN kb_events ev ON ev.id = da.asserted_by_event_id
          JOIN kb_resource_homes h ON h.resource_id = da.resource_id
          JOIN kb_data_artifact_shapes s
               ON NOT s.is_folded
              AND s.home_anchor_table = h.anchor_table AND s.home_anchor_id = h.anchor_id
              AND s.kind_owner_table = da.kind_owner_table AND s.kind_owner_id = da.kind_owner_id
              AND (s.artifact_kind = da.artifact_kind
                OR s.artifact_kind = ev.payload ->> 'artifact_kind')
         WHERE da.resource_id = p_resource
         ORDER BY s.id
    LOOP
        v_remainder := v_remainder || jsonb_build_object(
            'target', 'kb_data_artifact_shapes',
            'outcome', 'shape ' || v_row.id::text
                       || ' in the home declares a family the resource''s artifacts used; the home''s, never touched (Q1)');
    END LOOP;

    -- R's home context, whether R was ever a goal, and R's genesis — inputs to the telos arm
    -- below. Goal-ness is read from the LEDGER, not from kb_properties: step 9 sentinels and folds
    -- R's doc_type row, so a plan computed after the act (the survey on the husk, cut 2's
    -- completion pass re-deriving) would otherwise drop every telos copy without error. "Ever" —
    -- the created doc_type or any doc_type property event, normalized as the projection
    -- normalizes it — so a goal later re-typed still names the snapshots it contributed to.
    -- Cut 2 must read this before it redacts R's trail, or from the recorded resource_erased: both
    -- arms read paths this plan's own CASE names for redaction (`property_key` and `value`, and
    -- since 20261008100000 the created arm's `doc_type` too).
    SELECT h.anchor_id INTO v_home_ctx FROM kb_resource_homes h
     WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts';
    v_is_goal := EXISTS (
        SELECT 1 FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE et.name = 'resource_created'
           AND (ev.payload ->> 'resource_id')::uuid = p_resource
           AND ev.payload ->> 'doc_type' = 'goal')
      OR EXISTS (
        SELECT 1 FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE et.name IN ('property_set', 'property_asserted')
           AND (ev.payload -> 'owner') ->> 'table' = 'kb_resources'
           AND ((ev.payload -> 'owner') ->> 'id')::uuid = p_resource
           AND ev.payload ->> 'property_key' = 'doc_type'
           AND _property_value_normalized('doc_type', ev.payload -> 'value') #>> '{}' = 'goal')
      -- After cut 2 the act redacts both arms above, so a plan on the husk reads goal-ness from
      -- the act's own record: it named telos copies only when R was a goal (20261009100000).
      OR EXISTS (
        SELECT 1 FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         CROSS JOIN LATERAL jsonb_array_elements(ev.payload -> 'ledger_remainder') lr
         WHERE et.name = 'resource_erased' AND ev.payload ->> 'subject_id' = p_resource::text
           AND lr -> 'paths' ? 'telos_centroid');
    SELECT ev.id INTO v_genesis
      FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE et.name = 'resource_created' AND (ev.payload ->> 'resource_id')::uuid = p_resource
     ORDER BY ev.id
     LIMIT 1;
    SELECT ev.id INTO v_erased
      FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE et.name = 'resource_erased' AND ev.payload ->> 'subject_id' = p_resource::text
     ORDER BY ev.id
     LIMIT 1;

    -- ── THE LEDGER REDACTION (D3, D4; 20261009100000): every path of R's own trail events the act
    --    rewrites to its class sentinel, in exactly the shape `RedactedEventFields` uses —
    --    {event, paths}. One definition, _resource_erasure_payload_redaction, which the act runs
    --    for the payloads it writes. A path already at its sentinel is not listed, so on a husk
    --    erased under cut 2 this is empty. On a husk erased under cut 1 it is NOT yet the
    --    completion pass's list: step (9e) has re-pointed that husk's provenance, so the remote-
    --    source numbering this reads finds no original and the derivation raises. The completion
    --    pass must number those sources from the ledger. Paths only, never values.
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', d.event_id, 'paths', d.paths)
                               ORDER BY d.event_id), '[]'::jsonb)
      INTO v_redacted
      FROM _resource_erasure_payload_redaction(p_resource) d;

    -- ── THE LEDGER REMAINDER (D12): the ledger paths carrying R's content that the exception
    --    cannot reach, in the same {event, paths} shape. Since cut 2 the act reaches R's whole
    --    trail, so what remains is the telos copies below, anchored on the context (D3's
    --    exception never crosses the resource boundary).
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', s.event_id, 'paths', s.paths)
                               ORDER BY s.event_id), '[]'::jsonb)
      INTO v_ledger
      FROM (
        -- ── The telos copies (ruled 2026-10-04): `ledger_remainder` names the ledger paths
        --    carrying R's content, not only R's own trail. When R was ever a goal, every telos
        --    snapshot its current home context's materializations and salience refreshes recorded
        --    may average R's embedding in. Bounded by R's genesis below and, once R is erased,
        --    by its `resource_erased` above (ruled 2026-10-04): later snapshots are the context's
        --    own history, and a re-derive must name the same set the act recorded. Not named: a
        --    snapshot that a materialize already in flight at the act minted after it (loaded
        --    before the act, so its telos may still average R in; the region drain repairs the
        --    stored snapshot, not that ledger copy), and a former home's snapshots. Named, never reached here: the ledger edit is
        --    cut 2's. Bounded below by R's genesis — R contributed nothing before it existed — and
        --    otherwise over-inclusive by design: a snapshot minted while R was done or unembedded
        --    is named too. Not in the trail scope (anchored on the context, not on R), so no
        --    event appears twice. ────────────────────────────────────────────────────────────
        SELECT ev.id AS event_id, '["telos_centroid"]'::jsonb AS paths
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE v_is_goal
           AND et.name IN ('region_materialized', 'salience_refreshed')
           AND ev.producing_anchor_table = 'kb_contexts' AND ev.producing_anchor_id = v_home_ctx
           AND (v_genesis IS NULL OR ev.id > v_genesis)
           AND (v_erased IS NULL OR ev.id < v_erased)
           AND jsonb_typeof(ev.payload -> 'telos_centroid') = 'string'
       ) s;

    RETURN jsonb_build_object(
        'resource',        p_resource,
        'n_blocks',        v_n_blocks,
        'n_revisions',     v_n_revisions,
        'n_chunks',        v_n_chunks,
        'n_artifacts',     v_n_artifacts,
        'n_edges',         v_n_edges,
        'edges',           v_edges,
        'targets',         v_targets,
        'already_erased',  (SELECT r.erased_at IS NOT NULL FROM kb_resources r WHERE r.id = p_resource),
        'charter_of',      v_charter_of,
        'ingest_state',    v_ingest,
        'fingerprint_available',
            to_regproc('sensitivity.deriver_fingerprint_matches') IS NOT NULL,
        'remainder',       v_remainder,
        'redacted_fields', v_redacted,
        'ledger_remainder', v_ledger);
END;
$function$
;

-- ---------------------------------------------------------------------------
-- Section 8. The act writes the redaction.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.resource_erasure_execute(p_resource uuid, p_operator uuid, p_emitter uuid, p_request_ref uuid, p_also_strike_blobs uuid[] DEFAULT '{}'::uuid[])
 RETURNS jsonb
 LANGUAGE plpgsql
 SET search_path = public, pg_temp
AS $function$
DECLARE
    v_plan    jsonb;
    v_charter uuid;
    v_ingest  text;
    v_erased  boolean;
    v_erased_ts timestamptz;
    v_found   boolean;
    v_id      uuid;
    v_ev      uuid;
    v_targets jsonb;
    v_remainder jsonb;
    v_edges   jsonb;
    v_i       integer;
    v_eid     uuid;
    v_bid     uuid; v_rel boolean; v_path text;
    v_ledger  jsonb := '[]'::jsonb;
    v_redact  jsonb;
    v_fields  jsonb;
    v_payload jsonb;
BEGIN
    IF p_resource IS NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: p_resource is required';
    END IF;
    -- The request reference is the act's correlation id: every event the act appends carries
    -- it, and replay finds the act's span by it (D14). Without one, _event_append correlates
    -- each event to itself and the span is lost.
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: p_request_ref is required';
    END IF;

    -- ── THE REFUSAL VERDICTS, read PRE-act (D5). A refusal here RAISES — the Rust caller
    --    catches the typed message and records it through resource_erasure_refuse, so the SQL
    --    never silently widens the negative face to a partial act (and never appends a refusal
    --    event itself: _event_append's emitter is the OPERATOR and a refused attempt at SQL
    --    grain would attribute wrongly). The verdict reads happen before anything mutates, and
    --    under R's row lock (D13), taken right after the existence check: FOR UPDATE waits out
    --    every writer already holding the row (their FK KEY SHARE, the write guard's KEY SHARE,
    --    the body-hash recompute's NO KEY UPDATE) so the plan, the folds and the body all see one
    --    settled state, and a writer arriving after it waits on the lock, then refuses at the
    --    write guard (Section W). The same lock serializes a second execute: its verdict read
    --    runs after the first commits, sees `erased_at`, and raises `already erased` rather than
    --    both passing the reads and double-completing.
    --    PR 2's service parses these strings into refusal vocabulary. Two states are NOT
    --    refusals (D5): an in-flight ingest (below, where `targets` names it) and a tombstone
    --    (the paragraph after the verdicts). ──
    SELECT count(*) > 0 INTO v_found FROM kb_resources r WHERE r.id = p_resource;
    IF NOT v_found THEN
        RAISE EXCEPTION 'resource_erasure_execute: resource % not found', p_resource;
    END IF;
    PERFORM 1 FROM kb_resources WHERE id = p_resource FOR UPDATE;
    -- R's captured original remote sources, locked BEFORE the plan reads them, in uuid order (the
    -- order step (9e) locks them in, so two acts sharing originals cannot deadlock). A citer whose
    -- _upsert_remote_source already holds one of these rows makes the act wait for its commit, so
    -- the plan's shared/exclusive split and step (9e)'s delete decision read the same citers; a
    -- citer arriving later waits on the act, and if the act deleted the row, its upsert inserts
    -- the URL as a fresh row.
    PERFORM 1 FROM kb_remote_sources rs
     WHERE rs.id IN (SELECT o.source_id FROM _resource_erasure_remote_originals(p_resource) o)
     ORDER BY rs.id
       FOR UPDATE;
    SELECT c.telos_resource_id INTO v_charter FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource;
    SELECT r.erased_at INTO v_erased_ts
      FROM kb_resources r WHERE r.id = p_resource;
    v_erased := v_erased_ts IS NOT NULL;

    IF v_charter IS NOT NULL THEN
        RAISE EXCEPTION 'resource_erasure_execute: charter resource (map-grain erasure is filed task 01a0e960-0ca2-7f42-b33e-1ed19b024e6b)';
    END IF;
    IF v_erased THEN
        RAISE EXCEPTION 'resource_erasure_execute: already erased';
    END IF;
    -- A TOMBSTONE IS ERASABLE: a soft-deleted resource is not a refusal. It is arguably the
    -- flow's most common shape — the content was soft-deleted, and the compliance need then
    -- arrives that demands it not exist at all. The principal act has no tombstone refusal
    -- (it tombstones VIA the act, 20260909000025), F4's write floor makes is_active=false
    -- already permanent, and no restore verb exists (the spec's F4 — "un-modifiable on every
    -- axis"), so erasure is the only way out of a tombstone. The act completes over one
    -- mechanically: is_active is already false, the CHECK is satisfied, and COALESCE keeps
    -- erased_at stable. D6's rule ("a soft-deleted resource must never be mistaken for an
    -- erased one") is a PROJECTION honesty rule — erased_at is NULL until this act sets it. ──

    -- ── The ONE computation (D10). No re-enumeration of the remainder, block counts, artifact
    --    counts or edges happens below — the plan computed them once. The would_strike entries
    --    the principal plan's shape used do not exist here: the survey names related blobs via
    --    the remainder, and the operator's `also_strike_blobs` arrives AT THE ACT (D8), where
    --    the strike loop below consumes it. ──────────────────────────────────────────────────
    v_plan := resource_erasure_survey_plan(p_resource);
    v_targets := v_plan->'targets';
    v_ingest := v_plan->>'ingest_state';

    -- ── The ledger redaction (D3, D4; 20261009100000), derived NOW, before the fold events
    --    below exist and before the body's step 9 rewrites the projection: the same function the
    --    plan lists redacted_fields from, under R's lock, with the payloads it will write. ──
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', d.event_id, 'payload', d.new_payload,
                                                 'metadata', d.new_metadata, 'paths', d.paths)
                               ORDER BY d.event_id), '[]'::jsonb)
      INTO v_redact
      FROM _resource_erasure_payload_redaction(p_resource) d;
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', r->'event', 'paths', r->'paths')
                               ORDER BY r->>'event'), '[]'::jsonb)
      INTO v_fields
      FROM jsonb_array_elements(v_redact) r;

    -- ── The operator-listed blob strikes, through the wrapper, PER ROW (D8: a blob is struck
    --    ONLY when the operator listed it; the act never infers a strike from a relation). Each
    --    strike carries the wrapper's verdict at ITS OWN moment — the byte-delete fence runs
    --    inside — and its prose template is the ONE the fence parses by exact prefix. The plan
    --    itself predicts nothing here, because the operator's list arrives at the act, not at
    --    the survey (the survey names related blobs; the operator answers with the subset to
    --    strike).
    --
    --    A listed blob the plan did NOT name is refused, not silently struck: the survey is the
    --    reviewed record of what the act may reach, and an operator widening it mid-act is a
    --    drift the fence exists to catch. ─────────────────────────────────────────────────────
    FOR v_i IN 0 .. coalesce(array_upper(p_also_strike_blobs, 1), 0) - 1 LOOP
        v_bid := p_also_strike_blobs[v_i + 1];
        IF NOT EXISTS (
            SELECT 1 FROM jsonb_array_elements(v_plan->'remainder') rem
             WHERE rem->>'target' = 'kb_blobs'
               AND rem->>'outcome' LIKE '%blob ' || v_bid::text || ';%') THEN
            RAISE EXCEPTION 'resource_erasure_execute: blob % is not in the survey''s related-blob remainder; strike refused', v_bid;
        END IF;
        SELECT blob_id, released, pathname
          INTO v_bid, v_rel, v_path
          FROM blob_delete('blob_erased',
                           jsonb_build_object('blob_id', v_bid),
                           p_emitter,
                           p_correlation => p_request_ref);
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_blobs',
            'outcome', blob_strike_outcome_text(v_rel, v_path)));
    END LOOP;

    -- ── Ingest state is not a refusal (D5): a partial or in-flight ingest ends with the
    --    erasure. The husk keeps its ingest_state; erased_at is authoritative, and the write
    --    floor (Section W) refuses any later attempt to continue the ingest. The record names
    --    the ingest the act ended. ─────────────────────────────────────────────────────────
    IF v_ingest = 'in_progress' THEN
        v_targets := v_targets || jsonb_build_array(jsonb_build_object(
            'target',  'kb_resources.ingest_state',
            'outcome', 'ingest ' || v_ingest || '; ended by erasure; erased_at is authoritative'));
    END IF;

    -- ── Per-edge folds: ONE relationship_folded per edge touching R (D1 — the incumbent verb,
    --    its OWN trail shows who ended it and why, another principal's view reads as
    --    deliberately ended; replay folds through the existing projector). reason is a FIXED
    --    literal 'resource_erased', never operator prose. Each event carries the act's
    --    correlation id (the request reference), so the act's pairing is a fact, not a
    --    convention. Edges are folded FIRST (the projected is_folded) so the plan's edge arm and
    --    the fold events agree in the same transaction.
    v_edges := v_plan->'edges';
    FOR v_i IN 0 .. jsonb_array_length(v_edges) - 1 LOOP
        v_eid := (v_edges->>v_i)::uuid;
        SELECT id INTO v_id FROM kb_edges WHERE id = v_eid AND NOT is_folded FOR UPDATE;
        IF v_id IS NULL THEN
            RAISE EXCEPTION 'resource_erasure_execute: edge % missing or already folded', v_eid;
        END IF;
        v_ev := _event_append('relationship_folded', p_emitter,
                              (SELECT home_anchor_table FROM kb_edges WHERE id = v_eid),
                              (SELECT home_anchor_id FROM kb_edges WHERE id = v_eid),
                              jsonb_build_object(
                                  'edge_id', v_eid,
                                  'reason', 'resource_erased'),
                              p_correlation => p_request_ref);
        PERFORM _project_relationship_folded(v_ev, jsonb_build_object(
            'edge_id', v_eid,
            'reason', 'resource_erased'));
    END LOOP;

    -- ── Projection-side sentinels + the content sweep — THE one body, at the act's event
    --    position. No events inside; the appended event below is the record. ─────────────────
    v_payload := jsonb_build_object(
            'subject_table', 'kb_resources',
            'subject_id', p_resource,
            'actor', p_operator,
            'redacted_fields', v_fields,
            'folded_edges', v_edges,
            'targets', v_targets,
            'remainder', v_plan->'remainder',
            'ledger_remainder', v_plan->'ledger_remainder');
    v_ev := _event_append(
        'resource_erased', p_emitter, NULL, NULL, v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_resources','id', p_resource)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    -- ── The ledger exception, in D3's order: the event is appended and its projector records
    --    each (event, path) it names. The redaction body runs next, over the ledger as it was:
    --    step (9e)'s capture reads each remote source's position in its event's list, which the
    --    rewrite would erase, and it marks the subject erased, which the verifier requires. Only
    --    then are the events rewritten, in one statement; kb_events_append_only admits each row
    --    against those rows, this transaction's erasure and the class sentinels, and
    --    kb_events_redaction_in_trail checks the trail once for the statement. ─────────────
    PERFORM _project_resource_erased_redactions(v_ev, v_payload);

    PERFORM _resource_erasure_apply_redaction(p_resource, v_ev);

    UPDATE kb_events e
       SET payload  = r->'payload',
           metadata = r->'metadata'
      FROM jsonb_array_elements(v_redact) r
     WHERE e.id = (r->>'event')::uuid;

    RETURN jsonb_build_object(
        'event_id',        v_ev,
        'edges',           v_edges,
        'targets',         v_targets,
        'remainder',       v_plan->'remainder',
        'redacted_fields', v_fields,
        'ledger_remainder', v_plan->'ledger_remainder');
END;
$function$
;

SELECT declare_migration(
    20261009100000,
    'additive',
    'New: the table kb_event_field_redactions (append-only, its own trigger) and the functions _erasure_redact_paths, _erasure_path_class, _erasure_expand, _erasure_location_block, _erasure_location_is_remote, _erasure_sentinel_exact, _erasure_label_is_kept, _erasure_sentinel_admits, _project_resource_erased_redactions, _resource_erasure_ledger_key_numbers, _resource_erasure_facet_numbers, _resource_erasure_label_numbers and _resource_erasure_payload_redaction. CREATE OR REPLACE, with unchanged signatures and return types, of kb_events_append_only (now admitting an UPDATE that is a resource erasure''s sentinel redaction at paths a kb_event_field_redactions row names and the authorizing resource_erased event itself lists, on an event of that erasure''s subject''s trail; DELETE and every other UPDATE still raise), _resource_erasure_trail_scope (the same set as a UNION of indexed arms), resource_erasure_survey_plan (adds a redacted_fields key; ledger_remainder keeps only the telos copies) and resource_erasure_execute (writes redacted_fields and rewrites the trail''s events). A binary that predates this ignores the plan''s new key, and redacted_fields already exists on resource_erased, omitted when empty. No existing column, constraint or grant changes.'
);
