-- Post-backfill: every chunk now has first_revision_id populated. Tighten the
-- column to NOT NULL so future inserts are forced to supply it.
ALTER TABLE kb_chunks
    ALTER COLUMN first_revision_id SET NOT NULL;
