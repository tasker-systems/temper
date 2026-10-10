-- The sweep closes a field scrub's findings, and reads each scrubbable family's flags (field-grain
-- scrub, build Task 6).
--
-- Spec: temper-artifacts specs/2026-10-09-field-grain-scrub-design.md S1, S2, S7, S9 and witness 5,
-- read beside specs/2026-09-28-resource-erasure-design.md D10 and its *Rulings from build order 3c*.
-- Plan: plans/2026-10-09-field-grain-scrub.md, Task 6.
--
-- 20261018100010 gave the ledger exception a second authority, resource_scrubbed, and
-- 20261018100020 is the act that writes under it. Unchanged, the sweep would never close a scrubbed
-- finding (S9): place_closure's ledger arm counts a redaction row only under a resource_erased with
-- an erased subject, and its projection arm knows only erasure's sentinels.
--
--   1. sensitivity.place_closure, CREATE OR REPLACE with its signature unchanged, so the
--      finding_closure view and its three callers (erased_place_findings,
--      expire_erased_fingerprints, close_cardless_card_findings) are untouched:
--        * the ledger arm accepts a row of either authority, each under its own record and
--          precondition, still without the verifier's `created = now()` (closure reads long after
--          the act);
--        * the projection arm adds the scrub's sentinels (S7): scrubbed-<resource id> for a cleared
--          title or origin URI (S2), a scrubbed-key-<handle> key, a folded value at its asserting
--          event's erased:<event id> (a string, or the one-element array a kept `tags` key
--          normalises it to), and a facet mark at its scrubbed-facet-<event id>-<position> key.
--   2. sensitivity.field_scrub_family_flags: per row of resource_field_scrub_families, whether the
--      family holds an open finding, whether its live place does, and whether the sweep has read
--      the resource (S1, S2). Booleans only.
--   3. resource_field_scrub_family_flags: the public door the field scrub's listing calls, since no
--      application code may name the sensitivity schema (sensitivity_schema_unreachable_test).
--   4. EXECUTE revoked from PUBLIC on both new functions.

