-- =============================================================================
-- Consolidate kb_doc_types to canonical six
-- =============================================================================
-- Target types: task, goal, session, research, decision, concept
-- Removes: ticket, milestone, board, source, resource
-- Reclassifies any orphaned kb_resources to research before deletion.
--
-- Idempotent: uses IF EXISTS / ON CONFLICT guards so this is safe on both
-- fresh databases and production (where types were already manually deleted).

-- ─── 1. Add "decision" type ─────────────────────────────────────────────────
INSERT INTO kb_doc_types (id, name)
VALUES ('00000000-0000-0000-0001-00000000000b', 'decision')
ON CONFLICT (name) DO NOTHING;

-- ─── 2. Reclassify "resource" → "research" ─────────────────────────────────
-- Well-known IDs: resource = 0...0a, research = 0...04
UPDATE kb_resources
   SET kb_doc_type_id = '00000000-0000-0000-0001-000000000004'
 WHERE kb_doc_type_id = '00000000-0000-0000-0001-00000000000a';

-- ─── 3. Reclassify remaining stale types → "research" ──────────────────────
-- ticket (01), milestone (03), board (05), source (07)
UPDATE kb_resources
   SET kb_doc_type_id = '00000000-0000-0000-0001-000000000004'
 WHERE kb_doc_type_id IN (
     '00000000-0000-0000-0001-000000000001',  -- ticket
     '00000000-0000-0000-0001-000000000003',  -- milestone
     '00000000-0000-0000-0001-000000000005',  -- board
     '00000000-0000-0000-0001-000000000007'   -- source
 );

-- ─── 4. Catch-all: reclassify ANY orphan doc_type_id ────────────────────────
-- Safety net for manually-deleted types or future drift.
UPDATE kb_resources
   SET kb_doc_type_id = '00000000-0000-0000-0001-000000000004'
 WHERE kb_doc_type_id NOT IN (SELECT id FROM kb_doc_types);

-- ─── 5. Delete removed types ────────────────────────────────────────────────
DELETE FROM kb_doc_types
 WHERE name IN ('ticket', 'milestone', 'board', 'source', 'resource');
