# Bootstrap an Org

**For operators.** This playbook takes a blank-but-stable self-hosted Temper
install — database up, schema migrated, a compatible binary, MCP configured
against your IdP — and turns it into a usable org with people in it.

By the end you will have: a first system admin, instance settings recorded,
an everyone-team every member auto-joins, an org-identity cognitive map born
and reconciled and bound to that team, and team contexts resources can be
written into. Every step after the first is an idempotent `temper` command,
so re-running the sequence converges rather than duplicating.

## Prerequisites

- **A deployed, migrated instance** — see
  [self-hosting Temper](./self-host-temper.md). For the full end-to-end
  sequence (deploy → SAML → org → agents), see the
  [enterprise install](./enterprise-install.md).
- **An `embed`-capable `temper` binary.** `cogmap create` and `cogmap
  reconcile` embed the charter client-side (ONNX). The default install
  bundles it; a non-`embed` build returns a clear `requires the 'embed'
  feature` error rather than running.
- **Authentication.** The operator running the commands must be logged in
  (`temper auth login`, or `TEMPER_TOKEN` exported) as the profile promoted
  in the root step below. A profile auto-provisions on its first
  authenticated request, so sign in once *before* the root step to
  materialize it.
- **For the root step only:** `psql` and the database connection string
  (admin role).

For the trust model that gates every command below, see
[trust boundary](../concepts/trust-boundary.md). For the team and role model
that membership and roles follow, see
[teams and roles](../concepts/teams-and-roles.md).

## Why a blank install isn't yet a usable org

Resource writes into a team context already work — a team member can
`temper resource create --context +team/ctx` today. The chain breaks *above*
the write path: on a fresh install no team exists, no team-owned context can
be created, no org-identity cognitive map has been born, nothing is bound to
a team, and gating/admin configuration is SQL-only. This playbook closes
that gap.

## What you end up with

| Outcome | Produced by |
|---------|-------------|
| A first system admin | the SQL root step (irreducible) |
| Instance settings (name, gating, mode) | `temper admin settings` |
| An everyone-team every member auto-joins | `temper team create … --auto-join-role watcher` |
| An org-identity cognitive map, born + populated | `temper cogmap create` then `temper cogmap reconcile` |
| The map reaching the org's shared corpus | `temper cogmap bind` |

## The sequence

> **SAML instances.** On an instance that fronts a SAML IdP, some SAML steps
> run *before* this sequence and one runs *after* it: provision the IdP and
> apply its row before anyone can authenticate; the first admin signs in via
> SAML to JIT-provision their profile (the precondition of step 0 below);
> run this playbook; then map SAML groups to the teams this playbook creates.
> For the full SAML sequence, see the
> [enterprise install](./enterprise-install.md).

### 0. The irreducible SQL root step

There is nothing to authenticate an admin-gated command against until the
first admin exists — so the first admin and the initial gating configuration
are set directly in the database. This is the **one** step that is not a
surfaced `temper` command.

Find the first admin's profile id (the operator must have signed in once
already so the profile row exists):

```sql
SELECT id, handle FROM kb_profiles WHERE handle = '<the-operator-handle>';
```

Then admit the profile and promote it. **These are two separate things**, and
each reads exactly one table:

- **Admission** — may this principal use the instance at all?
  `has_system_access` reads `kb_principal_standing` and nothing else. Without
  an `approved` row there, every gated request is a `403 SYSTEM_ACCESS_REQUIRED`.
- **Governance** — may this principal change the rules?
  `is_system_admin` reads `kb_principal_governance` and nothing else.

Neither reads team membership. Owning the gating team does not make anyone an
admin, and being an admin does not admit you.

```sql
-- Create the gating team if it does not exist (temper-system is the conventional slug).
INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System')
  ON CONFLICT (slug) DO NOTHING;

UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1;

-- Admit the profile. Prefer the function over a raw INSERT: it records the
-- transition in kb_principal_standing_events and emits the corresponding event.
SELECT principal_standing_apply(
    '<first-admin-profile-uuid>'::uuid, 'approve', 'approved', NULL, 'root bootstrap');

-- Promote it to system admin.
SELECT principal_governance_set(
    '<first-admin-profile-uuid>'::uuid, true, NULL, 'root bootstrap');

-- Confirm BOTH took — they are independent, and one without the other is a
-- silent half-state:
SELECT has_system_access('<first-admin-profile-uuid>'::uuid);  -- expect: true
SELECT is_system_admin('<first-admin-profile-uuid>'::uuid);    -- expect: true
```