-- ---------------------------------------------------------------------------
-- Section 1. place_closure learns the field scrub (S9).
-- ---------------------------------------------------------------------------
-- 20261011100000 Section 2's body, with two changes.
--
-- The ledger arm. A row counts only as the verifier (kb_events_append_only, 20261018100010
-- Section 6) counts it: its redacted_by is an event of its authority's type (resource_erased for
-- `erasure`, resource_scrubbed for `scrub`) that lists this event and path. Each authority has its
-- own precondition on the record's subject:
--   * erasure: the subject is erased, as before. The act sets erased_at in its own transaction, and
--     nothing clears it;
--   * scrub: the subject is a resource, erased or not. The verifier admitted the rewrite only on a
--     subject that was not erased, in the act's own transaction. A later erasure of that subject
--     leaves the scrub's sentinel where it is: the scrub writes erasure's exact sentinels for the
--     title, origin URI and values (S7), and the erasure lists only a path whose value it changes
--     (_resource_erasure_payload_redaction: `IF v_new IS DISTINCT FROM v_loc.val`), so it writes no
--     row of its own there. Requiring a live subject would reopen those findings on erasure.
-- The verifier's `created = now()` is dropped for both, as before.
--
-- The projection arm. Each scrub sentinel is the exact value the act writes
-- (_resource_field_scrub_rewrite_projection and the clearing resource_update, 20261018100020):
--   * kb_resources.title / origin_uri: scrubbed-<resource id>, the placeholder clear mode sets. A
--     title kept in keep mode is not a sentinel, and its finding stays open (S9);
--   * kb_properties.property_key: scrubbed-key-<uuid>, the shape the verifier admits under `scrub`
--     (_erasure_sentinel_admits). The handle is not derivable from the row once renamed;
--   * kb_properties.property_value: erased:<the row's asserting event id>, the sentinel
--     _erasure_sentinel_exact gives a property value or doc type, as _property_value_normalized
--     stores it: a string, or under the literal key `tags` the one-element array. Bound to the
--     row's own asserted_by_event_id, the event whose payload the act rewrote, so no other
--     `erased:` value closes a finding;
--   * kb_properties.property_value: a facet mark, the one-key object of a
--     scrubbed-facet-<uuid>-<n> inner key and "erased", the row the rewrite gives each mark (S7).
-- The doc_type placeholder `scrubbed` (S2) needs no arm: clear mode writes it into a NEW live row,
-- which no prior finding names; the folded row the clearing event leaves takes erased:<id> above.
CREATE OR REPLACE FUNCTION sensitivity.place_closure(p_surface text, p_target uuid, p_hash text, p_path text) RETURNS text
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE
        WHEN p_surface IN ('kb_events.payload', 'kb_events.metadata') THEN
            CASE WHEN EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                JOIN kb_events e ON e.id = r.redacted_by
                                JOIN kb_event_types t ON t.id = e.event_type_id
                               WHERE r.event_id = p_target
                                 AND sensitivity.ledger_path_covers(r.path, p_surface, p_path)
                                 AND t.name = CASE r.authority
                                                  WHEN 'erasure' THEN 'resource_erased'
                                                  WHEN 'scrub'   THEN 'resource_scrubbed'
                                              END
                                 AND e.payload -> 'redacted_fields' @> jsonb_build_array(jsonb_build_object(
                                         'event', p_target::text, 'paths', jsonb_build_array(r.path)))
                                 AND EXISTS (SELECT 1 FROM kb_resources s
                                              WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                                AND (r.authority = 'scrub' OR s.erased_at IS NOT NULL)))
                 THEN 'sentinel' END
        WHEN p_surface NOT IN ('kb_block_content.content', 'kb_chunk_content.content', 'kb_chunks.header_path',
                               'kb_resources.title', 'kb_resources.origin_uri', 'kb_properties.property_key',
                               'kb_edges.label', 'kb_citation_audits.reason', 'kb_remote_sources.uri',
                               'kb_properties.property_value') THEN NULL
        WHEN x.present IS NULL THEN 'row_missing'
        -- The resource erasure act's D4 sentinels (20260929040730, steps 9a, 9b, 9d).
        WHEN (p_surface = 'kb_resources.title'           AND x.unit = 'erased-' || p_target::text)
          OR (p_surface = 'kb_resources.origin_uri'      AND x.unit = 'erased:' || p_target::text)
          OR (p_surface = 'kb_properties.property_key'   AND x.unit ~ '^erased-key-[0-9]+$')
          OR (p_surface = 'kb_properties.property_value' AND x.unit = '"erased"') THEN 'sentinel'
        -- The field scrub's sentinels (field-grain scrub S2, S7; 20261018100020).
        WHEN (p_surface IN ('kb_resources.title', 'kb_resources.origin_uri')
              AND x.unit = 'scrubbed-' || p_target::text)
          OR (p_surface = 'kb_properties.property_key'
              AND x.unit ~ '^scrubbed-key-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')
          OR (p_surface = 'kb_properties.property_value'
              AND x.unit ~ '^\{"scrubbed-facet-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}-[1-9][0-9]*": "erased"\}$')
          OR (p_surface = 'kb_properties.property_value'
              AND EXISTS (SELECT 1 FROM kb_properties p
                           WHERE p.id = p_target
                             AND p.property_value IN (to_jsonb('erased:' || p.asserted_by_event_id::text),
                                                      jsonb_build_array('erased:' || p.asserted_by_event_id::text))))
            THEN 'sentinel'
        WHEN coalesce(x.unit, '') = ''
          OR (p_surface = 'kb_properties.property_value' AND x.unit IN ('{}', '[]', '""', 'null')) THEN 'content_empty'
        WHEN o.content_hash <> p_hash THEN 'changed'
    END
      FROM (SELECT 1) one
      LEFT JOIN LATERAL (
          SELECT true, bc.content FROM kb_block_content bc
           WHERE p_surface = 'kb_block_content.content' AND bc.block_revision_id = p_target
          UNION ALL
          SELECT true, cc.content FROM kb_chunk_content cc
           WHERE p_surface = 'kb_chunk_content.content' AND cc.chunk_id = p_target
          UNION ALL
          SELECT true, c.header_path FROM kb_chunks c
           WHERE p_surface = 'kb_chunks.header_path' AND c.id = p_target
          UNION ALL
          SELECT true, r.title FROM kb_resources r
           WHERE p_surface = 'kb_resources.title' AND r.id = p_target
          UNION ALL
          SELECT true, r.origin_uri FROM kb_resources r
           WHERE p_surface = 'kb_resources.origin_uri' AND r.id = p_target
          UNION ALL
          SELECT true, p.property_key FROM kb_properties p
           WHERE p_surface = 'kb_properties.property_key' AND p.id = p_target
          UNION ALL
          SELECT true, p.property_value::text FROM kb_properties p
           WHERE p_surface = 'kb_properties.property_value' AND p.id = p_target
          UNION ALL
          SELECT true, e.label FROM kb_edges e
           WHERE p_surface = 'kb_edges.label' AND e.id = p_target
          UNION ALL
          SELECT true, a.reason FROM kb_citation_audits a
           WHERE p_surface = 'kb_citation_audits.reason' AND a.id = p_target
          UNION ALL
          SELECT true, s.uri FROM kb_remote_sources s
           WHERE p_surface = 'kb_remote_sources.uri' AND s.id = p_target
      ) x (present, unit) ON true
      LEFT JOIN sensitivity.place_observations o ON o.surface = p_surface AND o.target_id = p_target;
