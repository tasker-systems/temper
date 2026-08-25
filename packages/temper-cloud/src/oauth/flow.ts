import type { NeonClient } from "../db.js";
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
    RETURNING claims, profile_id
  `;
  const claimedRow = claimed[0] as { claims: unknown; profile_id: string | null } | undefined;
  if (!claimedRow) {
    throw new Error("authorization code was consumed concurrently");
  }

  return {
    claims: normalizeClaims(claimedRow.claims),
    profileId: claimedRow.profile_id ?? null,
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
  /** The principal who owns this chain, so an administrator can end it. `null` if unresolved. */
  profileId: string | null;
}

/**
 * Persists a newly-issued opaque refresh token (hashed at rest).
 *
 * `expires_at` is `LEAST(requested, chain_expires_at)` — computed in SQL against the same clock the
 * rotation guard reads. A token is never handed out advertising an expiry its own chain bound will
 * not honour, so `expires_in` and the chain never disagree in front of a client.
 */
export async function storeRefreshToken(
  db: NeonClient,
  args: StoreRefreshTokenArgs,
): Promise<void> {
  await db`
    INSERT INTO kb_oauth_refresh_tokens (
      token_hash, client_id, claims, expires_at, chain_expires_at, profile_id
    )
    VALUES (
      ${hashToken(args.token)}, ${args.clientId}, ${JSON.stringify(args.claims)}::jsonb,
      LEAST(${args.expiresAt.toISOString()}::timestamptz, ${args.chainExpiresAt}::timestamptz),
      ${args.chainExpiresAt}::timestamptz, ${args.profileId}
    )
  `;
}

export interface RotateRefreshTokenResult {
  claims: MintedClaims;
  // The token endpoint needs the original client_id to store the successor
  // refresh token, since a public client's refresh-token request carries no
  // client_id of its own — it's recovered from the token being rotated.
  clientId: string;
  /** The chain's absolute end, to be handed to the successor UNCHANGED. */
  chainExpiresAt: string;
  /** The chain's owner, if one was resolved when it was minted. */
  profileId: string | null;
}

/**
 * Redeems a refresh token exactly once: atomically marks it revoked (guarded
 * by `revoked_at IS NULL AND expires_at > now()`) and returns its claims plus
 * owning client_id so the caller can mint a new access token and store a
 * successor refresh token. `rotated_to` is intentionally left unset in Phase
 * 1 — `revoked_at` alone enforces single-use; linking successors is deferred.
 * Throws if the token is unknown, already revoked, or expired.
 */
export async function rotateRefreshToken(
  db: NeonClient,
  token: string,
): Promise<RotateRefreshTokenResult> {
  const rows = await db`
    UPDATE kb_oauth_refresh_tokens
    SET revoked_at = now()
    WHERE token_hash = ${hashToken(token)} AND revoked_at IS NULL AND expires_at > now()
      AND chain_expires_at > now()
    RETURNING claims, client_id, chain_expires_at, profile_id
  `;
  const row = rows[0] as
    | {
        claims: unknown;
        client_id: string;
        chain_expires_at: unknown;
        profile_id: string | null;
      }
    | undefined;
  if (!row) {
    throw new Error("refresh token is unknown, revoked, expired, or its chain has ended");
  }
  return {
    claims: normalizeClaims(row.claims),
    clientId: row.client_id,
    chainExpiresAt: normalizeTimestamp(row.chain_expires_at),
    profileId: row.profile_id ?? null,
  };
}

/**
 * May this principal be handed a new token pair?
 *
 * Reads the `principal_may_refresh` SQL predicate (migration 20260825000010) rather than restating
 * what "admission has ended" means — the same set `standing_service::apply` revokes live chains
 * for, asked from the other side. A restated copy here would be a true-looking statement about
 * real columns that nothing could compare against the incumbent.
 */
export async function principalMayRefresh(db: NeonClient, profileId: string): Promise<boolean> {
  const rows = await db`SELECT principal_may_refresh(${profileId}::uuid) AS ok`;
  const row = rows[0] as { ok: boolean | null } | undefined;
  return row?.ok === true;
}

/** Revokes a refresh token. Idempotent — revoking an already-revoked or unknown token is a no-op. */
export async function revokeRefreshToken(db: NeonClient, token: string): Promise<void> {
  await db`
    UPDATE kb_oauth_refresh_tokens
    SET revoked_at = now()
    WHERE token_hash = ${hashToken(token)} AND revoked_at IS NULL
  `;
}
