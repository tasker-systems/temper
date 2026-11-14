# Coordinating Through Records — claims, outcomes and wakes

Read this when several agents, from any runtime, share work through Temper records: one claims an
item, does it and writes back, and the others learn of it without a side channel.

## What Temper does here, and what it leaves to you

Temper gives you a few shared tools for this: guarded meta writes, data-artifact shapes, the
context change feed, and authorship on writes. **It adds no coordination mechanics of its own.**
Lifecycles, step state machines, sweepers, deadlines, leases, retries, budgets, late outcomes and
relaunching lost agents are all your workflow's policy. This file shows how to use the shared tools
safely. It holds no opinion on how strict your workflow should be.

| Your workflow's idea | In Temper |
|---|---|
| A participant (agent) | The principal that makes each write: the feed's `actor`. Optionally, a resource that carries a `coordination-agent` artifact |
| A role | The `persona` authorship field (not a team role, which is an authorization rank) |
| A work item (step) | **Any resource.** Temper's only contract on it is the claim keys below |
| A claim | A guarded `open_meta` write on the work item, plus a `coordination-claim` record |
| An outcome | A `coordination-outcome` artifact on the work item |
| A watch | Your own position in the context's change feed. A position is not a record |
| A process or run | Your workflow's own resources. Temper has no shape for them |

**Keep the workflow in its own context**, apart from discovery and research. Work items then never
crowd people's lists and searches. Any doc type can be a work item. If you use tasks, a task still
means what it means to people, so keep your workflow's state in your own `open_meta` keys and leave
`temper-stage` alone.

## The setup act — once per context, safe to re-run

A shape governs only its home, so every context that runs a coordinated workflow declares the
families there itself. Nothing is declared centrally. Run this with authoring authority over the
context.

1. **List what is live:** {% if surface == "cli" %}`temper data-artifact schema list --context <ctx> --format json`{% else %}`list_data_artifact_shapes {"home_type": "context", "home_id": "<ctx>"}`{% endif %}.
2. **For each family you set up, compare two things** between the live shape (matched on
   `artifact_kind`) and the one below:
   - the schema's root `$id`;
   - its `enforcement`.

   The families are `coordination-claim` and `coordination-outcome`, plus `coordination-agent` if
   you opt in.
3. **If both match, declare nothing.** If the family is missing, or either differs, declare it:
   {% if surface == "cli" %}`temper data-artifact schema declare <ctx> --kind <kind> --enforcement <mode> --content @<schema.json>`{% else %}`declare_data_artifact_shape {"home_type": "context", "home_id": "<ctx>", "kind": "<kind>", "schema": {…}, "enforcement": "<mode>"}`{% endif %}.
4. **List again** and confirm that each family's `$id` and enforcement are live.

**Why compare first: declaring is not idempotent.** Every declare, even of an identical schema,
does three things:
- it folds the live shape and mints a new version with a **new `shape_id`**;
- it queues a re-check of the family's existing artifacts, which read `declared_not_yet_checked`
  until the re-check reaches them;
- it breaks any ask that named the old `shape_id` (`shape_superseded`).

Two people running setup at once may both declare. That leaves a duplicate version, which does no
harm.

**Enforcement is your choice, per family.** The defaults are `enforcing` for claim and outcome, and
`advisory` for agent. Choose `advisory` anywhere you would rather record a non-conforming commit
than refuse it.

**A context with no resources in it needs {% if surface == "cli" %}`--kind-owner`{% else %}`kind_owner`{% endif %}.**
A family name is qualified by an owner, which is defaulted from a resource already in the home. In
an empty context the declare is refused ("name kind_owner explicitly"). Name the context's owner:
read it from {% if surface == "cli" %}`temper context show <ctx>`{% else %}`context_read {"view": "get", "id": "<ctx>"}`{% endif %}
(`kb_owner_table`, `kb_owner_id`) and pass {% if surface == "cli" %}`--kind-owner kb_teams:<id>` (or `kb_profiles:<id>`){% else %}`"kind_owner": {"kb_teams": "<id>"}` (or `{"kb_profiles": "<id>"}`){% endif %}.
That is the same owner a commit in that context defaults to, so commits find the shape.