$$;

COMMENT ON FUNCTION sensitivity.place_closure(text, uuid, text, text) IS
'How a finding''s place closes, from what it holds now (sweep D2 as amended): row_missing, sentinel, content_empty, or on a mutable surface changed by the sweep''s own later observation; NULL while it is open, and on a surface this function does not know. A ledger place (kb_events.payload, kb_events.metadata) closes as sentinel when a kb_event_field_redactions row on its event covers p_path and the row''s authorising event, of its authority''s type, lists that event and path: a resource_erased for an erased subject, or a resource_scrubbed for a resource, erased since or not (field-grain scrub S9); never otherwise. A projection place closes as sentinel on the erasure''s D4 sentinels and on the field scrub''s (S7): scrubbed-<resource id>, scrubbed-key-<uuid>, erased:<the row''s asserting event> and a scrubbed-facet mark. On a `[*]` path the row means some location under it was rewritten, not every one. p_path is read only on the ledger.';

-- ---------------------------------------------------------------------------
-- Section 2. The family flags (S1, S2).
-- ---------------------------------------------------------------------------
-- One row per row of resource_field_scrub_families(p_resource), in its order and keyed as it keys
-- them, (field, family) with family NULL for the title and origin URI: the rows are that function's
-- own, read once, so they cannot drift from the listing they annotate. Places are matched to rows
-- by the listing's ordinal, so the NULL family needs no NULL-safe comparison here; a caller joining
-- the two by (field, family) compares family with IS NOT DISTINCT FROM.
--
--   flagged          an open, countable finding (block_current_revision_flagged's test: countable,
--                    and no row in finding_closure) on any place of the field: in the ledger, a path
--                    the scrub of this field could redact on one of its events (the allowlist's
--                    paths whose class is the field's, as _resource_field_scrub_derivation reads
--                    them, matched through ledger_path_covers); in the projection, the resource's
--                    title or origin URI, or any kb_properties row, live or folded, an event of the
--                    family asserted;
--   current_flagged  the same, on today's place only: the title or origin URI, or the family's live
--                    rows. The survey's "the current value is flagged; it survives unless --clear"
--                    (S2), a support for the operator's judgement, never a replacement for it;
--   covered          whether the sweep has read all of the resource for every enabled detector
--                    (sensitivity.resource_covered). Per resource, so the same on every row. False
--                    when no detector is enabled, as resource_covered answers an empty set (R2): an
--                    unflagged family that is not covered says nothing about cleanliness.
--
-- Booleans only: no hash, category, detector, path or text leaves it (S1). SECURITY DEFINER with
-- search_path set, as block_current_revision_flagged (20261012100000 Section 4).
CREATE FUNCTION sensitivity.field_scrub_family_flags(p_resource uuid)
RETURNS TABLE (field text, family uuid, flagged boolean, current_flagged boolean, covered boolean)
LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_covered boolean;
BEGIN
    v_covered := sensitivity.resource_covered(
        p_resource, ARRAY(SELECT d.id FROM sensitivity.detectors d WHERE d.enabled ORDER BY d.id));

    RETURN QUERY
    WITH fam AS MATERIALIZED (
        SELECT l.field AS kind, l.family AS handle, l.n
          FROM resource_field_scrub_families(p_resource)
               WITH ORDINALITY AS l(field, family, events, live, unset, first_seen, first_by, value_type, n)
    ), place AS MATERIALIZED (
        -- The ledger: each event of the field. Which of its paths count is decided below.
        SELECT m.n, m.kind, false AS live, 'kb_events.payload'::text AS surface, e.id AS target,
               t.name AS event_type, e.payload
          FROM fam m
         CROSS JOIN LATERAL _field_scrub_family_events(p_resource, m.kind, m.handle) AS fe(id)
          JOIN kb_events e ON e.id = fe.id
          JOIN kb_event_types t ON t.id = e.event_type_id
        UNION ALL
        -- The projection: the title or origin URI, today's by construction.
        SELECT m.n, m.kind, true,
               CASE m.kind WHEN 'title' THEN 'kb_resources.title' ELSE 'kb_resources.origin_uri' END,
               p_resource, NULL, NULL
          FROM fam m
         WHERE m.kind IN ('title', 'origin_uri')
        UNION ALL
        -- The projection: every row an event of the family asserted, live or folded.
        SELECT m.n, m.kind, NOT p.is_folded, s.surface, p.id, NULL, NULL
          FROM fam m
         CROSS JOIN LATERAL _field_scrub_family_events(p_resource, m.kind, m.handle) AS fe(id)
          JOIN kb_properties p ON p.asserted_by_event_id = fe.id
                              AND p.owner_table = 'kb_resources' AND p.owner_id = p_resource
         CROSS JOIN (VALUES ('kb_properties.property_key'), ('kb_properties.property_value')) AS s(surface)
         WHERE m.kind = 'property'
    ), hit AS (
        SELECT x.n, x.live
          FROM place x
          JOIN sensitivity.findings f ON f.surface = x.surface AND f.target_id = x.target
         WHERE (x.event_type IS NULL
                OR EXISTS (SELECT 1 FROM _erasure_redact_paths() l
                            WHERE l.event_type = x.event_type
                              AND _erasure_path_class(x.event_type, l.path, x.payload)
                                  = ANY (CASE x.kind
                                             WHEN 'title'      THEN ARRAY['title']
                                             WHEN 'origin_uri' THEN ARRAY['origin-uri']
                                             ELSE ARRAY['property-key', 'property-value', 'doc-type',
                                                        'facet-value']
                                         END)
                              AND sensitivity.ledger_path_covers(l.path, f.surface, f.path)))
           AND sensitivity.countable(f.id)
           AND NOT EXISTS (SELECT 1 FROM sensitivity.finding_closure c WHERE c.finding_id = f.id)
    )
    SELECT m.kind, m.handle,
           EXISTS (SELECT 1 FROM hit h WHERE h.n = m.n),
           EXISTS (SELECT 1 FROM hit h WHERE h.n = m.n AND h.live),
           v_covered
      FROM fam m
     ORDER BY m.n;
