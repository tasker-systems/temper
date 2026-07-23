# Linked Identity State Machine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Replace the seven-facts-into-one `MintOutcome::Revoked` collapse with a typed linked-identity
state machine in the admission idiom, so every Slack refusal names its cause and its true remedy.

**Architecture:** A pure resolver in `temper-principal::linked_identity` returns
`Result<ActiveLink, LinkRefusal>` — the exact shape of `admit` (`admission.rs:37`). `temper-services`
gathers evidence and holds every identifier; the mint query re-roots at `kb_profile_auth_links` so
standing is reachable when no vault row exists. A sealed `VerifiedSlackPrincipal` inserted by the
signature middleware closes the ungated-internal-call-path gap. A new drift gate protects the two
internal wire contracts that no generator covers.

**Tech Stack:** Rust (temper-principal, temper-services, temper-api), sqlx, axum, trybuild, ts-rs,
bash CI guards, cargo-make.

**Spec:** `docs/superpowers/specs/2026-07-23-linked-identity-state-machine-design.md` — committed at
`70e9a1d9` on this branch. **Read the spec section each task cites. This plan is an index over it,
not a replacement for it.**

**Task:** `019f8fc5-0d7b-7b10-be5d-739adc43047f` · **Goal:** `019f6344-01a5-7fc0-9e22-a80585f801fc`

---

## How to read this plan (deliberate deviation from the plan template)

The plan template asks for complete code in every step. This project's
`implementation-grounding.md` (GD-4) forbids invented code bodies in plans, and gives the reason:

> *"A plan's **intent** (design, sequencing, rationale) reliably survives contact; its **specifics**
> (named functions, file lists, SQL bodies) are reliably stale on arrival… And the sketch is not
> merely waste — **it wins**: implementers build the code block, not the correct prose beside it."*

So code blocks below appear **only** where they carry a `file:line` citation or a spec-section
authorization. Everywhere else you get: the exact file, the exact assertion contract, the existing
sibling to copy the shape from, and the command to run. **Read the cited sibling before writing —
that is the grounding, and it is more current than any snippet here could be.**

Every task is tagged **CONFORM** (honor a load-bearing constraint on disk), **EXTEND** (build past an
existing affordance, spec-authorized), or **AMEND** (deliberately change something that exists).

## Global Constraints

- **`temper-principal` takes no identifiers and no `sqlx`.** `Cargo.toml:6-12` — *"This crate
  therefore takes NO ids at all."* `LinkEvidence` carries `bool`, `bool`, `Option<&str>`. Nothing else.
- **No `_ =>` arm in any match over `Standing` or a new state enum.** `lib.rs:9-11`. The one sanctioned
  catchall is on `&str` input returning a refusal (`standing.rs:44-49`).
- **Every refusal variant carries a non-empty `reason()`**, asserted across the whole cell space
  (`refusal.rs:62-66`).
- **`LinkRefusal` derives serde + `ts_rs::TS` only — NOT `utoipa::ToSchema`.** Spec §2.1. Both routes
  are allowlisted out of `openapi.json` (`check-openapi-routes.sh:63-64`).
- **The redacting hand-written `Debug` must survive on every token-carrying type.**
  `slack_grant_vault_service.rs:81`, `slack_mint.rs:59`. `.github/scripts/audit-credential-debug.sh`
  will red if it does not, and that guard exists because a derived `Debug` once leaked a credential.
- **Auth before writes**, and *"not-mintable checks first, before any cached token is decrypted or the
  RT is spent"* (`slack_grant_vault_service.rs:264-267`).
- **`cargo make check` before every commit.** It runs the five security tripwires (`main.toml:40-62`).
- **Never split the Slack principal.** It is 2–4 segments and travels whole
  (`temper-core/src/types/slack.rs:89-90`).

## Verification commands

| Scope | Command |
|---|---|
| Pure crate | `cargo nextest run -p temper-principal` |
| Services (DB) | `cargo nextest run -p temper-services --features test-db` |
| temper-api | `cargo nextest run -p temper-api --features test-db --test <target>` — **never bare `-p temper-api`, it hangs on the bin target** |
| E2E | `cargo build -p temper-cli --bin temper && cargo make test-e2e-embed` |
| Schema snapshots | `cargo make test` (a doc-comment edit on a `JsonSchema` type drifts a snapshot) |
| Everything local | `cargo make check` |