> **Snapshot prod before a hand-run data change.** On Neon, create a
> copy-on-write backup branch first.

### 1. Instance settings

Now an admin exists; everything below is a surfaced, admin-gated command run
as that admin. Record the human-facing instance name (and confirm
gating/mode):

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

A flat, **parentless** audience team every member auto-joins. It is
deliberately **not** the team DAG root — grants on a root would inherit
*down* into every sub-team and over-share. `--auto-join-role` is admin-gated
and makes enrollment idempotent and complete across open / invite_only:

```bash
temper team create everyone --name "Everyone" --auto-join-role watcher
```

Every existing profile with system access is backfilled into the team on
enable, and every future profile auto-joins — this is the org-wide audience
pool that org cogmaps are bound to.

### 3. Birth the org-identity cognitive map

Genesis births a new map with its telos charter from a genesis manifest.
Save the following as `org-identity.yaml` and replace the prose with your
org's identity:

```yaml
# Org-identity cognitive map — GENESIS manifest.
#
# Consumed by:  temper cogmap create --manifest org-identity.yaml
#
# Replace the prose below with your org's identity. Keep the shape:
#   name         — the map's display name.
#   telos_title  — the title of the telos charter resource the map is born with.
#   telos        — the authored charter (statement + questions-with-context + framing).
#                  Embedded CLIENT-SIDE by the CLI (ONNX) before the POST.
#
# IDENTITY: omit cogmap_id / telos_resource_id and the CLI mints stable uuidv7s
# for you and prints them. Pin them here once you want a reproducible,
# re-runnable genesis — a re-run at the same id is an idempotent no-op
# (created: false).
#
#   cogmap_id: "019f1600-0000-7000-8000-000000000001"
#   telos_resource_id: "019f1600-0000-7000-8000-000000000002"

name: "Acme — organizational foundation"
telos_title: "Acme organizational telos"
telos:
  statement: >-
    Orient an agent or member arriving into Acme's temper instance so it can
    act correctly under this org's settled way of working — by holding what
    Acme is, the shared vocabulary its corpus is written in, the invariants
    it must not break, and the wayfinding that routes to the right team,
    context, and more-specific map. This is the org's bottom-referent: every
    domain map is situated by it and routes through it.
  questions:
    - question: >-
        Is this something an arriving member needs the moment they join to
        know what Acme is and where they stand — the org's mission, its
        teams, the contexts work lives in?
      context: "The first thing anyone asks is where am I. Hold the few situating landmarks, not their depth."
    - question: >-
        Is this a shared term an agent must know to read Acme's corpus at
        all, versus jargon a specific team or domain map can own?
      context: "Shared vocabulary is the floor for collaboration; deeper or team-local terms live where they are used."
    - question: >-
        Is this a settled way-of-working an agent must not break — review
        gates, what needs a human, which acts are not an agent's to make on
        this org's behalf?
      context: "State the always/nevers plainly as landmarks; a weaker model will not infer them."
    - question: >-
        When a member or agent needs to do something here, does this map
        name the team, context, or more-specific map to reach for — and make
        reaching the obvious next move?
      context: "Routing is the org map's keep: need X, the place is Y, go there."
  framing:
    - "This map is the org's self-portrait: what Acme is, mapped in Acme's own temper substrate."
    - "It is a reference layer — born curated, not accreted from work — and every domain map routes through it."
    - "Authored for the arriving member or agent: invariant-forward, scannable, landmark-shaped."
```

Then run genesis:

```bash
temper cogmap create --manifest org-identity.yaml
```

The output reports the realized identity:

```json
{ "cogmap_id": "019f…", "telos_resource_id": "019f…", "created": true }
```

Capture `cogmap_id` — the next two steps need it. Genesis is **idempotent at
a given id**: pin `cogmap_id` in the manifest (or pass `--id`) and a re-run
is a no-op (`created: false`). Without a pinned id the CLI mints a fresh one
each run.

