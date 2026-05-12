# Manifold draft — `/theory/manifold`

First-pass draft for the manifold page. Introduces positions, fields, streams, bidirectional coupling, and projection.

**Target length:** ~700 words.

---

## Page copy (draft)

---

# The manifold

The model's geometric vocabulary for *aboutness*.

## The manifold

All resources — documents, decisions, concepts, sessions, observations, anything that can be referred to — occupy positions in a high-dimensional space. Position is determined by *aboutness*, and aboutness is multi-axial: a single resource is generally about several things at once (a goal, a surface area, a moment in some state machine, a constraint, an open question).

The manifold is not a tree. Hierarchical organization is at best a particular *projection* of position, and any given projection loses information. The manifold is also not flat: it has curvature, induced by the fields and deformations described below. Its geometry at any moment is the integrated result of all strong deformations recorded into the substrate up to that moment, with relaxation rates and decay distances applied. There is no "current state" of the manifold over and above the integrated event history.

## Streams and particles

Intention in motion is a *stream* flowing through the manifold. Each stream carries information — a query, an attention-state, a current focus — and its trajectory is shaped by the fields it traverses. Streams are not stored; they are *witnessed*.

Stream and particle are two ways of seeing the same phenomenon. Stream foregrounds continuity, flow, accumulation. Particle foregrounds discreteness, event, witness. The substrate is one thing; both vocabulary registers are useful depending on what is being described. A session is best described as a stream; a single query is best described as a particle within that stream.

At the level of the model, a stream is intention in motion. At the level of the schema, that motion resolves into events with topics. Emissions add new positions to the manifold; observations reinforce existing ones; corrections deform and scar. The model's "stream" and the schema's topic-classes describe the same phenomenon at different resolutions. The [schema](/theory/schema) is where the resolution lives in full.

## Fields

Concerns and intentions — active goals, decisions in force, ongoing sessions, standing constraints, ambient tolerances, open investigations — are *fields* over the manifold. A field is not a position; a field is a force that makes positions matter.

Each field has:

- A **spatial profile** — which regions it influences, with what intensity at each point.
- A **weight** — how much it matters at the current moment.
- A **temporal character** — how its profile and weight evolve over time.
- A **characteristic decay distance** — a finite range of influence that falls off with distance.

Fields interact. Where multiple high-weight fields constructively interfere, the manifold's local topology bends toward that region. That region is what becomes salient.

## Bidirectional coupling

The relationship between streams and the manifold is bidirectional. Streams flow through the manifold, shaped by its fields. Streams also *deform the manifold they flow through*, including the fields that shaped them. The manifold is not a thing the streams encounter from outside — the manifold is *constituted by* the history of streams that have flowed through it.

This is the model's most consequential commitment. Asking a question, choosing what to attend to, deciding what to investigate — these shape the field before any artifact is created or any code changes. Attention is not neutral observation; attention is force, and force deforms.

The mechanics of how that deformation operates — what counts as a strong deformation, what counts as weak, when sub-recording attention crystallizes into something the system tracks — live on [the deformation page](/theory/deformation).

## Projection

What an observer experiences as "the current context" is a *projection* of the stream activity against the currently-active field configuration, computed at projection time from the integrated event history. Projections are lossy by necessity — they collapse a high-dimensional, time-extended reality into a finite, addressable surface (a search result, a vault listing, a graph view, an editor pane).

Different projections serve different purposes; none is the territory. Any system that exposes only one projection is hiding most of the model from its users.

## Context is a verb-state

In conventional knowledge-management vocabulary, *context* is a noun: a container resources live in. In this model, context is a verb-state: the currently-active configuration of fields, weighted by their current intensities, projected through a temporal lens, from the position of a particular perspective.

A persistent, low-friction field that happens to be on most of the time may *behave* like a containment-context — it can be addressed, named, used as a folder. But it is not ontologically a container. It is a long-running field that has settled into stationarity.

Any addressing scheme that treats context as a noun is taking a single low-dimensional projection of position and discarding the rest. This may be a useful fiction for stable addressing, but the internal representation must be richer than the addressable surface, or the model collapses to the noun-version it was supposed to replace.

---

## Editorial notes

- The bidirectional-coupling claim is named with the source's unqualified phrasing — "the model's most consequential commitment." An earlier draft had "most consequential commitment about the geometry," which weakened the source's claim; restored.
- The "context as verb-state" section from the source document lives here rather than in its own page. It is short and depends on field/projection, so combining is more attention-economic than fragmenting.
- The page intro is intentionally a single line ("The model's geometric vocabulary for *aboutness*."). An earlier draft previewed the page's contents in the intro; trimmed to let the section headers do that work.
- **The closing paragraph of "Streams and particles" bridges to the schema's topic-classes.** The model-level "stream" and the schema-level "emissions / observations / corrections" describe the same phenomenon at different resolutions. Naming the bridge here keeps the model and schema vocabularies legible against each other without requiring the reader to detour through `/theory/schema` to make the connection.
- Forward links: `/theory/deformation` for the mechanics of how streams deform the manifold; `/theory/schema` for the topic-class resolution.
