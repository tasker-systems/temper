# Temper documentation

Temper is an event-sourced coordination substrate for agent-assisted development. It keeps
goals, tasks, sessions, research and decisions durable across conversations, so understanding
compounds instead of being rebuilt every time.

These pages are the operational half: how to install it, run it, and integrate with it. The
*why* — attention as the organizing constraint, what a cognitive map is, what the substrate
guarantees — lives at [temperkb.io](https://temperkb.io).

## Three ways in

Pick the one that matches what you're doing. Each is a route, not a wall — they cross-link
freely.

- **[Using Temper](./doors/for-users.md)** — you want Temper working for you and your agents:
  install the CLI, wire up Claude, keep a session's context alive, grow a cognitive map.

- **[Running Temper](./doors/for-operators.md)** — you're standing up a deployment: self-host
  it, wire an identity provider, bootstrap an org, connect Slack and GitHub, get telemetry out.

- **[Building against Temper](./doors/for-integrators.md)** — you're writing code that talks to
  Temper: the HTTP API, machine credentials, and the language SDKs.

## The API reference

Every endpoint and schema is generated from the router itself, so it cannot drift from what
ships. It is published alongside these pages.
