# Deliver L0 Kernel Content

**For operators.** This playbook delivers or updates the content of the L0
kernel cognitive map (`system-default`) on a live Temper instance. It covers
the non-obvious **fail-closed admin gate** and the **grant → reconcile →
re-lock** procedure an operator must follow to write to L0.

L0 is release/operator-governed, not operationally stewarded. This is an
operator runbook, not an end-user flow.

## Outcome

By the end you will have: the 22 kernel landmarks plus the telos charter
reconciled into the live L0 map, and the fail-closed admin gate restored so
the kernel is immutable again. A re-run against unchanged content is an
idempotent no-op.

## Prerequisites

- **A deployed, migrated instance** — see
  [self-hosting Temper](./self-host-temper.md), or the
  [enterprise install](./enterprise-install.md) for the full end-to-end
  sequence.
- **The charter-set primitive applied.** The `cogmap_charter_set` function
  must be present on the target database before a telos-bearing reconcile.
- **An `embed`-capable `temper` binary.** The reconcile path embeds each
  manifest entry client-side (ONNX), so it requires a binary built with the
  `embed` feature. The default install bundles it; a non-`embed` build
  returns a clear `requires the 'embed' feature` error rather than running.
- **An admin connection to the instance's Postgres** (admin role) for the
  grant and re-lock steps.
- **The operator's profile id.** Sign in once before the grant so the profile
  row exists; capture its UUID.
- **A fork of the Temper repository.** Self-hosters run this playbook
  alongside steward-agent deployment, and both need a checkout — fork first.

For the trust model that gates the L0 write, see
[trust boundary](../concepts/trust-boundary.md).

## What L0 is

The L0 kernel cognitive map is the public, root-team-joined "what is temper"
cognitive map. It is **born deterministically by migration** under the
`system` actor, with reserved ids:

| Entity | Reserved id |
|--------|-------------|
| L0 cogmap | `00000000-0000-0000-0005-000000000001` |
| L0 telos resource | `00000000-0000-0000-0005-000000000002` |
| Root team slug | `temper-system` |

L0 is a **living** map, but it evolves only by shipping **new additive
migrations** that call the substrate mutation functions against L0's reserved
id — never by editing the immutable birth migration. Its *content* (landmarks
+ telos charter) is delivered separately from its *schema*, via the operator
reconcile flow described here.

## The gotcha: L0 writes are fail-closed

`temper cogmap reconcile` against L0 is gated by
`require_cogmap_write_admin → is_system_admin`. Two things make this
non-obvious:

1. **`is_system_admin` reads `kb_principal_governance` and nothing else.** Not
   team membership, not `gating_team_slug`, not a column on `kb_profiles`. A
   profile is a system admin exactly when it has a row in that table.
2. **Nothing has one out of the box.** The canonical seed grants no governance,
   so the L0 write gate denies **everyone** and the kernel is immutable on a
   fresh instance. A reconcile attempt returns **403 Forbidden** until the
   temporary operator grant below has been run.

This is intentional: the L0 special-case is fail-**closed** so an unconfigured
instance cannot have its kernel rewritten by any authenticated user. The cure
is not a permission flag — it is the temporary operator grant below.

## Procedure

> **Snapshot prod before a hand-run data change.** Take a backup of your
> target database before running the grant or re-lock SQL.

### 0. Fork and clone

Self-hosters run this playbook alongside steward-agent deployment; both need a
repository checkout. Fork the Temper repository and clone your fork. The
inlined manifest below is saved into it.

### 1. Grant (temporary admin)

Connect to the target database with an admin role — any admin connection to
your instance's Postgres will do; the steps below are plain SQL. Point the
gating slug at the root team and make the operator an `owner` of it:

```sql
UPDATE kb_system_settings SET gating_team_slug = 'temper-system';

INSERT INTO kb_team_members (team_id, profile_id, role)
VALUES (
  (SELECT id FROM kb_teams WHERE slug = 'temper-system'),
  '<operator-profile-uuid>',
  'owner'
)
ON CONFLICT (team_id, profile_id) DO UPDATE SET role = 'owner';

-- Confirm the grant took:
SELECT is_system_admin('<operator-profile-uuid>');  -- expect: true
```

