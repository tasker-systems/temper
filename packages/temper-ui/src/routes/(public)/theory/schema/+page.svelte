<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Schema — temper</title>
  <meta
    name="description"
    content="The structural codification of the model. A snapshot, not a final specification. Entity types, event structure, topic taxonomy, resolved stances."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory/schema</div>
  <h1 class="t-hero-title"><em>Schema</em></h1>
  <p class="tagline t-tagline">
    The structural codification of the model. A snapshot, not a final
    specification.
  </p>
</section>

<div class="theory-page">

<Section label="What this is">
  <p>
    <strong>This page is a working reference.</strong> The structural
    codification of the model is a snapshot, not a final specification. It
    emerged from successive refinements of the underlying model and is at
    the resolution where it can be built against — not where it is
    finished. The substrate-level commitments below — events-as-primary,
    append-only ledger, cross-cutting point-in-time truth,
    observation-as-scoped-event-topic, derivable-not-denormalized
    perspective — feel stable enough to build against. Other areas are
    still moving. Unsettled material lives at
    <a href="/theory/open-questions#schema">/theory/open-questions#schema</a>.
  </p>
  <p>
    The schema is what the system <em>answers to</em>. It is not the
    system itself.
  </p>
</Section>

<Section label="Entity types">
  <h2>Entity types</h2>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Type</th>
        <th>Description</th>
        <th>Notes</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Discrete entity</td>
        <td>Perspective-bearing, non-aggregate. Has capacity-for-observation and a position in perspective-space. Subtypes: human, agent, deterministic-system.</td>
        <td>Plays the Emitter role within events.</td>
      </tr>
      <tr>
        <td>Aggregate-perspective</td>
        <td>Perspective-bearing aggregate (team, squad, affinity group, etc.). Has position-in-perspective-space, accumulates trajectory, but does not emit.</td>
        <td>Membership is a set of discrete entities.</td>
      </tr>
      <tr>
        <td>Resource</td>
        <td>Non-perspective-bearing addressable artifact. Subject of resource-lifecycle events.</td>
        <td>Documents, code files, decisions-as-records, etc.</td>
      </tr>
      <tr>
        <td>Resource-aggregate</td>
        <td>Entity produced by perspective-laden inclusion/exclusion of resources.</td>
        <td>Itself a resource for downstream purposes.</td>
      </tr>
      <tr>
        <td>Role-class</td>
        <td>Characterizable perspective-class providing Bayesian priors over intention-vectors and field-sets.</td>
        <td>Applies to both individual and group classes.</td>
      </tr>
      <tr>
        <td>Event</td>
        <td>Atomic recorded observation. The substrate's primary and universal unit.</td>
        <td>All other facts derive from events.</td>
      </tr>
    </tbody>
  </table>
</Section>

<Section label="Event structure">
  <h2>Event structure (universal)</h2>
  <p>Every event in the substrate carries this core, regardless of topic.</p>
  <p><strong>Core (always present):</strong></p>
  <ul>
    <li>Emitter (discrete entity)</li>
    <li>Time</li>
    <li>Topic (determines payload-schema expectations and downstream semantic)</li>
    <li>Payload (consistency-marked against topic)</li>
    <li>References (relations to other events or entities)</li>
    <li>On-behalf-of (optional; aggregate-perspectives in whose scope the emission occurred)</li>
    <li>Traceability metadata</li>
  </ul>
  <p><strong>Notably not in core:</strong></p>
  <ul>
    <li><em>Perspective</em> — derivable from emitter+time via the emitter's trajectory; never denormalized.</li>
    <li><em>Observer</em> — observation is a topic; observation-events have an emitter who is also the observer of the referenced event (self-reference within payload).</li>
  </ul>
</Section>

