-- Evidential-standing maturity projection — Set 3 (spec 019f81e8; plan
-- docs/superpowers/plans/2026-07-21-set3-maturity-projection.md). Phase A: the SQL substrate.
--
-- BEDROCK (spec preamble): standing is NOT truth. Every function here measures how *defensible on
-- the present evidence* a claim is — a fact about the structure of emitted evidence and its
-- relations, never about the world. A lone-right claim reads low-standing-with-no-contradiction and
-- that is correct, because low-standing was never a claim about truth.
--
-- STANDING IS THE VECTOR (spec §1.1): (independence-discounted-breadth, adversarial-survival,
-- contradiction-balance, freshness). Any band label is a LOSSY read-time chip over that shape,
-- computed at read (standing_band), never a stored column. §1.3 AMEND: there is deliberately NO
-- maturity/band enum on kb_resources — a stored band reintroduces the cache-vs-truth bug the
-- events-as-primary bedrock refuses.
--
-- MECHANISM (grounding correction 2026-07-21): this repo memoizes projections as PLAIN COLUMNS
-- refreshed by an application "clock" (region_clocks.rs) gated by a SQL drift fn — it has ZERO
-- recompute-on-write DB triggers and ZERO materialized views. So the memo tables below carry no
-- trigger; the Rust standing-clock (Phase B) calls refresh_resource_standing on the write path, and
-- the read (resource_standing_shape) recomputes components live. These are PROJECTION tables:
-- recomputable, NOT append-only-guarded (only event/standing LOGS get that guard).
--
-- Subject of standing = ANY kb_resource (finding_id = kb_resources.id) — Set 3 needs no findings
-- board (Set 2). Scar EMISSION (is_corrected / independence-edge scarring) is Set 5's; Set 3 only
-- READS the live is_corrected / is_folded filters, exactly as the incumbent reinforce_count does.

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- Task 1 — component-memo table + R_parent (breadth) and freshness producers
-- ────────────────────────────────────────────────────────────────────────────────────────────────

