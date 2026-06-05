<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import ShapeBoundaryDiagram from '$lib/components/cognitive-maps/diagrams/ShapeBoundaryDiagram.svelte';
  import PromotionDiagram from '$lib/components/cognitive-maps/diagrams/PromotionDiagram.svelte';
</script>

<svelte:head>
  <title>How maps relate — temper</title>
  <meta
    name="description"
    content="Two teams circling the same problem, unable to read each other's maps — and how a shared understanding still forms. Maps relate three ways without opening an interior: seeing that another cares (shape), borrowing its frame (delegation), and promoting a concept across scopes (a gated, curated send-forward)."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/how-maps-relate</div>
  <h1 class="t-hero-title">How maps <em>relate</em></h1>
  <p class="tagline t-tagline">
    Three ways to relate, none of them an open door.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  Two teams, working apart, are circling the same thing: how their sprints
  actually run. Neither can read the other's map — different scopes, different
  access. And yet a department-wide understanding of <em>good sprint process</em>
  obviously wants to exist, built from both. Maps relate three ways that make
  that possible without ever opening an interior: seeing that another cares,
  borrowing its frame to think with, and — when someone standing in both scopes
  chooses to — lifting a concept across into shared ground.
</blockquote>

<VizFigure placement="HERO" fidelity="conceptual">
  {#snippet diagram()}
    <ShapeBoundaryDiagram id="cm-shape" />
  {/snippet}
  {#snippet shows()}
    two cognitive maps (bridge-map and side-map) with a boundary between them.
    One map projects its <strong>shape</strong> across the boundary — regions
    with salience and a member-count blur — rendered as something the other map
    can <em>see</em>. The <strong>material interior</strong> (the actual member
    resources) stays drawn on the far side, behind the boundary, unreadable
    across it. The line the reader should feel: <em>"you can see that it cares,
    and roughly how, without seeing what."</em>
  {/snippet}
  {#snippet honestBasis()}
    the shape surface <code>cogmap_shape</code> (returns salience / label /
    member_count, never member identities) over <code>kb_cogmap_regions</code> /
    <code>kb_cogmap_region_members</code>; the shared-team relation
    <code>cogmaps_share_a_team</code>, demonstrated by scenario <strong>S5</strong>
    (bridge ∩ side = true; bridge ∩ directors = false); the materialized-shape
    read in <strong>S6</strong>.
  {/snippet}
</VizFigure>

<Section label="Two teams, one problem">
  <p>
    <code>epd-team-a</code>'s side-map and a map over on <code>epd-team-b</code>
    are, without coordinating, working the same question: how their sprints
    actually go — which rituals help, where they stall, what a healthy cadence
    feels like. Each is learning it under its own telos, inside its own scope.
    Neither can read the other's interior; the access simply isn't there.
  </p>
  <p>
    And yet the thing that <em>should</em> exist — a shared, department-level
    understanding of good sprint process, drawing on both teams' hard-won
    specifics — has nowhere to live. That gap is what this page is really about:
    <strong>how do two maps that can't read each other still come to inform, and
    eventually grow, one another?</strong>
  </p>
</Section>

<Section label="Why there's no shortcut">
  <p>
    The tempting answer — merge them into one canonical view — doesn't survive
    contact with what a map is. Every map is telos-seeded, and what matters under
    one telos isn't what matters under another. The two teams weight the same
    ritual differently and <em>both are right</em>, because weight is relative to
    purpose. There's no view from nowhere that reconciles them.
  </p>
  <p>
    So translation between maps is irreducible — not a gap waiting for a better
    merge algorithm. The real question isn't "how do we unify the maps," it's
    "how do they stay legible to each other, and pass understanding across,
    <em>without</em> a unified view." There are three answers, and none of them
    opens an interior.
  </p>
</Section>

<Section label="Seeing that another map cares">
  <p>
    The first is <strong>shape</strong>. A map materializes its own regions, and
    anyone who can read the map can read that shape: the regions, each with a
    salience and a <code>member_count</code> blur — the shape surface this set
    opened on. Across a boundary, that shape is <strong>salience made
    visible</strong>. One map can see <em>that</em> another cares about
    something, and roughly <em>how much</em>, without seeing <em>what</em> the
    caring is made of.
  </p>
  <p>
    So side-map(team-a) and the team-b map, which share a team and sit in the
    same world, can each see the other weighting a "sprint rituals" region
    heavily — while the material each map can actually <em>read</em> stays its
    own. Legible partiality, in one line: you get the outline and the weight, not
    the contents. Shape is a projection a map offers, not a peephole another map
    drills.
  </p>
</Section>

<Section label="Borrowing a frame">
  <p>
    The second is <strong>delegation</strong>, and it's deliberately narrow.
    <code>cogmaps_share_a_team</code> is true when two maps share at least one
    joined team. When they do, an agent working in one map may <strong>borrow the
    other's frame</strong> — its telos and its blurred shape — across a single
    bridge. That's frame-injection: enough to think <em>with</em> another map's
    purpose in view.
  </p>
  <p>
    What it is <strong>not</strong> is a material read. The resources an agent can
    actually open stay bound to <code>resources_accessible_to_cogmap</code> for
    the map it's producing in — strictly stronger than "we share a team." Sharing
    a team lets you borrow how another map <em>frames</em> a problem; it doesn't
    let you read what's inside it. Frame and contents come apart on purpose.
  </p>
</Section>

<!-- Snippets for the INLINE figure that sits mid-section below. Declared at
     page level (not as children of <Section>, which would bind them as that
     component's props) and given unique names so they don't collide with the
     HERO figure's inline diagram/shows/honestBasis. -->
{#snippet promotionDiagram()}
    <PromotionDiagram id="cm-promo" />
{/snippet}
{#snippet promotionShows()}
    three maps: side-map(team-a) and a team-b map below, an
    <code>epd-department</code> map above. Each lower map has a high-salience
    "sprint rituals" region; those salience markers, <em>visible across the
    boundary</em>, are drawn as the <strong>signal</strong> the department map
    reads. An actor who stands in both scopes (a person, badged as a member on
    each side) authors a <strong>send-forward event</strong> that seeds a new
    <em>good sprint process</em> concept in the department map and draws
    <strong>relationship edges</strong> back down to the two source concepts.
    Emphasis: the source interiors stay behind their boundaries (referenced, not
    copied); only the curated concept and its relations cross.
{/snippet}
{#snippet promotionBasis()}
    homed resources/edges (<code>kb_resource_homes</code>,
    <code>kb_edges.home_anchor_*</code>) and the team/cogmap join
    (<code>kb_team_cogmaps</code>, <code>cogmaps_share_a_team</code>); the
    cross-boundary salience signal is <code>cogmap_shape</code>
    (<strong>S6</strong>). The promotion <em>act</em> — the
    both-scopes-membership gate and the curated send-event — is a
    <strong>designed capability, not yet in the artifact</strong>; drawn as
    proposed.
{/snippet}

<Section label="Promoting a concept across scopes">
  <p>
    Shape and delegation let maps <em>see</em> and <em>think with</em> each other.
    Neither moves a concept from one map into another — and the two teams' sprint
    work has matured to where that's exactly what's needed. When a shared
    department concept <em>should</em> exist, something has to carry it across.
    That act is <strong>promotion</strong>.
  </p>
  <p>
    Promotion is deliberately not automatic, and it is not a leak. Because every
    resource and edge is <strong>homed</strong>, a concept in side-map(team-a)
    stays bound to its scope; nothing about team-b or the department gets to read
    it just because it would be useful. What happens instead is a
    <strong>send-forward</strong>: an intentional, curated event that seeds a
    concept into a broader-scoped map and records the relationship back to where
    it came from. Two things make it accountable rather than a back door:
  </p>
  <ul>
    <li>
      <strong>It's gated to an actor who stands in both scopes.</strong> Only
      someone who is a member of teams participating in <em>both</em> the
      send-from and the send-to map can promote — they already have legitimate
      standing on each side, so nothing crosses a boundary its mover couldn't
      already cross themselves. For now that actor is a human, by choice:
      promotion is where accountability matters most, so it stays on a person's
      judgment rather than automated.
    </li>
    <li>
      <strong>The mover curates the send-event.</strong> Promotion isn't a copy.
      The actor shapes what actually goes forward — the concept-content seeded
      into the broader map, and the graph relation drawn back to the origin — so
      the shared concept is <em>authored</em>, and the private interior it draws
      on stays put, <strong>referenced, not exposed.</strong>
    </li>
  </ul>
  <p>
    So a department-scoped concept <em>good sprint process</em> can come to exist
    — homed in the department, readable there — carrying relationships to the
    team-a and team-b concepts that informed it, without either team's interior
    being pulled across. The teams keep their boundaries; the shared
    understanding finally gets a home; and from there it can be grown by anyone in
    the department scope, mutually, where before it had nowhere to be.
  </p>
  <p>
    <strong>The signal that points the way.</strong> The neat part is that maps
    already tell you <em>where</em> promotion will pay off. Shape — the salience a
    map makes visible across a boundary — is a signal of what a map cares about
    and how much. When a department-scoped agent sees side-map(team-a) and the
    team-b map <em>both</em> weighting "sprint rituals" heavily, that mutual high
    salience is a flag: there's a generalizable concept here worth lifting.
    Shape-projection doesn't only describe relation; it points at where richer
    sharing is likely to repay someone's attention. Taken further, the same
    signals could propose <em>new</em> maps and teloi over time — when several
    maps keep circling the same ground, that's the system noticing an org-level
    purpose that doesn't yet have a map of its own.
  </p>

  <VizFigure
    placement="INLINE"
    fidelity="conceptual"
    diagram={promotionDiagram}
    shows={promotionShows}
    honestBasis={promotionBasis}
  />

  <p>
    Promotion is real in the data model — homed boundaries, an actor with standing
    on both sides, an event that seeds and relates — but the <em>protocol</em>
    around it is still open, deliberately. Who flags a concept as a candidate, and
    when? How far does an agent go on a salience signal — propose and leave the
    move to a person, or more? What exactly does "curate the send-event" put in
    the mover's hands? These are genuine forks, not settled mechanism, and they
    want real use before they're frozen. The capability is grounded; the
    choreography isn't drawn yet.
  </p>
</Section>

<Section label="The move we didn't make">
  <p>
    There's one move we deliberately <em>didn't</em> make, and saying which one
    clears up a lot — because an earlier version of this design did make it, and
    it was wrong. There's no permeability here: no surface bored with tiers of
    access, no membrane between an inside and an outside. The reason is upstream —
    <strong>a map has no interior to make porous.</strong> There's nothing to
    drill through. Relation between maps is shape, a shared-team predicate, and a
    deliberately-moved concept — all of them acts <em>over</em> the same RBAC that
    governs everything else, never a breach of it. Promotion is the cleanest case
    of it: concepts cross between scopes <strong>by intention, not by
    osmosis.</strong> The thing that made the porous-surface model tempting (maps
    need to relate) turned out to be fully satisfiable without inventing a new
    kind of boundary.
  </p>
</Section>

<Section label="Handing off to access">
  <p>
    This leaves the running example somewhere specific. The onboarding map is
    joined to org-common — so <em>its</em> reach is itself one of these team
    intersections, the same machinery side-map and bridge-map are made of. Which
    raises the question this thread kept leaning on and never quite answered:
    <em>what, exactly, decides who can read what.</em> That's what
    <a href="/cognitive-maps/whats-visible-from-here">what's visible from here</a>
    takes on.
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
