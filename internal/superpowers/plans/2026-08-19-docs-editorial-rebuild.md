# Docs editorial rebuild — execution plan

Executes step 7 of [the docs surface rebuild design](../specs/2026-08-19-docs-surface-rebuild-design.md):
*"Author `doors/`, `concepts/`, `playbooks/` — the prose pass, once the frame holds still."*

Step 5 (enrich `site-ia.md` to govern all three surfaces) is **done** — commit `090a2932`.
Steps 1–4, 6 and 8 shipped in PRs #722 and #724.

---

## Read this before anything else

### The audits are partly stale, and stale in the flattering direction

Four audit reports classified all 29 hand-written pages. **PRs #725 and #726 then fixed a
subset of what they found.** Every defect list in those reports is a *hypothesis about a tree
that has since changed*. Re-verify per page against the working tree before acting; a
subagent handed a stale defect list will "fix" text that is already correct and report
success.

Specifically already fixed, do NOT re-fix: the `temper login` / `temper whoami` sites; the
`self-hosting.md` topology and `vercel.json` summary; the cogmap-gate claims in
`org-bootstrap.md`, `team-self-cognition-bootstrap.md`, `l0-content-delivery.md`,
`enterprise-install.md` and `auth/cognitive-map-authoring.md`; the root bootstrap SQL; the
published infrastructure identifiers.

### Hard dependency

**PR #726 must merge before Beat C touches its pages.** It rewrites `org-bootstrap.md` § 0,
`enterprise-install.md`, `l0-content-delivery.md`, `team-self-cognition-bootstrap.md` and
`auth/cognitive-map-authoring.md`. Rewriting those on a branch that lacks it either conflicts
or silently reverts a live-defect fix.

### The trap this plan exists to avoid

The pages are largely **factually correct**. What is wrong is **assumed context** — they were
written for readers who already knew why the thing exists and had the repo checked out. A
pass that fixes typos and tightens sentences will feel productive and will not change whether
a stranger can follow a guide to its stated outcome.

The test, from `site-ia.md`: *a reader who knows nothing beyond this page and what it links to
reaches the stated outcome.* **Not satisfied by accuracy.**

### The second trap

**Never hand-edit `docs/reference/**`.** It is generated from the binary and from
`TemperConfig`, with a drift gate and an independent completeness cross-check. Fix the doc
comment or `--help` text at source and regenerate — from a binary **built from the tree under
review**, never the PATH binary (measured: PATH is 0.3.1 against a tree building 0.3.3).

---

## Grounding evidence

Facts established by execution or by reading the tree, not recalled. Each is load-bearing for
a step below.

**The target tree is already decided.** The design spec's structure block:

```
docs/
├── index.md              the three doors
├── doors/                thin routers — sequence, not content
├── concepts/             authored, thin; links up to temperkb.io for depth
├── playbooks/            authored; cross-persona task guides
└── reference/            GENERATED — never hand-edited
```

`docs/guides/` and `docs/auth/` **do not appear in it.** The spec's own "What moves where"
table routes `guides/` → `playbooks/` + `reference/` and `auth/` → `concepts/` +
`reference/api/`.

**`docs/reference/api/` does not exist and will not.** `docs-coverage` reports only
`reference/cli/` and `reference/config/` as generator-claimed. The API reference is published
from `openapi.json` by the docs host. Ruled in `site-ia.md` Surface 3; contract prose lands in
`concepts/`.

**`docs-coverage` is directory-agnostic except for `doors/`.** Grepped: the only hardcoded
directory name is `doors`, used to attribute a route. Restructuring costs the tool nothing.

**Reach is measured FROM the doors; dangling links are the only `--strict` failure.** So a
move breaks `--strict` the moment an inbound link goes stale, and orphaning is *reported but
never fails*. This decides the per-beat gate below.