**Upgrading.** A compatible change ships a new `$id` (`…/v2`) whose schema only adds optional
fields. Declaring it folds the old version, older agents' commits still pass, and existing
artifacts are re-checked in the background. A breaking change, such as a new required field or a
narrowed type, gets a new family name (`coordination-claim-2`), never a new version.

## The three shapes

Copy these as they are. They carry no workflow policy: no states, attempt numbers, deadlines or
budgets, and `additionalProperties` stays open, so you can add your own fields. Ids are lowercase
UUIDs. Integers stay within 2^53 − 1.

```json
{"kind": "coordination-claim", "enforcement": "enforcing", "schema": {
  "$id": "temper:coordination/claim/v1",
  "type": "object",
  "required": ["claim_id", "subject"],
  "properties": {
    "claim_id": {"type": "string", "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
                 "description": "The value written to the work item's claim key, and this commit's correlation_id."},
    "subject":  {"type": "string", "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
                 "description": "The work item claimed."},
    "model":    {"type": "string", "maxLength": 256, "description": "Repeated from authorship, so asks can read it."},
    "runtime":  {"type": "string", "maxLength": 64},
    "agent":    {"type": "string", "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
                 "description": "Optional coordination-agent resource id. Every reader of the context sees it."}
  }}}
```

```json
{"kind": "coordination-outcome", "enforcement": "enforcing", "schema": {
  "$id": "temper:coordination/outcome/v1",
  "type": "object",
  "required": ["claim_id", "subject", "status"],
  "properties": {
    "claim_id": {"type": "string", "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"},
    "subject":  {"type": "string", "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"},
    "status":   {"type": "string", "minLength": 1, "maxLength": 64,
                 "examples": ["succeeded", "failed", "canceled"],
                 "description": "Your workflow's own vocabulary; Temper does not close it."},
    "summary":  {"type": "string", "maxLength": 4000},
    "outputs":  {"type": "array", "maxItems": 64, "items": {"type": "string", "maxLength": 512}},
    "model":    {"type": "string", "maxLength": 256},
    "usage":    {"type": "object", "properties": {
                   "input_tokens":  {"type": "integer", "minimum": 0, "maximum": 9007199254740991},
                   "output_tokens": {"type": "integer", "minimum": 0, "maximum": 9007199254740991},
                   "total_tokens":  {"type": "integer", "minimum": 0, "maximum": 9007199254740991}}}
  }}}
```

```json
{"kind": "coordination-agent", "enforcement": "advisory", "schema": {
  "$id": "temper:coordination/agent/v1",
  "type": "object",
  "required": ["principal"],
  "properties": {
    "principal":    {"type": "string", "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
                     "description": "The principal id the change feed names as this agent's actor."},
    "runtime":      {"type": "string", "maxLength": 64},
    "model":        {"type": "string", "maxLength": 256},
    "capabilities": {"type": "array", "maxItems": 32, "items": {"type": "string", "maxLength": 64}}
  }}}
```

- **The claim record** makes claims askable: an ask reads artifact content, never authorship. It
  also outlives the claim key, which a reclaim clears.
- **The outcome record** holds the result, keyed to the claim it was done under.
- **The agent record** is opt-in because it names an identity (see Privacy).

## The patterns

### Wake from the change feed

- **Read from your saved position:** {% if surface == "cli" %}`temper context changes <ctx> --after <through> --kind property_set --kind resource_created --kind data_artifact_committed`{% else %}`context_read {"view": "changes", "context": "<ctx>", "after": "<through>", "kinds": ["property_set", "resource_created", "data_artifact_committed"]}`{% endif %}.
  Start with no `after`: you get no entries and a `through` at the present.
- **Keep paging while `has_more`.** One read scans a bounded slice of the ledger, so a short page,
  even an empty one, with `has_more: true` is not the end.
- **Save the `through` of each page you have handled.** It is the answer's own `through`, never a
  position read after you looked at the subjects. Handling is at-least-once, and every pattern
  below is safe to repeat.
- **An entry is a pointer, never content.** It carries kind, subject, actor, time and `act`. On a
  relevant entry, re-read the subject.
