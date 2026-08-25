// Wires the OAuth-authorize -> SAML round-trip -> authorization-code endpoints. This is thin HTTP
// glue over already-tested building blocks (src/oauth/flow.ts, src/saml/*.ts, src/oauth/mint.ts) --
// no persistence or SAML logic lives here.
import type { NeonClient } from "../db.js";
import { logger } from "../logger.js";
import { loadActiveIdp } from "../saml/config.js";
import { guardReplay } from "../saml/replay.js";
import {
  buildLoginRedirect,
  buildSpMetadata,
  extractGroups,
  mapProfileToClaims,
  validateAssertion,
} from "../saml/sp.js";
import { isRedirectUriAllowed, loadClientRegistry } from "./clients.js";
import {
  bindCodeToFlow,
  consumeCode,
  createPendingFlow,
  principalMayRefresh,
  rotateRefreshToken,
  storeRefreshToken,
} from "./flow.js";
import { touchMachineLastSeen, verifyMachineSecret } from "./machine-clients.js";
import {
  accessTtlSeconds,
  type MintedClaims,
  mintAccessToken,
  mintMachineAccessToken,
  newOpaqueToken,
} from "./mint.js";
import { reconcileMemberships } from "./reconcile.js";
import { resolvePrincipal } from "./resolve.js";

/** How long a pending flow (awaiting the IdP round-trip) stays valid. */
const PENDING_FLOW_TTL_SECONDS = 600;
/** How long a freshly-issued authorization code stays redeemable at /oauth/token. */
const CODE_TTL_SECONDS = 300;
/** How long a consumed SAML assertion ID is retained in the replay guard. */
const REPLAY_TTL_SECONDS = 600;
/** Default TTL for a freshly-issued refresh token, when AS_REFRESH_TTL_SECONDS is unset/invalid. */
const DEFAULT_REFRESH_TTL_SECONDS = 2592000;
/**
 * Default absolute lifetime of a refresh CHAIN, when AS_REFRESH_CHAIN_MAX_SECONDS is unset/invalid.
 * 90 days — a meaningful multiple of the 30-day per-token TTL, so rotation still does its job,
 * and a bound an operator can state at an offboarding review. Mirrored as a literal in the
 * migration's backfill (20260825000010), which cannot read this env.
 */
const DEFAULT_REFRESH_CHAIN_MAX_SECONDS = 7776000;

/** Validated refresh-token TTL, read from AS_REFRESH_TTL_SECONDS (mirrors mint.ts's accessTtlSeconds). */
function refreshTtlSeconds(): number {
  const raw = process.env.AS_REFRESH_TTL_SECONDS;
  if (!raw) {
    return DEFAULT_REFRESH_TTL_SECONDS;
  }
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_REFRESH_TTL_SECONDS;
}

/**
 * Validated chain lifetime, read from AS_REFRESH_CHAIN_MAX_SECONDS.
 *
 * The bound an operator states: how long a session can live at most, measured from the last full
 * SAML login. Rotation cannot move it; only another login can. It is also the arm that holds
 * independently of standing — reconciliation against the IdP acts on `source='idp'` team
 * memberships and leaves standing `approved`, so a clock covers that axis where an admission check
 * does not.
 */
function refreshChainMaxSeconds(): number {
  const raw = process.env.AS_REFRESH_CHAIN_MAX_SECONDS;
  if (!raw) {
    return DEFAULT_REFRESH_CHAIN_MAX_SECONDS;
  }
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_REFRESH_CHAIN_MAX_SECONDS;
}

function badRequest(reason: string): Response {
  return new Response(reason, { status: 400 });
}

function serviceUnavailable(reason: string): Response {
  return new Response(reason, { status: 503 });
}

function redirect(location: string): Response {
  return new Response(null, { status: 302, headers: { location } });
}

function isValidUrl(value: string): boolean {
  try {
    new URL(value);
    return true;
  } catch {
    return false;
  }
}

/**
 * `GET /oauth/authorize` — the OAuth entry point. Validates the PKCE authorize request, stashes it
 * as a pending flow keyed by a fresh opaque relay_state, and hands off to the SAML login redirect.
 */
