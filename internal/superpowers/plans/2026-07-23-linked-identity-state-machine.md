# Linked Identity State Machine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Replace the seven-facts-into-one `MintOutcome::Revoked` collapse with a typed refusal that
names its cause and its true remedy.

**Architecture:** A free function in temper-services returns `Result<Mintable, LinkRefusal>`,
delegating standing to `temper_principal::admit` rather than restating it. The mint query holds its
row lock in a CTE so the vault can be LEFT JOINed without losing it. A sealed `VerifiedSlackPrincipal`
— minted by a verification that *moves into* temper-services — closes the ungated-internal-call-path
gap. A general ts-rs drift gate lands last, after #498 brings the second TS consumer.

**Tech Stack:** Rust (temper-principal, temper-services, temper-api), sqlx, axum, trybuild, ts-rs,
bash CI guards, cargo-make.

**Spec:** `docs/superpowers/specs/2026-07-23-linked-identity-state-machine-design.md` — **Revision 2**.
Read the spec section each task cites; this plan is an index over it. **Read spec §0 first** — it
records what Revision 1 got wrong and why, and several tasks below exist because of it.

**Task:** `019f8fc5-0d7b-7b10-be5d-739adc43047f` · **Goal:** `019f6344-01a5-7fc0-9e22-a80585f801fc`

---

## How to read this plan (deliberate deviation from the plan template)

Per `implementation-grounding.md` GD-4, code blocks appear **only** where they carry a `file:line`
citation or a spec-section authorization. Everywhere else: the exact file, the assertion contract,
the sibling to copy, and the command. **Read the cited sibling before writing.**

Revision 1 of the spec had exactly two invented code blocks. **Both were blockers.** That is the
evidence for this deviation, not an argument for it.

Every task is tagged **CONFORM** / **EXTEND** / **AMEND**.

## Global Constraints

- **Denial is a 200 with a typed payload, never an `ApiError`.** `slack_mint.rs:31-33` — *"Neither is
  an error, so neither is an HTTP failure."* This is why `ScopedAuthority` was rejected (spec §0).
- **`LinkRefusal` uses `tag = "reason"`, and `Standing` is a struct variant with a named field.**
  Spec §2.1. `tag = "kind"` collides with `Refusal`'s own tag and emits duplicate JSON keys.
- **Delegate standing to `admit`; never restate it.** Spec §3.1.
- **No arity pin on `resolve`.** Spec §3.1 — copying `admit`'s anti-conjunction test onto a
  deliberate three-fact conjunction disables D2's alarm.
- **The redacting hand-written `Debug` must survive** on every token-carrying type
  (`slack_grant_vault_service.rs:81`, `slack_mint.rs:59`). `audit-credential-debug.sh` baselines them
  by name.
- **Auth before writes**, and *"not-mintable checks first, before any cached token is decrypted or the
  RT is spent"* (`slack_grant_vault_service.rs:264-267`).
- **`cargo make check` before every commit.** It runs the five security tripwires.
- **Never split the Slack principal** (`temper-core/src/types/slack.rs:89-90`).

## Verification commands

| Scope | Command |
|---|---|
| Services (DB) | `cargo nextest run -p temper-services --features test-db` |
| temper-api | `cargo nextest run -p temper-api --features test-db --test <target>` — **never bare `-p temper-api`, it hangs on the bin target** |
| E2E | `cargo build -p temper-cli --bin temper && cargo make test-e2e-embed` |
| trybuild | `cargo make test-trybuild` |
| Schema snapshots | `cargo make test` |
| Everything local | `cargo make check` |

> **None of these can catch invalid SQL.** Revision 1's query passed every one of them and was
> illegal to execute. Task 2 Step 1 is the mandatory executable step.

---

## PR boundaries (spec §10, revised at implementation)