**Current inventory (this branch, after #725):** 33 hand-written `.md` — `auth/` 6,
`guides/` 23, `doors/` 3, `index.md`. Plus `reference/cli` 25, `reference/config` 1,
`diagrams/` 6 svg, 2 brand svg.

**Four gaps re-verified against the working tree, all still open:**

1. **No page teaches an individual user to authenticate.** `temper auth login` appears in six
   pages, *every one an operator page*. `docs/doors/for-users.md` mentions authentication
   **zero** times. It is a prerequisite of every command in the users door.
2. **The integrator route never gives the API base URL, and never links the API reference.**
   `for-integrators.md` states a base URL zero times. The string `openapi` appears in exactly
   **one** file in all of `docs/` — `cloud-agents.md`, which Beat A moves to `internal/`. After
   Beat A there is no mention of the API spec anywhere on the site.
3. **No page defines a context.** `context create` appears in two pages, both operator
   runbooks, neither defining the term. It is the first sticking point on three separate pages
   (`operational-memory.md`, `corpus-ingestion.md`, `teams.md`).
4. **`temper skill install` has no page.** Two passing mentions, neither a walkthrough. It is
   the only Claude Code path.

**Gate facts, verified in code — the docs must not contradict these:**

| operation | gate |
|---|---|
| `cogmap create` (genesis) | open to any authenticated profile; creator gets read+write+grant |
| `cogmap bind` | system-admin OR team owner/maintainer who administers the map |
| `cogmap reconcile` | admin-gated |
| `has_system_access` | `kb_principal_standing` = `approved`, nothing else |
| `is_system_admin` | `kb_principal_governance`, nothing else |

Consequence for the users door: **there is no broken route.** An individual user can create a
map and author into it. The cogmap pages stay in the users door with a *named prerequisite*,
not a gate warning.

---

## The per-beat gate

Every beat ends with all of these, run and read:

```bash
python3 scripts/docs-coverage.py --no-network --strict    # MUST be exit 0
bash .github/scripts/check-docs-public-only.sh
bash .github/scripts/test-check-docs-public-only.sh
bash .github/scripts/test-docs-coverage.sh
```

**`--strict` green is non-negotiable per beat**, because a move breaks inbound links
immediately. So: **every move updates its inbound links in the same commit.** Reach will
legitimately dip below 100% during Beats B–D as new pages land before the doors route to them;
that is reported, not a failure, and Beat E restores it. Do not "fix" a reach dip by
prematurely rewriting a door.

If any beat touches `crates/` or `docs/reference/`, add `cargo make check`.

---

## Beats

Each carries a **CONFORM / EXTEND / AMEND** tag. CONFORM = honour an existing load-bearing
constraint, and cite it. EXTEND = build beyond an existing affordance, and cite what authorizes
it. AMEND = deliberately change an existing thing, and cite both.

### Beat A — amend the frame, subtract what is not public

**AMEND** — `site-ia.md` Surface 2 currently names three authored kinds. Pete's ruling adds a
fourth (`sdks/`) for per-language manuals, which are neither durable concepts nor single task
sequences. Amend the tree block and the concept-vs-playbook section to admit it.

**CONFORM** — the publish invariant (`site-ia.md` Surface 2): anything not fit for a stranger
moves to `internal/`, never suppressed in place.

Move to `internal/`, all four confirmed contributor-addressed by audit with quoted evidence:

| page | why | audit citation |
|---|---|---|
| `guides/drain-operator-queries.md` | organizing device is the author's own `[live]`/`[shape]`/`[blind]` verification marks; every example is our production output on a named date; closes on a maintainer to-do list | `:15-21`, `:30`, `:231-243` |
| `guides/cloud-agents.md` | *"how to prepare tasks for and work as a cloud-based Claude Code agent on the temper project"*; deliverable is a PR | `:3`, `:229-237` |
| `auth/README.md` | *"if you are changing anything on an auth path…"* plus a PR checklist | `:4-6`, `:85-102` |
| `auth/authorization-seam.md` | title is a Rust module path; body is crate-internal signatures; closes with `cargo make test-e2e` | `:1`, `:30-56`, `:225` |

**Two things must not be lost in the move, and both are the whole risk of this beat:**

- `cloud-agents.md:23-34` is the **only** documentation anywhere of how a headless session
  authenticates without browser OAuth (`TEMPER_TOKEN` / `TEMPER_PROVIDER` / `TEMPER_DEVICE_ID`
  / `TEMPER_API_URL`). That is integrator content. **Capture it before the move**; it becomes
  part of Beat B's integrator material.
- `auth/README.md` is the only page **either** door offers for the trust boundary
  (`for-integrators.md:11-12`, `for-operators.md:26-27`). Moving it leaves both pointing at
  nothing. Beat B1 writes the replacement; **A and B1 land together or A leaves the doors
  linking a moved page.**

Update `internal/agents/environment.md:13`, which currently points *into* `docs/` for
cloud-agents — the dependency inverts correctly on the move.

### Beat B1 — the four concepts the doors need

**EXTEND** — authorized by the design spec: *"Concepts that the docs surface still needs (telos,
cogmap, substrate, context) are authored fresh and thin in `docs/concepts/`, linking up to
temperkb.io — they are not a copy of these twelve files."*

