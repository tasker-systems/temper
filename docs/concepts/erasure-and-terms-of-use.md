# Erasure and the terms of use

**For users, operators, and integrators.** Anyone who writes into a shared context — or
into someone else's personal context — is agreeing to two lines of terms of use, and any
operator who may one day execute an erasure should know exactly what the code does and does
not reach.

Temper ships no terms of service: it is a substrate, and the organization that adopts it
stands behind its own terms. What this page documents is the behavior the code enforces
today, so those terms can quote it rather than promise past it.

**Erasure is a destructive, compliance-based act — not a normal part of leaving.** In most
deployments the organization that hosts Temper owns the workspace the way it owns its mail
or chat tenancy: a "personal" context is personal to the worker *inside the organization's
estate*, the way a work mailbox is yours as a worker — not your property — and leaving does
not entitle anyone to deletion. Erasure exists for the case where the organization itself
determines erasure is necessary or warranted: a data-protection request it must honor, a
regulator's order. It is destructive and irreversible — content emptied, identity scrubbed —
and it is never self-serve; the door belongs to the organization, not the subject.

## The two lines a contributor crosses

**1. Writing into a team's context gives up any sole claim on deletion.** Content homed in
a team-owned context — resources, edges, properties, uploaded files, data artifacts,
schemas — stays when you leave the team, and stays when an erasure acts on your profile,
whoever authored it. You crossed this line the moment you wrote there; deleting the content
afterwards is the team's act, not yours, and your erasure never performs it for you.

**2. Writing into someone else's personal context declares the content's lifetime.** When
the owner of a personal context grants you authorship into it, everything you write there
lives and dies with that context: if that profile is later erased, your content there is
erased with it. You tied its lifetime to theirs by writing into their context — being its
author does not hold it back. One exception is named where the code's reach is described
below: binary files a guest uploaded there survive the erasure — the act's record names
each one rather than striking it.

**The local-copy remainder.** Neither line reaches a contributor's own machine. If you
synced content to your laptop, that copy is yours and Temper has no enforcement over it —
the same is true of any export from Google Drive or OneDrive. Collecting a departing
person's local copies is an IT/Security process outside Temper, not a Temper control.

## What an erasure wipes — and what it leaves

The act runs against a **subject profile** — the profile an erasure request names —
executed by an instance administrator (system-admin standing) on the organization's
determination. In one server-side transaction it:

- Empties the content of everything homed in the subject's **own personal contexts** —
  text, structured content, embeddings and search data — **whoever authored it**. Content
  a guest wrote there under line 2 dies with the context.
- Scrubs the subject's identity fields: handle, display name, email, linked provider
  identifiers. Attribution to them anywhere else breaks because their profile stops
  resolving to a person (the pseudonym break) — their past contributions to team contexts
  are never edited to hide them.
- **Retires the subject's personal contexts** in the same transaction. This is what makes
  line 2 enforceable rather than promised: every share and grant that reached the estate
  *through the context* dies with the retirement — nobody can read into or write into a
  retired context afterward, guests included.
- Leaves team-owned contexts untouched: the content stays, whoever authored it.

One pre-existing door the retirement deliberately leaves open, stated because erasure
inherits it: whoever **owns a resource** homed in the wiped estate keeps owner standing
over that resource — the same door that lets an owner move work out of a merely-retired
context. What they keep is a husk: the content was emptied before the door could matter,
and what they write there now, nobody else can reach.

## What the code enforces, and what it only reports

The lines above are enforced by the act's scope computation, not by documentation: the
wipe is computed from where content is homed, so shared-context contributions are never in
scope and personal-context content never escapes it, regardless of author. What the act
cannot enforce, it names in its record — the erasure's response and the ledger event carry
per-target outcomes, so an operator reads the remainder at the door and the audit trail
keeps it afterward:

- **Files the subject contributed to team or map homes are reported, not struck.** Each
  one is named in the record with its content hash as held by the team or map that homes
  it. The team keeps the bytes; the record is what proves the erasure was not silent about
  them.
- **Files a guest uploaded into the subject's contexts are named as surviving.** The sweep
  strikes uploaded files by who owns, originates, or uploaded them *as the subject*. A
  file a guest uploaded into the subject's context is the guest's own content, so the act
  leaves it standing — and names it in the record with its blob id and content hash, the
  same reporting a team-held file gets. The record is honest about the two remainders that
  survive with it: an unattached guest file has no custodian once the home's owner is
  erased, so its bytes are retained with no release path and the record says so; a guest
  holding delete standing over an attached file may still strike it after the act.
  Structured-data artifacts a guest's kind owns on those resources are named the same way.

## Where the act lives

The door is the admin API's `POST /api/admin/erasure`, operator-only: a caller without
system-admin standing gets a recorded refusal (answered as a not-found, so the door's
existence discloses nothing), and the refusal is itself part of the audit trail. A retried
request with the same reference completes as a no-op on an already-erased subject.
Re-committing identical bytes after an erasure is an ordinary write — nothing refuses it,
and nothing treats it as a violation by whoever wrote it.

## Further reading

- **The delete act erasure is bounded by — custody, the ledger, and bytes:**
  [Deleting a blob](./blob-delete-and-erasure.md).
- **The departure runbook erasure is deliberately not part of (reversible acts only):**
  [Offboard a Departure](../playbooks/offboard-a-departure.md).
- **What a context is and who can write into one:** [Contexts and Refs](./contexts-and-refs.md).
- **Teams, the role ladder, and what membership grants:** [Teams and roles](./teams-and-roles.md).