export async function handleAuthorize(req: Request, db: NeonClient): Promise<Response> {
  const params = new URL(req.url).searchParams;
  const responseType = params.get("response_type");
  const clientId = params.get("client_id");
  const redirectUri = params.get("redirect_uri");
  const codeChallenge = params.get("code_challenge");
  const codeChallengeMethod = params.get("code_challenge_method");
  const state = params.get("state");

  if (responseType !== "code") {
    return badRequest("response_type must be 'code'");
  }
  if (!clientId) {
    return badRequest("client_id is required");
  }
  if (!redirectUri || !isValidUrl(redirectUri)) {
    return badRequest("redirect_uri is required and must be a valid URL");
  }
  if (!codeChallenge) {
    return badRequest("code_challenge is required");
  }
  if (codeChallengeMethod !== "S256") {
    return badRequest("code_challenge_method must be 'S256'");
  }
  if (!state) {
    return badRequest("state is required");
  }

  const registry = loadClientRegistry();
  if (!isRedirectUriAllowed(registry, clientId, redirectUri)) {
    return badRequest("unregistered client_id or redirect_uri");
  }

  const relayState = newOpaqueToken();
  await createPendingFlow(db, {
    relayState,
    clientId,
    redirectUri,
    codeChallenge,
    codeChallengeMethod: "S256",
    oauthState: state,
    audience: process.env.AS_AUDIENCE ?? "",
    expiresAt: new Date(Date.now() + PENDING_FLOW_TTL_SECONDS * 1000),
  });

  return redirect(`/oauth/saml/login?rs=${encodeURIComponent(relayState)}`);
}

/** `GET /oauth/saml/login` — redirects the browser to the configured IdP's SSO endpoint. */
export async function handleSamlLogin(req: Request, db: NeonClient): Promise<Response> {
  const rs = new URL(req.url).searchParams.get("rs");
  if (!rs) {
    return badRequest("rs is required");
  }

  const idp = await loadActiveIdp(db);
  if (!idp) {
    return serviceUnavailable("no active IdP configured");
  }

  const location = await buildLoginRedirect(idp, rs);
  return redirect(location);
}

/**
 * `POST /oauth/saml/acs` — the SAML Assertion Consumer Service endpoint. Validates the posted
 * assertion, guards against replay, maps the profile to claims, and binds a fresh authorization
 * code to the pending flow before redirecting back to the client's redirect_uri.
 */
export async function handleSamlAcs(req: Request, db: NeonClient): Promise<Response> {
  const form = await req.formData();
  const samlResponse = form.get("SAMLResponse");
  const relayState = form.get("RelayState");
  if (!samlResponse || !relayState) {
    return badRequest("SAMLResponse and RelayState are required");
  }

  const idp = await loadActiveIdp(db);
  if (!idp) {
    return serviceUnavailable("no active IdP configured");
  }

  try {
    const { profile, assertionId } = await validateAssertion(idp, String(samlResponse));
    await guardReplay(db, assertionId, new Date(Date.now() + REPLAY_TTL_SECONDS * 1000));
    const claims = mapProfileToClaims(profile, idp);

    // Phase 2: reconcile IdP-driven team memberships before minting. Fail-open — a provisioning
    // error must never block authentication (design spec §3.8). Its own try/catch so a reconcile
    // failure is NOT misreported as an assertion rejection by the outer catch.
    try {
      const groups = extractGroups(profile, idp);
      // Signal-missing guard: null means the assertion carried no group signal (groups_attr not
      // configured, or the attribute absent from THIS assertion). Skip reconcile so a transient
      // IdP attribute-drop never revokes memberships. A present-but-empty list ([]) IS a signal
      // ("in no mapped groups now") and DOES reconcile, revoking stale idp rows.
      if (groups !== null) {
        await reconcileMemberships({
          provider: `saml:${idp.idp_key}`,
          external_user_id: claims.sub,
          email: claims.email,
          email_verified: claims.email_verified,
          idp_key: idp.idp_key,
          groups,
        });
      }
    } catch (reconcileErr) {
      logger.error(
        { err: reconcileErr instanceof Error ? reconcileErr.message : String(reconcileErr) },
        "SAML ACS: membership reconcile failed (fail-open, login proceeds)",
      );
    }

    // Who this login is, so the refresh chain the token endpoint is about to mint carries an owner
    // an administrator can end. Fail-open on its own, exactly like the reconcile above and for the
    // same reason (design spec §3.8): a principal we could not name must still be able to sign in.
    // The cost of failing is bounded and stated — that chain is outside the standing hook's reach
    // until its next rotation re-resolves it, and is held by its absolute lifetime meanwhile.
    let profileId: string | null = null;
    try {
      profileId = (
        await resolvePrincipal({
          external_user_id: claims.sub,
          email: claims.email,
          email_verified: claims.email_verified,
        })
      ).profile_id;
    } catch (resolveErr) {
      logger.error(
        { err: resolveErr instanceof Error ? resolveErr.message : String(resolveErr) },
        "SAML ACS: principal resolve failed (fail-open, login proceeds without a chain owner)",
      );
    }

    const code = newOpaqueToken();
    const { redirectUri, oauthState } = await bindCodeToFlow(db, String(relayState), {
      code,
      claims,
      expiresAt: new Date(Date.now() + CODE_TTL_SECONDS * 1000),
      profileId,
    });

    const u = new URL(redirectUri);
    u.searchParams.set("code", code);
    u.searchParams.set("state", oauthState);
    return redirect(u.toString());
  } catch (err) {
    // node-saml errors can embed assertion XML (NameID, email, other PII) in their message/stack --
    // log only the message, never the full error object.
    logger.error(
      { err: err instanceof Error ? err.message : String(err) },
      "SAML ACS: assertion rejected",
    );
    return badRequest("SAML assertion rejected");
  }
}

