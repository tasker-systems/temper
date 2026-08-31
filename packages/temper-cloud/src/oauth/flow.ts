import type { NeonClient } from "../db.js";
import { servedAudiences } from "./env.js";
import type { MintedClaims } from "./mint.js";
import { hashToken } from "./mint.js";
import { verifyPkceS256 } from "./pkce.js";

/**
 * Normalizes a JSONB column read back from the DB. `postgres` (used in
 * integration tests) returns a `::jsonb`-cast column as a *string*; `neon()`
 * (used in production) returns it already parsed as an object. Both drivers
 * run the exact same store functions, so every read path must tolerate
 * either shape.
 */
function normalizeClaims(value: unknown): MintedClaims {
  return (typeof value === "string" ? JSON.parse(value) : value) as MintedClaims;
}

/**
 * Normalizes a TIMESTAMPTZ column read back from the DB into an ISO string. Same driver split as
 * `normalizeClaims`: `postgres` (integration tests) hands back a `Date`, `neon()` (production) a
 * string. Everything downstream of a read passes the value straight back into another query, so
 * one shape at the boundary keeps the two drivers from disagreeing about a bound.
 */
function normalizeTimestamp(value: unknown): string {
  return value instanceof Date ? value.toISOString() : String(value);
}

export interface CreatePendingFlowParams {
  relayState: string;
  clientId: string;
  redirectUri: string;
  codeChallenge: string;
  codeChallengeMethod: string;
  oauthState: string;
  audience: string;
  expiresAt: Date;
}

/** Creates a pending OAuth flow row awaiting the SAML ACS callback to bind a code. */
export async function createPendingFlow(db: NeonClient, p: CreatePendingFlowParams): Promise<void> {
  await db`
    INSERT INTO kb_oauth_flow (
      relay_state, status, client_id, redirect_uri, code_challenge,
      code_challenge_method, oauth_state, audience, expires_at
    ) VALUES (
      ${p.relayState}, 'pending_saml', ${p.clientId}, ${p.redirectUri}, ${p.codeChallenge},
      ${p.codeChallengeMethod}, ${p.oauthState}, ${p.audience}, ${p.expiresAt.toISOString()}
    )
  `;
}

export interface BindCodeToFlowArgs {
  code: string;
  claims: MintedClaims;
  expiresAt: Date;
  /**
   * The profile this login resolved to, for stamping on the refresh chain the token endpoint is
   * about to mint. `null` when the resolve call was unavailable — login proceeds regardless, and
   * the chain is bounded by its absolute lifetime alone until a rotation re-resolves it.
   */
  profileId: string | null;
}

export interface BindCodeToFlowResult {
  redirectUri: string;
  oauthState: string;
}

/**
 * Atomically binds a freshly-minted one-time authorization code to a pending
 * flow (found by `relayState`), moving it from `pending_saml` to
 * `code_issued`. Throws if there is no matching pending flow (unknown
 * relay_state, the flow was already bound, or the pending flow's
 * `expires_at` has already passed).
 */
export async function bindCodeToFlow(
  db: NeonClient,
  relayState: string,
  args: BindCodeToFlowArgs,
): Promise<BindCodeToFlowResult> {
  const rows = await db`
    UPDATE kb_oauth_flow
    SET code_hash = ${hashToken(args.code)},
        claims = ${JSON.stringify(args.claims)}::jsonb,
        status = 'code_issued',
        profile_id = ${args.profileId},
        expires_at = ${args.expiresAt.toISOString()}
    WHERE relay_state = ${relayState} AND status = 'pending_saml' AND expires_at > now()
    RETURNING redirect_uri, oauth_state
  `;
  const row = rows[0] as { redirect_uri: string; oauth_state: string } | undefined;
  if (!row) {
    throw new Error("no pending OAuth flow for relay_state (unknown or already bound)");
  }
  return { redirectUri: row.redirect_uri, oauthState: row.oauth_state };
}

