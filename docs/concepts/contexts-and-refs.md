# Contexts and Refs

**For users, operators, and integrators.** If you will run more than one
command, you need to know how Temper addresses things.

## What a context is

A **context** is a named, owned boundary for resources — tasks, sessions, research notes,
concepts, decisions. Every resource lives in exactly one context. Contexts are what you share,
transfer, and subscribe to; resources are what you create and read inside them.

A context has an owner (a profile or a team) and a slug (a short name, unique per owner). The
slug is not unique globally — `@alice/work` and `@bob/work` are two different contexts. What is
unique is the **ref**, the addressing form that names a context unambiguously.

## The ref grammar

There are two grammars in Temper, and they are not interchangeable. Confusing them is the
first thing that goes wrong.

### Context refs

| Form | Names | Accepted by | Rejected by |
|---|---|---|---|
| `<uuid>` | a context, canonically | everywhere — all commands, all surfaces | nowhere |
| `@me/<slug>` | your own profile-owned context | most server-backed commands | `context share` and `context unshare` (client-side only) |
| `@<handle>/<slug>` | a named profile's context | everything `@me/` accepts, plus share/unshare | `context create --owner` |
| `+<team-slug>/<slug>` | a team-owned context | every context-ref position | MCP `context_manage` |
| `+<team-slug>` (no slug) | a **team**, not a context | team-typed arguments only | any context-ref position |
| `<name>` bare | nothing addressable | `context subscribe` / `unsubscribe` (local config only) | everywhere else |

### Resource refs — the second grammar

A **resource ref** is `slug-<uuid>` — the sluggified title with the UUID appended. It is
never a context ref. A reader who has internalised "the slug half is decoration" will try it
on a context argument and get an error naming neither grammar. The two grammars look similar;
they are not the same thing.

## The four traps

**1. `@me` is rejected by [`context share`](../reference/cli/context.md) and `context unshare`.** These are the only two
commands that reject it, and the rejection is client-side. There is no `temper profile`
command — to find your handle, run [`temper context list`](../reference/cli/context.md) and read the `owner_ref` column.

**2. A malformed ref reports "not found", not a grammar error.** The share/unshare commands
hand-roll their argument parsing rather than using the shared context-ref parser, so a syntax
error looks like a permissions problem. If share fails unexpectedly, check the ref form first.

**3. [`context create`](../reference/cli/context.md) is not idempotent.** Re-running it with the same name auto-suffixes:
`my-project` becomes `my-project-2`. A re-run silently forks the context rather than returning
the existing one. Scripts that assume idempotency will create duplicates.

**4. MCP `context_manage` takes UUIDs only.** The sibling MCP read tool advertises ref forms,
but the write tool's context argument is a UUID field. An agent following skill documentation
that shows `@me/temper` will pass it into a UUID field and fail.

## The model

**Read inherits up the team tree; write does not. Sharing is read-only; transferring ownership
is the only path to shared authorship.**

- A team's members can read contexts the team owns or is granted — membership confers read
  reach, automatically, through nested teams.
- Writing into a context requires administering it (or being an instance admin). Membership
  alone does not grant write.
- `context share` gives a team **read** access to a context you own. It does not let them
  write. The only way to share authorship is `context transfer`, which moves ownership.
- Renaming a context re-addresses it: the slug changes and every stored ref that used the old
  slug breaks. The UUID is stable; the slug is not. This is the argument for storing the UUID
  in scripts and automation rather than the slug.

## Further reading

- **The cognitive-map concept and what lives in a context:**
  [temperkb.io/cognitive-maps/what-lives-in-a-map](https://temperkb.io/cognitive-maps/what-lives-in-a-map).
- **How contexts grow and relate:**
  [temperkb.io/cognitive-maps/how-maps-relate](https://temperkb.io/cognitive-maps/how-maps-relate).
- **Using Temper from the CLI:**
  [temperkb.io/using-temper](https://temperkb.io/using-temper).
