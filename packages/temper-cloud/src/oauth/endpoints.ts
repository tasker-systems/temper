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
  endRefreshChain,
  findRotatedToken,
  recordRefreshReplay,
  rotateRefreshToken,
  storeRefreshToken,
  unmarkRotation,
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
/**
 * Ceiling on a configured chain lifetime — ten years. Not a policy, a units check: anything above
 * this is a seconds-vs-milliseconds slip rather than an intent, and a chain that outlives the
 * deployment is not a bound.
 */
const MAX_REFRESH_CHAIN_SECONDS = 315360000;
/**
 * Default grace window for a replayed refresh token, when AS_REFRESH_REPLAY_GRACE_SECONDS is unset
 * or unusable. Ten seconds.
 *
 * **This is the boundary between a client and a thief, so it is stated rather than implied.** A
 * rotated token that comes back is, per RFC 6819 §5.2.2.3, the canonical indication that a chain
 * has been copied — but it is also what an ordinary client produces in two benign ways. It may
 * have lost the response carrying the successor and retried; or it may have had two requests in
 * flight at once, one of which lost the race. Both arrive within seconds of the rotation, because
 * both are the same HTTP exchange still happening. A replay hours or days later has neither
 * explanation available to it.
 *
 * **Ten and not sixty, because the two benign cases cost differently to get wrong, and only one of
 * them is what this window is for.** A client that lost the response holds no live token and must
 * re-authenticate whatever we do — ending its chain costs it nothing, and mislabelling its retry
 * costs an operator one row to read past. The concurrent-refresh client is the one that actually
 * holds the successor, and its two requests are one exchange still in flight: that resolves in well
 * under a second, not in a minute.
 *
 * The width has a cost running the other way, which is why it is not generous. The AS only ever
 * sees the LOSER of a rotation race and cannot tell whether the winner was the client or someone
 * holding a copy. Inside the window it lets the winner keep the session and files the presentation
 * as benign — so every second of width is a second in which a fresh theft is answered gently and
 * recorded as a retry. Ten covers the case this exists for and leaves six times less of that.
 *
 * Zero is a legal setting and means the BCP's strictest reading: every replay ends the chain.
 */
const DEFAULT_REFRESH_REPLAY_GRACE_SECONDS = 10;
/** Ceiling on a configured replay grace window — one hour. A units check, not a policy. */
const MAX_REPLAY_GRACE_SECONDS = 3600;

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
  if (!Number.isFinite(parsed) || parsed <= 0 || parsed > MAX_REFRESH_CHAIN_SECONDS) {
    // REFUSE rather than substitute, which is where this parser parts company with its two
    // siblings above. Those describe how often a token is reminted; this one is the number the
    // playbook tells an operator to state at an offboarding review. Silently swallowing
    // `AS_REFRESH_CHAIN_MAX_SECONDS=7d` and serving 90 days would make what they deployed and what
    // they reported differ by an order of magnitude, with nothing anywhere disagreeing.
    //
    // The upper bound catches the same mistake in the other direction: a value in milliseconds is
    // finite, positive, and yields a deadline centuries out — an unbounded chain wearing a bound's
    // clothes. Past ~1e13 it also overflows `new Date(...)`, which would surface as a 500 on every
    // login rather than as anything a reader could diagnose.
    throw new Error(
      `AS_REFRESH_CHAIN_MAX_SECONDS must be a positive number of seconds no greater than ` +
        `${MAX_REFRESH_CHAIN_SECONDS}; got ${JSON.stringify(raw)}`,
    );
  }
  return parsed;
}