---

## File structure

**Create**
- `crates/temper-principal/src/linked_identity.rs` — the pure resolver, `LinkEvidence`, `ActiveLink`, `LinkRefusal`
- `crates/temper-principal/tests/linked_identity_matrix.rs` — the exhaustive cell matrix
- `crates/temper-services/tests/compile_fail/forge_verified_slack_principal.rs` (+ `.stderr`)
- `.github/scripts/check-slack-contract-drift.sh`
- `.github/scripts/test-check-slack-contract-drift.sh`
- `packages/agent-workflows/mention/agent/generated/admission.ts` — ts-rs output, committed

**Modify**
- `crates/temper-principal/src/lib.rs:16-26` — module + re-exports
- `crates/temper-services/src/services/slack_grant_vault_service.rs` — query re-root, resolver call, `MintOutcome`
- `crates/temper-services/src/services/slack_link_service.rs:159` — third `SlackLinkOutcome` arm
- `crates/temper-services/src/services/slack_mint_service.rs:32-37` — AMEND the stale comment; return type
- `crates/temper-services/src/auth/mod.rs` (or a sibling module) — `VerifiedSlackPrincipal`, sealed
- `crates/temper-api/src/middleware/internal_auth.rs` — mint gate inserts the proof
- `crates/temper-api/src/handlers/slack_mint.rs` — proof extractor, refusal arms
- `crates/temper-api/src/handlers/slack_link.rs` — link-state rendering, callback third page
- `packages/agent-workflows/mention/agent/lib/link.ts:35-37` — consume generated types
- `tools/cargo-make/main.toml` — `generate-ts-types` second export; new `slack-contract-drift` task; `[tasks.check]`
- `.github/workflows/code-quality.yml:121` — `guard-tests` gains the self-test step

---

### Task 1: The pure resolver

**Tag: EXTEND** — spec §2.1, §2.2, §3 authorize a new module in `temper-principal`.

**Files:**
- Create: `crates/temper-principal/src/linked_identity.rs`
- Create: `crates/temper-principal/tests/linked_identity_matrix.rs`
- Modify: `crates/temper-principal/src/lib.rs:16-26`

**Interfaces produced** (fixed by spec §2.1 — later tasks depend on these exact names):

```rust
pub struct LinkEvidence<'a> { pub linked: bool, pub vaulted: bool, pub standing: Option<&'a str> }
pub struct ActiveLink { /* private */ }
impl ActiveLink { pub fn standing(&self) -> Standing; }
pub enum LinkRefusal { NotLinked, Standing(Refusal), NotVaulted }
impl LinkRefusal { pub fn reason(&self) -> String; }
pub fn resolve(ev: LinkEvidence<'_>) -> Result<ActiveLink, LinkRefusal>;
```

**Read first:** `crates/temper-principal/src/admission.rs` in full. `resolve` is `admit` with three
facts instead of one; `ActiveLink` is `AdmittedPrincipal` (`:14-25`) — private field, no `Default`,
no `From`, accessor only. Copy that shape rather than inventing one.

- [ ] **Step 1: Write the failing matrix test.**
  Model on `crates/temper-principal/tests/matrix.rs` — read its `STATES` const (`:14-22`) and
  `every_standing_variant_is_in_the_matrix`, which is the trick that makes a new `Standing` variant a
  test failure rather than a silent gap. Reproduce that property here.
  **The cell space:** `linked` (2) × `vaulted` (2) × standing (the 6 `STATES` entries + one
  unrecognized string, e.g. `"quarantined"`) = 28 cells.
  **Assert, per spec §2.2:**
  - every cell decides — `Ok` or a named `Err`, never a panic;
  - `linked: false` ⇒ `LinkRefusal::NotLinked`, **for every standing and every `vaulted`** (standing is
    unknowable before the link exists);
  - `linked: true` + standing not `approved` ⇒ `LinkRefusal::Standing(_)` — **including when
    `vaulted: false`** (this is the ordering that fixes the false remedy);
  - the `Standing(_)` payload is the *same* `Refusal` `temper_principal::admit` returns for that input
    — compare against `admit` directly, do not restate the mapping;
  - `linked: true` + `approved` + `vaulted: false` ⇒ `LinkRefusal::NotVaulted`;
  - `linked: true` + `approved` + `vaulted: true` ⇒ `Ok`;
  - every `Err` has a non-empty `reason()` (mirror `every_cell_is_decided_and_every_refusal_carries_a_reason`).
  Add the arity pin modeled on `admit_reads_standing_and_nothing_else` (`admission.rs:102-109`),
  including its "do not fix this by updating the call" comment — it is the alarm for a future
  conjunction being ANDed into the decision.