### 2. Reconcile

Save the manifest below as `l0-kernel.yaml` in your checkout, then reconcile
the live L0 map against it:

```bash
temper cogmap reconcile 00000000-0000-0000-0005-000000000001 \
  --manifest l0-kernel.yaml
```

The CLI reads the manifest, **embeds each entry client-side** (ONNX, via the
`embed` feature), builds a pre-embedded request, and PUTs it to
`PUT /api/cognitive-maps/{id}`.

It is **idempotent** — a re-run against unchanged content reports zero
changes:

```json
{ "created": 0, "updated": 0, "folded": 0, "unchanged": 22, "charter": "unchanged" }
```

The `charter` field is a distinct grain from the landmark counts. Its values
are `absent` (manifest carried no `telos:`), `unchanged`, `created` (first
delivery into an empty telos), or `updated` (live charter differed and was
replaced). First delivery reads `"created": 22, … "charter": "created"`.

Confirm the outcome counts match expectation (first delivery creates; a re-run
reports `unchanged` / `charter: unchanged`).

### 3. Re-lock (restore fail-closed)

Undo the grant so L0 returns to immutable:

```sql
UPDATE kb_system_settings SET gating_team_slug = NULL;

DELETE FROM kb_team_members
WHERE team_id = (SELECT id FROM kb_teams WHERE slug = 'temper-system')
  AND profile_id = '<operator-profile-uuid>';
```

Delivered content persists. The next lifecycle update repeats this same
grant → reconcile → re-lock dance.

## The L0 kernel manifest

Save the following as `l0-kernel.yaml`. It delivers the 22 orientation
landmarks and the authored telos charter to the live `system-default` cogmap.
The CLI embeds each entry client-side (ONNX) before the PUT, so the server
stays embed-free on the request path.

Each entry carries a pre-generated, stable `id` (uuidv7) — the landmark's
substrate identity. The reconcile diff matches manifest entry ↔ live resource
by this `id`, and edges reference their target by `id`. `origin_uri` is pure
attribution (loose, non-unique — never a key). These ids are permanent once
shipped; a future landmark gets a freshly-generated uuidv7. First delivery is
all-additive: no `fold_resources` / `fold_edges`.

The `telos:` section is delivered via `cogmap_charter_set` (fold-then-reproject).
`block_mutate` is revise-only and cannot populate an empty telos;
`cogmap_charter_set` is the correct primitive for initial and subsequent
charter delivery.

