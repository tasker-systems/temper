-- The admin ledger's epoch (spec 2026-07-16 §8).
--
-- NOT a backfill. ~14 admin acts occurred in prod before any writer existed and 8 of them are
-- permanently unreconstructable: kb_teams has no creator column at all, kb_team_members has no
-- actor, and revoked grants were hard-DELETEd. The 6 that "survive" don't either -- the grant
-- upsert overwrites granted_by_profile_id AND sets granted_at = now(), so those columns are a
-- current snapshot, not history. Synthesizing events from them would mint immortal, append-only
-- rows asserting the wrong actor at a fabricated time.
--
-- A partially-backfilled ledger is WORSE than an honestly-empty one: a reader cannot distinguish
-- "no event" from "predates the writer" from "reconstruction with the wrong actor". An empty
-- ledger with an epoch is unambiguous.
--
-- Emitted by the system actor -- the bare `system` entity, which never resolves through
-- resolve_emitter (20260624000003_canonical_seed.sql). Both-NULL producing anchor: the epoch has
-- no cognition home, and neither will any admin event after it (the cognition firewall). Its
-- EventKind::AdminLedgerOpened variant + replay no-op arm ride in this same migration's PR, so
-- replay never sees an unknown type.

-- Payload is `{ note }` only — the epoch's TIME is the event's own occurred_at (ledger_epoch reads
-- it there), and payloads::AdminLedgerOpened deliberately carries no opened_at (a timestamp in a
-- payload duplicates occurred_at; this module's rule keeps derived/carried state out of payloads).
INSERT INTO kb_events (event_type_id, emitter_entity_id, payload, "references")
SELECT et.id,
       e.id,
       jsonb_build_object(
         'note', 'Admin ledger opens here. No administrative history exists before this event: '
              || 'the acts happened, but no writer recorded them and their actors are not '
              || 'reconstructable from surviving columns.'
       ),
       '[]'::jsonb
  FROM kb_event_types et
  CROSS JOIN kb_entities e
 WHERE et.name = 'admin_ledger_opened'
   AND e.name  = 'system'
   AND NOT EXISTS (
         SELECT 1 FROM kb_events x JOIN kb_event_types xt ON xt.id = x.event_type_id
          WHERE xt.name = 'admin_ledger_opened'
       );
