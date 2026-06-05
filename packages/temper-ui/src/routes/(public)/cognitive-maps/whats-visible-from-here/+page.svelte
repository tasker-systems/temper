<script lang="ts">
  import Section from '$lib/components/landing/Section.svelte';
  import VizFigure from '$lib/components/VizFigure.svelte';
  import SeedDagDiagram from '$lib/components/cognitive-maps/diagrams/SeedDagDiagram.svelte';
  import TwoGatesDiagram from '$lib/components/cognitive-maps/diagrams/TwoGatesDiagram.svelte';
</script>

<svelte:head>
  <title>What's visible from here — temper</title>
  <meta
    name="description"
    content="Visibility = permission × precedence — two orthogonal answers the page keeps apart. Two kinds of reader (a person vs. a map), down-DAG inheritance, leak-safety, edge-home protection; then salience, where the attention-economy lives."
  />
</svelte:head>

<section class="hero">
  <div class="hero-label t-label">/cognitive-maps/whats-visible-from-here</div>
  <h1 class="t-hero-title">What&#39;s <em>visible</em> from here</h1>
  <p class="tagline t-tagline">
    Permission and precedence — two orthogonal answers.
  </p>
</section>

<div class="cognitive-maps-page">

<blockquote class="epigraph">
  Alice, Bob, and Nomad each ask the same thing — <em>what can I see here?</em> —
  and get three different answers. Same system, same question, three different
  worlds. The reason isn't one rule but two, and they're easy to mistake for each
  other: what you may see, and — within that — what ranks.
</blockquote>

