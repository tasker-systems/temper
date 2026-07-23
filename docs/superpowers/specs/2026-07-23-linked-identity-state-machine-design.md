# Linked identity as a state machine — the Slack auth flow in the admission idiom

**Status:** design, 2026-07-23. Goal `019f6344-01a5-7fc0-9e22-a80585f801fc` (@temper on Slack).
**Character:** behaviour change on two undocumented internal wire contracts, plus a new CI gate.
No DB migration. No `openapi.json` / gem / `schema.ts` regeneration. **Does** require a `.sqlx`
regeneration — §2.3 changes query text (see §8).
**Successor to:** the principal-admission state machine
(`2026-07-20-principal-admission-state-machine-design.md`) and the admin-authz enclosure
(`2026-07-22-admin-authz-enclosure-design.md`), whose idioms this instantiates for a second domain.

---

## 1. The finding

`mint_access_token` collapses seven distinct facts into one outcome, and the fact the outcome is
*named* after cannot happen.

`crates/temper-services/src/services/slack_grant_vault_service.rs:268`:

```rust
if row.revoked_at.is_some() || row.standing.as_deref() != Some("approved") {
    return Ok(MintOutcome::Revoked);
}
```

**The left disjunct is dead in production.** Every writer of `kb_slack_grant_vault.revoked_at` is a
test — `slack_grant_vault_service.rs:686`, `:840`, `tests/e2e/tests/slack_link_test.rs:1736` — and
the two production sites write `NULL` (`:179`, `:188`). Commit `3a45b1ab` says so in its subject:
*"drop the never-wired vault revoke flag in favour of disconnect's delete"*. Disconnect `DELETE`s the
row (`slack_disconnect_service.rs:161`), which yields `NotVaulted`.

**The right disjunct is six facts, not one.** `temper_principal::admit` already returns each as a
reason-carrying `Refusal` (`crates/temper-principal/src/admission.rs:37-59`): `Denied`, `Requested`,
`Revoked`, `Deactivated`, `NoStanding`, `UnrecognizedStanding { raw }`. The gate consults none of
them; it compares a raw `Option<&str>` against a string literal.

So in production `MintOutcome::Revoked` means exactly *"this human's standing is not approved"* — and
never what it says.

### 1.1 The consequence is a false remedy, shipped to a human

`crates/temper-api/src/handlers/slack_mint.rs:49-51`:

```rust
/// A vault row exists but is not mintable: explicitly revoked, or the profile deactivated.
/// The user must re-link; retrying will never succeed.
Revoked,
```

Re-linking calls `store_grant`, which touches `kb_slack_grant_vault` and sets `revoked_at = NULL`
(`:188`). **It never touches `kb_principal_standing`.** For every reachable cause of this arm,
re-linking provably cannot change the outcome. The mention agent renders exactly that instruction on
the unmerged T4 branch (`packages/agent-workflows/mention/agent/lib/identity.ts:151-158`):

> *"Your temper access from Slack has been revoked… If that wasn't deliberate, reconnecting will
> restore it."*

### 1.2 And on a fresh instance it is the modal first-link outcome

`crates/temper-principal/src/transition.rs:35-38` births every `OauthFirstLogin` human `Denied`. The
Slack callback's only identity gate is Level 1, which passes `Denied` **deliberately**
(`crates/temper-services/src/auth/mod.rs:249-251`):

> *"A `Deactivated` standing bars authentication entirely; `Denied` and `Revoked` deliberately pass
> Level 1 so they can still reach the auth-only request/review routes — only Level 2
> (`require_system_access`) refuses them."*