<Section label="Topic taxonomy">
  <h2>Topic taxonomy</h2>
  <p>Categories below are illustrative, not closed.</p>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Topic Class</th>
        <th>Examples</th>
        <th>Notes</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Resource lifecycle</td>
        <td>publication, contribution, modification, supersession</td>
        <td>Acts on resources; emissions add or alter geometry</td>
      </tr>
      <tr>
        <td>Declaration</td>
        <td>decision, role-grant, authority-grant, membership-grant</td>
        <td>Establish or change relations; strong deformations</td>
      </tr>
      <tr>
        <td>Observation</td>
        <td>observation</td>
        <td>Observing-entity-scoped; emitter is the observer; payload references the event observed; terminal — not itself observable; no implied commitment</td>
      </tr>
      <tr>
        <td>Operational commitment</td>
        <td>ack, rej, act, dispatch</td>
        <td>Secondary to observation-events; represent observer's agency in publicly engaging</td>
      </tr>
      <tr>
        <td>Judgment</td>
        <td>attribution-judgment, evaluation, scar-marking</td>
        <td>Recorded assessments about other events or entities</td>
      </tr>
      <tr>
        <td>Translation</td>
        <td>translation-event, bridge-formation</td>
        <td>Translation-work-as-events between perspectives</td>
      </tr>
      <tr>
        <td>State change</td>
        <td>role-change, position-update, membership-change</td>
        <td>Entity-trajectory events; project to point-in-time entity state</td>
      </tr>
      <tr>
        <td>Deformation</td>
        <td>strong-deformation, decay-marker, fold-event</td>
        <td>Manifold-reshaping events</td>
      </tr>
    </tbody>
  </table>
</Section>

<Section label="Roles within events">
  <h2>Roles within events</h2>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Role</th>
        <th>Entity Type</th>
        <th>Cardinality per event</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Emitter</td>
        <td>Discrete entity</td>
        <td>Exactly one</td>
      </tr>
      <tr>
        <td>On-behalf-of</td>
        <td>Aggregate-perspective</td>
        <td>Zero, one, or many</td>
      </tr>
      <tr>
        <td>Subject</td>
        <td>Resource or entity</td>
        <td>Zero or many (per topic)</td>
      </tr>
    </tbody>
  </table>
  <p>
    For observation-topic events, the emitter is also the observer of the
    referenced event; this is recorded in the payload, not as a separate
    role.
  </p>
</Section>

<Section label="Derived structures">
  <h2>Derived structures</h2>
  <p>Computed from event history at projection time, not maintained as separate state.</p>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Structure</th>
        <th>Description</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Manifold geometry</td>
        <td>High-dimensional space of positions; deformed by event history per topic-class</td>
      </tr>
      <tr>
        <td>Event-graph</td>
        <td>Network of events connected by references; carries temporal structure</td>
      </tr>
      <tr>
        <td>Resource-graph</td>
        <td>Network of resources connected by aboutness/supersession/derivation; non-isomorphic to event-graph</td>
      </tr>
      <tr>
        <td>Field configuration</td>
        <td>Active concerns/intentions as forces at projection time</td>
      </tr>
      <tr>
        <td>Aggregate trajectory</td>
        <td>Arc-through-time of an aggregate-perspective; computed from events emitted on-behalf-of it</td>
      </tr>
      <tr>
        <td>Entity trajectory</td>
        <td>Arc-through-time of a discrete entity; computed from its emissions and state-change events about it</td>
      </tr>
      <tr>
        <td>Entity state at time T</td>
        <td>Projection of entity's trajectory to a specific time; supports cross-cutting as-of queries</td>
      </tr>
      <tr>
        <td>Bridging structure</td>
        <td>Accumulated translation outcomes between perspectives</td>
      </tr>
      <tr>
        <td>Authority projection</td>
        <td>Who-directs-whom; declared and emergent strata projected separately</td>
      </tr>
    </tbody>
  </table>
</Section>