CREATE TABLE kb_resource_standing (
    finding_id            UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    indep_breadth         DOUBLE PRECISION NOT NULL DEFAULT 0,  -- independence-discounted breadth (§2.1)
    adversarial_survival  DOUBLE PRECISION NOT NULL DEFAULT 0,  -- N withstood; 0 = no challenges yet (§1)
    challenge_count       INT              NOT NULL DEFAULT 0,  -- distinguishes 0-challenges from N-withstood
    contradiction_balance DOUBLE PRECISION NOT NULL DEFAULT 0,  -- supports − contradicts, vector-sum (§1)
    freshness             DOUBLE PRECISION NOT NULL DEFAULT 0,  -- reversible decay off R_parent recency
    r_parent              DOUBLE PRECISION NOT NULL DEFAULT 0,  -- breadth term (reinforce_count)
    refreshed_event_id    UUID REFERENCES kb_events(id),        -- watermark; stamped by the Phase-B clock
    updated               TIMESTAMPTZ      NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_resource_standing IS
  'Memoized component scalars of a resource''s evidential standing (spec 019f81e8 §1.3 AMEND). '
  'The band/shape is computed AT READ from these columns (standing_band / resource_standing_shape); '
  'NEVER store a band. A projection table — recomputable, refreshed by the Phase-B standing clock, '
  'not append-only-guarded.';

-- R_parent: reinforcement breadth of the finding. CONFORM to the incumbent tally
-- cogmap_region_reference_standing (canonical_functions.sql:474-483) — count of uncorrected
-- provenance over the finding's live blocks. is_corrected excluded (that filter is the scar read).
CREATE FUNCTION resource_r_parent(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT coalesce(count(p.*), 0)::double precision
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    WHERE b.resource_id = p_finding AND NOT b.is_folded;
$$;

-- Freshness: reversible fade clocked off R_parent recency (spec §1 decay). 1.0 at "just
-- reinforced", decaying by a 30-day half-life toward 0. The half-life is a tunable default (the
-- Task-4 surfacing pass). now() is stable-within-txn, so the memo stores a snapshot and the read
-- recomputes — freshness is the one component that must be live at read.
CREATE FUNCTION resource_freshness(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH last AS (
        SELECT max(p.created) AS at
        FROM kb_content_blocks b
        JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
        WHERE b.resource_id = p_finding AND NOT b.is_folded
    )
    SELECT CASE
        WHEN (SELECT at FROM last) IS NULL THEN 0.0
        ELSE pow(0.5, extract(epoch FROM (now() - (SELECT at FROM last))) / (30.0 * 86400.0))
    END::double precision;
$$;

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- Task 2 — independence_pairs memo + refresh + independence-discounted breadth
-- ────────────────────────────────────────────────────────────────────────────────────────────────

-- Flattened pairwise-independence memo (spec §2.3). One row per affirmatively-asserted
-- independent-of pair among a finding's evidentiary bases. SILENCE DEFAULT (§2.4): a pair with NO
-- row is NOT independent (assumed correlated) — breadth rises only on affirmation. is_scarred = the
-- underlying independent-of edge was folded/superseded (§2.1); Set 3 READS it, the scar WRITER is
-- Set 5.
CREATE TABLE kb_independence_pairs (
    finding_id  UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    base_a      UUID NOT NULL,                          -- resource-base; canonical order base_a < base_b
    base_b      UUID NOT NULL,
    weight      DOUBLE PRECISION NOT NULL DEFAULT 1.0,  -- independence estimate × R_indep
    is_scarred  BOOLEAN NOT NULL DEFAULT false,
    edge_id     UUID NOT NULL REFERENCES kb_edges(id),
    PRIMARY KEY (finding_id, base_a, base_b)
);
CREATE INDEX idx_kb_independence_pairs_finding ON kb_independence_pairs(finding_id) WHERE NOT is_scarred;

COMMENT ON TABLE kb_independence_pairs IS
  'Flattened per-finding pairwise independence over evidentiary bases (spec 019f81e8 §2.3). '
  'Sparse-by-assertion: independence is not transitive, so only affirmatively-judged pairs get a '
  'row (§2.4 silence=correlated). Rebuilt by refresh_independence_pairs from live independent-of '
  'edges. is_scarred mirrors the edge being folded.';

-- Evidentiary bases of a finding = distinct resource-source provenance over its live blocks.
CREATE FUNCTION resource_bases(p_finding uuid)
RETURNS TABLE(source_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT p.source_id
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    WHERE b.resource_id = p_finding AND NOT b.is_folded
      AND p.source_kind = 'resource';
$$;

-- Rebuild the finding's independence memo from live independent-of edges among its bases. Model:
-- region member recompute (write.rs refresh_salience). is_scarred := the edge is folded (superseded).
-- No recursion — a terminating flatten (§2.1). Endpoints are resources (kb_edges CHECK).
CREATE FUNCTION refresh_independence_pairs(p_finding uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM kb_independence_pairs WHERE finding_id = p_finding;
    INSERT INTO kb_independence_pairs (finding_id, base_a, base_b, weight, is_scarred, edge_id)
    SELECT p_finding,
           least(e.source_id, e.target_id),
           greatest(e.source_id, e.target_id),
           e.weight, e.is_folded, e.id
    FROM kb_edges e
    WHERE e.edge_kind = 'express'
      AND e.label = 'independent-of'
      AND e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
      AND e.source_id IN (SELECT source_id FROM resource_bases(p_finding))
      AND e.target_id IN (SELECT source_id FROM resource_bases(p_finding))
    ON CONFLICT (finding_id, base_a, base_b)
        DO UPDATE SET weight = EXCLUDED.weight, is_scarred = EXCLUDED.is_scarred, edge_id = EXCLUDED.edge_id;
END;
$$;

-- Independence-discounted breadth (spec §2.1 terminating leaf-tally, no recursion; §2.4 silence =
-- correlated). Effective independent rank = 1 (the base correlated cluster, if any bases exist)
-- + Σ non-scarred affirmed-independence pair weights. Monotone in affirmed, non-scarred
-- independence; a monoculture of N unasserted bases stays at 1.0. The exact aggregation is the
-- Task-4 tuning target.
CREATE FUNCTION resource_independence_breadth(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT (CASE WHEN EXISTS (SELECT 1 FROM resource_bases(p_finding)) THEN 1.0 ELSE 0.0 END)
         + coalesce((SELECT sum(weight) FROM kb_independence_pairs
                     WHERE finding_id = p_finding AND NOT is_scarred), 0.0);
$$;

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- Task 3 — contradiction-balance + adversarial-survival readers
-- ────────────────────────────────────────────────────────────────────────────────────────────────

-- Contradiction balance (§1 vector-sum): Σ weight(support-labelled) − Σ weight(contradicts-labelled)
-- over express edges incident to the finding. CONFORM to cogmap_region_internal_tension's
-- label-match idiom (canonical_functions.sql:505-521): opposition is a free-text label, not a
-- kernel-reserved polarity. 5 supports + 4 contradicts nets +1 — a live concern, not near-canonical.
CREATE FUNCTION resource_contradiction_balance(p_finding uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT coalesce(sum(CASE WHEN e.label = 'contradicts' THEN -e.weight ELSE e.weight END), 0)::double precision
    FROM kb_edges e
    WHERE e.edge_kind = 'express' AND NOT e.is_folded
      AND e.label = ANY (ARRAY['supports', 'corroborates', 'contradicts'])  -- support/oppose set; Set 5/6 may extend
      AND ((e.source_table = 'kb_resources' AND e.source_id = p_finding)
        OR (e.target_table = 'kb_resources' AND e.target_id = p_finding));
$$;

-- Adversarial survival (§1): N challenges withstood, kept DISTINCT from 0 challenges (absence of
-- challenge is not survival). The adversary's challenge/survived label vocabulary is SET 5's to
-- finalize — these placeholder labels return 0 for every finding until Set 5 emits.
CREATE FUNCTION resource_adversarial_survival(p_finding uuid)
RETURNS TABLE(challenge_count int, survived double precision) LANGUAGE sql STABLE AS $$
    SELECT count(*) FILTER (WHERE e.label = 'challenged')::int,
           coalesce(sum(e.weight) FILTER (WHERE e.label = 'survived-challenge'), 0)::double precision
    FROM kb_edges e
    WHERE e.edge_kind = 'express' AND NOT e.is_folded
      AND ((e.source_table = 'kb_resources' AND e.source_id = p_finding)
        OR (e.target_table = 'kb_resources' AND e.target_id = p_finding));
$$;

-- ────────────────────────────────────────────────────────────────────────────────────────────────
-- Task 4 — refresh, read-time band, and the access-gated shape read
-- ────────────────────────────────────────────────────────────────────────────────────────────────

-- Read-time band chip (spec §1.1: a LOSSY summary over the shape, presented WITH it, never stored).
-- Thresholds are the tunable defaults Set 3 owns (the "exact thresholds" surfacing pass). IMMUTABLE:
-- depends only on its arguments.
CREATE FUNCTION standing_band(
    p_indep_breadth         double precision,
    p_challenge_count       int,
    p_survived              double precision,
    p_contradiction_balance double precision,
    p_freshness             double precision)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        -- near-canonical: ≥3 effective-independent breadth, ≥1 survived challenge, balance clearly positive
        WHEN p_indep_breadth >= 3.0 AND p_survived >= 1.0 AND p_contradiction_balance > 1.0 THEN 'near-canonical'
        -- reinforced: ≥2 effective-independent breadth and not under live contradiction
        WHEN p_indep_breadth >= 2.0 AND p_contradiction_balance >= 0.0 THEN 'reinforced'
        ELSE 'provisional'
    END;
$$;

-- Cheap refresh: recompute components, UPSERT the memo. CONFORM to refresh_salience (write.rs:851).
-- The Phase-B standing clock calls this on the write path; refreshed_event_id stamping is added
-- there (needs the clock's refresh event). Reading stays live (resource_standing_shape) — the memo
-- is a read-cost optimization, not the read's source of truth.
CREATE FUNCTION refresh_resource_standing(p_finding uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_ch int; v_surv double precision;
BEGIN
    PERFORM refresh_independence_pairs(p_finding);
    SELECT challenge_count, survived INTO v_ch, v_surv FROM resource_adversarial_survival(p_finding);
    INSERT INTO kb_resource_standing
        (finding_id, indep_breadth, adversarial_survival, challenge_count,
         contradiction_balance, freshness, r_parent, updated)
    VALUES (p_finding, resource_independence_breadth(p_finding), v_surv, v_ch,
            resource_contradiction_balance(p_finding), resource_freshness(p_finding),
            resource_r_parent(p_finding), now())
    ON CONFLICT (finding_id) DO UPDATE SET
        indep_breadth         = EXCLUDED.indep_breadth,
        adversarial_survival  = EXCLUDED.adversarial_survival,
        challenge_count       = EXCLUDED.challenge_count,
        contradiction_balance = EXCLUDED.contradiction_balance,
        freshness             = EXCLUDED.freshness,
        r_parent              = EXCLUDED.r_parent,
        updated               = now();
END;
$$;

-- Access-gated read: memoized-shape-recomputed-live + read-time band. CONFORM to resource_blocks'
-- gate (resources_readable_by) and wayfind's memoized-vs-recompute. Recomputes components live
-- rather than reading kb_resource_standing, because freshness is time-decayed and must be current
-- at read; the memo/refresh exists for the write-path clock + parity, not as the read's authority.
-- The gate is the FULL canonical visibility predicate (resources_readable_by), not a subset.
CREATE FUNCTION resource_standing_shape(p_finding uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(finding_id uuid, indep_breadth double precision, adversarial_survival double precision,
              challenge_count int, contradiction_balance double precision, freshness double precision,
              r_parent double precision, band text)
LANGUAGE sql STABLE AS $$
    WITH gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ),
    adv AS (
        SELECT g.fid, a.challenge_count, a.survived
        FROM gated g, LATERAL resource_adversarial_survival(g.fid) a
    )
    SELECT g.fid,
           resource_independence_breadth(g.fid),
           adv.survived,
           adv.challenge_count,
           resource_contradiction_balance(g.fid),
           resource_freshness(g.fid),
           resource_r_parent(g.fid),
           standing_band(resource_independence_breadth(g.fid), adv.challenge_count, adv.survived,
                         resource_contradiction_balance(g.fid), resource_freshness(g.fid))
    FROM gated g
    JOIN adv ON adv.fid = g.fid;
$$;