**PR 1** = Tasks 1–7 (the behavioural change **plus** the sealed proof — Task 7 folded in on Pete's
call, since it shares `slack_mint.rs` with PR 1 and the anti-stack rule made a separate PR mean
"wait for PR 1 to merge"). Then **#498 rebases and lands**. **PR 2** (was PR 3) = Tasks 8–9
(generation + gate), last, because `mint.ts` exists only on #498's branch.

---

### Task 1: The resolver and its refusal type

**Tag: EXTEND** — spec §2.1, §2.2, §3.

**Files:**
- Create: `crates/temper-services/src/services/slack_link_state.rs` (or a sibling module beside the
  Slack services — name it for the concept, not the channel)
- Modify: `crates/temper-services/src/services/mod.rs`
- Modify: `crates/temper-principal/src/admission.rs` — add the "see also" half of §3.2's kinship

**Interfaces produced** (fixed by spec §2.1–§2.2):

```rust
pub struct LinkEvidence<'a> { pub linked: bool, pub vaulted: bool, pub standing: Option<&'a str> }
pub struct Mintable { /* private — gates the decrypt, never leaves the service */ }
pub enum LinkRefusal { NotLinked, NotVaulted, Standing { refusal: Refusal } }
pub fn resolve(ev: LinkEvidence<'_>) -> Result<Mintable, LinkRefusal>;
```

**Read first:** spec §3.1 (why there is no arity pin) and §3.2 (why nothing is extracted). Then
`crates/temper-principal/src/admission.rs` for the shape, and
`crates/temper-core/src/types/access_gate.rs:145` for how `Refusal` is carried.

- [ ] **Step 1: Write the failing tests.** Two kinds, and the second is the one Revision 1 lacked:
  - **Cell matrix**, modelled on `temper-principal/tests/matrix.rs` including its
    `every_standing_variant_is_in_the_matrix` trick (a new `Standing` variant must fail the test, not
    slip through). Cells: `linked` (2) × `vaulted` (2) × (the 6 `STATES` + one unrecognized string).
    Assert per spec §2.2: `linked: false` ⇒ `NotLinked` for **every** standing and `vaulted`;
    `linked: true` + not-approved ⇒ `Standing{..}` **including when `vaulted: false`**; the payload is
    the *same* `Refusal` `admit` returns for that input — compare against `admit` directly rather than
    restating the mapping; `approved` + `!vaulted` ⇒ `NotVaulted`; `approved` + `vaulted` ⇒ `Ok`;
    every `Err` has a non-empty reason.
  - **Serialization round-trip over every arm.** `serde_json::to_string` then back, asserting
    equality. Also assert the emitted JSON has **no duplicate keys** and that the `Standing` arm nests
    as `{"reason":"standing","refusal":{"kind":…}}`. Spec §2.1 — the absence of this test is why
    Revision 1's blocker would have shipped.
  - **A pin that only `admit`-reachable `Refusal` variants appear** (spec §2.1): the 5 from
    `admission.rs:37-58`, never `IllegalTransition` / `InsufficientAuthority` / `NoPriorStanding`.

- [ ] **Step 2: Red.** `cargo nextest run -p temper-services --features test-db slack_link_state`

- [ ] **Step 3: Implement.** Exhaustive matches, no `_ =>`. Standing via
  `admit(ev.standing).map_err(|refusal| LinkRefusal::Standing { refusal })?`. Add the kinship doc
  comments in both directions (spec §3.2), and state §3.1's D2 reasoning in `resolve`'s doc.

- [ ] **Step 4: Green**, then `cargo make check`, then commit.

---

### Task 2: The mint query

**Tag: AMEND** — spec §2.3. **Do not start until Step 1 passes.**

**Files:**
- Modify: `crates/temper-services/src/services/slack_grant_vault_service.rs` (`MintOutcome` `:60`,
  `mint_access_token` `:237`, the SELECT `:246-258`, the gate `:268`)

