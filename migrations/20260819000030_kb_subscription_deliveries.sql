-- Delivery rows: one per (subscription, event) the radius matched (S2 chunk C of "external
-- systems as subscribed emitters", spec 2026-07-13).
--
-- A PLAIN INFRA TABLE, projected in Rust inside the intake transaction — NOT a payload-first
-- `_project_*` half. That is a deliberate CONFORM, and the precedent is the one the goal itself
-- cites: `region_materialized` writes its N `kb_cogmap_region_members` rows in Rust, in the
-- event's own transaction (`crates/temper-substrate/src/write.rs:196-204`), while
-- `_project_region_materialized` only stamps the anchor's watermark. The reason a projection half
-- is unavailable here is stated on the ledger itself: "The projection halves (_project_*) read
-- ONLY this [payload]" (20260624000001_canonical_schema.sql:473-474). Chunk B's payload is the
-- remote's verbatim body and does not carry the matched set — the matched set rides `references`
-- — so a payload-first half literally cannot see what it would need to project. Wrapping the
-- verbatim body to smuggle the matched set into the payload was the alternative, and it would
-- AMEND a shipped contract (chunk B's `payload preserved verbatim` witness) to buy replay
-- coverage for a table whose whole purpose is mutable per-subscriber state. Region tables are
-- excluded from replay's diff for the same reason (`replay.rs:9-11`).
--
-- TWO LIFECYCLES SHARE THIS TABLE. `kb_invocations.originating_cogmap_id` is NOT NULL
-- (canonical_schema.sql:518) and `kb_cogmaps.telos_resource_id` is NOT NULL (:246), so only a
-- kb_cogmaps subscriber can ever be acted for under an invocation envelope. A kb_contexts or
-- kb_teams subscriber has no telos, no steward and no envelope: it subscribed in order to BE
-- AWARE, and its row is terminal at in_scope (or undetermined). That undisposed row is a record
-- of awareness, NOT an unfinished queue — the goal's `No phantom backlog` negative forbids any
-- surface reporting it as one. Nothing in this schema requires a disposition; the CHECKs below
-- constrain what a disposition must CARRY when one is made, never that one must be made.
--
-- THE DISPOSITION'S ACCOUNTABILITY CARRIER IS THE ACTOR, NOT THE ENVELOPE. The design originally
-- said "an authored event under an invocation envelope"; that named a mechanism which silently
-- scoped the lifecycle to one of the three subscriber kinds. The load-bearing property is
-- ACCOUNTABLE AND CITABLE. So decided_by_invocation_id is NULLABLE and decided_by_profile_id is
-- its peer: an agent acting for a cogmap disposes under an envelope, a human disposes as a
-- profile, and the CHECK requires at least one. See the goal register, "Who may dispose, and who
-- may only be aware".
--
-- ALSO THE READ SURFACE, not only the queue. Three read paths exist over kb_events and a
-- webhook_received event is unreachable on all three: event_service::latest_event_id_for_context
-- is a scalar cursor; event_service::element_trail is keyed to a resource or edge and a webhook
-- creates neither (goal C7); admin_ledger_service::readable_event_types is an allowlist whose
-- default is "absent from this fn => admin-only => fail closed" and webhook_received has no arm.
-- A team cannot today see that a webhook it subscribed to ever arrived. So for goal clause C11
-- (`routing-is-readable-by-the-routed-to`) this table IS the surface, which is why the
-- subscription-grain index below is a first-class access path and not an afterthought.

CREATE TABLE kb_subscription_deliveries (
    id                       UUID PRIMARY KEY DEFAULT uuid_generate_v7(),

    -- What was routed, and to which declaration. subscription_id carries a real FK (unlike
    -- kb_subscriptions.subscriber_id, whose polymorphism makes one impossible) because a
    -- delivery is always against exactly one subscription row. Subscriptions are revoked, never
    -- deleted (20260819000010), so this FK can never dangle — which is precisely the property
    -- the delivery table's research-corpus claim depends on: a subscription that existed at
    -- intake stays resolvable at disposition time even if it was revoked in between.
    subscription_id          UUID        NOT NULL REFERENCES kb_subscriptions(id),
    event_id                 UUID        NOT NULL REFERENCES kb_events(id),

    -- The scope leg, walked by enrichment (S4). Born pending_scope at intake because the coarse
    -- radius is payload-only: a GitHubCodeownersPaths selector matches on repo and cannot know
    -- whether the CODEOWNERS paths were hit until the changed-file list is fetched.
    --
    -- `undetermined` is the DLQ marker and it is a FIRST-CLASS TERMINAL STATE, not an error code.
    -- Goal invariant 6: an enrichment that fails leaves the delivery undetermined and VISIBLE —
    -- it must never silently resolve to out_of_scope. The CHECK admits it as an ordinary value
    -- precisely so no code path can treat it as exceptional and collapse it.
    status                   TEXT        NOT NULL DEFAULT 'pending_scope'
        CHECK (status IN ('pending_scope', 'in_scope', 'out_of_scope', 'undetermined')),
    -- Why the scope resolved as it did. REQUIRED for undetermined (see CHECK below): an
    -- undetermined delivery that cannot say what stopped it is visible without being legible,
    -- which satisfies the letter of invariant 6 and none of its purpose.
    scope_reason             TEXT            NULL,
    scoped_at                TIMESTAMPTZ     NULL,

    -- The judgment leg. NULL disposition = not judged, which for an awareness-only subscriber is
    -- the expected terminal state and not a backlog entry.
    disposition              TEXT            NULL
        CHECK (disposition IS NULL OR disposition IN ('acted', 'declined')),
    -- The authored judgment event. A disposition is an ACT on the ledger, not a column write:
    -- this points at the kb_events row that carries the reasoning, so the delivery table's
    -- rationale/confidence are a projection of an authored fact rather than free-floating text.
    decided_by_event_id      UUID            NULL REFERENCES kb_events(id),
    -- The two accountability carriers. At least one is required when a disposition is set; both
    -- may be present (an agent's envelope also names the profile it runs as).
    decided_by_invocation_id UUID            NULL REFERENCES kb_invocations(id),
    decided_by_profile_id    UUID            NULL REFERENCES kb_profiles(id),
    decided_at               TIMESTAMPTZ     NULL,
    -- Reasoning and confidence, mirrored from the judgment event's payload for queryability.
    -- The research-corpus claim (churn x judgment) is a claim about being able to ASK, and an
    -- answer that requires unpacking every event payload is not an answer anyone will get.
    rationale                TEXT            NULL,
    confidence               DOUBLE PRECISION NULL
        CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),

    created                  TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- One delivery per (subscription, event). Intake projects exactly one row per matched
    -- subscription, so a second row for the same pair is a double-projection bug, not a legal
    -- second routing. This also makes the projection safely idempotent under retry.
    UNIQUE (subscription_id, event_id),

    -- A disposition names an actor. This is the clause's load-bearing property expressed as a
    -- constraint: "accountable and citable" fails the moment a judgment exists with nobody
    -- attached to it.
    CONSTRAINT disposition_names_an_actor CHECK (
        disposition IS NULL
        OR decided_by_invocation_id IS NOT NULL
        OR decided_by_profile_id IS NOT NULL
    ),

    -- A disposition carries its reasoning, its confidence, its time, and the event that authored
    -- it. A declined delivery without reasoning is exactly the silent cursor bump the delivery
    -- row exists to make impossible, so the schema refuses it rather than trusting a service.
    CONSTRAINT disposition_carries_its_reasoning CHECK (
        disposition IS NULL
        OR (rationale IS NOT NULL
            AND confidence IS NOT NULL
            AND decided_at IS NOT NULL
            AND decided_by_event_id IS NOT NULL)
    ),

    -- Judgment only follows a resolved scope, and only where there is something to judge.
    -- pending_scope has not been looked at yet; out_of_scope was determined NOT to touch this
    -- subscriber, so there is nothing to act on or decline. in_scope and undetermined are the two
    -- states the steward tick sees (goal invariant 6: undetermined is surfaced too).
    CONSTRAINT disposition_follows_a_resolved_scope CHECK (
        disposition IS NULL
        OR status IN ('in_scope', 'undetermined')
    ),

    -- An undetermined delivery says what stopped it. Visible-but-unexplained is not what
    -- invariant 6 asks for: a consumer must be able to tell "I could not see whether it was"
    -- from "nothing you subscribe to was touched", and the reason is the half that does that.
    CONSTRAINT undetermined_says_why CHECK (
        status <> 'undetermined' OR scope_reason IS NOT NULL
    )
);

-- The C11 read: "what was routed to this declaration, and what became of it?" Newest first, by
-- uuidv7 id (whose byte order IS time order — the same property steward_ingest_delta relies on
-- for max_event_id), so this serves both the listing and the "has this ever matched?" existence
-- probe C12 needs.
CREATE INDEX idx_kb_subscription_deliveries_subscription
    ON kb_subscription_deliveries(subscription_id, id DESC);

-- The reverse leg: "who did this event get routed to?" Used when a single webhook is being
-- examined, and by the projection's own idempotence check.
CREATE INDEX idx_kb_subscription_deliveries_event
    ON kb_subscription_deliveries(event_id);

-- The DLQ. Partial, because undetermined is the rare state and the sweep that surfaces it should
-- not pay for the common ones. This is the index that makes "is anything stuck?" answerable
-- without an agent asking — the job no steward can infer, because two of the three subscriber
-- kinds have no steward at all.
CREATE INDEX idx_kb_subscription_deliveries_undetermined
    ON kb_subscription_deliveries(subscription_id, id DESC)
    WHERE status = 'undetermined';

COMMENT ON TABLE kb_subscription_deliveries IS
  'One row per (subscription, event) the radius matched. Projected in Rust inside the intake transaction — the region_materialized precedent (write.rs:196-204), not a payload-first _project_* half, because the projection halves read only the payload and chunk B''s payload is the remote''s verbatim body. Two lifecycles share the table: a kb_cogmaps subscriber can be acted for under an invocation envelope and runs to acted/declined; a kb_contexts or kb_teams subscriber has no telos, no steward and no envelope, and is terminal at in_scope/undetermined — awareness, not backlog. Also the read surface for goal clause C11: a webhook_received event is unreachable on all three existing kb_events read paths, so a team cannot otherwise see that a webhook it subscribed to ever arrived.';

COMMENT ON COLUMN kb_subscription_deliveries.status IS
  'pending_scope (born here — the coarse radius is payload-only) | in_scope | out_of_scope | undetermined. undetermined is the DLQ marker and a first-class terminal state, NOT an error: goal invariant 6 requires that an enrichment which fails leaves the delivery visible and never silently out_of_scope. It is an ordinary CHECK value precisely so no code path treats it as exceptional and collapses it.';

COMMENT ON COLUMN kb_subscription_deliveries.scope_reason IS
  'Why the scope resolved as it did; REQUIRED when status = undetermined (constraint undetermined_says_why). Visible-but-unexplained does not satisfy invariant 6 — distinguishing "I could not see whether it was" from "nothing was touched" is exactly what the reason carries.';

COMMENT ON COLUMN kb_subscription_deliveries.disposition IS
  'NULL = not judged. For an awareness-only subscriber (kb_contexts, kb_teams) that is the EXPECTED TERMINAL STATE, not a queue entry — the goal''s No phantom backlog negative forbids any surface reporting it as outstanding work. Only in_scope and undetermined deliveries may be disposed (constraint disposition_follows_a_resolved_scope): out_of_scope was determined not to touch this subscriber, so there is nothing to act on or decline.';

COMMENT ON COLUMN kb_subscription_deliveries.decided_by_invocation_id IS
  'NULLABLE by design, with decided_by_profile_id as its peer. kb_invocations.originating_cogmap_id is NOT NULL, so only a kb_cogmaps subscriber can ever be acted for under an envelope; requiring one here would scope the entire lifecycle to one of the three subscriber kinds. The load-bearing property is accountable and citable, not enveloped — constraint disposition_names_an_actor requires at least one carrier.';

COMMENT ON COLUMN kb_subscription_deliveries.decided_by_event_id IS
  'The authored judgment event on kb_events. A disposition is an ACT, not a column write: rationale and confidence here are a queryable projection of that event''s payload, not free-floating text. Required whenever a disposition is set.';

COMMENT ON COLUMN kb_subscription_deliveries.confidence IS
  'The judgment''s confidence in [0,1], mirrored from the judgment event for queryability. The research-corpus claim (churn x judgment) is a claim about being able to ASK; an answer requiring every event payload to be unpacked is not one anyone will get.';

-- ---------------------------------------------------------------------------
-- The disposition event type.
--
-- Unlike webhook_received (20260819000020), this one is TYPED with a published payload_schema.
-- That migration's reasoning for going permissive was specific and does not transfer: "the
-- payload is the remote's verbatim body, which has no fixed shape temper can publish." A
-- disposition is temper's OWN act, with a shape temper defines and controls, so the permissive
-- path would be dishonest here. The schema below is the committed schemars snapshot of
-- payloads::SubscriptionDeliveryDisposed — repo == registry == Rust types (payload spec §6
-- chain), enforced by tests/payload_schema.rs.
--
-- category = 'domain', matching webhook_received and for the same reason: a disposition is the
-- ledger's subject matter — judgment made on the unjudged record — not an authority act.
-- Category is spelled at registration, never stamped afterwards (20260719000010 dropped the
-- column DEFAULT so an omitting registration fails loudly).
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('subscription_delivery_disposed', $JS${
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "SubscriptionDeliveryDisposed",
  "description": "`subscription_delivery_disposed` — a steward's judgment on one routed event (S2 chunk C).\n\nThe disposition is an **act**, not a column write. `acted` cites what was authored; `declined`\nrecords that the delivery was judged immaterial, *with* its reasoning and confidence — so a\ndecline is accountable and citable rather than a silent cursor bump. The delivery row's\n`rationale`/`confidence` columns are a queryable projection of this payload, not a second\nsource of truth.\n\n**The accountability carrier is the actor, not the envelope.** `decided_by_profile_id` is\nalways present; `decided_by_invocation_id` is present only when an agent acted for a cogmap,\nbecause `kb_invocations.originating_cogmap_id` is `NOT NULL` and a `kb_contexts` or `kb_teams`\nsubscriber can never originate one. Requiring an envelope here would scope judgment to one of\nthe three subscriber kinds while reading as though it covered all of them — which is exactly\nwhat the design said until the chunk C grounding pass caught it.",
  "type": "object",
  "properties": {
    "confidence": {
      "description": "The judgment's confidence in `[0,1]`.",
      "type": "number",
      "format": "double"
    },
    "decided_by_invocation_id": {
      "description": "Present only when an agent acted for a cogmap; `None` for a human disposition.",
      "type": [
        "string",
        "null"
      ],
      "format": "uuid"
    },
    "decided_by_profile_id": {
      "$ref": "#/$defs/ProfileId"
    },
    "delivery_id": {
      "type": "string",
      "format": "uuid"
    },
    "disposition": {
      "description": "`acted` | `declined`.",
      "type": "string"
    },
    "event_id": {
      "type": "string",
      "format": "uuid"
    },
    "rationale": {
      "description": "Why. Required — a disposition without reasoning is the silent cursor bump this event type\nexists to prevent.",
      "type": "string"
    },
    "subscription_id": {
      "type": "string",
      "format": "uuid"
    }
  },
  "required": [
    "delivery_id",
    "subscription_id",
    "event_id",
    "disposition",
    "rationale",
    "confidence",
    "decided_by_profile_id"
  ],
  "$defs": {
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    }
  }
}$JS$, 1, 'domain')
ON CONFLICT (name) DO UPDATE
  SET payload_schema = EXCLUDED.payload_schema,
      schema_version = EXCLUDED.schema_version,
      category       = EXCLUDED.category;

SELECT declare_migration(
    20260819000030,
    'additive',
    'Adds kb_subscription_deliveries (S2 chunk C): one row per (subscription, event) the radius matched, projected in Rust inside the intake transaction following the region_materialized precedent (write.rs:196-204) rather than a payload-first _project_* half, because the projection halves read only the payload and chunk B''s payload is the remote''s verbatim body. Registers subscription_delivery_disposed as a TYPED event type (published payload_schema from the committed schemars snapshot, category=domain) — unlike webhook_received, a disposition is temper''s own act with a shape temper controls, so the permissive path does not apply. decided_by_invocation_id is nullable with decided_by_profile_id as its peer: kb_invocations.originating_cogmap_id is NOT NULL, so requiring an envelope would scope judgment to kb_cogmaps subscribers alone and exclude kb_contexts/kb_teams, which subscribe to be aware rather than to judge. Four CHECK constraints carry the invariants: a disposition names an actor, carries its reasoning, follows a resolved scope (never pending_scope or out_of_scope), and an undetermined delivery says why. Additive-only on main: new table plus one new event-type row; no existing object altered, no existing row invalidated, and the pre-deploy binary neither reads nor writes either.'
);
