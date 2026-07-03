# Bootstrapping a Temper org

This runbook takes a **blank-but-stable** self-hosted Temper install — database up,
schema migrated, a compatible binary, MCP configured against your IdP — to a
**usable org**: an everyone-team every member auto-joins, an org-identity cognitive
map born + reconciled + bound, and team contexts that resources can be written into.

**Audience:** an operator standing up a new self-hosted instance (see
[self-hosting.md](./self-hosting.md) for the deploy that produces the blank install
this runbook starts from). You need a database with admin credentials for the one
irreducible root step, and an authenticated `temper` binary for everything else.

This is the **standard operating procedure** (SoP). Each step is a surfaced,
idempotent `temper` command except the first — the irreducible SQL root step. The
SoP doubles as the spec for the applier script
([`scripts/bootstrap/system-bootstrap.sh`](../../scripts/bootstrap/system-bootstrap.sh)),
which loops these same commands from a declarative
[`install-profile.yaml`](../../schema-artifact/install-profile.yaml). Run it by hand
to understand it; run the applier to repeat it.

> **Doing a full ground-up enterprise install?** This guide is one phase — the SAML bracket
> note below covers *this guide's* relationship to SAML setup specifically. For the single
> end-to-end sequence (deploy → SAML → org → agents) see [enterprise-install.md](./enterprise-install.md).

## Why a blank install isn't yet a usable org

Resource **writes into** a team context already work end-to-end — a team member can
`temper resource create --context +team/ctx` today. The chain breaks *above* the
write path: on a fresh install no team exists, no team-owned context can be created,
no org-identity cognitive map has been born, nothing is bound to a team, and
gating/admin configuration is SQL-only. This runbook closes that gap with the
provisioning surfaces shipped in chunks 1–6 of the org-provisioning work.

The shape generalizes the L0 kernel's **grant → reconcile** delivery
([l0-content-delivery.md](./l0-content-delivery.md)) from temper-the-system to
your-org-on-temper.

## What you end up with

| Outcome | Produced by |
|---------|-------------|
| A first system admin | the SQL root step (irreducible) |
| Instance settings (name, gating, mode) | `temper admin settings` |
| An everyone-team every member auto-joins | `temper team create … --auto-join-role watcher` |
| An org-identity cognitive map, born + populated | `temper cogmap create` then `temper cogmap reconcile` |
| The map reaching the org's shared corpus | `temper cogmap bind` |

## Prerequisites

- **A deployed, migrated instance.** `migrations/` applied; the API + MCP surfaces
  reachable. The canonical seed leaves `kb_system_settings.gating_team_slug` NULL, so
  `is_system_admin` is false for everyone until the root step below.
- **An `embed`-capable `temper` binary.** `cogmap create` / `cogmap reconcile` embed
  the charter client-side (ONNX). The default install bundles it; if you rebuilt the
  CLI, reinstall: `cargo install --path crates/temper-cli --locked --force`. A non-`embed`
  build returns a clear `requires the 'embed' feature` error rather than running.
- **Authentication.** The operator running the surfaced commands must be logged in
  (`temper auth login`, or `TEMPER_TOKEN` exported) **as the profile promoted in the
  root step** — a profile auto-provisions on its first authenticated request, so log
  in once *before* the root step to materialize it.
- **For the root step only:** `psql` and the database connection string (admin role).

## The sequence

> **SAML instances — this runbook is *bracketed* by SAML setup.** On an instance that fronts a
> SAML IdP (see [self-hosting-saml.md](./self-hosting-saml.md)), some SAML steps run *before* this
> sequence and one runs *after* it:
>
> 1. `temper admin saml provision` → set the emitted env on **both** Vercel functions → deploy →
>    apply the `kb_saml_idp` row. This must happen **before** anyone can authenticate.
> 2. The first admin signs in via SAML, which JIT-provisions their `kb_profiles` row — this is the
>    "must have signed in once already" precondition of step 0 below.
> 3. Run this runbook (step 0 root → steps 1–5), which creates the teams.
> 4. `temper admin saml map-group` → the `kb_saml_group_mappings` rows, **after** the teams those
>    groups map to exist (created in this runbook).
> 5. `temper admin saml verify`.

### 0. The irreducible SQL root step (operator-with-DB-credentials)

There is nothing to authenticate an admin-gated command against until the first admin
exists — so the first admin and the initial gating configuration are set directly in
the database. This is the **one** step that is not a surfaced `temper` command.

Find the first admin's profile id (it must have signed in once already):

```sql
SELECT id, handle FROM kb_profiles WHERE handle = '<the-operator-handle>';
```

Then point the gating slug at a gating team and promote the profile. The
`system_access = 'admin'` update fires the auto-join trigger, which mints the profile
as **owner** of the gating team — and `is_system_admin` reads gating-team ownership,
so this is what makes the profile a system admin:

```sql
-- Create the gating team if it does not exist (temper-system is the conventional slug).
INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System')
  ON CONFLICT (slug) DO NOTHING;

UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1;

UPDATE kb_profiles SET system_access = 'admin' WHERE id = '<first-admin-profile-uuid>';

-- Confirm the grant took:
SELECT is_system_admin('<first-admin-profile-uuid>');  -- expect: true
```

This is exercised verbatim as `root_bootstrap_first_admin` in
`tests/e2e/tests/admin_surface_e2e.rs`. Leaving `access_mode = 'open'` (the default)
keeps blast radius low: `gating_team_slug` then only enables
system-admin-by-team-ownership; the invite / join-request flow engages exclusively
under `access_mode = 'invite_only'`.

> **Snapshot prod before a hand-run data change.** On Neon, create a copy-on-write
> backup branch first — see [releasing.md](./releasing.md) / [DEPLOYING.md](../../DEPLOYING.md).

