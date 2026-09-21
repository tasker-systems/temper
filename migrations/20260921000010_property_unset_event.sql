-- Register `property_unset` — the key-grain delete verb for resource-owned properties.
--
-- A resource's open_meta key can be SET (`property_set` folds the key's live rows then asserts
-- a value) but never REMOVED: no event kind folds without asserting, so a metadata key, once
-- written, survives every update — the only escape was delete-and-recreate the whole resource.
-- `property_unset` completes the verb set: fold every live row for `(owner, property_key)`,
-- assert nothing.
--
-- THE FOLD PREDICATE IS NOT NEW — `_project_property_set` (20260730000010, its newest
-- definition) already folds exactly this live set; the unset is that clause without the
-- INSERT. Projection runs in Rust
-- (`events::project_property_unset`, the `property_retracted` shape: no `_project_*` function,
-- fire and replay share one body), and the resource-owned rows keep having no stable external
-- id — the verb is addressed BY KEY, the way every surface already addresses open_meta.
--
-- Permissive registration (NULL `payload_schema`), the `property_set` / `property_retracted`
-- precedent: typed payload struct, no committed schema snapshot, absent from
-- `TYPED_EVENT_NAMES`. `category = 'domain'` is spelled here because registration is the only
-- stamping moment (20260719000010 dropped the DEFAULT).
--
-- The name is ALSO seeded by `seed_system` from system.yaml: `reset_schema` TRUNCATEs this
-- registry and the boot-seed loop rebuilds it, so a type a write path fires must be on that
-- list or the reset+seed baseline fails "event_type property_unset not seeded".

INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
VALUES ('property_unset', NULL, 1, 'domain')
ON CONFLICT (name) DO NOTHING;

SELECT declare_migration(
    20260921000010,
    'additive',
    'registers one new event type (property_unset) with a NULL payload_schema and a fixed '
    'category; no table, column, function or constraint changes. The type is inert until the '
    'paired binary fires it, so the migration is safe to apply ahead of the binary.'
);
