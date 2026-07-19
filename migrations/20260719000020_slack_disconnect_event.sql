-- The Slack-disconnect ledger event (task 019f75ec).
--
-- Registers `slack_principal_disconnected`, classifies it `admin`, and defines the SQL chokepoint
-- that unbinds the principal AND appends the event in one transaction.
--
-- WHY THE SCHEMA IS INLINED VERBATIM. The JSON below is copied byte-for-byte from the committed
-- crates/temper-substrate/tests/fixtures/payloads/slack_principal_disconnected.v1.schema.json,
-- which is GENERATED from `payloads::SlackPrincipalDisconnected`
-- (UPDATE_SCHEMA=1 cargo make test-schema, package-scoped -p temper-substrate). That is the
-- repo == registry == Rust-types chain every other typed name rides; retyping it by hand breaks
-- `payload_schemas_match_snapshots` or, worse, silently stamps a registry schema that disagrees
-- with the struct. Template: 20260717000010_admin_event_types.sql.
--
-- NOT ADDED TO system.yaml, deliberately. `seed_migration_event_types_match_system_yaml`
-- (bootseed.rs) requires every system.yaml name to also appear in canonical_seed.sql -- a shipped,
-- applied migration that must not be edited. `admin_ledger_opened` set the precedent: typed,
-- stamped by its own forward migration, absent from system.yaml. This is safe because
-- `bootseed_publishes_payload_schemas` counts non-NULL payload_schema rows against
-- TYPED_EVENT_NAMES.len() rather than comparing the two SETS -- so a migration-stamped 19th name
-- satisfies it exactly as the 18th did.

INSERT INTO kb_event_types (name, payload_schema, schema_version) VALUES
  ('slack_principal_disconnected', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "SlackPrincipalDisconnected",
  "description": "`slack_principal_disconnected` — the audit record of a Slack principal being unbound from a\ntemper profile. The subject is the **profile**, not the link: `AnchorTable` has no\n`kb_profile_auth_links` variant, and the auth-link row is deleted by the same act that emits\nthis — so the row it describes no longer exists to point at. `disconnected_by` is the acting\nprofile; it EQUALS the subject on the self-serve arm and DIFFERS on the admin arm, and telling\nthose two apart is most of why this event is worth having.",
  "type": "object",
  "properties": {
    "disconnected_by": {
      "$ref": "#/$defs/ProfileId"
    },
    "slack_principal_id": {
      "type": "string"
    },
    "subject_id": {
      "type": "string",
      "format": "uuid"
    },
    "subject_table": {
      "$ref": "#/$defs/AnchorTable"
    }
  },
  "required": [
    "subject_table",
    "subject_id",
    "slack_principal_id",
    "disconnected_by"
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
        "kb_machine_clients"
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}$JS$::jsonb, 1)
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema, schema_version = EXCLUDED.schema_version;

-- CLASSIFY IT ADMIN. `kb_event_types.category` defaults to 'cognition' (20260718000020) and the
-- element-trail firewall is an ALLOWLIST (`et.category = 'cognition'`), so an admin type left
-- unstamped passes filter B and leans entirely on filter A (anchor nullity). Both halves ship for
-- exactly this reason -- neither default is safe alone. UPDATE-by-name, never INSERT: the row above
-- may or may not pre-exist, and an UPDATE is correct and idempotent either way.
UPDATE kb_event_types
   SET category = 'admin'
 WHERE name = 'slack_principal_disconnected';

-- THE CHOKEPOINT. Task 5's shape (20260718000010): one function that performs the state mutation
-- AND appends the event, in one transaction.
--
-- WHY SQL AND NOT RUST: the subject's identity is only knowable *from* the delete. The auth-link row
-- is keyed by (auth_provider, auth_provider_user_id) and its profile_id is gone the moment it is
-- removed, so DELETE ... RETURNING captures it in the same statement that destroys it. A Rust pair
-- of statements could not guarantee that, and could not guarantee the delete and the event share a
-- fate. This also mirrors how cognition acts fire (_event_append, canonical_functions.sql:765).
--
-- BOTH-NULL PRODUCING ANCHOR, always. Unbinding an identity is an authority act; it has no cognition
-- home. Anchoring it would put it in front of every region producer and break the "governance is
-- traceable, but it isn't knowledge" boundary.
--
-- THE SUBJECT IS THE PROFILE, not the auth-link row: `AnchorTable` (payloads.rs) has nine variants,
-- `kb_profile_auth_links` is not one of them, and `as_str` has no `_ =>` arm -- so the enum cannot
-- name a row that does not survive the act. The Slack principal rides in the payload instead.
--
-- The payload spells subject_table/subject_id and NEVER resource_id/block_id/edge_id/owner:
-- element_trail_node/_edge match on payload key shape with NO type filter and are gated only by
-- resources_visible_to, so those keys would leak the unbinding into any reader's element trail
-- (spec 2026-07-16 §5).

CREATE FUNCTION _admin_slack_disconnected(
    p_emitter             uuid,
    p_slack_principal_id  text,
    p_disconnected_by     uuid,
    p_correlation         uuid DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_profile uuid;
BEGIN
    -- RETURNING captures the subject in the same statement that destroys the row. There is no second
    -- chance: afterwards nothing in the database associates this principal with a profile.
    --
    -- The literal 'slack' duplicates slack_link_service::SLACK_AUTH_PROVIDER, which is what the Rust
    -- DELETE this replaces used to bind. The duplication is deliberate and contained: this function
    -- is slack-scoped by name and by the event it emits, so a provider parameter would imply a
    -- genericity it does not have. If that const ever changes, this literal must change with it.
    DELETE FROM kb_profile_auth_links
     WHERE auth_provider = 'slack'
       AND auth_provider_user_id = p_slack_principal_id
    RETURNING profile_id INTO v_profile;

    -- Emit only when a row was actually removed. A disconnect of an already-unlinked principal is not
    -- an admin act, and kb_events is append-only -- a spurious event is immortal and can only be
    -- quarantined, never corrected. Mirrors _admin_grant_revoked's suppression of no-op deletes.
    IF v_profile IS NOT NULL THEN
        PERFORM _event_append(
            'slack_principal_disconnected', p_emitter, NULL, NULL,
            jsonb_build_object(
                'subject_table', 'kb_profiles',
                'subject_id', v_profile,
                'slack_principal_id', p_slack_principal_id,
                'disconnected_by', p_disconnected_by),
            p_references => jsonb_build_array(
                jsonb_build_object('rel','subject','target',
                    jsonb_build_object('kind','kb_profiles','id', v_profile))),
            p_correlation => p_correlation);
        RETURN true;
    END IF;

    RETURN false;
END;
$$;

COMMENT ON FUNCTION _admin_slack_disconnected IS
  'Slack auth-link DELETE + slack_principal_disconnected event, one txn. Both-NULL producing anchor: unbinding an identity is an authority act with no cognition home. The subject is the PROFILE, captured by DELETE ... RETURNING because the auth-link row does not survive the act; the Slack principal rides in the payload. Emits only when a row was actually removed -- kb_events is append-only and a spurious event is immortal.';