/**
 * Consumes a one-time authorization code, validating its PKCE verifier and binding it to the
 * client that is redeeming it. Order matters: PKCE is checked BEFORE the code is atomically
 * claimed, so a wrong verifier never burns the code (the caller can retry with the right one). The
 * claim itself is a single, atomic, status-guarded UPDATE so a race between two concurrent
 * redemptions can only succeed once. Both the lookup and the claim are additionally scoped to
 * `clientId` so a code issued for one client can never be redeemed by another.
 */
export interface ConsumedCode {
  claims: MintedClaims;
  /** The profile the ACS resolved for this login; `null` when it could not be resolved. */
  profileId: string | null;
  /**
   * The RFC 8707 resource indicator this flow was authorized for — validated against the
   * instance's served audiences at `/oauth/authorize` and minted into the access token's `aud`.
   */
  audience: string;
}

export async function consumeCode(
  db: NeonClient,
  code: string,
  codeVerifier: string,
  clientId: string,
): Promise<ConsumedCode> {
  const codeHash = hashToken(code);

  const rows = await db`
    SELECT code_challenge, expires_at
    FROM kb_oauth_flow
    WHERE code_hash = ${codeHash} AND status = 'code_issued' AND expires_at > now()
      AND client_id = ${clientId}
  `;
  const row = rows[0] as { code_challenge: string; expires_at: string } | undefined;
  if (!row) {
    throw new Error("unknown, expired, already-consumed, or wrong-client authorization code");
  }

  if (!verifyPkceS256(codeVerifier, row.code_challenge)) {
    throw new Error("PKCE verification failed");
  }

  const claimed = await db`
    UPDATE kb_oauth_flow
    SET status = 'consumed'
    WHERE code_hash = ${codeHash} AND status = 'code_issued' AND client_id = ${clientId}
    RETURNING claims, profile_id, audience
  `;
  const claimedRow = claimed[0] as
    | {
        claims: unknown;
        profile_id: string | null;
        audience: string;
      }
    | undefined;
  if (!claimedRow) {
    throw new Error("authorization code was consumed concurrently");
  }

  return {
    claims: normalizeClaims(claimedRow.claims),
    profileId: claimedRow.profile_id ?? null,
    audience: claimedRow.audience,
  };
}

export interface StoreRefreshTokenArgs {
  token: string;
  clientId: string;
  claims: MintedClaims;
  expiresAt: Date;
  /**
   * End of the CHAIN this token belongs to. Stamped once at the chain's first token and passed
   * back unchanged by every rotation — a successor never gets a fresh one, which is what makes the
   * bound absolute rather than sliding.
   */
  chainExpiresAt: string;
  /**
   * The chain this token joins. `null` STARTS a new chain rooted at this token, which the
   * authorization_code grant is the only caller entitled to do — every rotation passes back the
   * identity it read off the token it rotated, unchanged, so a chain keeps one name for its whole
   * life and the response to a replay can reach all of it.
   */
  chainId: string | null;
  /** The principal who owns this chain, so an administrator can end it. `null` if unresolved. */
  profileId: string | null;
  /**
   * The RFC 8707 resource this chain's login was authorized for. Stamped at the chain's first
   * token and handed back UNCHANGED by every rotation, so a refreshed session mints the audience
   * it originally asked for rather than silently switching to the instance audience behind the
   * MCP middleware's dual-accept. `null` means "no audience recorded" — a chain minted before
   * the column existed, or a login with no requested resource — and rotation falls back to the
   * instance audience, which is the behaviour those chains have always had.
   */
  audience: string | null;
}