END;
$$;

COMMENT ON FUNCTION sensitivity.field_scrub_family_flags(uuid) IS
'For each row of resource_field_scrub_families(p_resource), in its order and keyed (field, family) as it keys them: flagged (an open countable finding on a place of the field: a redactable path of one of its events, the title or origin URI, or a row an event of the family asserted), current_flagged (the same on today''s place only: the title or origin URI, or a live row), and covered (sensitivity.resource_covered for every enabled detector, the same on every row). Booleans only, never a hash, category or text (field-grain scrub spec S1, S2).';

-- ---------------------------------------------------------------------------
-- Section 3. The door the field scrub's listing calls (erasure D10's pattern, 20261012100000
-- Section 5). Survey-only: the act never calls it, and reads only its plan. Without the sweep's
-- function it answers as a sweep that read nothing would: every row of the listing, nothing
-- flagged, nothing covered. It takes one resource from its caller, so it is a per-resource oracle
-- (does R hold a finding, has the sweep read it) and belongs behind the system-admin gate.
-- ---------------------------------------------------------------------------
CREATE FUNCTION resource_field_scrub_family_flags(p_resource uuid)
RETURNS TABLE (field text, family uuid, flagged boolean, current_flagged boolean, covered boolean)
LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    IF to_regproc('sensitivity.field_scrub_family_flags') IS NULL THEN
        RETURN QUERY
        SELECT l.field, l.family, false, false, false
          FROM resource_field_scrub_families(p_resource) l;
        RETURN;
    END IF;
    RETURN QUERY
    SELECT s.field, s.family, s.flagged, s.current_flagged, s.covered
      FROM sensitivity.field_scrub_family_flags(p_resource) s;