- [ ] **Step 1 (MANDATORY, executable): run the SQL before writing Rust around it.**
  Paste spec §2.3's query into `psql "postgresql://temper:temper@localhost:5437/temper_development"`
  inside `BEGIN; … ROLLBACK;`. Confirm it executes. Then confirm the lock: open the CTE in one
  session, and in another run `SELECT … FROM kb_slack_grant_vault WHERE slack_principal_id = … FOR
  UPDATE NOWAIT` — it must fail with `could not obtain lock on row`.
  **This step exists because Revision 1's query passed every static gate and was illegal to execute.**
  If your adaptation of §2.3 diverges from the spec's text at all, re-run this.

- [ ] **Step 2: Write the failing service tests** in the file's own `#[cfg(test)]` module, using its
  existing helpers (read their real signatures at the bottom of the file):
  - no auth-link row ⇒ `NotLinked` — *not* `NotVaulted`. Unrepresentable today.
  - linked, unvaulted, `denied` ⇒ the standing refusal, **not** `NotVaulted`. Today's vault-rooted
    query cannot reach this cell.
  - `denied` / `requested` / `revoked` / `deactivated` / no-standing-row ⇒ **distinguishable**.
  - **A vault row whose `profile_id` differs from the link's ⇒ `NotVaulted`** (spec §2.3's
    fail-closed correlation). Seed it directly; no production path produces it.

- [ ] **Step 3: Red.** `cargo nextest run -p temper-services --features test-db slack_grant_vault`

- [ ] **Step 4: Implement.** Spec §2.3's CTE shape, with the `profile_id` correlation and the sqlx
  nullability annotations (`v.rt_nonce AS "rt_nonce?"`, `vaulted` as `!`). `MintOutcome` becomes the
  two arms of spec §2.1. Drop the `revoked_at.is_some() ||` disjunct; record commit `3a45b1ab` in a
  comment. **Keep the decision before the cache branch** (`:264-267`).

- [ ] **Step 5: Update the five churned tests — not four.** `slack_grant_vault_service.rs:679`,
  `:707`, `:735`, `:762`, **and `tests/e2e/tests/slack_link_test.rs:1727`**
  (`mint_reports_a_revoked_grant_as_revoked`), whose whole premise is the dropped disjunct. Spec §7:
  the column survives, so either the test survives against a directly-flipped flag or it goes with a
  recorded reason. **Decide it; do not delete it as an unexplained red.**

- [ ] **Step 6: Green**, then regenerate: `cargo sqlx prepare --workspace -- --all-features` then
  `cargo make prepare-services`. Order matters.

- [ ] **Step 7: Cold offline check.**
  `cargo clean -p temper-services && SQLX_OFFLINE=true cargo check -p temper-services --all-targets --features test-db`
  **This proves cache honesty, not SQL validity** — Step 1 is what proves the latter.

- [ ] **Step 8: Commit** after `cargo make check`.

---

### Task 3: The mint wire type

**Tag: AMEND** — spec §4.2.

**Files:** `crates/temper-api/src/handlers/slack_mint.rs` (`:38-57`, `:59-69`, `:71-85`, `:98-114`)

- [ ] **Step 1: Failing test** in the Slack temper-api integration target (`rg -l slack crates/temper-api/tests/`):
  serialized `reason` values are distinct across `not_linked` / `not_vaulted` / standing, and the
  standing arm carries the nested `refusal` object a client can branch on.
- [ ] **Step 2: Red.** `cargo nextest run -p temper-api --features test-db --test <target>`
- [ ] **Step 3: Implement.** Mirror `MintOutcome`'s two arms; keep the `From` impl total. **Delete**
  the false remedy at `:49-51` (spec §1.1). Extend the hand-written redacting `Debug`.
- [ ] **Step 4: Green**, plus `bash .github/scripts/audit-credential-debug.sh` — this file is one of
  its named exemplars.
- [ ] **Step 5: Commit** after `cargo make check`.

---

### Task 4: `link-state` — NO CODE CHANGE (spec §4.1 narrowed)