- **Re-query on a timer as well.** The feed is a change hint, not a guarantee, and it never
  announces an admin act. So periodically re-read the work items you hold, and drop any your read
  now refuses. Re-read your candidate work even when nothing woke you.
- **Expect lag.** The feed serves only changes below the cluster's settled horizon, so a long
  transaction anywhere delays wakes. The delay never skips an entry, and the timed re-query
  bounds it.

### Claim with `expect`

**The convention's default claim keys** are two `open_meta` keys on the work item: `claim_id`
(opaque) and `claimed_at` (an ISO-8601 UTC string). Any key works the same way. The names are only
a default, so that participants and operators all know where to look.

1. **Mint a claim value `C`:** a fresh lowercase UUIDv7 for every attempt. Never reuse one.
2. **Write the claim keys, expecting `claim_id` to be absent:**
{%- if surface == "cli" %}

   ```bash
   temper resource meta set <work-item> --managed '{}' \
     --open '{"claim_id": "<C>", "claimed_at": "<now>"}' \
     --expect-absent open:claim_id \
     --correlation <C> --model <model> --persona <role> --confidence probable --format json
   ```
{%- else %}

   ```json
   update_resource_meta {"id": "<work item>", "managed_meta": {},
     "open_meta": {"claim_id": "<C>", "claimed_at": "<now>"},
     "expect": {"open": {"claim_id": null}},
     "correlation_id": "<C>", "model": "<model>", "persona": "<role>", "confidence": "probable"}
   ```
{%- endif %}

   A top-level `null` in `expect` means "absent". You may add your workflow's own expectations to
   the same write, such as its own state key.
3. **Read the answer.**
   - **It landed:** you hold the claim.
   - **It was refused:** nothing was written. Read the refusal before concluding anything (next
     section).
4. **Record the claim.** Commit a `coordination-claim` artifact,
   `{"claim_id": "<C>", "subject": "<work item>", "model": "<model>"}`, with intent `member` and
   `correlation_id` `C`.
   - The record is not what decides the claim; the guarded write is.
   - A commit takes no idempotency key, so a retried commit can leave two records with one
     `claim_id`. Readers dedupe by `claim_id`.

**Why only one claimant wins.** The guard is judged under the work item's writer lock, in the
write's own transaction, so only one write can find `claim_id` absent. A guard checks a value, not a
history. A value that changed and changed back would pass, but a fresh `C` per attempt means that
cannot happen on `claim_id`.

{% if surface == "mcp" -%}
**Spell `expect` exactly.** A misspelled top-level argument (`expects`) is currently ignored
without an error, and the write lands unguarded.

{% endif -%}
**Spell the guarded key exactly too.** A misspelled key inside the expectation reads as absent, so
an absence guard on it always holds.

### Read the refusal

{% if surface == "cli" -%}
A refused write exits 1, and `--format json` prints code `PRECONDITION_FAILED` with each failed key
and its value now in `details.failed`.
{%- else -%}
A refused write is a tool error result (`isError: true`), not a protocol error. Its
`structuredContent` is `{code, message, precondition}`: `code` is `PRECONDITION_FAILED`, and
`precondition.failed` lists `{tier, key, expected, current}` for each expectation that did not
hold. `message` names keys only, never values. Branch on `structuredContent`, never on the text.
{%- endif %}
`current: null` means the key is absent now.

| `current` for `claim_id` | Meaning | Do |
|---|---|---|
| Your own `C` | **You won.** An earlier attempt of yours landed and its answer was lost, so this is the retry meeting your own write | Continue as the holder |
| Any other value | You lost | Write nothing more for this item |

**This check is mandatory.** The MCP relay retries a write once on a server error or a network
failure. So a write that committed can come back refused, with your own value as `current`. A
caller who cannot read the work item never gets this refusal at all: it gets the same not-found
answer a read would give.

The refusal carries only the value stored under each guarded key. It never says who wrote it or
when.

### Echo values exactly

Each side of the comparison is normalized the way Temper stores a value, then compared as JSON
values. So:

- **Echo back the raw value you read**, never a reformatted one. A string is compared byte for
  byte: re-serializing a time through a date library (which can add `.000Z`) makes it a different
  string.