- [ ] **Step 2: Run it and watch it fail.**
  `cargo nextest run -p temper-principal linked_identity`
  Expected: compile error — `linked_identity` is not a module.

- [ ] **Step 3: Implement the module.** Exhaustive matches, no `_ =>`. Delegate standing to
  `admit(ev.standing).map_err(LinkRefusal::Standing)?` — **call the incumbent, never restate it**
  (`plan-verification.md`: *"For every predicate the plan authors, find the incumbent"*). Register in
  `lib.rs` beside the existing `mod`/`pub use` blocks (`:16-26`).

- [ ] **Step 4: Green.** `cargo nextest run -p temper-principal` — the existing 17 tests plus the new
  matrix. Confirm the pre-existing count did not drop.

- [ ] **Step 5: Confirm purity held.** `cargo tree -p temper-principal | grep -c sqlx` must print `0`.
  This is the D3 property `Cargo.toml:6-12` claims; assert it rather than assume it.

- [ ] **Step 6: Commit.** `cargo make check` first.

---

### Task 2: Re-root the mint query and call the resolver

**Tag: AMEND** — spec §2.3 authorizes changing the query's root; §1 is the defect it fixes.

**Files:**
- Modify: `crates/temper-services/src/services/slack_grant_vault_service.rs` (`MintOutcome` at `:60`,
  `mint_access_token` at `:237`, the SELECT at `:246-258`, the gate at `:268`)

**Interfaces consumed:** Task 1's `resolve`, `LinkEvidence`, `LinkRefusal`.
**Interfaces produced:** `MintOutcome` gains a refusal-carrying shape. Tasks 3 and 4 map it to the wire.

**Read first:** spec §2.3 for both SQL forms and why the old root makes §2.2's ordering
unimplementable. Then `mint_access_token` in full — the `FOR UPDATE OF v` lock, the cached-token fast
path at `:273`, and the RT-spend below it.

- [ ] **Step 1: Write the failing service tests.**
  In the existing `#[cfg(test)]` module of that file, using **its own** helpers (`insert_profile`,
  `set_standing`, `seed_link`, `seed_grant` — read their real signatures at the bottom of the file;
  do not assume them). New cases:
  - a principal with **no auth-link row** ⇒ `NotLinked` — *not_ `NotVaulted`_. Today unrepresentable.
  - a linked, **unvaulted**, `denied` principal ⇒ the standing refusal, **not** `NotVaulted`. This is
    the cell today's vault-rooted query cannot even reach.
  - `denied` / `requested` / `revoked` / `deactivated` / no-standing-row each ⇒ **distinguishable**
    refusals. One assertion per state; the point is that they differ.
  **Also update, do not delete:** `mint_refuses_a_profile_without_approved_standing` (`:762`),
  `mint_refuses_a_deactivated_profile` (`:707`), `mint_refuses_a_revoked_profile` (`:735`),
  `mint_reports_revoked_and_does_not_refresh` (`:679`). Their assertions become named refusals.
  Spec §7: *"A test that still passed unchanged would mean the collapse survived."*

- [ ] **Step 2: Red.**
  `cargo nextest run -p temper-services --features test-db slack_grant_vault`

- [ ] **Step 3: Implement.** Re-root the SELECT per spec §2.3. Build `LinkEvidence` from the row,
  call `resolve`, map. **Preserve** `FOR UPDATE OF v`, and keep the decision **before** the cache
  branch (`:264-267`). Drop the `revoked_at.is_some() ||` disjunct (spec §2.4) — the column stays.
  Leave a comment recording that soft-revoke was superseded by disconnect's `DELETE` (commit
  `3a45b1ab`), so the next reader does not re-derive it.

