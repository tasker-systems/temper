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

## PR boundaries (spec §10)

**PR 1** = Tasks 1–6 (the behavioural change). **PR 2** = Task 7 (the sealed proof).
Then **#498 rebases and lands**. **PR 3** = Tasks 8–9 (generation + gate), last, because `mint.ts`
exists only on #498's branch. Confirm with Pete before opening any.

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

### Task 4: `link-state` speaks the same vocabulary

**Tag: AMEND** — spec §4.1, **including both recorded widenings**.

**Files:** `crates/temper-api/src/handlers/slack_link.rs` (`:60`, `:81-124`)

- [ ] **Step 1: Failing e2e test** beside `a_linked_principal_gets_its_handle_and_mints_no_intent`
  (`slack_link_test.rs:731`): a linked-but-`denied` principal's response names the standing refusal,
  **and still mints no intent** — that test's existing property must not regress.
- [ ] **Step 2: Red.** `cargo build -p temper-cli --bin temper && cargo make test-e2e-embed`
- [ ] **Step 3: Implement.** Render the resolved state minus the token. Keep the `:96-98` read-first
  short-circuit, whose comment calls that ordering *"the whole fix"* for a prior bug.
  **Note the second widening (spec §4.1b):** this handler now reads `kb_slack_grant_vault` for
  `vaulted`. It needs only a boolean — keep it one.
- [ ] **Step 4: Green.** **Step 5: Commit** after `cargo make check`.

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

### Task 7: The sealed `VerifiedSlackPrincipal` — PR 2

**Tag: EXTEND + AMEND** — spec §5. **Read §5.1 first: the verification moves into temper-services, or
the seal is decorative.**

**Files:**
- Modify: `crates/temper-services/src/auth/mod.rs` (or a sibling) — `verify_mint_request` + the type
- Modify: `crates/temper-api/src/middleware/internal_auth.rs` — per-gate hook (`:39-92`, `:166-179`)
- Modify: `crates/temper-api/src/handlers/slack_mint.rs` — proof extractor
- Modify: `crates/temper-services/src/services/slack_mint_service.rs:32-37` — AMEND the comment
- Modify: `.github/scripts/audit-signature-secrets.sh` — it reads gate/secret pairing **from source**
- Create: `crates/temper-services/tests/compile_fail/forge_verified_slack_principal.rs` + `.stderr`

- [ ] **Step 1: trybuild fixture**, shaped like `forge_system_admin.rs`. Generate the `.stderr` by
  running, not by hand — the existing fixture's comment guesses `E0603` while its snapshot records
  `E0423`.
- [ ] **Step 2: Red.** `cargo make test-trybuild`
- [ ] **Step 3: Implement `verify_mint_request` in temper-services** (spec §5.1) — it both verifies
  the HMAC and mints the proof; passing the signature is the only way to obtain one. Private field,
  accessor, no other constructor.
- [ ] **Step 4: Per-gate hook in `require_signature_with`** (spec §5.2). The shared helper serves all
  three gates; parsing-and-inserting inside it would mint the proof for `INTERNAL_RECONCILE_SECRET`
  and `SLACK_LINK_SECRET` holders too. **Write the negative test**: a proof obtained behind the link
  gate must fail.
- [ ] **Step 5: Handler** takes the proof. `validate_slack_principal` moves onto this path and **must
  keep returning `BadRequest`** — `mint_rejects_a_malformed_principal` (`slack_link_test.rs:1750-1761`)
  asserts 400 while every other refusal here is `Unauthorized`.
- [ ] **Step 6: AMEND the comment** at `slack_mint_service.rs:32-37`. Rewrite, do not delete — say
  what is now true, why it changed, and keep §5.3's honest limit. A reader must not conclude this
  closed more than it did.
- [ ] **Step 7: Move the signature-secrets guard** with the verify, and run it.
- [ ] **Step 8: Green** — trybuild, the Slack e2e suite, `bash .github/scripts/audit-route-auth.sh`
  (per-builder, both `create_app` and `create_internal_app`).
- [ ] **Step 9: Commit** after `cargo make check`.

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
- [ ] **Step 5: Commit the generated file — it must be tracked**, or Task 9's gate diffs nothing.

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
- [ ] **Step 8: Commit.**

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
