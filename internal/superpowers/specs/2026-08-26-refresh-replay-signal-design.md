# Refresh-token replay signal — design

**Task:** [A rotated refresh token that comes back is a theft signal nobody listens for](https://temperkb.io) — `01a0390d-335a-7440-a27a-565e0a70ccce`
**Migration:** `20260826000140_refresh_replay_signal.sql`
**PR:** [#787](https://github.com/tasker-systems/temper/pull/787)
**Date:** 2026-08-26

This carries the reasoning behind the migration and the AS changes. The migration itself is
immutable once applied, so it holds only what a reader of the DDL cannot work out; everything that
is argument rather than instruction lives here, where it can be corrected.

## The property

RFC 6819 §5.2.2.3 and the OAuth 2.0 Security BCP read a rotated refresh token that is presented
again as the canonical indication that a chain has been **copied**. Acting on that reading requires
four facts the authorization server can state:

1. that a given row was retired **by rotation** specifically;
2. which **chain** it belonged to;
3. whether that chain has been **ended**;
4. that the presentation **happened**, durably, where an operator can read it.

One mechanism each: `rotated_at`, `chain_id`, `kb_oauth_refresh_chain_ends`,
`kb_oauth_refresh_replays` + `vw_oauth_refresh_replays`.

## D1 — `rotated_at`, not `rotated_to`

`revoked_at` is one column with **five** writers, and only one is rotation:

| Writer | Meaning | `rotated_at` |
|---|---|---|
| `rotateRefreshToken` (`flow.ts`) | spent, successor minted | **set** |
| `revokeRefreshToken` (`flow.ts`) | administrator | NULL |
| `standing_service::apply` terminal hook | administrator | NULL |
| `slack_disconnect_service::revoke_as_refresh_token` | administrator | NULL |
| `endRefreshChain` (`flow.ts`, this feature) | chain judged copied | NULL |

Each of the four is held to leaving `rotated_at` NULL by a test in the module that owns it,
including both Rust crates — the AS's own suite cannot see those, and a stray `rotated_at` in any of
them would turn an ordinary administrative action into a permanent, unfalsifiable theft report.

**Rejected: the `rotated_to` descent link** that `20260701000006:75` declares. A descent link can
only be stamped once the successor row exists, so it is necessarily a statement issued *after* the
successor INSERT — a third statement, outside the guard, where a process that dies in between leaves
a rotated row unable to say what retired it. `rotated_at` is part of the guard statement that
already runs. The column is kept rather than dropped: the chain **topology** it would record is a
capability `chain_id` does not provide.

## D2 — `chain_id`, and why existing rows are named by an explicit UPDATE

Chain identity mirrors `chain_expires_at` (`20260825000010`) exactly: stamped at the chain's first
token, inherited unchanged by every successor, threaded through the same three call sites. It is one
more field on a value already being passed, not a new mechanism.

**NOT NULL, defaulting to `uuid_generate_v7()`.** The default buys the rolling-deploy window: an AS
binary that predates the column omits it, the row still gets a name, and the paired binary inherits
that name on the next rotation — so a chain spanning a mixed window stays reachable end to end. A
nullable column would leave that row unnamed and its successor rooting a fresh chain, and would
force every reader to carry an unnameable-chain branch.

**Existing rows are named by `UPDATE ... SET chain_id = id`, not by the default, and that is the
non-obvious part.** Adding the column *with* the default would name them too — but only because the
default is VOLATILE, which is what makes PostgreSQL rewrite the table and evaluate per row instead
of taking the fast path and storing one constant for every existing row. That property is not ours
to rely on: `20260624000001:48-70` resolves the one portable name to two implementations —
`pg_uuidv7`'s on PG17 (Neon) and a shim over native `uuidv7()` on PG18 (local, CI). The deciding
behaviour therefore belongs to an implementation that cannot be checked from a PG18 dev box, and
`pg_uuidv7` is not installable there. Had it differed, every historical token would have landed in
one chain and a single replay would have ended all of them.

The explicit UPDATE costs the same rewrite, assumes nothing, and says something truer: every row
already on disk is the root of its own chain, which is exactly what is known about it. Verified
against a populated table — 5,000 rows, 5,000 distinct chain ids, each equal to its own `id`.

*General form: when a property cannot be checked in the environment you have, stop depending on it
rather than measuring harder.*

## D3 — `kb_oauth_refresh_chain_ends`: recording the ending, not its effect

Rotation is two statements with no transaction across them — the AS speaks to Postgres over an HTTP
driver. The guard revokes the predecessor; the successor is inserted afterwards.

A chain-ending that lands **in that gap** finds every row of the chain momentarily dead, so revoking
rows takes nothing — and the successor then arrives behind it and the chain is alive again, with the
responder having reported that it ended it. The gap is not exotic: it is exactly what a client
racing two refreshes produces, which is the case `AS_REFRESH_REPLAY_GRACE_SECONDS=0` — the BCP's
strictest reading — is aimed at.

So the ending is **recorded** rather than only applied, and `storeRefreshToken` refuses to mint a
successor into a chain carrying a marker. The revoked-row count returned by `endRefreshChain` is
then an honest report of what that statement took, never the thing the ending depends on.

Scoped to this responder. `standing_service::apply` and `slack_disconnect_service` end chains
through their own statements and keep the one-token-pair excursion `20260825000010` states for them;
widening the marker to cover them would change a documented behaviour in another crate and is not
this change's business.

## D4 — the replay record

**One row per token, upserted, not one per presentation.** The write is reachable by anyone holding
a retired token, so an append-shaped table would let a loop grow it without bound. The primary key
caps it at the token table's own size and the counters carry what an append would. BIGINT counters
for the same reason: an INTEGER that wrapped would make the upsert throw and the record silently
stop advancing.

**Three counters, not one.** `replay_count - graced_count` is the count of presentations this
instance judged hostile; `tokens_revoked = 0` beside a non-zero one of those means the chain held
nothing live when the judgement was made. Collapsing them loses the distinction an operator reads
for.

**`first_age_seconds` is carried from the AS, not re-derived.** Deriving the age from
`first_seen - rotated_at` would read a *second* clock, a statement later, so a presentation graced at
9.8s under a 10s window could be stored reading 10.2s — the detector disagreeing with its own
record. Both operands of the judged value come from one `now()`.

**Why a view rather than log retention.** Logs are the surface a self-hosting operator is least
likely to have kept, and the AS side is the one that does not ship to Tempo — `20260825000010`'s
`chains_ended` warning is emitted from Rust for exactly that reason.

## D5 — the grace window, and where the policy lives

`AS_REFRESH_REPLAY_GRACE_SECONDS` (default **10**) is the line between a client and a thief, and it
lives in the AS because the trade is an operator's to set.

A rotated token that comes back is also what an ordinary client produces in two benign ways: it lost
the response carrying the successor and retried, or it had two requests in flight and one lost the
race.

**The two are not equally costly to get wrong, and only one is what the window is for.** A client
that lost the response holds no live token and must re-authenticate whatever we do — ending its
chain costs it nothing, and mislabelling its retry costs an operator one row to read past. The
concurrent-refresh client actually holds the successor, and its two requests are one exchange still
resolving: well under a second.

**The width has a cost running the other way.** The AS only ever sees the party that *lost* a
rotation race and cannot tell whether the winner was the user or someone holding a copy. Inside the
window it lets the winner keep the session and files the presentation as benign — so every second of
width is a second in which a fresh theft is answered gently and recorded as a retry. Ten covers the
case the window exists for and leaves six times less of that than sixty would.

Consequently the playbook states `first_replay_age` in seconds as *consistent with* a client retry
rather than proof of one.

`0` is legal and means the strictest reading. The parser **substitutes and warns** where
`refreshChainMaxSeconds` refuses, because its caller catches everything so a failed audit write
cannot change the client's answer — a throw would therefore surface as nothing at all, leaving the
detector configured and inert. `MAX_REPLAY_GRACE_SECONDS` (3600) is a units check, not a policy.

## D6 — a rotation refused by the admission gate withdraws its own mark

The admission gate runs *after* the rotation guard: a terminal principal's refresh really does
rotate, and `storeRefreshToken`'s predicate declines only afterwards. Left marked, that row makes
the user's next retry a replay — putting a de-provisioned person in the operator's view under a
hostile count, which is the same false theft report the administrative revokers are held away from,
reached from the side no revoker writes.

`rotated_at` means *retired by a rotation that produced a successor*. When none was produced the
chain was ended, not rotated, and the row should say what happened. Best-effort: the answer is
already settled, and failing to tidy the mark must not turn it into a 500.

## Witnesses, and what shaped them

`packages/temper-cloud/tests/integration/oauth/refresh-replay.test.ts` (12), plus assertions in
`standing_service.rs` and `slack_disconnect_service.rs`.

Two are shaped by what a witness has to be able to **express**:

- **The replayed token is deliberately not a chain root.** A root satisfies `chain_id = id`, so an
  implementation reaching by the presented row's own id is indistinguishable from one reaching by
  its chain. The test pins `id !== chain_id`.
- **The graced case runs at a non-trivial elapsed time.** A presentation milliseconds after the
  rotation is inside the window under seconds *or* milliseconds, so a units slip in the comparison
  would be invisible.

Both came out of an adversarial pass whose brief was to construct wrong implementations that still
pass — six did, and a mutation probe structurally cannot find that class, because it confirms a
witness fails when the code breaks and never that it passes for the reason it names.

## Operator surface

`docs/playbooks/self-host-with-saml.md` — *Watch for replayed refresh tokens*: the query, how to read
each column, what the window trades. Both env tables carry
`AS_REFRESH_REPLAY_GRACE_SECONDS`; it is not emitted by `temper admin saml provision`, because the
sibling's reason for explicit emission (a figure stated at a review board) does not transfer to a
grace window.
