# Adopt operational memory

**For individual users** — someone who wants working knowledge from agent sessions to persist
across conversations, reach their future sessions, and optionally reach a team.

## Outcome

By the end of this playbook you will have configured Temper's memory model on your machine,
captured working knowledge as `memory`-typed resources, recalled it through search and the
rendered index, and — if you choose — shared a context of memories with your team so members
reach them without configuring anything.

## Prerequisites

- **Temper installed** — see [Install Temper](./install-temper.md).
- **Authenticated** — you have signed in and your access is approved. See
  [Authenticate](./authenticate.md).
- **Familiar with contexts and refs** — memories live in and reach through
  [contexts](../concepts/contexts-and-refs.md), and the sharing commands use the ref grammar
  described there.

For the durable explanation of what operational memory is, the CLI memory model, and what
makes a memory worth keeping, see
[Operational Memory](../concepts/operational-memory.md). This playbook is the how-to; that
page is the why.

## Configure the `[memory]` section

Nothing happens until you add a `[memory]` section to your Temper config. The section is
deliberately optional — *not configured* and *configured empty* stay distinguishable, and at
least one context is required, so an empty block is rejected rather than silently doing
nothing.

```toml
[memory]
# Contexts whose memories reach EVERY project on this machine.
shared_contexts = ["@me/working-agreements"]
# Contexts for this project. A list — a project may legitimately span contexts.
project_contexts = ["@me/temper"]
# The rendered index. `emit` refuses to overwrite a file it did not write, so an
# existing hand-written MEMORY.md is safe until it is deliberately moved aside.
index_path = "~/.claude/projects/<project-dir>/memory/MEMORY.md"
```

`shared_contexts` are rendered for every project on the machine; `project_contexts` are
scoped to one. Both lists are read shared-first and deduped, so a context named in both is
fetched and rendered once. `stale_after_days` — how old a `verified` date must be before the
index marks it `UNEXAMINED` — defaults to 90; omit it unless you want a different number.

Check what you have, before and after:

```bash
temper memory status
```

`opted_in: false` is not an error — it is the primary state this command exists to report, and
a supported end state. A machine may measure itself, decide against the whole idea, and never
return. Declining is deleting the section; nothing else changes.

## Capture working knowledge as memories

Operational memory is captured by authoring `memory`-typed resources, the same way you author
any other note. A session note is the raw material; a memory is the durable claim distilled
from it.

### Save the session

Save a session note at the end of work, piping the body via stdin:

```bash
cat <<'EOF' | temper resource create --type session --title "<title>" --context @me/<ctx>
## Goal
What we set out to do

## What Happened
Key actions, decisions, and outcomes

## Decisions
Choices made and why

## Connections
Related tasks, concepts, or contexts touched

## Next Steps
What to pick up next session
EOF
```

Without stdin, `resource create --type session` writes placeholder boilerplate that must be
edited manually. Reference every resource the note names with its full UUID — a prefix is a
timestamp, and the goal and task written a minute later share their first characters.

### Distil a memory from the session

When the session produced a claim worth keeping — a working agreement, a trap that fired, a
constraint the system enforces — author it as a memory:

```bash
cat <<'EOF' | temper resource create --type memory --title "<ascii title>" \
    --context @me/<ctx> \
    --open-meta '{"status":"active","verified":"2026-08-19","descriptor":"one-line recall hook"}'
The body of the memory: the claim, stated so it stands alone outside the session that
produced it.
EOF
```

The open tier carries what the index renders on:

- **`status`** — `active` or `superseded`.
- **`verified`** — the ISO date the claim was last checked against the system. An old date
  means nobody has re-checked it; it does not mean the claim is false.
- **`descriptor`** — the one-line recall hook. A line in the index is a title, not the memory;
  the descriptor is what makes that line worth reading.

Nothing validates these at write time — a malformed one is accepted by the write, reported by
`temper memory status`, and refused by `temper memory emit`. The render is where a bad one
surfaces, deliberately: those are the fields the render's own claims depend on.

### Reinforce a memory that did work

