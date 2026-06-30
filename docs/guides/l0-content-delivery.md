# Delivering L0 Kernel Cogmap Content

This guide is for operators delivering or updating the content of the **L0 kernel
cognitive map** (`system-default`) on a live Temper instance. It covers the
non-obvious **fail-closed admin gate** and the **grant → reconcile → re-lock**
procedure an operator must follow to write to L0.

**Audience:** operators with direct database access to the target instance
(`temperkb.io` or a self-hosted deployment). This is an operator runbook, not an
end-user flow — L0 is release/operator-governed, not operationally stewarded.

## What L0 is

The L0 kernel cogmap is the public, root-team-joined "what is temper" cognitive
map. It is **born deterministically by migration**
(`20260625000001_l0_kernel_cogmap.sql`) under the `system` actor, with reserved
ids:

| Entity | Reserved id |
|--------|-------------|
| L0 cogmap | `00000000-0000-0000-0005-000000000001` |
| L0 telos resource | `00000000-0000-0000-0005-000000000002` |
| Root team slug | `temper-system` |

L0 is a **living** map, but it evolves only by shipping **new additive
migrations** that call the substrate mutation functions against L0's reserved id
— never by editing the immutable birth migration. Its *content* (landmarks +
telos charter) is delivered separately from its *schema*, via the operator
reconcile flow described here.

## The command

```bash
temper cogmap reconcile 00000000-0000-0000-0005-000000000001 \
  --manifest schema-artifact/manifests/l0-kernel.yaml
```

This reconciles the live L0 map to the committed manifest: the 22 kernel
landmarks plus the telos charter. The CLI reads the manifest, **embeds each
entry client-side** (ONNX, via the `embed` feature), builds a pre-embedded
request, and PUTs it to `PUT /api/cognitive-maps/{id}`.

It is **idempotent** — a re-run against unchanged content reports zero changes.
The printed outcome (JSON by default) looks like:

```json
{ "created": 0, "updated": 0, "folded": 0, "unchanged": 22, "charter": "unchanged" }
```

The `charter` field is a distinct grain from the landmark counts. Its values are
`absent` (manifest carried no `telos:`), `unchanged`, `created` (first delivery
into an empty telos), or `updated` (live charter differed and was replaced).
First delivery reads `"created": 22, … "charter": "created"`.

### Prerequisites

- **Migration applied.** The charter-set primitive
  (`20260629000001_cogmap_charter_set.sql`, the `cogmap_charter_set` function)
  must be applied to the target database before a telos-bearing reconcile.
- **`embed`-capable binary.** The reconcile path embeds client-side, so it
  requires a `temper` binary built with the `embed` feature (the default
  install bundles it via `embed-download`, so no `ORT_DYLIB_PATH` is needed). If
  you changed `temper-cli`, reinstall first:
  ```bash
  cargo install --path crates/temper-cli --locked --force
  ```
  A non-`embed` build returns a clear `requires the 'embed' feature` error
  rather than running.
- **Admin grant.** The write is admin-gated and fail-closed — see below.

## The gotcha: L0 writes are fail-closed

`temper cogmap reconcile` against L0 is gated by
`require_cogmap_write_admin → is_system_admin`
(`crates/temper-api/src/services/access_service.rs`). Two things make this
non-obvious:

1. **`is_system_admin` does NOT read `kb_profiles.system_access`** (despite the
   name). It returns true only when the profile is an **`owner`** member of the
   team whose slug equals `kb_system_settings.gating_team_slug`. Having
   `system_access = 'admin'` does **not** help.
2. **`gating_team_slug` is `NULL` by canonical-seed default.** With it unset, no
   one is a system admin, so the L0 write gate denies **everyone** — the kernel
   is immutable out of the box. A reconcile attempt returns **403 Forbidden**.

This is intentional: the L0 special-case is fail-**closed** so an unconfigured
instance cannot have its kernel rewritten by any authenticated user. The cure is
not a permission flag — it is the temporary operator grant below.

## Procedure: grant → reconcile → re-lock

Connect to the target database first. For `temperkb.io` (Neon project
`crimson-fog-23541670`, PostgreSQL 17):

```bash
neonctl connection-string main \
  --project-id crimson-fog-23541670 \
  --org-id org-wild-snow-32921543 \
  --role-name neondb_owner
```

> **Always snapshot prod before a hand-run DDL/data change.** Create a
> copy-on-write Neon backup branch first — see
> [releasing.md](./releasing.md) / [DEPLOYING.md](../../DEPLOYING.md) for the
> `neonctl branches create … --parent main` convention. Restore with
> `neonctl branches restore main <backup-name>`.

### 1. Grant (temporary admin)

Point the gating slug at the root team and make the operator an `owner` of it:

```sql
UPDATE kb_system_settings SET gating_team_slug = 'temper-system';

INSERT INTO kb_team_members (team_id, profile_id, role)
VALUES (
  (SELECT id FROM kb_teams WHERE slug = 'temper-system'),
  '<operator-profile-uuid>',
  'owner'
)
ON CONFLICT (team_id, profile_id) DO UPDATE SET role = 'owner';

-- Confirm the grant took:
SELECT is_system_admin('<operator-profile-uuid>');  -- expect: true
```

**Blast radius is low when `access_mode = 'open'`** (the prod default).
`gating_team_slug` otherwise feeds only the invite / join-request flow, which
engages exclusively under `access_mode = 'invite_only'`. With open mode, setting
it merely enables system-admin-by-team-ownership for the duration of the grant.

### 2. Reconcile

Run the command from a checkout of the repo (the manifest path is repo-relative):

```bash
temper cogmap reconcile 00000000-0000-0000-0005-000000000001 \
  --manifest schema-artifact/manifests/l0-kernel.yaml
```

Confirm the outcome counts match expectation (first delivery creates; a re-run
reports `unchanged` / `charter: unchanged`).

### 3. Re-lock (restore fail-closed)

Undo the grant so L0 returns to immutable:

```sql
UPDATE kb_system_settings SET gating_team_slug = NULL;

DELETE FROM kb_team_members
WHERE team_id = (SELECT id FROM kb_teams WHERE slug = 'temper-system')
  AND profile_id = '<operator-profile-uuid>';
```

Delivered content persists. The next lifecycle update repeats this same
grant → reconcile → re-lock dance.

## Lifecycle framing

L0 content evolves through two complementary mechanisms:

- **Schema / structural birth and additive evolution** ship as **migrations**
  that call the substrate mutation functions against L0's reserved id. These are
  immutable once shipped.
- **Content delivery** (landmarks + telos charter) is **operator-directed
  reconciles** of the committed manifest, each gated by the temporary grant
  above.

Both are operator-governed; neither is ambient or steward-driven. L0's charter
declares its ambient steward wake = never.

## References

- Gate code: `crates/temper-api/src/services/access_service.rs`
  (`require_cogmap_write_admin`, `is_system_admin`).
- Birth migration: `migrations/20260625000001_l0_kernel_cogmap.sql`.
- Charter-set primitive: `migrations/20260629000001_cogmap_charter_set.sql`.
- Manifest: `schema-artifact/manifests/l0-kernel.yaml`.
- Design: `docs/superpowers/specs/2026-06-25-cognitive-map-agent-invocation-architecture-design.md`
  (L0 kernel cognitive map), `docs/superpowers/specs/2026-06-28-l0-telos-charter-delivery-design.md`
  (telos charter delivery, PR #199).
- Neon backup convention: [releasing.md](./releasing.md), [DEPLOYING.md](../../DEPLOYING.md).
