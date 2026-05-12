# Schema draft — `/theory/schema`

First-pass draft for the schema reference. Structural tables + resolved stances inline; "intentionally open" and "pending opinionated stance" lists migrate to `/theory/open-questions#schema`.

**Target length:** ~1000 words, mostly tables.

---

## Page copy (draft)

---

# Schema

**This page is a working reference.** The structural codification of the model is a snapshot, not a final specification. The substrate-level commitments below — events-as-primary, append-only ledger, cross-cutting point-in-time truth, observation-as-scoped-event-topic, derivable-not-denormalized perspective — feel stable enough to build against. Other areas are still moving. Unsettled material lives at [/theory/open-questions#schema](/theory/open-questions#schema).

The schema is what the system *answers to*. It is not the system itself.

## Entity types

| Type | Description | Notes |
|------|-------------|-------|
| Discrete entity | Perspective-bearing, non-aggregate. Has capacity-for-observation and a position in perspective-space. Subtypes: human, agent, deterministic-system. | Plays the Emitter role within events. |
| Aggregate-perspective | Perspective-bearing aggregate (team, squad, affinity group, etc.). Has position-in-perspective-space, accumulates trajectory, but does not emit. | Membership is a set of discrete entities. |
| Resource | Non-perspective-bearing addressable artifact. Subject of resource-lifecycle events. | Documents, code files, decisions-as-records, etc. |
| Resource-aggregate | Entity produced by perspective-laden inclusion/exclusion of resources. | Itself a resource for downstream purposes. |
| Role-class | Characterizable perspective-class providing Bayesian priors over intention-vectors and field-sets. | Applies to both individual and group classes. |
| Event | Atomic recorded observation. The substrate's primary and universal unit. | All other facts derive from events. |

## Event structure (universal)

Every event in the substrate carries this core, regardless of topic.

**Core (always present):**
- Emitter (discrete entity)
- Time
- Topic (determines payload-schema expectations and downstream semantic)
- Payload (consistency-marked against topic)
- References (relations to other events or entities)
- On-behalf-of (optional; aggregate-perspectives in whose scope the emission occurred)
- Traceability metadata

**Notably not in core:**
- *Perspective* — derivable from emitter+time via the emitter's trajectory; never denormalized.
- *Observer* — observation is a topic; observation-events have an emitter who is also the observer of the referenced event (self-reference within payload).

## Topic taxonomy

Categories below are illustrative, not closed.

| Topic Class | Examples | Notes |
|-------------|----------|-------|
| Resource lifecycle | publication, contribution, modification, supersession | Acts on resources; emissions add or alter geometry |
| Declaration | decision, role-grant, authority-grant, membership-grant | Establish or change relations; strong deformations |
| Observation | observation | Observing-entity-scoped; emitter is the observer; payload references the event observed; terminal — not itself observable; no implied commitment |
| Operational commitment | ack, rej, act, dispatch | Secondary to observation-events; represent observer's agency in publicly engaging |
| Judgment | attribution-judgment, evaluation, scar-marking | Recorded assessments about other events or entities |
| Translation | translation-event, bridge-formation | Translation-work-as-events between perspectives |
| State change | role-change, position-update, membership-change | Entity-trajectory events; project to point-in-time entity state |
| Deformation | strong-deformation, decay-marker, fold-event | Manifold-reshaping events |

## Roles within events

| Role | Entity Type | Cardinality per event |
|------|-------------|----------------------|
| Emitter | Discrete entity | Exactly one |
| On-behalf-of | Aggregate-perspective | Zero, one, or many |
| Subject | Resource or entity | Zero or many (per topic) |

For observation-topic events, the emitter is also the observer of the referenced event; this is recorded in the payload, not as a separate role.

## Derived structures

Computed from event history at projection time, not maintained as separate state.

| Structure | Description |
|-----------|-------------|
| Manifold geometry | High-dimensional space of positions; deformed by event history per topic-class |
| Event-graph | Network of events connected by references; carries temporal structure |
| Resource-graph | Network of resources connected by aboutness/supersession/derivation; non-isomorphic to event-graph |
| Field configuration | Active concerns/intentions as forces at projection time |
| Aggregate trajectory | Arc-through-time of an aggregate-perspective; computed from events emitted on-behalf-of it |
| Entity trajectory | Arc-through-time of a discrete entity; computed from its emissions and state-change events about it |
| Entity state at time T | Projection of entity's trajectory to a specific time; supports cross-cutting as-of queries |
| Bridging structure | Accumulated translation outcomes between perspectives |
| Authority projection | Who-directs-whom; declared and emergent strata projected separately |

## Mechanics

| Mechanic | Description | What it answers |
|----------|-------------|-----------------|
| Strong deformation | Discrete, deliberate, substantial geometric change (declaration topics; strong-deformation topic) | When is the manifold reshaped overtly? |
| Weak deformation | Continuous, sub-recording pressure | What accumulates without being captured? |
| Emission-adds-geometry | Resource-lifecycle and declaration events add new positions to the manifold | How does the manifold acquire new structure? |
| Observation-reinforces-geometry | Observation-topic events reinforce the position-strength of events they reference | How does engagement strengthen existing structure? |
| Correction-scars-geometry | Supersession and correction events both deform and carry scar-property | How does past-wrongness persist as a property of corrected regions? |
| Self-cohesion | Resistance to bending under weak forces at any moment | Does this deformation register? |
| Background relaxation | Tendency to return to ambient state over time without reinforcement | Does it stick? |
| Recording threshold | Boundary at which weak crystallizes into strong | When does sub-recording attention become tracked? |
| Decay (forgetting) | Drift away from active manifold region without reinforcement | When does this become "no longer relevant"? |
| Deformation (forgetting) | Topological change after correction/supersession | When does this become "no longer true"? |
| Folding (forgetting) | Moved to separate sheet, accessible only by deliberate time-travel | What is "preserved but not present"? |
| Scarification | Property attached to corrective deformations | How does trace of past-wrongness inform future engagement? |
| Aggregation | Perspective-act of inclusion/exclusion producing a new entity | How does selection produce new addressable things? |
| Projection | Lens-enacted view at query-time | How does a perspective surface what matters? |

## Stratification

| Layer | Description |
|-------|-------------|
| Substrate | Totality of recorded events (all topics), append-only. Not "objective data"; the perspective-laden record of past emission acts. |
| Information | What happens when one perspective engages with events emitted by other perspectives (including past versions of itself). |
| Knowledge | Relational outcome of engagement — never stored, always produced. |

## Accountability vectors

Accountability decomposes into distinguishable vectors, not types. A single event may have all of them pointing at different entities.

| Vector | What it captures |
|--------|------------------|
| Emission | Discrete entity whose act produced the event; the formally traceable one |
| Attribution-by-contribution | Weighted across contributors based on what they contributed; never computed by the substrate |
| Authority | Power-relations that shaped the emission; has declared and emergent strata |
| On-behalf-of chains | Scopes within which the emission occurred; may chain across delegation kinds |

## Chain link kinds (within on-behalf-of chains)

| Kind | Example |
|------|---------|
| Technical delegation | Agent invokes another agent for a subtask |
| Instrumental delegation | Human deploys agent with instructions |
| Representational membership | Human acts as part of an aggregate |
| Organizational authority | Aggregate operates under another aggregate's scope |

## Resolved stances

The schema's load-bearing commitments. Stable enough to build against.

- Attention is the teleological anchor; design constraints derive from it.
- Events-as-primary; the substrate IS the event history.
- Append-only ledger: no retroactive mutation; everything else is derivable.
- Cross-cutting point-in-time truth: for any event at time T, all associated entities' states at T are projectable from the trajectory of state-change events.
- Every event has an emitter and a perspective; perspective is derivable from emitter-trajectory at event-time, not denormalized.
- Emitter is discrete-non-aggregate.
- Aggregate-perspectives have positions and trajectories but do not emit.
- On-behalf-of is the bridge primitive between discrete accountability and aggregate scope.
- Observation is an event-topic (observing-entity-scoped, terminal, no implied commitment); audit happens at projection, not as further observation-emissions.
- Operational commitments (ack/rej/act) are secondary emissions referencing observation-events; receipt does not imply commitment.
- Emissions add geometry; observations reinforce geometry; corrective deformations scar geometry — bidirectional coupling differentiated by topic-class.
- Entity state is derived from state-change events, not stored as denormalized state.
- Capture richly, project plurally: substrate carries consistent metadata; weighting/scoring lives in projections.
- System surfaces, does not resolve: translation, attribution, accountability, disagreement.
- Resource-graph and event-graph are non-isomorphic derived structures.
- Atomicity is observer-relative, not absolute.
- A single substrate-emission can produce multiple observation-events (one source event; one observation-event per engaging observer).
- Affordance-composition over typed inheritance; topic carries payload-schema affordances.
- Authority has declared (formal) and emergent (practical) strata; may diverge; both projectable.
- Weak observer-relativity: shared substrate, observer-specific projections; shared understanding emerges from convergent histories.
- Knowledge is relational; the system stores conditions for knowledge production, not knowledge.
- Internal multitudes of perspectives are a commitment-about-what-we-don't-model; substrate records emissions, not perspective-states.

## What's still moving

Two classes of unsettled material live at [/theory/open-questions#schema](/theory/open-questions#schema):

- **Intentionally open** — questions the model deliberately does not specify. Downstream system designs answer these; the schema does not. Examples: storage substrate, query language, authority model, persona/role library.
- **Pending opinionated stance** — gaps the schema needs to close before it is fully coherent. Each wants pressure before stabilizing. Examples: minimum-viable state-change event schema, whether on-behalf-of is single- or multi-valued, scar decay mechanics, manifold composition.

The boundary between *intentionally open* and *pending opinionated stance* will shift as the work continues.

---

## Editorial notes

- The WIP framing is the opening paragraph, not a banner. A banner would treat WIP as a deficiency; the paragraph treats it as a property the reader needs to know about. The framing schema source document treats its own provisionality this way.
- The unsettled lists are *not* duplicated here. Schema page → single canonical reference; open-questions page → single canonical location for what's moving. Two pointers in one direction; nothing repeated.
- The page is heavy on tables. That is appropriate: it is a reference surface and the source document is already tabular.
