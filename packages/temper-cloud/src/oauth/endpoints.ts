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
  mapProfileToClaims,
  validateAssertion,
} from "../saml/sp.js";
import {
  bindCodeToFlow,
  consumeCode,
  createPendingFlow,
  rotateRefreshToken,
  storeRefreshToken,
} from "./flow.js";
import { mintAccessToken, newOpaqueToken } from "./mint.js";

/** How long a pending flow (awaiting the IdP round-trip) stays valid. */
const PENDING_FLOW_TTL_SECONDS = 600;
/** How long a freshly-issued authorization code stays redeemable at /oauth/token. */
const CODE_TTL_SECONDS = 300;
/** How long a consumed SAML assertion ID is retained in the replay guard. */
const REPLAY_TTL_SECONDS = 600;
/** How long a freshly-issued refresh token stays valid before it must be rotated. */
const REFRESH_TTL_SECONDS = 2592000;

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
    const code = newOpaqueToken();
    const { redirectUri, oauthState } = await bindCodeToFlow(db, String(relayState), {
      code,
      claims,
      expiresAt: new Date(Date.now() + CODE_TTL_SECONDS * 1000),
    });

    const u = new URL(redirectUri);
    u.searchParams.set("code", code);
    u.searchParams.set("state", oauthState);
    return redirect(u.toString());
  } catch (err) {
    logger.error({ err }, "SAML ACS: assertion rejected");
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

/** An RFC 6749 §5.2 OAuth error response body. */
interface OAuthErrorBody {
  error: string;
  error_description?: string;
}

function oauthJson(body: TokenResponse | OAuthErrorBody, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });
}

function oauthError(error: string, status = 400): Response {
  return oauthJson({ error }, status);
}

function accessTtlSeconds(): number {
  return Number(process.env.AS_ACCESS_TTL_SECONDS ?? 900);
}

/**
 * `POST /oauth/token` — exchanges an authorization code (grant_type=authorization_code) or an
 * existing refresh token (grant_type=refresh_token) for a fresh access token + refresh token pair.
 * Both grants mint the access token via `mintAccessToken` and issue a new opaque refresh token,
 * storing it via `storeRefreshToken`; the refresh grant rotates (single-use) via `rotateRefreshToken`.
 */
export async function handleToken(req: Request, db: NeonClient): Promise<Response> {
  const form = await req.formData();
  const grantType = String(form.get("grant_type") ?? "");

  if (grantType === "authorization_code") {
    const code = form.get("code");
    const codeVerifier = form.get("code_verifier");
    const clientId = String(form.get("client_id") ?? "");
    if (!code || !codeVerifier) {
      return oauthError("invalid_request");
    }

    let claims: Awaited<ReturnType<typeof consumeCode>>;
    try {
      claims = await consumeCode(db, String(code), String(codeVerifier));
    } catch {
      return oauthError("invalid_grant");
    }

    const accessToken = await mintAccessToken(claims);
    const refreshToken = newOpaqueToken();
    await storeRefreshToken(db, {
      token: refreshToken,
      clientId,
      claims,
      expiresAt: new Date(Date.now() + REFRESH_TTL_SECONDS * 1000),
    });

    return oauthJson(
      {
        access_token: accessToken,
        token_type: "Bearer",
        expires_in: accessTtlSeconds(),
        refresh_token: refreshToken,
      },
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

    const accessToken = await mintAccessToken(rotated.claims);
    const newRefresh = newOpaqueToken();
    await storeRefreshToken(db, {
      token: newRefresh,
      clientId: rotated.clientId,
      claims: rotated.claims,
      expiresAt: new Date(Date.now() + REFRESH_TTL_SECONDS * 1000),
    });

    return oauthJson(
      {
        access_token: accessToken,
        token_type: "Bearer",
        expires_in: accessTtlSeconds(),
        refresh_token: newRefresh,
      },
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
