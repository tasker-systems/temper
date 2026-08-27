# The Slack mint gate held — the enablement checklist, and one seal that is misdescribed

**Date:** 2026-08-26
**Status:** Recorded — an enablement checklist, not a gap
**Scope:** the Slack mint path and its preconditions
**Task:** `01a035f2-d37a-7a83-9f6c-b93d58eb5847`

## The gate held

The investigation that produced this row found no defect. What follows is therefore the set of
**conditions under which the gate's soundness continues to hold** — the thing an operator must satisfy
before turning the Slack surface on.

But the "sealed three ways" framing needs one correction first, because the third seal is
misdescribed in a way that matters.

## Seal 1 — the wire secret: confirmed, and it is two secrets at two hops

**Hop A (Slack → agent):** Slack's own request-signing, verified by the eve runtime — **not by Temper
code**. No `X-Slack-Signature` verification exists anywhere in this repo. It fails closed: with no
`SLACK_SIGNING_SECRET` the route returns 401 and the `url_verification` branch is never reached.

**Hop B (agent → Temper):** a Temper-specific shared HMAC secret, `SLACK_MINT_SECRET`, via
`temper_core::internal_sig` — `"{timestamp}.{body}"`, lowercase-hex HMAC-SHA256, headers
`X-Temper-Timestamp` + `X-Temper-Signature`. Replay window is present and symmetric
(`MAX_SKEW_SECS = 30`, pinned by `freshness_window_is_symmetric`), verification is constant-time, and
a cross-runtime known-answer vector pins the TypeScript signer to the Rust verifier.

**Key separation is machine-checked.** `.github/scripts/audit-signature-secrets.sh` returns
`OK — 3 signature gates, all reading distinct secrets`. Its header states the incumbent risk:

> a shared key is not a tidiness problem. Possession of the cheap key would forge the expensive call:
> whoever can ask "is Alice linked?" could instead mint Alice's token.

The route cannot be mounted ungated — `slack_mint_internal_routes` appears three times in
`routes.rs`: one definition and two merge sites, each layered with `require_slack_mint_signature`.

## Seal 2 — the structural proof: real, but it is a containment seal, not anti-forgery

`VerifiedSlackPrincipal` has a private field whose only constructor is `verify_mint_request`, which
performs the HMAC verify inside `temper-services`. The handler takes it by `Extension` and there is no
client-supplied principal field to read.

**But the module doc refuses the strong reading**, and the register row should not restate it:

> it does NOT make the principal string more trustworthy — provenance is still extrinsic. It makes
> **calling the mint with a principal that did not come through the gate** unrepresentable — the
> enclosure's class of bug, not a wire-level upgrade. Possession of `SLACK_MINT_SECRET` remains the
> wire-level enforcement.

So the three seals are **not independent**. Seal 2 is a containment seal layered on seal 1, defending
against temper-api code calling the mint with an unverified principal. Forgery resistance is entirely
the secret.

**And its proof does not run by default.** `crates/temper-services/tests/compile_fail.rs` is
`#![cfg(feature = "trybuild")]` — off in every `--workspace` run, running only in the dedicated
"Seal Proof (trybuild)" job.

## Seal 3 — REFUTED as stated: it is a standing gate, not a capability machine

The register row describes the mint as issuing *"a scoped capability, not general authority — even a
successful mint yields something narrow."*

**That is the opposite of what ships.** `mint_access_token` returns an ordinary IdP access token from
`refresh_grant`, whose form body sends **no `scope` parameter**, so the token inherits the original
grant's scopes fixed at link time. The repo says so twice, in its own words —
`require_slack_mint_signature`:

> This one guards an endpoint that hands back an **act-as-the-human access token**: a bearer that
> resolves to a real profile and carries that human's full reach, personal contexts included.
> `resources_visible_to` takes a profile and nothing else, so there is no narrowing to fall back on —
> whoever holds the minted token is, to temper, that person.

Confirmed at the SQL layer: `resources_visible_to(p_profile uuid)` takes **one argument**. No caller
axis, no scope axis, no provenance axis. A Slack-minted token is indistinguishable downstream from a
CLI token.

**What is actually checked** is `slack_link_state::resolve` — a three-fact conjunction of *linked*,
*admitted* (via `temper_principal::admit` on `kb_principal_standing.state`), and *vaulted*.
**`has_system_access` and `kb_access_grants` are consulted nowhere in the mint path.**

The only narrowing that exists is **temporal**: 24h ceiling, 1h default, re-mint 5 minutes early.

The repo's own vocabulary draws the line correctly — `resolve` *"answers a capability question layered
on top of an admitted human: given that they are admitted, is there a credential to present as
them?"* **It decides whether, never how much.** The register row should say *"a standing gate."*

## The enablement checklist

### Secrets — nothing in Temper can verify these for you

