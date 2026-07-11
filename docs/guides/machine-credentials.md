# Machine Credentials

A **machine principal** is a non-human agent — a steward, a CI job, an SDK client — that
authenticates to Temper with its own credential instead of a person's login. It acts as an
**agent profile**: an ordinary Temper profile that holds team memberships and capability grants,
and is governed by exactly the same access rules as a human with the same memberships.

This guide covers standing one up and reasoning about what it can do: the two ways to mint a
credential, who is allowed to mint one, how a machine's reach is bounded to its minter's authority,
and the credential lifecycle (rotation, rebind, revocation). It is the consumable summary of three
design specs — machine-principal registration (Phase A), the issuer grant (Phase B1), and
team-owner registration with reach containment (Phase B2) — linked at the end; reach for those when
you need the rationale.

## Two ways to mint a credential

Temper supports two kinds of machine credential. Pick by **who owns the secret**.

| | `provision` | `issue` |
|---|---|---|
| **Secret lives at** | an external IdP (Auth0 M2M app) | Temper (Temper is the Authorization Server) |
| **`client_id`** | you supply the IdP's client id | Temper mints it (`tmpr_…`) |
| **Secret** | held by the IdP; Temper never sees it | Temper mints a 256-bit secret, **returned once**, stored only as a SHA-256 hash |
| **`issuer` recorded** | `auth0-m2m` | `temper` |
| **Use when** | you already run an IdP and want it to keep minting M2M tokens | you want Temper to be the whole loop — no external IdP |

Both create the same thing on Temper's side: an agent profile, its emitter entities, its
gating-team enrollment, and a `kb_machine_clients` registration row. The difference is only where
the secret and token-minting live. A token from either kind is verified the same way and rides the
same authorization rails.

```bash
# External IdP: register an Auth0 M2M app you already created
temper admin machine provision --client-id <auth0-client-id> --label "acme ci"

# Temper-issued: Temper mints the id and secret; the secret prints once
temper admin machine issue --label "acme steward"
```

> The `issue` command prints the plaintext secret **exactly once**. Temper stores only its hash and
> cannot recover it — capture it at mint time or rotate to get a new one.

## Who may mint one

Registration is authorized by **team ownership**:

> `is_system_admin` **OR** owner of the team that will own the machine.

A system admin — the owner of the gating team — can mint any machine. A **team owner** can mint a
machine owned by *their own* team, without an operator in the loop. That is the point of the model:
a team runs its own agents.

The owning team is set with `--owner-team`. It records **who owns the machine — not what the machine
can reach** (see the next section). If you omit `--owner-team`, the machine is teamless, and a
teamless machine is **admin-only** to create, read, or operate — the empty owning team fails closed,
never open.

```bash
# A team owner mints a machine for their own team — no admin needed
temper admin machine issue --label "acme steward" --owner-team acme-eng
```

## Reach containment — a machine can reach nothing you couldn't

A machine's **reach** is the teams it belongs to (`--team`) and the cognitive maps it can write
(`--cogmap`). Reach is always **explicit and plural** — it is never inferred from `--owner-team`.

The load-bearing rule: **a non-admin may only grant a machine reach they could confer on a human
themselves.**

| Requested reach | You must hold |
|---|---|
| `--team <ref>[:role]` | `owner` or `maintainer` on that team (`can_manage`), and the role may not be `owner` |
| `--cogmap <ref>[:ro]` | `can_grant` on that cognitive map |

So a team owner can enroll a machine into any team they manage (at `member`/`maintainer`/`watcher`,
never `owner`) and grant it write on any map they can already delegate — and nothing beyond that. A
machine also never receives `can_grant` or `can_delete` on a map: it cannot re-delegate its own
access.

A **system admin** is exempt from this check (they can already confer anything), so an admin may mint
a machine with any reach.

```bash
# Team owner: enroll the machine in a team they manage, grant write on a map they can delegate
temper admin machine issue --label "acme steward" \
  --owner-team acme-eng \
  --team acme-eng:maintainer \
  --cogmap acme-roadmap
```

## The lifecycle — who can do what

Once a machine exists, its owning-team owner can operate it — with one deliberate exception.

| Command | Who | Notes |
|---|---|---|
| `provision` / `issue` | admin, or owner of the owning team | mint a credential |
| `rotate-secret` | admin, or owner of the machine's team | roll a temper-issued secret |
| `revoke` | admin, or owner of the machine's team | deny the credential |
| `list` / `show` | admin sees all; a team owner sees only machines owned by their teams | |
| `rebind` | **system admin only** | see below |