/**
 * Persists a newly-issued opaque refresh token (hashed at rest). Answers with the identity of the
 * chain the token joined, or `null` when no chain was minted at all.
 *
 * The id is supplied by the statement rather than left to the column default, because a chain
 * that is being BORN takes its identity from its own first token — `COALESCE(chainId, new_id)`.
 * Generating it here in SQL rather than in the caller keeps the whole of a chain's identity on the
 * database clock and out of the token endpoint's bundle.
 *
 * **The admission predicate lives in this INSERT, not in a statement before it**, and that placement
 * is doing two jobs. It makes the check atomic with the write, closing the window in which a
 * rotation that passed a separate gate could insert its successor after an administrator's revoke
 * had already committed and scanned — a live chain for a principal whose admission just ended.
 * And it means EVERY caller inherits the gate: the authorization_code grant cannot mint a fresh
 * 90-day chain for a principal a revoke has just ended, which a check placed only on the refresh
 * path would have allowed on the subject's very next login.
 *
 * A NULL owner passes deliberately — an unresolved principal cannot be asked, and is not refused
 * for it.
 *
 * **The second predicate is what makes a chain-ending stick.** Rotation is two statements with no
 * transaction across them, so a chain-ending can land between a rotation's guard and this INSERT —
 * finding every row of the chain momentarily dead, revoking nothing, and then watching this insert
 * bring the chain back to life behind it. Reading `kb_oauth_refresh_chain_ends` here closes that
 * gap, because the marker records the ENDING and not merely its effect. A new chain passes it for
 * free: `chain_id = NULL` matches no row, so `NOT EXISTS` holds.
 *
 * `expires_at` is `LEAST(requested, chain_expires_at)`, so a stored token never outlives the chain
 * it belongs to and the rotation guard never has to choose between two disagreeing bounds.
 *
 * **The audience is validated HERE, at the write, and an out-of-set value is never persisted.**
 * Same fail-closed discipline as the `/oauth/authorize` handler, asked against the same
 * `servedAudiences()` set — but at the opposite end of the flow's life. The value being checked
 * arrives from the flow row, so no client input can reach this check unserved; what CAN put an
 * out-of-set value here is configuration, because the set itself is env — drift between the
 * authorize moment and this write, or a misconfigured deployment. That is a deployment fault,
 * and the response is a throw naming the fault rather than a silent skip. Silently declining the
 * chain the way the admission predicate does would hand the client an access token with no
 * refresh token and log nothing about why, on every login, for as long as the drift lasted. The
 * throw is not free either — the refresh grant's rotation has already burned the presented token
 * by the time this fires, which is why `handleToken`'s refresh branch catches it, withdraws the
 * rotation mark, and answers `temporarily_unavailable` instead of letting a platform 500 name
 * nothing and leave the mark reading as a replay.
 *
 * Only the COMPARISON happens in SQL. Both operands originate on the AS host clock (`Date.now()`
 * at the call site), while the rotation guard's `now()` is the database's — so host/DB skew shifts
 * a chain's real length by that skew. Small in practice, named here because "computed in SQL"
 * would otherwise read as "on the database clock", which is not what this does.
 *
 * Note this says nothing about `expires_in` in the token response: that is the ACCESS token's TTL.
 * A refresh token's expiry is never advertised to a client at all.
 */
export async function storeRefreshToken(
  db: NeonClient,
  args: StoreRefreshTokenArgs,
): Promise<string | null> {
  if (args.audience !== null && !servedAudiences().includes(args.audience)) {
    throw new Error(
      `refresh chain audience is not served by this authorization server: ${args.audience}`,
    );
  }
  const rows = await db`
    INSERT INTO kb_oauth_refresh_tokens (
      id, token_hash, client_id, claims, expires_at, chain_expires_at, chain_id, profile_id, audience
    )
    SELECT
      g.new_id, ${hashToken(args.token)}, ${args.clientId}, ${JSON.stringify(args.claims)}::jsonb,
      LEAST(${args.expiresAt.toISOString()}::timestamptz, ${args.chainExpiresAt}::timestamptz),
      ${args.chainExpiresAt}::timestamptz,
      COALESCE(${args.chainId}::uuid, g.new_id),
      ${args.profileId},
      ${args.audience}
    FROM (SELECT uuid_generate_v7() AS new_id) g
    WHERE (${args.profileId}::uuid IS NULL
           OR principal_may_refresh(${args.profileId}::uuid))
      AND NOT EXISTS (
            SELECT 1 FROM kb_oauth_refresh_chain_ends
             WHERE chain_id = ${args.chainId}::uuid
          )
    RETURNING chain_id
  `;
  const row = rows[0] as { chain_id: string } | undefined;
  return row?.chain_id ?? null;
}

