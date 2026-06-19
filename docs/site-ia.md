# temperkb.io — Public Site Information Architecture

This is the standing information architecture for the public `temperkb.io` site. It
records *what the public surface is, why it is shaped that way, and what work the flip
requires* — the decisions-and-rationale, not the page copy. Page copy is downstream of
this document; each page is its own drafting pass.

**Status:** current. This document governs the public-site IA going forward.

---

## Scar: what this supersedes

This document **supersedes and replaces** `docs/theory-ia-proposal.md`, which was written
*before* the reorientation described below and is wrong on its core premises. That document
has been **removed from the working tree** in the same change that introduced this one; its
history remains reachable in git for anyone who wants the audit trail. It is folded, not
forgotten — preserved in history, absent from default projection — so it cannot be re-read
as if current and cannot clutter grep/agent context later.

What the old proposal got wrong, and why it had to go rather than be amended:

- It asserted the existing product site was **"not being replaced or rewritten,"** and that
  `/`, `/agents`, `/builders`, `/how-it-works` should stay **"exactly as they are… they
  frame Temper-as-product."** The flip reverses exactly this: the product framing is demoted
  to *one projection* over the substrate, not the headline.
- It placed `/theory` **"alongside"** the product site, explicitly *not competing with `/`
  for general traffic* — a peer tier bolted on. The flip makes the theory/substrate frame
  the *foundation the site rests on*, not a side annex.
- It specified `/docs` → `/using-temper` as a rename and recorded it as resolved — but the
  rename was **never executed** (the route is still `/docs`; no redirect exists). The flip is
  the moment to actually do it.

Because the old document's premises are inverted rather than merely incomplete, amending it
in place would have left a confusing half-corrected artifact. The honest move is supersession
with this scar carried forward here, where it will actually be read.

---

## The flip, stated precisely

Temper's public site was built in the **workflow-tool** frame: the *vault* is the product,
the organizing verb is *remember*, the reader is a solo builder wiring an agent into their
own knowledge base. Three strata accreted at three moments and no longer agree with each
other: the product surface (oldest), a shipped-but-unlinked `/theory` tier, and a
shipped-but-unlinked `/cognitive-maps` set (newest, already speaking the reoriented
language).

The flip makes the reoriented frame the trunk:

> Temper is an event-sourced coordination substrate whose organizing purpose is to be
> economical with attention. A cognitive map is a telos-seeded region of that substrate
> where humans and agents grow a shared, situated understanding together — and everything
> else is a projection over it.

Personal knowledge management does not disappear. It becomes **one projection over the
substrate** — a valid, useful view, no longer the whole story. This is the site expressing
the same commitment the confidence inventory states plainly: *Temper is a coordination
substrate, and personal knowledge management is one view over it — not the other way round.*

---

## The model: four trunks, three front doors

The flip is expressed as IA by separating three dials that the naive version conflates:
**navigation surface**, **URL hierarchy**, and **entry-point status**. They are independent.
A page can be a top-level URL without being a front door; a page can be reached often without
sitting in the primary nav.

```
/                      Router landing. Narrow hero, routes deeper. NOT "the product."
                       Offers THREE front doors a newcomer actually picks:

   → /cognitive-maps   The WHAT. Concrete, the on-ramp. Telos-seeded regions shown
                       proven-in-the-schema. The graph-walk "start here" surface.

   → /operating        The RUNNING-IT. For the cold enterprise evaluator: what the
                       architecture fixes vs. what a deployment shapes. (Promoted to
                       top-level — see below.)

   → using Temper      The PKM PROJECTION. The reframed /builders + /agents — "this is
                       one view over the substrate, the personal-knowledge view."

/theory                A fourth top-level URL, but NOT a front door. The WHY — attention as
                       teleological anchor, knowledge-as-relationship, the commitments the
                       whole system answers to. Reached by ascent from the concrete (from
                       within cognitive-maps, from operating's back-links, from the README),
                       by a reader who has seen the concrete thing and now wants the why.
                       It is the foundation all three doors rest on, surfaced on demand —
                       not a door itself.

/using-temper          The CLI/sync/MCP reference. The /docs rename, finally executed.
```

### Why theory is a flat top-level URL and not `/cognitive-maps/theory`