> **Authority.** `cogmap create` is open to any authenticated profile with
> approved principal standing; the creator is granted read+write+grant on
> the new map. A caller-supplied id is honored only for a system admin.

### 4. Deliver the map's landmark content

Genesis births the map and its charter; **reconcile** delivers the landmark
content. Save the following as `org-identity-landmarks.yaml` and replace the
example landmarks with your org's:

```yaml
# Org-identity cognitive map — RECONCILE (content delivery) manifest.
#
# Consumed by:  temper cogmap reconcile <org-cogmap-id> \
#                 --manifest org-identity-landmarks.yaml
#
# Genesis births the map with its telos charter. This manifest DELIVERS the
# map's landmark content. Each entries[*] is a landmark resource keyed by a
# STABLE pre-generated uuidv7 (the reconcile diff key; origin_uri is pure
# attribution, never a key). The CLI embeds each body CLIENT-SIDE before the
# PUT.
#
# Reconcile is IDEMPOTENT: a re-run against unchanged content reports zero
# changes ({ created: 0, updated: 0, folded: 0, unchanged: N,
# charter: "unchanged" }). First delivery creates. Re-carrying telos: keeps
# the charter in lock-step; omit it to leave the charter untouched.
#
# Mint a fresh uuidv7 per new landmark and never reuse a retired one (fold
# it with fold_resources: instead).

telos:
  statement: >-
    Orient an agent or member arriving into Acme's temper instance so it can
    act correctly under this org's settled way of working — by holding what
    Acme is, the shared vocabulary its corpus is written in, the invariants
    it must not break, and the wayfinding that routes to the right team,
    context, and more-specific map. This is the org's bottom-referent: every
    domain map is situated by it and routes through it.
  questions:
    - question: >-
        Is this something an arriving member needs the moment they join to
        know what Acme is and where they stand — the org's mission, its
        teams, the contexts work lives in?
      context: "The first thing anyone asks is where am I. Hold the few situating landmarks, not their depth."
    - question: >-
        Is this a shared term an agent must know to read Acme's corpus at
        all, versus jargon a specific team or domain map can own?
      context: "Shared vocabulary is the floor for collaboration; deeper or team-local terms live where they are used."
    - question: >-
        Is this a settled way-of-working an agent must not break — review
        gates, what needs a human, which acts are not an agent's to make on
        this org's behalf?
      context: "State the always/nevers plainly as landmarks; a weaker model will not infer them."
    - question: >-
        When a member or agent needs to do something here, does this map
        name the team, context, or more-specific map to reach for — and make
        reaching the obvious next move?
      context: "Routing is the org map's keep: need X, the place is Y, go there."
  framing:
    - "This map is the org's self-portrait: what Acme is, mapped in Acme's own temper substrate."
    - "It is a reference layer — born curated, not accreted from work — and every domain map routes through it."
    - "Authored for the arriving member or agent: invariant-forward, scannable, landmark-shaped."

entries:
  - id: "019f1601-0000-7000-8000-000000000001"
    origin_uri: "org://acme/landmark/what-is-acme"
    title: "What Acme is"
    body: |
      Acme is the organization that owns this temper instance. This map is its
      bottom-referent: it says "this is the org you are in" and routes to the
      teams, contexts, and domain maps where the work actually lives. Hold the
      situating landmark here, not the depth.
    facets:
      layer: concept
    edges:
      - to: "019f1601-0000-7000-8000-000000000002"
        kind: leads_to
        label: "routes to the org's teams"

  - id: "019f1601-0000-7000-8000-000000000002"
    origin_uri: "org://acme/landmark/teams-and-contexts"
    title: "Teams and contexts"
    body: |
      Work in Acme is organized by teams (the unit of membership and sharing)
      and contexts (where resources are written). The everyone-team is the
      org-wide audience pool every member auto-joins; this org-identity map is
      bound to it so the map reaches the org's shared corpus. More-specific
      domain maps route through this one.
    facets:
      layer: reference

  - id: "019f1601-0000-7000-8000-000000000003"
    origin_uri: "org://acme/landmark/ways-of-working"
    title: "Settled ways of working"
    body: |
      The invariants an agent must not break on Acme's behalf: human-gated
      promotion across maps, attribution on every act, and the scoped-principal
      access floor. State the always/nevers plainly — a weaker model will not
      infer them.
    facets:
      layer: invariant
```

