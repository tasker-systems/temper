-- Standing splits into three citation-grain axes (Set 5, Task 3; spec
-- `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §3.1-3.4, §4.1).
--
-- ⚠️ THIS MIGRATION IS NON-ADDITIVE. It DROPS three columns of `kb_resource_standing`
-- (`indep_breadth`, `adversarial_survival`, `challenge_count`), DROPS four functions
-- (`resource_independence_breadth`, `refresh_independence_pairs`, `resource_adversarial_survival`,
-- `resource_bases`), DROPS the table `kb_independence_pairs`, and CHANGES the return type of
-- `resource_standing_shape` and the argument list of `standing_band`. That breaks the
-- additive-only-on-`main` invariant that makes `main` auto-deploy safe (CLAUDE.md, DEPLOYING.md):
-- a binary that is one deploy behind and still SELECTs `indep_breadth` from
-- `resource_standing_shape` will error after this applies.
--
-- IT IS NONETHELESS CORRECT, for two reasons that must both hold:
--   1. The retired model is provably inert. `resource_independence_breadth` is a CONSTANT today —
--      it reads `kb_independence_pairs`, which is (re)built only from `'independent-of'` edges, and
--      NOTHING in this repo writes that label outside a Set-3 test (spec §1, §3.4: "because the
--      pairs table is provably empty, this AMEND carries no data migration and no behavior
--      change"). Same for `resource_adversarial_survival`: the `'challenged'` /
--      `'survived-challenge'` labels have no writer, so it returns `(0, 0.0)` for every finding.
--      Dropping them removes zero information; there is nothing to backfill.
--   2. The binary that stops calling the dropped objects (Task 7 — the readback/service/wire
--      shapes) ships in the SAME deploy as this migration. This is a big-bang cutover, operator-run
--      per target, NOT an auto-deploy of `main`. Task 14 adds the cutover entry to DEPLOYING.md;
--      follow it rather than letting a target pick this up on its own cadence.
--
-- WHAT REPLACES WHAT (spec §3.1's component-mapping table, so no column's fate is inferred):
--   indep_breadth        → citation_magnitude + audit_coverage + citation_quality (three axes; the
--                          one scalar could not express "many sources" vs "many *evaluated*
--                          sources" vs "evaluated *well*").
--   adversarial_survival → subsumed into citation_quality: the signed gradient IS the survival
--                          signal (§3.3). A citation that withstood audit carries a positive value.
--   challenge_count      → audit_coverage, the clearer name for the same job (distinguishing
--                          "nobody tried" from "N sources evaluated"). Dropped rather than left
--                          carrying a name that lies about its meaning.
--   contradiction_balance, freshness, r_parent → UNCHANGED. `r_parent` is deliberately NOT the same
--                          thing as `citation_magnitude`: r_parent counts ALL uncorrected
--                          provenance rows (total accretion, duplicates included), magnitude counts
--                          DISTINCT LIVE sources. Ten citations of one source is
--                          `r_parent = 10, citation_magnitude = 1`, and that gap is the echo case.
--
-- LIVENESS IS NOT OPTIONAL (spec §3.1). Soft-delete only flips `kb_resources.is_active` — it does
-- NOT fold blocks or provenance (`_project_resource_deleted`,
-- `20260624000002_canonical_functions.sql:1051-1061` is the whole projector). So a component that
-- counts provenance without a liveness join keeps a deleted source conferring standing forever.
-- Every producer below joins `kb_resources` and requires the source `is_active` — the CLAUDE.md
-- rule "`ingest_state = 'complete'` goes exactly where `r.is_active` already goes", applied to the
-- new axis. Set 3's `resource_bases` (`20260721000010_evidential_standing_memo.sql:104-111`) did
-- not carry it, which is why it is retired here rather than reused: leaving a bases producer with
-- the wrong join shape on disk invites the next producer to copy it.

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 1. New component columns (added before anything reads them)
-- ────────────────────────────────────────────────────────────────────────────────────────────────

ALTER TABLE kb_resource_standing
    ADD COLUMN citation_magnitude INT              NOT NULL DEFAULT 0,
    ADD COLUMN audit_coverage     INT              NOT NULL DEFAULT 0,
    ADD COLUMN citation_quality   DOUBLE PRECISION NOT NULL DEFAULT 0;

COMMENT ON COLUMN kb_resource_standing.citation_magnitude IS
  'Count of DISTINCT LIVE cited sources (spec §3.1 findability axis). Monotone: citing more '
  'evidence never lowers it. NOT r_parent — that counts every provenance row, duplicates included.';
COMMENT ON COLUMN kb_resource_standing.audit_coverage IS
  'Count of those distinct sources carrying >=1 citation audit (spec §3.1 evaluated-ness axis). '
  'Monotone under the append-only trail: once a source is audited it stays covered.';
COMMENT ON COLUMN kb_resource_standing.citation_quality IS
  'Mean over the AUDITED SUBSET ONLY of each audited source''s decay-weighted audit value, in '
  '[-1,1] (spec §3.1). 0.0 when audit_coverage = 0 — an unaudited finding makes no quality claim, '
  'and its low standing comes from the band gate, not from a poisoned mean.';

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 2. Retire the pairwise-independence model and its edge-based challenge reader (spec §3.4)
-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- Dropped consumers-first so the order is correct whether or not PostgreSQL tracks the dependency
-- (it does not parse `LANGUAGE sql` bodies for dependencies, but relying on that is a trap for the
-- next reader).

DROP FUNCTION resource_standing_shape(uuid, text, uuid);
DROP FUNCTION standing_band(double precision, int, double precision, double precision, double precision);
DROP FUNCTION resource_independence_breadth(uuid);
DROP FUNCTION refresh_independence_pairs(uuid);
DROP FUNCTION resource_adversarial_survival(uuid);
DROP FUNCTION resource_bases(uuid);
DROP TABLE kb_independence_pairs;

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 3. The shared citation producer — ONE definition of "a live citation of this finding"
-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- All three axes are cuts of the same set, so the set is produced once. Emitting (block_id,
-- source_id) rather than just source_id is what lets `resource_citation_quality` join the audit
-- trail on the full citation key `(block_id, source_kind, source_id)` — a citation is a
-- (block, source) pair (spec §4.1), and `kb_citation_audits` is keyed that way
-- (`20260723000010_citation_audits.sql:23-39`).
--
-- Join shape CONFORMs to the retired `resource_bases`
-- (`20260721000010_evidential_standing_memo.sql:104-111`: `NOT p.is_corrected`, `NOT b.is_folded`,
-- `source_kind = 'resource'`) plus the liveness join spec §3.1 makes mandatory. Only resource-kind
-- citations participate — that is also what the audit write path enforces
-- (`20260723000010_citation_audits.sql:123-126`), so the two ends agree.
CREATE FUNCTION resource_live_citations(p_finding uuid)
RETURNS TABLE(block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT p.block_id, p.source_id
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    JOIN kb_resources src ON src.id = p.source_id AND src.is_active
    WHERE b.resource_id = p_finding AND NOT b.is_folded
      AND p.source_kind = 'resource';
$$;

COMMENT ON FUNCTION resource_live_citations(uuid) IS
  'The finding''s live citations as (block, source) pairs — the one definition the three standing '
  'axes share (spec §3.1). Distinct LIVE sources: soft-deleted sources are excluded, because '
  'soft-delete does not fold blocks or provenance.';

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 4. The three axes
-- ────────────────────────────────────────────────────────────────────────────────────────────────

-- Findability. DISTINCT sources, not provenance rows: ten citations of one source is magnitude 1
-- (spec §3.1 — collapsing this into r_parent reintroduces the actor-count fallacy).
CREATE FUNCTION resource_citation_magnitude(p_finding uuid)
RETURNS int LANGUAGE sql STABLE AS $$
    SELECT count(DISTINCT c.source_id)::int FROM resource_live_citations(p_finding) c;
$$;

-- Evaluated-ness. A source counts once however many of the finding's blocks cite it and however
-- many audits it carries — `count(DISTINCT source_id)` over sources with at least one audit.
CREATE FUNCTION resource_audit_coverage(p_finding uuid)
RETURNS int LANGUAGE sql STABLE AS $$
    SELECT count(DISTINCT c.source_id)::int
    FROM resource_live_citations(p_finding) c
    WHERE EXISTS (
        SELECT 1 FROM kb_citation_audits a
        WHERE a.block_id = c.block_id
          AND a.source_kind = 'resource'
          AND a.source_id = c.source_id
    );
$$;

-- Quality — the decay-weighted mean over the AUDITED SUBSET ONLY, in two stages.
--
-- THE ORDER OF THE TWO STAGES IS LOAD-BEARING (spec §3.1). Stage 1 (`per_source`) collapses WITHIN
-- a source: the decay-weighted aggregate of every audit that source carries, across every block of
-- this finding that cites it — one value per source. Stage 2 (the outer `avg`) is the plain mean
-- ACROSS distinct audited sources. A naive single-stage weighted mean over the flat
-- provenance-joined-to-audits row set gives a source cited by five blocks five votes in the outer
-- mean — the echo/actor-count fallacy re-entering through block multiplicity, in the very function
-- written to exclude it.
--
-- 0.0 when nothing is audited: `avg` over the empty audited subset is NULL, coalesced to a neutral
-- 0.0. An unaudited finding makes no quality claim; its low standing comes from the band gate
-- (spec §3.1's "not from a poisoned mean", and §3.2's rejection of the -0.5-per-unaudited draft
-- that produced a perverse gradient).
--
-- DECAY — the single tunable constant of this migration, deliberately spelled once. 30-day
-- half-life, mirroring `resource_freshness`
-- (`20260721000010_evidential_standing_memo.sql:73`: `pow(0.5, epoch_age / (30.0 * 86400.0))`), so
-- the two time-decayed components on this table fade on the same clock. Spec §4.1 authorizes the
-- value as a default this set owns ("the exact decay half-life and aggregation shape are tunable
-- defaults this set owns"). Newer verdicts weigh more; older ones fade in influence but NEVER
-- vanish from the trail — the ledger is append-only and nothing here mutates it (spec §4.1). A
-- SINGLE audit's weight cancels in its own per-source mean, so a lone old +0.8 still reads +0.8
-- indefinitely; decay only arbitrates BETWEEN competing audits (spec §3.1).
--
-- `nullif(sum(w), 0)` is the divide guard, not defensive noise: `pow(0.5, ...)` underflows to
-- exactly 0.0 for an audit roughly 88 years old, which would otherwise raise `division by zero` in
-- a READ path. A source whose every audit has decayed below double precision yields NULL and is
-- skipped by the outer `avg` — it still counts toward `audit_coverage`, which is the honest
-- reading: it was evaluated, but the verdict no longer carries numeric weight.
CREATE FUNCTION resource_citation_quality(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH weighted AS (
        SELECT c.source_id,
               a.value,
               pow(0.5, extract(epoch FROM (now() - a.created)) / (30.0 * 86400.0)) AS w
        FROM resource_live_citations(p_finding) c
        JOIN kb_citation_audits a
          ON a.block_id = c.block_id
         AND a.source_kind = 'resource'
         AND a.source_id = c.source_id
    ),
    per_source AS (
        SELECT w1.source_id, sum(w1.w * w1.value) / nullif(sum(w1.w), 0) AS source_value
        FROM weighted w1
        GROUP BY w1.source_id
    )
    SELECT coalesce(avg(ps.source_value), 0.0)::double precision FROM per_source ps;
$$;

COMMENT ON FUNCTION resource_citation_quality(uuid) IS
  'Two-stage decay-weighted audit mean (spec §3.1): collapse within a source first (one value per '
  'distinct source, across all its citing blocks), then mean across distinct AUDITED sources. The '
  'stage order is what keeps a multi-block source from voting more than once.';

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 5. The read-time band — four arms over the new axes
-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- Still a LOSSY read-time chip over the shape, never a stored column (spec §1.1/§1.3 of 019f81e8,
-- CONFORMed by §3.1). The fourth arm — `disputed` — exists so "never evaluated" and "evaluated and
-- found wanting" are not flattened together, which is spec §1's requirement on its negative side.
--
-- CALIBRATED FOR ONE-PASS REACHABILITY (spec §3.1). Two structural choices, and they are the
-- load-bearing part of this function:
--   * The magnitude floor for the top band is 2, NOT 3 — the honest Landmesser line, "more than one
--     independent source". A lone source can never be near-canonical however well audited (that is
--     the whole point), but demanding three makes the top band structurally unreachable for the
--     large fraction of an atomic KB that rests on two vetted sources.
--   * Full coverage (`audit_coverage = citation_magnitude`) is reachable in ONE thorough pass,
--     because coverage is per-distinct-source: one auditor visit that audits every cited source
--     reaches ratio 1.0 at once. The top band does not require repeated rounds.
--
-- `p_freshness` is accepted and DELIBERATELY NOT GATED ON (spec §3.1: "staleness is carried by the
-- separate freshness component, which the band deliberately does not gate on, precisely so
-- infrequent review does not demote earned standing"). It stays in the signature because the band
-- is presented WITH the shape and a future re-tune is a one-migration change; removing the
-- parameter now would make that re-tune a signature break for no gain.
--
-- THE `> 0.3` CUT IS A PROVISIONAL LOW DEFAULT, NOT A FIXED THRESHOLD (spec §3.1). An absolute
-- quality number is meaningful only relative to how the auditor uses the [-1,1] range, and that
-- distribution does not exist until the auditor prompt does. It is co-calibrated with the persona
-- work and re-tuning it later is a one-migration change with no backfill. Do not build anything on
-- this number; build on the floor of 2 and the full-coverage requirement.
--
-- `nullif(p_citation_magnitude, 0)` guards the ratio: SQL does not guarantee short-circuit
-- evaluation of `AND`, so the `>= 1` conjunct is NOT enough to keep the division from running. A
-- magnitude-0 finding yields NULL >= 0.5 (not true) and falls through to `provisional`, which is
-- exactly where a citation-less finding belongs.
CREATE FUNCTION standing_band(
    p_citation_magnitude    int,
    p_audit_coverage        int,
    p_citation_quality      double precision,
    p_contradiction_balance double precision,
    p_freshness             double precision)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        -- near-canonical: >=2 distinct live sources, EVERY one of them audited, net-positive
        -- quality, and not under live contradiction.
        WHEN p_citation_magnitude >= 2
         AND p_audit_coverage = p_citation_magnitude
         AND p_citation_quality > 0.3
         AND p_contradiction_balance >= 0.0
            THEN 'near-canonical'
        -- reinforced: at least half the cited sources evaluated, positively, not contradicted.
        WHEN p_citation_magnitude >= 1
         AND p_audit_coverage::double precision / nullif(p_citation_magnitude, 0) >= 0.5
         AND p_citation_quality > 0.0
         AND p_contradiction_balance >= 0.0
            THEN 'reinforced'
        -- disputed: the adversary examined it and it did not hold up. Distinct from the floor.
        --
        -- `<= 0.0`, NOT `< 0.0`, and the boundary is the point. Spec §1 asks that "nobody tried"
        -- never be flattened together with "we looked and it did not hold up" — and a fully-audited
        -- finding whose verdicts CANCEL is the latter. At exactly `0.0` the strict form fell through
        -- every arm to `provisional`, byte-identical to a finding nobody has ever opened, and
        -- `audit_drift_sweep` selects on `coverage < magnitude`
        -- (`20260723000030_audit_drift_sweep.sql:109`), so a covered finding at quality 0.0 could
        -- never be re-queued to escape it. That state is ordinary, not a corner: two sources audited
        -- `+1.0` and `-1.0` land there, and `0.0` is an explicit anchor on the auditor's own published
        -- scale ("on-topic but does not bear on this connection either way"), which the persona is
        -- told to PREFER when a citation is undecidable.
        --
        -- `p_audit_coverage > 0` is what carries the whole distinction now: it is the "somebody
        -- looked" conjunct, and it is no longer redundant the way it was under `< 0.0` (where a
        -- negative quality already implied a joined audit).
        WHEN p_audit_coverage > 0 AND p_citation_quality <= 0.0
            THEN 'disputed'
        -- provisional: the floor, including EVERY unaudited finding (coverage 0 lands here no
        -- matter how large the citation set).
        ELSE 'provisional'
    END;
$$;

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 6. Refresh — same signature, new components
-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- CREATE OR REPLACE: `refresh_resource_standing(uuid) RETURNS void` is unchanged, so the Rust
-- wrapper (`temper_substrate::write::refresh_resource_standing`) and the write-path clock need no
-- edit. The `refresh_independence_pairs` and `resource_adversarial_survival` calls are gone with
-- the model they served (spec §3.4); the `TODO(Set 5)` at `db_backend.rs:1170` about re-driving
-- breadth on independence-edge writes DISSOLVES rather than being fixed — once breadth reads
-- citations and audits, no edge write can make magnitude, coverage, or quality stale.
--
-- The narrower edge-staleness that REMAINS (spec §3.4, and it must not be claimed away):
-- `contradiction_balance` still reads `kb_edges`
-- (`20260721000010_evidential_standing_memo.sql:159-163`) while the standing clock fires only on
-- resource writes, so the `kb_resource_standing.contradiction_balance` COLUMN is stale after an
-- edge write until the next resource write over the finding. Harmless ONLY because
-- `resource_standing_shape` recomputes every component live at read and never trusts the memo
-- column. Any future consumer that reads `kb_resource_standing` directly must first wire an
-- edge-incident refresh for `contradiction_balance`.
CREATE OR REPLACE FUNCTION refresh_resource_standing(p_finding uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO kb_resource_standing
        (finding_id, citation_magnitude, audit_coverage, citation_quality,
         contradiction_balance, freshness, r_parent, updated)
    VALUES (p_finding,
            resource_citation_magnitude(p_finding),
            resource_audit_coverage(p_finding),
            resource_citation_quality(p_finding),
            resource_contradiction_balance(p_finding),
            resource_freshness(p_finding),
            resource_r_parent(p_finding),
            now())
    ON CONFLICT (finding_id) DO UPDATE SET
        citation_magnitude    = EXCLUDED.citation_magnitude,
        audit_coverage        = EXCLUDED.audit_coverage,
        citation_quality      = EXCLUDED.citation_quality,
        contradiction_balance = EXCLUDED.contradiction_balance,
        freshness             = EXCLUDED.freshness,
        r_parent              = EXCLUDED.r_parent,
        updated               = now();
END;
$$;

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 7. The access-gated shape read — new return type, same gate
-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- DROP+CREATE (done above), not CREATE OR REPLACE: the return type changed.
--
-- THE `gated` CTE IS CARRIED OVER BYTE-FOR-BYTE from
-- `20260721000010_evidential_standing_memo.sql:239-242`. It is this read's leak-safety: the gate is
-- the FULL canonical visibility predicate (`resources_readable_by`), not a subset, and it lives
-- INSIDE the SQL so a principal who cannot read the finding gets ZERO rows rather than an error.
--
-- Components are recomputed LIVE, never read from the `kb_resource_standing` memo — `freshness`
-- and `citation_quality` are both time-decayed and must reflect the current moment, and
-- `contradiction_balance`'s memo column is edge-stale (see §6 above). The memo exists for the
-- write-path clock and for parity, not as this read's authority.
--
-- The `components` CTE evaluates each producer ONCE. The shipped version called
-- `resource_independence_breadth` twice (once for the column, once inside `standing_band`); with
-- five producers that pattern would double every one of them.
CREATE FUNCTION resource_standing_shape(p_finding uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(finding_id uuid, citation_magnitude int, audit_coverage int,
              citation_quality double precision, contradiction_balance double precision,
              freshness double precision, r_parent double precision, band text)
LANGUAGE sql STABLE AS $$
    WITH gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ),
    components AS (
        SELECT g.fid,
               resource_citation_magnitude(g.fid)    AS magnitude,
               resource_audit_coverage(g.fid)        AS coverage,
               resource_citation_quality(g.fid)      AS quality,
               resource_contradiction_balance(g.fid) AS balance,
               resource_freshness(g.fid)             AS fresh,
               resource_r_parent(g.fid)              AS parent
        FROM gated g
    )
    SELECT c.fid, c.magnitude, c.coverage, c.quality, c.balance, c.fresh, c.parent,
           standing_band(c.magnitude, c.coverage, c.quality, c.balance, c.fresh)
    FROM components c;
$$;

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- 8. Drop the superseded columns (last — after every reader above is on the new set)
-- ────────────────────────────────────────────────────────────────────────────────────────────────

ALTER TABLE kb_resource_standing
    DROP COLUMN indep_breadth,
    DROP COLUMN adversarial_survival,
    DROP COLUMN challenge_count;
