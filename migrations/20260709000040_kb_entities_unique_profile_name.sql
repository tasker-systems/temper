-- Enforce `<handle>@<surface>` as a real natural key: unique on kb_entities (profile_id, name).
--
-- The schema never enforced it. `writes::resolve_emitter` reads the key with `fetch_one`, which in
-- sqlx is `fetch_optional` + RowNotFound — `sqlx::Error` has no too-many-rows variant. So a
-- duplicate never raised. It silently returned whichever row Postgres yielded first, splitting the
-- events of one logical emitter across two `entity_id`s. A quietly corrupted attribution ledger is
-- worse than a loud failure, which is what this migration converts it into.
--
-- Two writers could produce that duplicate. `provision_profile_entities` commits each emitter as
-- its own unguarded auto-commit statement, so two concurrent first-authenticated-requests for the
-- same new profile both observe no row and both insert. And a backfill migration racing that loop
-- sees a profile's `@web` before its own `@sdk` commits — the shape documented in
-- 20260709000030_backfill_sdk_emitter_entities.sql, which could only mitigate by deploy ordering.
--
-- Production was checked before this shipped: 10 entities, 10 distinct (profile_id, name) pairs,
-- zero duplicates. The quarantine below is therefore a no-op there. It is kept, and runs first,
-- because migrations are shipped code — self-hosted installs consume this repo on their own cadence
-- against databases we cannot inspect, and a bare CREATE UNIQUE INDEX would hard-fail on any of
-- them that carries a duplicate, stranding the operator mid-migration.
--
-- WHY QUARANTINE AND NOT DELETE. Two NOT NULL foreign keys reference kb_entities(id), and these are
-- the only two:
--
--   * kb_events.emitter_entity_id
--   * kb_invocations.scoped_entity_id
--
-- Deleting a duplicate that carries events would mean first repointing those events onto the
-- survivor — but `kb_events` is append-only, guarded by the `kb_events_append_only` trigger
-- (BEFORE DELETE OR UPDATE, see 20260624000001_canonical_schema.sql). Repointing an event is an
-- UPDATE on the ledger. The only way to do it is to disable that trigger inside a migration, which
-- would establish that migrations may rewrite history. We decline.
--
-- So the loser is renamed, not removed: `<handle>@<surface>` becomes
-- `<handle>@<surface>#dup-<id>`. Uniqueness is restored, every foreign key still resolves, and no
-- event moves. This is also the more honest repair — those events *were* emitted against that
-- entity row, and rewriting them to claim otherwise would falsify the ledger to make a constraint
-- fit. The renamed row is self-announcing, so an operator can audit the split after the fact.
-- `resolve_emitter` matches the canonical name and so now finds exactly one row.
--
-- The survivor is the lowest id per group: ids are UUIDv7 and Postgres orders uuid bytewise, so the
-- lowest is the oldest row — the row that first claimed the canonical name keeps it. Selected with
-- DISTINCT ON rather than `min(id)`, because Postgres defines no min/max aggregate over `uuid` on
-- either PG17 (Neon) or PG18 (local/CI).
--
-- OPERATOR NOTE — what quarantine costs, and how to see it. A quarantined row keeps the events and
-- invocations it carried, so on an install that *did* hold duplicates the history of one logical
-- emitter stays split across two entity ids, permanently. No consolidation tooling ships with this
-- migration, because consolidating means rewriting the append-only ledger. To find what was
-- quarantined, and how much history sits behind each:
--
--     SELECT e.id, e.name, count(ev.id) AS events
--       FROM kb_entities e
--       LEFT JOIN kb_events ev ON ev.emitter_entity_id = e.id
--      WHERE e.name LIKE '%#dup-%'
--      GROUP BY e.id, e.name;
--
-- An empty result — the case on temperkb.io — means nothing was ever duplicated.
--
-- Additive: no row is dropped, no column altered, so the additive-only-on-`main` invariant holds.
--
-- DEPLOY SKEW. This lands before the code that guards the insert with ON CONFLICT. In that window
-- the old unguarded loop runs against a constrained table, so a concurrent double-provision now
-- errors instead of silently duplicating. That is the trade this migration exists to make: the same
-- race, failing loudly. It is reachable only by the concurrent first-request path that was already
-- broken.
--
-- No CONCURRENTLY: the table holds one row per profile per surface, and CONCURRENTLY cannot run
-- inside the transaction sqlx wraps a migration in (no migration in this repo uses the
-- `-- no-transaction` escape).

-- 1. Quarantine every non-survivor by renaming it out of the canonical namespace. The join reads
-- the pre-UPDATE `name`, so a group of three collapses to one survivor and two distinct
-- `#dup-<id>` names in a single pass. Re-running the migration is a no-op: a quarantined name is
-- unique, so it is the sole member — and therefore the survivor — of its own group.
--
-- The loop exists because the quarantine namespace is NOT reserved. `name` is arbitrary text, and
-- a database we cannot inspect may already hold a row named exactly `<name>#dup-<loser id>`. A
-- single pass would rename the loser straight onto it, minting a fresh duplicate that fails the
-- index build below and rolls the whole migration back — the precise outcome this quarantine
-- exists to avoid. So: rename, then look again, until nothing collides.
--
-- It terminates. Names minted in one pass embed their own row's id, so they never collide with
-- each other — only, possibly, with a name that already existed. Each such collision consumes one
-- pre-existing name, which is never minted again (every minted name is strictly longer). The
-- supply of pre-existing names is finite, so the loop runs at most once per row.
DO $quarantine$
DECLARE
    renamed integer;
BEGIN
    LOOP
        WITH survivors AS (
            SELECT DISTINCT ON (profile_id, name) profile_id, name, id AS keep_id
              FROM kb_entities
             ORDER BY profile_id, name, id
        ), quarantined AS (
            UPDATE kb_entities e
               SET name = e.name || '#dup-' || e.id::text
              FROM survivors s
             WHERE s.profile_id = e.profile_id
               AND s.name = e.name
               AND e.id <> s.keep_id
            RETURNING 1
        )
        SELECT count(*) INTO renamed FROM quarantined;

        EXIT WHEN renamed = 0;
    END LOOP;
END
$quarantine$;

-- 2. The constraint itself. `ON CONFLICT (profile_id, name)` infers this index, which is what lets
-- `provision_profile_entities` and any future backfill drop their NOT EXISTS guards.
CREATE UNIQUE INDEX IF NOT EXISTS kb_entities_profile_id_name_key
    ON kb_entities (profile_id, name);