**Tag: CONFORM** — spec §4.1, as amended 2026-07-23.

**Decision (Pete's call at implementation):** `link-state` stays two-arm (`linked` / `unlinked`);
`mint` carries the full `LinkRefusal` vocabulary. Making `link-state` resolve would be a redundant
second answer whose only new effect is the F10 vault-read on the cheap `SLACK_LINK_SECRET` endpoint —
cost without benefit, since the agent calls `mint` next and `mint` (Tasks 2–3) already delivers every
refusal. See the amended spec §4.1.

**Files:** none. `slack_link.rs:81-124` is already correct for this design.

- [x] **No code.** The current handler returns `Linked{handle}` / `Unlinked{authorize_url}`, which is
  the two-arm design. A linked-but-unmintable human reads as `linked` here and gets the specific
  refusal from `mint`.
- [x] **Coverage already exists.** `mint_reports_not_vaulted_distinctly_from_not_linked`
  (`slack_link_test.rs`) pins exactly this: link-state says `linked` for an unvaulted user while mint
  says `refused`/`not_vaulted`. `a_linked_principal_gets_its_handle_and_mints_no_intent` pins
  mints-no-intent. No new link-state test is needed; the standing-refusal-at-the-wire assertion lives
  on `mint` (Task 3 unit + Task 6 e2e).
- [x] **Done by amendment** — folded into the Tasks 2–3 work; no separate commit.

---

### Task 5: The callback's third page

**Tag: AMEND** — spec §4.3. **No new `SlackLinkOutcome` arm** — see spec §0.1.

**Files:** `crates/temper-api/src/handlers/slack_link.rs` (`run_callback` ~`:225-332`, renderers `:487-511`)

**Read first:** spec §0.1. Revision 1 called the handler's overrule a defect; it is a documented
decision (`slack_link.rs:279-311`) and the rollback stays exactly as it is.

- [ ] **Step 1: Failing e2e test.** An un-approved principal completes the callback: the page does
  **not** contain a bare `Account <em>connected</em>.`, and does contain the standing `reason()`.
  Model the HTML assertions on `slack_link_test.rs:1629-1646`.
  **Plus an escaping test for the new renderer** (spec §4.3) — the incumbents escape (`:501`, `:509`)
  and `UnrecognizedStanding` formats `raw` with `{:?}`, which does not escape `<`/`>`.
- [ ] **Step 2: Red.** **Step 3: Implement** the third renderer beside `connected_page` /
  `not_connected_page`; keep writing both rows in the one transaction.
- [ ] **Step 4: Green.** **Step 5: Commit** after `cargo make check`.

---

### Task 6: The end-to-end test that does not exist today

**Tag: EXTEND** — spec §1.2, §7.

**Files:** `tests/e2e/tests/slack_link_test.rs`

- [ ] **Step 1: One narrative test.** Link an un-approved principal end to end; assert the callback
  page is honest, assert the mint names the standing refusal and **not** `revoked`, then
  `approve_standing` and assert the *same* principal mints a token **with no re-link** — the property
  spec §4.3 chose its design for. **Do not call `approve_standing` at setup**: that call
  (`:866-868`) is the workaround that hid this path.
- [ ] **Step 2: Run**, then **prove it is not vacuous**: revert Task 2's ordering locally, watch it go
  red, restore.
- [ ] **Step 3: Commit** after `cargo make check`.

---

### Task 7: The sealed `VerifiedSlackPrincipal` — DONE (folded into PR 1, commit `c4721ffd`)

**Tag: EXTEND + AMEND** — spec §5. Folded into PR 1 (not a separate PR 2) on Pete's call: it touches
`slack_mint.rs` which PR 1 already changed, and the anti-stack rule made a separate PR mean "wait for
PR 1 to merge." The authz change is the PR's headline; the rigor (trybuild + negative test + guards)
held regardless.

- [x] **`verify_mint_request` + `VerifiedSlackPrincipal` in `slack_mint_service`** (NOT `auth/mod.rs`
  — co-located with the mint logic). Sole constructor, private field, does the FULL verify (spec §5.1
  — the verify MOVED into temper-services).
- [x] **Two design deviations from the plan text, both grounded:**
  - **NOT a per-gate hook in `require_signature_with`** (the plan's Step 4). Putting the seal in the
    shared helper is the F7 leak surface. Instead the mint gate has its OWN verify path; the other two
    gates never call `verify_mint_request`, so cross-gate containment is **structural**, not a
    discipline. The shared plumbing (header extract + body buffer) is factored into
    `buffer_signed_request` so `require_signature_with` is not duplicated.
  - **`audit-signature-secrets.sh` NOT modified** (the plan's Step 7 / files list). The mint gate
    still reads `slack_mint_secret` (to pass it to `verify_mint_request`), so the guard's source scan
    still finds it — the spec's assumption that moving the verify moves what the guard reads did not
    hold under this design.
- [x] **`validate_slack_principal` + constants + tests moved** to `slack_link_service`; still returns
  `BadRequest` (400 distinct from a 401). Handler takes `Extension<VerifiedSlackPrincipal>`, drops
  `Json<SlackMintRequest>`.
- [x] **AMENDED** the stale comment at `slack_mint_service.rs` (the newtype-rejection), keeping §5.3's
  honest limit.
- [x] **Cross-gate containment proven three ways:** unit (`a_link_secret_signature_cannot_mint_a_proof`),
  route-level e2e (`mint_refuses_the_link_state_key`), trybuild forgery (`E0451`).
- [x] **Verified:** trybuild, e2e slack_link_test 22/22, reconcile gate (`internal_saml_test`) 4/4
  unchanged (shared-helper refactor safe), `cargo make check` green.

---

> ### ⛔ Gate: #498 lands before Tasks 8–9
> `mint.ts` exists only on `origin/jct/slack-t4-agent-half`. A gate built on `main` would cover one
> consumer. Spec §10.

---

### Task 8: Generated types for the mention agent — PR 3

**Tag: EXTEND** — spec §6.

**Files:** `tools/cargo-make/main.toml:272-284`; create
`packages/agent-workflows/mention/agent/generated/admission.ts`; modify `…/agent/lib/link.ts:35-37`
and `…/agent/lib/mint.ts` (present only after #498).

- [ ] **Step 1: Second export line** targeting the mention package's own tree, mirroring the existing
  `mkdir -p` guard (`:275`). The package is *"deliberately NOT a bun `workspaces` member"* and cannot
  import from temper-ui's tree; `"imports": { "#*": "./agent/*" }` makes it `#generated/admission.js`.
- [ ] **Step 2: Generate and inspect.** `cargo make generate-ts-types`. Confirm the file-count
  assertion (`:282-283`) guards **both** output dirs — if it counts only temper-ui's, an empty mention
  export passes silently.
- [ ] **Step 3: Point `link.ts` and `mint.ts` at the generated types.**
- [ ] **Step 4: Verify.** `cd packages/agent-workflows/mention && npm install && npm run typecheck && npm run test`
  — **from inside the agent dir**; a root `npm install` inherits the root's bun `overrides` and fails.
- [x] **Step 5: Commit the generated file — it must be tracked**, or Task 9's gate diffs nothing.

**DONE (commit `dc5059f4`). Three deviations from the plan text, each found by running it:**

- [x] **The crate is temper-CORE, not temper-principal.** The plan's `-p temper-principal` export
  emits `admission.ts` and **no `slack_link.ts`** — i.e. everything except `LinkRefusal`, the type
  the agent branches on. It lives in `crates/temper-core/src/types/slack.rs:27`. The plan inherited
  temper-principal from the `Refusal`-shaped framing of §6 and never checked which crate owns the
  outer type.
- [x] **The export is FILTERED to `export_bindings_linkrefusal`**, not a whole-crate run. ts-rs emits
  one test per type and exports each type's TRANSITIVE CLOSURE, so the filter yields exactly
  `slack_link.ts` + the `admission.ts` it imports. Unfiltered, `-p temper-core` deposits **36** files
  into the agent — measured, not estimated. The closure is also why there is no hand-maintained file
  list: a new dependency on `LinkRefusal` emits its file automatically.
- [x] **`ts-rs/import-esm` enabled for that invocation only.** ts-rs otherwise emits
  `from "./admission"`, which is error **TS2835** under the mention package's
  `"moduleResolution": "NodeNext"`. Not enabled repo-wide: temper-ui resolves via its bundler, so a
  global flip rewrites the import line of all 36 of its generated files to fix a problem only this
  consumer has. The two trees therefore differ in import style **by design** — each is regenerated
  and diffed against itself.
- [x] **Step 3 is only half-done, and cannot be finished.** `mint.ts` now imports the generated
  types; **`link.ts` does not and will not** — `LinkState` mirrors `SlackLinkStateResponse` in
  temper-api, which has no ts-rs, so there is nothing generated to point it at. This is the residual
  Step 7 files, and the plan's Step 3 overreached by naming `link.ts:35-37` as if a generated
  counterpart existed.
- [x] **Consequence not in the plan:** `standingReply` now takes the WHOLE generated `Refusal` (nine
  variants), not a hand-narrowed six. The three transition-machine refusals route to the our-bug
  reply and log, handled *explicitly* rather than by a `default:` so the switch stays exhaustive — a
  tenth Rust variant is a compile error in that file.
- [x] **Found in passing:** `packages/temper-ui/src/lib/types/generated/slack_link.ts` had never been
  generated (Task 1 added the derives and deliberately did not regenerate). The gate would have gone
  red on `main` for this alone.

---

### Task 9: `check-ts-rs-drift.sh` — general, not Slack-shaped — PR 3

**Tag: EXTEND** — spec §6.

**Files:** create `.github/scripts/check-ts-rs-drift.sh` and `test-check-ts-rs-drift.sh`; modify
`tools/cargo-make/main.toml`; modify `.github/workflows/code-quality.yml`.

**Read first:** spec §6 — a Slack-named gate over `admission.ts` covers the wrong types, because the
`status` discriminants live in temper-api, which has **no `ts-rs`**. And
`.github/scripts/check-temper-ts-drift.sh` in full: regenerate then `git diff --exit-code`, and
**assert the artifact is tracked before diffing** (`:26-34`).

- [ ] **Step 1: Self-test first.** It must prove the gate goes **red**: patch a type in a temp copy,
  run the gate, assert non-zero, restore, assert zero.
- [ ] **Step 2: Red.** `bash .github/scripts/test-check-ts-rs-drift.sh`
- [ ] **Step 3: Write the gate** over **everything** `generate-ts-types` writes — both output trees.
  Error message must say regenerate **and stage**, since the diff is against git.
- [ ] **Step 4: Green**, both.
- [ ] **Step 5: Wire both surfaces.** `[tasks.check]` dependencies (`main.toml:27-37`), and the
  **`rust-quality`** CI job — *not* `guard-tests`, whose header says *"Pure bash, no toolchain"*
  (`code-quality.yml:119`) while this needs cargo. The **harness** goes in `guard-tests`.
- [ ] **Step 6: Prove the wiring.** Run `cargo make check`, confirm the task appears; break a type,
  watch it red; restore.
- [ ] **Step 7: File the residual** — the temper-api `status`-tag gap (spec §6, §9), with the evidence
  and the options. Do not imply coverage that does not exist.
- [x] **Step 8: Commit.**

**DONE (commit `366547a8`). Deviations, all in the direction of covering more:**

- [x] **The tree list is DERIVED, not written into the gate.** The plan said "over both output
  trees"; hardcoding two paths would be the same two-copies-of-the-truth drift this gate exists to
  stop, one level up. The gate greps main.toml's `TS_RS_EXPORT_DIR=…` lines, so a third consumer is
  covered with no edit to the gate — and finding **zero** trees is a hard failure, because the loop
  would otherwise run zero times and exit 0 having checked nothing.
- [x] **`git status --porcelain`, not `git diff --exit-code`** (the model script's form). The diff
  form reports only *tracked* changes, so a newly derived type — a `.ts` nobody has committed — is
  invisible to it. That is exactly the state temper-ui's `slack_link.ts` was in. `status` covers
  modified, deleted and untracked in one predicate.
- [x] **The tracked-check is PER TREE**, not once: `git status` over a path git does not know about
  reports nothing, so one gitignored tree would pass forever while checking nothing.
- [x] **Harness case the plan did not ask for: a FAILING generator must fail the gate.** The
  generator is a cargo build, and its most ordinary failure is a Rust compile error — precisely when
  someone is midway through changing the types this gate protects. Exiting 0 there reports "up to
  date" on the strength of files nothing regenerated, which is indistinguishable from a real pass.
  Verified: the gate exits 105.
- [x] **Two harness-only seams** (`TS_RS_DRIFT_REPO_ROOT`, `TS_RS_DRIFT_GENERATE_CMD`), following
  `audit-signature-secrets.sh`'s `MIDDLEWARE_FILE` idiom. Stubbing the generator is precisely what
  lets the harness stay pure-bash and live in `guard-tests` while the gate lives in `rust-quality` —
  the tension the plan named but did not resolve. No CI job sets either.
- [x] **`cargo-make` added to `rust-quality`'s existing `taiki-e/install-action` step.** The job runs
  discrete `cargo` commands and had no cargo-make; the gate invokes `cargo make generate-ts-types`
  deliberately — the same entry point a developer runs — so it cannot check a different generator
  than the one people use.
- [x] **Bite-verified on the real repo, not only on fixtures:** a one-word change to `LinkRefusal`'s
  type-level doc comment turned the gate red and named BOTH trees; restoring turned it green. (A
  first probe changed a *variant* doc comment and correctly did **not** trip it — ts-rs emits
  type-level docs into a union, not per-variant ones.)
- [x] **Step 7 residual filed** as temper task `019f910b-579b-74c2-bf05-702aaed0a011`, and cited in
  the gate's own header so a reader hits the limit where they would otherwise infer coverage.

---

## Self-review

**Spec coverage.** §1 → Tasks 2, 3, 6. §2.1 → Task 1 (incl. the round-trip test). §2.2 → Task 1.
§2.3 → Task 2 (Step 1 executable). §2.4 → Task 2. §3/§3.1/§3.2 → Task 1. §4.1 → Task 4 (both
widenings). §4.2 → Task 3. §4.3 → Task 5 (incl. escaping). §5.1/§5.2/§5.3 → Task 7. §6 → Tasks 8–9.
§7 → distributed; the missing e2e is Task 6; the five-site churn is Task 2 Step 5. §8 → Task 2
Steps 6–7, Task 3 Step 4. §9 → no tasks, correctly; Task 9 Step 7 files one residual. §10 → the PR
gate before Task 8.

**Fixed relative to Revision 1:** the arity pin is gone (it disabled D2's alarm); the fifth churned
test is named; the executable SQL step is mandatory and marked as the only thing that can catch what
the static ladder cannot; `NoRefreshToken` is gone; the gate moved to `rust-quality` and became
general; the sequencing gate before Task 8 is explicit.

**Type consistency.** `resolve` / `LinkEvidence` / `Mintable` / `LinkRefusal` in Tasks 1–4.
`verify_mint_request` / `VerifiedSlackPrincipal` only in Task 7. `MintOutcome`'s two arms in Tasks
2–3.

**Placeholder scan.** No TBD/TODO. Code blocks only in Global Constraints (quoted from disk) and
Task 1's interface block (fixed by spec §2.1–§2.2).
