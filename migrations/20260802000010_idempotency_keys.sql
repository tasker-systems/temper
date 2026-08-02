-- Owner-scoped idempotency for the resource-create path (issue #581, spike rung 3).
-- Design: docs/research/2026-08-01-failed-write-recovery-spike.md.
--
-- ── Why owner-scoped, and not "the resource id is the key" ──────────────────────────────────────
-- The first cut used the client-supplied resource id itself as the idempotency key. That reopened an
-- existence oracle the read path deliberately closes: a create under a FREE id mints (200), but a
-- create under a FOREIGN-but-hidden id must refuse (404) — a distinction `GET` never makes (it 404s
-- both nonexistent and hidden alike). The oracle is inherent to keying on the GLOBAL id namespace,
-- where a hidden foreign resource collides with your create.
--
-- Keying on `(owner_profile_id, idempotency_key)` dissolves it by construction: the lookup is always
-- scoped to the caller, so a foreign key simply is not in your namespace — you mint fresh, there is no
-- collision to adjudicate, and nonexistent vs hidden stay indistinguishable exactly as they are for a
-- read. The client supplies a `key`; the SERVER mints the resource id.
--
-- ── Integrity is the create transaction's job, not a foreign key's ──────────────────────────────
-- The claim (`INSERT … ON CONFLICT DO NOTHING`) and the resource create run in ONE transaction: the
-- claim is inserted with a server-minted candidate id, then the resource is born under that id, then
-- both commit. On a key collision the insert no-ops and the create never runs (the caller gets the
-- already-committed resource back). So a claim can never point at a resource that does not exist —
-- there is no dangling-pointer window and thus no FK on `resource_id` (which a non-deferrable FK could
-- not express anyway, since the claim is written before the resource row within the same tx). The
-- `owner_profile_id` FK is safe (the profile predates the claim) and cascades on profile deletion.
CREATE TABLE kb_idempotency_keys (
    owner_profile_id UUID        NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    -- Client-minted opaque token (a UUIDv7 by convention). Owner-scoped, never the resource id.
    idempotency_key  UUID        NOT NULL,
    -- The resource this key minted. An internal pointer maintained by the create transaction, not a
    -- foreign key (see header). Replay reads it back to return the already-committed resource.
    resource_id      UUID        NOT NULL,
    created          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The uniqueness that makes replay converge instead of duplicating, and the only lookup index the
    -- claim/return path needs (both probe `WHERE owner_profile_id = $1 AND idempotency_key = $2`).
    PRIMARY KEY (owner_profile_id, idempotency_key)
);

-- Purely additive: a new table, referenced by no existing function or view, so a binary that does not
-- carry the create-path change ignores it entirely.
SELECT declare_migration(
    20260802000010,
    'additive',
    'New table kb_idempotency_keys(owner_profile_id, idempotency_key, resource_id) PK (owner, key) — owner-scoped idempotency for resource create (issue #581, spike rung 3). Additive: new table, no existing caller reads it; the create path writes it only when a client supplies an idempotency_key.'
);
