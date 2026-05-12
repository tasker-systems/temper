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
/                           What Temper is for             (manifesto layer)
/theory                     Working context: a model       (model overview + anchor)
/theory/ontology              Data, intention, information, knowledge
/theory/manifold              Positions, fields, streams
/theory/time                  Time, events, derivation
/theory/deformation           Forming, forgetting, scarification
/theory/perspectives          Who is asking
/theory/translation           Irreducibility, bridges, observer-relativity
/schema                     Schema reference (work in progress)   (framing layer)
/using-temper               How to use Temper                     (existing /docs content, rehomed)
```

Four tiers, each operating at a different layer of the source documents. No tier pretends to be a simpler version of the tier above it. The existing `/docs` route — semantically nondescript — is retired; its content rehomes at `/using-temper` and every current "getting started" link resolves there.

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

**Open question deferred to Pete:**
> Q: Should `/theory` include the source document's "Open Questions about the Model Itself" section, or is that internal-facing? See *Editorial questions surfaced* below.

---

## Tier 3 — `/schema` Schema reference (framing-schema layer)

**Source:** *A Framing Schema for the Working Context Model*

**Naming:** `/schema` rather than `/primitives`. "Primitives" is the right internal word but is easy to misread — readers reach for it expecting a programming-language primitive or a low-level type. "Schema" matches the source document's own title and keeps the register honest: what lives here is the *structural codification* of the model, not the model itself and not the system that instantiates it.

**Status:** included in this first pass, with a load-bearing **work-in-progress** marker on the page itself. The source document is explicit that it is "a snapshot, not a final specification," and that the boundary between *intentionally open* and *pending opinionated stance* will continue to shift. Public exposure has to inherit that honesty rather than paper over it.

**What it is:** a single reference surface that contains, in roughly this order:
- An opening note: what this page is for, what it is not, and how stable each section is. The note is the page's WIP framing — not a banner-style "🚧 under construction" disclaimer, but a paragraph that names exactly what the schema is and where it sits relative to the manifesto and theory tiers.
- The entity types, event structure, topic taxonomy, roles-within-events, derived structures, mechanics, stratification, accountability vectors, and chain-link-kinds tables. The source document's tabular form is appropriate as is.
- The resolved stances. These are the load-bearing commitments and are stable enough to publish.
- The "Intentionally Open" and "Pending Opinionated Stance" lists. Both belong on this page — they are part of what the schema *is* as a document. Cutting them would misrepresent the source. See Q3 below for the alternative if that decision is reversed.

**Rationale for including the unsettled material:** The source document is honest about its own provisional shape, and the manifesto's commitment to making perspective-differences visible applies recursively to the docs themselves. A schema page that publishes only the stable parts and hides the unstable ones would be the "single canonical view" failure mode the model rules out. Better to publish the schema as it is — work-in-progress and all — than to pretend a partial view is the whole.

**Tone for the page:** reference rather than tutorial. The page assumes the reader has read `/theory` (or, more likely, will use this page as a lookup surface alongside `/theory`). It does not re-introduce the model. It also does not pretend to be a system spec — readers reaching this page from the API or the source code should immediately see the WIP framing and understand that the schema is what the system *answers to*, not what the system *is*.

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

## Editorial questions

### Resolved by Pete's guidance

- **Q1 — Operational content in this first pass.** Resolved: included. Existing operational content rehomes at `/using-temper`; current getting-started links route there. Tier 4 above reflects this.
- **Q5 — Framing-schema material in this first pass.** Resolved: included. Lives at `/schema` (not `/primitives`) with an explicit work-in-progress framing on the page. Tier 3 above reflects this.
- **Q6 — Existing get-started content.** Resolved: retained, rehomed at `/using-temper`. The retire of the semantically-nondescript `/docs` is total but the content survives.

### Still open

The schema decision (Q5) partially informs Q2 and Q3 but does not fully resolve them. The page-level treatment of unsettled material is still a real call.

- **Q2 — Naming what is not settled within `/theory`.** The schema page will include its "Intentionally Open" and "Pending Opinionated Stance" lists as part of the published schema. Separately, the semantic-model document has its own "Open Questions about the Model Itself" section. Should the `/theory` tier surface those — as a dedicated `/theory/open-questions` page, as footnotes on the relevant theory pages, or not at all in this first pass? My instinct: a dedicated page. Symmetric with the schema's transparency about its own state.

- **Q3 — Scope of unsettled material on `/schema`.** Pete's guidance includes the schema with WIP framing, which I'm reading as: include the Intentionally-Open and Pending-Opinionated-Stance lists. Confirming. If the intent was "include the resolved stances and the structural tables but hold the unsettled lists internal," the Tier 3 description above needs a corresponding trim.

- **Q4 — Audience and tone.** Still open. The three plausible primaries:
   - Newcomer (just heard about Temper)
   - Technical reader (engineer / researcher engaging the model on its own merits)
   - Self-hoster (operational; cares about substrate commitments)

   The current landing privileges (1). The new framing tilts toward (2) but does not have to. Tone decision is most visible at `/` and `/theory`; less load-bearing on `/schema` (reference register is appropriate regardless) and `/using-temper` (existing register is fine).

- **Q7 — Layout shell, navigation, other public pages.** The current site has `/agents`, `/builders`, `/how-it-works`. None of those are touched in this proposal. Whether they survive the landing rewrite, get rewritten, or redirect into the new structure is a decision the new `/` makes downstream pressure on.

---

## What this proposal deliberately does not do

- **Does not write the foundations pages.** The brief was one draft page; this proposal does not preempt six.
- **Does not specify component structure or Svelte routing details.** Those follow from the IA, not the other way around.
- **Does not propose visual design changes.** The current site has an established register (font choices, palette, layout). The new frame can inhabit that register without redesigning it.
- **Does not propose changes to `/agents`, `/builders`, `/how-it-works`.** Those are out of scope until the landing is settled.
- **Does not commit the existing `/docs` page to deletion.** That is one of the open questions.

---

## What I'd want to see resolved before drafting more pages

In rough order of impact:

1. Q4 (primary audience). The largest remaining unknown; affects voice on `/` and the entire `/theory` tier.
2. Q2 + Q3 (handling of unsettled material on `/theory` and `/schema`).
3. Q7 (other public pages — `/agents`, `/builders`, `/how-it-works`).

Resolving Q4 unblocks the per-page drafting in `/theory`. Q2/Q3 can be resolved alongside, but slightly downstream — they affect at most one additional page.
