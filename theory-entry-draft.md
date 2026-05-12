# `/theory` entry draft

First-pass draft for the `/theory` entry page. Built from two source documents: opens with the *Attention Manifesto* in its own voice (first person), shifts to third person for the *Semantic Model* material, and closes in first person where the manifesto's closing supplies the right note.

**Target length:** ~900 words. Longer than a typical entry because the entry is doing the work of orienting a reader to both the why (manifesto) and the what (model preview).

**Important framing:** this is **not** the temperkb.io landing. The existing landing at `/` is unchanged. `/theory` sits alongside it for readers who have opted into "tell me what this is for, at the level the work is thinking about."

---

## Page copy (draft)

---

# What Temper is building toward

Attention is our most precious resource. I am building Temper as a commitment to respecting it.

Attention is how I experience myself, time, and the world. It is how I am present to my own life, how I direct my agency, how I make any of the choices I think of as mine. It is the medium of intention — what I do with my attention is what I do at all. When I direct it well, I am the agent of my own work; when it is fractured or hijacked or spent without my consent, I lose ground in the most fundamental sense.

This is true of every perspective capable of attention, not just mine. When I demand attention from a colleague, a friend, or an agent working on my behalf, I am asking for their capacity to be present and to act. Every interrupt is a demand on that capacity. Every poorly-justified ping, every system that requires re-discovering what it could have made present, every interface that demands construction-from-scratch when reasonable defaults exist, is a demand on something irreplaceable. The cost compounds, because every low-leverage demand on attention is attention not available for what actually matters — and attention, unlike most resources, does not regenerate. When it is spent, it is spent.

I believe this gives the design of information systems an ethical character it usually lacks. If attention is the medium of agency, then a system that wastes attention isn't just inefficient — it is treating something precious as fungible. Efficiency is not, in this frame, a productivity virtue. It is an obligation — *efficiency-as-ethic* — that follows from taking attention seriously as the thing it actually is.

Temper is built around what falls out of that obligation. Building it in this frame of reference, I am committed to a few key principles.

## Common queries should not require fresh attention each time

Most people in a given role ask roughly the same kinds of questions, with the same kinds of intent behind them. Pre-paying those queries — making them cheap by default — is not paternalism. It is the system doing the work that does not need to be done freshly each time, so attention can land on what is actually new.

## Perspective-differences are real and should be made visible, not silently flattened

Different people working on the same thing produce genuinely different information from the same data, because they engage it from different positions with different concerns. A system that pretends otherwise — that produces a single canonical view — forces attention to be spent re-discovering those differences in every conversation. Surfacing them is how a system spends attention once and saves it forever.

## Information past its time should fade, not crowd the present

Information that is no longer relevant should grow harder to find; information that is no longer true should not surface as if it were; information that needs to be preserved for audit should not pollute default retrieval. None of this is unusual to want. What is unusual is taking it seriously enough to design for, rather than letting deprecation tags pile up while everything stays equally findable.

## Where the system has been wrong, future engagement should know

Confidence and accuracy are not the same thing. Without a trace of where errors have lived before, attention cannot land on what needs scrutiny — and a system that hides its own past errors is asking attention to do work the system should have done.

## One commitment worth naming up front

These are not features I want to ship. They are commitments I want to keep. There is a separate semantic model where the architectural detail lives — what the manifold is, what fields are, how forgetting works geometrically, how perspectives are characterized. The pages under `/theory` introduce that model.

One commitment in the model runs through everything that follows, and deserves naming up front: the system stores data and traces of past intentional acts — recorded questions, notes, decisions, which themselves become further data. It does not store *knowledge*. Knowledge is the relationship between a perspective and the information that perspective produces through engagement with data.

The label "knowledge base" is a misnomer in light of this. Knowledge is always potential, never actual, until activated by a perspective. The system's job is never to *be right about what something means* — meaning is not in the system. Its job is to faithfully represent data, faithfully record intentions, and faithfully compute projections such that perspectives engaging with those projections are well-equipped to produce knowledge.

## The shape of the model

