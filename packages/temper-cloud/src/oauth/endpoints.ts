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
import { bindCodeToFlow, createPendingFlow } from "./flow.js";
import { newOpaqueToken } from "./mint.js";

/** How long a pending flow (awaiting the IdP round-trip) stays valid. */
const PENDING_FLOW_TTL_SECONDS = 600;
/** How long a freshly-issued authorization code stays redeemable at /oauth/token. */
const CODE_TTL_SECONDS = 300;
/** How long a consumed SAML assertion ID is retained in the replay guard. */
const REPLAY_TTL_SECONDS = 600;

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