- [ ] **Step 4: Green**, then regenerate the caches — the query text changed:
  `cargo sqlx prepare --workspace -- --all-features` then `cargo make prepare-services`.
  **Order matters** (CLAUDE.md). Expect churn in `crates/temper-services/.sqlx`; check each pruned
  entry has a same-query replacement rather than assuming the diff is noise.

- [ ] **Step 5: Prove the cache is honest with a cold offline check.**
  `cargo clean -p temper-services && SQLX_OFFLINE=true cargo check -p temper-services --all-targets --features test-db`
  Without the `clean`, cargo reuses the warm build and passes vacuously.

- [ ] **Step 6: Commit** after `cargo make check`.

---

### Task 3: The mint wire type

**Tag: AMEND** — spec §4.2. `SlackMintResponse` gains refusal arms; the false remedy at
`slack_mint.rs:49-51` is deleted.

**Files:**
- Modify: `crates/temper-api/src/handlers/slack_mint.rs` (`SlackMintResponse` `:38-57`, its `Debug`
  `:59-69`, the `From` `:71-85`, the handler `:98-114`)

**Read first:** the whole file (114 lines) — the `From` impl is currently total and
information-preserving, and it must stay that way over the wider enum.

- [ ] **Step 1: Write the failing test.** In the temper-api integration target for Slack (find it:
  `rg -l "slack" crates/temper-api/tests/`). Assert the serialized `status` values are **distinct**
  for a standing refusal versus `not_vaulted` versus `not_linked`, and that the standing arm carries
  the typed `Refusal` (so a client can branch on `kind`).

- [ ] **Step 2: Red.** `cargo nextest run -p temper-api --features test-db --test <that target>`

- [ ] **Step 3: Implement.** Widen `SlackMintResponse`; keep `#[serde(tag = "status", rename_all = "snake_case")]`.
  **Delete** the *"The user must re-link; retrying will never succeed"* doc on the old `Revoked` arm —
  it is the false remedy (spec §1.1). Extend the hand-written `Debug` (`:59-69`) to the new arms; it
  must stay hand-written and must keep redacting the token.

- [ ] **Step 4: Green**, and run the credential guard explicitly, since this file is one of its named
  exemplars: `bash .github/scripts/audit-credential-debug.sh`

- [ ] **Step 5: Commit** after `cargo make check`.

---

### Task 4: `link-state` speaks the same vocabulary

**Tag: AMEND** — spec §4.1, including the recorded information-widening.

**Files:**
- Modify: `crates/temper-api/src/handlers/slack_link.rs` (`SlackLinkStateResponse` `:60`,
  `slack_link_state` `:81-124`)

**Read first:** spec §4.1 — both routes keep separate secrets; one vocabulary, two renderings.
**Do not merge the routes.** `mint.ts:8-13` explains why, and it is right.

- [ ] **Step 1: Write the failing e2e test.** In `tests/e2e/tests/slack_link_test.rs`, beside
  `a_linked_principal_gets_its_handle_and_mints_no_intent` (`:731`). Assert a linked-but-`denied`
  principal's link-state response names the standing refusal — and, critically, that it still
  **mints no intent** (the `:731` test's existing property must not regress: an already-linked user
  must never be re-prompted with a fresh authorize URL).

- [ ] **Step 2: Red.** `cargo build -p temper-cli --bin temper && cargo make test-e2e-embed`
  (the CLI binary must exist or every CLI-driven e2e test fails with `Os NotFound`).

- [ ] **Step 3: Implement.** Render the resolved state minus the token. Keep the `:96-98` read-first
  short-circuit — its comment calls that ordering *"the whole fix"* for a prior bug; do not disturb it.

- [ ] **Step 4: Green.**

- [ ] **Step 5: Commit** after `cargo make check`.

---

### Task 5: The callback's third page

**Tag: AMEND** — spec §4.3. This is where the born-`Denied` trap dies.

**Files:**
- Modify: `crates/temper-services/src/services/slack_link_service.rs:159` (third arm)
- Modify: `crates/temper-api/src/handlers/slack_link.rs` (`run_callback` ~`:225-332`, the page
  renderers `:487-511`)

**Read first:** spec §4.3's table, and `slack_link.rs:225-332` in full — particularly the
one-transaction comment. **The transaction is load-bearing and must not change**; its rollback
property is pinned by `link_is_rolled_back_with_its_caller_transaction`
(`slack_link_service.rs:474`).