| page | source | note |
|---|---|---|
| `concepts/contexts-and-refs.md` | **new** | Highest leverage in the whole plan. What a context is; the ref grammar `@me/slug`, `@handle/slug`, `+team-slug`, bare UUID. Closes the first sticking point on three pages at once. Two pages currently teach the grammar **by example only and contradict each other** on whether `@me` is accepted — `teams.md:122` vs `operational-memory.md:399-403`. Resolve against the code, do not restate either. |
| `concepts/trust-boundary.md` | **new**, replacing `auth/README.md`'s role | Audit gave a 10-obligation list. Non-negotiable inclusions: authentication is a Bearer JWT and an instance validates exactly one issuer; the audience requirement **and how a caller obtains the value**; the two gates as *outcomes* (401 vs 403 `SYSTEM_ACCESS_REQUIRED`), never as "Level 1/2"; the wire-level error contract; **registration is not admission and nothing fails in between**; a machine principal is an ordinary principal; what a human-driven agent authenticates as. Plus two things no page supplies: **the API base URL and a link to the published API reference.** |
| `concepts/auth-identity.md` | `self-hosting.md:188-...` (§ Auth identity) | The audience/issuer contract is currently stated **four times and owned nowhere** — `self-hosting.md`, `self-hosting-saml.md` ×2, `self-hosting-okta.md`, `enterprise-install.md`. One page; the four playbooks link in. |
| `concepts/teams-and-roles.md` | `teams.md:3-24` | What a team is, the personal team, the role ladder. Durable; not task-shaped. |

**CONFORM** — `site-ia.md`'s rule: a concept ends by linking **up** to temperkb.io for the why,
and never restates an endpoint, flag, or config field.

### Beat B2 — the remaining concepts

Same EXTEND authorization. Extracted from pages whose task-shaped half becomes a playbook in
Beat C, so B2 and C are coupled per page — do the extraction and the playbook in one commit
per source page where that is cleaner.