**`rebind` is admin-only, and it is the one exception on purpose.** Every other command merely
*operates on* a machine's row. `rebind` is different in kind: it **transplants an existing agent
profile's identity — and the full reach that profile already holds — onto a new `client_id`**. That
inherited reach may have been granted by an admin and can exceed a team owner's own authority, so
team ownership cannot safely bound it. `rebind` therefore keeps the system-admin bar, and it refuses
a **revoked** source outright (a dead credential is re-created with a fresh `provision`, never
resurrected). A team owner rotating a *temper-issued* credential uses `rotate-secret`, not `rebind`;
`rebind` is for rotating the external IdP application behind an `auth0-m2m` machine.

### Rotation

`rotate-secret` mints a new secret and keeps the **previous** one valid for a grace window (default
24h, capped at 7 days) so a running fleet can pick up the new secret with no downtime. At most **two**
secrets are ever live at once; a second rotation drops the oldest.

```bash
temper admin machine rotate-secret <machine-id> --grace 3600   # old secret valid 1 more hour
```

### Revocation

`revoke` denies the credential's authentication on the **very next request**, on both the HTTP API
and the MCP surface — the gate re-checks the registration on every call, so a still-unexpired token
stops working immediately.

Revocation is **credential-scoped**. It deliberately **leaves the agent profile's team memberships
and capability grants in place** — so revoking one credential does not tear down an agent's reach.
If you need to kill an agent's *reach*, that is a separate, explicit step: remove its team
memberships and grants, or deactivate the profile.

```bash
temper admin machine revoke <machine-id>
```

## Machine RBAC comes for free

Because a machine's reach is contained to its minter's authority and the machine acts as an ordinary
agent profile, **a machine is governed by exactly the same team-and-grant rules as a human with the
same memberships.** There is no machine-specific authorization path. A team-bound machine cannot read,
write, or grant anything outside what its profile's memberships and grants confer.

This is what makes autonomous and managed-agent sessions safe without a human in the loop: the agent
inherits its permissions from the credential, rather than a caller having to reconstruct what is safe
from the shape of the graph. The credential *is* the boundary.

## Machine credentials vs. proxied human auth

Temper also supports human authentication proxied through SAML or OAuth (see
[self-hosting-saml.md](self-hosting-saml.md)). Choose by **who is acting**:

- **Machine credential (this guide)** — the actor is a service or agent with no human behind it. It
  holds its own long-lived credential, authenticates as an agent profile, and is bounded by the reach
  it was minted with. Use this for stewards, CI, and SDK clients running unattended.
- **SAML / OAuth-proxied** — the actor is a person, authenticated through your identity provider,
  acting as their own human profile with whatever teams and grants they hold. Use this for interactive
  users.

An integration (temper-rb, temper-py, temper-ts) may support both: a proxied human token for
interactive use, and a machine credential for unattended runs. They resolve to different profiles
with different reach; they are not interchangeable.

## Command reference

```
temper admin machine provision --client-id <id> --label <l> [--owner-team <ref>] [--team <ref>[:role]]... [--cogmap <ref>[:ro]]...
temper admin machine issue --label <l> [--owner-team <ref>] [--team <ref>[:role]]... [--cogmap <ref>[:ro]]...
temper admin machine rotate-secret <machine-id> [--grace <seconds>]
temper admin machine rebind <from-machine-id> --client-id <new-id> --label <l> [--no-revoke-old]   # admin only
temper admin machine revoke <machine-id>
temper admin machine list [--include-revoked]
temper admin machine show <machine-id>
```

- `--team` / `--cogmap` are **repeatable** — pass one per team or map. A `--team` ref may carry a
  role suffix (`acme-eng:maintainer`, default `member`); a `--cogmap` ref may carry `:ro` for
  read-only (default read + write).
- `--owner-team` records the machine's **owner**, never its reach.
- `rebind`'s `--no-revoke-old` leaves the old credential live for an overlap window instead of
  revoking it in the same step.

## See also

- **Design specs (authoritative):**
  - Registration, rotation & revocation gate (Phase A): `docs/superpowers/specs/2026-07-10-machine-principal-registration-design.md`
  - Temper as a `client_credentials` issuer (Phase B1): `docs/superpowers/specs/2026-07-10-machine-principal-phase-b1-issuer-grant-design.md`
  - Team-owner registration + reach containment (Phase B2): `docs/superpowers/specs/2026-07-11-machine-principal-phase-b2-team-owner-registration-design.md`
- [Working with Teams](teams.md) — the roles and ownership this model builds on.
- [Self-hosting with SAML](self-hosting-saml.md) — the proxied-human auth path.