- [ ] **Step 1: Write the failing tests.**
  - Service: `SlackLinkOutcome` gains `NoRefreshToken` (spec §2.5) — the state at `slack_link.rs:279-311`
    that the service currently reports as `Linked` and the handler overrules.
  - E2E, in `slack_link_test.rs`: an **un-approved** principal completes the callback and the page
    does **not** contain a bare `Account <em>connected</em>.`; it does contain the standing
    `reason()`. Model the HTML assertions on `:1629-1646`, which is the existing pattern for exactly
    this kind of check.

- [ ] **Step 2: Red.** Both suites.

- [ ] **Step 3: Implement.** Three pages per spec §4.3. **Keep writing both rows** — do not refuse the
  link (spec §4.3 gives the reason: refusing means re-linking after approval, for no gain). Add the
  third renderer beside `connected_page` / `not_connected_page` (`:494-511`).

- [ ] **Step 4: Green.**

- [ ] **Step 5: Commit** after `cargo make check`.

---

### Task 6: The end-to-end test that does not exist today

**Tag: EXTEND** — spec §1.2, §7. The gap is that **no test drives the un-approved link path**.

**Files:**
- Modify: `tests/e2e/tests/slack_link_test.rs`

- [ ] **Step 1: Write it as one narrative test.** Link an un-approved principal end to end, then
  mention: assert the callback page is honest (Task 5), assert the mint names the standing refusal
  and **not** `revoked` (Task 2/3), and assert that after `approve_standing` the *same* principal
  mints a token **with no re-link** — which is the property spec §4.3 chose the design for.
  **Do not call `approve_standing` at setup.** That call (`:866-868`) is the workaround that hid this
  path; this test exists to walk it.

- [ ] **Step 2: Run.** `cargo build -p temper-cli --bin temper && cargo make test-e2e-embed`
  It should pass on the code from Tasks 1–5. **If it passes trivially, it is not testing what it
  claims** — verify by reverting Task 2's ordering locally, watching it go red, and restoring.

- [ ] **Step 3: Commit** after `cargo make check`.

---

### Task 7: The sealed `VerifiedSlackPrincipal`

**Tag: EXTEND + AMEND** — spec §5. EXTEND: the new sealed type. AMEND: `slack_mint_service.rs:32-37`,
whose stated premise is stale.

**Files:**
- Modify: `crates/temper-services/src/auth/mod.rs` (or a sibling module in the same crate)
- Modify: `crates/temper-api/src/middleware/internal_auth.rs` (mint gate, `:166-179`; the shared
  `require_signature_with` `:39-92`)
- Modify: `crates/temper-api/src/handlers/slack_mint.rs` — drops `Json<SlackMintRequest>`
- Modify: `crates/temper-services/src/services/slack_mint_service.rs:32-37` — AMEND the comment
- Create: `crates/temper-services/tests/compile_fail/forge_verified_slack_principal.rs` + `.stderr`

**Read first:** spec §5 **including §5.2**, which states precisely what this does and does not claim —
it does *not* make the string more trustworthy; it makes an ungated internal call path
unrepresentable. Then read the working instance: `middleware/auth.rs:119` (insert),
`auth.rs:34-37` (extract, note the `.cloned()`), `auth/mod.rs:285-288` (the seal),
`tests/compile_fail/forge_authenticated_profile.rs` (the proof).

**CONFORM:** do **not** reach for `Authorized<A>` / `ScopedAuthority` — they are `pub(crate)` to
temper-services (`authz/mod.rs:54,99`) and unreachable from temper-api. The `auth::`-style seal (pub
struct, private fields, pub module) is the one that crosses crates.

- [ ] **Step 1: Write the trybuild fixture.** Copy the shape of `forge_system_admin.rs` and its
  committed `.stderr`. Note the existing fixture's comment guesses `E0603` while the snapshot records
  `E0423` — trust the snapshot, and generate yours by running rather than by writing it by hand.
  Requires the `trybuild` feature (`Cargo.toml:53-59`); it is OFF by default and runs in its own CI
  job plus `cargo make test-trybuild`.

- [ ] **Step 2: Red.** `cargo make test-trybuild` — fails because the type does not exist.