<Section label="Mechanics">
  <h2>Mechanics</h2>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Mechanic</th>
        <th>Description</th>
        <th>What it answers</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Strong deformation</td>
        <td>Discrete, deliberate, substantial geometric change (declaration topics; strong-deformation topic)</td>
        <td>When is the manifold reshaped overtly?</td>
      </tr>
      <tr>
        <td>Weak deformation</td>
        <td>Continuous, sub-recording pressure</td>
        <td>What accumulates without being captured?</td>
      </tr>
      <tr>
        <td>Emission-adds-geometry</td>
        <td>Resource-lifecycle and declaration events add new positions to the manifold</td>
        <td>How does the manifold acquire new structure?</td>
      </tr>
      <tr>
        <td>Observation-reinforces-geometry</td>
        <td>Observation-topic events reinforce the position-strength of events they reference</td>
        <td>How does engagement strengthen existing structure?</td>
      </tr>
      <tr>
        <td>Correction-scars-geometry</td>
        <td>Supersession and correction events both deform and carry scar-property</td>
        <td>How does past-wrongness persist as a property of corrected regions?</td>
      </tr>
      <tr>
        <td>Self-cohesion</td>
        <td>Resistance to bending under weak forces at any moment</td>
        <td>Does this deformation register?</td>
      </tr>
      <tr>
        <td>Background relaxation</td>
        <td>Tendency to return to ambient state over time without reinforcement</td>
        <td>Does it stick?</td>
      </tr>
      <tr>
        <td>Recording threshold</td>
        <td>Boundary at which weak crystallizes into strong</td>
        <td>When does sub-recording attention become tracked?</td>
      </tr>
      <tr>
        <td>Decay (forgetting)</td>
        <td>Drift away from active manifold region without reinforcement</td>
        <td>When does this become "no longer relevant"?</td>
      </tr>
      <tr>
        <td>Deformation (forgetting)</td>
        <td>Topological change after correction/supersession</td>
        <td>When does this become "no longer true"?</td>
      </tr>
      <tr>
        <td>Folding (forgetting)</td>
        <td>Moved to separate sheet, accessible only by deliberate time-travel</td>
        <td>What is "preserved but not present"?</td>
      </tr>
      <tr>
        <td>Scarification</td>
        <td>Property attached to corrective deformations</td>
        <td>How does trace of past-wrongness inform future engagement?</td>
      </tr>
      <tr>
        <td>Aggregation</td>
        <td>Perspective-act of inclusion/exclusion producing a new entity</td>
        <td>How does selection produce new addressable things?</td>
      </tr>
      <tr>
        <td>Projection</td>
        <td>Lens-enacted view at query-time</td>
        <td>How does a perspective surface what matters?</td>
      </tr>
    </tbody>
  </table>
</Section>

<Section label="Stratification">
  <h2>Stratification</h2>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Layer</th>
        <th>Description</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Substrate</td>
        <td>Totality of recorded events (all topics), append-only. Not "objective data"; the perspective-laden record of past emission acts.</td>
      </tr>
      <tr>
        <td>Information</td>
        <td>What happens when one perspective engages with events emitted by other perspectives (including past versions of itself).</td>
      </tr>
      <tr>
        <td>Knowledge</td>
        <td>Relational outcome of engagement — never stored, always produced.</td>
      </tr>
    </tbody>
  </table>
</Section>

<Section label="Accountability">
  <h2>Accountability vectors</h2>
  <p>
    Accountability decomposes into distinguishable vectors, not types. A
    single event may have all of them pointing at different entities.
  </p>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Vector</th>
        <th>What it captures</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Emission</td>
        <td>Discrete entity whose act produced the event; the formally traceable one</td>
      </tr>
      <tr>
        <td>Attribution-by-contribution</td>
        <td>Weighted across contributors based on what they contributed; never computed by the substrate</td>
      </tr>
      <tr>
        <td>Authority</td>
        <td>Power-relations that shaped the emission; has declared and emergent strata</td>
      </tr>
      <tr>
        <td>On-behalf-of chains</td>
        <td>Scopes within which the emission occurred; may chain across delegation kinds</td>
      </tr>
    </tbody>
  </table>
</Section>

<Section label="Chain links">
  <h2>Chain link kinds (within on-behalf-of chains)</h2>
  <table class="theory-table">
    <thead>
      <tr>
        <th>Kind</th>
        <th>Example</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Technical delegation</td>
        <td>Agent invokes another agent for a subtask</td>
      </tr>
      <tr>
        <td>Instrumental delegation</td>
        <td>Human deploys agent with instructions</td>
      </tr>
      <tr>
        <td>Representational membership</td>
        <td>Human acts as part of an aggregate</td>
      </tr>
      <tr>
        <td>Organizational authority</td>
        <td>Aggregate operates under another aggregate's scope</td>
      </tr>
    </tbody>
  </table>
