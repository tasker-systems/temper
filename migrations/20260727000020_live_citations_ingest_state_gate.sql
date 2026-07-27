-- `resource_live_citations`: an interrupted ingest is not a citable source.
--
-- THE DEFECT. CLAUDE.md states the rule without qualification -- "`ingest_state = 'complete'` goes
-- exactly where `r.is_active` already goes". `resource_live_citations` (`20260724000120`) is the ONE
-- shared definition of "a live citation of this finding", and it gates the source on `is_active`
-- and nothing else. So a segmented upload still in flight -- a resource excluded from list and
-- search precisely because an interrupted ingest is not a document -- counts toward a finding's
-- `citation_magnitude`, is offered for audit, and sits in `citation_quality`'s denominator.
--
-- ONE EDIT, FIVE CONSUMERS. Every reader inherits the gate from the shared producer; none of them
-- needed touching (`pg_proc`, 2026-07-27):
--   `resource_citation_magnitude` · `resource_audit_coverage` · `resource_citation_quality`
--   `resource_citation_audit_trail` · `resource_has_stale_citation`
-- That is all three evidential-standing axes, the attributed read surface, and the tier-1 auditor
-- staleness predicate. Fixing the producer is the whole fix; fixing any consumer is the wrong shape.
--
-- MONOTONICITY IS PRESERVED, AND THAT IS WHY THIS GATE IS SAFE ON A SHIPPED AXIS. `ingest_state`
-- only ever moves `in_progress` -> `complete` (`20260714000001`), so a gated source APPEARS when it
-- finalizes and never disappears. `citation_magnitude` stays the monotone findability axis it is
-- documented to be. (`is_active` already breaks strict monotonicity on soft-delete; this adds
-- nothing new.)
--
-- NO BACKFILL, AND NONE IS NEEDED. `kb_resource_standing`'s columns are a write-cost memo;
-- `resource_standing_shape` recomputes all three axes live at read and the memo is never the read's
-- authority. Standing for any finding citing an in-flight source therefore corrects itself on the
-- next read, with no migration-time data pass.
--
-- SCOPE: THE SOURCE SIDE ONLY. The finding's own `ingest_state` is deliberately NOT gated here.
-- Callers that need it already apply it -- `audit_drift_sweep`'s `candidates` does, citing the same
-- rule -- and whether `resource_standing_shape` should return zeros for a half-uploaded FINDING is a
-- different question about a different surface. Widening this producer would decide it silently.
--
-- ADDITIVE. `CREATE OR REPLACE`; signature and return columns unchanged. Precedent and rationale:
-- `20260727000010`.

CREATE OR REPLACE FUNCTION resource_live_citations(p_finding uuid)
RETURNS TABLE(block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT p.block_id, p.source_id
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    -- Two separate questions, both asked of the SOURCE: `is_active` asks "does this still exist?",
    -- `ingest_state` asks "are all its bytes here?". Neither implies the other.
    JOIN kb_resources src ON src.id = p.source_id
                         AND src.is_active
                         AND src.ingest_state = 'complete'
    WHERE b.resource_id = p_finding AND NOT b.is_folded
      AND p.source_kind = 'resource';
$$;

COMMENT ON FUNCTION resource_live_citations(uuid) IS
$c$The finding's live citations as (block, source) pairs -- the one definition the three standing
axes share (spec §3.1), plus the audit trail and the tier-1 staleness predicate.

Distinct sources that are both LIVE and COMPLETE. Soft-deleted sources are excluded because
soft-delete does not fold blocks or provenance, so the provenance row outlives the source. In-flight
sources are excluded by `20260727000020` because an interrupted ingest is not a document: its
citation set is still forming, and weighing a verdict against a passage that may not be final spends
the verdict on nothing. Exclusion here is not deletion -- `ingest_state` only moves
`in_progress` -> `complete`, so the source re-enters every axis when it finalizes.

KNOWN ASYMMETRY, WIDENED HERE, NOT INTRODUCED. `citation_is_live` -- the gate `citation_audit` runs
before accepting a verdict (`20260724000110`) -- reads `kb_block_provenance` alone. It checks neither
`is_active` nor `ingest_state`, so an audit may still be WRITTEN against a citation this function
does not RETURN, and that audit is inert for standing. That was already true for soft-deleted sources
before this migration; completeness now joins liveness in the same gap. Closing it means deciding
whether the write gate should mirror the read producer or stay deliberately permissive (an
append-only trail arguably should record the attempt), which is its own task.$c$;