Theory is not *smaller* than cognitive-maps — it is *more general*. A cognitive map is one
place the theory cashes out (the richest, most grounding one); `/operating` is another; the
PKM projection is a third. Nesting the foundation *under* one of the things built on it would
invert the architecture and orphan the others (operating rests on the same theory; under a
nested scheme it would have to deep-link sideways into a sibling's children).

The instinct that prompted the question was correct in its *direction of illumination*:
theory is elucidated and grounded *by* cognitive-maps, not the reverse — theory-read-cold is
abstract, theory-read-after-seeing-the-onboarding-cogmap-work is concrete. But "X is best
understood after Y" is a claim about **reading order and cross-linking**, not **hierarchy**.
The fix for "theory shouldn't be a cold entry point" is to demote it *in navigation* and
route to it *by ascent* — not to move its URL. So: theory comes off the primary nav as a peer
front-door; it stays at `/theory/*` (no redirects, no move); the cognitive-maps graph-walk
leads up into a theory cluster as a *destination arrived at*.

This also honors the cognitive-maps set's own discipline — cross-references by concept, never
by ordinal or hierarchy, *because web pages have no inherent order and a reader can arrive
anywhere*. Encoding "theory is part of cognitive-maps" in the URL path would bake a reading
sequence into the structure, exactly the ordering that discipline warns against.

(The one frame in which nesting *would* be right: if cognitive-maps were declared Temper's
single headline and theory/operating/PKM were all framed as aspects of understanding-and-
running-cognitive-maps. That is a *different flip* — it makes cognitive-maps the trunk rather
than the substrate-and-its-regions. The chosen flip points away from it: the substrate is the
trunk, a map is one telos-seeded region of it, operating is its own promoted top-level.)

---

## `/operating` — the promotion, and the seam it crosses

### Audience: the cold enterprise evaluator

`/operating` is for someone arriving from *outside* to answer "what do I run, what do I get
out of the box, what's still mine to decide." They did **not** read the cognitive-maps
conceptual walkthrough and may never. This rules out leaving the operating content where it
was composed — as movement 7 of the cognitive-maps set, which assumes the reader has met the
seed cast (alice, bob, the onboarding-cogmap) across movements 1–6 and opens on "the very
first thing that exists is the seed file" trusting that grounding.

### The structure

- **`/operating` is a top-level hub** with the same four children the cognitive-maps
  operating set already has: `deployment`, `governance`, `observability`, `insights`. Those
  four child pages are well-built and need little content change; it is the *seam* — how
  `/operating` relates to its origin under cognitive-maps — that needs care.
- **A short orienting top** establishes the three-tier confidence ledger (below) and the
  "0→1 is invariant, everything after is shaped" framing, introducing just enough vocabulary
  to **stand alone** and linking *back* into `/cognitive-maps` and `/theory` where terms
  (telos, cogmap, event-sourced substrate) need their grounding — rather than restating
  movements 1–6.
- **`self-hosting.md`** (the existing operator runbook — Vercel project, Neon, Auth0 tenant)
  is the concrete floor beneath `/operating/deployment`: the "if you're self-hosting on
  Vercel, here's exactly what to do" detail the conceptual deployment page points down into.

### The seam: cognitive-maps movement 7 becomes a bridge

The cognitive-maps set has a deliberate genre split — **pages 1–6 *show* from the schema
outward** (the visuals are evidence; "here is a thing whose shape is proven by the data
model"), and **page 7 *invites* from operations inward** ("this has to be run somewhere, that's
a good problem"). Moving the operating content out must not cost the journeyer their ending.

So movement 7 **shrinks to a bridge**: it keeps the turn-outward beat that closes the
conceptual arc ("the map had to be stood up somewhere — a good problem, walked through under
[operating]") and hands across to `/operating` for the detail, rather than carrying the full
operating content. The 1–6-show / 7-invites split survives; movement 7 still *invites*, it
just invites *toward `/operating`* now. The journeyer gets their exhale; the evaluator gets a
real home.

---

## The three-tier confidence ledger

This is the spine of `/operating` and the register-setter for the whole flip. The evaluator
is best served by a page that keeps three things visibly separate rather than blurring them
into a flat "it's all handled" — and keeping them honest is what makes the page *trustworthy*.

The grounding for the middle tier is the invocation-envelope + neutral-contract work
(`kb_invocations`, the `temper-agents` crate, `DeploymentProfile` as runtime × residency).

1. **Fixed by the architecture** — invariant across every deployment, proven in the artifact
   and replay-verified. The event-primary ledger; the convention-agnostic kernel; teams-RBAC
   over homed boundaries; actors-as-entities; administration-is-event-sourced; the invocation
   envelope (accountability-grain run, telos/scope binding, terminal outcome); the delegation
   launch-gate; authorship-invisible-to-affinity. The strongest tier: *proven, not promised.*

2. **Extensible by design — and the edges are still being found.** The runtime choice is
   localized to a thin contract the substrate never reads (the `temper-agents` crate depends
   on the substrate, never the reverse; the kernel never branches on runtime). The *shape* of
   the delegation problem is the same across the platforms modeled so far (Vercel Eve and
   Claude Managed Agents are both first-class `RuntimeBinding`s; runtime and residency are
   orthogonal axes). **This is stated as extensible-shape with partial neutrality, not flat
   "runtime-neutral."** Concretely: adding a runtime today is a *patch to the contract crate*
   (the targets are enum variants, not configuration) — a deliberate too-early-abstraction-
   avoidance tradeoff — and *what becomes pure configuration vs. what the substrate must model
   is being determined by real deployment.* The seam is stated **as** a seam, and it is a live
   research front, not a permanent limitation: deploying on real runtimes is expected to move
   the config-vs-substrate-knows line, and the line will move as we learn. (This is also the
   forward-exciting close the `insights` page reaches for — the honest seam and the
   look-what-becomes-possible ending are the same sentence.)

3. **Genuinely yours / open** — the deployment chooses, and the page does not pretend
   otherwise. Which runtime; residency; token budget; tenancy model; per-tenant integration;
   the trigger cadence that wakes an agent; observability scope; how guarded the admin surface
   must be.

**The risk to guard against** (named so the drafting doesn't drift into it): the temptation,
now that there are clean plans and a soon-to-exist POC, to let *tier 2* masquerade as *tier
1* — "what we run" presented as "what Temper is." The evaluator is better served by "this is
invariant; this is what we happen to run and you can swap it; this is genuinely yours" than by
a blurred claim. Keep the three columns honest.

**A timing note for drafting:** the POC deployment status (Eve, possibly CMA) is expected to
change within days. Write the prose at the level of the *finding and the contract* (which are
stable), and let "running on Eve" become a concrete reference point added once true — the way
temperkb.io anchors the deployment-shape range. The architecture claim does not depend on the
POC; the POC is evidence for it.

---

## What changes, by surface

### Correct already — connect, don't rewrite

- All `/theory/*` pages. Strong as-is.
- The entire `/cognitive-maps/*` set, **except** movement 7 (which becomes the bridge above)
  and the index (which needs its "start here" graph-walk updated to reflect the new IA and to
  include the theory cluster as a destination).
- `docs/guides/self-hosting.md` — accurate operator runbook; becomes the floor beneath
  `/operating/deployment`.

### Out of date — content (says the wrong thing now)

- **Landing `/`** — body claims the vault is the product (workflow strip, doc-type cards,
  "Temper Cloud / your vault everywhere"). The hero ("Clarify your intention") largely
  survives; the *body* is what asserts vault-as-product and gets replaced with the router.
- **README.md** — "A knowledge base for builders," context-rot opener, throughline/goals/
  tasks framing, `/builders` + `/agents` as primary entry points. Also has *intra-frame*
  drift: its Quick Start already shows the cloud-first `temper pull` / `temper resource
  create --from` flow while `/builders` and `/docs` still show the old local `temper add` /
  `temper init` flow — so the README and the product pages don't even agree today.
- **Nav + Footer** — link only How-it-works / Builders / Agents / Docs. `/theory`,
  `/cognitive-maps`, `/operating` are invisible. Highest-leverage mechanical fix: the trunks
  that *are* the story can't currently be reached.
- **`/builders`, `/agents`** — wholly in the old frame; to be reframed as the PKM projection
  (see below).
- **`/docs`** — mostly accurate as CLI reference, but teaches `temper skill install` as the
  agent path, which the roadmap replaces with `temper plugin install`. Predates the substrate
  vocabulary entirely.

### The reframes

- **`/builders` + `/agents` → the PKM projection.** Surgical, not a rewrite: each gets a
  *frame-setting top* declaring "this is one view over the substrate — the personal-knowledge
  view" and linking up to the trunk. The bodies (warmup/save loop, doc types, MCP pathways)
  stay largely intact — as a *description of the PKM projection* they remain true; what
  changes is the claim of primacy. The cross-sell footers get rewritten to point at the
  substrate, not at each other. **Coupling to flag:** `/agents` teaches `temper skill
  install`; the roadmap moves to `temper plugin install`. The reframe is the natural moment
  to fix this, but it couples to the plugin-system work — decide fix-now vs. scar-and-defer at
  drafting time.

- **`/docs` → `/using-temper`** — the specced-but-never-done rename, plus a `/docs` →
  `/using-temper` redirect (single canonical URL), internal-link updates, and a minimal title/
  lede edit. A short header sentence points to `/theory` for the conceptual frame; the page
  itself stays operational.

---

## Work map (dependency-ordered)

The umbrella task tracks these phases. Items 1–3 are unambiguous and fast; 4–7 carry the real
writing and the judgment calls. The two **seam-sensitive** items must not be batched with the
mechanical work — they need the cognitive-maps register and a careful review pass, because
they are where a careless change degrades something already good.

0. **Supersede + remove `theory-ia-proposal.md`.** Scar carried forward in this document
   (above); the file is removed from the working tree (`git rm`), history preserved. *Item
   zero — it actively contradicts every decision here.*
1. **Nav + Footer.** Add Theory (as a non-front-door link) / Cognitive-maps / Operating;
   present Builders + Agents under a "Using Temper" grouping. Mechanical, highest leverage —
   makes the correct-but-invisible trunks reachable.
2. **`/docs` → `/using-temper` + redirect.** Small; the flip is the moment.
3. **Landing body → router.** Keep/lightly-tune the hero; replace the body with the
   three-door router. (Depends on the doors existing — 1–2.)
4. **Promote `/operating` to top-level.** Hub + four children, re-fronted for the evaluator,
   re-registered against the three-tier ledger; `deployment` upgraded with the
   extensible-shape/partial-neutrality framing; `self-hosting.md` as the floor.
   **★ seam-sensitive.**
5. **Cognitive-maps movement 7 → bridge** (+ index "start here" graph-walk update to reflect
   the new IA and include the theory cluster). **★ seam-sensitive.**
6. **Reframe `/builders` + `/agents`** as the PKM projection. Most copy; couples to the
   plugin-vs-skill question.
7. **README** — rewrite substrate-first; resolve the intra-frame `temper add` vs. `temper
   pull` drift while doing so.

---

## Provenance

- **Conceptual ground:** `working-context-semantic-model.md`, `attention-manifesto.md`,
  `working-context-framing-schema.md`, `feature-development-and-coordination-substrate.md`,
  `temper-confidence-inventory.md` (the attention thesis; substrate-as-trunk; PKM-as-
  projection; translation-is-irreducible).
- **The cognitive-maps set & its register/discipline:** `docs/cognitive-maps/` (the genre
  split, the threaded seed, cross-reference-by-concept, partner-not-lecturer voice).
- **The middle-tier grounding (extensibility, the neutral contract):**
  `docs/superpowers/plans/2026-06-18-invocation-envelope-and-authorship-metadata.md` and
  `docs/superpowers/plans/2026-06-18-temper-agents-neutral-contract-crate.md`; the Eve/CMA
  comparison research under `docs/research/`.
- **Operator runbook (the deployment floor):** `docs/guides/self-hosting.md`.
- **Superseded:** `docs/theory-ia-proposal.md` (removed; see the scar above).

---

## Deliberate non-goals (this document)

- Does **not** write the page copy. Each page is its own drafting pass; this is decisions-and-
  rationale only.
- Does **not** specify Svelte component structure or routing internals — those follow from the
  IA, not the reverse.
- Does **not** propose visual-design changes. The site has an established register (palette,
  type, layout); the new frame inhabits it without redesign.
- Does **not** relitigate `/theory` or `/cognitive-maps` content. The flip is about *situating
  and connecting* them, not rewriting them.
