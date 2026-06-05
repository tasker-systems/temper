<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import AuditHomesDiagram from '$lib/components/cognitive-maps/diagrams/AuditHomesDiagram.svelte';
</script>

<svelte:head>
  <title>Observability &amp; audit — temper</title>
  <meta
    name="description"
    content="Is the system healthy, and what did it know and why did it change — two questions, two homes. Operational audit lives in external tooling and is scoped to an organization's needs; epistemic audit is the ledger itself. Known mechanism, organization-shaped scope, one responsibility boundary."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/operating-temper/observability-and-audit</div>
  <h1 class="t-hero-title">Observability &amp; <em>audit</em></h1>
  <p class="tagline t-tagline">
    Is it healthy? What did it know, and why did it change?
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  The onboarding agent woke, read a charter, and wrote a regulation. Two fair
  operator questions follow: is the system that did this <em>healthy</em> — and,
  separately, can we reconstruct <em>what it knew and why it changed its
  mind</em>? Those are two different kinds of audit, in two different homes. The
  mechanism for each is known; <em>what</em> to capture is the part your
  organization scopes.
</blockquote>

<Section label="Two questions that sound alike">
  <p>
    An operator watching Temper run has two kinds of question, easy to file
    together even though they want different tools.
  </p>
  <p>
    The first is <strong>operational</strong>: is it up, is it fast, is it
    erroring? Calls, latencies, failures — the ordinary health of a running
    service. The path is known. Temper already carries tracing, the patterns are
    well-worn OpenTelemetry tooling for observability, and the same approach
    extends to the CLI, which would give an organization cross-usage visibility
    into how Temper is actually used.
  </p>
  <p>
    The scope, though, is yours. <em>Which</em> metrics matter — what an
    organization watches, alerts on, and retains — is a decision that varies and
    evolves; Prometheus metrics are undefined here not because the mechanism is
    missing but because the choice is genuinely organizational. temperkb.io
    captures little of this today; a larger deployment captures far more. It's a
    low-risk place to contribute, and a dial each operator sets for themselves.
  </p>
  <p>
    The second question is <strong>epistemic</strong>: what did the system know,
    when, and why did a concept change? That one isn't a tooling question at all.
  </p>
</Section>

<Section label="The audit you get for free">
  <p>
    The epistemic audit is the ledger itself. Because every change is an event —
    every assertion, fold, reinforcement, and scar, each with its emitter and its
    place in a correlation thread — the question <em>"why does this concept look
    the way it does, and what shaped it"</em> is answered by reading the
    substrate, not by bolting on an audit log. The system that grows
    understanding and the system that records <em>how</em> understanding formed
    are the same system.
  </p>
  <p>
    There's a second audit on that same ledger, kept deliberately apart: the
    <strong>governance</strong> trail — who was granted what, and when.
    Administrative acts are events too, but they're compliance records,
    firewalled by design from the cognitive stream (the reasoning behind that
    separation is in
    <a href="/cognitive-maps/operating-temper/governance-and-administration"
      >governance &amp; administration</a
    >). So the inside-the-substrate audit is really two streams that don't mix:
    <em>how understanding formed</em>, and <em>who was allowed to do what</em>.
    Keeping them apart is what lets each answer its own question cleanly.
  </p>
  <p>
    So the audits live in distinct homes, and that division is the design:
    operational audit in external tooling, outside; epistemic and governance
    audit in the event ledger, inside. Running them together is the mistake;
    keeping them apart is what lets each be good at its job.
  </p>
</Section>

<VizFigure placement="INLINE" fidelity="conceptual">
  {#snippet diagram()}
    <AuditHomesDiagram id="cm-audit" />
  {/snippet}
  {#snippet shows()}
    three lanes around the system boundary. <strong>Operational</strong>
    (outside): a running Temper emitting traces and metrics — calls, latency,
    errors — into external tooling (OpenTelemetry / Prometheus), with a note that
    <em>which</em> metrics is an organization-scoped dial.
    <strong>Epistemic</strong> (inside): the event ledger as the trail of
    <em>how understanding formed</em> — assertions, folds, reinforcements, scars.
    <strong>Governance</strong> (inside, firewalled): administrative events —
    <em>who was granted what, when</em> — on the same ledger but in a separate
    compliance channel that does not feed the cognitive maps. A dashed line
    beneath everything marks the <strong>Postgres responsibility boundary</strong>,
    below which direct database commands fall outside the ledger entirely.
  {/snippet}
  {#snippet honestBasis()}
    the epistemic and governance streams are real on <code>kb_events</code>
    (<code>emitter_entity_id</code>, <code>correlation_id</code>, producing
    anchor) with the fold / provenance trail (<code>kb_block_provenance</code>,
    <code>is_folded</code>). The operational side (OpenTelemetry / Prometheus) is
    <strong>external tooling, not in the artifact</strong>.
  {/snippet}
</VizFigure>

<Section label="One non-goal we'd rather name">
  <p>
    There's a limit here and it's the same line governance draws from its side.
    Audit at the <strong>Postgres boundary</strong>, protecting against someone
    with direct database access, is out of scope on purpose. Anyone with admin
    access to the database can read everything, and at that point you've left what
    Temper's RBAC and ledger can speak to. Compliance for <em>that</em> threat is
    an <strong>extra-system</strong> concern — database-level controls,
    infrastructure policy — and how it's handled depends entirely on how your
    organization runs its data layer. Better to be clear about where the system's
    guarantees stop than to imply they reach further than they do.
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

  .epigraph {
    max-width: 800px;
    margin: 0 auto 1rem;
    padding: 0.5rem 2.5rem 0.5rem 3.75rem;
  }
  :global(.cognitive-maps-page .epigraph) {
    border-left: 2px solid var(--temper-blue-border);
    font-family: var(--font-serif);
    font-style: italic;
    color: var(--parchment);
    line-height: 1.7;
  }
</style>
