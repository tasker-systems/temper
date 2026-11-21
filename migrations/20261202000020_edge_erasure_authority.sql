-- Edge erasure: one ledger-derivation loop for resource and edge trails, an edge's trail, and the
-- `edge` authority that lets an act rewrite that trail.
--
-- An edge homed in the erased person's estate that no resource erasure reached is erased one edge
-- at a time. Rewriting such an edge's ledger needs what resource erasure already has: the events
-- that are the edge's own, each one's payload at the class sentinels, and an authority the ledger
-- guard admits the rewrite under.
--
--   1. _erasure_trail_payload_redaction(uuid[], jsonb) is the per-event loop of
--      _resource_erasure_payload_redaction (20261010100000), whose output is unchanged. The loop
--      leaves every event correlated with a resource_erased or edge_erased other than itself.
--   2. _edge_erasure_trail_scope(uuid) is an edge's own events, domain only, and
--      _edge_erasure_payload_redaction(uuid) the shared loop over it.
--   3. edge_erased, admin and NULL-anchored, and the redaction authority `edge`; the predicates
--      _edge_is_erased(uuid) (a record names the edge) and _edge_homed_in_estate(uuid, uuid).
--   4. _project_edge_erased_redactions projects its rows through _project_field_redactions.
--   5. The guard: an `edge` row admits a rewrite when its record's edge is folded and homed in a
--      context of _erasure_estate_contexts(estate_of) (_edge_homed_in_estate), the event is on
--      that edge's trail, and the value is the sentinel `erasure` would write. The person act's
--      classes stay its own.
--   6. edge_erased joins the admin acts the change readers leave out, and place_closure's arms.
--
-- A later edit must not break:
--   * one loop: a sentinel rule changed in _erasure_trail_payload_redaction changes both acts;
--   * the shared loop filters nothing by category or type: the trail scopes do;
--   * the phantom arm reads the edge's row, so the edge trail is read while the kb_edges row
--     exists;
--   * every reader of an authority maps all of them: the guard, the trail trigger, place_closure.
--
-- From Section 3's ALTER on, the migration runs under SET LOCAL lock_timeout = '500ms', the bound
-- and the reasons of 20261201000010_erasure_records_schema.sql: the ALTER takes ACCESS EXCLUSIVE
-- on kb_event_field_redactions and validates every row, and unbounded it would queue every reader
-- and writer of that table behind the longest open transaction. When the bound fires the migration
-- rolls back, the serving deployment keeps its schema, and the next deploy retries.

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

