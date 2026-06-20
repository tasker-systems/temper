# Public-site tone & framing audit (2026-06-20)

A telos-charter sweep of the `temperkb.io` public pages (`packages/temper-ui/src/routes/(public)/`).
For each page we name **who we're speaking to**, **the tone we mean to hold**, **what the page is
for**, and **whether its sections earn that intention** — then make subjective-but-rigorous calls
about what's coherent, what wants its own home, and where the copy speaks from *inside* a
conversation the reader was never part of.

This document is downstream of [`docs/site-ia.md`](./site-ia.md), which sets the standing
information architecture (the four trunks, three front doors, the `/operating` promotion). The IA
doc deliberately stops at decisions-and-rationale — *"each page is its own drafting pass."* This
audit is the per-page drafting-conscience layer the IA doc left open: persona, tone, and
section-level stewardship.

---

## 1. The site runs in two registers

Every public page sits in one of two tonal coordinate systems. Naming them is what makes "drift"
legible — a page drifts when it's *filed* in one register but *written* in the other.

| Register | Nav home | Audience posture | Voice | Pages |
|---|---|---|---|---|
| **Concepts** (the *why / behind-the-scenes*) | "Concepts" menu | "I want to understand the purpose and commitments under this" | First-person, essayistic, commitment-driven, *held lightly*. Partner, not lecturer. | `/manifesto`, `/theory` (+children), `/cognitive-maps` (the set) |
| **Using Temper** (the *how do I use / run it*) | "Using Temper" menu | "I showed up to do something — build, integrate, run, look up a command" | Second-person, concrete, oriented to the reader's task. Honest, but starting from *their* question. | `/builders`, `/agents`, `/operating`, `/using-temper` (Reference) |

The landing page (`/`) is the router that hands a newcomer into one of these. Its three doors —
*What it is* → cognitive maps, *How to use it* → builders/agents/operating/reference, *Get started*
→ login — are well-shaped and register-honest.

The user's instinct is exactly right: **`/theory` and `/cognitive-maps` land because their Concepts
voice matches a Concepts audience.** The drift is on the Using-Temper side, concentrated entirely in
`/operating`, which is *filed* under "how do I run this" but *written* in the Concepts register —
and worse, in a specific sub-dialect of it (cognitive-maps **movement 7**) that assumes the reader
arrived mid-journey.

---

## 2. Per-page charter

Verdicts: **HOLDS** (coherent, sized right, register-true) · **DRIFTS** (good material, wrong
posture for its audience) · **WATCH** (holds today, one seam worth a light touch).

### Concepts register

**`/manifesto` — An Attention Manifesto** · *HOLDS*
- **Persona:** a reflective adopter deciding whether to trust the author's values before spending
  attention on the tool.
- **Tone:** first-person, ethical, intimate. The most personal page on the site, by design.
- **Purpose:** state the thesis — attention is the medium of agency; efficiency is an *obligation*,
  not a productivity virtue — and the four commitments that fall out.
- **Sections earn it?** Yes. It is explicitly written to be read cold: *"anyone considering this
  tool deserves to know what its author thinks the tool is for before they decide whether to spend
  their attention on it."* That sentence is the gold standard for the whole site — it names the
  reader's actual situation instead of presuming a prior conversation.

