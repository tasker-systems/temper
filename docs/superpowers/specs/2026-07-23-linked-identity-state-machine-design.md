# Linked identity as a state machine — the Slack auth flow in the admission idiom

**Status:** design, 2026-07-23. **Revision 2** — see §0. Goal
`019f6344-01a5-7fc0-9e22-a80585f801fc` (@temper on Slack).
**Character:** behaviour change on two undocumented internal wire contracts, plus a CI gate.
No DB migration. No `openapi.json` / gem / `schema.ts` regeneration. **Requires** a `.sqlx`
regeneration — §2.3 changes query text (see §8).
**Successor to:** the principal-admission state machine
(`2026-07-20-principal-admission-state-machine-design.md`) and the admin-authz enclosure
(`2026-07-22-admin-authz-enclosure-design.md`).

---

## 0. Revision 2 — what changed and why

Revision 1 was reviewed by two independent adversarial passes (security, design-coherence), both
instructed to refute. Both broke it. The **diagnosis in §1 survived both and was independently
re-verified**; most of the prescription did not. Recorded here rather than silently rewritten,
because several of the corrections are more instructive than the original.

| R1 claim | Verdict | Now |
|---|---|---|
| `FROM kb_profile_auth_links l LEFT JOIN … FOR UPDATE OF v` | **Invalid SQL.** `ERROR: FOR UPDATE cannot be applied to the nullable side of an outer join` | §2.3 — CTE-locked shape, **executed**, with lock behaviour proven by a two-session test |
| The join needs only the two uniqueness constraints | **Wrong.** Nothing tied `v.profile_id` to `l.profile_id`; the old query held that correlation *structurally* | §2.3 — explicit correlation, fail-closed |
| `LinkRefusal::Standing(Refusal)` | **Does not serialize.** Two internally-tagged enums nest to duplicate `kind` keys; ts-rs emits an uninhabitable `never` | §2.1 — distinct tag + struct variant, per `SystemAccessDetails` |
| Site it in `temper-principal`, "Linear lands beside it" | **Rationalization.** Phase 2's Linear is Eve-brokered outbound — no auth-link row, no vault, no triple. The emitters spec forecloses new authz vocabulary | §3 — temper-services |
| "Structurally identical to `admit`" + copy its arity pin | **Trips D2.** Three provisional facts ANDed is D2's forbidden shape; copying `admit`'s anti-conjunction pin onto it *disables the alarm* | §3.1 — D2 addressed head-on; no arity pin |
| `SlackLinkOutcome::NoRefreshToken` | **No producer**, and it mis-framed a paid-for decision as a defect | Dropped (§0.1) |
| §5's premise is stale | **Half true.** The HMAC verify runs in temper-api, so a `pub` no-op constructor is still required unless the *verification* moves | §5.1 |
| The drift gate closes the gap | **Covers the wrong types.** temper-api has no `ts-rs`, so the `status` tags stay ungated | §6 — general gate; tags filed separately |

An interim revision proposed `LinkAuthority: ScopedAuthority`. **Rejected on grounding:**
`resolve(pool, caller: ProfileId, subject)` presumes a caller axis this domain does not have (the
mint is a secret-holding agent acting for a Slack principal, with no authenticated caller profile);
`fn denial() -> ApiError` is static, so a denial arm cannot carry its cause; and `authorize` renders
denial as `Err(ApiError)` where this surface deliberately returns **200 with a typed payload**
(`slack_mint.rs:31-33`).

### 0.1 One R1 finding was itself a misreading — retracted

R1 §1.3 called it a defect that the service returns `Linked` while the handler overrules. It is a
documented decision (`slack_link.rs:279-311`):

> *"THIS ARM MUST NOT RENDER SUCCESS. It used to `warn!` and return `Ok(slug)`… `tx` is DROPPED
> without commit, so the directory row is rolled back too. That is the deliberate call: half-linked
> is the worst of the three states… Rolling back leaves the user cleanly UNLINKED, which is the one
> state the flow can recover from on its own."*

**Consequence, and it corrects §2.2's rationale:** because that arm rolls back, *"linked but not
vaulted"* is **no longer producible by the callback**. It survives only for rows written before T3.
`slack_mint.rs:52-56`'s doc comment describes the pre-rollback world and is stale; R1 cited it as
evidence.

---

## 1. The finding

