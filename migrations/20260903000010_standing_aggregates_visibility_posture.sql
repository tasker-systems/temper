-- The stored-aggregates exception, stated at the fields it excepts.
--
-- Every COMMENT below carries the posture sentence verbatim:
--   "Reader-independent by decision: one value for every caller who can read the finding, with
--    per-contributor attribution on the citation-audit trail, never on the shape
--    (stored-aggregates exception: internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md)"
-- (resource_citation_audit_trail carries the attribution-door variant of the same sentence).
--
-- THE CONSTRAINT A LATER EDIT MUST NOT BREAK: any future COMMENT on one of these functions carries
-- the sentence forward -- it is what a `\df+` reader with no git access sees in place of the
-- record, and `evidential_standing.rs` asserts its presence on all nine. Scope, rationale and the
-- re-review triggers live in the record; nothing here re-argues them. Metadata-only: no signature,
-- return column, table or function-body changes.

COMMENT ON FUNCTION resource_standing_shape(uuid, text, uuid) IS
  'The finding''s evidential-standing shape: six components plus the lossy band, recomputed live '
  'at read -- never the kb_resource_standing memo. Zero rows unless the caller reads the finding: '
  'the gate (resources_readable_by) is inside this function, so an unreadable finding is '
  'indistinguishable from an absent one. Reader-independent by decision: one value for every '
  'caller who can read the finding, with per-contributor attribution on the citation-audit trail, '
  'never on the shape (stored-aggregates exception: '
  'internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).';

COMMENT ON FUNCTION resource_live_citations(uuid) IS
$c$The finding's live citations as (block, source) pairs -- the one definition the three standing
axes share (spec §3.1), plus the audit trail and the tier-1 staleness predicate.

Distinct sources that are both LIVE and COMPLETE. Soft-deleted sources are excluded because
soft-delete does not fold blocks or provenance, so the provenance row outlives the source. In-flight
sources are excluded by 20260727000020 because an interrupted ingest is not a document: its
citation set is still forming, and weighing a verdict against a passage that may not be final spends
the verdict on nothing. Exclusion here is not deletion -- `ingest_state` only moves
`in_progress` -> `complete`, so the source re-enters every axis when it finalizes.

KNOWN ASYMMETRY, WIDENED HERE, NOT INTRODUCED. `citation_is_live` -- the gate `citation_audit` runs
before accepting a verdict (20260724000110) -- reads `kb_block_provenance` alone. It checks neither
`is_active` nor `ingest_state`, so an audit may still be WRITTEN against a citation this function
does not RETURN, and that audit is inert for standing. That was already true for soft-deleted sources
before this migration; completeness now joins liveness in the same gap. Closing it means deciding
whether the write gate should mirror the read producer or stay deliberately permissive (an
append-only trail arguably should record the attempt), which is its own task.

Reader-independent by decision: one value for every caller who can read the finding, with
per-contributor attribution on the citation-audit trail, never on the shape (stored-aggregates
exception: internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).$c$;

COMMENT ON FUNCTION resource_citation_magnitude(uuid) IS
  'Count of DISTINCT LIVE cited sources of the finding (spec §3.1) -- the findability axis, '
  'monotone by design. Deliberately NOT r_parent, which counts every provenance row, duplicates '
  'included: ten citations of one source is r_parent = 10, citation_magnitude = 1. '
  'Reader-independent by decision: one value for every caller who can read the finding, with '
  'per-contributor attribution on the citation-audit trail, never on the shape (stored-aggregates '
  'exception: internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).';

COMMENT ON FUNCTION resource_audit_coverage(uuid) IS
  'Count of distinct cited sources carrying at least one citation audit (spec §3.1) -- the '
  'evaluated-ness axis, monotone under the append-only trail. An audit counts only for the '
  'citation it was emitted against: the (block, source) key is load-bearing, so an audit of one '
  'finding''s citation never covers a second finding that cites the same source. '
  'Reader-independent by decision: one value for every caller who can read the finding, with '
  'per-contributor attribution on the citation-audit trail, never on the shape (stored-aggregates '
  'exception: internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).';

