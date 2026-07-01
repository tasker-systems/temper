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
}

export interface BindCodeToFlowResult {
  redirectUri: string;
  oauthState: string;
}

/**
 * Atomically binds a freshly-minted one-time authorization code to a pending
 * flow (found by `relayState`), moving it from `pending_saml` to
 * `code_issued`. Throws if there is no matching pending flow (unknown
 * relay_state, or the flow was already bound).
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
        expires_at = ${args.expiresAt.toISOString()}
    WHERE relay_state = ${relayState} AND status = 'pending_saml'
    RETURNING redirect_uri, oauth_state
  `;
  const row = rows[0] as { redirect_uri: string; oauth_state: string } | undefined;
  if (!row) {
    throw new Error("no pending OAuth flow for relay_state (unknown or already bound)");
  }
  return { redirectUri: row.redirect_uri, oauthState: row.oauth_state };
}

/**
 * Consumes a one-time authorization code, validating its PKCE verifier.
 * Order matters: PKCE is checked BEFORE the code is atomically claimed, so a
 * wrong verifier never burns the code (the caller can retry with the right
 * one). The claim itself is a single, atomic, status-guarded UPDATE so a
 * race between two concurrent redemptions can only succeed once.
 */
export async function consumeCode(
  db: NeonClient,
  code: string,
  codeVerifier: string,
): Promise<MintedClaims> {
  const codeHash = hashToken(code);

  const rows = await db`
    SELECT code_challenge, claims, expires_at
    FROM kb_oauth_flow
    WHERE code_hash = ${codeHash} AND status = 'code_issued' AND expires_at > now()
  `;
  const row = rows[0] as
    | { code_challenge: string; claims: unknown; expires_at: string }
    | undefined;
  if (!row) {
    throw new Error("unknown, expired, or already-consumed authorization code");
  }

  if (!verifyPkceS256(codeVerifier, row.code_challenge)) {
    throw new Error("PKCE verification failed");
  }

  const claimed = await db`
    UPDATE kb_oauth_flow
    SET status = 'consumed'
    WHERE code_hash = ${codeHash} AND status = 'code_issued'
    RETURNING claims
  `;
  const claimedRow = claimed[0] as { claims: unknown } | undefined;
  if (!claimedRow) {
    throw new Error("authorization code was consumed concurrently");
  }

  return normalizeClaims(claimedRow.claims);
}

export interface StoreRefreshTokenArgs {
  token: string;
  clientId: string;
  claims: MintedClaims;
  expiresAt: Date;
}

/** Persists a newly-issued opaque refresh token (hashed at rest). */
export async function storeRefreshToken(
  db: NeonClient,
  args: StoreRefreshTokenArgs,
): Promise<void> {
  await db`
    INSERT INTO kb_oauth_refresh_tokens (token_hash, client_id, claims, expires_at)
    VALUES (
      ${hashToken(args.token)}, ${args.clientId}, ${JSON.stringify(args.claims)}::jsonb, ${args.expiresAt.toISOString()}
    )
  `;
}

export interface RotateRefreshTokenResult {
  claims: MintedClaims;
  // The token endpoint needs the original client_id to store the successor
  // refresh token, since a public client's refresh-token request carries no
  // client_id of its own — it's recovered from the token being rotated.
  clientId: string;
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
    RETURNING claims, client_id
  `;
  const row = rows[0] as { claims: unknown; client_id: string } | undefined;
  if (!row) {
    throw new Error("refresh token is unknown, revoked, or expired");
  }
  return { claims: normalizeClaims(row.claims), clientId: row.client_id };
}

/** Revokes a refresh token. Idempotent — revoking an already-revoked or unknown token is a no-op. */
export async function revokeRefreshToken(db: NeonClient, token: string): Promise<void> {
  await db`
    UPDATE kb_oauth_refresh_tokens
    SET revoked_at = now()
    WHERE token_hash = ${hashToken(token)} AND revoked_at IS NULL
  `;
}