export interface RotateRefreshTokenResult {
  /** The row that was just spent, so a caller that mints no successor can withdraw its mark. */
  tokenId: string;
  claims: MintedClaims;
  // The token endpoint needs the original client_id to store the successor
  // refresh token, since a public client's refresh-token request carries no
  // client_id of its own — it's recovered from the token being rotated.
  clientId: string;
  /** The chain's absolute end, to be handed to the successor UNCHANGED. */
  chainExpiresAt: string;
  /** The chain's identity, to be handed to the successor UNCHANGED. */
  chainId: string;
  /** The chain's owner, if one was resolved when it was minted. */
  profileId: string | null;
  /**
   * The RFC 8707 resource the chain's login was authorized for, handed to the successor
   * UNCHANGED — the same inheritance rule as the chain deadline. `null` on a chain with no
   * recorded audience (predating the column, or no requested resource): the caller then mints
   * the instance audience, the fallback those chains have always had.
   */
  audience: string | null;
}

/**
 * Redeems a refresh token exactly once: atomically marks it revoked (guarded
 * by `revoked_at IS NULL AND expires_at > now()`) and returns its claims plus
 * owning client_id so the caller can mint a new access token and store a
 * successor refresh token. Throws if the token is unknown, already revoked, or expired.
 *
 * **`rotated_at` is set in this statement and not a later one.** It is the mark that says this row
 * was retired BY ROTATION. Rotation is one of FIVE writers of `revoked_at`: three administrative
 * revokers (`revokeRefreshToken` below, `standing_service::apply`'s terminal hook,
 * `slack_disconnect_service::revoke_as_refresh_token`) and this file's own `endRefreshChain`. All
 * four of the others leave `rotated_at` NULL, each held to it by a test in the module that owns it,
 * so the pair of columns says which happened. Setting it here makes that claim atomic with the
 * guard that earns it.
 *
 * It is a timestamp rather than a `rotated_to` descent link because such a link can only be stamped
 * once the successor row exists — necessarily a later statement — where this word is part of the
 * guard itself and cannot be interrupted.
 */
export async function rotateRefreshToken(
  db: NeonClient,
  token: string,
): Promise<RotateRefreshTokenResult> {
  const rows = await db`
    UPDATE kb_oauth_refresh_tokens
    SET revoked_at = now(), rotated_at = now()
    WHERE token_hash = ${hashToken(token)} AND revoked_at IS NULL AND expires_at > now()
      AND chain_expires_at > now()
    RETURNING id, claims, client_id, chain_expires_at, chain_id, profile_id, audience
  `;
  const row = rows[0] as
    | {
        id: string;
        claims: unknown;
        client_id: string;
        chain_expires_at: unknown;
        chain_id: string;
        profile_id: string | null;
        audience: string | null;
      }
    | undefined;
  if (!row) {
    throw new Error("refresh token is unknown, revoked, expired, or its chain has ended");
  }
  return {
    tokenId: row.id,
    claims: normalizeClaims(row.claims),
    clientId: row.client_id,
    chainExpiresAt: normalizeTimestamp(row.chain_expires_at),
    chainId: row.chain_id,
    profileId: row.profile_id ?? null,
    audience: row.audience ?? null,
  };
}

/** Revokes a refresh token. Idempotent — revoking an already-revoked or unknown token is a no-op. */
export async function revokeRefreshToken(db: NeonClient, token: string): Promise<void> {
  await db`
    UPDATE kb_oauth_refresh_tokens
    SET revoked_at = now()
    WHERE token_hash = ${hashToken(token)} AND revoked_at IS NULL
  `;
}

