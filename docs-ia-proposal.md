# Docs IA Proposal — temperkb.io

A first-pass information architecture for replacing the current "get started" framing of temperkb.io with the conceptual-first frame established by the three source documents (manifesto, semantic model, framing schema).

This is a proposal, not a decision. Section trees are annotated with rationale and with the questions they're waiting on.

---

## Constraints the IA is trying to honor

Pulled directly from the three source documents so the rationale below is checkable against them:

- **Attention is the teleological anchor.** A docs surface is itself a demand on attention. The IA should be navigable cold, surface what matters first, and not require the reader to construct a model from scratch.
- **Translation is irreducible.** The IA cannot produce a "neutral" or "canonical" simplification of the model. It can introduce the model progressively, but it must not pretend a simpler version is the real version.
- **Don't operationalize what's deliberately open.** The schema's "Intentionally Open" list is part of the model's discipline. The IA must not flatten it into "coming soon" or "roadmap" framing.
- **Don't smuggle product claims.** Pages should commit only to what the substrate-level model commits to. Where Temper-as-system instantiates a model commitment, the page should be explicit about which it is naming.
- **Efficiency-as-ethic.** Pages have a length budget. A page that takes 20 minutes where 5 would do has cost the reader 15 minutes they don't get back. The IA should be navigable by short hops, not long scrolls.

---

## Proposed tree

```
/                           What Temper is for                    (manifesto layer)
/theory                     Working context: a model              (model overview + anchor)
/theory/ontology              Data, intention, information, knowledge
/theory/manifold              Positions, fields, streams
/theory/time                  Time, events, derivation
/theory/deformation           Forming, forgetting, scarification
/theory/perspectives          Who is asking
/theory/translation           Irreducibility, bridges, observer-relativity
/theory/schema                Schema reference (work in progress) (framing layer)
/theory/open-questions        What is not settled
/using-temper               How to use Temper                     (existing /docs content, rehomed)
```

Three tiers (manifesto, theory, operational) under one root. Everything new lives under `/theory`; the operational layer keeps the existing CLI/sync/MCP reference at a semantically meaningful URL.

Existing top-level pages — `/agents`, `/builders`, `/how-it-works` — are retained as is. The only change to the existing surface in this pass is `/docs` → `/using-temper`. The existing `/docs` route — semantically nondescript — retires; its content rehomes at `/using-temper` and every current "getting started" link resolves there.

---

## Tier 1 — `/` What Temper is for (manifesto layer)

**Source:** *An Attention Manifesto*

**Replaces:** the current `(public)/+page.svelte`, which leads with "knowledge work deserves structure" and walks through CLI verbs.

**What it is:** a single page that establishes attention as the teleological anchor and names the four commitments that follow from taking that seriously.

**What it deliberately is not:**
- A feature tour. The manifesto explicitly rejects framing the commitments as "features I want to ship."
- A landing page in the conventional sense. It does not ask the reader to install or sign up.
- A summary of the model. The model lives at `/theory`; this page points to it without summarizing it.

**Rationale:** The manifesto is the only one of the three documents that names *why the rest exists*. A visitor who reads only this page should leave knowing what Temper takes a stand on, without having absorbed any of the model's machinery. A visitor who wants the machinery follows the link to `/theory`.

The first-draft copy for this page is in `landing-draft.md`.

---

## Tier 2 — `/theory` Working context: a model (semantic-model layer)

**Source:** *A Semantic Model of Working Context*

**What it is:** a six-page conceptual progression that introduces the model's primitives in the order the source document does. Each page is short enough to read in one sitting; the set is navigable as a sequence or as standalone pages.

**Section index:**

- `/theory` — Overview and the teleological anchor. Restates what the model is and what it is for. Single short page. Approximately the first 300 words of the source document, edited for a reader who has not read the manifesto separately.

- `/theory/ontology` — Data, intention, information, knowledge. The stratified ontology. Names the *knowledge-bases-are-misnomers* commitment explicitly — this is one of the model's load-bearing claims and deserves a page.

- `/theory/manifold` — Positions, fields, streams. The geometry. Includes the bidirectional coupling claim ("attention is force, force deforms"), which is the model's most consequential commitment and is easy to mishandle if buried.

- `/theory/time` — Time as primary axis. Events-as-primary. Why this is a substrate commitment, not an implementation detail. This page is where the model justifies append-only ledger semantics without yet naming any system that implements them.

- `/theory/deformation` — Forming and forgetting. Strong vs. weak deformations, the recording threshold, decay/deformation/folding as three distinct forgetting mechanics, scarification as a property certain deformations carry, self-cohesion and background relaxation as the two primitives beneath everything else.

