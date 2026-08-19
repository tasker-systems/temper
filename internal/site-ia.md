# Public surfaces — Information Architecture

This is the standing information architecture for everything Temper publishes. It records
*what each public surface is, why it is shaped that way, and what work each one still
requires* — the decisions-and-rationale, not the page copy. Page copy is downstream of this
document; each page is its own drafting pass.

**Status:** current. This document governs all three public surfaces going forward.

**Scope, widened `[2026-08-19]`.** This document governed `temperkb.io` alone until the docs
surface rebuild, which needed a governing document and had never had one. It now governs
three. Part I below is the shared frame — the surfaces, the rule that separates them, and the
audiences they serve. Surfaces 1–3 govern one surface each. Surface 1 is the original body of
this document, unchanged in substance; where it says "the public site" it means `temperkb.io`.

---

## The three surfaces, and the rule that separates them

Temper publishes three things. They are produced by different means, they answer different
questions, and the commonest failure is a page drifting into the surface next door. That is
not a style problem but a maintenance one: the copy that drifts is the copy nobody sees
corrected, because the surface it drifted into already has an owner for that material.

| Surface | Source | Answers | Register |
|---|---|---|---|
| **`temperkb.io`** | SvelteKit app in `packages/temper-ui` | **Why** — what the substrate is for, what a cognitive map is, what is fixed by the architecture versus what a deployment chooses | Argued. Persuades, grounds, situates. |
| **`docs/`** | markdown in this repo, published from `main` | **How** — install it, run it, integrate with it | Operational. Followed, not read. |
| **the API reference** | generated from `openapi.json` | **What exactly** — every endpoint, every schema | Machine-derived. Never authored. |

**The rule: why lives up, how lives across, exact lives below.**

- A `docs/` page that finds itself explaining *why the thing exists* should **link up** to
  `temperkb.io` and keep one sentence. `docs/` is the operational half; the conceptual half is
  one click away and is maintained by people writing prose rather than runbooks.
- A `temperkb.io` page that finds itself giving a command sequence should **link across** to
  `docs/`. The argued surface is the wrong home for a step that changes with a release.
- Neither ever restates an endpoint, a flag, a config field, or a schema. Those are
  **generated**, gated, and below both. A hand-written copy of generated content is a lie with
  a delay fuse.

The rule is directional, not a wall. A `docs/` page may absolutely carry the one paragraph of
concept a reader needs in order to act — what it must not become is the place that concept is
maintained.

---

## The audiences

Three, and only three. They were established for the docs surface and they hold across all of
it.

- **Individual user** — wants Temper working for them and their agents.
- **Deployer / self-hoster** — is standing up a deployment.
- **API consumer / integrator** — is writing code that talks to Temper.

**Contributors are not a public audience.** Material addressed to someone changing Temper's
own code lives in `internal/`, reachable through `CLAUDE.md` / `AGENTS.md`. This is not a tone
preference. A page opening *"if you are changing anything on an auth path…"* answers a
question no public reader asked, and it displaces the page they came for — and it reads as an
invitation into a codebase they have no plans to open.

Two consequences that are easy to get wrong:

- **Audience is declared, not implied.** Every authored public page says near the top who it
  is for. A reader landing from search needs to know within a sentence whether to keep going.
- **Audience is a navigation dial, never a hierarchy.** A page serves whichever audiences need
  it; the doors route, they do not own. This is the same three-dials discipline Surface 1 has
  held since this document governed one surface — navigation surface, URL hierarchy and
  entry-point status are independent.

---

# Surface 1 — `temperkb.io`

Everything from here to *Surface 2* governs the SvelteKit site. It is the original body of
this document.


## Scar: what this supersedes

This document **supersedes and replaces** `docs/theory-ia-proposal.md`, which was written
*before* the reorientation described below and is wrong on its core premises. That document
has been **removed from the working tree** in the same change that introduced this one; its
history remains reachable in git for anyone who wants the audit trail. It is folded, not
forgotten — preserved in history, absent from default projection — so it cannot be re-read
as if current and cannot clutter grep/agent context later.

What the old proposal got wrong, and why it had to go rather than be amended:

- It asserted the existing product site was **"not being replaced or rewritten,"** and that
  `/`, `/agents`, `/builders`, `/how-it-works` should stay **"exactly as they are… they
  frame Temper-as-product."** The flip reverses exactly this: the product framing is demoted
  to *one projection* over the substrate, not the headline.
- It placed `/theory` **"alongside"** the product site, explicitly *not competing with `/`
  for general traffic* — a peer tier bolted on. The flip makes the theory/substrate frame
  the *foundation the site rests on*, not a side annex.
- It specified `/docs` → `/using-temper` as a rename and recorded it as resolved — but the
  rename was **never executed** (the route is still `/docs`; no redirect exists). The flip is
  the moment to actually do it.

Because the old document's premises are inverted rather than merely incomplete, amending it
in place would have left a confusing half-corrected artifact. The honest move is supersession
with this scar carried forward here, where it will actually be read.

---

## The flip, stated precisely

Temper's public site was built in the **workflow-tool** frame: the *vault* is the product,
the organizing verb is *remember*, the reader is a solo builder wiring an agent into their
own knowledge base. Three strata accreted at three moments and no longer agree with each
other: the product surface (oldest), a shipped-but-unlinked `/theory` tier, and a
shipped-but-unlinked `/cognitive-maps` set (newest, already speaking the reoriented
language).

The flip makes the reoriented frame the trunk:

> Temper is an event-sourced coordination substrate whose organizing purpose is to be
> economical with attention. A cognitive map is a telos-seeded region of that substrate
> where humans and agents grow a shared, situated understanding together — and everything
> else is a projection over it.

Personal knowledge management does not disappear. It becomes **one projection over the
substrate** — a valid, useful view, no longer the whole story. This is the site expressing
the same commitment the confidence inventory states plainly: *Temper is a coordination
substrate, and personal knowledge management is one view over it — not the other way round.*

---

## The model: four trunks, three front doors

The flip is expressed as IA by separating three dials that the naive version conflates:
**navigation surface**, **URL hierarchy**, and **entry-point status**. They are independent.
A page can be a top-level URL without being a front door; a page can be reached often without
sitting in the primary nav.

```
/                      Router landing. Narrow hero, routes deeper. NOT "the product."
                       Offers THREE front doors a newcomer actually picks:

   → /cognitive-maps   The WHAT. Concrete, the on-ramp. Telos-seeded regions shown
                       proven-in-the-schema. The graph-walk "start here" surface.

   → /operating        The RUNNING-IT. For the cold enterprise evaluator: what the
                       architecture fixes vs. what a deployment shapes. (Promoted to
                       top-level — see below.)

   → using Temper      The PKM PROJECTION. The reframed /builders + /agents — "this is
                       one view over the substrate, the personal-knowledge view."

/theory                A fourth top-level URL, but NOT a front door. The WHY — attention as
                       teleological anchor, knowledge-as-relationship, the commitments the
                       whole system answers to. Reached by ascent from the concrete (from
                       within cognitive-maps, from operating's back-links, from the README),
                       by a reader who has seen the concrete thing and now wants the why.
                       It is the foundation all three doors rest on, surfaced on demand —
                       not a door itself.

/using-temper          The CLI/sync/MCP reference. The /docs rename, finally executed.
```

### Why theory is a flat top-level URL and not `/cognitive-maps/theory`

