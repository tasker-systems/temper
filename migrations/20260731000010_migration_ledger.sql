-- A migration says what it is, and what happened to it, in a place a binary that does not carry it
-- can still read.
--
-- Task 019fb4e5-2365-7880-8cf1-7fea6755d60e · goal 019fb35b-c64e-7cd2-a7c0-aa117d1ab1a7.
-- Design: docs/superpowers/specs/2026-07-30-schema-binary-pairing-design.md §1.

-- ── Why this exists ─────────────────────────────────────────────────────────────────────────────
-- On 2026-07-30, migration `20260730000010` changed `facet_set` and `property_set` from
-- `RETURNS uuid` to `RETURNS uuid[]`. `CREATE OR REPLACE FUNCTION` drops nothing and updates every
-- in-repo caller in the same commit, so it read as additive — to a human and to an agent, both
-- answering from memory. It was not: the running binary decodes the result BY TYPE, which makes it
-- a caller nobody updated. Every write through those wrappers failed for ~40 minutes while reads
-- stayed perfectly healthy, so the first notification was a user hitting a write.

-- ── Why an append-only LEDGER and not a classification row ──────────────────────────────────────
-- `[decided — 2026-07-31, Pete]` The first cut of this migration was a single row per version, and
-- it was wrong in two ways that only surfaced once the failure states were written down:
--
--   1. **A wrong classification could not be corrected.** A shipped migration cannot be edited
--      (sqlx checksum-verifies at `sqlx-core/src/migrate/migrator.rs:175`), and a single-row table
--      keyed on version rejected any second declaration. The only path that worked was a raw
--      UPDATE from a later migration, which DESTROYED the original claim and recorded nothing about
--      the correction — no who, no why, no trace that it happened.
--   2. **There was nowhere to put what happened to the migration**, only what it claimed.
--
-- Both are the same mistake: treating a thing that evolves as a thing that is stated once. The rest
-- of the schema already knows better — `kb_events` is append-only, and `kb_citation_audits`
-- deliberately has no supersession because multiple and even opposing claims over time are the
-- point. The visible answer is a VIEW recomputed from the trail, never a column mutated in place.
--
-- So an entry may move the STATE, or revise the CLASS, or both. Two axes, independently latest-wins,
-- which is what makes "enrich the classification without changing the state" expressible at all.

-- ── The constraint that decides where state is written, measured not assumed ────────────────────
-- `[observed — 2026-07-31]` sqlx wraps a migration's body and its `_sqlx_migrations` bookkeeping in
-- ONE transaction (`sqlx-postgres-0.8.6/src/migrate.rs:222-224`). So a `pending` row written from
-- inside a migration is rolled back by the very failure it exists to record — probed directly, zero
-- rows survive. **`failed` is unrecordable from inside a migration.**
--
-- And sqlx cannot tell us either: `_sqlx_migrations.success` is written in exactly one place,
-- hardcoded `TRUE` (`migrate.rs:288`). Nothing ever writes `false`, so `dirty_version()` — whose
-- only job is `SELECT version … WHERE success = false` — can never return a row on Postgres. The
-- `Migrate` trait's own comment promises "insert new row on completion (success or failure)"; the
-- Postgres implementation does not do that.
--
-- Hence the split, and it is not a style choice:
--   * the CLASS is written by the migration itself, inside its transaction — a claim that should
--     vanish along with a migration that never applied;
--   * the STATE is written by the runner, OUTSIDE that transaction, before and after the apply.
-- The runner is `temper-substrate`'s `migrate` command, which supplies the value sqlx's own
-- `dirty_version()` asks for and Postgres leaves dead.

-- ── The vocabulary ──────────────────────────────────────────────────────────────────────────────
-- `[decided — 2026-07-31, Pete]` Two classes, `additive` and `shape-breaking`. `DEPLOYING.md` is
-- updated in the same change to speak them. `additive` is DEFINED, per spec §3, as safe for a
-- binary that does not carry this migration — the lagging-binary test. Anything else is
-- `shape-breaking`. `destructive` is deliberately not a third class: dropping something IS
-- shape-breaking, and §3's auto-apply rule only ever asks whether a class is additive.
--
-- Each vocabulary lives in ONE place in SQL, as an enum. A CHECK constraint plus a validating RAISE
-- would be the same list written twice, free to drift.

