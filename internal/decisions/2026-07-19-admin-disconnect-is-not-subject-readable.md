# An admin-initiated Slack disconnect is not readable by its subject

**Date:** 2026-07-19
**Status:** Decided
**Scope:** `slack_principal_disconnected` on the admin ledger
**Task:** `019f75ec-f82f-73f1-b038-81993e822f5a`

## Decision

`slack_principal_disconnected` gets **no arm** in `admin_ledger_service::readable_event_types`.
The fail-closed default (absence ⇒ admin-only) is the intended policy, not an unfinished edge.

A profile therefore cannot read a disconnect event about itself when an **admin** performed it.

## Why

An admin Slack disconnect is plausibly one step in an **offboarding playbook** — a script or SoP
that deprovisions a person across several systems. Those steps are ordered, and the ordering is not
guaranteed to put temper last. So there is a real window in which someone has been unbound from
Slack↔temper while still holding a live temper session, and still being present in other systems.

In that window, letting the subject read *"an admin disconnected your Slack principal at 14:02"*
leaks the **timing** of an in-flight administrative action — most consequentially a termination —
before the action has completed. The event body is unremarkable; the signal that an admin acted on
you, right now, is not.

This is a minor loophole rather than a severe one, and closing it is cheap. Closing it deliberately
as policy is worth more than the read access is.

## What is deliberately still readable

Self-serve disconnect stays visible to the person who did it. `list_by_actor` gates on
`caller == actor || is_system_admin`, and on the self-serve arm **actor == subject** — you disconnected
yourself, so there is no third party whose action is being concealed. No leak, and no reason to hide
a user's own act from them.

The distinction the policy actually draws is **actor ≠ subject**, not "disconnects are secret."

## How it is enforced

Three independent things, none of which required new code:

1. **No arm in `readable_event_types`** (`admin_ledger_service.rs:56-99`) ⇒ the type is returned only
   by the `is_system_admin` short-circuit. `fetch` binds the *authorized* set to `t.name = ANY($1)`,
   never the full `ADMIN_EVENT_TYPES` catalogue — the gate is not decorative.
2. **`list_by_actor` does not reach it either.** On an admin disconnect the actor is the admin, so a
   subject querying that axis matches nothing.
3. **`can_administer_grant` cannot open it.** Even if that predicate were true for a profile over
   itself, it grants only `grant_created` / `grant_revoked` — it does not name this type.

## Revisiting

If a future arm is proposed for this event type, this rationale must be revisited **first**. The
fail-closed default is doing policy work here, so "nobody wrote an arm yet" and "the arm is
deliberately absent" are indistinguishable in the code. That ambiguity is why this doc exists.

A more complete answer, if subject-visibility is ever wanted, is probably **deferred** disclosure —
visible once deprovisioning is complete — rather than immediate. That is a larger design than an
authz arm.