/**
 * Validated replay grace window, read from AS_REFRESH_REPLAY_GRACE_SECONDS.
 *
 * **Substitutes and warns rather than refusing**, which is the opposite of `refreshChainMaxSeconds`
 * above, and the difference is what a refusal would actually cost. That one is read once per login,
 * in a branch that answers 503 — the refusal reaches the operator, and a login is the thing being
 * protected. This one is read on a path whose caller catches everything, precisely so a failed
 * audit write cannot change the client's answer; a throw here would therefore surface as nothing at
 * all, leaving the detector configured and inert while every refused refresh logged an error. A
 * default that warns loudly and keeps detecting is the better failure.
 *
 * `MAX_REPLAY_GRACE_SECONDS` is a units check, not a policy: an hour is already orders of magnitude
 * beyond any client retry, so a larger value is a seconds-vs-milliseconds slip, and honouring it
 * would leave the detector configured and inert. What a wrong value here can cost is bounded by
 * that ceiling; the widest chain bound that would pass `refreshChainMaxSeconds`' own validation
 * outlives the deployment, which is why that one refuses instead of substituting.
 */
function refreshReplayGraceSeconds(): number {
  const raw = process.env.AS_REFRESH_REPLAY_GRACE_SECONDS;
  if (!raw) {
    return DEFAULT_REFRESH_REPLAY_GRACE_SECONDS;
  }
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || parsed < 0 || parsed > MAX_REPLAY_GRACE_SECONDS) {
    logger.warn(
      { configured: raw, using: DEFAULT_REFRESH_REPLAY_GRACE_SECONDS },
      "token: AS_REFRESH_REPLAY_GRACE_SECONDS is not a usable number of seconds; using the default",
    );
    return DEFAULT_REFRESH_REPLAY_GRACE_SECONDS;
  }
  return parsed;
}

/**
 * What a refused rotation means BEYOND the refusal, and what it costs.
 *
 * Every branch here ends in the same `invalid_grant` the caller was already going to send — this
 * decides what the instance RECORDS and whether the chain survives, and deliberately not what the
 * client is told. Telling a client which of "unknown", "expired", "revoked" and "already spent" it
 * hit would hand an attacker holding a stolen token a probe for whether it is worth replaying.
 *
 * A token with no `rotated_at` is not a replay: unknown, expired, administratively revoked, or on
 * a chain that reached its absolute end. None of those is evidence of anything and none is
 * recorded here.
 *
 * Nothing in here can fail the request, which is why the whole body is wrapped by its caller. The
 * ordering inside is chosen so the record cannot overstate: the chain is ended FIRST and the count
 * it actually took is what gets written, rather than writing an intention and hoping.
 */