END;
$$;

COMMENT ON FUNCTION resource_field_scrub_family_flags(uuid) IS
'The field scrub listing''s flags (field-grain scrub S1): sensitivity.field_scrub_family_flags, one row per row of resource_field_scrub_families in its order, or every row unflagged and uncovered when that function is absent. Booleans only. Survey only; the act never calls it.';

-- ---------------------------------------------------------------------------
-- Section 4. No role reaches these by default (20261012100000 Section 6). One role migrates and
-- serves today (Q17), so the owning role keeps EXECUTE and nothing else is granted.
-- ---------------------------------------------------------------------------
REVOKE EXECUTE ON FUNCTION sensitivity.field_scrub_family_flags(uuid) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION resource_field_scrub_family_flags(uuid) FROM PUBLIC;

SELECT declare_migration(
    20261018100030,
    'additive',
    'The sweep closes a field scrub''s findings (field-grain scrub spec S9). CREATE OR REPLACE of sensitivity.place_closure with its signature, return type and search_path unchanged, so the sensitivity.finding_closure view and its three callers are untouched: on kb_events.payload and kb_events.metadata it also answers sentinel under a kb_event_field_redactions row of authority scrub whose resource_scrubbed event lists the event and path, and an erasure row now counts only under a resource_erased (every row written before 20261018100010 is an erasure row under one); on the projection it also answers sentinel for scrubbed-<resource id> titles and origin URIs, scrubbed-key-<uuid> keys, a property value at erased:<its asserting event id> (or that one-element array) and a scrubbed-facet mark; every answer it gave before is unchanged. New: sensitivity.field_scrub_family_flags and its public door resource_field_scrub_family_flags, SECURITY DEFINER with search_path set, returning (field, family) and three booleans per row of resource_field_scrub_families; EXECUTE on both revoked from PUBLIC, the owning role keeps it. No table, column or event type changes. Additive: no deployed binary names the sensitivity schema (the grep gate holds it), and no deployed binary calls the new door.'
);
