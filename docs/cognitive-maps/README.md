# `/cognitive-maps` — page set (markdown source)

This directory is the **markdown source** for a new `/cognitive-maps` tier on
`temperkb.io`, sibling to the shipped `/theory` tier. It is a **handoff artifact**: a
**successor task** translates these pages into SvelteKit routes in `packages/temper-ui`
at `/cognitive-maps/<slug>`. Nothing here is a finished SVG — every visualization is a
**placeholder note** (see the convention below) describing what the visual must show and
its honest basis in the schema artifact.

## The north star

Every page hangs off one sentence — the bridge from `/theory` (which ends on *"the system
does not store knowledge"*) into `/cognitive-maps` (which answers *"so what does it do,
concretely"*):

> Temper is an event-sourced coordination substrate whose organizing purpose is to be
> economical with attention. A cognitive map is a telos-seeded region of that substrate
> where humans and agents grow a shared, situated understanding together — and everything
> else is a projection over it.

## Audience & register

Written for **collaborators being invited to co-develop**, not builders being onboarded.
Precision ceiling is **conceptual + illustrative**; column-precise ERDs and
higher-precision workflows are deferred to *later* child pages.

**The genre split is structural — do not flatten it:**

- **Pages 1–6 *show* from the schema outward.** "Here is a thing whose shape is proven by
  the data model; help us finish building it." The visuals are *evidence*.
- **Page 7 *invites* from operations inward.** "This has to be run somewhere, that's a
  good problem, and the exhaust from running it is itself a payoff." It argues toward
  decisions *not yet made*.

**Reflexive discipline.** These pages respect the reader's attention the way the system
does: lead with what carries the most weight, mark open seams *as* seams, keep it terse. The
documentation is itself an argument for the thesis.

**Partner, not lecturer.** The reader is an informed peer we're inviting to co-develop —
not a novice with wrong ideas to correct, and not a student to be taught by authority.
Stay confident *about the subject* and peer-level *toward the reader*. Concretely: no
imperatives aimed at the reader's mind — drop "hold this," "notice," "unlearn," "ask not,"
"meet the…," "the first thing to understand." We arrived at these ideas by working through
them, so share *what we found and why it mattered to us* ("the distinction that did the
most work for us was…") rather than telling the reader what they currently believe or what
to do with their attention. A useful test: if a sentence assumes the reader holds a
mistaken model, or instructs them where to point their mind, rewrite it as a thing *we*
discovered. The claims about the system stay crisp; only the posture toward the reader
changes.

**Name the thing, don't point.** Web pages have no inherent order — a reader can arrive
anywhere. In prose, and *especially* in callouts, never reference another page by number
("see page 5") or assume sequential reading. Name the concept and link it
(`[how maps relate](05-how-maps-relate.md)`). Cross-references are by title/concept, never
by ordinal. The page manifest's numbering is a build-order convenience; the prose must not
lean on it. (A within-page "this page" or "the opening" is fine — that's not an ordinal
pointer.)

**No internal jargon in public prose.** Developer shorthand for crate boundaries and spec
bookkeeping — `Domain-A` / `Domain-B`, lean/decision IDs, `OQ-3`, `CS-1`, "the A2
invariant" — are coding that's only useful while building. They make the reader learn the
system's internal vocabulary to follow an argument. Express the *intention* instead: a
convention-agnostic kernel with patterns modelled expressly atop it, so the model reaches
beyond the early scenario while convention guides the common cases. If a term only makes
sense to someone who built it, say what it *does*. (Real schema names — `kb_events`,
`cogmap_genesis`, `resources_visible_to` — are fair game: they're the artifact the reader
is being invited to help finish, not internal shorthand.)

**Vary the rhetoric.** Watch for argumentative tics repeating across the set —
"load-bearing," "does real work," "it's worth…," "the point," "the trick," "earns its
keep." Any one is fine in isolation; clustered across a narrow corpus they read as a tell
that one rhetorical tool got over-used. Reach for a different one.

**Journey-first, what-it-does.** Open every page *inside a scenario from the seed* — a
concrete moment of use (a week-one engineer, a steward writing a lesson down, two teams
circling the same problem, three people running the same query) — and let the mechanics fall
out of the need. Lead with what a thing *does*; name it at the threshold of communicability,
as a handle for the tool we've just watched work, not as an ontology asserted up front. The
definitional content still appears — it just arrives as the shape of something already
in-use (concepts are ready-to-hand tools, not categories to be defined before use). Test: if
a section opens by defining a noun before the reader knows why they'd reach for it, re-open
it on the use and let the definition emerge. The whole set is an *engaging walkthrough of
personas and scenarios* through which the architecture and data design are shown working — not
a legend studied before the journey begins.

**Operating questions are organization-shaped.** For the *invite* arc (operating Temper),
don't frame deployment / governance / observability / insights as single questions the
project will answer once. Separate what the architecture **fixes** (invariant across
deployments — event-primary, the convention-agnostic kernel, teams-RBAC over homed
boundaries, actors-as-entities, the shared event shape, administration-is-event-sourced)
from what a **deployment shapes** (a *range* from minimum to maximum — topology, tenancy,
per-tenant integration, agent infrastructure, observability scope, governance surface,
the insights you pursue). The org-shaped answers vary between organizations and evolve over time;
anchor them with **temperkb.io** as one concrete, near-minimal reference point (Vercel
functions / Neon / single-tenant / platform agents), the way the seed scenario anchors the
show pages. State settled commitments plainly rather than leaving them as open forks — but scope
them to what ships, and name the mechanism rather than the aspiration. E.g. *access grants are
event-sourced and readable* (the grant pair, not yet the whole administrative surface),
*firewalled from cognition by construction* (admin events carry **no producing anchor**, and a
database constraint refuses one that does — which is precisely what keeps them out of cogmaps /
subscriptions / relationships; governance is traceable but not knowledge), and *bounded at the
persistence layer* (direct Postgres commands fall below the ledger — a system-responsibility
boundary, not a gap).

## The threaded seed (learn the cast once)

There is **one running example** — the seed scenario in
[`schema-artifact/03_seed.sql`](../../schema-artifact/03_seed.sql). The reader learns it
once and re-meets it at each altitude. **Do not introduce a second example.**

- **People:** **alice** ∈ epd-team-a · **bob** ∈ epd-team-b · **dave** ∈ org-common
  (maintainer) · **carol** ∈ directors (owner) · **sysadmin** (admin) · **nomad** (no
  teams, sees nothing).
- **Teams DAG:** `temper-system` root → {org-common, epd-department, directors};
  `epd-team-a` & `epd-team-b` each descend from {epd-department, org-common}.
- **Cogmaps:** system-default (root) · bridge-map (team-a ∩ team-b) · side-map (team-a) ·
  directors-map · **onboarding-cogmap** (`cogmap_genesis`-seeded; joined to org-common;
  telos = *"help a new EPD engineer reach first-merge confidence in week one"*; three
  guiding questions incl. *"where are the sharp edges that scar newcomers?"*; regulation =
  *"pair on the first PR"*; agent = `onboarding-agent#1`, claude-opus-4-8, steward).

**Connective tissue:** the *what-is / substrate / what-lives / how-it-grows* pages lean on
**onboarding-cogmap**; the *how-maps-relate / what's-visible* pages lean on the **team-a /
team-b / directors** access structure. The thread holds because onboarding-cogmap *is
joined to org-common* — *how maps relate* hands to *what's visible* and *operating Temper*
via "onboarding-cogmap's reach is itself one of these intersections." The conceptual pages
may breathe
abstractly; access pages (5, 6) carry the scenario most heavily.

## Page manifest

| # | File | Claim (one line) | Register | Hero visual |
|---|------|------------------|----------|-------------|
| 1 | [`01-what-a-cognitive-map-is.md`](01-what-a-cognitive-map-is.md) | A telos-seeded incubation home; no inside/outside. | Evocative, conceptual — the emotional entry point. | Cluster / region field (the honest `kb_cogmap_regions` picture). |
| 2 | `02-the-substrate-beneath-it.md` | Events are primary; everything else is a projection; the kernel stays convention-agnostic. | Foundational — "this is the commitment." | System-architecture SVG (ledger spine + projections). |
| 3 | `03-what-lives-in-a-map.md` | Resource vs. content-block vs. charter vs. regulation. | Concrete — "what data drives this." | Introductory ERD + `cogmap_genesis` step diagram. |
| 4 | `04-how-a-map-grows.md` | The five learning-acts mapped to real mechanisms; agents as personas, actor is an entity. | Concrete, grounded in the seed. | Triage / learning workflow. |
| 5 | `05-how-maps-relate.md` | Translation is irreducible; three modes without porosity — shape, delegation, and gated/curated promotion across scopes. | Concrete — the why and the how. | Two maps, legible-partiality boundary; **+** a promotion / send-forward diagram. |
| 6 | `06-whats-visible-from-here.md` | Visibility = permission × precedence — two orthogonal answers. | Precise — the page a technical collaborator will most poke at. | Seed DAG + the two axes; a two-gates panel. |
| 7 | `07-operating-temper.md` | The map had to be *stood up* somewhere — what the architecture fixes vs. what a deployment shapes; temperkb.io as one near-minimal point on the range. | Invitation, answerable-but-open. | (Hub; children carry the visuals.) |
| 7a | `07a-deployment.md` | 0→1 is the invariant seed; topology / tenancy / per-tenant integration / agent infra are the org-shaped range. | Answerable, not answered. | The 0→1→N path + the deployment-shape range around it. |
| 7b | `07b-governance-and-administration.md` | Authoring (built) vs. an org-shaped admin surface; administration **is** event-sourced (settled), firewalled from cognition, bounded at Postgres. | Authoring vs. administration. | Authoring, the admin dial, and the firewalled compliance-audit stream. |
| 7c | `07c-observability-and-audit.md` | Two kinds of audit, different homes; metric scope is an org call; the Postgres responsibility boundary. | Settled mechanism, open scoping. | The audits and their homes (operational outside; epistemic + governance inside). |
| 7d | `07d-insights.md` | Cross-system correlated reasoning-provenance (cognitive, not governance); what to ask of it is org-shaped. The forward-exciting close. | Look-what-becomes-possible. | The correlated reasoning-provenance chain. |

## Visualization placeholder convention

Every visual is a clearly-marked block, not an SVG. The successor task maps each block to
a `<VizPlaceholder>` component (or a real asset once drawn). Format:

> **▣ VISUALIZATION PLACEHOLDER — `<SLOT>` · `<kind>`**
> **Shows —** what the reader must come away seeing.
> **Honest basis —** the exact tables / functions / scenario it draws from, in
> [`schema-artifact/`](../../schema-artifact/). This is the line that keeps the visual
> truthful: it must depict what the artifact actually does, no walled gardens, no
> invented affordances.
> **Fidelity —** `conceptual` or `illustrative` (never column-precise in this task).
> **For successor —** any layout/interaction guidance for the SvelteKit translation.

`<SLOT>` is `HERO` for the page's anchor visual, `INLINE` for supporting ones.

## Scrubbed — never reintroduce

These were **superseded**; reconstructing them is the exact failure mode Temper exists to
dissolve (superseded thinking staying equally findable, carrying no scar, no fold):

- **`porosity`** / the permeable-surface "three tiers" / any **inside/outside/membrane**
  framing of a cognitive map.

A cogmap has **no inside/outside** — it is a telos-seeded incubation home. Visibility is
**teams:RBAC**; cross-map relation is **shape-projection + delegation**.

## Provenance

- **Ground-truth artifact:** [`schema-artifact/`](../../schema-artifact/) — `01_schema`,
  `02_functions`, `03_seed`, `04_scenarios`, `README` (the historical pre-collapse
  artifact namespace; the Arc-1 destination shape, empirically loadable).
- **Marshaling session (storyline, register, the threaded seed):** vault session
  `2026-06-04-marshaling-the-cognitive-maps-page-set-storyline-register-and-the-threaded-seed`.
- **Grounded Arc-1 specs:** [`docs/superpowers/specs/`](../superpowers/specs/) — the six
  2026-06-0{1..4} design docs.
- **Sibling tier (voice/structure model; cross-link pass is a *separate* later task):**
  the shipped `/theory` pages in `packages/temper-ui/src/routes/(public)/theory/`.

## Deliberate non-goals (this task)

No SQL signatures; no column-precise ERDs; no exhaustive event-family enumeration. No
relitigating `/theory` and **no cross-link pass back to it** (a separate later pass). Do
not reintroduce the scrubbed concepts above.