`machine-tokens.md` (from `auth/machine-token-contract.md:25-109`, `:146-158`) ·
`authoring-authorization.md` (from `auth/cognitive-map-authoring.md:1-56`, `:85-106` — **after
#726**) · `token-verification.md` (from `auth/jwt-verification.md:12-26`, `:109-136`) ·
`saml-reconcile-channel.md` (from `auth/reconcile-channel.md`, and route it to the **operators**
door, not integrators — it is a server-to-server call inside one deployment) ·
`release-verification.md` (from `install.md:53-236`) · `telemetry.md` (from
`open-telemetry-setup.md:21-148`) · `operational-memory.md` (from `operational-memory.md:7-18`,
`:382-414`) · `slack-identity-and-revocation.md` (from `slack-integration.md:40-133`, `:357-468`).

Each source page's contributor half — status tables keyed to PR numbers, post-mortems,
source-path indexes — goes to `internal/`, not into the concept.

### Beat C — playbooks, in audience batches

**AMEND** — every page moves; `guides/` ceases to exist. Authorized by the design spec's
structure block and Pete's ruling of 2026-08-19.

Each page: declare the audience near the top; name prerequisites rather than assuming them;
state the outcome in the first paragraph; resolve escaping links per the audits' per-link
rulings (**inline** the manifests a reader cannot proceed without; **cite** as backticked text
with no href; or **drop**); strip correction-archaeology, PR numbers, and internal workstream
labels (`B3`, `B5`, `T1`/`T2`/`T6`, `chunks 1–6`, `A2`, `Stage 4`, `Phase B1`, `G3`, `D11`).

**C1 users** — `install-temper` (the audit measured ~50% of `install.md` is provenance essay
and ~27% restates generated `version.md`/`update.md`; the install-shaped content is ~140 of 369
lines) · `authenticate` **(new — gap 1)** · `connect-claude-desktop` · `connect-claude-code`
**(new — gap 4)** · `run-a-team` · `adopt-operational-memory` · `build-a-cognitive-map` ·
`ingest-a-corpus`.

**C2 operators, standing it up** — `self-host-temper` · `self-host-with-okta` (cleanest page in
the corpus; closest to the doors' register — use it as the model) · `self-host-with-saml` ·
`enterprise-install` · `bootstrap-an-org` · `deploy-the-web-ui` (from `self-hosting.md:343+`, a
second outcome).

**C3 operators, connecting** — `slack-mentions` · **`github-connection` (MERGE the two pages)**:
the infra guide defers its own final verification to the temper guide (`:212-215`) and the
temper guide bounces back five times; one nine-step procedure cut mid-sequence · `deploy-a-
steward-agent` and `deliver-l0-content` — **Pete ruled self-hosters run both**, so each needs a
fork-first step, the manifest inlined, and the `neonctl` step generalised · `bootstrap-team-
self-cognition` (**move to the operators door** — it states its own audience as operator at
`:10-13` while `for-users.md:33` lists it) · `send-traces-to-an-otlp-backend`.

**C4 integrators** — `standing-up-a-machine-credential` (from `machine-credentials.md`, the
only page in the corpus already written for someone outside the repo; its `:282-299` command
reference is a verbatim duplicate of generated `reference/cli/admin.md` and becomes a link).

### Beat D — `sdks/temper-rb.md`

**EXTEND** — the fourth authored kind, added to `site-ia.md` in Beat A. Keep the page whole;
its concept and task material interleave too tightly to split without wrecking it. Delete the
eight *"Backed by:"* blocks — provenance for a doc reviewer, citing paths no reader can open,
including two that confess what is *not* covered.

### Beat E — doors and index

**AMEND** — rewrite all four to route the finished tree. Reach returns to 100%; this is the
beat that restores it. Fix the ordering contradiction the audits found: the operators door
presents `enterprise-install` as an annotated variant while four pages open by calling it the
spine and pushing the reader to it. Pick one and make four pages agree.

### Beat F — verification and review

`docs-coverage --strict`, the guard tests, `cargo make check` if `crates/` was touched, and a
**consolidated multi-lens adversarial review at the end of the plan** — not per beat.

At least one reviewer must be given only a rewritten page and the question *"following only
this page and what it links to, where do you get stuck?"* — the review that matters is the one
that re-runs the acceptance test, not one that re-reads for style.

---

## Out of scope, deliberately

- **The 45 contributor comments and 2 dead references** from the D11 sweep. Filed; separate work.
- **The 25 escaping links as a baselined sweep.** Resolved per page during C, not as a rail.
- **Apidog navigation.** Its API cannot prune nav nodes or set order — settled by executed
  probes. Nav debris and ordering are manual UI actions after a merge republishes, and
  `docs-coverage` reports navigation UNKNOWN and structurally so.
- **The parked minors** from plan 1: gate escapes for nested/renamed directories and non-`.md`
  loose files; no `/internal/` CODEOWNERS segment; `internal/specs/` vs
  `internal/superpowers/specs/` collision; 6 residual `docs/cognitive-maps` citations; two dated
  `[2026-08-13]` claims naming a then-nonexistent path.

## Acceptance criteria

- Every surviving page under `docs/` is addressed to one of the three named audiences and says
  which near the top.
- A reader with no prior context can follow any playbook to its stated outcome; prerequisites
  are named and linked, never assumed.
- `concepts/` exists as a thin tier linking up to temperkb.io; `playbooks/` holds the
  task-shaped sequences; `guides/` and `auth/` no longer exist.
- The four gaps are closed: user authentication, the API base URL + reference link, contexts
  and the ref grammar, and the Claude Code path.
- `docs-coverage --strict` green and reach back to 100%.
- No editorial change touches `docs/reference/**`; source-side fixes plus regeneration instead.