Theory is not *smaller* than cognitive-maps — it is *more general*. A cognitive map is one
place the theory cashes out (the richest, most grounding one); `/operating` is another; the
PKM projection is a third. Nesting the foundation *under* one of the things built on it would
invert the architecture and orphan the others (operating rests on the same theory; under a
nested scheme it would have to deep-link sideways into a sibling's children).

The instinct that prompted the question was correct in its *direction of illumination*:
theory is elucidated and grounded *by* cognitive-maps, not the reverse — theory-read-cold is
abstract, theory-read-after-seeing-the-onboarding-cogmap-work is concrete. But "X is best
understood after Y" is a claim about **reading order and cross-linking**, not **hierarchy**.
The fix for "theory shouldn't be a cold entry point" is to demote it *in navigation* and
route to it *by ascent* — not to move its URL. So: theory comes off the primary nav as a peer
front-door; it stays at `/theory/*` (no redirects, no move); the cognitive-maps graph-walk
leads up into a theory cluster as a *destination arrived at*.

This also honors the cognitive-maps set's own discipline — cross-references by concept, never
by ordinal or hierarchy, *because web pages have no inherent order and a reader can arrive
anywhere*. Encoding "theory is part of cognitive-maps" in the URL path would bake a reading
sequence into the structure, exactly the ordering that discipline warns against.

(The one frame in which nesting *would* be right: if cognitive-maps were declared Temper's
single headline and theory/operating/PKM were all framed as aspects of understanding-and-
running-cognitive-maps. That is a *different flip* — it makes cognitive-maps the trunk rather
than the substrate-and-its-regions. The chosen flip points away from it: the substrate is the
trunk, a map is one telos-seeded region of it, operating is its own promoted top-level.)

---

## `/operating` — the promotion, and the seam it crosses

### Audience: the cold enterprise evaluator

`/operating` is for someone arriving from *outside* to answer "what do I run, what do I get
out of the box, what's still mine to decide." They did **not** read the cognitive-maps
conceptual walkthrough and may never. This rules out leaving the operating content where it
was composed — as movement 7 of the cognitive-maps set, which assumes the reader has met the
seed cast (alice, bob, the onboarding-cogmap) across movements 1–6 and opens on "the very
first thing that exists is the seed file" trusting that grounding.

### The structure

- **`/operating` is a top-level hub** with the same four children the cognitive-maps
  operating set already has: `deployment`, `governance`, `observability`, `insights`. Those
  four child pages are well-built and need little content change; it is the *seam* — how
  `/operating` relates to its origin under cognitive-maps — that needs care.
- **A short orienting top** establishes the three-tier confidence ledger (below) and the
  "0→1 is invariant, everything after is shaped" framing, introducing just enough vocabulary
  to **stand alone** and linking *back* into `/cognitive-maps` and `/theory` where terms
  (telos, cogmap, event-sourced substrate) need their grounding — rather than restating
  movements 1–6.
- **`self-hosting.md`** (the existing operator runbook — Vercel project, Neon, Auth0 tenant)
  is the concrete floor beneath `/operating/deployment`: the "if you're self-hosting on
  Vercel, here's exactly what to do" detail the conceptual deployment page points down into.

### The seam: cognitive-maps movement 7 becomes a bridge

The cognitive-maps set has a deliberate genre split — **pages 1–6 *show* from the schema
outward** (the visuals are evidence; "here is a thing whose shape is proven by the data
model"), and **page 7 *invites* from operations inward** ("this has to be run somewhere, that's
a good problem"). Moving the operating content out must not cost the journeyer their ending.

So movement 7 **shrinks to a bridge**: it keeps the turn-outward beat that closes the
conceptual arc ("the map had to be stood up somewhere — a good problem, walked through under
[operating]") and hands across to `/operating` for the detail, rather than carrying the full
operating content. The 1–6-show / 7-invites split survives; movement 7 still *invites*, it
just invites *toward `/operating`* now. The journeyer gets their exhale; the evaluator gets a
real home.

---

## The three-tier confidence ledger

This is the spine of `/operating` and the register-setter for the whole flip. The evaluator
is best served by a page that keeps three things visibly separate rather than blurring them
into a flat "it's all handled" — and keeping them honest is what makes the page *trustworthy*.

The grounding for the middle tier is the invocation-envelope + neutral-contract work
(`kb_invocations`, the `temper-agents` crate, `DeploymentProfile` as runtime × residency).

1. **Fixed by the architecture** — invariant across every deployment, proven in the artifact
   and replay-verified. The event-primary ledger; the convention-agnostic kernel; teams-RBAC
   over homed boundaries; actors-as-entities; administration-is-event-sourced; the invocation
   envelope (accountability-grain run, telos/scope binding, terminal outcome); the delegation
   launch-gate; authorship-invisible-to-affinity. The strongest tier: *proven, not promised.*

2. **Extensible by design — and the edges are still being found.** The runtime choice is
   localized to a thin contract the substrate never reads (the `temper-agents` crate depends
   on the substrate, never the reverse; the kernel never branches on runtime). The *shape* of
   the delegation problem is the same across the platforms modeled so far (Vercel Eve and
   Claude Managed Agents are both first-class `RuntimeBinding`s; runtime and residency are
   orthogonal axes). **This is stated as extensible-shape with partial neutrality, not flat
   "runtime-neutral."** Concretely: adding a runtime today is a *patch to the contract crate*
   (the targets are enum variants, not configuration) — a deliberate too-early-abstraction-
   avoidance tradeoff — and *what becomes pure configuration vs. what the substrate must model
   is being determined by real deployment.* The seam is stated **as** a seam, and it is a live
   research front, not a permanent limitation: deploying on real runtimes is expected to move
   the config-vs-substrate-knows line, and the line will move as we learn. (This is also the
   forward-exciting close the `insights` page reaches for — the honest seam and the
   look-what-becomes-possible ending are the same sentence.)

3. **Genuinely yours / open** — the deployment chooses, and the page does not pretend
   otherwise. Which runtime; residency; token budget; tenancy model; per-tenant integration;
   the trigger cadence that wakes an agent; observability scope; how guarded the admin surface
   must be.

**The risk to guard against** (named so the drafting doesn't drift into it): the temptation,
now that there are clean plans and a soon-to-exist POC, to let *tier 2* masquerade as *tier
1* — "what we run" presented as "what Temper is." The evaluator is better served by "this is
invariant; this is what we happen to run and you can swap it; this is genuinely yours" than by
a blurred claim. Keep the three columns honest.

**A timing note for drafting:** the POC deployment status (Eve, possibly CMA) is expected to
change within days. Write the prose at the level of the *finding and the contract* (which are
stable), and let "running on Eve" become a concrete reference point added once true — the way
temperkb.io anchors the deployment-shape range. The architecture claim does not depend on the
POC; the POC is evidence for it.

---

## What changes on temperkb.io, by page group

### Correct already — connect, don't rewrite

- All `/theory/*` pages. Strong as-is.
- The entire `/cognitive-maps/*` set, **except** movement 7 (which becomes the bridge above)
  and the index (which needs its "start here" graph-walk updated to reflect the new IA and to
  include the theory cluster as a destination).
- `docs/guides/self-hosting.md` — accurate operator runbook; becomes the floor beneath
  `/operating/deployment`.

### Out of date — content (says the wrong thing now)

- **Landing `/`** — body claims the vault is the product (workflow strip, doc-type cards,
  "Temper Cloud / your vault everywhere"). The hero ("Clarify your intention") largely
  survives; the *body* is what asserts vault-as-product and gets replaced with the router.
- **README.md** — "A knowledge base for builders," context-rot opener, throughline/goals/
  tasks framing, `/builders` + `/agents` as primary entry points. Also has *intra-frame*
  drift: its Quick Start already shows the cloud-first `temper pull` / `temper resource
  create --from` flow while `/builders` and `/docs` still show the old local `temper add` /
  `temper init` flow — so the README and the product pages don't even agree today.
- **Nav + Footer** — link only How-it-works / Builders / Agents / Docs. `/theory`,
  `/cognitive-maps`, `/operating` are invisible. Highest-leverage mechanical fix: the trunks
  that *are* the story can't currently be reached.
- **`/builders`, `/agents`** — wholly in the old frame; to be reframed as the PKM projection
  (see below).
- **`/docs`** — mostly accurate as CLI reference; teaches `temper skill install` as the agent
  path, which **stays** (see the plugin-vs-skill scar below). Predates the substrate
  vocabulary entirely.

### The reframes

- **`/builders` + `/agents` → the PKM projection.** Surgical, not a rewrite: each gets a
  *frame-setting top* declaring "this is one view over the substrate — the personal-knowledge
  view" and linking up to the trunk. The bodies (warmup/save loop, doc types, MCP pathways)
  stay largely intact — as a *description of the PKM projection* they remain true; what
  changes is the claim of primacy. The cross-sell footers get rewritten to point at the
  substrate, not at each other. **Scar (2026-06-19):** an earlier draft of this spec assumed
  the roadmap would replace `temper skill install` with `temper plugin install`. That task was
  **cancelled** — skills + tools are the de facto way of working now — so `temper skill install`
  **stays as-is**, on `/agents` and `/docs` alike. The reframe makes no CLI change.

- **`/docs` → `/using-temper`** — the specced-but-never-done rename, plus a `/docs` →
  `/using-temper` redirect (single canonical URL), internal-link updates, and a minimal title/
  lede edit. A short header sentence points to `/theory` for the conceptual frame; the page
  itself stays operational.

---

## Work map — temperkb.io (dependency-ordered)

The umbrella task tracks these phases. Items 1–3 are unambiguous and fast; 4–7 carry the real
writing and the judgment calls. The two **seam-sensitive** items must not be batched with the
mechanical work — they need the cognitive-maps register and a careful review pass, because
they are where a careless change degrades something already good.

0. **Supersede + remove `theory-ia-proposal.md`.** Scar carried forward in this document
   (above); the file is removed from the working tree (`git rm`), history preserved. *Item
   zero — it actively contradicts every decision here.*
1. **Nav + Footer.** Add Theory (as a non-front-door link) / Cognitive-maps / Operating;
   present Builders + Agents under a "Using Temper" grouping. Mechanical, highest leverage —
   makes the correct-but-invisible trunks reachable.
2. **`/docs` → `/using-temper` + redirect.** Small; the flip is the moment.
3. **Landing body → router.** Keep/lightly-tune the hero; replace the body with the
   three-door router. (Depends on the doors existing — 1–2.)
4. **Promote `/operating` to top-level.** Hub + four children, re-fronted for the evaluator,
   re-registered against the three-tier ledger; `deployment` upgraded with the
   extensible-shape/partial-neutrality framing; `self-hosting.md` as the floor.
   **★ seam-sensitive.**
5. **Cognitive-maps movement 7 → bridge** (+ index "start here" graph-walk update to reflect
   the new IA and include the theory cluster). **★ seam-sensitive.**
6. **Reframe `/builders` + `/agents`** as the PKM projection. Most copy. (The plugin-vs-skill
   coupling is resolved: the plugin task was cancelled; `temper skill install` stays — see the
   scar above.)
7. **README** — rewrite substrate-first; resolve the intra-frame `temper add` vs. `temper
   pull` drift while doing so.

---

# Surface 2 — `docs/`, the operational tree

**Status:** the structure is built; the prose is not. Everything from here to *Surface 3*
governs the docs tree.

## What `docs/` is

The operational half of the public surface: how to install Temper, how to run a deployment,
how to write code against it. Published to the Apidog-hosted docs site from `main` through
Apidog's GitHub app — so the repository *is* the CMS, and a merge is a publish. Nothing on a
branch changes the published site; everything on `main` does.

## The invariant

> **Everything under `docs/` is public. There is no unpublished corner of it.**

This is a property of the tree rather than a configuration that has to be right. The
alternative — an allowlist naming what publishes — was considered during the rebuild and
rejected: an allowlist is a file someone can get wrong, and getting it wrong publishes
security audits. The invariant cannot be got wrong by editing a config file.

Its cost is real and accepted. Anything unfit for a stranger's eyes **moves** to `internal/`
rather than being suppressed in place. `internal/` is the sibling holding process artifacts,
the engineering record, and contributor guidance; it is reached through `CLAUDE.md` /
`AGENTS.md`, never from `docs/`.

The invariant is enforced by a CI gate with its own guard test, including a denylist of
directory names whose reappearance under `docs/` is a regression rather than a relocation.

## The tree is kind-shaped; navigation is audience-shaped

The same three-dials discipline as Surface 1, applied one level down. The directory says what
kind of page this is; the doors say who should read it and in what order. They are separate
dials and neither is derivable from the other.

```
docs/
├── index.md         the three doors
├── doors/           thin routers — sequence, not content
├── concepts/        authored, thin; links UP to temperkb.io for depth
├── playbooks/       authored; task-shaped sequences with stated outcomes
├── sdks/            authored; per-language manuals (neither concepts nor single task sequences)
└── reference/       GENERATED — never hand-edited
    ├── cli/         from the built binary's --help tree
    └── config/      from TemperConfig via schemars
```

**Why doors are pages and not directories.** A door carries reading *order* — the sequence a
persona should walk. Encoding that as hierarchy bakes one reader's path into the URL of
material that serves three, and forces `reference/cli` to be duplicated or arbitrarily
assigned to one persona. A reader can arrive anywhere; cross-reference by concept, never by
ordinal.

**Why `reference/` is one subtree.** It is the only arrangement in which the entire generated
surface has a single drift boundary. Scattering generation across audience directories means
one gate per location and no way to assert the set is complete.

**What separates a concept from a playbook.** A **concept** answers *what is this and how does
it work*. It is durable, it has no numbered steps, and it ends by linking up to `temperkb.io`
for the why. A **playbook** answers *how do I get to X*. It names its outcome in the first
paragraph, names its prerequisites rather than assuming them, and a reader who follows it
exactly arrives at the outcome. A page that does both is two pages.

**What `sdks/` is.** A per-language SDK manual — the generated gem, how it tracks the API, and
the usage patterns specific to that language's idioms. It is neither a durable concept (it
tracks a generated artifact) nor a single task sequence (it covers the whole surface). It keeps
the SDK material whole rather than splitting it across concepts and playbooks, because its
concept and task material interleave too tightly to separate without wrecking it.

## What a page owes its reader

The docs rebuild established these because the surviving prose violated all of them — it was
written for our own consumption, by people with the repository checked out, before the
audiences existed. The pages were not wrong; they were addressed to someone else.

- **Say who it is for, near the top.** One of the three audiences, named.
- **Name prerequisites; never assume them.** If the reader needs a deployment, a credential,
  or a completed earlier step, say so and link it. "Assumed context" is the defect class, and
  it survives every copy-edit because the prose reads fine to someone who already has it.
- **A reader who knows nothing beyond this page and what it links to reaches the stated
  outcome.** This is the test. It is not satisfied by accuracy.
- **No repository paths.** `../../scripts/bootstrap/saml-setup.sh` means nothing to a reader
  who will never clone this repository.
- **Vocabulary is defined or linked on first use.** Telos, cogmap, substrate, context,
  projection, seam, register — every one of these is ours, and `temperkb.io` owns the long
  answer.

## Generated content is fixed at its source

`reference/cli` and `reference/config` are committed projections. Each carries a drift gate
*and* an independent completeness cross-check — because **an artifact compared against itself
measures reproducibility, never correctness**. A generator that drops a page drops it from
both sides of a re-emit-and-diff, and the gate stays green forever. So each gate carries a
second, independent derivation: a parse of the clap command tree for the CLI, a flat walk of
the JSON schema for the config.

**An editorial pass does not apply to `docs/reference/`.** A wrong sentence there is fixed in
the Rust doc comment or the `--help` text and regenerated. Hand-editing it is reverted by the
gate, correctly.

`^docs/reference/` sits in `RUST_COUPLED` in CI scope detection, so a change there summons the
full Rust corpus. That is deliberate: scoped as docs-only, the gates were unreachable on
exactly the change they exist to catch.

## What establishes that the tree is sound — and what cannot

`python3 scripts/docs-coverage.py [--strict]` reports reach (which door routes to each page),
dangling links, links escaping `docs/`, generated-tree integrity, and publish coverage read
from the site's `llms.txt`. Dangling links are the only `--strict` failure, because they are
the only category that is a plain fact rather than a judgement.

**Navigation is reported UNKNOWN, and structurally so.** Apidog reconciles pages but leaves
emptied folder nodes behind and orders the sidebar itself. Neither is observable from the
repository: empty folders contribute no lines to `llms.txt`, and Apidog's public API has no
endpoint for navigation nodes or ordering — its whole surface is four operations, established
by executed probes against a real-route/non-route discriminator rather than by reading
documentation. Pruning and ordering are **manual UI actions**, done after a merge republishes.

> **A green `docs-coverage` run does not mean the site navigates correctly.** It means the
> tree is internally sound. The two claims are unrelated, and only one of them is checkable
> from here.

## Links out of `docs/` are dead links

Apidog publishes `docs/**` and nothing else. A link to `../../DEPLOYING.md` or
`../../packages/agent-workflows/mention/README.md` resolves on disk and reaches nothing on the
published site.

**Do not repath such links into `internal/`.** That makes them resolve in the repository and
still 404 on the site — it fixes the symptom the tooling reports and not the one the reader
hits. Either inline what the reader actually needs, or convert the link to a plain-text
backticked citation naming the document with no href. Provenance survives; no dead link on
either surface.

---

# Surface 3 — the API reference

Generated from the router itself — `openapi.json` is emitted from the Axum routes and their
utoipa annotations, and the docs host renders it into the reference a caller reads.

**It is not a directory in this repository.** `docs/reference/api/` does not exist and is not
planned; an earlier design routed material there before this was settled. The only generated
trees committed under `docs/` are `cli/` and `config/`.

Two consequences that decide where contract prose goes:

- **No authored page restates an endpoint or a schema.** If a reader needs a request shape,
  the reference is the answer and a link is the whole treatment.
- **Contract prose belongs in `docs/concepts/`.** The things a generated reference cannot
  express — what the trust boundary is, what a machine principal is and why it is not a user
  holding a long-lived token, what decides whether a write is permitted — are concepts. They
  are the integrator door's real content.

The register this settles: an integrator arriving cold needs **the boundary**, not the
endpoints. The endpoints are already complete and already generated. The boundary is the part
a person has to write, and it is the part that has never been written for them.

---

## Provenance

- **Conceptual ground:** `working-context-semantic-model.md`, `attention-manifesto.md`,
  `working-context-framing-schema.md`, `feature-development-and-coordination-substrate.md`,
  `temper-confidence-inventory.md` (the attention thesis; substrate-as-trunk; PKM-as-
  projection; translation-is-irreducible).
- **The cognitive-maps set & its register/discipline:** `(public)/cognitive-maps/the-set` (the genre
  split, the threaded seed, cross-reference-by-concept, partner-not-lecturer voice).
- **The middle-tier grounding (extensibility, the neutral contract):**
  `internal/superpowers/plans/2026-06-18-invocation-envelope-and-authorship-metadata.md` and
  `internal/superpowers/plans/2026-06-18-temper-agents-neutral-contract-crate.md`; the Eve/CMA
  comparison research under `internal/research/`.
- **Operator runbook (the deployment floor):** the self-hosting playbook under `docs/`.
- **Superseded:** `docs/theory-ia-proposal.md` (removed; see the scar above).

Grounding for Surfaces 2 and 3, added with the scope widening:

- **The docs surface design:** `internal/superpowers/specs/2026-08-19-docs-surface-rebuild-design.md` —
  the kind-shaped tree, the invariant, the doors-are-pages argument, and the decision to
  retire `docs/cognitive-maps/` in favour of `temperkb.io`.
- **The derivation layer:** `scripts/docs-coverage.py`, `scripts/emit-cli-reference.py`,
  `scripts/emit-config-reference.py`, and the drift gates in `.github/scripts/check-*-drift.sh`.
- **Discipline model:** `scripts/register-coverage.py` — detects-does-not-decide; never infers
  coverage from absence.

---

## Deliberate non-goals (this document)

- Does **not** write the page copy. Each page is its own drafting pass; this is decisions-and-
  rationale only.
- Does **not** specify Svelte component structure or routing internals — those follow from the
  IA, not the reverse.
- Does **not** propose visual-design changes. The site has an established register (palette,
  type, layout); the new frame inhabits it without redesign.
- Does **not** relitigate `/theory` or `/cognitive-maps` content. The flip is about *situating
  and connecting* them, not rewriting them.
- Does **not** enumerate the pages of `docs/`. Which page exists, and what each one says, is
  the drafting pass — this document governs the kinds, the audiences, and the separation rule.
- Does **not** own the generated trees. What `reference/cli` and `reference/config` contain is
  decided by the code they are emitted from; this document only rules that they are never
  hand-edited.