### 1. Instance settings

Now an admin exists; everything below is a surfaced, admin-gated command run as that
admin. Record the human-facing instance name (and confirm gating/mode):

```bash
temper admin settings --instance-name "Acme Temper"
# Show current settings (no flags ⇒ read):
temper admin settings
```

To promote a second admin so you are not a bus factor of one:

```bash
temper admin promote <second-admin-profile-uuid>    # defaults to the gating team
```

### 2. Create the everyone-team

A flat, **parentless** audience team every member auto-joins. It is deliberately
**not** the team DAG root — grants on a root would inherit *down* into every sub-team
and over-share. `--auto-join-role` is admin-gated and makes enrollment idempotent and
complete across open / invite_only:

```bash
temper team create everyone --name "Everyone" --auto-join-role watcher
```

Every existing profile with system access is backfilled into the team on enable, and
every future profile auto-joins — this is the org-wide audience pool that org cogmaps
are bound to.

### 3. Birth the org-identity cognitive map

Genesis births a new map with its telos charter from a genesis manifest (the org
analogue of the L0 kernel). Edit
[`schema-artifact/manifests/org-identity.yaml`](../../schema-artifact/manifests/org-identity.yaml)
to your org first:

```bash
temper cogmap create --manifest schema-artifact/manifests/org-identity.yaml
```

The output reports the realized identity:

```json
{ "cogmap_id": "019f…", "telos_resource_id": "019f…", "created": true }
```

Capture `cogmap_id` — the next two steps need it. Genesis is **idempotent at a given
id**: pin `cogmap_id` in the manifest (or pass `--id`) and a re-run is a no-op
(`created: false`). Without a pinned id the CLI mints a fresh one each run.

### 4. Deliver the map's landmark content

Genesis births the map and its charter; **reconcile** delivers the landmark content —
the same split L0 uses. Edit
[`schema-artifact/manifests/org-identity-landmarks.yaml`](../../schema-artifact/manifests/org-identity-landmarks.yaml),
then:

```bash
temper cogmap reconcile <cogmap-id> --manifest schema-artifact/manifests/org-identity-landmarks.yaml
```

Reconcile is idempotent — a re-run against unchanged content reports
`{ created: 0, updated: 0, folded: 0, unchanged: N, charter: "unchanged" }`.

### 5. Bind the map to the everyone-team

Binding widens the map's reach to the team's shared resources (an unbound map reaches
nothing through the team — empty join, default-closed):

```bash
temper cogmap bind <cogmap-id> +everyone
```

The org is now usable: members auto-join the everyone-team, the org-identity map is
born + populated + reaching the org's shared corpus, and resources written into a
team context (`temper context create <ctx> --owner +everyone`, then
`temper resource create --context +everyone/<ctx>`) land in a place the map can see.

## Running it as the applier

The applier automates steps 1–5 (and optionally step 0) from the declarative profile:

```bash
# Dry-run first — prints the commands without executing:
scripts/bootstrap/system-bootstrap.sh --dry-run

# Apply steps 1–5 (root step done manually per §0):
scripts/bootstrap/system-bootstrap.sh --profile schema-artifact/install-profile.yaml

# Or include the SQL root step (needs DATABASE_URL + psql):
DATABASE_URL=postgresql://… scripts/bootstrap/system-bootstrap.sh --run-root
```

It needs `yq` to read the profile and `temper` on PATH (authenticated). Because every
step is idempotent, re-applying the profile **converges** rather than duplicating —
pin the org-identity `cogmap_id` in the profile to keep genesis a no-op on re-runs.
There is no state backend; plan/diff (Terraform-like) semantics are deferred.

The SAML half of an install (provision the IdP, apply `kb_saml_idp`, map groups) is a
**separate** applier, [`scripts/bootstrap/saml-setup.sh`](../../scripts/bootstrap/saml-setup.sh) —
kept out of this script so `system-bootstrap.sh` stays auth-agnostic and usable for Auth0/Okta-OAuth
installs. See the SAML bracket note above and [self-hosting-saml.md](./self-hosting-saml.md#running-it-as-the-applier).

## Validation gate

The end-to-end sequence is proven by `tests/e2e/tests/org_bootstrap_e2e.rs` (the CI
"harness operator"): against a blank database it drives the exact SoP commands — root
step → `admin settings` → `team create … --auto-join-role` → `cogmap create` →
`cogmap reconcile` → `cogmap bind` — through the real `temper` binary, then asserts a
team-visible resource becomes reachable through the bound map. The test is
`embed`-gated (the CLI embeds client-side), so it runs in the Embed CI job /
`cargo make test-e2e-embed`.

## Deferred seams

- **Cogmap-write gate vs. team roles.** Org cogmaps bound to the everyone-team will
  eventually want **maintainers of that team** to write them, not only system admins.
  The interim gate for `cogmap create` / `reconcile` / `bind` is `is_system_admin`.
- **Plan/diff (Terraform-like) applier semantics.** The applier is stateless and
  idempotent; a desired-vs-actual diff is deferred.
- **Graduating the applier to a `temper admin init` subcommand.** It stays an external
  script first, by design.

## References

- Surfaced commands: `temper admin`, `temper team`, `temper cogmap`
  (`crates/temper-cli/src/cli.rs`).
- Root-step reference: `root_bootstrap_first_admin`
  (`tests/e2e/tests/admin_surface_e2e.rs`).
- Template / shape: [l0-content-delivery.md](./l0-content-delivery.md).
- Design: `docs/superpowers/specs/2026-06-28-org-provisioning-bootstrap-surface-design.md`
  (§4 Chunk 7, the capstone).
- Self-hosting deploy that produces the blank install: [self-hosting.md](./self-hosting.md).
