-- The erasure record: the declared act, its member map, its holder snapshot and its estate snapshot,
-- the closed refusal vocabulary, the per-type fence views, and the D1.1 checksum.
--
-- Every row of the four record tables is a projection (spec R8): a projector writes it from an
-- event, and replay rebuilds it. This migration creates the shape only; it writes no record row and
-- nothing reads the tables yet. The vocabulary table is seeded here and is replay input, like
-- kb_event_types.
--
-- THE CONSTRAINTS A LATER EDIT MUST NOT BREAK:
--
--   * EVERY `*_table` COLUMN HAS A CHECK ON THAT COLUMN ALONE that lists its tables
--     (`col IN (...)`). resource_erasure_surface_test reads polymorphic edges only from such a
--     CHECK; a pairing CHECK beside it does not count. subject_table's CHECK names kb_resources, so
--     the resource-erasure reach walk enters kb_erasures and every table keyed to it: each column of
--     those tables is structural or a reference, never a carrier.
--   * THE STATUS TRIGGER IS NOT A NO-OP UNDER `temper.replaying`. Replay rebuilds the map with
--     TRUNCATE, which row triggers do not see, and then inserts; every UPDATE a projector makes,
--     live or in replay, meets the same moves.
--   * A ROW MARKED `erased` IS FROZEN. Re-projecting a born-erased row is an INSERT ... ON CONFLICT
--     DO NOTHING, never a DO UPDATE, which the trigger refuses as an erased -> erased move.
--   * THE CHECKSUM SEPARATORS ARE BYTES. chr(9) and chr(10): under standard_conforming_strings a
--     plain '\t' is a backslash and a `t`.
--   * kb_event_types.write_retired_at IS READ BY NOTHING YET. _event_append (20260719000010) does
--     not consult it; the first retirement raises there.
--
-- _erasure_redact_paths()'s COMMENT is re-issued to name its manifest's present path; its body is
-- unchanged.
--
-- THE LOCK BOUND. This migration takes strong locks on tables the live system writes or reads,
-- in this order:
--
--   1. ACCESS EXCLUSIVE on kb_event_types, for `ALTER TABLE ... ADD COLUMN write_retired_at`
--      (Section 0). Every _event_append reads kb_event_types, so from the grant to the commit
--      every append waits. It comes first, so its wait is queued before this migration holds any
--      other lock.
--   2. SHARE ROW EXCLUSIVE on kb_events, for each `REFERENCES kb_events(id)` (kb_erasures and
--      kb_erasure_members). It conflicts with the ROW EXCLUSIVE every append takes.
--   3. SHARE ROW EXCLUSIVE on kb_profiles, for kb_erasure_former_holders' `REFERENCES
--      kb_profiles(id)`. It conflicts with every write to a profile.
--   4. SHARE ROW EXCLUSIVE on kb_erasure_refusal_reasons and kb_erasures, tables this migration
--      creates, which nothing else can hold.
--
-- An additive migration runs while the previous deployment is still serving (DEPLOYING.md).
-- Unbounded, a request for any of these locks queues behind every open transaction holding a
-- conflicting one, and every later request queues behind it, so the deploy would stall the live
-- ledger for as long as the longest open transaction. So the whole migration runs under
-- `SET LOCAL lock_timeout = '500ms'`, below the default `deadlock_timeout` (1s), so each wait gives
-- up before a deadlock check would even start.
--
--   * temper-migrate applies each migration through sqlx's `Migrator`, which wraps one file and
--     its bookkeeping in one transaction (temper-migrate `ledger`, its module doc), and the test
--     runners apply each file as one multi-statement query, one implicit transaction. Either way
--     `SET LOCAL` lasts exactly as long as this file, and no later migration inherits it.
--   * When the bound fires, this migration rolls back, the build fails, the previous deployment
--     keeps serving on the schema it already has, and the next deploy retries the migration.
--   * It is set here, in the file, and not as a function's SET clause: a later CREATE OR REPLACE
--     silently drops a function's SET clauses (20261015100000_erasure_acts_pin_lock_timeout.sql's
--     header).
--   * It is the first migration to set a lock_timeout, because it is the first additive migration
--     that adds foreign keys into kb_events and a column to kb_event_types while a deployment
--     serves.
--   * The bound is per wait, and a lock once granted is held to commit. So the waits stack: while
--     this migration waits for 2 or 3 it already holds 1, and appends wait with it, for at most
--     500ms per wait. A wait that fires rolls everything back and releases every grant.

SET LOCAL lock_timeout = '500ms';

-- ---------------------------------------------------------------------------
-- Section 0. Retiring a kind for writing. First, so its lock is requested before any other.
-- ---------------------------------------------------------------------------
ALTER TABLE kb_event_types ADD COLUMN write_retired_at timestamptz;

COMMENT ON COLUMN kb_event_types.write_retired_at IS
    'When the kind was retired for writing; NULL while it may be appended. A retired kind stays '
    'registered so old ledgers replay. The Rust WRITE_RETIRED list names exactly the kinds whose '
    'column is set. _event_append does not read it yet.';

-- ---------------------------------------------------------------------------
-- Section 1. The closed refusal vocabulary.
-- ---------------------------------------------------------------------------
CREATE TABLE kb_erasure_refusal_reasons (
    code    text PRIMARY KEY CONSTRAINT kb_erasure_refusal_reasons_code_check
                             CHECK (code ~ '^[a-z][a-z0-9_]*$'),
    meaning text NOT NULL    CONSTRAINT kb_erasure_refusal_reasons_meaning_check
                             CHECK (btrim(meaning) <> '')
);

INSERT INTO kb_erasure_refusal_reasons (code, meaning) VALUES
    ('charter_resource', 'The item is a cogmap''s telos or charter resource, which only map-grain erasure erases.'),
    ('defect',           'A verifier raised on the item, so only a code or data fix lets it finish.'),
    ('batch_fatal',      'The item failed its batch alone until the reaper set it aside.'),
    ('unclassified',     'The item''s unit raised an error that no other code names.');

COMMENT ON TABLE kb_erasure_refusal_reasons IS
$c$The closed vocabulary of an erasure member's refusal_reason. Held set-equal, codes and meanings, to
policy/erasure/estate.toml [refusal_reasons], the bootseed rows in system.yaml and the Rust
ErasureMemberRefusalReason. No event writes it: replay restores it as input, like kb_event_types.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. The declared act.
-- ---------------------------------------------------------------------------
CREATE TABLE kb_erasures (
    id                    uuid        PRIMARY KEY,
    kind                  text        NOT NULL
                          CONSTRAINT kb_erasures_kind_check
                          CHECK (kind IN ('estate', 'resource', 'selection', 'block')),
    subject_table         text
                          CONSTRAINT kb_erasures_subject_table_check
                          CHECK (subject_table IN ('kb_profiles', 'kb_resources')),
    subject_id            uuid,
    request_ref           uuid,
    manifest_version      text
                          CONSTRAINT kb_erasures_manifest_version_check
                          CHECK (manifest_version ~ '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'),
    manifest_digest       text
                          CONSTRAINT kb_erasures_manifest_digest_check
                          CHECK (manifest_digest ~ '^[0-9a-f]{64}$'),
    legacy                boolean     NOT NULL DEFAULT false,
    declared_event_id     uuid        NOT NULL REFERENCES kb_events(id),
    scope_sealed_event_id uuid        REFERENCES kb_events(id),
    completed_event_id    uuid        REFERENCES kb_events(id),
    CONSTRAINT kb_erasures_id_kind_key UNIQUE (id, kind),
    CONSTRAINT kb_erasures_request_ref_key UNIQUE (kind, request_ref),
    CONSTRAINT kb_erasures_estate_subject_is_a_profile
        CHECK ((kind = 'estate') = (subject_table = 'kb_profiles')),
    CONSTRAINT kb_erasures_selection_has_no_subject
        CHECK ((kind = 'selection') = (subject_id IS NULL)),
    CONSTRAINT kb_erasures_subject_pair
        CHECK ((subject_table IS NULL) = (subject_id IS NULL)),
    CONSTRAINT kb_erasures_legacy_has_no_manifest
        CHECK (legacy = (manifest_digest IS NULL)),
    CONSTRAINT kb_erasures_legacy_has_no_version
        CHECK (legacy = (manifest_version IS NULL)),
    CONSTRAINT kb_erasures_legacy_has_no_request
        CHECK (legacy = (request_ref IS NULL)),
    CONSTRAINT kb_erasures_legacy_is_a_resource
        CHECK (NOT legacy OR kind = 'resource')
);

CREATE UNIQUE INDEX kb_erasures_one_estate_per_subject
    ON kb_erasures (subject_id)
    WHERE kind = 'estate';

COMMENT ON TABLE kb_erasures IS
$c$One row per declared erasure: an estate (subject a profile), a resource or a block (subject a
resource), or a selection (no subject). Projected from erasure_declared, or for a legacy row from
an old-door resource_erased; replay rebuilds it. A legacy row carries no manifest and no
request_ref. Complete means completed_event_id is set and no member is open; there is no stored
status.$c$;

COMMENT ON COLUMN kb_erasures.request_ref IS
    'The declaring request''s idempotency key, unique per kind. NULL exactly when legacy.';
COMMENT ON COLUMN kb_erasures.manifest_version IS
    'The release semver of the erasure policy the erasure was declared under, CHECKed to a semver '
    'shape so the column can hold nothing else. NULL exactly when legacy.';
COMMENT ON COLUMN kb_erasures.manifest_digest IS
    'sha256 (lowercase hex) of the canonical parsed erasure policy the erasure was declared under. '
    'NULL exactly when legacy.';
COMMENT ON COLUMN kb_erasures.declared_event_id IS
    'The erasure_declared event; for a legacy row, the subject''s first resource_erased or the '
    'legacy erasure_declared that recorded an owed completion pass.';
COMMENT ON COLUMN kb_erasures.completed_event_id IS
    'The latest erasure_completed. An arrival after completion clears it (erasure_reopened).';

-- ---------------------------------------------------------------------------
-- Section 3. The holder snapshot.
-- ---------------------------------------------------------------------------
CREATE TABLE kb_erasure_former_holders (
    erasure_id uuid NOT NULL REFERENCES kb_erasures(id),
    profile_id uuid NOT NULL REFERENCES kb_profiles(id),
    PRIMARY KEY (erasure_id, profile_id)
);

COMMENT ON TABLE kb_erasure_former_holders IS
$c$The profiles that held an erasure's items when it was declared, projected from the holder
snapshot in erasure_declared. A profile here is answered 410 RESOURCE_ERASED for the erasure's
items; every other profile gets what an unreadable item answers. Pseudonymous: profile ids only.$c$;

-- ---------------------------------------------------------------------------
-- Section 4. The member map.
-- ---------------------------------------------------------------------------
CREATE TABLE kb_erasure_members (
    erasure_id         uuid        NOT NULL REFERENCES kb_erasures(id),
    member_table       text        NOT NULL
                       CONSTRAINT kb_erasure_members_member_table_check
                       CHECK (member_table IN ('kb_resources', 'kb_contexts', 'kb_cogmaps',
                                               'kb_blobs', 'kb_edges')),
    member_id          uuid        NOT NULL,
    parent_table       text
                       CONSTRAINT kb_erasure_members_parent_table_check
                       CHECK (parent_table IN ('kb_contexts', 'kb_cogmaps')),
    parent_id          uuid,
    status             text        NOT NULL DEFAULT 'pending'
                       CONSTRAINT kb_erasure_members_status_check
                       CHECK (status IN ('pending', 'erased', 'refused')),
    prior_erasure_id   uuid        REFERENCES kb_erasures(id),
    late               boolean     NOT NULL DEFAULT false,
    refusal_reason     text        REFERENCES kb_erasure_refusal_reasons(code),
    refusal_sqlstate   text
                       CONSTRAINT kb_erasure_members_refusal_sqlstate_check
                       CHECK (refusal_sqlstate ~ '^[0-9A-Z]{5}$'),
    refusal_routine    text
                       CONSTRAINT kb_erasure_members_refusal_routine_check
                       CHECK (refusal_routine ~ '^[a-z_][a-z0-9_]{0,62}(\.[a-z_][a-z0-9_]{0,62})?$'),
    first_refused_at   timestamptz,
    erased_by_event_id uuid        REFERENCES kb_events(id),
    attempts           integer     NOT NULL DEFAULT 0
                       CONSTRAINT kb_erasure_members_attempts_check
                       CHECK (attempts >= 0),
    next_attempt_at    timestamptz,
    lease_expires_at   timestamptz,
    PRIMARY KEY (erasure_id, member_table, member_id),
    CONSTRAINT kb_erasure_members_parent_fkey
        FOREIGN KEY (erasure_id, parent_table, parent_id)
        REFERENCES kb_erasure_members (erasure_id, member_table, member_id),
    CONSTRAINT kb_erasure_members_refused_has_reason
        CHECK ((status = 'refused') = (refusal_reason IS NOT NULL AND first_refused_at IS NOT NULL)),
    CONSTRAINT kb_erasure_members_refusal_detail_only_when_refused
        CHECK (status = 'refused'
               OR (refusal_reason IS NULL AND refusal_sqlstate IS NULL AND refusal_routine IS NULL)),
    CONSTRAINT kb_erasure_members_pending_never_refused
        CHECK (status <> 'pending' OR first_refused_at IS NULL),
    CONSTRAINT kb_erasure_members_erased_has_event
        CHECK ((status = 'erased') = (erased_by_event_id IS NOT NULL)),
    CONSTRAINT kb_erasure_members_prior_only_when_erased
        CHECK (prior_erasure_id IS NULL OR status = 'erased'),
    CONSTRAINT kb_erasure_members_prior_is_another
        CHECK (prior_erasure_id <> erasure_id),
    CONSTRAINT kb_erasure_members_late_has_no_prior
        CHECK (NOT (late AND prior_erasure_id IS NOT NULL)),
    CONSTRAINT kb_erasure_members_parent_pair
        CHECK ((parent_table IS NULL) = (parent_id IS NULL)),
    CONSTRAINT kb_erasure_members_erased_is_off_the_drain
        CHECK (status <> 'erased' OR (lease_expires_at IS NULL AND next_attempt_at IS NULL))
);

-- The fence: is this item a member of any erasure?
CREATE INDEX kb_erasure_members_item
    ON kb_erasure_members (member_table, member_id) INCLUDE (erasure_id, status);
-- Oldest-first claiming of open items, per type.
CREATE INDEX kb_erasure_members_open
    ON kb_erasure_members (member_table, erasure_id, member_id)
    WHERE status <> 'erased';
-- A container's open members.
CREATE INDEX kb_erasure_members_open_by_parent
    ON kb_erasure_members (erasure_id, parent_table, parent_id)
    WHERE status <> 'erased';
-- Health: the oldest refusals.
CREATE INDEX kb_erasure_members_refused
    ON kb_erasure_members (first_refused_at)
    WHERE status = 'refused';

COMMENT ON TABLE kb_erasure_members IS
$c$An erasure's map: one row per item, keyed by (erasure_id, member_table, member_id), member_table
holding the DDL name. An item may belong to several erasures. Any row, whatever its status, fences
its item. A row is born pending or erased, never refused, and its status moves pending -> erased,
pending -> refused, refused -> refused and refused -> erased only
(kb_erasure_members_status_moves). attempts, next_attempt_at and lease_expires_at are the drain's
working columns and are not replayed; every other column is projected from the ledger.$c$;

COMMENT ON COLUMN kb_erasure_members.parent_table IS
    'The container member (kb_contexts or kb_cogmaps) of the same erasure; with parent_id, a '
    'foreign key into this table, so containers are inserted before their leaves.';
COMMENT ON COLUMN kb_erasure_members.prior_erasure_id IS
    'The earlier erasure that already finished this item. Set only on a row born erased.';
COMMENT ON COLUMN kb_erasure_members.late IS
    'Mapped after the declaration, by erasure_member_arrived. Excluded from the declared checksum.';
COMMENT ON COLUMN kb_erasure_members.refusal_routine IS
    'The routine that raised the refusal, a function name and never a message: CHECKed to a '
    'lowercase identifier, optionally schema-qualified, as refusal_sqlstate is CHECKed to a '
    'SQLSTATE. The record is never redacted, so the column must not be able to hold free text.';
COMMENT ON COLUMN kb_erasure_members.first_refused_at IS
    'When the item was first refused. Set only by a move into refused, and never changed once '
    'set: a retry keeps it, and so does the move to erased.';
COMMENT ON COLUMN kb_erasure_members.erased_by_event_id IS
    'The item''s authorising event, or for a row born erased the event that erased it before.';

-- The status moves. Modelled on kb_principal_standing_events_append_only (20260720000040): BEFORE,
-- FOR EACH ROW, and refusing rather than repairing. An INSERT is checked for its birth: a row is
-- born pending or erased, and reaches refused by an UPDATE. first_refused_at is set only by a move
-- into refused, so a row born erased, or moved pending -> erased, carries none; on a pending row
-- kb_erasure_members_pending_never_refused refuses it, and the trigger leaves that case to the
-- CHECK so the CHECK's name reports it. The one rewrite it
-- makes is the refused -> erased move clearing the refusal's reason, SQLSTATE and routine, so the
-- projector that marks an item erased writes the same statement whatever the row's prior status.
CREATE FUNCTION kb_erasure_members_status_moves()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'kb_erasure_members: a member row is never deleted'
            USING ERRCODE = 'check_violation';
    END IF;

    IF TG_OP = 'INSERT' THEN
        IF NEW.status = 'refused' THEN
            RAISE EXCEPTION 'kb_erasure_members: a member row is born pending or erased, never refused'
                USING ERRCODE = 'check_violation';
        END IF;
        IF NEW.status = 'erased' AND NEW.first_refused_at IS NOT NULL THEN
            RAISE EXCEPTION 'kb_erasure_members: first_refused_at is set only by a move into refused'
                USING ERRCODE = 'check_violation';
        END IF;
        RETURN NEW;
    END IF;

    IF (NEW.erasure_id, NEW.member_table, NEW.member_id, NEW.parent_table, NEW.parent_id,
        NEW.prior_erasure_id, NEW.late)
       IS DISTINCT FROM
       (OLD.erasure_id, OLD.member_table, OLD.member_id, OLD.parent_table, OLD.parent_id,
        OLD.prior_erasure_id, OLD.late) THEN
        RAISE EXCEPTION 'kb_erasure_members: identity, parent, prior_erasure_id and late are fixed at insert'
            USING ERRCODE = 'check_violation';
    END IF;

    IF OLD.first_refused_at IS NOT NULL
       AND NEW.first_refused_at IS DISTINCT FROM OLD.first_refused_at THEN
        RAISE EXCEPTION 'kb_erasure_members: first_refused_at is never changed once set'
            USING ERRCODE = 'check_violation';
    END IF;

    IF OLD.status = 'erased'
       OR (OLD.status = 'refused' AND NEW.status = 'pending') THEN
        RAISE EXCEPTION 'kb_erasure_members: the status move % -> % is refused', OLD.status, NEW.status
            USING ERRCODE = 'check_violation';
    END IF;

    IF OLD.first_refused_at IS NULL AND NEW.first_refused_at IS NOT NULL
       AND NEW.status = 'erased' THEN
        RAISE EXCEPTION 'kb_erasure_members: first_refused_at is set only by a move into refused'
            USING ERRCODE = 'check_violation';
    END IF;

    IF OLD.status = 'refused' AND NEW.status = 'erased' THEN
        NEW.refusal_reason   := NULL;
        NEW.refusal_sqlstate := NULL;
        NEW.refusal_routine  := NULL;
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER kb_erasure_members_status_moves
    BEFORE INSERT OR DELETE OR UPDATE ON kb_erasure_members
    FOR EACH ROW
    EXECUTE FUNCTION kb_erasure_members_status_moves();

COMMENT ON FUNCTION kb_erasure_members_status_moves IS
  'Refuses an INSERT on kb_erasure_members whose status is refused (a row is born pending or '
  'erased), DELETE, any change to a row''s identity, parent, prior_erasure_id or late, any '
  'change to first_refused_at once set, a first_refused_at on a row born erased or moved '
  'pending -> erased (it is set only by a move into refused), and every status move but pending -> pending, '
  'pending -> erased, pending -> refused, refused -> refused (the same or a changed reason) and '
  'refused -> erased, which it makes clear the refusal''s reason, SQLSTATE and routine. An erased '
  'row is frozen. Raises check_violation naming the move. Enforced under temper.replaying too; '
  'replay''s TRUNCATE is not a row event.';

-- ---------------------------------------------------------------------------
-- Section 5. The estate snapshot.
-- ---------------------------------------------------------------------------
CREATE TABLE kb_erasure_estate_snapshots (
    erasure_id     uuid    PRIMARY KEY,
    kind           text    NOT NULL DEFAULT 'estate'
                   CONSTRAINT kb_erasure_estate_snapshots_kind_check
                   CHECK (kind = 'estate'),
    resource_count integer NOT NULL CONSTRAINT kb_erasure_estate_snapshots_resource_count_check
                                    CHECK (resource_count >= 0),
    context_count  integer NOT NULL CONSTRAINT kb_erasure_estate_snapshots_context_count_check
                                    CHECK (context_count >= 0),
    cogmap_count   integer NOT NULL CONSTRAINT kb_erasure_estate_snapshots_cogmap_count_check
                                    CHECK (cogmap_count >= 0),
    blob_count     integer NOT NULL CONSTRAINT kb_erasure_estate_snapshots_blob_count_check
                                    CHECK (blob_count >= 0),
    edge_count     integer NOT NULL CONSTRAINT kb_erasure_estate_snapshots_edge_count_check
                                    CHECK (edge_count >= 0),
    checksum       text    NOT NULL CONSTRAINT kb_erasure_estate_snapshots_checksum_check
                                    CHECK (checksum ~ '^[0-9a-f]{64}$'),
    CONSTRAINT kb_erasure_estate_snapshots_erasure_fkey
        FOREIGN KEY (erasure_id, kind) REFERENCES kb_erasures (id, kind)
);

COMMENT ON TABLE kb_erasure_estate_snapshots IS
$c$An estate erasure's point-in-time estate: the declared member count per type and the map's
checksum (erasure_map_checksum). The same counts and checksum are in the erasure_declared payload,
so the snapshot is verified against the ledger, never trusted.$c$;

-- ---------------------------------------------------------------------------
-- Section 6. The fence views: every member row of one type, whatever its status.
-- ---------------------------------------------------------------------------
CREATE VIEW kb_erasure_fenced_resources AS
    SELECT m.member_id AS id, m.erasure_id FROM kb_erasure_members m
     WHERE m.member_table = 'kb_resources';
CREATE VIEW kb_erasure_fenced_contexts AS
    SELECT m.member_id AS id, m.erasure_id FROM kb_erasure_members m
     WHERE m.member_table = 'kb_contexts';
CREATE VIEW kb_erasure_fenced_cogmaps AS
    SELECT m.member_id AS id, m.erasure_id FROM kb_erasure_members m
     WHERE m.member_table = 'kb_cogmaps';
CREATE VIEW kb_erasure_fenced_blobs AS
    SELECT m.member_id AS id, m.erasure_id FROM kb_erasure_members m
     WHERE m.member_table = 'kb_blobs';
CREATE VIEW kb_erasure_fenced_edges AS
    SELECT m.member_id AS id, m.erasure_id FROM kb_erasure_members m
     WHERE m.member_table = 'kb_edges';

COMMENT ON VIEW kb_erasure_fenced_resources IS
    'The resources any erasure maps, whatever the member''s status: a gate consults it as a '
    'top-level EXISTS conjunct, never inside one arm.';
COMMENT ON VIEW kb_erasure_fenced_contexts IS
    'The contexts any erasure maps, whatever the member''s status: a gate consults it as a '
    'top-level EXISTS conjunct, never inside one arm.';
COMMENT ON VIEW kb_erasure_fenced_cogmaps IS
    'The cogmaps any erasure maps, whatever the member''s status: a gate consults it as a '
    'top-level EXISTS conjunct, never inside one arm.';
COMMENT ON VIEW kb_erasure_fenced_blobs IS
    'The blobs any erasure maps, whatever the member''s status: a gate consults it as a '
    'top-level EXISTS conjunct, never inside one arm.';
COMMENT ON VIEW kb_erasure_fenced_edges IS
    'The edges any erasure maps, whatever the member''s status: a gate consults it as a '
    'top-level EXISTS conjunct, never inside one arm.';

-- ---------------------------------------------------------------------------
-- Section 7. The map checksum (D1.1).
-- ---------------------------------------------------------------------------
-- The sha256 idiom of 20260624000002_canonical_functions.sql. The header and each line end in a
-- newline byte, fields are tab-separated, and the lines are the declared (non-late) rows in
-- (member_table COLLATE "C", member_id) order:
--
--   header = 'temper-erasure-map/v1' TAB erasure_id TAB manifest_digest LF
--   line   = member_table TAB member_id TAB parent_table TAB parent_id TAB prior_erasure_id LF
--
-- with an absent parent or prior erasure as the empty string. NULL for a legacy erasure, which has
-- no manifest digest and no checksum, and for an id that names no erasure.
CREATE FUNCTION erasure_map_checksum(p_erasure uuid)
RETURNS text
LANGUAGE sql
STABLE
AS $$
    SELECT encode(sha256(convert_to(
               'temper-erasure-map/v1' || chr(9) || e.id::text || chr(9) || e.manifest_digest || chr(10)
               || coalesce((SELECT string_agg(
                                       m.member_table || chr(9) || m.member_id::text || chr(9)
                                       || coalesce(m.parent_table, '') || chr(9)
                                       || coalesce(m.parent_id::text, '') || chr(9)
                                       || coalesce(m.prior_erasure_id::text, '') || chr(10),
                                       '' ORDER BY m.member_table COLLATE "C", m.member_id)
                              FROM kb_erasure_members m
                             WHERE m.erasure_id = e.id AND NOT m.late), ''),
               'UTF8')), 'hex')
      FROM kb_erasures e
     WHERE e.id = p_erasure AND NOT e.legacy;
$$;

COMMENT ON FUNCTION erasure_map_checksum(uuid) IS
$c$The checksum of an erasure's declared map (late rows excluded), as erasure_declared and
kb_erasure_estate_snapshots carry it: sha256 hex over a 'temper-erasure-map/v1' header naming the
erasure and its manifest digest, then one tab-separated line per member in (member_table COLLATE
"C", member_id) order. NULL for a legacy erasure, which has no checksum. Computed only here, so no
other runtime re-implements it.$c$;

-- ---------------------------------------------------------------------------
-- Section 8. The allowlist's COMMENT names its present manifest.
-- ---------------------------------------------------------------------------
COMMENT ON FUNCTION _erasure_redact_paths() IS
$c$The ledger exception's allowlist: every (event type, path, qualifier) a resource_erased event may
name in redacted_fields, with its sentinel class; `keep` marks a qualified line that refines a
redact line back to structural (the literal keys doc_type and facet). Generated from the [payload]
lines of policy/erasure/resource-erasure.toml and held equal to them by
resource_erasure_surface_test (the_allowlist_is_the_manifests_redact_lines). A function, so
widening what the verifier admits takes DDL.$c$;

SELECT declare_migration(
    20261201000010,
    'additive',
    'Adds the tables kb_erasure_refusal_reasons (seeded with four codes), kb_erasures, '
    'kb_erasure_former_holders, kb_erasure_members (with its status trigger) and '
    'kb_erasure_estate_snapshots; the views kb_erasure_fenced_resources, _contexts, _cogmaps, '
    '_blobs and _edges; the function erasure_map_checksum(uuid); and the nullable column '
    'kb_event_types.write_retired_at, which nothing reads. Re-issues the COMMENT on '
    '_erasure_redact_paths() only. No existing table, column, constraint, function body or row '
    'changes, and nothing writes or reads the new objects yet, so a binary without this change is '
    'unaffected. Runs under SET LOCAL lock_timeout = 500ms: if a lock on kb_events or '
    'kb_event_types is not granted in time, it rolls back and the serving deployment is untouched.'
);