Level 2 is never applied to the callback: `slack_link_public_routes()` is merged with no layer
(`crates/temper-api/src/routes.rs:426`), by design (`:326-330`, *"Ungated by design: it is the IdP's
redirect target"*).

So a `Denied` human completes the whole browser link, gets a grant vaulted, and is shown
**"Account connected."** (`slack_link.rs:494-503`) — after which every mention fails permanently with
a remedy that cannot work.

The repo's own e2e suite documents this as a workaround rather than a bug
(`tests/e2e/tests/slack_link_test.rs:866-868`):

```rust
// D-A: the mint gate is `standing = 'approved'`, and the callback births `Denied` — approve so
// the happy-path mint below returns a token rather than `Revoked`.
approve_standing(&pool, profile_id_for_sub(&pool, sub).await).await;
```

**No test drives the un-approved link path end to end.** Every mint test seeds `approved` first.

### 1.3 The link half has the mirror-image problem

- **13 refusal sites → 11 distinct sentences → 1 status code.** `callback` returns `Response`, not
  `Result` (`slack_link.rs:164-169`), so every outcome including success is HTTP 200 `text/html`.
  The e2e tests discriminate by grepping rendered HTML for `Account <em>connected</em>.`, because
  there is no typed outcome to assert on.
- **`SlackLinkOutcome` has two variants where the callback has three post-auth outcomes.** "Linked
  but un-vaultable" — the IdP returned no refresh token (`slack_link.rs:279-311`) — is not a
  variant. The service returns `Linked`, having written its row; the handler overrules it and drops
  the transaction.
- **The one typed refusal is erased at the boundary.** `AlreadyLinkedToAnotherProfile` is compared
  with `==` at `slack_link.rs:252` and converted to a `String`.

### 1.4 What the model is today

Six Rust types and two hand-written TypeScript mirrors, none of which reference each other:

| Incumbent | Arms | Site |
|---|---|---|
| `SlackLinkOutcome` | `Linked` / `AlreadyLinkedToAnotherProfile` | `slack_link_service.rs:159` |
| `SlackLinkStateResponse` | `Linked{handle}` / `Unlinked{authorize_url}` | `slack_link.rs:60` |
| `MintOutcome` | `Token` / `Revoked` / `NotVaulted` | `slack_grant_vault_service.rs:60` |
| `SlackMintResponse` | wire mirror of the above, own redacting `Debug` + `From` | `slack_mint.rs:38-57` |
| `LinkState` (TS) | hand-written mirror of row 2 | `mention/agent/lib/link.ts:35-37` |
| `MintOutcome` (TS) | hand-written mirror of row 4 | `mention/agent/lib/mint.ts:31-45` (T4 branch) |

The **cartesian product of rows 2 and 4 is computed at the far edge, in TypeScript** — it is the
four-row table in PR #498's description. The state machine exists; it lives in the agent, and nothing
type-checks it against the Rust.

---

## 2. The model

### 2.1 Shape: `Result`, mirroring `admit`

The decision is `Result<ActiveLink, LinkRefusal>`, structurally identical to
`admit(…) -> Result<AdmittedPrincipal, Refusal>` (`admission.rs:37`). Two outcomes; the refusal
carries the reason.

```rust
/// Evidence the service gathered. NO IDENTIFIERS — see §3.
pub struct LinkEvidence<'a> {
    pub linked: bool,
    pub vaulted: bool,
    /// Raw text, parsed inside the machine — as `admit` takes it (`admission.rs:37`).
    pub standing: Option<&'a str>,
}

/// Proof that a linked identity may act. SEALED: private field, no public constructor,
/// no `Default`, no `From` — `AdmittedPrincipal`'s shape (`admission.rs:14-25`).
pub struct ActiveLink { standing: Standing }

pub enum LinkRefusal {
    NotLinked,
    /// Verbatim, never restated. Six reasons, each with `reason()`.
    Standing(Refusal),
    NotVaulted,
}

pub fn resolve(ev: LinkEvidence<'_>) -> Result<ActiveLink, LinkRefusal>;
```

`LinkRefusal` takes `Refusal`'s serde and ts-rs derives (`refusal.rs:19-23`) —
`#[serde(rename_all = "snake_case", tag = "kind")]` plus `ts_rs::TS` under `typescript`.

**Deliberately NOT `utoipa::ToSchema`.** Both routes that carry this type are allowlisted out of
`openapi.json` (`check-openapi-routes.sh:63-64`), so the derive would buy nothing and imply a public
contract that does not exist — which is also why §8 records no OpenAPI regeneration. If either route
is ever published, adding `ToSchema` requires a per-variant
`#[cfg_attr(feature = "web-api", schema(title = "…"))]` for the reason `refusal.rs:13-18` gives:
without them openapi-generator names anonymous `oneOf` branches positionally, and they renumber when
a variant is inserted rather than appended.

Every variant carries a non-empty `reason()`, asserted across the whole cell space, per
`refusal.rs:62-66`.

### 2.2 The resolution order is forced, and it is the fix

**`NotLinked` first — by data availability, not preference.** Standing is a property of the *temper
profile*. An unlinked Slack principal has no profile to look up, so standing is unknowable until the
link exists. The order is a consequence of the domain, not a policy choice.

**`Standing` before `NotVaulted` — and this is the correction.** For a linked-but-unvaulted human,
"re-link" is the right remedy. For a `Denied` human it is the false one we ship today. Checking
standing first means the actionable sentence wins.

### 2.3 Consequence: the mint query must re-root at the identity

Today's query roots at the vault (`slack_grant_vault_service.rs:246-258`):

```sql
FROM kb_slack_grant_vault v
LEFT JOIN kb_principal_standing s ON s.profile_id = v.profile_id
WHERE v.slack_principal_id = $1
```

With no vault row there is no `profile_id`, so **standing is structurally unreachable** — which is
why `NotVaulted` currently short-circuits before it, and why mint cannot tell "never linked" from
"linked, not vaulted". §2.2's ordering is not implementable against this shape.

It re-roots at the identity row:

```sql
FROM kb_profile_auth_links l
LEFT JOIN kb_slack_grant_vault v  ON v.slack_principal_id = l.auth_provider_user_id
LEFT JOIN kb_principal_standing s ON s.profile_id = l.profile_id
WHERE l.auth_provider = 'slack' AND l.auth_provider_user_id = $1
```

One read, three facts, refusals ordered by usefulness rather than by query shape. `NotLinked` becomes
representable at the mint for the first time. `FOR UPDATE OF v` still locks the vault row for the RT
spend. The unique constraints both joins rely on exist:
`kb_profile_auth_links_auth_provider_auth_provider_user_id_key` and
`kb_slack_grant_vault_slack_principal_id_key` (live DDL).

**CONFORM:** the ordering rule *"not-mintable checks first, before any cached token is decrypted or
the RT is spent"* (`:264-267`) is preserved — `resolve` runs on the row before the cache branch at
`:273`.

### 2.4 `revoked_at`: drop the disjunct, keep the column

The `revoked_at.is_some()` term is removed from the gate; `LinkRefusal` has no arm for it. The
**column stays** — dropping it is a destructive migration for a column that costs nothing sitting
`NULL`, and this design carries no migration. The spec records that soft-revoke was superseded by
disconnect's `DELETE` (commit `3a45b1ab`) so the next reader does not re-derive it.

### 2.5 `SlackLinkOutcome` gains its third arm

`NoRefreshToken` — the state at `slack_link.rs:279-311` that the service currently reports as
`Linked` and the handler overrules. Three arms, matching the callback's three outcomes (§4.3).

---

## 3. Where it lives

**`crates/temper-principal/src/linked_identity.rs`.**

Named for the general concept, not for Slack: goal `019f6344` Phase 2 brings a Linear pull, and goal
`019f5e07` brings external systems as emitters. A per-channel crate would need a sibling per
integration; a `slack.rs` module would need a rename. If the module later outgrows this crate it
lifts into a `temper-integrations` crate intact — a file move, not a redesign.

**This honours the crate's constraint literally, not by exception.** `lib.rs:11-13` says the crate
*"performs no I/O, holds no identifiers, and never resolves a credential."* The `slack:<team>:<user>`
string is the **lookup key the service uses to gather evidence** — the judgment itself needs only
three facts, so `LinkEvidence` carries no identifier at all.

**CONFORM — the crate boundary is the enforcement.** `lib.rs:7-10`:

> *"Every `match` over [`Standing`] here is exhaustive with no `_ =>` arm, so adding a state becomes
> a compile error at every decision site (spec §7 obligation 3). That property is what the crate
> boundary buys; it cannot be bought by discipline inside a larger crate."*

Siting this in `temper-services` was considered and rejected for exactly that reason: every
neighbouring file has `sqlx`, so nothing would stop a future edit putting a query inside the decision
function. Purity by convention is the failure mode this design exists to remove.

**CONFORM — `Cargo.toml:6-12` stays honest.** The module adds no dependency: `Refusal` and `Standing`
are already local, and `LinkEvidence` takes no `ProfileId` (which would drag `sqlx` back in via
`temper-core`).

**The service seam mirrors `standing_service`** (`standing_service.rs:1-10`): services gather
evidence, `temper-principal` decides, and every identifier stays on the services side. Note
`standing_service::admit` returns `Result<_, Refusal>` and **not** `ApiResult` (`:58`) — the typed
refusal escapes the service layer intact. The Slack seam does the same.

---

## 4. The call sites

### 4.1 Both internal routes keep their separate secrets

Collapsing `link-state` and `mint` into one call was considered — PR #498's four-row table is
precisely their product — and **rejected**. `mention/agent/lib/mint.ts:8-13`:

> *"`SLACK_LINK_SECRET` gates an endpoint that answers a question — 'is this principal linked?'.
> `SLACK_MINT_SECRET` gates one that hands back a token carrying that human's ENTIRE temper reach.
> Sharing a key would make compromise of the cheap capability yield the expensive one."*

Instead: **one vocabulary, two renderings.** `link-state` returns the resolved state *without* a
token; `mint` returns it *with*. The agent's cartesian product collapses because both endpoints speak
the same enum — not because the endpoints merge.

**Recorded deliberately, not as a side effect:** `link-state`'s answer widens from *linked/unlinked*
to *linked/unlinked + why-refused*, which tells a `SLACK_LINK_SECRET` holder a linked human's
standing. Judged acceptable — that holder can already enumerate who is linked, and the agent cannot
state the true remedy without it — but it is a widening and it is written down.

### 4.2 `slack_mint`

Takes the sealed principal proof (§5) instead of `Json<SlackMintRequest>`. `mint_for_mention`
(`slack_mint_service.rs:59`) returns `Result<ActiveLink, LinkRefusal>`; the handler renders it.

### 4.3 The callback gets a third page

This is where the §1.2 trap dies. **Not by refusing the link** — refusing means the human must
re-link after approval, for no gain. By writing both rows in the same transaction as today and
rendering honestly:

| Outcome | Page |
|---|---|
| linked, vaulted, `ActiveLink` | "Account connected." (as today) |
| linked, vaulted, `LinkRefusal::Standing(_)` | connected — **and you cannot act until an admin approves**, carrying `Refusal::reason()` |
| `AlreadyLinkedToAnotherProfile` / `NoRefreshToken` | "Not connected." (as today) |

The moment an admin approves, it works with no further user action.

**CONFORM:** the one-transaction invariant for `{identity row, vaulted grant}` is untouched
(`slack_link.rs:225-332`, proven by `link_is_rolled_back_with_its_caller_transaction`,
`slack_link_service.rs:474`).

---

## 5. The sealed transport proof

`slack_mint_service.rs:32-37` considered and rejected a `VerifiedSlackPrincipal` newtype:

> *"its constructor must be `pub` for the handler (a different crate) to call it, so any code could
> mint the proof alongside the claim… If temper ever wants this structurally, the honest shape is for
> the signature middleware to insert a non-constructible token into request extensions, which is a
> larger change than T4 warrants."*

**That premise is now stale, and the shape it names as honest is the incumbent pattern.** It assumes
the *handler* mints. In the pattern that shipped afterwards, the middleware receives an
already-minted value from the sealing crate and the handler only extracts:

- `crates/temper-api/src/middleware/auth.rs:119` — `request.extensions_mut().insert(authed)`, where
  `authed` came from `temper_services::auth::authenticate_token` (`:82`)
- `crates/temper-api/src/middleware/auth.rs:34-37` — `parts.extensions.get::<AuthenticatedProfile>().cloned()`
- `crates/temper-services/src/auth/mod.rs:285-288` — private fields, in a `pub` module of another crate
- `crates/temper-services/tests/compile_fail/forge_authenticated_profile.rs` — trybuild, compiled as
  an **external** crate, proves the struct literal is `E0451`

Constructible only inside `temper-services`; extractable in `temper-api`; forgery a compile error.
Chronology confirms the comment was accurate when written: commit `e45ec167` (T4, the comment)
predates `c740b399` (sealed `SystemAdmin`) and `7ae32970` (sealed `AuthenticatedProfile`).

**AMEND** — the stale comment is corrected in place as part of this work.

### 5.1 Shape

`require_slack_mint_signature` already buffers the body and re-attaches it
(`internal_auth.rs:90`), so it parses the principal from the **verified** bytes, mints the proof
inside `temper-services`, and inserts it. The handler drops its `Json` extractor entirely.

```rust
// temper-services — the sealing crate
pub struct VerifiedSlackPrincipal { id: String }   // private field
impl VerifiedSlackPrincipal { pub fn id(&self) -> &str { &self.id } }
```

Requires `#[derive(Clone)]` if extracted by value via `FromRequestParts` — `auth.rs:37` calls
`.cloned()`, and `SystemAdmin` derives `Debug` only (`auth/mod.rs:353`), which is why only
`&SystemAdmin` is obtainable there.

### 5.2 What this claims, precisely

It does **not** make the string more trustworthy. `slack_mint_service.rs:22-24` remains correct:

> *"Provenance is extrinsic. Handed `"slack:T123:U456"`, no function can tell whether it was read off
> a Slack-signed webhook or typed by an attacker; the string is identical either way."*

What becomes impossible is **calling the mint with a principal that did not come through the gate**.
That is the enclosure's class of bug — *"forgot to gate an internal call path"* — not a wire-level
upgrade. Possession of `SLACK_MINT_SECRET` remains the wire-level enforcement, as
`internal_auth.rs:158-161` states.

**CONFORM:** `ScopedAuthority` / `Authorized<A>` are **not** used here — they are `pub(crate)` to
`temper-services` (`authz/mod.rs:54,99`) and unreachable from `temper-api`. The `auth::`-style seal
(pub struct, private fields, pub module) is the one that crosses the crate boundary.

---

## 6. The contract gate

Both internal routes are deliberately excluded from `openapi.json` — `/internal/slack/link-state` and
`/internal/slack/mint` are on the allowlist in `.github/scripts/check-openapi-routes.sh:63-64`. Their
only consumers are **hand-written** TypeScript unions (`link.ts:35-37`, `mint.ts:31-45`).

**A Rust-side variant rename compiles clean, passes `openapi-check`, `openapi-routes-check`,
`openapi-rb-drift` and `openapi-ts-drift`, and breaks the agent at runtime.** It is the one contract
in the repo with no gate at all — and typed exhaustive states enforced on one side of a boundary are
half a design.

Putting the routes into `openapi.json` was rejected: the allowlist exists to keep internal
server-to-server routes out of the public contract and both SDKs.

### 6.1 Generation

`temper-principal` is already in `generate-ts-types` (`tools/cargo-make/main.toml:281`), exporting to
`admission.ts`. But `TS_RS_EXPORT_DIR` points at
`packages/temper-ui/src/lib/types/generated/` — and **the mention package is deliberately not a bun
workspace member** (CLAUDE.md: *"workspace-isolated — deliberately NOT a bun `workspaces` member"*),
so it cannot import across that boundary.

`TS_RS_EXPORT_DIR` is per-invocation, so `generate-ts-types` gains a second `-p temper-principal`
line targeting the mention package's own tree. The generated file lands **inside** the package and is
imported locally — no cross-package import, no coupling to temper-ui's toolchain, isolation intact.

### 6.2 `check-slack-contract-drift.sh`

Modelled on `check-temper-ts-drift.sh`, including the two things that script learned the hard way:

- **Regenerate, then `git diff --exit-code`** — compares against git, not against a fresh build.
  Corollary, already documented in CLAUDE.md: a just-correctly-regenerated artifact still fails while
  unstaged. The error message says so.
- **Assert the artifact is tracked before diffing.** `git diff --exit-code -- <path>` exits 0 when
  the path matches nothing, so an untracked or gitignored target makes the gate pass forever while
  checking nothing. `check-temper-ts-drift.sh:26-34`: *"A gate that cannot fail is not a gate; make
  that state loud instead of green."*

**It never skips.** It needs `cargo test -p temper-principal --features typescript` — a compile, not
just Node. Acceptable: `temper-principal` has no `sqlx` and no `ort`, `cargo make check` already
builds the workspace for clippy and docs, so the marginal cost is small. Named here so the trade is
explicit rather than discovered.

### 6.3 Wiring

- **`cargo make check`** — a new `slack-contract-drift` task added to `[tasks.check].dependencies`
  (`tools/cargo-make/main.toml:27-37`), beside `openapi-rb-drift` and `openapi-ts-drift`, for the
  reason `:18-26` already gives about products of the router.
- **CI** — a step in the existing `guard-tests` job (`code-quality.yml:121`, on `main` since #501),
  which is where the `audit-*` harnesses already run.
- **A self-test** — `test-check-slack-contract-drift.sh`, matching the four existing
  `test-audit-*.sh` harnesses that `guard-tests` runs. It proves the gate goes red when the thing it
  protects breaks: patch a Rust variant, observe red, restore, observe green. A guard whose failure
  path is never exercised is the gap #498's audit found in `audit-route-auth.sh`.

This closes one bounded instance of a known repo gap — CLAUDE.md records that there is **no CI gate
on ts-rs type drift** at all. It does not close the general case, and does not claim to.

---

## 7. Testing

| Test | Mirrors |
|---|---|
| Full-cell matrix: `linked` × `vaulted` × (5 standings + absent + unrecognized). Every cell decided; every refusal carries a non-empty `reason()` | `temper-principal/tests/matrix.rs` — `every_cell_is_decided_and_every_refusal_carries_a_reason` |
| Signature test pinning `resolve`'s parameter, so a future "and also check X" conjunction fails loudly rather than silently widening the decision | `admit_reads_standing_and_nothing_else` (`admission.rs:102-109`) |
| trybuild fixtures forging `VerifiedSlackPrincipal` and `ActiveLink` | `tests/compile_fail/forge_system_admin.rs`; feature-gated `trybuild`, `.stderr` committed |
| **The test that does not exist today:** link an un-approved principal end to end; assert the callback does *not* render bare "Account connected."; assert mint names the standing refusal, not `revoked` | — (§1.2) |
| The `guard-tests` self-test for the new drift gate | `test-audit-route-auth.sh` and siblings |

**Expected churn, which is the deliverable:** `mint_refuses_a_profile_without_approved_standing`
(`slack_grant_vault_service.rs:762`) and its siblings at `:679`, `:707`, `:735` currently assert
`MintOutcome::Revoked`. Their assertions become named refusals. A test that still passed unchanged
would mean the collapse survived.

---

## 8. Blast radius

Established by grounding, not assumed.

| Type | Exposure | Regenerates |
|---|---|---|
| `SlackLinkOutcome` | purely internal — one consumer (`slack_link.rs:252`) | nothing |
| `SlackLinkStateResponse` | wire, allowlisted out of `openapi.json` | the TS mirror (§6) |
| `MintOutcome` + `SlackMintResponse` | wire, allowlisted out | the TS mirror (§6) |

No `openapi.json`, no gem, no `clients/temper-ts/src/generated/schema.ts`, no substrate payload
snapshot, no migration. **No `.sqlx` churn** — §2.3 changes query *text*, so the workspace cache and
`crates/temper-services/.sqlx` both need `prepare`; `tests/e2e/.sqlx` holds one
`kb_profile_auth_links` entry and is unaffected unless e2e SQL changes. Ritual order per CLAUDE.md:
`cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`.

**Two CI guards hardcode the type names.** `.github/scripts/audit-credential-debug.sh:7,69,164,181`
cites `MintOutcome` / `NewGrant` / `SlackMintResponse` as its canonical redacting-`Debug` exemplars
and carries a literal `BASELINE` list at `:73-79`; `test-audit-credential-debug.sh:97,101` fixtures
the same names. Renaming restales a security guard's failure message and its self-test. The redacting
`Debug` on the token-carrying arm must survive the refactor — that guard exists because a derived
`Debug` once leaked a credential.

---

## 9. Out of scope — named, not dropped

- **The disconnect half and `IdpRevocation`.** Six committed artifacts, a **shipped immutable
  migration** (`20260719000020_slack_disconnect_event.sql`, which inlines the payload schema
  byte-for-byte), and a live `kb_event_types.payload_schema` row. A variant change there needs a new
  forward migration, and `verify_ledger_roundtrip` (`payloads.rs:1139-1141`) must keep deserializing
  already-persisted rows. Recorded so nobody folds it in casually.
- **Dropping the `revoked_at` column** (§2.4) — destructive migration, operator-run.
- **The intent lifecycle.** `consume_intent` runs on the pool (`slack_link_service.rs:59`) 38 lines
  before `pool.begin()` (`slack_link.rs:235`), so the nonce burn is outside the transaction:
  `consumed` conflates *succeeded*, *refused*, and *rolled back*. Also `create_intent` is unbounded —
  no per-principal cap, no dedup, plaintext PKCE verifier per mention.
- **`_event_append` performs no `payload_schema` validation.** The registry schema is descriptive;
  the only thing holding the ledger to the enum's spellings is `IdpRevocation::as_str` at one call
  site. A ledger concern, orthogonal to this axis.
- **MCP parity.** Consistent with the admission arc, where it was held as lower tier.

---

## 10. Consequence for PR #498

#498 is now TypeScript-only (19 files; the Rust half shipped in #496/#501). Its
`revokedPrompt`/`notVaultedPrompt` (`identity.ts:132,151`) become a match over the new refusal arms,
and its four-row table becomes one switch over generated types. `channels/slack.ts:137,143` reads
`link.handle` *inside* the mint switch — a cross-enum coupling that this unification changes.

**It should rebase onto this rather than merge first.** Merging first ships the §1.1 false remedy to
the only surface that shows it to a human.

The three dependencies named in its draft comment have all landed since (Phase 2 A1 repointed
`slack_grant_vault_service`'s `is_active` reader onto standing; `is_system_admin` moved to
`kb_principal_governance`; `access_mode` retired and dropped in #515), so the original reason to hold
it is spent. This is a different reason.

---

## 11. Adjacent, filed separately

`admin_disconnect_slack_principal` (`slack_disconnect_service.rs:267`) is a Bucket-1 system-authority
act still gated by a bare bool, which the enclosure's audit missed — task
`019f8ec3-793f-7c52-9378-47dda5d90a5d`. It takes `&SystemAdmin` and reads `admin.actor()`. That is
the enclosure's pattern, in the disconnect half, and it is independent of this design.
