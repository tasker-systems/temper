-- When an inbound webhook attestation was first accepted, so a repeat can be judged by its age.
--
-- `kb_inbound_attestations_seen` records each accepted attestation so a repeat is recognised.
-- Connect sends one attestation for several distinct deliveries sent close together: a merge-queue
-- merge's `pull_request` `closed` and `dequeued` arrive under the same one, with different bodies,
-- under a second apart (task 01a12e27). A repeat that close to the first is one of those; a repeat
-- long after it is a captured attestation presented again. The intake accepts a repeat within a
-- short window of `first_seen_at` and treats a later one as a replay.
--
-- The intake writes the column from its own clock, the same clock that judged the attestation
-- fresh. The default fills rows that predate this migration, and rows a binary without it writes:
-- such a row's window starts when the row was written, which is when that binary accepted it.

ALTER TABLE kb_inbound_attestations_seen
    ADD COLUMN first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now();

COMMENT ON COLUMN kb_inbound_attestations_seen.first_seen_at IS
  'When the attestation was first accepted. A repeat within the intake''s window of this is a burst '
  'from Connect; a later one is a replay.';

SELECT declare_migration(
    20261031100000,
    'additive',
    'Adds first_seen_at to kb_inbound_attestations_seen, with a default of now(), so the webhook '
    'intake can tell a repeat sent in the same burst from one presented later. Existing rows and rows '
    'a binary without this migration writes take the default. Nothing existing is read differently.'
);