Then deliver the landmarks:

```bash
temper cogmap reconcile <cogmap-id> --manifest org-identity-landmarks.yaml
```

Reconcile is idempotent — a re-run against unchanged content reports
`{ created: 0, updated: 0, folded: 0, unchanged: N, charter: "unchanged" }`.

### 5. Bind the map to the everyone-team

Binding widens the map's reach to the team's shared resources (an unbound map
reaches nothing through the team — empty join, default-closed):

```bash
temper cogmap bind <cogmap-id> +everyone
```

> **Authority.** `cogmap bind` takes a system admin, **or** a team
> owner/maintainer who administers the map (on a non-gating team). The
> gating team itself can never be a bind target.

The org is now usable: members auto-join the everyone-team, the org-identity
map is born + populated + reaching the org's shared corpus, and resources
written into a team context (`temper context create <ctx> --owner +everyone`,
then `temper resource create --context +everyone/<ctx>`) land in a place the
map can see.

## Running it as the applier

The applier script `system-bootstrap.sh` automates steps 1–5 (and optionally
step 0) from a declarative profile. Save the following as
`install-profile.yaml`, filling in your first admin's profile id:

```yaml
# Declarative desired-state for bootstrapping a blank temper org.
# Input to system-bootstrap.sh. Idempotency is inherited from the primitives
# — every temper command is idempotent, so re-applying converges rather than
# duplicating. There is no state backend.

instance_name: "Acme Temper"

root:
  gating_team_slug: "temper-system"
  first_admin_profile_id: "REPLACE-WITH-FIRST-ADMIN-PROFILE-UUID"

auto_join_team:
  slug: "everyone"
  name: "Everyone"
  auto_join_role: "watcher"

org_identity:
  # id: "019f1600-0000-7000-8000-000000000001"  # pin for idempotent re-runs
  genesis_manifest: "org-identity.yaml"
  landmarks_manifest: "org-identity-landmarks.yaml"
  bind_teams:
    - "everyone"
```

Then run:

```bash
# Dry-run first — prints the commands without executing:
system-bootstrap.sh --dry-run

# Apply steps 1–5 (root step done manually per §0):
system-bootstrap.sh --profile install-profile.yaml

# Or include the SQL root step (needs DATABASE_URL + psql):
DATABASE_URL=postgresql://… system-bootstrap.sh --run-root
```

It needs `yq` to read the profile and `temper` on PATH (authenticated).
Because every step is idempotent, re-applying the profile **converges** rather
than duplicating — pin the org-identity `cogmap_id` in the profile to keep
genesis a no-op on re-runs. There is no state backend; plan/diff
(Terraform-like) semantics are deferred.

The SAML half of an install (provision the IdP, apply its row, map groups) is
a **separate** applier, `saml-setup.sh` — kept out of `system-bootstrap.sh`
so the script stays auth-agnostic and usable for Auth0/Okta-OAuth installs.
See the [enterprise install](./enterprise-install.md) for the full SAML
sequence.

## Validation

After the sequence completes, verify the org is usable end to end. Create a
context owned by the everyone-team and write a resource into it:

```bash
temper context create welcome --owner +everyone
temper resource create --context +everyone/welcome \
  --title "Hello, Acme" --body "The first resource in the org's shared corpus."
```

Then confirm the resource is reachable through the bound org-identity map:

```bash
temper cogmap show <cogmap-id>
```

The map's foundations should include the team context's resources. If the
resource is not visible, check that:

- `has_system_access` and `is_system_admin` both returned `true` for the
  admin (step 0 — they are independent; one without the other is a silent
  half-state).
- The everyone-team was created with `--auto-join-role watcher` (step 2).
- The cogmap was bound to `+everyone` (step 5).

## Further reading

- **The trust model that gates every command in this playbook:**
  [trust boundary](../concepts/trust-boundary.md).
- **The team and role model that membership and roles follow:**
  [teams and roles](../concepts/teams-and-roles.md).
- **Governance and administration:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
- **The base deployment this playbook starts from:**
  [self-hosting Temper](./self-host-temper.md).
- **The full end-to-end install sequence:**
  [enterprise install](./enterprise-install.md).