<VizFigure placement="HERO" fidelity="illustrative">
  {#snippet diagram()}
    <SeedDagDiagram id="cm-dag" />
  {/snippet}
  {#snippet shows()}
    the full seed world: six teams as a DAG (<code>temper-system</code> root →
    {'{'}org-common, epd-department, directors{'}'}; team-a and team-b each
    descending from epd-department <em>and</em> org-common), and the five cogmaps
    joined to their teams. Overlaid, the <strong>two axes</strong> as two distinct
    reading-motions: the <strong>consumer axis</strong> as a <em>person</em>
    reading <strong>down</strong> the DAG (grants inheriting downward), and the
    <strong>producer axis</strong> as a <em>map</em> reading the
    <strong>intersection</strong> of its joined teams (highlight bridge-map =
    team-a ∩ team-b shrinking the reachable set). The reader should see one
    structure read two different ways.
  {/snippet}
  {#snippet honestBasis()}
    <code>resources_visible_to</code>, <code>vis_team</code>,
    <code>resources_accessible_to_cogmap</code> over the teams DAG
    (<code>kb_teams_parents</code>) and the five cogmaps; verdicts
    <strong>S1</strong> (consumer) and <strong>S2</strong> (producer /
    leak-safety).
  {/snippet}
</VizFigure>

<Section label="Three people, three worlds">
  <p>
    Put the same question to three of the cast: <em>what can I see?</em>
  </p>
  <p>
    <strong>Alice</strong>, on team-a, sees her team's working docs, the org-wide
    policy every team shares, a document shared with her by name, and the public
    concepts on the floor. <strong>Bob</strong>, on team-b, sees the org policy
    and that same public floor — but not team-a's private work; he was never
    granted it. <strong>Nomad</strong>, enabled for nothing and owning nothing,
    sees <em>nothing at all.</em>
  </p>
  <p>
    Same system, same query, three different worlds. And once you ask <em>why</em>
    each set looks the way it does, a second question separates out from the
    first: not only <em>what may I see</em>, but — of the things I may see —
    <em>what comes up first.</em> Those are different machinery, and most
    confusion about "what an agent can see" comes from running them together.
  </p>
</Section>

<Section label="Permission × precedence">
  <p>
    So: two orthogonal answers. <strong>Permission</strong> decides what's in your
    set at all — the hard, structural yes/no that makes Alice's world and Bob's
    differ. <strong>Precedence</strong> decides, within that set, what surfaces
    first — a graded, earned ranking. Permission comes first here (it decides
    what's even possible); precedence closes (it's where the attention-economy
    lives).
  </p>
</Section>

<Section label="Two kinds of reader">
  <p>
    Two very different things read from the substrate, for different reasons — and
    the access question has a different answer for each.
  </p>
  <p>
    A <strong>person</strong> reads to <em>use</em> what's there: Alice opening
    docs, Bob checking the policy. This is the <strong>consumer</strong> side —
    what you may read follows from who you are, your team memberships and anything
    granted directly to you, inherited <em>down</em> the teams DAG. It's the
    function behind the three worlds above: <code>resources_visible_to(you)</code>.
  </p>
  <p>
    An <strong>agent</strong> reads to <em>do a map's work</em>: the onboarding
    steward growing its map, materializing its shape, deciding what to write into
    regulation. This is the <strong>producer</strong> side, and the move that
    matters is that a producing agent reads as the <strong>map</strong>, not as
    whoever launched it — with reach
    <code>resources_accessible_to_cogmap(map)</code>, the least-privilege
    <em>intersection</em> of the map's joined teams (why it's an intersection is
    the next thing).
  </p>
  <p>
    Tying the agent's reach to the map rather than to its launcher closes a real
    risk. If a producing agent inherited the access of the person who started it,
    you could launch one inside a narrowly-scoped map from a broadly-privileged
    account and quietly let that map pull in — and be shaped by — material it has
    no business touching. So every substrate read carries exactly
    <strong>one principal</strong>: one identity behind the read, a person
    <em>or</em> a map, never both. And it holds structurally, not as a rule
    someone has to remember — the read function has no parameter for <em>the human
    behind the agent</em>, so the map's agent sees what the map may see, and who
    happens to be at the keyboard can't enter the question at all.
  </p>
</Section>

<Section label="More teams, narrower reach">
  <p>
    The producer axis is an <strong>intersection</strong>, and that inverts the
    usual intuition. A map joined to <code>epd-team-a</code> <em>and</em>
    <code>epd-team-b</code> can read only their <strong>common ground</strong> — a
    resource has to be visible to <em>every</em> joined team to be in reach. So
    <code>team-a-private</code> is in side-map's reach (joined to team-a alone) but
    <strong>falls out</strong> of bridge-map's reach (a∩b), even though bridge-map
    "has" team-a. More teams means <em>narrower</em>, not wider.
  </p>
  <p>
    The empty case follows the same logic to its end: a map joined to
    <strong>no</strong> team has an empty intersection and reads
    <strong>nothing</strong> shared — default-<strong>CLOSED</strong>, not
    default-open. (Its own homed resources stay readable; that's map-home, not a
    grant.) Scenario <strong>S2</strong> runs exactly this.
  </p>
</Section>

<Section label="Down-only inheritance, and the root floor">
  <p>
    On the consumer side, grants inherit <strong>down</strong> the DAG: a grant on
    a team is readable by that team and everything descending from it, never
    upward into a parent's privates. Every team descends from the
    <code>temper-system</code> root, so a grant on root is a
    <strong>universal floor</strong> — the public baseline everyone with access
    shares.
  </p>
  <p>
    "With access" is itself a real membership, not a special case: enabling a
    profile auto-joins it to the root team via a maintained
    <code>kb_team_members</code> row, so the floor falls out of the same mechanism
    as every other grant. Scenario <strong>S1</strong> is the proof — alice
    (team-a) and bob (team-b) see overlapping-but-different sets;
    <strong>nomad</strong>, enabled for nothing, owning nothing, sees
    <em>nothing at all</em>.
  </p>
</Section>

<Section label="Leak-safety">
  <p>
    One invariant makes the whole thing safe to extend to per-person grants: a
    <strong>profile-anchored grant is consumer-axis only</strong>. It can let a
    <em>person</em> read a specific resource, and it <strong>never</strong> enters
    a team's visibility set or a producer intersection. So
    <code>shared-with-alice</code> is readable by alice and is in
    <strong>no map's</strong> reach — a personal grant can't be laundered into a
    cogmap or leaked across a team boundary. That's what makes admitting profiles
    as grantees safe rather than a hole. <strong>S2</strong> checks this too.
  </p>
</Section>

<Section label="Edge-home protection">
  <p>
    An edge is access-gated like everything else: it's <em>homed</em>, and its
    home gates whether you can even see <em>that the assertion exists</em>. This
    produces a result that's easy to miss. The directors' <code>leads_to</code>
    edge runs between two <strong>public</strong> concepts — both endpoints
    readable by anyone — but the edge is homed in directors-map, so it's invisible
    to anyone who can't read that map. Both nodes visible, the link between them
    not. Scenario <strong>S3</strong>: carol (∈ directors) sees it; alice and
    nomad don't, <em>despite</em> alice being able to read both endpoints.
  </p>
</Section>

<!-- Second placeholder on the page: snippet names must be unique within the
     file (inline {#snippet shows()} would collide with the HERO's), so its
     content is declared as named snippets and passed as props. -->
{#snippet gatesShows()}
    the two gates that are <em>easiest to conflate</em>, side by side, as two
    differently-coloured arrows into the same map: (1)
    <strong>membership-based shape-readability</strong> —
    <code>cogmap_readable_by_profile</code>, "are you a member of a team joined to
    this map" — which gates seeing the map's <em>shape</em>; and (2)
    <strong>down-DAG grant-inheritance</strong> —
    <code>resources_visible_to</code>, "has a resource been granted to a team you
    reach" — which gates reading <em>resources</em>. Different questions, different
    functions, different answers; the panel's whole job is to keep them from
    blurring.
{/snippet}
{#snippet gatesBasis()}
    <code>cogmap_readable_by_profile</code> vs.
    <code>resources_visible_to</code>; the edge-home traversal gate
    <code>edges_visible_to</code> and scenario <strong>S3</strong> for why the
    distinction bites.
{/snippet}
{#snippet gatesDiagram()}
    <TwoGatesDiagram id="cm-gates" />
{/snippet}
<VizFigure
  placement="INLINE"
  fidelity="conceptual"
  maxWidth="460px"
  diagram={gatesDiagram}
  shows={gatesShows}
  honestBasis={gatesBasis}
/>

<Section label="Salience: the other answer">
  <p>
    Everything above is permission — binary, structural, the same for everyone who
    shares your position. <strong>Salience</strong> is the orthogonal answer:
    within what you <em>may</em> see, what <em>ranks</em>. It's <code>weight</code>
    on edges and properties, <code>salience</code> on regions, reinforce-count on
    questions — all graded, all earned, all changing as the map learns.
  </p>
  <p>
    This is the attention-economy in operation. <em>What surfaces first is an
    attention decision, not a permission one.</em> Permission says you're allowed
    to see ten thousand things; salience is why you meet the three that matter
    under this telos first. Keeping the two apart matters because they fail
    differently: a permission bug leaks data, a salience bug wastes attention — and
    the system takes the second problem as seriously as the first.
  </p>
</Section>

<Section label="Runnable proof">
  <p>
    None of the verdicts on this page are assertions to take on trust. S1, S2, and
    S3 are labelled queries in <code>04_scenarios.sql</code> that run against the
    seed and print exactly the behaviour described — alice's set, bob's set,
    nomad's nothing, the intersection that drops team-a-private, the private edge
    carol sees and alice doesn't. The model is meant to be <em>checked</em>, which
    is exactly why the artifact gets built before the migration.
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