COMMENT ON FUNCTION resource_citation_quality(uuid) IS
  'THREE-stage decay-weighted audit mean (spec §3.1, per-auditor collapse). (1) Collapse within an '
  'AUDITOR per source -- decay-weighted, so one principal''s N audits count once and its newest audit '
  'dominates its own older ones (the retraction mechanic, with no supersession). (2) Collapse across '
  'auditors per source, WEIGHTED BY EACH AUDITOR''S FRESHEST-AUDIT WEIGHT, not a plain mean: one vote '
  'per principal AND decay still arbitrating between competing auditors -- a plain mean would let a '
  'two-year-old verdict count equally with today''s. (3) Plain mean across distinct AUDITED sources. '
  'The stage order is what keeps a multi-block source, and now a persistent auditor, from voting more '
  'than once. A source every one of whose auditors has decayed to zero weight still yields NULL and '
  'drops out of the outer mean rather than reading 0.0 -- the divide guard is nullif(sum(...), 0) at '
  'BOTH collapse stages, and stage 2''s is sufficient because a NULL auditor_value implies that '
  'auditor''s max(w) is 0, so it adds nothing to the denominator either. '
  'Reader-independent by decision: one value for every caller who can read the finding, with '
  'per-contributor attribution on the citation-audit trail, never on the shape (stored-aggregates '
  'exception: internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).';

COMMENT ON FUNCTION resource_contradiction_balance(uuid) IS
  'Supports minus contradicts over the finding''s live express edges -- a vector-sum of edge '
  'weights over the support/oppose label set (supports, corroborates, contradicts), not a '
  'headcount. Reader-independent by decision: one value for every caller who can read the finding, '
  'with per-contributor attribution on the citation-audit trail, never on the shape '
  '(stored-aggregates exception: '
  'internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).';

COMMENT ON FUNCTION resource_freshness(uuid) IS
  'Reversible time-decay off the finding''s most recent uncorrected reinforcement -- 1.0 just after '
  'reinforcement, 30-day half-life, 0.0 when never reinforced. Recomputed live at read; the memo '
  'column is a snapshot, never the read''s authority. Reader-independent by decision: one value '
  'for every caller who can read the finding, with per-contributor attribution on the '
  'citation-audit trail, never on the shape (stored-aggregates exception: '
  'internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).';

COMMENT ON FUNCTION resource_r_parent(uuid) IS
  'Reinforcement breadth: count of uncorrected provenance rows over the finding''s live blocks, '
  'duplicates included -- deliberately NOT citation_magnitude, which counts distinct live sources. '
  'Reader-independent by decision: one value for every caller who can read the finding, with '
  'per-contributor attribution on the citation-audit trail, never on the shape (stored-aggregates '
  'exception: internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).';

COMMENT ON FUNCTION resource_citation_audit_trail(uuid, text, uuid) IS
$c$One row per citation audit of a finding, with the auditor's profile id, handle and display_name
joined in -- the attribution behind resource_standing_shape's aggregates (spec §4.1, §5.2).
Access-gated INSIDE the SQL by the same `gated` CTE over resources_readable_by that
resource_standing_shape uses, so an unreadable finding, an absent finding and a readable finding
with no audits are all ZERO ROWS and indistinguishable here. Rows are the audits that the
coverage/quality axes actually summed: joined through resource_live_citations on the full
citation key (block, 'resource', source). Most recent first, tie-broken on the UUIDv7 id.

Reader-independent by decision: one value for every caller who can read the finding, with
per-contributor attribution on the citation-audit trail, never on the shape -- this function IS
that attribution door (stored-aggregates exception:
internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).$c$;

SELECT declare_migration(
    20260903000010,
    'additive',
    'Metadata-only posture statements: COMMENT ON FUNCTION for resource_standing_shape, its six component producers plus the shared live-citation producer, and resource_citation_audit_trail. The stored-aggregates exception (internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md, amended 2026-09-03) requires the visibility posture to be stated at the field, and these COMMENTs are what a df+ reader with no git access sees in place of the record. Additive: no signature, return column, table or function-body changes; no query text changes, so no .sqlx entry moves.'
);