- **Guard on strings, or on integers within ±2^53.** `1` equals `1.0`, but many JSON clients round
  larger integers, so an echo of one can miss. Keep times as strings, never epoch numbers.
- **Do not guard on objects or on `tags`.** Both work: object keys are compared regardless of
  order, and a bare-string `tags` value equals its one-element array. But a guard on these is hard
  to echo exactly, and an opaque string does the same job.

### Heartbeat and reclaim, guarded on the claim value

- **Heartbeat.** Write whatever liveness key your workflow uses (a lease time, say), expecting
  `claim_id` to equal `C`. A guard may name a key the write does not set. Never guard a heartbeat
  on a holder's identity: a holder can be reclaimed and then win again, and a guard checks only the
  value.
- **A refused heartbeat** whose `current` is not your `C` means you no longer hold the claim. Stop
  work.
- **Reclaim.** Whoever your workflow lets reclaim writes `claim_id: null` and `claimed_at: null`,
  expecting `claim_id` to equal the stale value it read. Writing `null` to an open key removes it.
  After a reclaim, the old holder's guarded writes are refused.
- **Before writing an outcome**, a holder can confirm it still holds the claim with one more guarded
  write, such as a final heartbeat. A guarded write must write something: an `expect` on a write
  that sets nothing is refused as malformed. What happens to an outcome written after losing the
  claim is your workflow's rule.
- **The same retry rule applies to all of these.** A retried heartbeat or reclaim can come back
  refused, showing the state its own first attempt produced. Compare `current` with what you meant
  to write before treating the refusal as a loss.

### Write outcomes with correlation and authorship

- **Commit a `coordination-outcome`**, `{"claim_id": "<C>", "subject": "<work item>", "status": "<your vocabulary>", …}`,
  with `correlation_id` `C` and `model`, `persona` and `confidence`.
- **Send `correlation_id` on every coordination write**, not only claims. The feed reports it as
  each entry's `act`. Without it, each write gets its own act id, and one claim's writes cannot be
  joined up again.
- **An enforcing shape refuses a non-conforming commit** and says which field failed, so fix the
  content and commit again. An advisory shape lands it as `declared_not_satisfied`.

### Privacy

- **Use an opaque claim value**, a fresh UUID per attempt, never an agent's or a person's identity.
  Then the work item's state and any refusal tell a reader only that the item is claimed.
- **Never describe a refusal as telling the loser who holds the claim.** It returns the stored
  value, and nothing about who wrote it.
- **The feed still discloses the claimant.** Every reader of the context sees each write's `actor`
  and time, and the authorship fields ride the same writes. An opaque value keeps identity out of
  the item's state and out of refusals. It does **not** hide who claimed from anyone who can read
  the context.
- **If you store an identity anyway**, such as an agent resource in the claim record's `agent` or a
  holder key in meta, every reader of the context sees it.

### Recover a stuck flow

Temper stores no flow state beyond these records, so recovery needs no extra mechanism.

- **Repair** is an ordinary guarded write by someone who can modify the work item. To free a stuck
  claim, write `claim_id: null, claimed_at: null`, expecting `claim_id` to equal the stuck value.
  If it is refused, the flow moved while you looked: re-read and decide again. Send a
  `correlation_id` and a `persona` such as `operator`, so the repair reads as one in the feed.
- **Audit** with what already exists:
  - the context's change feed gives the order, actor and act of every write to the work item;
  - {% if surface == "cli" %}`temper data-artifact list <work-item>`{% else %}`list_data_artifacts`{% endif %} on the work item gives every claim and outcome record, including those of reclaimed attempts.

## What holds, and what does not

- **Holds:** at most one claimant holds an item at a time; a reclaimed holder's guarded writes are
  refused; a retried winner recognizes itself; artifacts are append-only, so no claim or outcome
  record is lost to a concurrent write.
- **Does not hold: a writer that skips `expect`.** An unguarded write can overwrite or clear
  `claim_id`. Guards protect claims from writers that follow this convention, not from those that
  ignore it.
- **A won claim can lack its record**, if the claim landed and the record's commit failed. Only a
  reader of the meta sees it. Noticing that is your workflow's job.