</Section>

<Section label="Resolved stances">
  <h2>Resolved <em>stances</em></h2>
  <p>The schema's load-bearing commitments. Stable enough to build against.</p>
  <ul>
    <li>Attention is the teleological anchor; design constraints derive from it.</li>
    <li>Events-as-primary; the substrate IS the event history.</li>
    <li>Append-only ledger: no retroactive mutation; everything else is derivable.</li>
    <li>Cross-cutting point-in-time truth: for any event at time T, all associated entities' states at T are projectable from the trajectory of state-change events.</li>
    <li>Every event has an emitter and a perspective; perspective is derivable from emitter-trajectory at event-time, not denormalized.</li>
    <li>Emitter is discrete-non-aggregate.</li>
    <li>Aggregate-perspectives have positions and trajectories but do not emit.</li>
    <li>On-behalf-of is the bridge primitive between discrete accountability and aggregate scope.</li>
    <li>Observation is an event-topic (observing-entity-scoped, terminal, no implied commitment); audit happens at projection, not as further observation-emissions.</li>
    <li>Operational commitments (ack/rej/act) are secondary emissions referencing observation-events; receipt does not imply commitment.</li>
    <li>Emissions add geometry; observations reinforce geometry; corrective deformations scar geometry — bidirectional coupling differentiated by topic-class.</li>
    <li>Entity state is derived from state-change events, not stored as denormalized state.</li>
    <li>Capture richly, project plurally: substrate carries consistent metadata; weighting/scoring lives in projections.</li>
    <li>System surfaces, does not resolve: translation, attribution, accountability, disagreement.</li>
    <li>Resource-graph and event-graph are non-isomorphic derived structures.</li>
    <li>Atomicity is observer-relative, not absolute.</li>
    <li>A single substrate-emission can produce multiple observation-events (one source event; one observation-event per engaging observer).</li>
    <li>Affordance-composition over typed inheritance; topic carries payload-schema affordances.</li>
    <li>Authority has declared (formal) and emergent (practical) strata; may diverge; both projectable.</li>
    <li>Weak observer-relativity: shared substrate, observer-specific projections; shared understanding emerges from convergent histories.</li>
    <li>Knowledge is relational; the system stores conditions for knowledge production, not knowledge.</li>
    <li>Internal multitudes of perspectives are a commitment-about-what-we-don't-model; substrate records emissions, not perspective-states.</li>
  </ul>
</Section>

<Section label="What is moving">
  <h2>What's still <em>moving</em></h2>
  <p>
    Two classes of unsettled material live at
    <a href="/theory/open-questions#schema">/theory/open-questions#schema</a>:
  </p>
  <ul>
    <li>
      <strong>Intentionally open</strong> — questions the model
      deliberately does not specify. Downstream system designs answer
      these; the schema does not. Examples: storage substrate, query
      language, authority model, persona/role library.
    </li>
    <li>
      <strong>Pending opinionated stance</strong> — gaps the schema
      needs to close before it is fully coherent. Each wants pressure
      before stabilizing. Examples: minimum-viable state-change event
      schema, whether on-behalf-of chains nest on the emission or as a
      separate graph, scar decay mechanics, manifold composition.
    </li>
  </ul>
  <p>
    The boundary between <em>intentionally open</em> and <em>pending
    opinionated stance</em> will shift as the work continues. See
    <a href="/theory/open-questions#schema">/theory/open-questions#schema</a>
    for the live lists.
  </p>
</Section>

</div>

<style>
  .hero {
    min-height: 50vh;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    text-align: center;
    padding: 5rem 2.5rem 2rem;
  }
  .hero-label { margin-bottom: 1.5rem; }
  .hero h1 { margin-bottom: 1.5rem; }
  .tagline { max-width: 36em; }
</style>