/** A rotated token that was presented again — the record needed to judge and log the replay. */
export interface RotatedTokenPresentation {
  /** The row that was presented, so the replay record has a subject. */
  tokenId: string;
  /** The chain it belonged to. */
  chainId: string;
  profileId: string | null;
  clientId: string;
  /** When the rotation happened, as recorded. */
  rotatedAt: string;
  /**
   * How long ago that was, **computed on the database clock**. The grace window is judged from
   * this rather than from `Date.now() - rotatedAt`, because the two operands would otherwise come
   * from different clocks: `rotated_at` is the database's `now()` and the AS host's is its own.
   * `storeRefreshToken` already names that skew where it compares two host-clock values; here it
   * would decide whether a user is treated as a thief, so both operands are taken from one clock.
   */
  secondsSinceRotation: number;
}

/**
 * Identifies a refused rotation that is a REPLAY — a token this instance itself retired by
 * rotation, presented again — as opposed to one that is unknown, expired, administratively
 * revoked, or on a chain that has reached its absolute end.
 *
 * Answers `null` for every one of those other cases, and that is the point: they are all answered
 * `invalid_grant` and none of them is evidence of anything. Only a token bearing this instance's
 * own `rotated_at` says the client presented a credential that was already spent, which RFC 6819
 * §5.2.2.3 reads as the chain having been copied.
 *
 * The mark is read, never inferred. A row that does not carry one is not reported as a replay on
 * the strength of anything else it says, which is what keeps an administrator's revoke from being
 * filed as a theft.
 */
export async function findRotatedToken(
  db: NeonClient,
  token: string,
): Promise<RotatedTokenPresentation | null> {
  const rows = await db`
    SELECT id, chain_id, profile_id, client_id, rotated_at,
           EXTRACT(EPOCH FROM (now() - rotated_at))::float8 AS seconds_since_rotation
      FROM kb_oauth_refresh_tokens
     WHERE token_hash = ${hashToken(token)}
       AND rotated_at IS NOT NULL
  `;
  const row = rows[0] as
    | {
        id: string;
        chain_id: string;
        profile_id: string | null;
        client_id: string;
        rotated_at: unknown;
        seconds_since_rotation: number | string;
      }
    | undefined;
  if (!row) {
    return null;
  }
  return {
    tokenId: row.id,
    chainId: row.chain_id,
    profileId: row.profile_id ?? null,
    clientId: row.client_id,
    rotatedAt: normalizeTimestamp(row.rotated_at),
    secondsSinceRotation: Number(row.seconds_since_rotation),
  };
}

/**
 * Ends a whole refresh chain, and answers how many LIVE tokens that actually took.
 *
 * The count is the answer, not a diagnostic. Single-use rotation means a healthy chain has exactly
 * one live token, so `1` is the ordinary result and `0` means the chain was already dead when the
 * replay arrived — a distinction the replay record keeps, because "we ended their session" and "we
 * judged a replay hostile but there was nothing left to end" are different events with the same
 * refusal in front of them. `standing_service::apply` counts its own chain-endings for the same
 * reason and refuses to report a no-op in the words of a success.
 *
 * Reaches the chain and not the principal. Ending every chain a principal owns is what an
 * administrator's revoke does; a replay is evidence about ONE copied chain, and answering it by
 * signing the user out of their other devices would punish them for someone else's theft.
 *
 * **The marker is written in the same statement, and it — not the count — is what ends the chain.**
 * Rotation is two statements with no transaction across them, so this can run at a moment when
 * every row of the chain is momentarily dead: the predecessor revoked by a guard that has already
 * passed, the successor not yet inserted. Revoking rows would then take nothing and the successor
 * would arrive behind us. `kb_oauth_refresh_chain_ends` records the ENDING rather than its effect,
 * and `storeRefreshToken` refuses to mint into a chain that carries one — so the count below is an
 * honest report of what this statement took, and never the thing the ending depends on.
 *
 * Written by this responder only. The administrative revokers keep the one-token-pair excursion
 * `standing_service::apply` states for itself; widening the marker to them would change a
 * documented behaviour in another crate.
 */