- [ ] **Step 3: Implement the seal.** In temper-services: private field, accessor, and **one**
  constructor. Derive `Clone` only if the handler extracts by value via `FromRequestParts` —
  `auth.rs:37` shows why (`SystemAdmin` derives `Debug` only, which is why only `&SystemAdmin` is
  obtainable there).

- [ ] **Step 4: Wire the middleware.** The mint gate parses the principal from the **already-verified**
  buffered bytes and inserts the proof. `require_signature_with` already buffers and re-attaches
  (`:78-90`); the two other gates that share it must be unaffected — a shared helper touched for one
  caller is a drift site.
  **Keep `validate_slack_principal`** (`slack_link.rs:133-150`) on the path: the shape check must
  still run, now before the proof is minted rather than in the handler.

- [ ] **Step 5: Handler.** Drop `Json<SlackMintRequest>`; take the proof. Decide whether
  `SlackMintRequest` still earns its place, or whether the middleware's parse supersedes it.

- [ ] **Step 6: AMEND the stale comment** at `slack_mint_service.rs:32-37`. Do not delete the
  paragraph — rewrite it to say what is now true and why it changed, and keep §5.2's honest limit
  (possession of `SLACK_MINT_SECRET` is still the wire-level enforcement, per
  `internal_auth.rs:158-161`). A future reader must not conclude this closed more than it did.

- [ ] **Step 7: Green.** `cargo make test-trybuild`, the Slack e2e suite, and
  `bash .github/scripts/audit-route-auth.sh` (it asserts layers **per builder** — both `create_app`
  and `create_internal_app` — precisely because a whole-file grep missed one).

- [ ] **Step 8: Commit** after `cargo make check`.

---

### Task 8: Generated types for the mention agent

**Tag: EXTEND** — spec §6.1. Works around the mention package's deliberate workspace isolation.

**Files:**
- Modify: `tools/cargo-make/main.toml:272-284` (`generate-ts-types`)
- Create: `packages/agent-workflows/mention/agent/generated/admission.ts` (committed)
- Modify: `packages/agent-workflows/mention/agent/lib/link.ts:35-37`

**Read first:** spec §6.1. `TS_RS_EXPORT_DIR` is **per-invocation** (`main.toml:276-281`), which is
what makes the second export possible. The mention package is *"deliberately NOT a bun `workspaces`
member"* (CLAUDE.md), so it cannot import from `packages/temper-ui/...` — the generated file must land
inside the package. `package.json` declares `"imports": { "#*": "./agent/*" }`, so it is importable as
`#generated/admission.js`.

- [ ] **Step 1: Add the second export line** in `generate-ts-types`, targeting
  `packages/agent-workflows/mention/agent/generated`, beside the existing `-p temper-principal` line
  (`:281`) and mirroring its `mkdir -p` guard (`:275`).

- [ ] **Step 2: Generate and inspect.** `cargo make generate-ts-types`. Confirm the file contains
  `LinkRefusal` and `Refusal`, and that the file-count assertion (`:282-283`) still guards both dirs
  — if it only counts the temper-ui dir, an empty mention export would pass silently.

- [ ] **Step 3: Point `link.ts` at the generated types.** Replace the hand-written union at
  `link.ts:35-37`. Keep the doc comment's *"the contract with the Rust enum"* framing — it is now
  literally true.

- [ ] **Step 4: Verify the agent still typechecks and tests.**
  `cd packages/agent-workflows/mention && npm install && npm run typecheck && npm run test`
  **Install from inside the agent dir** — a root `npm install` inherits the root's bun `overrides`
  and fails (CLAUDE.md).

- [ ] **Step 5: Commit** the generated file. It must be **tracked** — Task 9's gate cannot diff an
  untracked path, and would then pass forever while checking nothing.

---

### Task 9: The drift gate

**Tag: EXTEND** — spec §6.2, §6.3. Closes one bounded instance of the "no CI gate on ts-rs drift" gap.

**Files:**
- Create: `.github/scripts/check-slack-contract-drift.sh`
- Create: `.github/scripts/test-check-slack-contract-drift.sh`
- Modify: `tools/cargo-make/main.toml` — new task + `[tasks.check].dependencies` (`:27-37`)
- Modify: `.github/workflows/code-quality.yml:121-146` — `guard-tests` step