/** The `/oauth/token` success response body (RFC 6749 §5.1). */
interface TokenResponse {
  access_token: string;
  token_type: "Bearer";
  expires_in: number;
  refresh_token: string;
}

/** The `/oauth/token` success body for client_credentials — no refresh token (RFC 6749 §4.4.3). */
interface MachineTokenResponse {
  access_token: string;
  token_type: "Bearer";
  expires_in: number;
}

/** An RFC 6749 §5.2 OAuth error response body. */
interface OAuthErrorBody {
  error: string;
  error_description?: string;
}

function oauthJson(
  body: TokenResponse | MachineTokenResponse | OAuthErrorBody,
  status: number,
): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });
}

function oauthError(error: string, status = 400): Response {
  return oauthJson({ error }, status);
}

/**
 * Mints an access token + a fresh opaque refresh token for `claims`, persists the refresh token
 * (scoped to `clientId`), and returns the RFC 6749 §5.1 success body. Shared by both the
 * authorization_code and refresh_token grants in `handleToken`.
 *
 * `chainExpiresAt` is the load-bearing parameter: the authorization_code grant computes a fresh one
 * (a new chain), and the refresh grant passes back the one it read off the token it rotated. There
 * is deliberately no default — a caller that omits it fails to compile rather than choosing a
 * deadline by accident.
 */
async function issueTokenPair(
  db: NeonClient,
  claims: MintedClaims,
  clientId: string,
  chain: { expiresAt: string; profileId: string | null },
): Promise<TokenResponse> {
  const accessToken = await mintAccessToken(claims);
  const refreshToken = newOpaqueToken();
  await storeRefreshToken(db, {
    token: refreshToken,
    clientId,
    claims,
    expiresAt: new Date(Date.now() + refreshTtlSeconds() * 1000),
    chainExpiresAt: chain.expiresAt,
    profileId: chain.profileId,
  });

  return {
    access_token: accessToken,
    token_type: "Bearer",
    expires_in: accessTtlSeconds(),
    refresh_token: refreshToken,
  };
}

/** Reads client credentials from HTTP Basic (preferred, RFC 6749 §2.3.1) or the form body. */
function readClientCredentials(
  req: Request,
  form: FormData,
): { clientId: string; clientSecret: string } | null {
  const auth = req.headers.get("authorization");
  if (auth?.startsWith("Basic ")) {
    const decoded = Buffer.from(auth.slice("Basic ".length), "base64").toString("utf8");
    const sep = decoded.indexOf(":");
    if (sep > 0) {
      return { clientId: decoded.slice(0, sep), clientSecret: decoded.slice(sep + 1) };
    }
  }
  const clientId = String(form.get("client_id") ?? "");
  const clientSecret = String(form.get("client_secret") ?? "");
  return clientId && clientSecret ? { clientId, clientSecret } : null;
}

/**
 * `POST /oauth/token` — exchanges an authorization code (grant_type=authorization_code) or an
 * existing refresh token (grant_type=refresh_token) for a fresh access token + refresh token pair.
 * Both grants issue the pair via `issueTokenPair`; the refresh grant rotates (single-use) via
 * `rotateRefreshToken`. The client_credentials grant mints an access-token-only response for a
 * temper-issued machine principal (Phase B1).
 */
