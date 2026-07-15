-- Connection excess-reach affirmation (S1 chunk B3): record that a profile deliberately
-- affirmed binding this connection's coarse remote reach to a team.
--
-- Additive and all-nullable: existing rows are untouched (all three columns are NULL, which
-- is the honest "never affirmed" state). Additive-only ⇒ `main` stays auto-deployable; this
-- lands on the live, prod-migrated `kb_connections` table (born in 20260714000010_connections.sql).
--
-- NO enforcement here — this is the schema + read-side only. Beat 2 wires the affirmation into
-- the grant path. What this migration adds is the RECORD, not the rule.
ALTER TABLE kb_connections
  ADD COLUMN reach_affirmed_by  UUID        NULL REFERENCES kb_profiles(id),
  ADD COLUMN reach_affirmed_at  TIMESTAMPTZ NULL,
  ADD COLUMN reach_affirmation  TEXT        NULL;

COMMENT ON COLUMN kb_connections.reach_affirmed_by IS
  'The profile who affirmed (the WHO) that binding this connection''s coarse remote reach to a team is intentional. NULL = never affirmed: the connection either declares no reach, or no grant requiring affirmation has happened yet. This does NOT fix the privilege asymmetry between coarse remote scope and precise temper scope — remote and temper scope are incommensurable, and there is deliberately NO computed exceeds_temper_reach bool anywhere. It makes the asymmetry a declared, reviewable property of the connection instead of a latent surprise — the most an honest brokering design can offer. Recorded on the connection (single-valued, last-writer): the connection''s most-recent affirmation of intent, an audit stamp, not a per-grant ledger.';

COMMENT ON COLUMN kb_connections.reach_affirmed_at IS
  'When the affirmation was made (the WHEN). NULL = never affirmed. Paired with reach_affirmed_by / reach_affirmation as a single, last-writer affirmation stamp — not a per-grant history.';

COMMENT ON COLUMN kb_connections.reach_affirmation IS
  'The stated rationale (the WHY) — why binding this connection''s coarse remote reach to a team is intentional. NULL = never affirmed. This is the reviewable declaration that turns the latent remote/temper reach asymmetry into an on-the-record property of the connection; it does not compute or resolve the asymmetry (no exceeds bool exists), it only makes it deliberate and attributable.';