**Read first:** `.github/scripts/check-temper-ts-drift.sh` in full — it is the model, and its two
hard-won properties are mandatory here:
1. regenerate, then `git diff --exit-code` (compares against **git**, not a fresh build);
2. **assert the artifact is tracked before diffing** (`:26-34`) — *"A gate that cannot fail is not a
   gate; make that state loud instead of green."*

Also read `test-audit-route-auth.sh` as the model for the self-test.

- [ ] **Step 1: Write the self-test first.** It must prove the gate goes **red** when the contract
  breaks: patch a `LinkRefusal` variant name in a temp copy, run the gate, assert non-zero exit,
  restore, assert zero. A guard whose failure path is never exercised is the exact gap #498's audit
  found in `audit-route-auth.sh`.

- [ ] **Step 2: Red.** `bash .github/scripts/test-check-slack-contract-drift.sh` — the gate does not
  exist yet.

- [ ] **Step 3: Write the gate.** Regenerate via `generate-ts-types` (or a narrower `-p temper-principal`
  invocation), then the tracked-check, then the diff. The error message must say *"run
  `cargo make generate-ts-types`, then **stage** the regenerated file"* — CLAUDE.md records that the
  "just regenerated it and it still fails" confusion is the common one, because the gate diffs
  against git.
  **It never skips** (spec §6.2). Say so in the header comment, with the cost stated: unlike
  `check-temper-ts-drift.sh` it needs a cargo build, not just Node.

- [ ] **Step 4: Green.** Both the self-test and the gate itself.

- [ ] **Step 5: Wire both surfaces.**
  - `tools/cargo-make/main.toml`: a `slack-contract-drift` task modeled on `openapi-ts-drift`
    (`:320-325`), added to `[tasks.check].dependencies` beside it.
  - `code-quality.yml`: a step in `guard-tests` (`:121`). That job's own comment says *"Adding a guard
    means adding its harness HERE, in the same PR"* — this task is that PR.

- [ ] **Step 6: Prove the wiring, don't assume it.** `cargo make check` and confirm the new task
  appears in the output. Then break the contract locally, re-run `cargo make check`, watch it red,
  restore.

- [ ] **Step 7: Commit.**

---

## Self-review

**Spec coverage.** §1 → Tasks 2, 3, 6. §2.1 → Task 1. §2.2 → Task 1. §2.3 → Task 2. §2.4 → Task 2
(disjunct dropped, column kept). §2.5 → Task 5. §3 → Task 1. §4.1 → Task 4. §4.2 → Tasks 3, 7.
§4.3 → Task 5. §5 → Task 7. §6.1 → Task 8. §6.2/§6.3 → Task 9. §7 → distributed, with the missing
e2e test as Task 6. §8 → Task 2 Steps 4–5 (`.sqlx`) and Task 3 Step 4 (`audit-credential-debug`).
§9 → no tasks, correctly. §10 → no task; it is #498's rebase, tracked on that PR.

**Two gaps found and closed while reviewing:**
- §8 names `audit-credential-debug.sh` as hardcoding `MintOutcome` / `SlackMintResponse` in its
  `BASELINE` (`:73-79`) and self-test (`test-audit-credential-debug.sh:97,101`). Task 3 Step 4 now
  runs that guard explicitly; if the type names change, the baseline and its self-test must move in
  the same commit.
- The plan originally had no `.sqlx` cold-verification step; Task 2 Step 5 adds it, because a warm
  build passes vacuously.

**Type consistency.** `resolve` / `LinkEvidence` / `ActiveLink` / `LinkRefusal` are used with the same
spelling in Tasks 1, 2, 8, 9. `SlackLinkOutcome::NoRefreshToken` appears only in Task 5.
`VerifiedSlackPrincipal` only in Task 7.

**Placeholder scan.** No TBD/TODO. Code blocks appear only in the Global Constraints (quoted from
disk) and Task 1's interface block (fixed by spec §2.1); every other step cites the sibling to read,
per the deviation declared at the top.

**Sequencing note.** Tasks 1–6 are one coherent behavioural change and could ship as one PR. Task 7
(sealed proof) and Tasks 8–9 (generation + gate) are each independently shippable and independently
revertable — per the repo's *"split PRs on coherence when deployed"* convention, three PRs is the
natural cut. Confirm with Pete before opening any.