async function judgeRefusedRotation(db: NeonClient, token: string): Promise<void> {
  const presented = await findRotatedToken(db, token);
  if (!presented) {
    return;
  }

  const grace = refreshReplayGraceSeconds();
  const graced = presented.secondsSinceRotation <= grace;
  const tokensRevoked = graced ? 0 : await endRefreshChain(db, presented.chainId);

  // Logged BEFORE the record is written. The upsert below is the one statement here that can fail,
  // and its caller's handler sees only an error — so emitting the identifying fields first is what
  // leaves a trail on the single path where the table this feature exists for is unavailable.
  logger.warn(
    {
      token_id: presented.tokenId,
      chain_id: presented.chainId,
      profile_id: presented.profileId,
      client_id: presented.clientId,
      seconds_since_rotation: presented.secondsSinceRotation,
      grace_seconds: grace,
      graced,
      tokens_revoked: tokensRevoked,
    },
    graced
      ? "token: a rotated refresh token was presented again inside the grace window — refused as a client retry, chain left live"
      : tokensRevoked > 0
        ? "token: a rotated refresh token was presented again — the chain has been ended"
        : // Say which. The chain is ended either way — the marker went down in the same statement —
          // but "we signed a user out" and "there was nothing live left to take" are different
          // events, and reporting both in the words of the first is the failure
          // `standing_service::apply` already refuses to make in the other direction.
          "token: a rotated refresh token was presented again — the chain is ended, and held no live token to revoke",
  );

  await recordRefreshReplay(db, { presentation: presented, graced, tokensRevoked });
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
      // Signal-missing guard: null means the assertion carried no group signal (groups_attr not
      // configured, or the attribute absent from THIS assertion), and must never revoke
      // memberships on a transient IdP attribute-drop. A present-but-empty list ([]) IS a signal
      // ("in no mapped groups now") and DOES revoke stale idp rows.
      //
      // The null is PASSED ON rather than used to skip the call. Skipping made the guard's refusal
      // indistinguishable from a principal who simply never logged in — no call, no span, nothing
      // to find — and a de-provisioning that quietly stops happening is the case with no failure
      // signal to catch it. The API refuses the null exactly as this branch did
      // (`saml_provisioning_service::reconcile_idp_memberships` takes an `Option` and returns early
      // on `None`, adjacent to the DELETE it protects) and records that it refused, so the silence
      // becomes a queryable fact.
      await reconcileMemberships({
        provider: `saml:${idp.idp_key}`,
        external_user_id: claims.sub,
        email: claims.email,
        email_verified: claims.email_verified,
        idp_key: idp.idp_key,
        groups: extractGroups(profile, idp),
      });
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
  /**
   * Absent when the principal's admission has ended. RFC 6749 §5.1 makes this OPTIONAL, and the
   * distinction is deliberate rather than an error: such a principal may still hold a short-lived
   * access token — they need one to reach the ungated join-request and review-request surface —
   * but they do not get a renewable session to carry it forward.
   */
  refresh_token?: string;
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
 * `chain` is the load-bearing parameter, and both of its inherited halves work the same way: the
 * authorization_code grant computes a fresh deadline and passes `id: null` (start a chain), and the
 * refresh grant passes back both the deadline and the identity it read off the token it rotated.
 * There is deliberately no default on either — a caller that omits one fails to compile rather than
 * choosing a deadline, or forking a chain, by accident.
 */
async function issueTokenPair(
  db: NeonClient,
  claims: MintedClaims,
  clientId: string,
  chain: { id: string | null; expiresAt: string; profileId: string | null },
): Promise<TokenResponse> {
  const accessToken = await mintAccessToken(claims);
  const refreshToken = newOpaqueToken();
  const chainId = await storeRefreshToken(db, {
    token: refreshToken,
    clientId,
    claims,
    expiresAt: new Date(Date.now() + refreshTtlSeconds() * 1000),
    chainExpiresAt: chain.expiresAt,
    chainId: chain.id,
    profileId: chain.profileId,
  });

  if (chainId === null) {
    // Say so. The Rust side warns when a terminal transition ends no chains precisely because a
    // refusal must not report success in the same words as a success; this is the same event seen
    // from the other end, and leaving it silent would make a non-renewable session indistinguishable
    // from an ordinary one until the client's next refresh fails minutes later.
    logger.info(
      { profile_id: chain.profileId, client_id: clientId },
      "token: admission has ended for this principal — access token issued, no chain minted",
    );
  }

  return {
    access_token: accessToken,
    token_type: "Bearer",
    expires_in: accessTtlSeconds(),
    ...(chainId !== null ? { refresh_token: refreshToken } : {}),
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

    // A fresh login starts a NEW chain, and this is the only place a chain deadline is computed —
    // which is also the only place the configured bound is parsed, so a bad value lands HERE and
    // only here. Refusing an unusable value is right; letting the refusal escape as an uncaught
    // 500 is not. It would be a platform error with no reason attached, on new logins only, while
    // existing sessions kept rotating — the hardest possible shape to attribute to a typo in an
    // environment variable.
    let chainDeadline: string;
    try {
      chainDeadline = new Date(Date.now() + refreshChainMaxSeconds() * 1000).toISOString();
    } catch (configErr) {
      logger.error(
        { err: configErr instanceof Error ? configErr.message : String(configErr) },
        "token: refresh-chain bound is unusable; refusing to mint a session on a bound nobody can state",
      );
      return oauthError("temporarily_unavailable", 503);
    }

    //
    // Login is never refused on admission — a principal whose standing has ended still needs a
    // token to reach the ungated review-request surface. What they do not get is a renewable
    // session: `storeRefreshToken`'s predicate declines to mint the chain, so the response carries
    // an access token and no refresh token, and the revoke that ended their standing is not undone
    // by their next sign-in.
    return oauthJson(
      await issueTokenPair(db, consumed.claims, clientId, {
        // A fresh login starts a chain, so it has no identity to inherit — `storeRefreshToken`
        // roots the new one at this very token.
        id: null,
        expiresAt: chainDeadline,
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
      // The refusal is settled; what follows only decides what is RECORDED and whether a chain
      // survives. It is wrapped because a failure to write an audit row must not convert a
      // well-formed `invalid_grant` into an unattributed 500 — the client's answer is the same
      // either way, and losing the signal is a smaller harm than answering the wrong error to
      // every refused refresh. The loss is itself reported, because an audit surface that can go
      // quiet without saying so is worse than one that is absent.
      try {
        await judgeRefusedRotation(db, String(refreshToken));
      } catch (judgeErr) {
        logger.error(
          { err: judgeErr instanceof Error ? judgeErr.message : String(judgeErr) },
          "token: could not record or act on a refused rotation — a replay may have gone unrecorded",
        );
      }
      return oauthError("invalid_grant");
    }

    // A chain minted during a fail-open login, or by a binary that predates the owner column,
    // carries none. Re-resolve here so the successor — the row that will actually be presented
    // next — carries the owner, rather than waiting out the chain's remaining lifetime.
    //
    // THIS HEALS A TRANSIENT FAILURE ONLY, and the distinction is the whole difference between a
    // blip and a deployment that never had the mechanism. `resolvePrincipal` opens with
    // `requireEnv`, so an unset `INTERNAL_RESOLVE_URL` (or a secret the API disagrees with) throws
    // here exactly as it threw at the ACS — deterministically, every time. Such a deployment mints
    // ownerless chains forever, the admission gate below is skipped for every one of them, and
    // `standing_service::apply`'s hook matches nothing. It is not distinguishable at runtime from
    // a healthy one; what reports it is the Rust-side warning when a terminal transition ends zero
    // chains.
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

    const pair = await issueTokenPair(db, rotated.claims, rotated.clientId, {
      // Handed back UNCHANGED — both of them. The deadline is what makes the bound absolute; the
      // identity is what lets a later replay of ANY token in this chain reach the live one.
      id: rotated.chainId,
      expiresAt: rotated.chainExpiresAt,
      profileId,
    });

    // No successor chain means the admission predicate inside the INSERT declined — the principal's
    // standing has reached a terminal. Refused HERE rather than only at the API gate the access
    // token would later meet. `denied` and `requested` principals pass that predicate on purpose;
    // it asks whether admission has ENDED, not whether it was ever granted.
    if (!pair.refresh_token) {
      // Withdraw the rotation mark before answering. The guard above already revoked and marked the
      // presented token, and the admission predicate declined only afterwards — so the row is left
      // claiming a rotation that produced no successor. A de-provisioned user whose client retries
      // once would then be reported as a replay, naming them in the operator's view under a hostile
      // count: the same false theft report the administrative revokers are held away from, arrived
      // at from the other side. Best-effort — this answer is settled, and failing to tidy the mark
      // must not turn it into a 500.
      try {
        await unmarkRotation(db, rotated.tokenId);
      } catch (unmarkErr) {
        logger.error(
          { err: unmarkErr instanceof Error ? unmarkErr.message : String(unmarkErr) },
          "token: could not withdraw the rotation mark from a refused rotation — a later retry may be reported as a replay",
        );
      }
      return oauthError("invalid_grant");
    }

    return oauthJson(pair, 200);
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
