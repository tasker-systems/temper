/**
 * Auth0 loopback redirect-uri proxy.
 *
 * Auth0's RFC 8252 loopback port-wildcard matching (Native app → `http://localhost`
 * matches any port) does not work on all tenants — even with OIDC Conformant enabled.
 * Without it, MCP clients like opencode that use `http://127.0.0.1:<port>/callback` as
 * their redirect_uri get a 403 "Callback URL mismatch" from Auth0's `/authorize`.
 *
 * This module solves it by proxying the authorize and token endpoints through
 * temperkb.io. The flow:
 *
 * 1. MCP client → `GET /oauth/authorize` (our proxy)
 *    - `redirect_uri=http://127.0.0.1:<port>/callback`
 *    - We stash the original `redirect_uri` + `state` in an AES-256-GCM encrypted
 *      token, then redirect to Auth0's `/authorize` with `redirect_uri` rewritten
 *      to our relay (`https://<base>/api/auth/mcp-callback`) and `state` set to
 *      the encrypted token.
 *
 * 2. Auth0 → `GET /api/auth/mcp-callback` (our relay)
 *    - Auth0 redirects here with `?code=...&state=<encrypted_token>`.
 *    - We decrypt the token, extract the original `redirect_uri` + `state`,
 *      and redirect the browser to `http://127.0.0.1:<port>/callback?code=...&state=...`.
 *
 * 3. MCP client → `POST /oauth/token` (our proxy)
 *    - The client exchanges the code using `redirect_uri=http://127.0.0.1:<port>/callback`
 *      (what it originally sent). We rewrite it to the relay URL before forwarding
 *      to Auth0, so Auth0 sees the `redirect_uri` it expects.
 *
 * PKCE passes through untouched — the client's `code_challenge` goes to Auth0 in
 * step 1, and the client's `code_verifier` goes to Auth0 in step 3. We never see
 * or touch either.
 *
 * The stashed state is encrypted (not just signed) with AES-256-GCM, so the
 * redirect_uri and client state are not visible to the browser or any
 * intermediary. The encryption key is derived by scrypt (memory-hard KDF) from
 * `MCP_PROXY_SECRET` and nothing else, so the confidentiality of a stashed
 * redirect_uri rests on a value only the operator holds. A deployment without
 * that secret serves no request that needs the key — the loopback branch of
 * `proxyAuthorize` and the relay. The non-loopback pass-through and `proxyToken`
 * stash nothing, need no key, and stay served.
 *
 * This proxy serves instances fronted by an external IdP (Auth0, Okta); instances
 * running the Temper AS use `endpoints.ts` directly. All three entry points read
 * that mode through one name, `isTemperAsMode` in `env.js`, so they cannot drift
 * from each other: `api/oauth/authorize.ts` and `api/oauth/token.ts` dispatch to
 * the AS, and the relay below — which has no AS counterpart to dispatch to —
 * reports itself absent.
 */

import { createCipheriv, createDecipheriv, randomBytes, scryptSync } from "node:crypto";
import { isTemperAsMode } from "./env.js";

/** The relay URL that Auth0 redirects to after login. */
const RELAY_PATH = "/api/auth/mcp-callback";

/** How long a stashed state token is valid (10 minutes, matching PENDING_FLOW_TTL_SECONDS). */
const STATE_TTL_MS = 10 * 60 * 1000;

/** AES-256-GCM key length in bytes. */
const KEY_LEN = 32;

/** AES-GCM IV length (96 bits, per NIST SP 800-38D). */
const IV_LEN = 12;

/** scrypt salt — static, not secret. Its purpose is domain separation. */
const SCRYPT_SALT = "temper-mcp-proxy-v1";

/**
 * Minimum accepted length of `MCP_PROXY_SECRET`, in characters.
 *
 * This is a truncation guard, not an entropy measure: it counts characters, and
 * scrypt stretches whatever it is handed, so no threshold here can make a weak
 * secret strong. What it catches is a value that was cut short in transit — a
 * clipped paste, a shell-quoted fragment — which is the failure an operator hits
 * and cannot see. Generating the value the way both playbooks say
 * (`openssl rand -base64 48`, 64 characters) clears it with room to spare.
 */
const MIN_PROXY_SECRET_LEN = 32;

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`Missing required environment variable: ${name}`);
  return v;
}

/**
 * The derived key, held for the life of the process so the KDF cost is paid once
 * at cold start rather than per request. Only a success is cached: a deployment
 * with no secret re-reads one environment variable per request and pays nothing.
 */
let cachedKey: Buffer | undefined;