-- 1. The two vocabularies.
CREATE TYPE migration_class AS ENUM ('additive', 'shape-breaking');

COMMENT ON TYPE migration_class IS
$c$Is this migration safe to apply ahead of the binary that pairs with it?
`additive` is defined as safe for a binary that does NOT carry it (spec §3); anything else is
`shape-breaking` and stays operator-gated. Not a severity scale — a two-valued answer to one
question.$c$;

CREATE TYPE migration_state AS ENUM ('pending', 'success', 'failed', 'cancelled');

COMMENT ON TYPE migration_state IS
$c$What happened to this migration's apply. `pending` is written before the body runs and is the
only trace a crashed migration leaves — which is what makes one detectable at all, since a failure
inside the transaction rolls back everything that transaction wrote. `cancelled` is for an apply an
operator abandoned deliberately, so it stays distinguishable from one that broke.$c$;

-- 2. The ledger. Append-only: nothing here is ever UPDATEd or DELETEd, which is why a correction is
--    additive and the claim it corrects survives beside it.
CREATE TABLE kb_migration_ledger (
    id           uuid            PRIMARY KEY DEFAULT uuid_generate_v7(),
    version      bigint          NOT NULL,
    state        migration_state,
    class        migration_class,
    reason       text            NOT NULL,
    recorded_by  text            NOT NULL DEFAULT 'migration',
    occurred_at  timestamptz     NOT NULL DEFAULT now(),

    -- An entry moving neither axis records nothing. At least one must be present.
    CONSTRAINT kb_migration_ledger_says_something
        CHECK (state IS NOT NULL OR class IS NOT NULL),
    CONSTRAINT kb_migration_ledger_reason_present
        CHECK (btrim(reason) <> '')
);

CREATE INDEX kb_migration_ledger_version_idx ON kb_migration_ledger (version, id DESC);

COMMENT ON TABLE kb_migration_ledger IS
$c$Every claim made about a migration, and everything that happened to it, in the order it was
recorded. Append-only — no UPDATE, no DELETE, no supersession column. A correction is a NEW entry,
so what the authors believed at the time survives beside what was later understood; that is the
whole reason this is a ledger and not a row, and it is the shape `kb_citation_audits` already uses
for the same reason.

Two independent axes. An entry may carry a `state`, a `class`, or both — so revising a
classification without touching the apply's outcome is expressible, and so is recording an outcome
without restating the claim. `migration_current` resolves each axis to its latest entry.$c$;

COMMENT ON COLUMN kb_migration_ledger.reason IS
$c$Why, in the author's words. Required and non-empty on every entry including state transitions: a
token alone records a verdict without its argument, and the 2026-07-30 misclassification was wrong
precisely in its reasoning, not in its vocabulary.$c$;

COMMENT ON COLUMN kb_migration_ledger.recorded_by IS
$c$What appended this — `migration` for a declaration written by the migration itself,
`reclassification` for a later revision, `runner` for a state transition written by
`temper-substrate migrate`, or an operator's own label. A runner cannot write a class and a
migration cannot honestly write a terminal state, so this doubles as a provenance check on the
entry.$c$;

-- 3. The current view. The read authority is the trail; these columns are derived at read, never a
--    memo that can drift from what produced it.
--
--    Ordered by `id DESC`, NOT `occurred_at DESC`. Entries written in one transaction share `now()`
--    to the microsecond and would tie; `id` is UUIDv7, so it is time-sortable AND unique, which
--    makes "latest" total rather than arbitrary.
CREATE VIEW migration_current AS
SELECT
    v.version,
    s.state,
    s.reason      AS state_reason,
    s.occurred_at AS state_at,
    c.class,
    c.reason      AS class_reason,
    c.occurred_at AS class_at
  FROM (SELECT DISTINCT version FROM kb_migration_ledger) v
  LEFT JOIN LATERAL (
      SELECT l.state, l.reason, l.occurred_at
        FROM kb_migration_ledger l
       WHERE l.version = v.version AND l.state IS NOT NULL
       ORDER BY l.id DESC
       LIMIT 1
  ) s ON true
  LEFT JOIN LATERAL (
      SELECT l.class, l.reason, l.occurred_at
        FROM kb_migration_ledger l
       WHERE l.version = v.version AND l.class IS NOT NULL
       ORDER BY l.id DESC
       LIMIT 1
  ) c ON true;