1. **Generate all five shared secrets independently**: `INTERNAL_RECONCILE_SECRET`,
   `EMBED_DISPATCH_SECRET`, `SLACK_LINK_SECRET`, `SLACK_MINT_SECRET`, `SLACK_VAULT_ENC_KEY`.
   *Check:* the API boots — `check_secret_distinctness` refuses boot on any collision. But it checks
   **only exact byte equality**: no length check, no entropy check, no format check.
2. **Verify `SLACK_LINK_SECRET ≠ SLACK_MINT_SECRET` on the agent deployment too.** The boot check runs
   on the Temper API only. If **both** sides are set to one colliding value, every call succeeds and
   nothing warns.
3. **Confirm `SLACK_VAULT_ENC_KEY` never left the Temper API deployment.** *Check:* it is absent from
   the agent's environment. Its own documentation calls it *"the most dangerous member"* of the five —
   it decrypts **every** stored grant, not one. It is not a gate secret, so it falls outside the
   three-seal frame; any checklist that omits it is incomplete.
4. **Treat generation, transport, and storage as your responsibility.** The config doc concedes the
   boundary: *"IT CHECKS THE SOURCE, NEVER THE DEPLOYED VALUES — and it cannot."*

### Transport

5. **`SLACK_SIGNING_SECRET` is set on the agent before the request URL is declared.** Without it eve
   401s every request and Slack's handshake is unreachable.
6. **`TEMPER_API_URL` points at the API origin, not the UI origin** — the UI proxy does not forward
   `/internal`.
7. **Clock skew between agent and API stays inside 30 seconds.** That is the entire replay window.

### The structural seal

8. **The trybuild job is enabled in your CI and green.** The seal is unproven on any pipeline that
   does not run that one job.
9. **`audit-signature-secrets.sh` is wired into CI.** It is the only thing that catches two gates being
   repointed at one config field.

### Reach — accept this or do not enable

10. **A minted token is the human, unnarrowed.** There is nothing to configure. Any Slack user whose
    principal is linked and whose standing is `approved` can reach, through the agent, everything they
    can reach through the CLI. **If that is not acceptable for a given human, the lever is their
    standing, not the Slack surface.**
11. **Decide the standing posture before enabling.** Absence denies, an unrecognized value denies, and
    only `approved` mints. A workspace of linked-but-unapproved users is the safe default state.
12. **Do not enable `message.im`.** It is commented out in the app manifest and `onDirectMessage`
    returns null, both deliberately. The runtime's default handling of that event does not run the
    identity decision, the human-only gate, the link-state check, or the mint pre-flight that the
    mention path runs. **Enabling it is a change to the gate's design, not a configuration toggle**,
    and belongs in a reviewed diff.

### Offboarding — set expectations before relying on it

13. **Disconnect is not a cutoff.** The exposure window is the IdP's access-token TTL. Temper clamps
    its *cache* to 24h and defaults to 1h, but the real bound is the JWT's own `exp`, which Temper
    does not set. **To actually cut someone off, move their standing to `deactivated` or `revoked`** —
    that is enforced per request.
14. **Have an out-of-band IdP revocation path.** Under an external IdP revocation is best-effort; after
    a vault-key rotation it is not attempted at all.
15. **Disconnect does not uninstall the Slack app.** That is workspace-level and admin-only.

### IdP client

16. **Refresh-token rotation ON *and* rotation leeway enabled.** `mint_access_token` refreshes under a
    `FOR UPDATE` row lock while the IdP rotation is a non-atomic external step; without leeway, a kill
    at the wrong instant bricks the grant family and the user must re-link.
17. **The client is public (auth method None) with `offline_access` in scope.** A confidential client
    rejects the secret-less exchange, and without `offline_access` there is no refresh token to vault.

## What is already documented, and what is not

`docs/concepts/slack-identity-and-revocation.md` covers residual 3 comprehensively — *"'Disconnected'
means 'cannot mint again.' It does not mean 'cannot act.'"* — plus the two-hop trust boundary, the
secret split, and credential-at-rest. `docs/playbooks/slack-mentions.md` carries the operator half of
items 1–3, 5–6, and 16–17.

**Not documented:** items 8 and 9 (CI-internal), item 12 (the `message.im` hazard lives only in a code
comment), and most notably **the unnarrowed reach is stated affirmatively rather than as a caveat.**
The concepts doc says the token *"acts as that human under their own reach: their contexts, their
resources, nobody else's"* — true, but it reads as a limit. The code's own framing — *"their FULL
reach, personal contexts included… there is no narrowing to fall back on"* — is the operator-relevant
version, and it appears only in a doc comment and a CI script header.

## One stale claim in the shipped documentation

`docs/concepts/slack-identity-and-revocation.md` states: *"The mint path checks `is_active` before
decrypting anything."* **`is_active` was dropped** by principal-admission Phase 2. The mint checks
`kb_principal_standing.state` via `admit`, and `admit_reads_standing_and_nothing_else` explicitly
forbids `admit` ever taking `is_active`.

The mechanism still holds — deactivation now lives in standing as `state = 'deactivated'` — so the
doc's *conclusion* is correct while its stated *mechanism* is retired vocabulary. A one-line doc fix,
not a gap in the gate.
