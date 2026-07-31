-- A migration says what it is, in a place a binary that does not carry it can still read.
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
--
-- The judgement does not go away — someone must still decide. What changes is that the answer is
-- written down, per migration, where something else can check it and contradict it.

-- ── Why the database, and not a file header ─────────────────────────────────────────────────────
-- Spec §1, verbatim: "The declaration must be readable by a binary that does NOT have that
-- migration, so it cannot live only in a file header: `MIGRATOR` embeds only the migrations its
-- binary carries. It is written into the database by the migration itself, and therefore propagates
-- to every target automatically."

-- ── Why one call, and not a header plus a row ───────────────────────────────────────────────────
-- [decided — 2026-07-31, Pete] The declaration IS the `declare_migration` call. CI greps
-- `migrations/*.sql` for it, so a file is checkable with no database at all; applying the migration
-- writes the row, so the same fact reaches every target. A header comment in addition would be a
-- second spelling of one fact, free to drift from the row it duplicates. Eight migrations already
-- carry an informal `-- Additive:` marker, in two spellings, read by nothing — that is what the
-- second spelling becomes.

-- ── The vocabulary, and why it is one word and not three ────────────────────────────────────────
-- [decided — 2026-07-31, Pete] Two classes: `additive` and `shape-breaking`. `DEPLOYING.md` is
-- updated in the same change to speak them; it previously used "additive", "non-additive" and
-- "destructive" while the design spec's cross-check table was written in "shape-breaking". A
-- classification whose two documents disagree on the words is this goal's failure mode in
-- miniature, since the check quotes the token back at whoever trips it.
--
-- `additive` is DEFINED, per spec §3, as safe with any binary in either direction — the old binary
-- against the new schema AND the new binary against the old schema. Anything else is
-- `shape-breaking`. `destructive` is deliberately not a third class: dropping something IS
-- shape-breaking, and §3's auto-apply rule only ever asks whether a class is additive.
--
-- The vocabulary lives in ONE place in SQL — the `migration_class` enum. A CHECK constraint plus a
-- validating RAISE would be the same list written twice, six lines apart, free to drift.

-- ── One ordering fact this whole design depends on ──────────────────────────────────────────────
-- [observed — 2026-07-31] sqlx executes a migration's body BEFORE inserting its ledger row:
-- `sqlx-postgres-0.8.6/src/migrate.rs:279` executes `migration.sql`, and only then (line 285)
-- INSERTs into `_sqlx_migrations`. So when `declare_migration` runs, this migration's own ledger
-- row does not exist yet. Hence NO foreign key to `_sqlx_migrations`, and no "is this version
-- applied?" check inside the function — either would fail on every migration that declares itself,
-- which is every migration.

-- 1. The vocabulary.
CREATE TYPE migration_class AS ENUM ('additive', 'shape-breaking');

COMMENT ON TYPE migration_class IS
$c$Is this migration safe to apply ahead of the binary that pairs with it?
`additive` is defined as safe with any binary in either direction (spec §3); anything else is
`shape-breaking` and stays operator-gated. Not a severity scale — a two-valued answer to one
question.$c$;

-- 2. The record. Keyed on `version` because that is what `_sqlx_migrations` is keyed on, and what a
--    LEFT JOIN against it will ask. No FK, per the ordering fact above.
CREATE TABLE kb_migration_classifications (
    version      bigint          PRIMARY KEY,
    class        migration_class NOT NULL,
    reason       text            NOT NULL,
    declared_at  timestamptz     NOT NULL DEFAULT now(),

    CONSTRAINT kb_migration_classifications_reason_present
        CHECK (btrim(reason) <> '')
);

COMMENT ON TABLE kb_migration_classifications IS
$c$What each applied migration claimed about its own safety, written by the migration itself.
One row per migration version. Rows are never updated: a shipped migration cannot be edited (sqlx
checksum-verifies at `sqlx-core/src/migrate/migrator.rs:175`), so its claim is fixed at the moment
it shipped. A re-declaration of an existing version is a primary-key violation on purpose — loud
beats silent.$c$;

COMMENT ON COLUMN kb_migration_classifications.reason IS
$c$Why the class is what it is, in the author's own words. Required and non-empty: a class token
alone records a verdict without its argument, and the 2026-07-30 misclassification was wrong
precisely in its reasoning, not in its vocabulary.$c$;

-- 3. The declaration. Every migration calls this exactly once, naming its own version.
CREATE FUNCTION declare_migration(p_version bigint, p_class migration_class, p_reason text)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO kb_migration_classifications (version, class, reason)
    VALUES (p_version, p_class, p_reason);
$$;

COMMENT ON FUNCTION declare_migration(bigint, migration_class, text) IS
$c$Declare whether this migration is safe to apply ahead of its binary. Called by the migration
itself, naming its own version — which must equal the filename's timestamp, giving CI a second,
free check: a copy-pasted declaration naming the wrong migration is caught mechanically.

The class is validated by the `migration_class` enum, so an unknown token fails at parse. Silence
is not caught here at all and cannot be: a migration that never calls this simply never calls it.
That half lives in CI, which reads the files rather than the table.$c$;

-- 4. This migration declares itself.
SELECT declare_migration(
    20260731000010,
    'additive',
    'One new enum, one new table, one new function. Nothing pre-existing is altered or dropped, and no binary reads any of them yet.'
);
