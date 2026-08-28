# Security Policy

## Reporting a vulnerability

**Please do not open a public issue, discussion, or pull request for a security vulnerability.**

Temper backs a live, internet-facing hosted instance *and* ships as a self-hostable product. A
public report describes the weakness to everyone running an unpatched deployment before any of them
can act on it.

Two private channels, either is fine:

1. **[GitHub private vulnerability reporting](https://github.com/tasker-systems/temper/security/advisories/new)** — preferred. It keeps the
   report, the discussion, the fix, and the eventual advisory in one place, and it lets us credit you
   automatically.
2. **Email `pete.jc.taylor@hey.com`** if you would rather not use GitHub, or if you cannot reach the
   form.

Useful things to include, in rough order of how much they help:

- What you can do that you should not be able to do, and which surface you did it through
- The smallest reproduction you have
- Which version, and whether you observed it on the hosted instance or a self-hosted deployment
- Your read on impact — who is affected and how badly
- A suggested fix, if you have one

You do not need all of these. A partial report is worth sending.

## What to expect

| | |
|---|---|
| Acknowledgement | within 48 hours |
| Initial assessment — is it a vulnerability, and how bad | within 5 working days |
| Fix timeline | shared with you once the assessment is done; it depends on severity and on whether self-hosted deployments are exposed |

This is a small project. Those are commitments about *responsiveness*, not about having a team on
call.

We will coordinate disclosure timing with you rather than announcing on our own schedule, and we
will credit you in the advisory and release notes unless you would rather stay anonymous.

## Scope

**In scope** — the hosted instance at `temperkb.io`, the published `temper` binary and its release
artifacts, the HTTP API, the MCP surface, and the authorization and identity model that governs all
of them.

**Out of scope**, because these are documented properties of the design rather than defects:

- **Anything requiring database credentials.** The event ledger records acts that pass through the
  application. Below that line you are in the domain of database controls and infrastructure policy —
  a deliberate, published non-goal, described in [the trust boundary](docs/concepts/trust-boundary.md)
  and in the governance and observability pages on the site. A report that assumes possession of
  `DATABASE_URL` is describing the boundary, not crossing it.
- **Anything requiring a valid system-administrator principal.** Administrators can administer.
- **Findings against a deployment whose operator has changed the documented security configuration** —
  for example by widening CORS, sharing secrets across gates that the configuration requires to
  differ, or enabling a surface the playbooks say to leave off.
- Missing hardening headers, or scanner output, with no demonstrated impact.

If you are unsure which side of a line something falls on, report it. We would much rather read an
out-of-scope report than not hear about an in-scope one.

## Supported versions

| Version | Supported |
|---|---|
| 0.3.x | Yes |
| < 0.3 | No |

Temper is pre-1.0 and moves quickly. Fixes land on the current minor series; there are no backports
to earlier ones. **If you self-host, staying current is part of your security posture** — see
[release verification](docs/concepts/release-verification.md) for how to check that what you are
running is what we published.

## How this project handles security work internally

Worth stating, because it explains something you might otherwise read as neglect.

**Security-relevant gaps are not filed as public issues here.** For a live, internet-facing instance
with self-hosted deployments downstream, an open issue describing an unfixed weakness is a
disclosure rather than a task. So they are tracked privately and land as ordinary hardening changes.

The consequence, stated plainly: **the public issue tracker is not a complete picture of known work,
and it is not meant to be.** What is public is the fix, once it exists.

The same reasoning governs how changes are described. A pull request or release note says what a
change establishes going forward. It is not the place for an account of what was wrong beforehand,
because that account is actionable against everyone who has not yet upgraded.

### Who may set aside a security finding

Two different acts, deliberately kept apart, because conflating them is how a finding disappears
while everyone believes it was handled.

**Releasing a merge** — the `codeql-override` label on a pull request. It says *this analysis did
not pass and the merge proceeds anyway*, and it exists because CodeQL is separate compute that
sometimes stalls; a stalled required check blocks a pull request indefinitely, since pending is not
failed. It overrides the merge and **not** the finding: any alert raised stays open, and the label
stays on the merged pull request, so what was released and when is answerable afterwards from the
pull request itself.

**Dismissing an alert** — closing the finding in the Security tab as a false positive, as won't-fix,
or as used-in-tests. This is the one that ends a finding rather than deferring it, and the
convention is that it is the project owner's, or an agent the owner has explicitly authorized for
it. Every dismissal records a reason; "won't fix" with no stated reason is not a dismissal, it is a
deletion.

**This is a convention, not a control, and the difference matters.** GitHub grants alert dismissal
to anyone with write access to the repository, and offers no finer-grained permission for it — so
nothing mechanically prevents a dismissal outside this convention. What is available is detection
after the fact: every alert records who dismissed it and why. Stating that here is the point, since
a convention documented as though it were enforced is worse than one documented as a convention.

## What the project does to hold its ground

- **Fail-closed authorization.** Absence denies. A principal with no standing row has no access, and
  a record type nobody has authorized as readable is not readable by default.
- **Compile-time-checked SQL.** Queries go through `sqlx` macros verified against the real schema.
- **Authorization tripwires in CI.** Twelve audit scripts under `.github/scripts/` freeze
  security-relevant sets — grant write-sites, route auth coverage, ungated query fragments, signature
  secrets, elevation claims — against reviewed baselines, so the set cannot grow without someone
  acknowledging it. Each has its own test, and each states in its header what it does *not* catch.
- **Signed, verifiable releases.** Build-provenance attestation against a pinned Sigstore trust root,
  with sha256-pinned native dependencies. `temper attest` verifies an installed binary.
- **Dependency and secret scanning.** `cargo audit` blocking in CI, Dependabot, GitHub secret
  scanning with push protection.
- **CodeQL** static analysis, scoped to the languages a change can reach, reporting into the same
  single required check every other CI job does. **What that gates is that the analysis RAN** — a
  crashed, stalled or timed-out analysis blocks a merge. It does **not** yet gate on what the
  analysis *found*: the CodeQL action uploads results rather than failing on them, so blocking a
  merge on a finding needs code scanning merge protection on the branch ruleset, which is not
  enabled here. Findings alert; they do not block.

None of these is a guarantee, and several of them say so in their own documentation. They are what
makes a regression loud rather than silent.