```yaml
telos:
  statement: "Orient an arriving agent so it can act correctly under temper's substrate at minimal attention cost — by holding the landmarks that say what temper is and how it works, the settled invariants it must not break, and the wayfinding that routes it to the right tool, skill, or more-specific map. This is the bottom referent every agent and every other cognitive map is situated by: it says 'this is the system you are in,' and it actively lowers the activation energy to reach for — and compose — the capabilities temper offers, so a less-powerful model acts where it would otherwise stall. In service of any agent, on any model, becoming competent-to-act in temper without rediscovering the system."
  questions:
    - question: "Is this a landmark an agent needs the moment it arrives to know what temper is and where it stands — the substrate, this map's bottom-referent role, the telos it's currently thinking under?"
      context: "The first thing any agent asks is where am I. Hold the few situating landmarks, not their depth."
    - question: "Is this a core term an agent must share to read temper at all — cogmap, telos, resource, edge, facet, region, lens, event, invocation — versus jargon a specific map can own?"
      context: "An agent that can't read the system's words can't act in it. L0 is the kernel-vocabulary bedrock; deeper or domain terms live where they're used."
    - question: "Is this a settled invariant an agent must not break — event-as-primary, the access floor (it operates as a scoped principal), agents tend declared structure and never cluster, acts carry attribution, cross-map promotion is human-gated?"
      context: "A weaker model won't infer these and will violate them by default. State the always/nevers plainly as landmarks — this is where L0 earns its keep."
    - question: "When an agent needs to do something, does L0 name the tool, skill, or map to reach for — and make reaching the obvious next move?"
      context: "Weaker models stall not for lack of reasoning but for lack of willingness to reach for and compose tools. L0 routes — need X, the tool is Y, use it; compose Y with Z — it gives permission to act."
    - question: "Is this depth that belongs in a more-specific map, with L0 holding only the landmark and the path to it?"
      context: "L0 holds landmarks-and-the-way-to-reach, never contents. What falls through to be elaborated here is the saturation pole — it bloats the kernel an arriving model must read."
    - question: "Does the agent need this to know the edge of what it may do here — what's out of bounds, what needs a human, what it must not assume?"
      context: "An oriented agent also needs to know where its competence and authority stop — the HITL gates, the leak-safety floor it can't cross, the acts that aren't its to make. This is also where a steward learns to read a telos-charter as an instrument: acting-under-a-telos is the steward's job."
  framing:
    - "This map is self-referential: temper mapped in temper's own substrate; the canonical worked-example of a bootstrapped map."
    - "It is a reference layer — skill files and references — born populated and curated, not accreted from work. That is what distinguishes it from every other map."
    - "Every other map (organizational-foundational, domain) is situated by L0 and routes through it; L0 holds kernel landmarks and the paths, the specific maps hold depth."
    - "Authored for the arriving agent, possibly a weaker model — invariant-forward, scannable, landmark-shaped. Every byte costs context; the attention-manifesto extends to agents."
    - "L0 models how a telos-charter is read to make judgment calls, so a steward learns to find the edge of its mandate from the charter itself."
entries:
  # --- concept-landmarks (Q2 vocabulary) ---
  - origin_uri: "temper://kernel/concept/cogmap"
    id: "019f03f4-2ace-76cb-b1fc-260239dd16a5"
    title: "cogmap"
    doc_type: "kernel_landmark"
    body: "A cognitive map: a bounded, telos-governed view of resources and their relationships. An agent works inside one map's frame at a time."
    facets: { layer: concept }
    edges:
      - { to: "019f03f4-2ad3-7663-b196-55ec482efba3", kind: contains, label: holds }  # -> region
  - origin_uri: "temper://kernel/concept/telos"
    id: "019f03f4-2acf-7c45-bd12-a2a7152644a1"
    title: "telos"
    doc_type: "kernel_landmark"
    body: "A map's telos: its declared purpose, held as a charter (statement + questions-with-context + framing). The telos is the perspective under which salience is judged — salience is never universal."
    facets: { layer: concept }
    edges:
      - { to: "019f03f4-2ace-76cb-b1fc-260239dd16a5", kind: express, label: governs }  # -> cogmap
  - origin_uri: "temper://kernel/concept/resource"
    id: "019f03f4-2ad0-7f06-ae22-e53b2fb9b99f"
    title: "resource"
    doc_type: "kernel_landmark"
    body: "A resource: the named, findable unit of content in a map. Addressed by ref; its body is content blocks."
    facets: { layer: concept }
    edges:
      - { to: "019f03f4-2ad1-723f-a534-68e587e71220", kind: near, label: graph-grain }  # -> edge
  - origin_uri: "temper://kernel/concept/edge"
    id: "019f03f4-2ad1-723f-a534-68e587e71220"
    title: "edge"
    doc_type: "kernel_landmark"
    body: "An edge: a declared, typed relationship between resources (express, contains, leads_to, near). Edges are authored, never inferred."
    facets: { layer: concept }
  - origin_uri: "temper://kernel/concept/facet"
    id: "019f03f4-2ad2-7e87-961b-a9f94d1138cd"
    title: "facet"
    doc_type: "kernel_landmark"
    body: "A facet: a key/value property on a resource (e.g. layer: concept). Facet overlap binds resources into families and is an affinity input."
    facets: { layer: concept }
    edges:
      - { to: "019f03f4-2ad3-7663-b196-55ec482efba3", kind: express, label: binds-into }  # -> region
  - origin_uri: "temper://kernel/concept/region"
    id: "019f03f4-2ad3-7663-b196-55ec482efba3"
    title: "region"
    doc_type: "kernel_landmark"
    body: "A region: a materialized cluster of resources under a lens. Regions are the substrate's pure function over edges + facets — agents never assign them."
    facets: { layer: concept }
    edges:
      - { to: "019f03f4-2ad4-75b5-a39c-1de621ee588b", kind: express, label: shaped-by }  # -> lens
  - origin_uri: "temper://kernel/concept/lens"
    id: "019f03f4-2ad4-75b5-a39c-1de621ee588b"
    title: "lens"
    doc_type: "kernel_landmark"
    body: "A lens: a weighting over edge-kinds and salience that shapes how a map materializes into regions. The same map yields different regions under different lenses."
    facets: { layer: concept }
  - origin_uri: "temper://kernel/concept/event"
    id: "019f03f4-2ad5-7321-9492-334e8b0c8cf2"
    title: "event"
    doc_type: "kernel_landmark"
    body: "An event: the append-only record of a mutation, projected to state. The ledger, not the row, is the source of truth."
    facets: { layer: concept }
    edges:
      - { to: "019f03f4-2ad6-7226-af5f-f0eb16c346e4", kind: near, label: ledger-grain }  # -> invocation
  - origin_uri: "temper://kernel/concept/invocation"
    id: "019f03f4-2ad6-7226-af5f-f0eb16c346e4"
    title: "invocation"
    doc_type: "kernel_landmark"
    body: "An invocation: one accountable agent run — its trigger, its scope (a cogmap's telos), the mutation events it produced, and a terminal outcome."
    facets: { layer: concept }
    edges:
      - { to: "019f03f4-2ad7-7252-bac9-ddc6655be043", kind: express, label: run-by }  # -> steward
  - origin_uri: "temper://kernel/concept/steward"
    id: "019f03f4-2ad7-7252-bac9-ddc6655be043"
    title: "steward"
    doc_type: "kernel_landmark"
    body: "A steward: an agent that tends a map's declared structure under its telos — creating resources, asserting edges, setting facets — but never clustering."
    facets: { layer: concept }
  # --- invariant-landmarks (Q3) ---
  - origin_uri: "temper://kernel/invariant/event-as-primary"
    id: "019f03f4-2ad8-7737-b05c-c3e5d80223ce"
    title: "event-as-primary"
    doc_type: "kernel_landmark"
    body: "Always: every mutation is an event appended to the ledger and projected to state. Never edit state directly; the ledger is authoritative and replayable."
    facets: { layer: invariant }
    edges:
      - { to: "019f03f4-2ad5-7321-9492-334e8b0c8cf2", kind: express, label: governs }  # -> event
  - origin_uri: "temper://kernel/invariant/access-floor"
    id: "019f03f4-2ad9-7029-b962-6b1bb6c0ba43"
    title: "access-floor"
    doc_type: "kernel_landmark"
    body: "Always: you operate as a scoped principal. You can only read and write within your map's visibility; the substrate enforces this — you cannot reach beyond your bounds even by mistake."
    facets: { layer: invariant }
    edges:
      - { to: "019f03f4-2ad0-7f06-ae22-e53b2fb9b99f", kind: express, label: governs }  # -> resource
  - origin_uri: "temper://kernel/invariant/tend-not-cluster"
    id: "019f03f4-2ada-75f4-a3cb-584976bccf25"
    title: "tend-not-cluster"
    doc_type: "kernel_landmark"
    body: "Always tend declared structure (resources, edges, facets). Never compute regions or assign salience — region formation is the substrate's pure function on materialize."
    facets: { layer: invariant }
    edges:
      - { to: "019f03f4-2ad3-7663-b196-55ec482efba3", kind: express, label: governs }  # -> region
  - origin_uri: "temper://kernel/invariant/attribution"
    id: "019f03f4-2adb-747e-bb2d-dba22dbf5e82"
    title: "attribution"
    doc_type: "kernel_landmark"
    body: "Always: your structural acts carry attribution — a reason and a confidence band — so every act is reviewable and reversible."
    facets: { layer: invariant }
    edges:
      - { to: "019f03f4-2ad6-7226-af5f-f0eb16c346e4", kind: express, label: governs }  # -> invocation
  - origin_uri: "temper://kernel/invariant/promotion-gated"
    id: "019f03f4-2adc-7b1c-b7cd-6ac30a26a8c9"
    title: "promotion-gated"
    doc_type: "kernel_landmark"
    body: "Lifting a concept across into a different map (promotion-translation) is human-gated: it means something different under the target telos. Never promote autonomously."
    facets: { layer: invariant }
  # --- wayfinding references (Q4/Q5) ---
  - origin_uri: "temper://kernel/reference/search"
    id: "019f03f4-2add-7621-88c1-d1d58d771047"
    title: "search"
    doc_type: "kernel_landmark"
    body: "To find what already exists by meaning or graph-nearness, reach for the search tool. Start here before creating anything."
    facets: { layer: reference }
    edges:
      - { to: "019f03f4-2ad0-7f06-ae22-e53b2fb9b99f", kind: leads_to, label: to-find }  # -> resource
  - origin_uri: "temper://kernel/reference/create"
    id: "019f03f4-2ade-7166-b06a-1a6524013ae6"
    title: "create"
    doc_type: "kernel_landmark"
    body: "To add a resource reach for resource_create; to relate two, relationship_assert; to tag one, facet_set. Compose them: create, then relate, then facet."
    facets: { layer: reference }
    edges:
      - { to: "019f03f4-2ad0-7f06-ae22-e53b2fb9b99f", kind: leads_to, label: to-add }  # -> resource
  - origin_uri: "temper://kernel/reference/materialize"
    id: "019f03f4-2adf-77fc-86cf-868cecf9dce6"
    title: "materialize"
    doc_type: "kernel_landmark"
    body: "To see the map's current regions, reach for request_materialize under a lens. Read the regions to orient before acting."
    facets: { layer: reference }
    edges:
      - { to: "019f03f4-2ad3-7663-b196-55ec482efba3", kind: leads_to, label: to-see }  # -> region
  - origin_uri: "temper://kernel/reference/charter"
    id: "019f03f4-2ae0-74f9-ada7-d1a43a768047"
    title: "charter"
    doc_type: "kernel_landmark"
    body: "To understand a map's purpose and the edge of your mandate, read its telos-charter. The questions-with-context tell you what belongs and what doesn't."
    facets: { layer: reference }
    edges:
      - { to: "019f03f4-2acf-7c45-bd12-a2a7152644a1", kind: leads_to, label: to-understand }  # -> telos
  # --- boundary-landmarks (Q6) ---
  - origin_uri: "temper://kernel/boundary/hitl"
    id: "019f03f4-2ae1-7194-8f91-1f96802a9630"
    title: "hitl"
    doc_type: "kernel_landmark"
    body: "Out of bounds without a human: cross-map promotion, founding a new map's identity, and changing a settled commitment. Pause and ask."
    facets: { layer: boundary }
  - origin_uri: "temper://kernel/boundary/leak"
    id: "019f03f4-2ae2-7218-bfeb-3abf8693ff4e"
    title: "leak"
    doc_type: "kernel_landmark"
    body: "You cannot read another team's material into a lower map — the access floor forbids it structurally. If a read returns nothing, it may be out of scope, not absent."
    facets: { layer: boundary }
  - origin_uri: "temper://kernel/boundary/mandate"
    id: "019f03f4-2ae3-7550-8ef2-6146cc6b7a94"
    title: "mandate"
    doc_type: "kernel_landmark"
    body: "Read the charter's questions to find the edge of your mandate: if material is depth for a more-specific map, route it there rather than elaborating it here."
    facets: { layer: boundary }
```

## Lifecycle framing

L0 content evolves through two complementary mechanisms:

- **Schema / structural birth and additive evolution** ship as **migrations**
  that call the substrate mutation functions against L0's reserved id. These
  are immutable once shipped.
- **Content delivery** (landmarks + telos charter) is **operator-directed
  reconciles** of the manifest, each gated by the temporary grant above.

Both are operator-governed; neither is ambient or steward-driven. L0's charter
declares its ambient steward wake = never.

## Further reading

- **The trust model that gates the L0 write:**
  [trust boundary](../concepts/trust-boundary.md).
- **The base deployment this playbook starts from:**
  [self-hosting Temper](./self-host-temper.md).
- **The full end-to-end install sequence:**
  [enterprise install](./enterprise-install.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