export async function endRefreshChain(db: NeonClient, chainId: string): Promise<number> {
  const rows = await db`
    WITH marked AS (
      INSERT INTO kb_oauth_refresh_chain_ends (chain_id)
      VALUES (${chainId}::uuid)
      ON CONFLICT (chain_id) DO NOTHING
    )
    UPDATE kb_oauth_refresh_tokens
       SET revoked_at = now()
     WHERE chain_id = ${chainId}::uuid
       AND revoked_at IS NULL
    RETURNING id
  `;
  return rows.length;
}

/**
 * Withdraws the rotation mark from a token whose rotation minted no successor.
 *
 * The refresh grant runs its admission check AFTER the rotation guard — `storeRefreshToken`'s
 * predicate is what declines, and by then the presented token is already revoked and marked. Left
 * marked, that row is a replay waiting to be reported: the next time the client retries, an
 * ordinary de-provisioned user appears in the operator's view under a hostile count, which is
 * exactly the false theft report the administrative revokers are held away from.
 *
 * `rotated_at` means *retired by a rotation that produced a successor*. When none was produced the
 * chain was ended, not rotated, and the row should say what actually happened.
 */
export async function unmarkRotation(db: NeonClient, tokenId: string): Promise<void> {
  await db`
    UPDATE kb_oauth_refresh_tokens
       SET rotated_at = NULL
     WHERE id = ${tokenId}::uuid
  `;
}

export interface RecordRefreshReplayArgs {
  presentation: RotatedTokenPresentation;
  /** Whether THIS presentation fell inside the grace window and was treated as a client retry. */
  graced: boolean;
  /** How many live tokens the chain-ending revoked — 0 when graced, or when nothing was left. */
  tokensRevoked: number;
}

/**
 * Records the replay where an operator can find it without log retention.
 *
 * **Upserted per token, not appended per presentation.** The write is reachable by anyone holding
 * a retired token, so an append-shaped table would let a replay loop grow it without bound; the
 * primary key caps the table at the token table's own size and the counters carry everything an
 * append would have. `first_seen` is left to the row's default on insert and untouched on conflict.
 *
 * **`first_age_seconds` is carried, not re-derived.** It is the value the grace judgement was
 * actually made on, computed in one expression from one clock. Storing only `first_seen` and
 * letting the view subtract would read a SECOND clock a statement later, so a presentation graced
 * at 9.8 seconds under a 10-second window could be rendered as 10.2 — the operator would find the
 * detector disagreeing with its own record.
 */
export async function recordRefreshReplay(
  db: NeonClient,
  args: RecordRefreshReplayArgs,
): Promise<void> {
  const { presentation: p } = args;
  await db`
    INSERT INTO kb_oauth_refresh_replays (
      token_id, chain_id, profile_id, client_id, rotated_at, first_age_seconds,
      replay_count, graced_count, tokens_revoked
    )
    VALUES (
      ${p.tokenId}::uuid, ${p.chainId}::uuid, ${p.profileId}::uuid, ${p.clientId},
      ${p.rotatedAt}::timestamptz, ${p.secondsSinceRotation},
      1, ${args.graced ? 1 : 0}, ${args.tokensRevoked}
    )
    ON CONFLICT (token_id) DO UPDATE
       SET last_seen      = now(),
           replay_count   = kb_oauth_refresh_replays.replay_count + 1,
           graced_count   = kb_oauth_refresh_replays.graced_count + EXCLUDED.graced_count,
           tokens_revoked = kb_oauth_refresh_replays.tokens_revoked + EXCLUDED.tokens_revoked
  `;
}
