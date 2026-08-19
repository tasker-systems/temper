-- Subscriptions: a team/context/cogmap subscribes to an aspect of a connection
-- (S2 chunk A of "external systems as subscribed emitters", spec 2026-07-13).
--
-- A PLAIN INFRA TABLE, deliberately NOT event-sourced, on the same two grounds as
-- kb_connections (20260714000010_connections.sql): admin infra is not a receipt of anything
-- external, and the goal's own invariant holds that the ledger records receipt, never
-- elaboration. A team declaring "we subscribe to repo Y" is internal infra.
--
-- The polymorphic (subscriber_table, subscriber_id) shape follows two on-disk precedents:
--   - kb_access_grants (20260630000001_access_grants_seam.sql:24-36): dual-polymorphic
--     (subject) x (principal) with a CHECK IN on the discriminator.
--   - kb_cogmap_region_members (20260624000001_canonical_schema.sql:748-755): (member_table,
--     member_id) with a CHECK IN on member_table.
-- Three narrow tables (one per subscriber kind) would depart from every existing pattern,
-- force the matching query (chunk B) to UNION three sources, and duplicate the selector /
-- connection_id / created_by / revoked_at columns three times. The cost of the polymorphic
-- shape is that subscriber_id cannot be a declared FK (no single REFERENCES target for three
-- tables) — the same is already true of kb_access_grants.subject_id and
-- kb_cogmap_region_members.member_id, and the pattern there is to validate existence in the
-- service layer. We follow it.
--
-- subscriber_table's admitted set ('kb_contexts','kb_cogmaps','kb_teams') maps 1:1 to
-- AnchorTable::{Contexts, Cogmaps, Teams} (crates/temper-substrate/src/payloads.rs:33-52),
-- which are already the EventRef.target.kind variants chunk B will write as the `touches` rel
-- at intake. No new AnchorTable variant is needed.
--
-- The selector is JSONB on the row but a typed Rust enum (SubscriptionSelector,
-- #[serde(tag="kind")]) in the wire shape. The variant IS the capability declaration: a
-- GitHubCodeownersPaths selector declares "I need enrichment" by being that variant; no
-- separate needs_enrichment bool that can drift out of sync. Adding a provider = adding a
-- variant = a compile error at every match site, which is the desired forcing function.
--
-- Revocation, not deletion: mirrors kb_connections (20260714000010_connections.sql:87-88).
-- A subscription that existed at intake must stay resolvable at disposition time, even if it
-- was later revoked — the delivery row's research-corpus property depends on it. A revoked
-- subscription stops matching (chunk B's query filters revoked_at IS NULL); the history
-- stays.
CREATE TABLE kb_subscriptions (
    id                     UUID PRIMARY KEY DEFAULT uuid_generate_v7(),

    -- The subscriber: who is asking to be told. Polymorphic over the three kinds the goal
    -- names. subscriber_id carries no FK — same discipline as kb_access_grants.subject_id:
    -- existence is validated in the service layer, not by a declared REFERENCES.
    subscriber_table       VARCHAR(64) NOT NULL
        CHECK (subscriber_table IN ('kb_contexts', 'kb_cogmaps', 'kb_teams')),
    subscriber_id          UUID        NOT NULL,

    -- The connection whose events this subscription wants. FK to kb_connections; a revoked
    -- connection may still have live subscriptions against it (the revocation stops new
    -- events from arriving, but the subscription row stays honest about what was declared).
    connection_id          UUID        NOT NULL REFERENCES kb_connections(id),

    -- The team whose manage-capable role authorizes this subscription. NOT derived from
    -- the subscriber: kb_cogmaps has no owner team (only many-to-many kb_team_cogmaps links),
    -- and kb_contexts.owner_table can be 'kb_profiles' (a profile-owned context has no team).
    -- The caller NAMES the authoring team, and the two-leg gate checks against it:
    --   1. require_manage_on_team(authoring_team_id) — caller is owner/maintainer of it.
    --   2. kb_access_grants row: subject_table='kb_connections', subject_id=connection_id,
    --      principal_table='kb_teams', principal_id=authoring_team_id, can_read=true.
    -- For kb_teams subscribers, authoring_team_id = subscriber_id (a team subscribes for
    -- itself). For kb_contexts/kb_cogmaps, the authoring team is the team the caller manages
    -- that is linked to the subscriber (kb_team_contexts / kb_team_cogmaps) — the service
    -- layer validates the link exists; the row records which team.
    authoring_team_id      UUID        NOT NULL REFERENCES kb_teams(id),

    -- The selector: per-provider typed shape, stored as JSONB. The column is the storage;
    -- the wire type (SubscriptionSelector enum, temper-core/src/types/subscription.rs) is the
    -- shape. A reader that does not know the enum sees opaque JSON; a reader that does sees
    -- a typed value. The variant declares its own capability (what it needs to evaluate).
    selector               JSONB       NOT NULL,

    -- The authoring principal, for audit. The two-leg authz gate (authoring-team
    -- manage-capable + reach grant held) runs BEFORE this INSERT; the row records who passed
    -- the gate, never what the gate was.
    created_by_profile_id  UUID        NOT NULL REFERENCES kb_profiles(id),

    created                TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at             TIMESTAMPTZ     NULL,
    revoked_by_profile_id  UUID            NULL REFERENCES kb_profiles(id)
);

-- Chunk B's matching query is
--   SELECT ... FROM kb_subscriptions WHERE connection_id = $1 AND revoked_at IS NULL ...
-- so the connection leg is the hot path. Partial index on the live rows only — revoked rows
-- stay queryable by id (the PK) but do not clutter the live-match index.
CREATE INDEX idx_kb_subscriptions_connection
    ON kb_subscriptions(connection_id)
    WHERE revoked_at IS NULL;

-- Subscriber-side lookup: "what does this team/context/cogmap subscribe to?"
CREATE INDEX idx_kb_subscriptions_subscriber
    ON kb_subscriptions(subscriber_table, subscriber_id)
    WHERE revoked_at IS NULL;

-- Declaring the same selector against the same connection by the same authoring team twice is
-- not a new declaration — it is a conflict. Distinct selectors against the same connection by
-- the same authoring team ARE distinct subscriptions (e.g. two repos under one GitHub org
-- connection).
--
-- JSONB storage is canonical by construction (key order normalized on insert — that is what
-- the `b` means), so a plain UNIQUE over the JSONB column compares structurally, not by raw
-- text. Service-layer enforcement would duplicate the constraint and drift; the DB
-- constraint is the single source of truth. A revoked subscription's row stays in the table
-- (never deleted), so the constraint covers live and revoked rows alike — re-declaring a
-- revoked selector is still a conflict, which is the honest answer (you revoked it; re-declare
-- by revoking-and-recreating if you genuinely want a fresh row, though that is not supported
-- by this chunk's service layer).
CREATE UNIQUE INDEX uq_kb_subscriptions_declared
    ON kb_subscriptions(authoring_team_id, connection_id, selector);

COMMENT ON TABLE kb_subscriptions IS
  'A declaration that a team/context/cogmap wants to be told when a connection emits events matching a selector. Plain infra, not event-sourced: declaring a subscription is internal infra, not a receipt of anything external. Polymorphic subscriber follows kb_access_grants / kb_cogmap_region_members precedent. Revocation, not deletion: a subscription that existed at intake must stay resolvable at disposition time (the delivery row''s research-corpus property).';

COMMENT ON COLUMN kb_subscriptions.subscriber_table IS
  'kb_contexts | kb_cogmaps | kb_teams — the three subscriber kinds the goal names. Maps 1:1 to AnchorTable::{Contexts, Cogmaps, Teams} (payloads.rs:33-52), which are the EventRef.target.kind variants chunk B writes as the `touches` rel. No other table is admissible as a subscriber.';

COMMENT ON COLUMN kb_subscriptions.subscriber_id IS
  'The subscriber row. No FK — same discipline as kb_access_grants.subject_id: existence is validated in the service layer, not by a declared REFERENCES, because the polymorphic subscriber_table makes a single FK target impossible. For kb_contexts/kb_cogmaps, the service layer also validates the authoring_team_id is linked to the subscriber (kb_team_contexts / kb_team_cogmaps); for kb_teams, authoring_team_id = subscriber_id.';

COMMENT ON COLUMN kb_subscriptions.authoring_team_id IS
  'The team whose manage-capable role authorizes this subscription. NOT derived from the subscriber: kb_cogmaps has no owner team (only kb_team_cogmaps links), and kb_contexts.owner_table can be kb_profiles (no team). The caller names the team, and the two-leg gate (require_manage_on_team + kb_access_grants reach-grant read) checks against it. For kb_teams subscribers, authoring_team_id = subscriber_id.';

COMMENT ON COLUMN kb_subscriptions.connection_id IS
  'The connection whose events this subscription wants. A revoked connection may still have live subscriptions against it — the revocation stops new events from arriving, but the subscription row stays honest about what was declared.';

COMMENT ON COLUMN kb_subscriptions.selector IS
  'Per-provider typed selector (SubscriptionSelector enum in temper-core, #[serde(tag="kind")]). The variant IS the capability declaration: a GitHubCodeownersPaths selector declares "I need enrichment" by being that variant; no separate needs_enrichment bool that can drift. Stored as JSONB; the column is the storage, the Rust enum is the shape. Adding a provider = adding a variant = a compile error at every match site.';

COMMENT ON COLUMN kb_subscriptions.revoked_at IS
  'A revoked subscription stops matching (chunk B filters revoked_at IS NULL); the history stays. Rows are never deleted — mirrors kb_connections (20260714000010_connections.sql:87-88). The delivery row''s research-corpus property requires a subscription that existed at intake to be resolvable at disposition time, even if later revoked.';

SELECT declare_migration(
    20260819000010,
    'additive',
    'Adds the kb_subscriptions table (S2 chunk A): polymorphic (subscriber_table, subscriber_id) over (kb_contexts, kb_cogmaps, kb_teams), connection_id FK to kb_connections, selector JSONB, two indexes, and a unique constraint on (subscriber_table, subscriber_id, connection_id, selector). Additive-only on main (new table, no existing row invalidated); the pre-deploy binary neither writes nor reads it. No enum change, no wire-contract move. The two-leg authz gate (subscriber-team manage-capable via require_manage_on_team + reach grant held on kb_access_grants) is enforced in the service layer, not the schema — same discipline as kb_connections''s own authz. No references writes yet (chunk B owns the first-ever writer of kb_events.references).'
);