/**
 * The AES-256 key for the proxy's state tokens, derived from `MCP_PROXY_SECRET`
 * by scrypt — or `undefined` when that secret is absent or shorter than
 * `MIN_PROXY_SECRET_LEN`, which are the only two configurations that yield
 * `undefined`.
 *
 * scrypt is intentionally computationally expensive, satisfying the password-hash
 * effort requirement that CodeQL checks for.
 *
 * The key has exactly one input and no substitute for it, which is why this
 * returns an option rather than a key. Reporting the absence is the caller's
 * job, because only the caller knows the right shape for it: a handler turns it
 * into a 503 naming the variable, which is the one form of the answer an
 * operator can act on. Throwing here would surface as an unattributed platform
 * 500 from `proxyAuthorize`, and `handleMcpCallback` would report a
 * configuration fault as a 400 blaming the client's state token.
 */
export function stateKey(): Buffer | undefined {
  if (cachedKey) return cachedKey;

  const secret = process.env.MCP_PROXY_SECRET;
  if (!secret || secret.length < MIN_PROXY_SECRET_LEN) return undefined;

  cachedKey = scryptSync(secret, SCRYPT_SALT, KEY_LEN);
  return cachedKey;
}

/** The stashed data we carry through Auth0's `state` parameter. */
interface StashedState {
  /** Original redirect_uri from the MCP client (e.g. `http://127.0.0.1:19876/mcp/oauth/callback`). */
  r: string;
  /** Original state from the MCP client (CSRF token). */
  s: string;
  /** Expiry (epoch ms). */
  e: number;
}

function b64url(input: Buffer): string {
  return input.toString("base64url");
}

function fromB64url(input: string): Buffer {
  return Buffer.from(input, "base64url");
}

/**
 * Encrypts `{r, s, e}` into an AES-256-GCM sealed token:
 * `base64url(iv).base64url(ciphertext).base64url(authTag)`.
 *
 * Both confidentiality and integrity are provided by GCM — no separate HMAC needed.
 */
export function encodeStashedState(key: Buffer, redirectUri: string, oauthState: string): string {
  const payload: StashedState = {
    r: redirectUri,
    s: oauthState,
    e: Date.now() + STATE_TTL_MS,
  };

  const iv = randomBytes(IV_LEN);
  const cipher = createCipheriv("aes-256-gcm", key, iv);
  const plaintext = Buffer.from(JSON.stringify(payload), "utf8");
  const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const authTag = cipher.getAuthTag();

  return `${b64url(iv)}.${b64url(ciphertext)}.${b64url(authTag)}`;
}

/** Decrypts and validates the token, returning the stashed state. Throws on failure. */
function decodeStashedState(key: Buffer, token: string): StashedState {
  const parts = token.split(".");
  if (parts.length !== 3) throw new Error("malformed state token");

  const iv = fromB64url(parts[0]);
  const ciphertext = fromB64url(parts[1]);
  const authTag = fromB64url(parts[2]);

  if (iv.length !== IV_LEN) throw new Error("malformed state token");

  const decipher = createDecipheriv("aes-256-gcm", key, iv);
  decipher.setAuthTag(authTag);

  let plaintext: Buffer;
  try {
    plaintext = Buffer.concat([decipher.update(ciphertext), decipher.final()]);
  } catch {
    throw new Error("invalid or tampered state token");
  }

  const payload = JSON.parse(plaintext.toString("utf8")) as StashedState;
  if (Date.now() > payload.e) throw new Error("expired state token");
  return payload;
}

function redirect(location: string): Response {
  return new Response(null, { status: 302, headers: { location } });
}

function badRequest(reason: string): Response {
  return new Response(reason, { status: 400 });
}

/**
 * The refusal a request meets when this instance has no `MCP_PROXY_SECRET` to
 * derive its state-token key from. 503 rather than 500: the request is
 * well-formed and the fault is the instance's, so the status says so and the
 * body names the one variable that resolves it. Following the AS config errors,
 * it prescribes the relation and prints no value.
 */
function proxySecretUnavailable(): Response {
  return new Response(
    `MCP_PROXY_SECRET is unset or shorter than ${MIN_PROXY_SECRET_LEN} characters. ` +
      "The Auth0 loopback proxy derives its state-token key from that secret alone, " +
      "and serves no request that needs the key without it.",
    { status: 503, headers: { "cache-control": "no-store" } },
  );
}

function isValidUrl(value: string): boolean {
  try {
    new URL(value);
    return true;
  } catch {
    return false;
  }
}

/** True if a redirect_uri is a loopback address that needs proxying. */
function isLoopbackRedirect(uri: string): boolean {
  try {
    const u = new URL(uri);
    return (
      u.protocol === "http:" &&
      (u.hostname === "localhost" || u.hostname === "127.0.0.1" || u.hostname === "[::1]")
    );
  } catch {
    return false;
  }
}

/**
 * `GET /oauth/authorize` — Auth0 proxy entry point.
 *
 * If the client's `redirect_uri` is a loopback URL, rewrites it to the relay and
 * stashes the original in an encrypted `state` token. If it's already a non-loopback
 * URL (e.g. `https://claude.ai/api/mcp/auth_callback`), passes through to Auth0
 * unchanged — those work fine with Auth0's exact-match allowlist.
 */
