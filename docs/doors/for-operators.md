# Running Temper

For someone standing up a deployment and keeping it healthy — your own instance, your own
identity provider, your own data.

What the architecture fixes versus what your deployment chooses is set out at
[temperkb.io/operating](https://temperkb.io/operating). These pages are the runbooks.

## Standing it up

1. **[Self-hosting Temper](../playbooks/self-host-temper.md)** — the operator runbook: the
   Vercel project, the database, the environment.
2. **[Bootstrapping an org](../playbooks/bootstrap-an-org.md)** — turning a fresh deployment
   into one with people in it.

For a larger installation, **[Enterprise install](../playbooks/enterprise-install.md)** covers
the same ground with the decisions an enterprise has to make called out. It is an annotated
variant of the self-hosting runbook, not a separate path.

## Identity

Pick the one that matches your IdP:

- **[Okta](../playbooks/self-host-with-okta.md)**
- **[A SAML IdP](../playbooks/self-host-with-saml.md)** — the generic SAML path.

The trust boundary these configure — how a credential becomes a session, and where that is
enforced — is described in [The Trust Boundary](../concepts/trust-boundary.md). The
auth-identity contract — whose tokens this instance trusts and which it accepts — is in
[Auth identity](../concepts/auth-identity.md).

## Connecting the rest of your world

- **[Slack mentions](../playbooks/slack-mentions.md)** — end-to-end setup.
- **[Slack identity and revocation](../concepts/slack-identity-and-revocation.md)** — what the
  integration actually does: identity, credentials, and what is retained.
- **[GitHub connection](../playbooks/github-connection.md)** — provisioning a GitHub App and
  wiring it to Temper.

## Agents that run on your deployment

- **[Deploying a steward agent](../playbooks/deploy-a-steward-agent.md)** — fork-first
  deployment to your own Vercel project.
- **[Delivering L0 kernel content](../playbooks/deliver-l0-content.md)** — fork-first delivery
  of the L0 kernel cogmap.
- **[Bootstrapping a team's self-cognition map](../playbooks/bootstrap-team-self-cognition.md)**
  — standing up the team map the steward agents author into.

## Seeing what it is doing

- **[Sending traces to an OTLP backend](../playbooks/send-traces-to-an-otlp-backend.md)** —
  getting traces and metrics out.
- **[Telemetry](../concepts/telemetry.md)** — what Temper emits, the OTLP export model, and
  what the architecture fixes vs. what the deployment configures.

## Verifying releases

- **[Release verification](../concepts/release-verification.md)** — how to verify a Temper
  binary: manifest, signature, and attestation.

## Every setting

[The configuration reference](../reference/config/README.md) documents every field of
`config.toml` — type, default, and what it does — rendered from the config type itself.