*(Unchanged from R1 — independently re-verified by both reviewers.)*

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
the two production sites write `NULL` (`:179`, `:188`). Commit `3a45b1ab`: *"drop the never-wired
vault revoke flag in favour of disconnect's delete"*. Disconnect `DELETE`s the row
(`slack_disconnect_service.rs:161`), yielding `NotVaulted`. Independently confirmed by both
reviewers: no trigger on the table (`pg_trigger` → 0 rows), no SQL function, and
`slack_disconnect_service.rs:298`'s `SET revoked_at = now()` is on **`kb_oauth_refresh_tokens`**, a
different table.

**The right disjunct is six facts.** `temper_principal::admit` returns each as a reason-carrying
`Refusal` (`admission.rs:37-59`): `Denied`, `Requested`, `Revoked`, `Deactivated`, `NoStanding`,
`UnrecognizedStanding { raw }`. The gate consults none of them.

### 1.1 The consequence is a false remedy, shipped to a human

`slack_mint.rs:49-51` instructs: *"The user must re-link; retrying will never succeed."* Re-linking
calls `store_grant`, which touches the vault and sets `revoked_at = NULL` (`:188`). **It never
touches `kb_principal_standing`.** For every reachable cause, re-linking cannot change the outcome.
The mention agent renders exactly that on the unmerged T4 branch (`identity.ts:151-158`).

### 1.2 And on a fresh instance it is the modal first-link outcome

`transition.rs:35-38` births every `OauthFirstLogin` human `Denied`. The callback's only identity
gate is Level 1, which passes `Denied` **deliberately** (`auth/mod.rs:249-251`). Level 2 is never
applied to it — `slack_link_public_routes()` is merged with no layer (`routes.rs:426`), by design
(`:326-330`).

So a `Denied` human completes the browser link, gets a grant vaulted, and is shown **"Account
connected."** — after which every mention fails permanently with a remedy that cannot work. The e2e
suite documents the workaround rather than the bug (`slack_link_test.rs:866-868`), and **no test
drives the un-approved path.**

### 1.3 The link half's typed outcome is erased at the boundary

`AlreadyLinkedToAnotherProfile` is compared with `==` at `slack_link.rs:252` and converted to a
`String`. The callback returns `Response`, not `Result` (`:164-169`), so all 13 refusal sites and
success alike are HTTP 200 `text/html` — which is why the e2e tests discriminate by grepping for
`Account <em>connected</em>.`

### 1.4 What the model is today