**`/theory` (hub + 8 children)** · *HOLDS*
- **Persona:** someone who has seen the concrete thing and now wants the semantic model and the
  commitments it answers to. Reached *by ascent*, never as a cold door (IA doc, §"Why theory is a
  flat top-level URL").
- **Tone:** essayistic, first-person commitments, provisional — *"The model is held lightly; the
  commitment underneath it is not."*
- **Purpose:** the semantic model the attention commitment requires (ontology → manifold → time →
  deformation → perspectives → translation), plus schema/open-questions reference surfaces.
- **Sections earn it?** Yes. The IA doc rates the whole tier *"strong as-is."* No change.

**`/cognitive-maps` (hub + the set)** · *HOLDS*
- **Persona:** the newcomer on-ramp — the "what it is" front door. The most concrete entry into the
  reoriented frame.
- **Tone:** *show, don't tell* — schema-grounded, evidence-first, partner-not-lecturer.
- **Purpose:** the one-sentence thesis (telos-seeded region of an event-sourced substrate), worked
  outward from one running example.
- **Sections earn it?** Yes. The hub is tight (thesis + graph). The 1–6-*show* / 7-*invites* genre
  split is deliberate, and movement 7 has correctly shrunk to a **bridge** that hands the reader
  across to `/operating`. This is the part of the system working as designed.

### Using-Temper register

**`/builders`** · *HOLDS* (light WATCH)
- **Persona:** a solo developer / small team feeling context-rot across agent sessions.
- **Tone:** practical, second-person, concrete CLI. Sells the throughline.
- **Purpose:** present the **personal-knowledge projection** — the warmup/work/save loop, doc
  types, semantic search, the markdown vault.
- **Sections earn it?** Yes. The `projection-frame` aside (*"you're looking at one projection over
  the substrate… for what it's a view of, see cognitive maps"*) is a clean *outward* bridge — it
  invites, it doesn't presume. WATCH only: the page is long; every section pulls weight, so this is
  a note, not a finding.

**`/agents`** · *HOLDS*
- **Persona:** someone wiring an agent (Claude Code / MCP / skill) into a persistent context layer.
- **Tone:** practical, three-pathways, concrete transcripts.
- **Purpose:** the agent's view of the PKM projection — CLI, MCP server, generated skill.
- **Sections earn it?** Yes. Same clean projection-frame bridge as `/builders`. No drift.

**`/using-temper` (Reference)** · *HOLDS*
- **Persona:** an operator at the keyboard who wants the command, the flag, the config path.
- **Tone:** dry reference — and rightly so.
- **Purpose:** install, getting-started, resources, search, relationships, cloud, agents, config.
- **Sections earn it?** Yes. Appropriately sized; a one-line lede points up to the conceptual frame
  for anyone who wandered in wanting the *why*. No change.

**`/operating` (hub + deployment / governance / observability / insights)** · *DRIFTS* — **the
finding of this audit.** Detailed below.

---

## 3. The central finding: `/operating` drifted from its own charter

This is not a case of a missing or wrong charter. `docs/site-ia.md` already states the `/operating`
charter precisely, and it is the right one:

> **Audience: the cold enterprise evaluator.** `/operating` is for someone arriving from *outside*
> to answer "what do I run, what do I get out of the box, what's still mine to decide." They did
> **not** read the cognitive-maps conceptual walkthrough and may never. This rules out leaving the
> operating content where it was composed — as movement 7 of the cognitive-maps set, which assumes
> the reader has met the seed cast … and opens on "the very first thing that exists is the seed
> file" trusting that grounding.

The **structure** honors that charter: `/operating` got its own top-level hub and URL, its own
three-tier confidence ledger spine, prev/next within its own tier, and it links *back* to
`/cognitive-maps` and `/theory` for grounding rather than restating movements 1–6. The seam was
crossed.

The **voice** did not cross with it. The drafted copy still speaks as movement 7 — it opens on
posed-question epigraphs, drops the reader mid-scenario with an un-introduced cast, and refers to
its own place in "the set." The re-homing was structural; the re-fronting the IA asked for
(*"introducing just enough vocabulary to stand alone"*) was only partly executed.

A small corroborating tell: the cast isn't even stable across the surface — `site-ia.md` calls them
*"alice, bob,"* the bridge and governance pages call them *"Dave"* and *"Carol."* When a reader is
assumed to already know a cast, the names can drift unnoticed; that fragility is itself the argument
against leaning on them cold.

The user named the exact symptom: the hub's *"The honest way to answer 'is it all handled?'"* starts
from a rhetorical posture that assumes the reader showed up with a posed question, when they very
likely showed up to learn *"how do I spin this up."*

### 3a. The receipts — insider-framing, page by page

**Hub (`/operating/+page.svelte`)**
- Epigraph: *"You came to answer a practical question…"* — tells the reader why they came. Presumes
  the conversation.
- *"The honest way to answer 'is it all handled?' is to refuse the question's framing."* — answers a
  question no cold arrival posed. (The user's flagged line.)
- Section **"Keeping the columns honest"** — *"now that there are clean plans and a POC within reach,
  it would be easy to let tier 2 pass for tier 1…"* This is the **project talking to itself**. It is
  lifted almost verbatim from `site-ia.md`'s *"The risk to guard against (named so the drafting
  doesn't drift into it)."* That is drafting guidance for *us*; it leaked into reader-facing copy.

**`/operating/deployment`**
- *"visible already in the cast"*, *"exactly how the onboarding map was born"*, *"the seed you've
  been reading"* — all assume the reader came through movements 1–6.
- *"the temper-system dreaming we keep naming"* — *"we keep naming"* is an in-group marker.

**`/operating/governance-and-administration`**
- Epigraph: *"Dave is a maintainer of org-common. Carol owns directors."* — a cast dropped on the
  reader with no introduction.
- *"Look closely at what it took to set **the cast** up"* — names "the cast" as a known object.

**`/operating/observability-and-audit`**
- Epigraph: *"The onboarding agent woke, read a charter, and wrote a regulation."* — assumes the
  onboarding scenario is already in hand.

**`/operating/insights`**
- Epigraph: *"A PR merged on team-a. Minutes later, the onboarding map had a new piece of
  regulation."* — same assumed scenario.
- *"the forward-exciting close to the set"*, *"Why this is the closing note,"* *"This is where
  running Temper stops being a cost…"* — **structural-insider** language: it positions the page as
  the finale of a *set* the cold arrival doesn't know they're reading.

What is **not** the problem, and must be preserved: the three-tier ledger itself, the four-dimension
split, the runtime-seam honesty (*"extensible shape with partial neutrality, not flat
runtime-neutral"*), every `VizFigure` and its `honestBasis` schema grounding. These are the page's
strength and they serve the cold evaluator *well*. The fault is entirely in the **framing layer** —
epigraphs, openers, structural asides — not the substance.

---

## 4. Determinations

### 4a. Coherent, appropriately sized — keep as-is
`/manifesto`, `/theory` (+children), `/cognitive-maps` (hub + set incl. the movement-7 bridge),
`/builders`, `/agents`, `/using-temper`, `/` (landing). No content change indicated.

### 4b. Effective/necessary but mis-homed — move or re-seat
- **The running example (the "cast").** It currently has *no home on `/operating`* — it's borrowed
  from cognitive-maps, where it was introduced. Determination: `/operating` must either **de-cast**
  its scenarios to role-based actors (*"a maintainer," "a triage agent," "an external system"*) or
  **introduce its own compact example** that stands alone. The IA's stated intent ("stand alone…
  rather than restating movements 1–6") leans de-cast, and the pages already cite the real schema
  tables (`cogmap_genesis`, `kb_events`, `emitter_entity_id`, …), so concreteness survives without
  the named characters. *(This is the one genuine authorial fork — see §6.)*
- **"Keeping the columns honest" (hub section).** Determination: this is drafting-conscience meta
  that belongs in `site-ia.md` / our notes, not on the page. The *result* it argues for (honest tier
  labeling) is already delivered by the three tiers themselves. **Cut the confession; keep the
  labeling.** Migrate its intent back to the IA doc, where it already lives.

### 4c. Insider tone/framing to fix — the sweep
Everything catalogued in §3a. The shape of the fix is consistent across all five pages: **open each
page from the reader's actual arriving question, not from a posed question or a mid-scenario beat,
and strip the structural-set language.** Concretely:

1. **Rewrite the five epigraphs** (hub + 4 children) to start where the cold evaluator starts —
   "here's what you run and what stays yours" — instead of telling them what they came asking or
   dropping them into an un-introduced scene.
2. **De-cast or introduce-the-cast** (per §6 fork) so no scenario assumes prior acquaintance.
3. **Remove structural-insider phrases:** "the set," "the/this closing note," "the forward-exciting
   close to the set," "we keep naming," "the seed you've been reading," "visible already in the
   cast," "Look closely at what it took to set the cast up," and the *"is it all handled?"* opener.
4. **Cut/trim "Keeping the columns honest"** to at most a single honest-labeling sentence, or remove
   it (the tiers already do its work).
5. **Preserve** the three tiers, the four-dimension split, the runtime-seam paragraph, and all
   diagrams + `honestBasis` notes verbatim.

The essayistic warmth the site is known for is **not** the target. The target is the *presumption of
a prior conversation*. A page can be thoughtful, honest, and beautifully written while still opening
from the reader's situation rather than ours — `/manifesto` proves it.

---

## 5. Priority & sequencing
1. **`/operating` hub** — epigraph + "is it all handled?" opener + "Keeping the columns honest."
   Highest visibility (it's a landing-page door and a nav item).
2. **The four children** — epigraphs + structural-set language + cast decision, applied uniformly so
   the tier reads as one voice.
3. *(Optional, later)* a light WATCH pass on `/builders` length — not a defect, a trim if desired.

Everything here is **framing-layer** editing on `packages/temper-ui/src/routes/(public)/operating/`.
No IA change, no routing change, no diagram change. It is the completion of work-map item 4 in
`site-ia.md` ("re-fronted for the evaluator"), not a new direction.

---

## 6. Open authorial decisions (forks for the steward)
- **Cast approach:** de-cast `/operating` to role-based actors *(recommended — matches the
  stand-alone charter, keeps concreteness via the real schema)*, **or** give `/operating` its own
  one-paragraph introduction of the running example so the scenarios keep their narrative warmth.
- **Sweep scope:** `/operating` only *(recommended — it's the sole drifter)*, or also take the light
  `/builders` trim while we're in here.

---

## 7. Optional: mirror this as a cognitive map in temperkb.io
The framing the steward proposed — *pages-as-cognitive-map, each region with its own telos-charter
and stewardship* — maps cleanly onto this audit: each page becomes a resource whose managed meta
carries `persona`, `register`, `tone`, `purpose`, and a `verdict`, with edges to the trunk it rests
on (`/operating` → `/cognitive-maps`, `/theory`). That would make the charter *live* — drift becomes
queryable, and a future drafting pass can be checked against the page's own stated telos. Proposed,
not done: standing up resources writes to the live KB, so it waits on the steward's go-ahead.
