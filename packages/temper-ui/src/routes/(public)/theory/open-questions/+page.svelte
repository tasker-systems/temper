<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
</script>

<svelte:head>
  <title>Open questions — temper</title>
  <meta
    name="description"
    content="Questions the model and its schema have deliberately not resolved. Items move off this page as they resolve, into the appropriate /theory or /theory/schema section."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/theory/open-questions</div>
  <h1 class="t-hero-title">Open <em>questions</em></h1>
  <p class="tagline t-tagline">
    Questions the model and its schema have deliberately not resolved.
  </p>
</section>

<div class="theory-page">

<Section label="About this page">
  <p>
    This page gathers the questions the model and its schema have
    deliberately not resolved. Items move off this page as they resolve,
    with the resolution landing in the appropriate <a href="/theory">/theory</a>
    or <a href="/theory/schema">/theory/schema</a> section.
  </p>
  <p>
    The two sections below correspond to the two source documents. The
    boundary between them is real but soft — a question about field
    sub-typing in the model corresponds to a question about field-class
    typing in the schema; resolving one tends to resolve the other. Some
    items appear in both sections deliberately; resolving the schema-level
    version typically resolves the model-level version (and vice versa).
    Mirrored items are marked inline.
  </p>
</Section>

<section id="model">
  <Section label="Model">
    <h2>Open questions about the <em>model</em> itself</h2>
    <p>
      These are not engineering questions. They concern whether the model
      itself is complete and coherent.
    </p>
    <ul>
      <li>
        <strong>Is <em>field</em> too undifferentiated?</strong> Goals,
        decisions-in-force, constraints, and tolerances all behave somewhat
        differently as fields. They may be subspecies sharing mechanics, or
        they may genuinely be a single primitive at this level.
      </li>
      <li>
        <strong>What is the relationship between projection and resource?</strong>
        A document is in some sense a projection of past stream activity,
        written down. A search result is a projection of current field
        configuration. The asymmetry is suggestive but not yet pinned down.
      </li>
      <li>
        <strong>Are manifolds composable?</strong> Cross-project, cross-team,
        cross-owner reference is an open question. The model as stated
        allows for it but does not prescribe the composition mechanic.
      </li>
      <li>
        <strong>How do scars themselves decay?</strong> A scar near a region
        that hasn't been engaged in a long time should presumably fade with
        the region. But heavy scars (catastrophic past errors) may resist
        decay. The mechanic is not specified.
      </li>
      <li>
        <strong>What is the right granularity of perspective?</strong> A
        single individual is one perspective; a team is arguably another;
        an organization yet another. Whether perspectives compose
        hierarchically, federatively, or both, is open.
      </li>
      <li>
        <strong>How are role-personas themselves authored and evolved?</strong>
        They are priors used for cold-start; how they originate and update
        at the role-class level (rather than the individual level) is not
        in the model.
      </li>
      <li>
        <strong>What is the model's account of trust?</strong> If knowledge
        isn't stored, then trust isn't trust-in-information; it's
        trust-in-perspective-to-produce-reliable-information-from-data-in-region.
        The shape of this is implied but not yet developed.
      </li>
    </ul>
  </Section>
</section>

<section id="schema">
  <Section label="Schema">
    <h2>Schema-level <em>unsettled</em> material</h2>
    <p>
      The framing schema distinguishes two flavors of openness. Both live
      here.
    </p>

    <h3>Intentionally open (downstream design)</h3>
    <p>
      These are deliberately not specified by the model; they are answered
      by particular system designs.
    </p>
    <ul>
      <li>
        Storage substrate (single-node, distributed, ledger-flavored,
        CRDT — events-as-primary required, particular implementation not).
      </li>
      <li>Specific implementation of as-of queries and trajectory projection.</li>
      <li>Query language and retrieval mechanics.</li>
      <li>Specific aboutness computation.</li>
      <li>Field declaration, weighting, and evolution mechanics.</li>
      <li>Authority model for who may emit which topic-classes.</li>
      <li>Persona/role library specifics for any organization.</li>
      <li>
        Whether manifold is global, per-owner, per-project, or composable
        across scopes.
      </li>
      <li>
        Scar-informed engagement policy (source-agnostic at model level,
        source-aware at policy level).
      </li>
      <li>
        Topic taxonomy management — how new topics are introduced and
        payload-schemas evolve.
      </li>
      <li>Visual/interaction surface design.</li>
    </ul>

    <h3>Pending opinionated stance</h3>
    <p>
      Gaps the schema needs to close before it is fully coherent. Each
      wants pressure before stabilizing.
    </p>
    <ul>
      <li>
        Minimum-viable core schema for state-change events (what is the
        irreducible structure of a role-change, position-update, or
        membership-change event?).
      </li>
      <li>
        Whether on-behalf-of chains nest as a property on the emission or
        as a separate graph between aggregates (instinct: separate graph).
      </li>
      <li>
        How scars themselves decay; do heavy historical scars resist decay
        differently? <em>(mirrors <a href="#model">#model</a>.)</em>
      </li>
      <li>
        Whether manifolds compose hierarchically, federatively, or both.
        <em>(mirrors <a href="#model">#model</a>.)</em>
      </li>
      <li>
        How role-personas at the class level originate and update.
        <em>(mirrors <a href="#model">#model</a>.)</em>
      </li>
      <li>
        Caching tradeoffs — when, if ever, derived structure must be
        maintained rather than computed, and what reconciliation looks
        like.
      </li>
      <li>
        Whether the field primitive needs sub-typing (goals vs.
        decisions-in-force vs. constraints vs. tolerances).
        <em>(mirrors <a href="#model">#model</a>.)</em>
      </li>
      <li>
        The right granularity for attribution-judgment payload — the
        schema commits to attribution-by-contribution as a vector, but
        payload granularity remains open.
      </li>
      <li>
        The mechanic for practice-emergent aggregate-perspectives (those
        without a discrete forming-event).
      </li>
      <li>
        Aggregates-of-aggregates: different in kind from first-order
        aggregates, or only in degree?
      </li>
    </ul>
  </Section>
</section>

<Section label="A note">
  <p>
    This page exists for two reasons. First, the manifesto's commitment to
    making perspective-differences visible applies recursively to the docs
    themselves — a documentation surface that hides what is unsettled
    forces attention to be spent re-discovering the gaps. Second, the
    items here are work-in-progress. Some will resolve into the model and
    schema pages. Some will move from <em>intentionally open</em> to
    <em>pending opinionated stance</em> or vice versa. Some will turn out
    to dissolve under closer reading rather than resolve.
  </p>
  <p>
    The list will be wrong in different ways over time. That is the point.
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