COMMENT ON VIEW migration_current IS
$c$Where each migration stands now: the latest state and the latest class, resolved independently so
a reclassification does not disturb a recorded outcome and vice versa. A NULL `state` means no
runner ever recorded one — migrations applied by sqlx-cli before this existed, or by any path that
does not bracket the apply. NULL is "not observed", never "did not happen".$c$;

-- 4. The declaration, written by the migration itself, inside its own transaction.
CREATE FUNCTION declare_migration(
    p_version bigint, p_class migration_class, p_reason text,
    p_recorded_by text DEFAULT 'migration'
) RETURNS void LANGUAGE sql AS $$
    INSERT INTO kb_migration_ledger (version, class, reason, recorded_by)
    VALUES (p_version, p_class, p_reason, p_recorded_by);
$$;

COMMENT ON FUNCTION declare_migration(bigint, migration_class, text, text) IS
$c$Declare whether this migration is safe to apply ahead of its binary. Called by the migration
itself, naming its own version — which must equal the filename's timestamp, giving CI a second,
free check: a copy-pasted declaration naming the wrong migration is caught mechanically.

`p_recorded_by` exists for the one case where the caller is NOT the migration being described: the
backfill of the 148 that shipped before this mechanism, which passes 'backfill'. It has to go
through this function rather than a raw INSERT, because CI reads declarations by parsing
`declare_migration` calls out of the migration FILES — a backfill written as a bare INSERT would be
invisible to the check that every version is declared.

Writes no state on purpose. It runs inside the migration's transaction, so any state it wrote would
vanish along with a migration that failed — which is exactly the case worth recording. State is the
runner's, from outside.

Silence is not caught here and cannot be: a migration that never calls this simply never calls it.
That half lives in CI, which reads the files rather than the table.$c$;

-- 5. The correction. The path the single-row design did not have.
CREATE FUNCTION reclassify_migration(p_version bigint, p_class migration_class, p_reason text)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO kb_migration_ledger (version, class, reason, recorded_by)
    VALUES (p_version, p_class, p_reason, 'reclassification');
$$;

COMMENT ON FUNCTION reclassify_migration(bigint, migration_class, text) IS
$c$Revise what an already-shipped migration is understood to be — without editing it, which is
impossible, and without destroying what it originally claimed, which is the point.

Distinct from `declare_migration` by intent rather than mechanics: they write the same shape, and
the separate name is what makes a correction legible in the trail and in `recorded_by`. The reason
should say what was believed before and what changed, because the entry beside it will not.

Touches no state. A reclassification says nothing about whether the migration applied.$c$;

-- 6. The state transition, written by the runner from outside the migration's transaction.
CREATE FUNCTION record_migration_state(
    p_version bigint, p_state migration_state, p_reason text, p_recorded_by text DEFAULT 'runner'
) RETURNS void LANGUAGE sql AS $$
    INSERT INTO kb_migration_ledger (version, state, reason, recorded_by)
    VALUES (p_version, p_state, p_reason, p_recorded_by);
$$;

COMMENT ON FUNCTION record_migration_state(bigint, migration_state, text, text) IS
$c$Record what happened to an apply. Called by `temper-substrate migrate` before the body runs
(`pending`) and after it resolves (`success` or `failed`), each in its own transaction so the
outcome survives the migration it describes.

A `pending` with no later terminal entry for the same version IS the dirty state. That is what the
runner returns from `Migrate::dirty_version` — a question sqlx already asks and its Postgres
implementation can never answer, since it reads `_sqlx_migrations.success = false` and nothing in
sqlx ever writes `false`.$c$;

-- 7. This migration declares itself. No state: it is being applied right now, and by whom is the
--    runner's business.
SELECT declare_migration(
    20260731000010,
    'additive',
    'Two new enums, one new table, one new view, three new functions. Nothing pre-existing is altered or dropped, and no binary reads any of them yet.'
);
