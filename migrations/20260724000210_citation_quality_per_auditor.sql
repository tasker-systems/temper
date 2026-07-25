-- Per-auditor collapse: the third stage of `resource_citation_quality` (Set 5 per-auditor collapse,
-- Beat 2; spec `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §3.1, §4.1).
--
-- THE DEFECT, IN THE FUNCTION'S OWN WORDS. `resource_citation_quality` shipped deliberately
-- TWO-stage (`20260724000120_standing_citation_components.sql:154-224`) and its header says why:
-- *"a naive single-stage weighted mean over the flat provenance-joined-to-audits row set gives a
-- source cited by five blocks five votes in the outer mean — the echo/actor-count fallacy re-entering
-- through block multiplicity, in the very function written to exclude it"* (`:159-162`); the
-- `COMMENT ON FUNCTION` states the payoff as *"the stage order is what keeps a multi-block source
-- from voting more than once"* (`:226-229`). The principle was generalized to SOURCES and stopped
-- there. Stage 1 groups by `source_id` alone (`:218-222`), so within a source it is a weighted mean
-- over every audit that source carries FROM EVERY PRINCIPAL — and one principal can carry as many of
-- them as it likes. The identical fallacy, one axis over: persistence buys influence.
--
-- MEASURED, NOT ARGUED. Both bodies were created side by side in one transaction against the dev
-- database, fed the identical synthetic rows through the real `resource_live_citations` join, and
-- printed together (ROLLBACKed; the fixture is reproduced by the tests this migration ships with):
--
--   scenario                                                  shipped 2-stage   this migration
--   10x(-1.0) from ONE griefer vs 1x(+1.0) from one honest        -0.818182         0.000000
--   1x(-1.0) vs 1x(+1.0)   [the honest 1-vs-1 baseline]            0.000000         0.000000
--
-- `-0.818182` is `disputed` — the band gate turns `citation_quality <= 0.0` into it (`:312-314`) and
-- `-0.82` is not near the boundary, it is most of the way to the floor of the scale. Ten copies of
-- one opinion outvoted one honest verdict by 9:1 in the aggregate, for the cost of nine `POST`s. That
-- is the *whole adversarial premise* inverted: the axis built to record "an adversary examined this
-- and it did not hold up" instead records "somebody was persistent." After this migration the volume
-- case reads exactly what the 1-vs-1 case reads, which is the honest answer — two principals
-- disagreed, and the trail says so.
--
-- WHAT MAKES THIS EXPRESSIBLE AT ALL is Beat 1's `kb_citation_audits.audited_by_profile_id`
-- (`20260724000200_citation_audit_attribution.sql:69-87`) — `NOT NULL`, derived by the projector from
-- the owning event's `emitter_entity_id -> kb_entities.profile_id`, and immutable by construction
-- (`kb_events` is append-only; nothing reassigns `kb_entities.profile_id`). That migration changed no
-- standing number and said so (`:22-23`: *"That collapse is the NEXT migration's job. This one only
-- makes it expressible."*). This is that migration.
--
-- ── THE THREE STAGES ──────────────────────────────────────────────────────────────────────────
-- Stage 1, `per_auditor` — collapse WITHIN an auditor, per source: the decay-weighted aggregate of
--   every audit THIS principal has emitted against THIS source's citations. One value per
--   (source, auditor). This is what makes one principal's N audits count once.
--
--   IT IS ALSO THE RETRACTION MECHANIC, and that is not a side effect — it is the reason the collapse
--   is decay-weighted rather than a plain within-auditor mean. Because the weight is `pow(0.5, age)`,
--   an auditor's NEWEST audit dominates its own older ones: re-auditing is how a principal changes its
--   mind. Spec §4.1 forbids supersession outright (*"a later +1.0 never erases an earlier -1.0"*;
--   `20260724000110_citation_audits.sql:12-17` — no `is_superseded` column, no unique index on the
--   citation key), so "change your mind" HAS to be expressible as an append. Both rows stay permanent;
--   only their influence differs.
--
-- Stage 2, `per_source` — collapse ACROSS auditors, WEIGHTED BY EACH AUDITOR'S FRESHEST-AUDIT WEIGHT
--   (`max(w1.w)`, which is the freshest because `w` is monotone decreasing in age). One value per
--   distinct source. See the next block — this is the load-bearing judgment of the migration.
--
-- Stage 3, the outer `avg` — the plain mean across distinct AUDITED sources. Unchanged from the
--   shipped stage 2, and it must stay a plain mean: sources are not competing verdicts on one
--   question, they are separate pieces of evidence, so there is nothing for decay to arbitrate.
--
-- ── WHY STAGE 2 IS WEIGHTED AND NOT A PLAIN MEAN ──────────────────────────────────────────────
-- The shipped header states an invariant at `20260724000120_standing_citation_components.sql:177`:
-- *"decay only arbitrates BETWEEN competing audits."* Two auditors disagreeing about one source ARE
-- competing audits — the textbook case — so decay must arbitrate there or the sentence is false.
--
-- A plain `avg` across auditors silently destroys it, and again this was measured rather than
-- reasoned about. A third body, three-staged but with `avg(auditor_value)` at stage 2, was created in
-- the same transaction and fed the same rows:
--
--   scenario                                        2-stage    weighted (shipped here)   plain-mean
--   stale +1.0 (auditor X @365d) vs fresh -1.0 (Y)   -0.999565        -0.999565           0.000000
--
-- The plain mean reads a dead heat between a verdict from last year and a verdict from today. Worse,
-- it is a REGRESSION: the shipped two-stage function gets this case right (`-0.999565`), because with
-- one audit each the flat weighted mean already arbitrates by recency. Collapsing per auditor without
-- re-weighting would therefore fix the volume attack by breaking the recency guarantee — trading one
-- defect for another and calling it a repair.
--
-- Both properties are required and the weighted form is what carries both at once:
--   * exactly one vote per principal (each auditor contributes ONE term, whatever its audit count);
--   * that vote's influence still decays (its term is scaled by its own freshest weight).
-- Do not "simplify" this to `avg(auditor_value)`; the two-line diff reintroduces a fixed defect.
-- `citation_audits.rs::a_stale_verdict_does_not_outweigh_a_fresh_one_from_another_auditor` exists to
-- red exactly that edit.
--
-- ── NULL HANDLING IS PRESERVED EXACTLY, AND THE ARGUMENT IS NOT "IT LOOKS THE SAME" ───────────
-- The shipped guard is `nullif(sum(w), 0)` (`:219`), and its header (`:179-183`) explains it is not
-- defensive noise: a source whose every audit has decayed below `double precision` yields NULL and is
-- SKIPPED by the outer `avg`, still counting toward `audit_coverage` — *"it was evaluated, but the
-- verdict no longer carries numeric weight."* Under three stages the same idiom appears twice, and the
-- second one is only safe because of a fact about the first:
--
--   `auditor_value IS NULL`  <=>  `sum(w) = 0`  <=>  every `w` in that group is 0  <=>  `max(w) = 0`
--
-- (`w = pow(0.5, non-negative)` is never negative, so a zero sum of non-negatives forces every term to
-- zero.) So a fully-decayed auditor contributes a NULL to stage 2's numerator — which `sum` skips —
-- and EXACTLY 0 to its denominator. It cannot dilute a live auditor's vote, and it cannot be counted
-- as a 0.0 verdict. No `FILTER (WHERE auditor_value IS NOT NULL)` is needed on the denominator; adding
-- one would be inert, and inert code that looks load-bearing is worse than none.
--
-- A source ALL of whose auditors have decayed then yields `NULL / NULL` — numerator NULL (every term
-- skipped), denominator NULL (`nullif(0, 0)`) — so the source drops out of the outer mean exactly as
-- it does today, rather than becoming `0.0` or raising `division by zero`. Verified in the same
-- side-by-side run: a finding with one all-decayed source and one fresh `+1.0` reads `1.000000` under
-- both bodies (a stage-2 mean that admitted the decayed source would read `0.500000`).
--
-- ONE CORRECTION TO THE SHIPPED PROSE, RECORDED AND NOT FIXED. `:179-180` says the weight *"underflows
-- to exactly 0.0 for an audit roughly 88 years old"*. It does not: `pow(0.5, …)` resolves to the
-- NUMERIC overload here (`0.5` is a numeric literal), and numeric does not underflow — it reaches
-- exactly `0` only near 5000 half-lives (~410 years), and in the band between ~88 and ~410 years the
-- numeric -> `double precision` cast RAISES `out of range` instead. Measured against the shipped
-- function on the dev database: a 100-year-old audit makes `resource_citation_quality` error outright.
-- So `nullif(sum(w), 0)` is still exactly right and still reachable — just further out than the
-- comment says, with an error band before it. That band is a property of the `weighted` CTE this beat
-- carries over VERBATIM; fixing it means changing the decay expression, which is a different beat's
-- scope and a different argument. The test that exercises the NULL path ages by 500 years for this
-- reason, not by the 88 the prose would suggest.
--
-- ── EVERYTHING ELSE IS CARRIED OVER VERBATIM ──────────────────────────────────────────────────
-- Every EXPRESSION in the `weighted` CTE is carried over unchanged from the shipped one; the only
-- additions are the `a.audited_by_profile_id` column and a pointer replacing its inline comment (whose
-- content is restated below, so nothing is dropped). That is deliberate: this CTE is the half that is
-- NOT under test here, and re-typing it would put four properties at risk that the shipped header
-- argues for at length (`:154-200`):
--   * the 30-day half-life, *"the single tunable constant of this migration, deliberately spelled
--     once"*, mirroring `resource_freshness`
--     (`20260721000010_evidential_standing_memo.sql:73`) so the two time-decayed components on
--     `kb_resource_standing` fade on the same clock. Unchanged here, and out of this beat's scope.
--   * `greatest(now() - a.created, interval '0')`, the overflow clamp: a negative age makes the
--     exponent large and POSITIVE and PostgreSQL's `power()` RAISES rather than returning `inf`, so a
--     future-dated verdict would 500 a GET (`:193-200`). Note the clamp interacts with stage 1's
--     `max(w)` in the obvious way — a future-dated audit clamps to age 0, weight 1, and is therefore
--     its auditor's "freshest". That is the same full-weight treatment the clamp already chose.
--   * `a.block_id = c.block_id`, the full citation key, *"what keeps an audit of one finding's
--     citation from scoring a different finding that happens to cite the same source"* (`:209-212`).
--   * `a.source_kind = 'resource'`, unfalsifiable defence-in-depth (`:138-141`) — do not read it as
--     carrying meaning.
--
-- Also unchanged, and deliberately so:
--   * `resource_audit_coverage` (`:142-152`) counts DISTINCT SOURCES carrying >=1 audit. A per-auditor
--     collapse cannot move it: ten audits from one principal and one from another both make the source
--     covered, before and after. Coverage asks *"did anyone look?"*, not *"how many looked?"* — the
--     latter is a fourth axis nobody has specified, and inventing it here would be a wire change.
--   * `standing_band` (`:264-319`), `resource_citation_magnitude` (`:124-127`),
--     `refresh_resource_standing` (`:339-362`), `resource_standing_shape` (`:382-404`). The signature
--     `resource_citation_quality(uuid) RETURNS double precision` is unchanged, so every caller — the
--     `components` CTE at `:395`, the refresh at `:348` — needs no edit and no re-drop.
--
-- ⚠️ NON-ADDITIVE FOR THE LENGTH OF THIS TRANSACTION ONLY, the same shape and the same argument as
-- `20260724000200_citation_audit_attribution.sql:109-121`: a shipped migration is immutable, so
-- extending a shipped function is a NEW migration that DROPs and CREATEs. `DROP FUNCTION` breaks
-- migrate-ahead-of-deploy in general, but the CREATE follows in the same migration with the SAME name
-- and SAME signature, so no window exists outside the transaction. PostgreSQL does not parse
-- `LANGUAGE sql` bodies for dependencies (`20260724000120_standing_citation_components.sql:77-79`), so
-- neither `resource_standing_shape` nor `refresh_resource_standing` blocks the DROP or needs CASCADE —
-- and relying on that silently is the trap that header names, which is why it is stated here too.
--
-- THIS MIGRATION MOVES STANDING NUMBERS, unlike Beat 1's. It does so only for findings whose audit
-- trail contains more than one audit of one source from one principal — every other trail collapses
-- identically under two stages and three. Six of the seven scenarios in the side-by-side run are
-- byte-equal, including the shipped `a_source_cited_by_two_blocks_counts_once_in_quality` shape
-- (`0.500000` under both) and the shipped `recent_audit_outweighs_older_opposite` shape; only the
-- volume shape moved. No test in the suite asserts a quality number over a trail with two audits of
-- one source from one auditor, so nothing incumbent should red. There is no
-- backfill and none is possible: quality is recomputed live at read (`:374-378` — the memo column is
-- never the read's authority), so the memo self-corrects on the next `refresh_resource_standing` and
-- every read is already correct before it does.

DROP FUNCTION resource_citation_quality(uuid);

CREATE FUNCTION resource_citation_quality(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH weighted AS (
        -- Carried over verbatim from `20260724000120:203-217` plus `a.audited_by_profile_id` — the
        -- grouping key Beat 1 added for exactly this. See the header for the four properties this
        -- CTE holds that are not this beat's to touch.
        SELECT c.source_id,
               a.audited_by_profile_id,
               a.value,
               pow(0.5, extract(epoch FROM greatest(now() - a.created, interval '0'))
                        / (30.0 * 86400.0)) AS w
        FROM resource_live_citations(p_finding) c
        JOIN kb_citation_audits a
          ON a.block_id = c.block_id
         AND a.source_kind = 'resource'
         AND a.source_id = c.source_id
    ),
    -- STAGE 1 — within an auditor. One value per (source, auditor), so N audits from one principal
    -- count once. Decay-weighted, which is what makes a principal's newest audit dominate its own
    -- older ones: the retraction mechanic, with the append-only trail intact.
    --
    -- `max(w1.w)` is that auditor's FRESHEST audit's weight — `w` is monotone decreasing in age, so
    -- the largest weight is the newest verdict. It is carried out of this stage as the auditor's
    -- standing in stage 2, and it is 0 exactly when `auditor_value` is NULL (header: the equivalence
    -- that makes stage 2's divide guard sufficient).
    per_auditor AS (
        SELECT w1.source_id,
               w1.audited_by_profile_id,
               sum(w1.w * w1.value) / nullif(sum(w1.w), 0) AS auditor_value,
               max(w1.w)                                   AS auditor_weight
        FROM weighted w1
        GROUP BY w1.source_id, w1.audited_by_profile_id
    ),
    -- STAGE 2 — across auditors, weighted by each auditor's freshest-audit weight. NOT `avg(...)`:
    -- a plain mean gives one principal one vote but lets a two-year-old verdict count equally with
    -- today's, breaking the shipped invariant *"decay only arbitrates BETWEEN competing audits"*
    -- (`20260724000120:177`) — and REGRESSING a case the two-stage function already got right. Both
    -- properties are required; see the header's measured comparison.
    per_source AS (
        SELECT pa.source_id,
               sum(pa.auditor_weight * pa.auditor_value)
                 / nullif(sum(pa.auditor_weight), 0) AS source_value
        FROM per_auditor pa
        GROUP BY pa.source_id
    )
    -- STAGE 3 — plain mean across distinct AUDITED sources; unchanged. `coalesce(..., 0.0)` is the
    -- neutral reading of an unaudited finding: it makes no quality claim, and its low standing comes
    -- from the band gate, not from a poisoned mean (`20260724000120:164-167`).
    SELECT coalesce(avg(ps.source_value), 0.0)::double precision FROM per_source ps;
$$;

COMMENT ON FUNCTION resource_citation_quality(uuid) IS
  'THREE-stage decay-weighted audit mean (spec §3.1, per-auditor collapse). (1) Collapse within an '
  'AUDITOR per source — decay-weighted, so one principal''s N audits count once and its newest audit '
  'dominates its own older ones (the retraction mechanic, with no supersession). (2) Collapse across '
  'auditors per source, WEIGHTED BY EACH AUDITOR''S FRESHEST-AUDIT WEIGHT, not a plain mean: one vote '
  'per principal AND decay still arbitrating between competing auditors — a plain mean would let a '
  'two-year-old verdict count equally with today''s. (3) Plain mean across distinct AUDITED sources. '
  'The stage order is what keeps a multi-block source, and now a persistent auditor, from voting more '
  'than once. A source every one of whose auditors has decayed to zero weight still yields NULL and '
  'drops out of the outer mean rather than reading 0.0 — the divide guard is nullif(sum(...), 0) at '
  'BOTH collapse stages, and stage 2''s is sufficient because a NULL auditor_value implies that '
  'auditor''s max(w) is 0, so it adds nothing to the denominator either.';