When a memory's situation recurred — or when it *caught* you and a mistake did not get made
because of it — record the date:

```bash
temper resource update <ref> --open-meta-add '{"reinforced":["2026-08-19"]}'
```

Use `--open-meta-add`, never `--open-meta`: the latter **replaces** the list, discarding every
date already stored. The date is bare, and the contract is bare deliberately — no `by` field
(`resource update` already records the acting principal) and no note field (a free-text note
makes every record structurally unique and stops the union collapsing same-day duplicates). If
the catch revealed a shape the memory's body does not describe, amend the body — that is the
act. Amending the body is not re-checking the claim, so it does not advance `verified`.

`reinforced_min` in `[memory]` is the threshold below which a memory collapses into a
per-section tail line in the index. It has no default and is best left out until months of
real reinforcement data exist to choose one from. **Demotion is never deletion**: a collapsed
memory is still in the record, still addressable, still searchable — it has stopped occupying
a line in a file that is loaded every session.

## Search and recall

Recall is ordinary search and ordinary resource reads.

```bash
temper search "<topic>"                                       # semantic + full-text, cloud-side
temper resource list --type memory --context @me/<ctx> --all  # every memory in a context
temper resource show <ref>                                    # the memory itself
```

The rendered index is the projection a client loads each session so a cold start is not a
blank slate. Render and gate it:

```bash
temper memory emit          # render the index from Temper and write it
temper memory check         # exits non-zero when the on-disk index has drifted
```

`emit` refuses to overwrite a file it did not write — a previously-emitted index starts with a
generated-file header, and anything that does not is left alone. The index lives outside any
repo, so nothing in git can diff it; `check` is what a person or a hook runs instead, gating on
the exit code. If you emitted with `--path`, check with the same `--path`, or you are checking
a different file than the one that was written.

## Adopt shared memory across a team

A memory reaches whoever can read the [context](../concepts/contexts-and-refs.md) it lives in
— there is no per-memory reach field, so team adoption is a context question.

### Share a context of memories with your team

Put the shared memories in a context the team can read — either a team-owned context
(`+team-slug/<slug>`) or one you own, shared into the team's read-reach:

```bash
temper context share @<handle>/<slug> +<team-slug>
```

Sharing requires that you administer the context **and** manage the target team (owner or
maintainer), or that you are an instance administrator. **`@me` is not accepted by
`context share`** — use your handle or the context UUID. To find your handle, run
`temper context list` and read the `owner_ref` column. Sharing grants the team **read** access;
the only path to shared authorship is transferring context ownership. For the model, see
[Teams and Roles](../concepts/teams-and-roles.md).

### Have each member name the context

Each member names that shared context in their own `[memory]` section, under `shared_contexts`
if it should reach every project on their machine or `project_contexts` if it is scoped to one:

```toml
[memory]
shared_contexts = ["@<handle>/working-agreements"]
project_contexts = ["@me/temper"]
index_path = "~/.claude/projects/<project-dir>/memory/MEMORY.md"
```

### Members on Desktop, mobile, or the web configure nothing

Members who work from Desktop, mobile, or the web name nothing and configure nothing — they
read and author the same memories as resources. The index is one client's rendering, not the
mechanism, so a member who never adopts the CLI is not a second-class participant. Reach
follows membership and grants, so it changes when the team changes — nothing has to be
re-tagged when someone joins or leaves.

## Further reading

- **What operational memory is, the CLI memory model, and what makes a memory worth keeping:**
  [Operational Memory](../concepts/operational-memory.md).
- **The context model that memories live in and reach through:**
  [Contexts and Refs](../concepts/contexts-and-refs.md).
- **Teams, roles, and what membership grants:**
  [Teams and Roles](../concepts/teams-and-roles.md).
- **Using Temper from the CLI:** [temperkb.io/using-temper](https://temperkb.io/using-temper).
- **How a cognitive map grows around reinforced memory:**
  [temperkb.io/cognitive-maps/how-a-map-grows](https://temperkb.io/cognitive-maps/how-a-map-grows).
