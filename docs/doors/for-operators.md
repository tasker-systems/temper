# Running Temper

For someone standing up a deployment and keeping it healthy — your own instance, your own
identity provider, your own data.

What the architecture fixes versus what your deployment chooses is set out at
[temperkb.io/operating](https://temperkb.io/operating). These pages are the runbooks.

## Standing it up

1. **[Self-hosting Temper](../guides/self-hosting.md)** — the operator runbook: the Vercel
   project, the database, the environment.
2. **[Bootstrapping an org](../guides/org-bootstrap.md)** — turning a fresh deployment into one
   with people in it.

For a larger installation, **[Enterprise install — ground up](../guides/enterprise-install.md)**
covers the same ground with the decisions an enterprise has to make called out.

## Identity

Pick the one that matches your IdP:

- **[Okta](../guides/self-hosting-okta.md)**
- **[A SAML IdP](../guides/self-hosting-saml.md)** — the generic SAML path.

The trust boundary these configure — how a credential becomes a session, and where that is
enforced — is described in [Temper auth & security](../auth/README.md).

## Connecting the rest of your world

- **[`@temper` on Slack](../guides/slack-setup.md)** — end-to-end setup.
- **[What the Slack integration actually does](../guides/slack-integration.md)** — identity,
  credentials, and what is retained.
- **[Provisioning a GitHub connection](../guides/github-connection-temper.md)**
- **[A read-only GitHub credential via your own App](../guides/github-connection-infra.md)**

## Agents that run on your deployment

- **[Cloud agents](../guides/cloud-agents.md)** — the model for agents that run without a human
  in the loop.
- **[Deploying an Eve agent to Vercel](../guides/vercel-eve.md)**
- **[Delivering L0 kernel cogmap content](../guides/l0-content-delivery.md)**

## Seeing what it is doing

- **[OpenTelemetry setup](../guides/open-telemetry-setup.md)** — getting traces and metrics out.
- **[Drain operator queries](../guides/drain-operator-queries.md)** — the TraceQL queries worth
  having to hand when something looks wrong.