The pages here introduce the model in the order the source document does. Each is short; each can be read alone; the sequence is the most coherent path through.

- **[Ontology](/theory/ontology)** — Data, intention, information, knowledge. The stratified layers.
- **[Manifold](/theory/manifold)** — Positions, fields, streams. The geometry.
- **[Time](/theory/time)** — Time as a primary axis. Events-as-primary. Why this is a substrate commitment.
- **[Deformation](/theory/deformation)** — Forming and forgetting. Strong vs. weak. Scarification. Self-cohesion and relaxation.
- **[Perspectives](/theory/perspectives)** — Trajectories, not points. Role-perspective vs. individual. Access vs. expertise.
- **[Translation](/theory/translation)** — Why translation is irreducible. Bridges. Knowledge as relationship.

Two reference surfaces sit alongside:

- **[Schema](/theory/schema)** — The structural codification: entity types, event structure, topic taxonomy, resolved stances. Work in progress.
- **[Open questions](/theory/open-questions)** — What is not yet settled. Updates over time as items resolve.

The model is provisional. It captures a mental picture clearly enough to be argued with.

---

I am not trying to win an academic argument or start a movement. I am writing this so that when I am six months into implementation and tempted to take a shortcut that costs the user some attention they will not get back, I have something to read that reminds me why I started. I am publishing it because the commitment is more honest if it is shared, and because anyone considering this tool deserves to know what its author thinks the tool is *for* before they decide whether to spend their attention on it.

---

## Editorial notes

- **Voice.** First person where the manifesto is the source (anchor, four commitments, transition into the model, closing). Third person where the semantic model is the source (the *knowledge-base-is-a-misnomer* paragraphs). The two registers meet at "One commitment in the model runs through everything that follows" — the page shifts from "I am committed to..." to "the system stores..." without a hard break.
- **Opening through the four commitments: source verbatim or near-verbatim.** Paragraphs 1–4 are the manifesto's opening four paragraphs, lightly trimmed only where necessary to fit the page rather than the manifesto-as-essay (e.g., omitting "I am not just less productive — I am less present, less authentic" from paragraph 2, which is a self-referential note that doesn't carry the same load in a docs context). The four commitment sub-sections use the manifesto's own headings and prose. This is the correction from the previous draft, which paraphrased the framing paragraphs and produced "a working perspective brings to its work" — a hedge that made the sentence nonsensical.
- **Transition into the model:** "These are not features I want to ship. They are commitments I want to keep" is the manifesto's own pivot. The manifesto continues with "This document is not that. This document is what the model is *for*." The page adapts that to "The pages under `/theory` introduce that model" — inverting the framing because the page *is* the model's introduction.
- **The misnomer paragraphs are from the semantic-model doc** (third person). Drawn from the *K = experience(P, I)* discussion's consequence: "knowledge bases are misnomers."
- **Closing: manifesto verbatim** with one adjustment — "anyone considering this tool deserves to know what its author thinks the tool is *for*" stays, even though the reader has already navigated to `/theory`. The phrase still applies; nothing about reaching this page commits the reader to staying with it.
- **Title:** "What Temper is building toward" is the page's own title, not from any source. It frames the tier as forward-looking, which is the framing Pete has confirmed: `/theory` describes the direction, not the current state. Open to redirect.

## Things considered and rejected

- **Blockquoting the manifesto's opening.** Would treat the manifesto's voice as borrowed; the choice was instead to let the page wear the manifesto's voice as its own where the manifesto is the source. (Decided by AskUserQuestion in the rewrite session.)
- **Converting first person to impersonal docs voice.** Weakens the manifesto's force; the personal commitment is part of the claim. (Same decision.)
- **Opening with a product-category definition.** Would flatten the why into a category description. The page opens with what the work is *for*.
- **A CTA, install link, or "get started" affordance.** The page assumes the reader has opted into the theory tier and is here to engage on those terms.
- **A manifold or fields diagram.** Belongs at `/theory/manifold`. On the entry it would smuggle structural claims past the values frame.
- **A code or CLI sample.** Belongs at `/using-temper`.