Six Rust types and two hand-written TypeScript mirrors, none referencing each other:
`SlackLinkOutcome` (`slack_link_service.rs:159`), `SlackLinkStateResponse` (`slack_link.rs:60`),
`MintOutcome` (`slack_grant_vault_service.rs:60`), `SlackMintResponse` (`slack_mint.rs:38-57`),
plus `LinkState` (`mention/agent/lib/link.ts:35-37`) and `MintOutcome`
(`mention/agent/lib/mint.ts:31-45`, **#498's branch only**).

The **cartesian product of rows 2 and 4 is computed in TypeScript** — it is the four-row table in
PR #498's description.

---

## 2. The model

### 2.1 Two arms, and a refusal that carries its cause

```rust
pub enum MintOutcome {
    Token { access_token: String, expires_at: DateTime<Utc> },
    Refused(LinkRefusal),
}

#[serde(rename_all = "snake_case", tag = "reason")]   // NOTE: "reason", not "kind"
pub enum LinkRefusal {
    NotLinked,
    NotVaulted,
    Standing { refusal: Refusal },
}
```

**The tag name and the struct variant are both load-bearing, and this is the R1 blocker.** `Refusal`
is `#[serde(tag = "kind")]` (`refusal.rs:22`). Nesting a second internally-tagged enum under the
same key emits duplicate keys — measured:

```
Standing(Denied)  =>  {"kind":"standing","kind":"denied"}
round-trip        =>  Err(Error("duplicate field `kind`"))
```

and ts-rs emits `{ kind: "standing" } & Refusal`, which narrows `kind` to `never` — an uninhabitable
arm. A distinct tag (`reason`) plus a **named field** (`Standing { refusal }`) nests properly:
`{"reason":"standing","refusal":{"kind":"denied"}}`.

**CONFORM** — this is the incumbent carrier. `SystemAccessDetails` (`access_gate.rs:145`) carries
`Refusal` as a named struct field, never as a variant payload of a second tagged enum.

**`Refusal` is embedded whole, not narrowed.** Only 5 of its 9 variants are reachable here —
`IllegalTransition`, `InsufficientAuthority` and `NoPriorStanding` belong to the *transition*
machine and `admit` cannot return them (`admission.rs:37-58`). A parallel narrowed enum would be a
second copy to keep in sync; instead the reachable set is guaranteed by construction (`admit` is the
only producer) and **pinned by a test** asserting no other variant ever appears.

### 2.2 The decision

```rust
pub struct LinkEvidence<'a> { pub linked: bool, pub vaulted: bool, pub standing: Option<&'a str> }
pub struct Mintable { /* private */ }   // sealed; gates the decrypt, never leaves the service
pub fn resolve(ev: LinkEvidence<'_>) -> Result<Mintable, LinkRefusal>;
```

`Mintable` is internal — it gates the decrypt/refresh inside `mint_access_token` and is never
returned to a surface. **This is R1's §4.2 defect fixed:** R1 had `mint_for_mention` returning
`Result<ActiveLink, LinkRefusal>` where `ActiveLink` was a sealed single-field `{ standing }`, i.e.
nowhere for the access token to go on the endpoint whose purpose is returning one — whose tempting
repair was to put a live credential inside a type the plan told the implementer to derive `Debug` on.

**Ordering.** `NotLinked` first — by data availability, not preference: standing is a property of the
temper *profile*, so it is unknowable before the link exists. Then `Standing`, then `NotVaulted` —
so a `Denied` human is told the remedy that works rather than the one that cannot.

> **Scope note (per §0.1):** `NotVaulted` is now reachable only for pre-T3 rows, since the callback
> rolls back a link it cannot vault. It stays a named arm because those rows exist and the agent
> must say something true about them — not because the callback still produces the state.

### 2.3 The query — executed, not asserted

R1's re-rooted query is invalid. Postgres, live:

```
ERROR:  FOR UPDATE cannot be applied to the nullable side of an outer join
```

`FOR NO KEY UPDATE OF v` and bare `FOR UPDATE` fail identically. The lock cannot be expressed in a
statement where the vault is the nullable side.

**The shape that works** — the lock lives in a CTE, so the vault is a driving relation there while
the outer query still LEFT JOINs it:

```sql
WITH locked AS (
  SELECT * FROM kb_slack_grant_vault WHERE slack_principal_id = $1 FOR UPDATE
)
SELECT l.profile_id,
       v.rt_nonce, v.rt_ciphertext, v.at_nonce, v.at_ciphertext,
       v.access_expires_at,
       (v.id IS NOT NULL) AS vaulted,
       s.state AS standing
  FROM kb_profile_auth_links l
  LEFT JOIN locked v                ON v.profile_id = l.profile_id
  LEFT JOIN kb_principal_standing s ON s.profile_id = l.profile_id
 WHERE l.auth_provider = 'slack' AND l.auth_provider_user_id = $1
```

**Executed evidence, not inference.** It runs; it takes the same three locks the incumbent does
(`kb_slack_grant_vault` + both its indexes, `RowShareLock`); and a two-session test proves the **row**
lock is genuinely held — with the CTE open in one session, both a mint-style and a
disconnect-style `FOR UPDATE NOWAIT` in another fail with:

```
ERROR:  could not obtain lock on row in relation "kb_slack_grant_vault"
```

So both properties R1's shape would have destroyed are preserved: mint↔mint RT-rotation
serialization (`slack_grant_vault_service.rs:209-214`) and mint↔disconnect mutual exclusion
(`:341-342`).

**`ON v.profile_id = l.profile_id` is the correlation, and it is deliberate.** The incumbent roots
at the vault and reads standing through `v.profile_id`, so the standing checked is *definitionally*
the grant owner's. Re-rooting loses that unless it is stated: there is **no** constraint tying the
two columns (no composite FK, no shared unique key, no trigger). Joining on `profile_id` means a
skewed pair reads as *no vault row* — fail-closed to `NotVaulted` rather than minting profile A's
refresh token under profile B's standing. This is the repo's *"absence denies"* posture (admission
spec §7 obligation 1) applied to a join. Not reachable today (rebind is refused atomically,
`slack_link_service.rs:212-217`; disconnect deletes both in one transaction), but the open
account-merge task `019f4473` exists to repoint `profile_id`s.

**sqlx nullability requires explicit annotation.** The committed cache proves sqlx infers columns
from the nullable side of a LEFT JOIN as `NOT NULL` — the incumbent gets `Option<String>` only from
its hand-written `"standing?"` override. So `v.*` columns need `?` (e.g. `v.rt_nonce AS "rt_nonce?"`)
and `vaulted` needs `!`. Without them, `rt_nonce` generates as `Vec<u8>` and decode-errors to a 500
on exactly the `NotVaulted` / `NotLinked` cells this design exists to reach — passing every static
gate on the way.

**CONFORM:** the decision still runs before the cache branch (`:264-267`, *"Not-mintable checks
first, before any cached token is decrypted or the RT is spent"*).

### 2.4 `revoked_at`: drop the disjunct, keep the column

The term is removed; `LinkRefusal` has no arm for it. The **column stays** — dropping it is a
destructive migration for a column that costs nothing sitting `NULL`. Record in code that
soft-revoke was superseded by disconnect's `DELETE` (commit `3a45b1ab`).

---

## 3. Where it lives

**`crates/temper-services/`, beside the Slack services it serves. A free function.**

Not `temper-principal`: R1's justification (*"Linear lands beside it"*) does not survive. Phase 2's
Linear credential is **Eve-brokered outbound** — no `auth_provider='linear'` row, no per-user vault,
so no `linked × vaulted × standing` triple. And the emitters design forecloses the premise
(`2026-07-13-external-systems-as-subscribed-emitters-design.md:437-460`):

> *"**No new authz vocabulary** — and, as of PR #418, no new authz *predicate* either. A connection
> is a machine principal wearing an integration's clothes."*

Not `ScopedAuthority` — see §0 for the three grounded mismatches.

### 3.1 D2, addressed rather than skirted

`resolve` returns `Ok` only on `linked ∧ vaulted ∧ approved` — three provisional facts written by
three different paths. D2 (`2026-07-20-principal-admission-state-machine-design.md:349-356`) says:

> *"A conjunction across provisional conditions, written by different paths, is the bug shape
> itself."*

**D2 governs *admission* — whether a principal is admitted at all — and this is not that.** Admission
is answered once, by `admit`, from standing alone, and this design *calls* it rather than restating
it. What `resolve` adds is a *capability* question: given an admitted human, is there a credential to
present as them? A mint genuinely requires all three facts; requiring fewer would mint without a
grant.

**Therefore: no arity pin on `resolve`.** R1's plan told the implementer to copy
`admit_reads_standing_and_nothing_else` (`admission.rs:102-109`), whose comment reads *"Do not 'fix'
it by updating the call; re-read D2."* Copying that pin onto a deliberate three-fact conjunction does
not honour the obligation — it **disables the alarm**, by making a future reader think the obligation
is respected here. `admit`'s pin stays where it is and keeps meaning what it says.

### 3.2 Instance two of a one-instance pattern — deliberately not extracted

Two live patterns answer "what may this principal do":

| Pattern | Shape | Instances |
|---|---|---|
| Return flat facts, client decides | `Entitlements { system_access, is_admin, join_request_status }` (`access_gate.rs:110-114`) | the access surface |
| Decide server-side, return proof-or-typed-reason | `admit(…) -> Result<AdmittedPrincipal, Refusal>` | **one** — `standing_service.rs:58` is the only service fn returning `Result<_, non-ApiError>` |

The first is *what produced this bug*: the mention agent was handed facts, computed the product, and
got the remedy wrong. So this design takes the second, making it instance two.

**No shared machinery is extracted, on the repo's own precedent.** The ScopedAuthority spec named its
pattern only after *"three domains had independently grown the same shape"*. Extracting at two is
what the rejected interim revision attempted, and it failed because the trait was shaped by three
domains sharing a caller axis this one lacks.

Discoverability is bought with **explicit kinship instead**: `resolve`'s doc names `admit` as the
shape it follows and states the semantic difference (§3.1); `admit` gains a "see also". A grep for
`Refusal` then lands on `admit`, `SystemAccessDetails` and this together.

---

## 4. The call sites

### 4.1 Both internal routes keep their separate secrets

Collapsing them was rejected — `mint.ts:8-13`: *"`SLACK_LINK_SECRET` gates an endpoint that answers
a question… `SLACK_MINT_SECRET` gates one that hands back a token carrying that human's ENTIRE
temper reach. Sharing a key would make compromise of the cheap capability yield the expensive one."*

**One vocabulary, one renderer.** `mint` carries the full [`LinkRefusal`] vocabulary
(`not_linked` / `not_vaulted` / `standing`); `link-state` stays two-arm — `linked` / `unlinked` —
answering only its own prior question: *does this human need to link at all?*

> **AMENDED at implementation (2026-07-23), narrowing the original §4.1.** Revision 2 had `link-state`
> also *resolve* and report *why-refused*, and recorded two widenings for it: disclosing a linked
> human's standing to a `SLACK_LINK_SECRET` holder, and — the security review's F10 — giving the
> cheap endpoint a **read of `kb_slack_grant_vault`**, the credential store. Building it revealed the
> widening buys nothing: the mention agent already calls `mint` after `link-state`, and `mint` now
> delivers every refusal reason (§4.2, `MintOutcome::Refused(LinkRefusal)`), so the four-row product
> PR #498 computed in TypeScript collapses through **`mint` alone**. `link-state` resolving would be
> a redundant second answer whose only *new* effect is the F10 attack surface. So it stays two-arm,
> reads no vault, and discloses no standing. **This is a strict de-scope of an approved section, made
> because the concrete code showed the widening was cost without benefit — recorded here in place
> rather than silently dropped** ([[feedback_amend_working_docs_in_place_not_via_pr]]).

**Consequence for `link-state` code: none.** The current handler (`slack_link.rs:81-124`) already
returns `Linked{handle}` for any linked principal and `Unlinked{authorize_url}` otherwise, which is
exactly this design. A linked-but-unmintable human reads as `linked` at `link-state` and gets the
specific refusal from `mint` — the shape the e2e test
`mint_reports_not_vaulted_distinctly_from_not_linked` already pins.

This still bears on open task `019f7cd1-a3fb-79c1-aa3f-befd4b843b17` (*"linked ≠ mintable"*): the two
states are now genuinely distinguishable **at `mint`**, which is where the agent acts. Its item 3
(*"should `link-state` report it at all"*) is answered **no**, with the reason above; its second
criterion (operator observability) is still unaddressed — see §9. The task is advanced, not closed
done-by.

### 4.2 `slack_mint`

Renders `MintOutcome`'s two arms. The `From<MintOutcome> for SlackMintResponse` impl
(`slack_mint.rs:71-85`) stays total and information-preserving. **Delete** the false remedy at
`:49-51`. The hand-written redacting `Debug` (`:59-69`) must survive on the token arm.

### 4.3 The callback's third page

Where the §1.2 trap dies. **Not by refusing the link** — refusing means re-linking after approval,
for no gain:

| Outcome | Page |
|---|---|
| linked, vaulted, `Mintable` | "Account connected." (as today) |
| linked, vaulted, standing refuses | connected — **and you cannot act until an admin approves**, carrying `Refusal::reason()` |
| `AlreadyLinkedToAnotherProfile` / no refresh token | "Not connected." (as today, `slack_link.rs:279-311`) |

**CONFORM:** the one-transaction invariant is untouched, pinned by
`link_is_rolled_back_with_its_caller_transaction` (`slack_link_service.rs:474`). **No new
`SlackLinkOutcome` arm** — see §0.1.

**The new renderer must `html_escape`.** Both incumbents do (`:501`, `:509`) and there is a test for
the failure page (`:605-610`). `Refusal::UnrecognizedStanding` formats `raw` with `{:?}`
(`refusal.rs:68-70`), which escapes quotes but **not** `<`/`>`. It is currently safe only because a
CHECK constraint on *another table* bounds `raw` to five literals — a defence that should not live
there.

**Accepted cost, stated:** `store_grant` seals a live refresh token plus a cached access token
(TTL ≤ `MAX_AT_TTL_SECS`, 24h) for a principal that may never be admitted. No new access is
conferred — the mint still refuses — but this is the one place §4.3 expands credential-at-rest.

---

## 5. The sealed `VerifiedSlackPrincipal`

`slack_mint_service.rs:32-37` rejected this on the premise that the constructor must be `pub` for a
different crate to call it.

### 5.1 The premise holds unless the verification moves — R1 got this half wrong

The incumbent seal is real because **the check lives inside the sealing crate**: `auth.rs:82` calls
`temper_services::auth::authenticate_token`, and temper-api never mints an `AuthenticatedProfile` —
it hands over a token and receives a proof.

The Slack mint gate is not like that. The HMAC verify runs in **temper-api**
(`internal_auth.rs:24,39-92`). For its middleware to "mint the proof inside temper-services",
temper-services must expose a `pub` constructor that checks nothing — R1's objection, unchanged.

**So the verification moves.** temper-services exposes one function that both verifies and mints:

```rust
pub fn verify_mint_request(
    secret: &str, timestamp: i64, body: &[u8], signature: &str,
) -> Result<VerifiedSlackPrincipal, ApiError>;
```

Passing the signature is the *only* way to obtain the proof — `authenticate_token`'s shape exactly.
Without this, the trybuild fixture proves only that struct-literal forgery fails while
`VerifiedSlackPrincipal::new(attacker_string)` sits `pub` beside it.

**Note:** `audit-signature-secrets.sh` computes gate/secret pairing **from source**; moving the
verify moves what that guard reads, and the guard must move with it.

### 5.2 Cross-gate containment is structural, not a comment

`require_signature_with` (`internal_auth.rs:39-92`) is shared by all three gates, and the verified
bytes exist only as a local inside it. Parsing-and-inserting there would mint the proof for
`INTERNAL_RECONCILE_SECRET` and `SLACK_LINK_SECRET` holders too — the exact cross-gate leak the
separate keys exist to prevent (`:36-38`, `:153-156`).

**The helper takes an explicit per-gate hook** (a closure or enum parameter) so the mint proof is
minted by the mint gate *by construction*. A negative test — a proof obtained behind the link gate —
is required, not optional.

`validate_slack_principal` (`slack_link.rs:133-150`) moves onto this path and **must keep returning
`BadRequest`**: `mint_rejects_a_malformed_principal` (`slack_link_test.rs:1750-1761`) asserts 400,
while every other refusal in this middleware is `Unauthorized`.

### 5.3 What this claims, precisely

It does **not** make the string more trustworthy — `slack_mint_service.rs:22-24` remains correct
that *"provenance is extrinsic."* It makes **calling the mint with a principal that did not come
through the gate** unrepresentable: the enclosure's class of bug, not a wire-level upgrade.
Possession of `SLACK_MINT_SECRET` remains the wire-level enforcement.

**CONFORM:** not `Authorized<A>` — `pub(crate)` to temper-services (`authz/mod.rs:54,99`).

---

## 6. The drift gate — general, not Slack-shaped

R1 proposed a Slack-named gate over `temper-principal`'s `admission.ts`. That **covers the wrong
types**: the discriminants the agent switches on are `SlackLinkStateResponse` (`slack_link.rs:60`)
and `SlackMintResponse` (`slack_mint.rs:38-57`), both in **temper-api — which has no `ts-rs`
dependency at all**. Renaming `SlackMintResponse::NotVaulted` would still pass it.

**`check-ts-rs-drift.sh`**, over everything `generate-ts-types` writes. Same cargo build; closes the
repo-wide gap CLAUDE.md records (*"NO CI gate on ts-rs type drift"*) instead of one instance; and
covers both output trees — a narrow gate would catch drift in the mention tree and miss it in
temper-ui's, from the same generator run.

Modelled on `check-temper-ts-drift.sh`, including its two hard-won properties: regenerate then
`git diff --exit-code` (against **git**, not a fresh build), and **assert the artifact is tracked
before diffing** (`:26-34`) — *"A gate that cannot fail is not a gate."*

**Generation.** `TS_RS_EXPORT_DIR` is per-invocation (`main.toml:276-281`), so `generate-ts-types`
gains a second `-p temper-principal` line targeting the mention package's own tree — it is
*"deliberately NOT a bun `workspaces` member"* and cannot import from temper-ui's. `package.json`
declares `"imports": { "#*": "./agent/*" }`, so the file is importable as `#generated/admission.js`.

**Wiring.** `[tasks.check]` (`main.toml:27-37`), and the **`rust-quality`** CI job — *not*
`guard-tests`, whose header says *"Pure bash, no toolchain, no services"* (`code-quality.yml:119`).
The gate needs cargo. Its **harness** (`test-check-ts-rs-drift.sh`) goes in `guard-tests`, matching
the existing split: guards run in `rust-quality`, harnesses in `guard-tests`.

**Not closed, filed instead:** the temper-api `status` tags have no generator and remain ungated. A
separate task carries the evidence and the options (relocate the wire enums to a `ts-rs` crate, or
gate them another way). Saying so beats implying coverage that does not exist.

---

## 7. Testing

| Test | Mirrors |
|---|---|
| Full-cell matrix: `linked` × `vaulted` × (5 standings + absent + unrecognized); every cell decided; every refusal a non-empty `reason()` | `temper-principal/tests/matrix.rs` |
| **Serialization round-trip over every `LinkRefusal` arm** — the test whose absence would have shipped R1's duplicate-key blocker | — |
| A pin that only `admit`-reachable `Refusal` variants ever appear (§2.1) | — |
| trybuild: forging `VerifiedSlackPrincipal` | `tests/compile_fail/forge_system_admin.rs` |
| A proof obtained behind the **link** gate must fail (§5.2) | — |
| **The test that does not exist today:** un-approved principal, end to end | §1.2 |

**Expected churn** — five sites, not four. `slack_grant_vault_service.rs:679`, `:707`, `:735`,
`:762`, **and `tests/e2e/tests/slack_link_test.rs:1727`** (`mint_reports_a_revoked_grant_as_revoked`),
whose entire premise is the dropped disjunct — R1's plan missed it, which invited an implementer to
delete an unlisted red. Decide it deliberately: the column survives (§2.4), so either the test
survives against a directly-flipped flag, or it goes with a recorded reason.

**One executable step is mandatory before any Rust is written around §2.3's SQL: run it against the
dev database.** R1's query passed `cargo check`, `cargo sqlx prepare`, the cold-offline check and
`cargo make check` — every gate in the plan — while being illegal to execute.

---

## 8. Blast radius

| Type | Exposure | Regenerates |
|---|---|---|
| `SlackLinkOutcome` | purely internal — one consumer (`slack_link.rs:252`) | nothing |
| `SlackLinkStateResponse` | wire, allowlisted out of `openapi.json` | the TS mirror (§6) |
| `MintOutcome` + `SlackMintResponse` | wire, allowlisted out | the TS mirror (§6) |

No `openapi.json`, no gem, no `schema.ts`, no substrate snapshot, no migration. **`.sqlx` does churn**
— §2.3 changes query text, so `cargo sqlx prepare --workspace -- --all-features` then
`cargo make prepare-services`, in that order. Verify with a **cold** offline check
(`cargo clean -p temper-services` first), noting that this proves cache honesty and **not** SQL
validity (§7).

`audit-credential-debug.sh` baselines `MintOutcome` / `NewGrant` / `SlackMintResponse` by name
(`:73-79`) and its self-test fixtures them (`test-audit-credential-debug.sh:97,101`); a rename moves
both in the same commit.

---

## 9. Out of scope — named, not dropped

- **The disconnect half and `IdpRevocation`** — six committed artifacts, a shipped immutable
  migration (`20260719000020`), and a live `kb_event_types` row.
- **Dropping the `revoked_at` column** (§2.4).
- **The intent lifecycle** — the nonce burn outside the transaction (`consumed` conflates succeeded,
  refused and rolled back); unbounded `create_intent`.
- **Operator observability of the unvaulted-link state** — the second acceptance criterion of task
  `019f7cd1-a3fb-79c1-aa3f-befd4b843b17`. §4.1 closes its first; this one needs an operator surface
  and is a different piece of work. Named here because R1 dropped it silently.
- **The temper-api `status`-tag generator gap** (§6), filed separately.
- **`_event_append` performs no `payload_schema` validation.**
- **MCP parity.**

---

## 10. Sequencing — #498 lands first

R1 said #498 *"should rebase onto this"*. That is circular for the gate: `mint.ts` exists **only** on
#498's branch, so a gate built on `main` would cover one consumer, and R1's own §6 cited
`mint.ts:31-45` as a consumer that is not there.

- **PR 1–2** (the Rust work, §2–§5) land first. #498 rebases onto them — it must, since its
  `revokedPrompt` (`identity.ts:151`) is the false remedy §1.1 identifies, and merging it first
  ships that to the only surface a human reads.
- **#498** lands next, bringing `mint.ts`.
- **PR 3** (§6, generation + gate) lands last, when both consumers exist.

Against the repo's *"split PRs on coherence when deployed, not testability"*: the gate is testable
alone and **not shippable alone**, which is the case that convention names.

---

## 11. Adjacent, filed separately

`admin_disconnect_slack_principal` (`slack_disconnect_service.rs:267`) is a Bucket-1
system-authority act still gated by a bare bool — task `019f8ec3-793f-7c52-9378-47dda5d90a5d`. It
takes `&SystemAdmin` and reads `admin.actor()`. Independent of this design.