export async function handleToken(req: Request, db: NeonClient): Promise<Response> {
  // RFC 6749 §4: the token endpoint takes `application/x-www-form-urlencoded`. A client sending
  // JSON (as Auth0 tolerates) makes `formData()` throw — without this guard that surfaces as a 500,
  // which reads to the caller as "the server is broken" rather than "you encoded the request wrong".
  let form: FormData;
  try {
    form = await req.formData();
  } catch {
    return oauthError("invalid_request");
  }
  const grantType = String(form.get("grant_type") ?? "");

  if (grantType === "authorization_code") {
    const code = form.get("code");
    const codeVerifier = form.get("code_verifier");
    const clientId = String(form.get("client_id") ?? "");
    if (!code || !codeVerifier || !clientId) {
      return oauthError("invalid_request");
    }

    let consumed: Awaited<ReturnType<typeof consumeCode>>;
    try {
      consumed = await consumeCode(db, String(code), String(codeVerifier), clientId);
    } catch {
      return oauthError("invalid_grant");
    }

    // A fresh login starts a NEW chain, and this is the only place a chain deadline is computed.
    return oauthJson(
      await issueTokenPair(db, consumed.claims, clientId, {
        expiresAt: new Date(Date.now() + refreshChainMaxSeconds() * 1000).toISOString(),
        profileId: consumed.profileId,
      }),
      200,
    );
  }

  if (grantType === "refresh_token") {
    const refreshToken = form.get("refresh_token");
    if (!refreshToken) {
      return oauthError("invalid_request");
    }

    let rotated: Awaited<ReturnType<typeof rotateRefreshToken>>;
    try {
      rotated = await rotateRefreshToken(db, String(refreshToken));
    } catch {
      return oauthError("invalid_grant");
    }

    // A chain minted during a fail-open login, or by a binary that predates the owner column,
    // carries none. Re-resolve here so the successor — the row that will actually be presented
    // next — carries the owner, rather than waiting out the chain's remaining lifetime.
    let profileId = rotated.profileId;
    if (profileId === null) {
      try {
        profileId = (
          await resolvePrincipal({
            external_user_id: rotated.claims.sub,
            email: rotated.claims.email,
            email_verified: rotated.claims.email_verified,
          })
        ).profile_id;
      } catch (resolveErr) {
        logger.error(
          { err: resolveErr instanceof Error ? resolveErr.message : String(resolveErr) },
          "token: chain owner re-resolve failed (chain stays bounded by its absolute lifetime)",
        );
      }
    }

    // Admission ended ⇒ no new pair, answered HERE rather than only at the API gate the minted
    // token would later meet. `principal_may_refresh` is the SQL predicate, not a restatement of
    // it: the same terminal set `standing_service::apply` ends chains for, and `denied` /
    // `requested` principals pass it deliberately — they hold tokens to reach the ungated
    // join-request surface. An unresolved owner cannot be asked, and is not refused for it.
    if (profileId !== null && !(await principalMayRefresh(db, profileId))) {
      return oauthError("invalid_grant");
    }

    return oauthJson(
      await issueTokenPair(db, rotated.claims, rotated.clientId, {
        // Handed back UNCHANGED. This is the line that makes the bound absolute.
        expiresAt: rotated.chainExpiresAt,
        profileId,
      }),
      200,
    );
  }

  if (grantType === "client_credentials") {
    const creds = readClientCredentials(req, form);
    if (!creds) {
      return oauthError("invalid_request");
    }
    if (!(await verifyMachineSecret(db, creds.clientId, creds.clientSecret))) {
      return oauthError("invalid_client", 401);
    }
    await touchMachineLastSeen(db, creds.clientId);
    const accessToken = await mintMachineAccessToken(creds.clientId);
    return oauthJson(
      { access_token: accessToken, token_type: "Bearer", expires_in: accessTtlSeconds() },
      200,
    );
  }

  return oauthError("unsupported_grant_type");
}

/** `GET /oauth/saml/metadata` — serves this instance's SP metadata XML for IdP-side configuration. */
export async function handleSamlMetadata(_req: Request, db: NeonClient): Promise<Response> {
  const idp = await loadActiveIdp(db);
  if (!idp) {
    return serviceUnavailable("no active IdP configured");
  }

  return new Response(buildSpMetadata(idp), {
    status: 200,
    headers: { "content-type": "application/xml" },
  });
}