- `/theory/perspectives` — Who is asking. Perspectives as trajectories not points; role-perspective vs. individual-perspective; access vs. expertise; weak observer-relativity.

- `/theory/translation` — Translation is irreducible. Bridges, scars at translation points, knowledge as relationship. This page is the natural close of the model tier because it is where the model commits to *what the system will not do* — produce a canonical, neutral view.

**Rationale for splitting into six pages rather than one:** The source document is ~3000 words and rewards re-reading. Six pages let a reader stop at the level they need (someone interested in the forgetting mechanics doesn't need to scroll past perspectives), and let later pages link in at the right level of detail. The split follows the source document's own section breaks, so no editorial reshaping is needed at this stage.

**Sibling pages within `/theory`:** the structural codification (`/theory/schema`) and the consolidated open questions (`/theory/open-questions`) are described in Tiers 2b and 2c below.

---

## Tier 2b — `/theory/schema` Schema reference (framing-schema layer)

**Source:** *A Framing Schema for the Working Context Model*

**Placement:** under `/theory` rather than as a sibling. The schema is the structural codification of the model; it belongs adjacent to the model, not above or beside it. Routing it as `/theory/schema` keeps that relationship visible in the URL.

**Naming:** `schema` rather than `primitives`. "Primitives" is the right internal word but is easy to misread — readers reach for it expecting a programming-language primitive or a low-level type. "Schema" matches the source document's own title and keeps the register honest: what lives here is the *structural codification* of the model, not the model itself and not the system that instantiates it.

**Status:** included in this first pass, with a load-bearing **work-in-progress** marker on the page itself. The source document is explicit that it is "a snapshot, not a final specification."

**What it is:** a structural reference surface containing, in roughly this order:
- An opening note that names what the schema is, what it is not, and where it sits relative to `/theory` and `/using-temper`. This note is the page's WIP framing — a paragraph, not a banner.
- The entity types, event structure, topic taxonomy, roles-within-events, derived structures, mechanics, stratification, accountability vectors, and chain-link-kinds tables. The source document's tabular form is appropriate as is.
- The resolved stances. These are the load-bearing commitments and are stable enough to publish in line.
- A pointer to `/theory/open-questions#schema` for the unsettled material. The "Intentionally Open" and "Pending Opinionated Stance" lists *do not* live on the schema page itself — they unify with the model's open questions on the dedicated open-questions page below.

**Why the open lists move off this page:** the schema's unsettled material is genuinely the same kind of thing as the model's open questions. Putting them in two separate places risks them drifting; putting them together under a single anchored structure (with a `#schema` section) gives a single canonical location that gets updated as items resolve.

**Tone for the page:** reference rather than tutorial. The page assumes the reader has read or is reading `/theory`. It does not re-introduce the model and does not pretend to be a system spec — the schema is what the system *answers to*, not what the system *is*.

---

## Tier 2c — `/theory/open-questions` What is not settled

**Sources:** *A Semantic Model of Working Context* (its "Open Questions about the Model Itself" section) and *A Framing Schema for the Working Context Model* (its "Intentionally Open" and "Pending Opinionated Stance" lists).

**What it is:** a single page that gathers the open material from both source documents under stable anchors. Roughly:
- `#model` — open questions about the model itself (the model's own "Open Questions" section: field sub-typing, retroactive correction, projection-vs-resource, manifold composability, scar decay, perspective granularity, role-persona evolution, trust).
- `#schema` — the schema's "Intentionally Open" and "Pending Opinionated Stance" lists. Cross-linked from `/theory/schema`.

**Why one page rather than per-tier inline:** these questions belong to the work as a whole, not to any one page. Surfacing them in one place lets a reader who wants to see "what isn't yet settled" land on a single canonical surface. It also lets the page evolve coherently — as items resolve, they leave this page and land in the appropriate model or schema section, with the rest of the list staying intact.

**Tone:** honest reference. Each open question gets its source-document phrasing, with at most a sentence of context. No editorial smoothing — the source documents are deliberate about leaving these open, and the page inherits that posture. Items resolve by being moved off the page (with the body of the resolution landing on the appropriate `/theory` page), not by being annotated in place.

**Maintenance:** the page updates over time. The IA proposal does not specify a process for how items move on or off; that is a working-practice question, not an IA one.

---

## Tier 4 — `/using-temper` How to use Temper (operational layer)

**Source:** the existing `/docs` page (install, commands, sync, MCP, etc.).

**Status:** the operational tier lives at `/using-temper`. The current `/docs` route — semantically nondescript — retires. Every current "getting started" link in the site, in the README, in agent-facing surfaces, resolves to `/using-temper`.

**What it is:** the existing CLI / sync / MCP reference, rehomed at a semantically meaningful URL. No restructure in this first pass — that is a separate piece of work. The page keeps its current scope (install, vault, resources, search, knowledge graph, cloud sync, teams, agents, config).

**What changes in this pass:**
- Route renamed `/docs` → `/using-temper`. Redirects from `/docs` and `/docs/*` to the new path (single canonical URL; no duplicate-content split).
- Internal links on the existing site that point to `/docs` get updated.
- The `<title>` and lede update minimally to match the new framing ("Using Temper" rather than "Docs — temper").
- A short header sentence acknowledges the conceptual frame lives at `/theory` and points there for readers who want it. The page itself does *not* re-introduce the model — operational content stays operational.

**What deliberately does not change in this pass:**
- The CLI reference content itself. It is operationally complete and changes to it are out of scope for the translation work.
- Cross-links from `/theory` into `/using-temper`. The conceptual pages can deep-link operational examples later (e.g., the deformation page linking to `temper resource delete` as the system's instantiation of folding-vs-deformation), but that work belongs to the per-page drafts, not the IA.

---

## Editorial questions — all resolved

All seven questions surfaced in earlier passes have resolutions. Captured here for the audit trail; this section can collapse to a summary once the per-page drafting is underway.

- **Q1 — Operational content in this first pass.** Included. Existing operational content rehomes at `/using-temper`; current getting-started links route there.
- **Q2 — Naming what is not settled within `/theory`.** Dedicated `/theory/open-questions` page, with `#model` and `#schema` anchors. Items move off the page as they resolve and land on the appropriate `/theory` or `/theory/schema` section.
- **Q3 — Scope of unsettled material on the schema page.** The schema page itself holds only the structural tables and resolved stances; the "Intentionally Open" and "Pending Opinionated Stance" lists migrate to `/theory/open-questions#schema`. Single canonical home for unsettled material, cross-linked from `/theory/schema`.
- **Q4 — Audience and tone.** Primary audience is the technical reader, the project contributor, or the information-systems professional engaging the model on its own merits. Not the newcomer who has just heard about Temper. The voice can assume capacity for abstract argument and does not need to translate the model into productivity-speak. The current `landing-draft.md` is already in this register; no copy revisions needed on that basis.
- **Q5 — Framing-schema material in this first pass.** Included, under `/theory/schema`, with explicit work-in-progress framing on the page.
- **Q6 — Existing get-started content.** Retained, rehomed at `/using-temper`. The retire of `/docs` is total but the content survives.
- **Q7 — Layout shell, navigation, other public pages.** Existing top-level pages — `/agents`, `/builders`, `/how-it-works` — are retained as is. The only change to the existing surface in this pass is `/docs` → `/using-temper`; all new pages live under `/theory`.

---

## What this proposal deliberately does not do

- **Does not write the foundations pages.** The brief was one draft page; this proposal does not preempt six.
- **Does not specify component structure or Svelte routing details.** Those follow from the IA, not the other way around.
- **Does not propose visual design changes.** The current site has an established register (font choices, palette, layout). The new frame can inhabit that register without redesigning it.
- **Does not propose changes to `/agents`, `/builders`, `/how-it-works`.** Those are out of scope until the landing is settled.
- **Does not commit the existing `/docs` page to deletion.** That is one of the open questions.

---

## What's unblocked for the next session

All seven editorial questions have resolutions. The IA is stable enough to drive per-page drafting next. In rough order of value:

1. `/theory` — overview page. Establishes the model's anchor (attention as teleological anchor) and previews the six sub-pages, the schema, and the open-questions page. Roughly the source document's first ~300 words, edited for a reader who has already read `/` but not the source.
2. `/theory/ontology` — the data / intention / information / knowledge stratification, with the *knowledge-bases-are-misnomers* commitment made explicit.
3. `/theory/manifold` — positions, fields, streams, bidirectional coupling.
4. `/theory/time` — time-as-primary-axis, events-as-primary, why this is a substrate commitment.
5. `/theory/deformation` — forming and forgetting, recording threshold, scarification, self-cohesion vs. relaxation.
6. `/theory/perspectives` — trajectories not points, role vs. individual, access vs. expertise.
7. `/theory/translation` — irreducibility, bridges, observer-relativity, knowledge as relationship.
8. `/theory/schema` — structural reference, WIP framing, with a pointer to open-questions for unsettled material.
9. `/theory/open-questions` — the consolidated open material, anchored as `#model` and `#schema`.
10. `/using-temper` — rehome of the existing `/docs` content. Mostly a routing change; minor header / title edits.

Items 1–9 are conceptual drafting. Item 10 is mostly routing. None of these need to land in this session; each can be a separate review pass.
