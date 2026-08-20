# Operational Memory

**For users and operators.** If you want working knowledge from agent sessions to persist across

## What operational memory is

A **memory** is a resource with doc type `memory`. It lives in a
[context](./contexts-and-refs.md), and it is read, written, corrected, and searched with the
ordinary resource commands and tools — nothing about it is special-cased beyond its type.
What it needs beyond a body lives in its open tier:

- **`status`** — `active` or `superseded`. The field the index renders on, and the one it
  refuses to guess.
- **`verified`** — the ISO date the claim was last checked against the system. An old date
  means nobody has re-checked it; it carries no opinion about whether the claim still holds.
- **`descriptor`** — the one-line recall hook. A line in the rendered index is a title, not
  the memory; the descriptor is what makes that line worth reading.
- **`source_file`** — the local file a memory was migrated from, when it was migrated from
  one. A memory authored natively in Temper carries none and is never reported as orphaned.

Everything else — reach, history, attribution, search — is what a resource already does. A
memory makes no claim about its own audience: it reaches whoever can read the
[context](./contexts-and-refs.md) it lives in.

## How memories are captured and recalled

Memories are authored as resources, the same way any other note is. What is special is the
**rendered index** — a projection a client loads each session so a cold start is not a blank
slate. The index is generated, never hand-written: a previously-emitted index starts with a
generated-file header, and the emit command refuses to overwrite anything that does not carry
it. The file is a **cache**; the resources are the record. A client with no index file and no
command line reads the same memories over the cloud and needs none of the local machinery.

Recall is ordinary search and ordinary resource reads. The index lists hooks — a title and a
`verified` date per line — and its preamble states once what a line is, how to resolve one,
and what an old date does *not* mean. The marker for an unexamined claim is `UNEXAMINED`, never
*stale* or *wrong*: an old date means nobody has re-checked the claim, not that it is false. A
memory whose `status` or `verified` is missing or unparseable stops the whole render rather
than being emitted with a guessed value — those are the fields the render's own claims depend
on, and nothing validates them at write time.

## The CLI memory model

A machine opts into the local index by adding a `[memory]` section to its Temper config. The
section is deliberately optional, not defaulted, so *not configured* and *configured empty*
stay distinguishable — and at least one context is required, so an empty block is rejected
rather than silently doing nothing.

| Field | Meaning |
|---|---|
| `shared_contexts` | contexts whose memories reach every project on this machine |
| `project_contexts` | contexts scoped to one project. Both lists are read shared-first and deduped, so a context named in both is fetched and rendered once |
| `index_path` | where the rendered index is written, and the directory scanned for local files |
| `stale_after_days` | how old a `verified` date must be before the index marks it `UNEXAMINED`. Defaults to 90 |
| `reinforced_min` | how many distinct reinforcement dates a memory needs to keep its own line. No default — see below |

`opted_in: false` is not an error; it is the primary state the status command exists to report,
and a supported end state. A machine may measure itself, decide against the whole idea, and
never return. Declining is deleting the section — nothing else changes: no local file is
touched, no resource is removed, and the status command keeps answering.

### The collapsed tail — demotion, never deletion

An index bounded by hand stops being bounded the moment nobody trims it. Reinforcement dates —
the dates a memory did work — let the index bound itself by evidence instead. `reinforced_min`
is the threshold: below it, a memory stops rendering individually and falls into a per-section
tail line that states its own count and carries a route back to the full list.

The threshold has no default, and leaving it out is the right answer until months of real
reinforcement data exist to choose one from. A threshold can only honestly be chosen from
evidence, and guessing one now would put a constant with no evidence behind it into every
reader's index. **Demotion is never deletion**: nothing is dropped, nothing becomes
unfindable, and the file stays bounded however large the corpus grows. This is the same rule
the rest of the design already holds — supersession replaces deletion, and falling out of a
summary is never falling out of the record.

## Shared memory across a team

A memory reaches whoever can read the [context](./contexts-and-refs.md) it lives in. There is
no per-memory reach field, and a memory makes no claim about its own audience, so team
adoption is a context question, answered with the commands that already exist:

- Put the shared memories in a context the team can read — either a team-owned context
  (`+team-slug/<slug>`) or one you own, shared into the team's read-reach with
  `context share`. Sharing requires that you administer the context **and** manage the target
  team (owner or maintainer), or that you are an instance administrator. `@me` shorthand is
  **not** accepted by `context share` — use your handle or the context UUID.
- Each member names that context in their own `[memory]` section, under `shared_contexts` if
  it should reach every project on their machine or `project_contexts` if it is scoped to one.
- Members who work from Desktop, mobile, or the web name nothing and configure nothing. They
  read and author the same memories as resources.

Two consequences worth being explicit about. **Reach follows membership and grants, so it
changes when the team changes** — nothing has to be re-tagged when someone joins or leaves.
And **a member who never adopts the CLI is not a second-class participant**: the index is one
client's rendering, not the mechanism. For the team and role model that governs who can read
and write shared contexts, see [Teams and Roles](./teams-and-roles.md).

## What makes a memory worth keeping

A memory earns its place when it carries a **claim worth re-checking**, does work often enough
to be reinforced, and has a **descriptor that stands alone** — a hook that means something in a
flat list, not a fragment that only made sense inside the sentence that produced it. A title
harvested from link text that read fine inside its sentence — *"Slack — `[topic file]`"* — is
useless standing alone, because the sentence supplied the subject. Retitle offenders after the
first render; renaming a memory is not re-checking its claim, so it does not advance
`verified`.

When two accounts describe nearly the same thing, compare the **claim, not the wording**.
Shared vocabulary collides on surface text while making unrelated assertions — two memories
that mention the same subsystem are not an overlap — and that gets **worse in a shared
context, not better**, because shared vocabulary is exactly what a team's context accumulates.
Three outcomes a reader looking for overlap should expect, not just the first:

- **Duplicate** — one incident recorded twice. Keep the older, richer account.
- **Supersession** — a newer account strictly richer than the one it covers. Mark the older
  `superseded`.
- **Both stale** — both out of date, the newer one still the more wrong. Recency arbitrated
  nothing.

Adjudicating overlap is a judgement a reader makes before a memory is written, not a gate the
system enforces: nothing detects a near-duplicate, and the store can hold two accounts of one
incident. Supersession — not deletion — is how the record reconciles them.

## Further reading

- **The context model that memories live in and reach through:**
  [Contexts and Refs](./contexts-and-refs.md).
- **Teams, roles, and what membership grants:**
  [Teams and Roles](./teams-and-roles.md).
- **Using Temper from the CLI:** [temperkb.io/using-temper](https://temperkb.io/using-temper).
- **How a cognitive map grows around reinforced memory:**
  [temperkb.io/cognitive-maps/how-a-map-grows](https://temperkb.io/cognitive-maps/how-a-map-grows).