-- ---------------------------------------------------------------------------
-- Section 3. The edge_erased event type and the `edge` authority.
-- ---------------------------------------------------------------------------
-- Admin, NULL-anchored, typed. The payload schema literal is the committed fixture
-- crates/temper-substrate/tests/fixtures/payloads/edge_erased.v1.schema.json, and
-- `the_migration_literal_matches_the_committed_fixture` pins the pair. _event_append validates no
-- payload, so an appender of this kind must write one the fixture admits: the typed
-- payloads::EdgeErased, which verify_ledger_roundtrip and replay parse every row through.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('edge_erased', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "EdgeErased",
  "description": "`edge_erased` — one edge of an estate erased: a folded edge homed in a context of the person's\nestate, every prior value of its free text on its own trail redacted to the sentinels. One\nevent per edge, category `admin`, NULL-anchored. It projects `kb_event_field_redactions` rows\nunder the `edge` authority, whose subject check reads the folded edge's home against\n`_erasure_estate_contexts(estate_of)`.\n\nKeyed `subject_table` (`kb_edges`) / `subject_id`, never `edge_id`, and the person by\n`estate_of`, never `owner`: no key is a trail join-key shape (`edge_id`, `resource_id`,\n`owner`, `source`, `target`, `block_id`), and `_edge_erasure_trail_scope` joins on `edge_id`\nand `owner`. Paths only, never values: the record of a redaction must not carry what was\nredacted.",
  "type": "object",
  "properties": {
    "actor": {
      "description": "The acting system admin. `None` only where no actor exists to name.",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "estate_of": {
      "description": "The person whose estate homes the edge.",
      "$ref": "#/$defs/ProfileId"
    },
    "folded": {
      "description": "True when the act folded the edge itself, by a `relationship_folded` appended before this\nevent in the act's transaction; false when the edge was already folded. The act always\nwrites it; a reader takes an absent value as false.",
      "type": "boolean",
      "default": false
    },
    "redacted_fields": {
      "description": "The ledger paths this act redacted to their sentinels, in the shape `resource_erased` uses.",
      "type": "array",
      "items": {
        "$ref": "#/$defs/RedactedEventFields"
      }
    },
    "subject_id": {
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "description": "Always `kb_edges`.",
      "$ref": "#/$defs/AnchorTable"
    }
  },
  "required": [
    "subject_table",
    "subject_id",
    "estate_of"
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
    "EventId": {
      "description": "A `kb_events.id` value. Always UUIDv7 (time-sortable).",
      "type": "string",
      "format": "uuid"
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "RedactedEventFields": {
      "description": "The ledger paths of one event: redacted (`redacted_fields`) or named-and-unreached\n(`ledger_remainder`). ONE shape for both, so the cut-2 completion pass derives what it redacts\nfrom what cut 1 recorded without translating (resource erasure spec D12). The event is keyed\n`event`, never `event_id` — no trail join-key shape rides an admin payload (D1). Paths only,\nnever values: the record of a redaction must not carry what was redacted.",
      "type": "object",
      "properties": {
        "event": {
          "$ref": "#/$defs/EventId"
        },
        "paths": {
          "description": "JSON paths within that event's `payload` (or `metadata`), e.g. `title`, `origin_uri`.",
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "event",
        "paths"
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin');

SET LOCAL lock_timeout = '500ms';

ALTER TABLE kb_event_field_redactions
    DROP CONSTRAINT kb_event_field_redactions_authority_check,
    ADD CONSTRAINT kb_event_field_redactions_authority_check
        CHECK (authority IN ('erasure', 'scrub', 'principal', 'edge'));

COMMENT ON COLUMN kb_event_field_redactions.authority IS
$c$The authorising event's type: `erasure` (resource_erased, whose subject must be erased), `scrub`
(resource_scrubbed, whose subject must not be), `principal` (principal_erased, whose subject
profile must be tombstoned; context names and slugs, principal reasons and Slack ids only) or
`edge` (edge_erased, whose subject edge must be folded and homed in a context of the estate its
record names). Written by that type's projector. A structural token, never a value. The DEFAULT
'erasure' is the backfill and the reading of a binary that names no authority; a writer must never
lean on it for another authority. A row mislabelled `erasure` would occupy the key (event_id, path,
'erasure'): a later erasure of that path would collide on kb_event_field_redactions_pkey, which
resource_erasure_service reads as a concurrent completion and retries, colliding again every
time.$c$;

-- The two subject predicates of an edge erasure, each spelled once. _edge_is_erased answers for
-- an edge's writers and the sweep (20261202000030): whether an erasure record exists. The
-- verifier, place_closure and the unit instead check the subject of a record being written or
-- read, folded and homed in the estate it names, and share _edge_homed_in_estate for the home.
CREATE FUNCTION _edge_is_erased(p_edge uuid)
RETURNS boolean
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE et.name = 'edge_erased'
           AND ev.payload ->> 'subject_table' = 'kb_edges'
           AND ev.payload ->> 'subject_id' = p_edge::text);
$$;

COMMENT ON FUNCTION _edge_is_erased(uuid) IS
$c$Whether an edge is erased: some edge_erased record names it, payload subject_table 'kb_edges'
and subject_id the edge's id as text. Never NULL. It does not also require kb_edges.is_folded:
edge_erasure_execute appends its record only after the edge is folded, in the same transaction,
and no write path unfolds an edge, so a record implies a folded edge, and a second conjunct would
change no answer for any record the unit writes. The one predicate of the edge's writers (_resource_write_guard_owner's kb_edges
arm, _project_relationship_folded) and of the sweep's erased_place_findings edge arms, so they
agree on which edges are erased.$c$;

CREATE FUNCTION _edge_homed_in_estate(p_edge uuid, p_profile uuid)
RETURNS boolean
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_edges e
         WHERE e.id = p_edge
           AND e.home_anchor_table = 'kb_contexts'
           AND e.home_anchor_id IN (SELECT _erasure_estate_contexts(p_profile)));
$$;

COMMENT ON FUNCTION _edge_homed_in_estate(uuid, uuid) IS
$c$Whether an edge is homed in a context of p_profile's estate (_erasure_estate_contexts). Never
NULL: false for a missing edge, an edge homed outside kb_contexts, or a NULL profile. The `edge`
authority's home conjunct, read by edge_erasure_execute before it erases the edge, and with
is_folded by kb_events_append_only and sensitivity.place_closure on an `edge` row.$c$;

-- ---------------------------------------------------------------------------
-- Section 4. The projector.
-- ---------------------------------------------------------------------------
-- 20261022100000's body with an `edge` arm: the edge's trail from _edge_erasure_trail_scope, and
-- the resource acts' allowlist (_erasure_redact_paths), which the `p_authority <> 'principal'`
-- arm already reads for it.
CREATE OR REPLACE FUNCTION _project_field_redactions(p_event uuid, p_payload jsonb, p_authority text)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_name  text := CASE p_authority
                        WHEN 'erasure' THEN '_project_resource_erased_redactions'
                        WHEN 'scrub'   THEN '_project_resource_scrubbed_redactions'
                        WHEN 'principal' THEN '_project_principal_erased_redactions'
                        WHEN 'edge'    THEN '_project_edge_erased_redactions'
                    END;
    v_entry jsonb;
    v_path  text;
    v_type  text;
    v_trail jsonb;
BEGIN
    IF v_name IS NULL THEN
        RAISE EXCEPTION '_project_field_redactions: event % names no known authority %', p_event, p_authority;
    END IF;
    -- The subject's trail, once: event id → true. For the person act (`principal`), its scope:
    -- the context events of the estate it recorded, never a resource's trail. For an edge
    -- (`edge`), the edge's own trail.
    IF p_authority = 'principal' THEN
        SELECT coalesce(jsonb_object_agg(ce.event_id::text, true), '{}'::jsonb)
          INTO v_trail
          FROM _principal_erasure_context_events(
                   (p_payload ->> 'subject_id')::uuid,
                   ARRAY(SELECT jsonb_array_elements_text(
                             coalesce(p_payload -> 'estate_contexts', '[]'::jsonb))::uuid)) ce
         WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    ELSIF p_authority = 'edge' THEN
        SELECT coalesce(jsonb_object_agg(ts.event_id::text, true), '{}'::jsonb)
          INTO v_trail
          FROM _edge_erasure_trail_scope((p_payload ->> 'subject_id')::uuid) ts
         WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    ELSE
        SELECT coalesce(jsonb_object_agg(ts.event_id::text, true), '{}'::jsonb)
          INTO v_trail
          FROM _resource_erasure_trail_scope((p_payload ->> 'subject_id')::uuid) ts
         WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    END IF;
    FOR v_entry IN
        SELECT e FROM jsonb_array_elements(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) e
    LOOP
        SELECT et.name INTO v_type
          FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE ev.id = (v_entry ->> 'event')::uuid;
        IF v_type IS NULL THEN
            RAISE EXCEPTION '%: event % names a missing event %',
                v_name, p_event, v_entry ->> 'event';
        END IF;
        -- The replay walk projects the record as the ledger holds it, and a D14 order inversion
        -- can put a trail event's own block or edge after the act: checking the trail there would
        -- abort the whole replay. Live, kb_events_redaction_in_trail checks it again at the
        -- rewrite, so this check is the earlier of two.
        IF NOT v_trail ? (v_entry ->> 'event')
           AND coalesce(current_setting('temper.replaying', true), '') <> 'on' THEN
            RAISE EXCEPTION '%: event % names event %, which is not in its subject''s trail',
                v_name, p_event, v_entry ->> 'event';
        END IF;
        FOR v_path IN SELECT jsonb_array_elements_text(v_entry -> 'paths') LOOP
            -- Each authority on its own allowlist: the person act names only the context paths of
            -- _principal_erasure_redact_paths, and the resource and edge acts never name one.
            IF NOT EXISTS (
                SELECT 1 FROM _erasure_redact_paths() r
                 WHERE p_authority <> 'principal'
                   AND r.path = v_path AND r.class <> 'keep'
                   AND (r.event_type = v_type
                        OR (r.event_type IS NULL AND v_path LIKE 'metadata.%'))
                UNION ALL
                SELECT 1 FROM _principal_erasure_redact_paths() r
                 WHERE p_authority = 'principal'
                   AND r.path = v_path AND r.event_type = v_type) THEN
                RAISE EXCEPTION '%: event % names path % of a % event, which the allowlist does not hold',
                    v_name, p_event, v_path, v_type;
            END IF;
            INSERT INTO kb_event_field_redactions (event_id, path, redacted_by, authority)
            VALUES ((v_entry ->> 'event')::uuid, v_path, p_event, p_authority);
        END LOOP;
    END LOOP;
END;
$$;

COMMENT ON FUNCTION _project_field_redactions(uuid, jsonb, text) IS
$c$The one body of every redaction projection: one kb_event_field_redactions row per (event, path)
of p_payload's redacted_fields, under p_authority (`erasure`, `scrub`, `principal` or `edge`; any
other raises). Every named event must exist and, outside replay (temper.replaying), be in the
subject's trail: a resource's (_resource_erasure_trail_scope) for `erasure` and `scrub`, the
recorded estate's context events (_principal_erasure_context_events) for `principal`, an edge's
(_edge_erasure_trail_scope) for `edge`. Every path must be on its authority's allowlist:
_principal_erasure_redact_paths for `principal`, _erasure_redact_paths (never a `keep` path) for
the others. Called through the per-kind wrappers.$c$;

CREATE FUNCTION _project_edge_erased_redactions(p_event uuid, p_payload jsonb)
RETURNS void
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    SELECT _project_field_redactions(p_event, p_payload, 'edge');
$$;

COMMENT ON FUNCTION _project_edge_erased_redactions(uuid, jsonb) IS
$c$The edge_erased redaction projection: one kb_event_field_redactions row per (event, path) of
redacted_fields, authority `edge`, through _project_field_redactions. Every named event is on the
subject edge's own trail (_edge_erasure_trail_scope; skipped while temper.replaying is on), and every
path is on _erasure_redact_paths for its event's type. The act calls it after appending its record;
replay calls it at the event's position.$c$;

-- ---------------------------------------------------------------------------
-- Section 5. The verifier.
-- ---------------------------------------------------------------------------
-- 20261023100000's body; `edge` is admitted and answers exactly as `erasure`, the person act's
-- classes refused to it as to every authority but `principal`.
CREATE OR REPLACE FUNCTION _erasure_sentinel_admits(p_class text, p_event uuid, p_payload jsonb, p_at text[], p_old jsonb, p_new jsonb, p_authority text DEFAULT 'erasure'::text)
 RETURNS boolean
 LANGUAGE plpgsql
 IMMUTABLE
 SET search_path TO 'public', 'pg_temp'
AS $function$
DECLARE
    v_marks integer;
    v_scalar boolean;
BEGIN
    IF p_new IS NULL OR p_authority IS NULL
       OR p_authority NOT IN ('erasure', 'scrub', 'principal', 'edge') THEN
        RETURN false;
    END IF;
    -- A context's name and slug (20261022100000), and the subject's own admin-event reasons and
    -- disconnected Slack id (20261023100000), are the person act's alone, and the person act
    -- reaches nothing else: each authority's classes are disjoint, both ways.
    IF p_authority = 'principal' THEN
        IF p_class NOT IN ('context-name', 'context-slug', 'principal-reason', 'slack-principal') THEN
            RETURN false;
        END IF;
        RETURN p_new = _erasure_sentinel_exact(p_class, p_event, p_payload);
    END IF;
    IF p_class IN ('context-name', 'context-slug', 'principal-reason', 'slack-principal') THEN
        RETURN false;
    END IF;
    IF p_authority = 'scrub'
       AND p_class NOT IN ('keep', 'title', 'origin-uri', 'property-value', 'doc-type',
                           'property-key', 'facet-value') THEN
        RETURN false;
    END IF;
    -- From here `edge` takes every `erasure` arm: the edge act writes the sentinels of
    -- _erasure_trail_payload_redaction, the loop the resource act writes.
    CASE p_class
        WHEN 'keep' THEN
            RETURN p_new = p_old;
        WHEN 'property-key' THEN
            IF p_authority = 'scrub' THEN
                RETURN jsonb_typeof(p_new) = 'string'
                   AND p_new #>> '{}' ~ '^scrubbed-key-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$';
            END IF;
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
                                WHERE kv.key !~ CASE p_authority
                                          WHEN 'scrub'
                                          THEN '^scrubbed-facet-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}-[1-9][0-9]*$'
                                          ELSE '^erased-facet-[1-9][0-9]*$' END
                                   OR kv.value <> '"erased"'::jsonb);
        ELSE
            RETURN p_new = _erasure_sentinel_exact(p_class, p_event, p_payload);
    END CASE;
END;
$function$;

COMMENT ON FUNCTION _erasure_sentinel_admits(text, uuid, jsonb, text[], jsonb, jsonb, text) IS
$c$Whether p_new is a sentinel p_authority may write over p_old, a location of class p_class in event
p_event. Unknown authorities and a NULL p_new are refused. `principal` reaches only the context-name,
context-slug, principal-reason and slack-principal classes, at their exact sentinels, and no other
authority reaches them. `scrub` reaches title, origin-uri, property-value, doc-type, property-key
and facet-value, with its own scrubbed-key- and scrubbed-facet- patterns. `erasure` and `edge`
answer alike: keep holds the old value, property-key erased-key-<n>, edge-label erased-label-<n>
(a kept label unchanged), remote-source-url erased:<block>:<n> (a non-remote location unchanged),
facet-value erased:<event> for a scalar or one erased-facet-<n> mark per mark, every other class
its exact sentinel (_erasure_sentinel_exact).$c$;

CREATE OR REPLACE FUNCTION kb_events_append_only() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_type   text;
    v_path   text;
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
    FOR v_path IN
        SELECT DISTINCT r.path FROM kb_event_field_redactions r WHERE r.event_id = OLD.id
    LOOP
        v_rows := v_rows + 1;
        v_class := _erasure_path_class(v_type, v_path, OLD.payload);
        IF v_class IS NULL THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        v_meta := v_path LIKE 'metadata.%';
        FOR v_loc IN
            SELECT x.at, x.val
              FROM _erasure_expand(CASE WHEN v_meta THEN OLD.metadata ELSE OLD.payload END,
                                   CASE WHEN v_meta THEN substr(v_path, 10) ELSE v_path END) x
        LOOP
            v_new := (CASE WHEN v_meta THEN NEW.metadata ELSE NEW.payload END) #> v_loc.at;
            IF v_new IS DISTINCT FROM v_loc.val THEN
                IF NOT EXISTS (
                    SELECT 1
                      FROM kb_event_field_redactions r
                      JOIN kb_events e ON e.id = r.redacted_by
                      JOIN kb_event_types t ON t.id = e.event_type_id
                     WHERE r.event_id = OLD.id AND r.path = v_path
                       AND t.name = CASE r.authority
                                        WHEN 'erasure' THEN 'resource_erased'
                                        WHEN 'scrub'   THEN 'resource_scrubbed'
                                        WHEN 'principal' THEN 'principal_erased'
                                        WHEN 'edge'    THEN 'edge_erased'
                                    END
                       AND e.created = now()
                       AND e.payload -> 'redacted_fields' @> jsonb_build_array(jsonb_build_object(
                               'event', OLD.id::text, 'paths', jsonb_build_array(v_path)))
                       AND CASE r.authority
                               WHEN 'principal' THEN
                                   EXISTS (SELECT 1 FROM kb_profiles s
                                            WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                              AND s.tombstoned_at IS NOT NULL)
                               WHEN 'edge' THEN
                                   EXISTS (SELECT 1 FROM kb_edges s
                                            WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                              AND s.is_folded)
                                   AND _edge_homed_in_estate((e.payload ->> 'subject_id')::uuid,
                                                             (e.payload ->> 'estate_of')::uuid)
                               ELSE
                                   EXISTS (SELECT 1 FROM kb_resources s
                                            WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                              AND CASE r.authority
                                                      WHEN 'erasure' THEN s.erased_at IS NOT NULL
                                                      WHEN 'scrub'   THEN s.erased_at IS NULL
                                                  END)
                           END
                       AND _erasure_sentinel_admits(v_class, OLD.id, OLD.payload, v_loc.at,
                                                    v_loc.val, v_new, r.authority)) THEN
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
$c$The ledger's guard. DELETE always raises. An UPDATE is admitted only as an authorised redaction in
the authorising act's own transaction: no column but payload and metadata changes; every changed
location lies under a path a kb_event_field_redactions row names, and some row for that path
authorises it. A row authorises when its event is of its authority's type (resource_erased for
`erasure`, resource_scrubbed for `scrub`, principal_erased for `principal`, edge_erased for
`edge`), was written in this transaction, lists the event and the path, meets its precondition
(erasure: the resource subject is erased; scrub: it is not; principal: the subject profile is
tombstoned; edge: the subject edge is folded and homed in a context of
_erasure_estate_contexts(estate_of)), and the location holds the sentinel _erasure_sentinel_admits
admits under that authority; nothing else changed. kb_events_redaction_in_trail adds, per
statement, that each changed event is in the act's subject's trail and, for a scrub, in the
scrubbed field and prior. Anything else raises 'event ledger is append-only'.$c$;

CREATE OR REPLACE FUNCTION public.kb_events_redaction_in_trail()
 RETURNS trigger
 LANGUAGE plpgsql
 SET search_path = public, pg_temp
AS $function$
DECLARE
    v_subject uuid;
    v_record  uuid;
    v_payload jsonb;
    v_keys    jsonb;
    v_values  jsonb;
    v_scrubbed uuid;
    v_facets  jsonb;
BEGIN
    FOR v_subject IN
        SELECT DISTINCT (e.payload ->> 'subject_id')::uuid
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r
            ON r.event_id = n.id AND r.authority IN ('erasure', 'scrub')
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
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority IN ('erasure', 'scrub')
              JOIN kb_events e ON e.id = r.redacted_by
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND e.created = now()
               AND (e.payload ->> 'subject_id')::uuid = v_subject
               AND NOT EXISTS (SELECT 1 FROM trail t WHERE t.event_id = n.id)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;

    FOR v_record IN
        SELECT DISTINCT r.redacted_by
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority = 'scrub'
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        IF v_keys IS NULL THEN
            SELECT coalesce(jsonb_object_agg(o.id::text, o.payload -> 'property_key'), '{}'::jsonb)
              INTO v_keys
              FROM old_rows o
             WHERE o.payload ? 'property_key';
            SELECT coalesce(jsonb_object_agg(o.id::text, o.payload -> 'value'), '{}'::jsonb)
              INTO v_values
              FROM old_rows o
             WHERE o.payload ? 'value';
        END IF;
        SELECT e.payload INTO v_payload FROM kb_events e WHERE e.id = v_record;
        v_scrubbed := (v_payload ->> 'subject_id')::uuid;
        IF EXISTS (
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'scrub' AND r.redacted_by = v_record
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND NOT coalesce(_field_scrub_event_is_prior(
                       v_scrubbed,
                       v_payload #>> '{field,kind}',
                       (v_payload #>> '{field,family}')::uuid,
                       n.id, r.path, v_keys), false)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        -- The exact sentinel (S7). Metadata paths need no arm: none is a scrubbed field's own path,
        -- so the prior check above has already refused any scrub row naming one. Read only while
        -- the subject is not erased: on an erased subject a scrub row authorises nothing
        -- (kb_events_append_only), so a change there stands on an erasure row, under erasure's
        -- sentinels; that is the state when one transaction scrubs R and then erases it.
        v_facets := _field_scrub_facet_inner_keys(v_scrubbed, v_keys, v_values);
        IF EXISTS (SELECT 1 FROM kb_resources s WHERE s.id = v_scrubbed AND s.erased_at IS NULL)
           AND EXISTS (
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_types t ON t.id = o.event_type_id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'scrub' AND r.redacted_by = v_record
              LEFT JOIN _field_scrub_property_events(v_scrubbed, v_keys) pe ON pe.event_id = n.id
             CROSS JOIN LATERAL _erasure_expand(o.payload, r.path) x
             WHERE n.payload IS DISTINCT FROM o.payload
               AND r.path NOT LIKE 'metadata.%'
               AND (n.payload #> x.at) IS DISTINCT FROM x.val
               AND (n.payload #> x.at) IS DISTINCT FROM _field_scrub_sentinel(
                       _erasure_path_class(t.name, r.path, o.payload), o.id, o.payload, x.val,
                       pe.handle, v_facets)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        -- A family's key text is renamed whole, in one statement. Exactness alone holds one
        -- statement to one sentinel per family, but the handle is the lowest id STILL carrying the
        -- text: a statement renaming only part of a family's prior key text would leave the rest a
        -- family with a new handle, which a later statement could rename to a second sentinel. So
        -- when this record renames any key of a family, every event of that family whose key text
        -- is prior must carry a renamed key once the statement is done. The act renames them all
        -- in its one UPDATE (see resource_field_scrub_execute's header).
        IF EXISTS (SELECT 1 FROM kb_resources s WHERE s.id = v_scrubbed AND s.erased_at IS NULL)
           AND EXISTS (
            SELECT 1
              FROM _field_scrub_property_events(v_scrubbed, v_keys) pe
              JOIN kb_events cur ON cur.id = pe.event_id
             WHERE pe.handle IN (
                       SELECT pr.handle
                         FROM new_rows n
                         JOIN old_rows o ON o.id = n.id
                         JOIN kb_event_field_redactions r
                           ON r.event_id = n.id AND r.authority = 'scrub'
                          AND r.redacted_by = v_record AND r.path = 'property_key'
                         JOIN _field_scrub_property_events(v_scrubbed, v_keys) pr
                           ON pr.event_id = n.id
                        WHERE n.payload -> 'property_key' IS DISTINCT FROM o.payload -> 'property_key')
               AND cur.payload ->> 'property_key' IS NOT DISTINCT FROM pe.key_text
               AND coalesce(_field_scrub_event_is_prior(
                       v_scrubbed,
                       v_payload #>> '{field,kind}',
                       (v_payload #>> '{field,family}')::uuid,
                       pe.event_id, 'property_key', v_keys), false)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;

    -- The person act (20261022100000): every event its rows let this statement change is a
    -- context event of the estate its record names. The resource acts' loops above never read a
    -- `principal` row, whose subject is a profile and has no resource trail.
    FOR v_record IN
        SELECT DISTINCT r.redacted_by
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority = 'principal'
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        SELECT e.payload INTO v_payload FROM kb_events e WHERE e.id = v_record;
        IF EXISTS (
            WITH scope AS MATERIALIZED (
                SELECT ce.event_id
                  FROM _principal_erasure_context_events(
                           (v_payload ->> 'subject_id')::uuid,
                           ARRAY(SELECT jsonb_array_elements_text(
                                     coalesce(v_payload -> 'estate_contexts', '[]'::jsonb))::uuid)) ce
            )
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'principal' AND r.redacted_by = v_record
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND NOT EXISTS (SELECT 1 FROM scope s WHERE s.event_id = n.id)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;

    -- The edge act: every event its rows let this statement change is on its subject edge's own
    -- trail. The resource loop above never reads an `edge` row, whose subject is an edge.
    FOR v_record IN
        SELECT DISTINCT r.redacted_by
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority = 'edge'
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        SELECT e.payload INTO v_payload FROM kb_events e WHERE e.id = v_record;
        IF EXISTS (
            WITH trail AS MATERIALIZED (
                SELECT ts.event_id
                  FROM _edge_erasure_trail_scope((v_payload ->> 'subject_id')::uuid) ts
            )
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'edge' AND r.redacted_by = v_record
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND NOT EXISTS (SELECT 1 FROM trail t WHERE t.event_id = n.id)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;
    RETURN NULL;
END;
$function$;

COMMENT ON FUNCTION public.kb_events_redaction_in_trail() IS
$c$The ledger guard's per-statement half, beside kb_events_append_only: for each redaction record
written in this transaction whose rows let this statement change an event, every such event is in
the record's subject's trail: a resource's (_resource_erasure_trail_scope) for `erasure` and
`scrub`, the recorded estate's context events (_principal_erasure_context_events) for `principal`,
the subject edge's (_edge_erasure_trail_scope) for `edge`. A scrub record's changed events must
also be prior in its field or family (_field_scrub_event_is_prior), and, on a subject not erased,
hold the exact scrub sentinel, with a family's prior key text renamed whole in one statement.
Anything else raises 'event ledger is append-only'.$c$;

-- ---------------------------------------------------------------------------
-- Section 6. The readers: admin acts and the sweep's closure.
-- ---------------------------------------------------------------------------
-- 20261117000040's body: edge_erased joins the correlated admin acts.
CREATE OR REPLACE FUNCTION _event_is_admin_act(p_kind text, p_correlation uuid, p_emitter uuid, p_xact xid8)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT p_kind IN ('blob_erased', 'context_erased')
        OR (p_correlation IS NOT NULL AND EXISTS (
               SELECT 1
                 FROM kb_events act
                 JOIN kb_event_types act_t ON act_t.id = act.event_type_id
                WHERE act.category = 'admin'
                  AND act.correlation_id = p_correlation
                  AND act.emitter_entity_id = p_emitter
                  AND act.xact_id IS NOT DISTINCT FROM p_xact
                  AND act_t.name IN ('resource_erased', 'principal_erased',
                                     'resource_scrubbed', 'block_history_scrubbed',
                                     'edge_erased')))
$$;

COMMENT ON FUNCTION _event_is_admin_act(text, uuid, uuid, xid8) IS
$c$Whether an event is an admin erasure act's own work rather than a reader-visible change: a
blob_erased or context_erased by its kind alone, or an event sharing its correlation id, emitter
and transaction (xact_id, NULL matching NULL for legacy rows) with an admin resource_erased,
principal_erased, resource_scrubbed, block_history_scrubbed or edge_erased record. The change
feed and the context cursor withhold what it answers true for.$c$;

-- 20261107000000's body: edge_erased joins the admin acts an emitter's changes leave out.
CREATE OR REPLACE FUNCTION _profile_change_events(p_profile uuid)
RETURNS TABLE (occurred_at timestamptz,
               resource_id uuid, owner_resource_id uuid, block_id uuid,
               producing_anchor_table varchar, producing_anchor_id uuid)
LANGUAGE sql STABLE AS $$
    -- The admin acts this profile's emitters recorded, by correlation id: bounded by the
    -- profile's own history and five rare types, so it never reads the rest of the ledger.
    WITH acts AS MATERIALIZED (
        SELECT act.correlation_id, act.emitter_entity_id
          FROM kb_entities en
          JOIN kb_events act ON act.emitter_entity_id = en.id
          JOIN kb_event_types act_t ON act_t.id = act.event_type_id
         WHERE en.profile_id = p_profile
           AND act.correlation_id IS NOT NULL
           AND act_t.name IN ('resource_erased', 'principal_erased',
                              'resource_scrubbed', 'block_history_scrubbed', 'edge_erased')
    )
    SELECT ev.occurred_at,
           (ev.payload ->> 'resource_id')::uuid,
           CASE WHEN ev.payload -> 'owner' ->> 'table' = 'kb_resources'
                THEN (ev.payload -> 'owner' ->> 'id')::uuid END,
           (ev.payload ->> 'block_id')::uuid,
           ev.producing_anchor_table, ev.producing_anchor_id
      FROM kb_entities en
      JOIN kb_events ev ON ev.emitter_entity_id = en.id
      JOIN kb_event_types et ON et.id = ev.event_type_id
     WHERE en.profile_id = p_profile
       AND ev.producing_anchor_table IS NOT NULL
       AND et.category = 'domain'
       AND et.name <> ALL (ARRAY['webhook_received',
                                 'region_materialized', 'salience_refreshed', 'relationship_decayed',
                                 'blob_erased', 'context_erased',
                                 'invocation_closed', 'delegated_launch',
                                 'subscription_delivery_disposed', 'citation_audited'])
       AND NOT EXISTS (SELECT 1 FROM acts
                        WHERE acts.correlation_id = ev.correlation_id
                          AND acts.emitter_entity_id = ev.emitter_entity_id)
$$;

COMMENT ON FUNCTION _profile_change_events(uuid) IS
    'UNGATED, and private to resources_changed_by and contexts_changed_by: the anchored, authored domain events emitted by any entity of p_profile (never webhook_received; never the system-computed region_materialized, salience_refreshed or relationship_decayed; never an admin act on others'' work: blob_erased, context_erased, or any event sharing its correlation id and emitter with a resource_erased, principal_erased, resource_scrubbed, block_history_scrubbed or edge_erased record; never invocation_closed, delegated_launch, subscription_delivery_disposed or citation_audited), with the three resource ids the element trail matches on. The reader''s own changes, including those of agents acting under the profile, and never a machine client''s, which is its own profile. Not one of the verdict-handed ungated fragments (20260808000030): it is handed none. Call the gated readers instead.';

-- 20261018100040's body: an `edge` row closes a ledger finding as an `erasure` row does, its
-- subject the folded estate edge its record names. `principal` stays outside this function.
CREATE OR REPLACE FUNCTION sensitivity.place_closure(p_surface text, p_target uuid, p_hash text, p_path text)
 RETURNS text
 LANGUAGE sql
 STABLE
 SET search_path = public, pg_temp
AS $function$
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
                                                  WHEN 'edge'    THEN 'edge_erased'
                                              END
                                 AND e.payload -> 'redacted_fields' @> jsonb_build_array(jsonb_build_object(
                                         'event', p_target::text, 'paths', jsonb_build_array(r.path)))
                                 AND CASE r.authority
                                         WHEN 'edge' THEN
                                             EXISTS (SELECT 1 FROM kb_edges s
                                                      WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                                        AND s.is_folded)
                                             AND _edge_homed_in_estate((e.payload ->> 'subject_id')::uuid,
                                                                       (e.payload ->> 'estate_of')::uuid)
                                         ELSE
                                             EXISTS (SELECT 1 FROM kb_resources s
                                                      WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                                        AND (r.authority = 'scrub' OR s.erased_at IS NOT NULL))
                                     END
                                 AND (r.authority IN ('erasure', 'edge')
                                      OR coalesce((
                                          SELECT bool_and(_erasure_sentinel_admits(
                                                     _erasure_path_class(tt.name, r.path, ev.payload),
                                                     ev.id, ev.payload, x.at, x.val, x.val, 'scrub'))
                                            FROM kb_events ev
                                            JOIN kb_event_types tt ON tt.id = ev.event_type_id
                                           CROSS JOIN LATERAL _erasure_expand(
                                                     CASE WHEN r.path LIKE 'metadata.%'
                                                          THEN ev.metadata ELSE ev.payload END,
                                                     CASE WHEN r.path LIKE 'metadata.%'
                                                          THEN substr(r.path, 10) ELSE r.path END) x
                                           WHERE ev.id = p_target), false)))
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
              AND x.unit = _field_scrub_placeholder(p_target))
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
$function$;

COMMENT ON FUNCTION sensitivity.place_closure(text, uuid, text, text) IS
'How a finding''s place closes, from what it holds now (sweep D2 as amended): row_missing, sentinel, content_empty, or on a mutable surface changed by the sweep''s own later observation; NULL while it is open, and on a surface this function does not know. A ledger place (kb_events.payload, kb_events.metadata) closes as sentinel when a kb_event_field_redactions row on its event covers p_path and the row''s authorising event, of its authority''s type, lists that event and path: a resource_erased for an erased subject; an edge_erased whose subject edge is folded and homed in a context of _erasure_estate_contexts(estate_of); or a resource_scrubbed for a resource, erased since or not, whose rewrite the event still holds (its value at the path is a sentinel the scrub admits; field-grain scrub S9); never otherwise, and never on a principal_erased row. A projection place closes as sentinel on the erasure''s D4 sentinels and on the field scrub''s (S7): scrubbed-<resource id>, scrubbed-key-<uuid>, erased:<the row''s asserting event> and a scrubbed-facet mark. On an erasure or edge row''s `[*]` path the row means some location under it was rewritten, not every one. p_path is read only on the ledger.';

SELECT declare_migration(
    20261202000020,
    'additive',
    'Adds the functions _erasure_trail_payload_redaction(uuid[], jsonb), '
    '_edge_erasure_trail_scope(uuid), _edge_erasure_payload_redaction(uuid), '
    '_edge_is_erased(uuid), _edge_homed_in_estate(uuid, uuid) and '
    '_project_edge_erased_redactions(uuid, jsonb), and the admin event type edge_erased with its '
    'payload_schema. Widens kb_event_field_redactions_authority_check to admit ''edge'' (every '
    'existing row still passes), under SET LOCAL lock_timeout = 500ms: if its lock is not granted in '
    'time, the migration rolls back and the serving deployment is untouched. CREATE OR REPLACE, each with its signature, defaults, return type '
    'and search_path unchanged: _resource_erasure_payload_redaction (the shared loop over the same '
    'trail with the same numbering, so its rows are unchanged), _project_field_redactions, '
    '_erasure_sentinel_admits, kb_events_append_only, kb_events_redaction_in_trail, '
    '_event_is_admin_act, _profile_change_events and sensitivity.place_closure. Each gains an arm '
    'for edge_erased or the `edge` authority; no edge_erased record exists when this migration '
    'runs, so every existing answer is unchanged: no row carries `edge`, and the trail trigger''s resource loop, now read '
    'for erasure and scrub rows by name, already skipped every principal row. Comments on the '
    'authority column and every replaced function. No table, column or grant changes.'
);
