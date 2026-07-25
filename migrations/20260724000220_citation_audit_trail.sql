-- The per-audit attribution read — who audited what (Set 5 per-auditor collapse, Beat 3; spec
-- `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §4.1, §5.2).
--
-- WHAT IS MISSING TODAY. The evidence surface returns AGGREGATES only. `resource_standing_shape`
-- (`20260724000120_standing_citation_components.sql:383-403`) yields `citation_magnitude`,
-- `audit_coverage`, `citation_quality` and a `band` chip — every one of them a scalar collapsed over
-- the whole trail. So an audit that drops a finding to `disputed` is UNATTRIBUTABLE from the read
-- surface: the verdict is visible, the voter never is. Beat 1 put the voter on the row
-- (`20260724000200_citation_audit_attribution.sql`: `kb_citation_audits.audited_by_profile_id`); this
-- migration is the read that lets anything outside the database see it.
--
-- WHY A SIBLING READ RATHER THAN MORE COLUMNS ON THE SHAPE. `resource_standing_shape` is a
-- FIXED-WIDTH row recomputed live on every call — that is why it can be cheap and why the service
-- never reads the memo (`20260724000120:376-379`). A trail is VARIABLE-LENGTH and grows with every
-- audit ever emitted; folding it into the shape would make the cheap read pay for the expensive one
-- on every `/evidence` call, whether or not the caller wants attribution. Opt-in, therefore a
-- separate function.
--
-- ── THE GATE IS THE INCUMBENT, VERBATIM ───────────────────────────────────────────────────────────
-- The `gated` CTE below is carried over BYTE-FOR-BYTE from `resource_standing_shape`
-- (`20260724000120_standing_citation_components.sql:387-390`, itself carried byte-for-byte from
-- `20260721000010_evidential_standing_memo.sql:239-242`). Same predicate
-- (`resources_readable_by` — the FULL canonical visibility function, not a subset), same parameter
-- triple `(p_finding, p_principal_kind, p_principal_id)`, same call shape from Rust
-- (`… ($1, 'profile', $2)`, `temper-substrate/src/readback/mod.rs:958-966`). There is ONE spelling of
-- this gate in the codebase and a second one would be the drift this rule exists to prevent.
--
-- Consequently ZERO ROWS is a THREE-WAY-INDISTINGUISHABLE answer at this layer: the finding is
-- unreadable, the finding does not exist, or the finding is readable and simply carries no audits.
-- The SQL deliberately cannot tell a caller which — that is the same leak-safety `/evidence` relies
-- on ("a denied and an absent finding are indistinguishable",
-- `temper-services/src/services/evidential_standing_service.rs:11-12`). The surface resolves
-- empty-vs-404 by asking the readability question SEPARATELY, through the same predicate's
-- Rust-callable spelling (`readback::is_resource_visible`, `readback/mod.rs:146`) — never by reading
-- a discriminator out of this function.
--
-- ── THE TRAIL IS THE TRAIL BEHIND THE NUMBER ──────────────────────────────────────────────────────
-- Rows are joined through `resource_live_citations` (`20260724000120:103-111`) — "the one definition
-- the three standing axes share" — on the FULL citation key, with the same three-conjunct join
-- `resource_audit_coverage` uses (`:146-151`) and `resource_citation_quality` repeats (`:213-216`):
--     a.block_id = c.block_id AND a.source_kind = 'resource' AND a.source_id = c.source_id
-- That is not join hygiene, it is the read's meaning. `a.block_id = c.block_id` is load-bearing for
-- exactly the reason `resource_audit_coverage`'s header gives (`:132-136`): one source is routinely
-- cited by several findings, so dropping the block conjunct would show F2's trail an audit that was
-- emitted against F1's citation of the same source. And joining through the live-citation set rather
-- than over `kb_content_blocks` directly makes this read return PRECISELY the audits that the
-- coverage and quality axes summed — the attribution behind the band, not a superset of it. An audit
-- of a citation that has since been corrected, or whose source has been soft-deleted, moves no
-- standing number, so surfacing it here would explain a verdict with evidence that did not produce
-- it. The ledger keeps that row forever (append-only, `20260724000110:14-19`); this read is a
-- projection of the ledger, not the ledger.
--
-- ── ORDER: MOST RECENT FIRST ──────────────────────────────────────────────────────────────────────
-- `ORDER BY a.created DESC, a.id DESC`. Most-recent-first is the useful default for a trail whose
-- aggregate is decay-weighted by age (`resource_citation_quality` weights each audit by
-- `pow(0.5, age/30d)`, `20260724000120:196-206`): the rows at the head of this list are the rows
-- carrying the most weight in the number the caller just read. `a.id` breaks ties DETERMINISTICALLY
-- rather than decoratively — two audits can share a `created` (it comes from the owning event's
-- `occurred_at`, `20260724000110:68-78`, and one transaction can emit several), and without the
-- tiebreak their relative order would be plan-dependent, so a paging or diffing caller could see two
-- different orders for the same unchanged trail. `id` is UUIDv7, so `id DESC` is itself
-- newest-first — the tiebreak agrees with the primary key rather than contradicting it.
--
-- ── NO NEW INDEX, AND THAT IS A DECISION ──────────────────────────────────────────────────────────
-- The join predicate is a full three-column match on `idx_kb_citation_audits_citation(block_id,
-- source_kind, source_id)` (`20260724000110:39`) — the same access path `resource_audit_coverage`
-- and `resource_citation_quality` already take against this table. This read adds a `kb_profiles`
-- lookup by primary key. Nothing here asks for an index that does not exist; adding one on
-- `audited_by_profile_id` would serve a "what has THIS auditor audited" query that no caller issues
-- yet, and every audit write would pay its maintenance. Add it when a measured plan asks.

CREATE FUNCTION resource_citation_audit_trail(
    p_finding uuid, p_principal_kind text, p_principal_id uuid
) RETURNS TABLE(audit_id uuid, block_id uuid, source_id uuid, value double precision,
                reason text, created timestamptz, auditor_profile_id uuid,
                auditor_handle text, auditor_display_name text)
LANGUAGE sql STABLE AS $$
    WITH gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ),
    citations AS (
        SELECT c.block_id, c.source_id
        FROM gated g, LATERAL resource_live_citations(g.fid) c
    )
    SELECT a.id, a.block_id, a.source_id, a.value, a.reason, a.created,
           a.audited_by_profile_id, pr.handle, pr.display_name
    FROM citations c
    JOIN kb_citation_audits a
      ON a.block_id    = c.block_id
     AND a.source_kind = 'resource'
     AND a.source_id   = c.source_id
    JOIN kb_profiles pr ON pr.id = a.audited_by_profile_id
    ORDER BY a.created DESC, a.id DESC;
$$;

COMMENT ON FUNCTION resource_citation_audit_trail(uuid, text, uuid) IS
  'One row per citation audit of a finding, with the auditor''s profile id, handle and display_name '
  'joined in — the attribution behind resource_standing_shape''s aggregates (spec §4.1, §5.2). '
  'Access-gated INSIDE the SQL by the same `gated` CTE over resources_readable_by that '
  'resource_standing_shape uses, so an unreadable finding, an absent finding and a readable finding '
  'with no audits are all ZERO ROWS and indistinguishable here. Rows are the audits that the '
  'coverage/quality axes actually summed: joined through resource_live_citations on the full '
  'citation key (block, ''resource'', source). Most recent first, tie-broken on the UUIDv7 id.';