export async function proxyAuthorize(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const params = url.searchParams;

  const redirectUri = params.get("redirect_uri");
  const state = params.get("state");

  if (!redirectUri || !isValidUrl(redirectUri))
    return badRequest("redirect_uri is required and must be a valid URL");
  if (!state) return badRequest("state is required");

  const auth0Domain = requireEnv("AUTH_ISSUER").replace(/\/+$/, "");
  const relayUri = `${requireEnv("MCP_BASE_URL")}${RELAY_PATH}`;

  if (isLoopbackRedirect(redirectUri)) {
    // Only this branch stashes anything, so only this branch needs the key. A
    // non-loopback redirect_uri below is forwarded to Auth0 untouched and stays
    // served, which keeps a missing secret from taking down browser clients that
    // never used the proxy's crypto.
    const key = stateKey();
    if (!key) return proxySecretUnavailable();

    // Stash original redirect_uri + state, rewrite redirect_uri to our relay.
    const stashed = encodeStashedState(key, redirectUri, state);

    const auth0Params = new URLSearchParams(params);
    auth0Params.set("redirect_uri", relayUri);
    auth0Params.set("state", stashed);

    return redirect(`${auth0Domain}/authorize?${auth0Params.toString()}`);
  }

  // Non-loopback redirect (Claude Desktop's HTTPS callback, etc.) — pass through.
  return redirect(`${auth0Domain}/authorize?${params.toString()}`);
}

/**
 * `GET /api/auth/mcp-callback` — relay that Auth0 redirects to after login.
 *
 * Decrypts the state token, extracts the original redirect_uri + state,
 * and redirects the browser to the MCP client's local callback server with the
 * authorization code.
 */
export function handleMcpCallback(req: Request): Response {
  // The relay is part of the Auth0 proxy, and the proxy serves Auth0-fronted
  // instances. In AS mode `endpoints.ts` owns the authorization code flow end to
  // end and redirects no client here, so the endpoint is absent rather than
  // merely unused — the treatment `handleJwks` gives `/oauth/jwks` on an
  // instance fronted by an external IdP. Its two sibling entry points read the
  // same `isTemperAsMode` and dispatch to the AS; this one has no AS counterpart
  // to dispatch to, so absence is the whole of its answer.
  if (isTemperAsMode()) {
    return new Response("Not Found", { status: 404 });
  }

  const url = new URL(req.url);

  const error = url.searchParams.get("error");
  if (error) {
    const description = url.searchParams.get("error_description") ?? "unknown error";
    return new Response(`Authentication failed: ${error} — ${description}`, { status: 400 });
  }

  const code = url.searchParams.get("code");
  const stashed = url.searchParams.get("state");
  if (!code || !stashed) return badRequest("Missing code or state parameter");

  // Before the decode, not inside its catch: that catch reports a bad token as
  // the client's fault, and a missing secret is the instance's.
  const key = stateKey();
  if (!key) return proxySecretUnavailable();

  let original: StashedState;
  try {
    original = decodeStashedState(key, stashed);
  } catch {
    return badRequest("Invalid or expired state token");
  }

  // Defense-in-depth: even if the encryption key were compromised, the relay must
  // never redirect to a non-loopback URL. The authorize proxy only stashes
  // loopback redirect_uris, so a valid token should always carry one — but this
  // check ensures a forged or tampered token cannot turn the relay into an open
  // redirect.
  if (!isLoopbackRedirect(original.r)) {
    return badRequest("Invalid redirect_uri in state token");
  }

  const target = new URL(original.r);
  target.searchParams.set("code", code);
  target.searchParams.set("state", original.s);

  return redirect(target.toString());
}

/**
 * `POST /oauth/token` — Auth0 proxy token endpoint.
 *
 * For `authorization_code` grants, rewrites `redirect_uri` from the client's
 * loopback URL to the relay URL so Auth0 sees the `redirect_uri` it received
 * during `/authorize`. For `refresh_token` and `client_credentials` grants,
 * passes through to Auth0 unchanged (neither carries a `redirect_uri`).
 */
export async function proxyToken(req: Request): Promise<Response> {
  const auth0Domain = requireEnv("AUTH_ISSUER").replace(/\/+$/, "");
  const relayUri = `${requireEnv("MCP_BASE_URL")}${RELAY_PATH}`;

  const body = await req.text();
  const params = new URLSearchParams(body);
  const grantType = params.get("grant_type");

  if (grantType === "authorization_code") {
    const redirectUri = params.get("redirect_uri");
    if (redirectUri && isLoopbackRedirect(redirectUri)) {
      params.set("redirect_uri", relayUri);
    }
  }

  const tokenResp = await fetch(`${auth0Domain}/oauth/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: params.toString(),
  });

  const respBody = await tokenResp.text();
  return new Response(respBody, {
    status: tokenResp.status,
    headers: {
      "content-type": tokenResp.headers.get("content-type") ?? "application/json",
      "cache-control": "no-store",
    },
  });
}
