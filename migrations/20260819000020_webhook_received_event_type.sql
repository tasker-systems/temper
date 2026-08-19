-- Register `webhook_received` — the first foreign event type (S2 chunk B).
--
-- A foreign event is a webhook payload emitted by a remote system into kb_events via a
-- connection. It is NOT one of the 21 typed names in payloads::TYPED_EVENT_NAMES: those are
-- temper's own substrate events (cogmap_seeded, resource_created, etc.), each with a committed
-- JSON-Schema snapshot stamped by the bootseed. A foreign body has no published contract — it is
-- whatever the remote system sent, preserved verbatim — so it follows the permissive path the
-- schema's column comment already names: "NULL = unregistered/permissive — foreign/webhook types
-- may stay NULL" (canonical_schema.sql:448).
--
-- NOT ADDED TO system.yaml, deliberately. TYPED_EVENT_NAMES is pinned to the bootseed-stamped
-- typed registry, and `webhook_received` is permissive (NULL payload_schema), so it does not
-- belong there. The slack_principal_disconnected precedent (20260719000020) set the shape: a
-- migration-stamped event type absent from system.yaml. Unlike that one, this is NOT admin —
-- it is 'domain', the category ordinary knowledge-graph mutations live under. A webhook IS
-- the ledger's subject matter: it is the unjudged record judgment is made from (goal C1).
--
-- CATEGORY IS SPELLED AT REGISTRATION, not stamped afterwards. 20260719000010 dropped the column
-- DEFAULT precisely so an omitting registration fails loudly (NOT NULL) rather than silently
-- joining the trail allowlist. This migration spells 'domain' explicitly.
--
-- WHY NOT IN TYPED_EVENT_NAMES / no payload schema snapshot. The payload is the remote's verbatim
-- body, which has no fixed shape temper can publish. A JSON-Schema for it would either be so
-- permissive as to be meaningless (`{}`) or would lie about a shape we do not control. The
-- permissive path is the honest one: NULL payload_schema, validated Rust-side through
-- SubscriptionSelector matching at intake (the payload's repo / project_id fields are read by
-- the matcher, not by a schema validator). See goal exercise-status row for kb_event_types.
-- payload_schema: "0 foreign types registered" → this migration is the first.
--
-- The producing anchor for a webhook_received event is the connection's home_context_id
-- (kb_contexts). One event, one anchor — the receipt fact lives in one place. The matched
-- subscribers ride `references` (the fan), computed at intake before the INSERT. This is the
-- first-ever writer of kb_events.references (0 of 11,952 prod events today).

INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('webhook_received', NULL, 1, 'domain')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category = EXCLUDED.category;

SELECT declare_migration(
    20260819000020,
    'additive',
    'Registers webhook_received as a permissive (NULL payload_schema) foreign event type, category=domain. The first foreign event type: the payload is a remote webhook body preserved verbatim, which has no fixed shape temper can publish, so it follows the permissive path the schema column comment already names. Not added to system.yaml or TYPED_EVENT_NAMES (those are the bootseed-stamped typed registry; this is permissive). S2 chunk B is the first-ever writer of kb_events.references via this event type — the matched subscribers ride `references`, computed at intake before the INSERT (kb_events is append-only, so references cannot be UPDATEd after).'
);