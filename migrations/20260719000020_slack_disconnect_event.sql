-- The Slack-disconnect ledger event (task 019f75ec).
--
-- Registers `slack_principal_disconnected` as an `admin` event type and defines the SQL chokepoint
-- that unbinds the principal AND appends the event in one transaction.
--
-- CATEGORY IS SPELLED AT REGISTRATION, not stamped afterwards. `20260719000010` retaxed the
-- vocabulary (cognition -> domain, plus `system`) and DROPPED the column's DEFAULT precisely so a
-- registration that omits `category` fails loudly rather than silently joining the trail allowlist.
-- Its CONSEQUENCE paragraph is binding on this migration: once any event of a type exists, the
-- composite FK `kb_events_category_matches_type` is `ON UPDATE RESTRICT`, so a type's category can
-- never be corrected afterwards. An earlier draft of this file used the pre-retax idiom -- INSERT
-- without `category`, then `UPDATE ... SET category = 'admin'` -- which would abort at apply time on
-- NOT NULL. That is the failure `20260719000010` was written to force, and it worked.
--
-- `admin` also buys the two new runtime guarantees for free: `kb_events_admin_is_unanchored`
-- (admin => NULL producing anchor, which this function satisfies by passing NULL,NULL) and the
-- element-trail allowlist, which now admits only `domain`.
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
-- satisfies it exactly as the 18th did. (The bootseed loop classifies by
-- `payloads::ADMIN_EVENT_NAMES`, which carries this name, so a `reset_schema` rebuild reproduces
-- `admin` rather than defaulting it to `domain`.)

INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('slack_principal_disconnected', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "SlackPrincipalDisconnected",
  "description": "`slack_principal_disconnected` — the audit record of a Slack principal being unbound from a\ntemper profile. The subject is the **profile**, not the link: `AnchorTable` has no\n`kb_profile_auth_links` variant, and the auth-link row is deleted by the same act that emits\nthis — so the row it describes no longer exists to point at. `disconnected_by` is the acting\nprofile; it EQUALS the subject on the self-serve arm and DIFFERS on the admin arm, and telling\nthose two apart is most of why this event is worth having.\n\n`idp_revocation` carries what happened to the grant AT SLACK, because \"the binding is gone\" and\n\"the token is dead\" are different facts and an offboarding auditor needs the second one. The\nunbind commits regardless of the revoke outcome (the revoke is best-effort by design — Slack\nbeing down must not block the only unbind lever), so without this field a disconnect performed\nwhile Slack was unreachable would read, on the ledger, as indistinguishable from a clean one.\nIt is `temper_core`'s own three-state enum rather than a copy: the ledger and the HTTP surface\nthen cannot disagree about what `revoked` means.",
  "type": "object",
  "properties": {
    "disconnected_by": {
      "$ref": "#/$defs/ProfileId"
    },
    "idp_revocation": {
      "$ref": "#/$defs/IdpRevocation"
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
    "disconnected_by",
    "idp_revocation"
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
    "IdpRevocation": {
      "description": "What happened to the stored grant at the identity provider.\n\nA three-state enum rather than a `bool`, because `false` used to collapse\nthree genuinely different facts — \"there was no grant, so nothing was\nattempted\", \"a revoke was attempted and failed\", and (in AS mode) \"the\nUPDATE matched zero rows\" — and consumers could not tell them apart. The CLI\nconsequently warned \"the identity provider did not confirm revocation\" at a\nuser who had no grant at all.",
      "oneOf": [
        {
          "description": "No stored grant, so no revocation was attempted.",
          "type": "string",
          "const": "not_attempted"
        },
        {
          "description": "The IdP (or, in AS mode, the local token store) confirmed revocation.",
          "type": "string",
          "const": "revoked"
        },
        {
          "description": "A revocation was attempted and did not succeed. The local grant was\ndestroyed regardless; the grant may remain live at the IdP.",
          "type": "string",
          "const": "failed"
        }
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}$JS$::jsonb, 1, 'admin')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema, schema_version = EXCLUDED.schema_version;

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

-- p_idp_revocation is the OUTCOME OF A STEP THAT ALREADY RAN, passed down rather than computed
-- here: the revoke is a network call to Slack (or, in AS mode, a local token-store UPDATE) that the
-- caller performs BEFORE this transaction, deliberately best-effort. The ledger records it because
-- "the binding is gone" and "the token is dead at Slack" are different facts, and the second is the
-- one an offboarding auditor actually needs. Spelled as text and validated by the registry's
-- payload_schema enum (temper_core::types::slack::IdpRevocation, snake_case): not_attempted |
-- revoked | failed.

CREATE FUNCTION _admin_slack_disconnected(
    p_emitter             uuid,
    p_slack_principal_id  text,
    p_disconnected_by     uuid,
    p_idp_revocation      text,
    p_correlation         uuid DEFAULT NULL
) RETURNS boolean
LANGUAGE plpgsql AS $$
DECLARE
    v_profile uuid;
    v_deleted integer;
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

    -- ROW_COUNT alongside the RETURNING, matching _admin_grant_revoked (20260718000010). Plain
    -- `INTO` (no STRICT) takes ONE row and silently discards any others, so on a multi-row match it
    -- would delete N and emit 1 -- an under-count with no error. That is unreachable today:
    -- kb_profile_auth_links carries UNIQUE(auth_provider, auth_provider_user_id) (20260624000001),
    -- so at most one row can match. The assertion is here so that if that constraint is ever
    -- relaxed, this fails loudly instead of quietly under-reporting an authority act.
    GET DIAGNOSTICS v_deleted = ROW_COUNT;
    IF v_deleted > 1 THEN
        RAISE EXCEPTION
            'slack principal % matched % auth-link rows; the UNIQUE(auth_provider, auth_provider_user_id) assumption this function relies on no longer holds',
            p_slack_principal_id, v_deleted;
    END IF;

    -- Emit only when a row was actually removed. A disconnect of an already-unlinked principal is not
    -- an admin act, and kb_events is append-only -- a spurious event is immortal and can only be
    -- quarantined, never corrected. Mirrors _admin_grant_revoked's suppression of no-op deletes.
    -- (profile_id is NOT NULL in the DDL, so v_profile IS NULL is exactly "no row was deleted".)
    IF v_profile IS NOT NULL THEN
        PERFORM _event_append(
            'slack_principal_disconnected', p_emitter, NULL, NULL,
            jsonb_build_object(
                'subject_table', 'kb_profiles',
                'subject_id', v_profile,
                'slack_principal_id', p_slack_principal_id,
                'disconnected_by', p_disconnected_by,
                'idp_revocation', p_idp_revocation),
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
