/**
 * Two strategies behind one interface. Precedence is the CALLER's explicit choice, never discovered
 * from the environment — that is how the steward's schedules went Connect-first while its MCP
 * connection went M2M-first, so on the Auth0-fronted instance the schedules' REST calls silently
 * failed while MCP worked.
 *
 * This is `Temper::Credentials` (clients/temper-rb/lib/temper/credentials.rb) transliterated. That
 * Ruby module was itself ported FROM the steward's hand-rolled mint, and fixed two bugs in the
 * process; this brings the fixed version home so the two first-party clients cannot drift again.
 * Of the gem's two divergences, only ONE is reproduced here:
 *
 *   - `refresh()` — KEPT. Refresh-ahead-of-expiry alone is insufficient: the steward resolves a
 *     token once per tick, so a tick outliving its cached token takes a 401 that nothing recovers.
 *     Temper's own AS mints 900-second tokens by default, which makes that ordinary rather than
 *     exotic. Re-mint ON 401.
 *   - The mutex — DROPPED. The gem needs one because Puma is threaded. A serverless function is
 *     not, and a bare field is the honest shape for it.
 */

/** `expiresAt` is ABSOLUTE (ms since epoch). A duration cannot survive being cached — and eve's connection auth wants exactly this shape. */
export interface TokenResult {
  token: string;
  expiresAt: number;
}

export interface Credentials {
  token(): Promise<string>;
  tokenResult(): Promise<TokenResult>;
  refresh(): Promise<TokenResult>;
}

export class TokenMintError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "TokenMintError";
    this.status = status;
  }
}

/** A token the caller already holds — a request serving a signed-in human. No I/O, no refresh. */
export class BearerToken implements Credentials {
  readonly #token: string;

  constructor(token: string) {
    if (token === "") {
      throw new TypeError("token must be a non-empty string");
    }
    this.#token = token;
  }

  async token(): Promise<string> {
    return this.#token;
  }

  async tokenResult(): Promise<TokenResult> {
    // No expiry is knowable from a token handed to us. `0` would claim "already expired"; a caller
    // that needs refresh-ahead must use ClientCredentials.
    return { token: this.#token, expiresAt: Number.POSITIVE_INFINITY };
  }

  async refresh(): Promise<TokenResult> {
    throw new TokenMintError("BearerToken cannot refresh; mint a new token upstream", 401);
  }
}

export interface ClientCredentialsOptions {
  tokenUrl: string;
  clientId: string;
  clientSecret: string;
  /**
   * Auth0 REQUIRES it; temper's own AS ignores a request-supplied audience entirely and mints with
   * its server-side AS_AUDIENCE. Omit it for a temper-issued (`tmpr_`) credential — sending an
   * empty one would be a lie.
   */
  audience?: string;
  /** Injectable clock (ms since epoch) — tests drive expiry without sleeping. */
  now?: () => number;
}

/** A `client_credentials` machine principal. Works against BOTH issuers a temper instance can be fronted by. */
export class ClientCredentials implements Credentials {
  /** Re-mint this far AHEAD of expiry rather than racing it. */
  static readonly SKEW_MS = 60_000;

  readonly #tokenUrl: string;
  readonly #clientId: string;
  readonly #clientSecret: string;
  readonly #audience: string | undefined;
  readonly #now: () => number;
  #cached: TokenResult | undefined;

  constructor(opts: ClientCredentialsOptions) {
    this.#tokenUrl = requireNonEmpty(opts.tokenUrl, "tokenUrl");
    this.#clientId = requireNonEmpty(opts.clientId, "clientId");
    this.#clientSecret = requireNonEmpty(opts.clientSecret, "clientSecret");
    this.#audience = opts.audience === undefined ? undefined : requireNonEmpty(opts.audience, "audience");
    this.#now = opts.now ?? (() => Date.now());
  }

  async token(): Promise<string> {
    return (await this.tokenResult()).token;
  }

  async tokenResult(): Promise<TokenResult> {
    if (this.#cached !== undefined && this.#cached.expiresAt - ClientCredentials.SKEW_MS > this.#now()) {
      return this.#cached;
    }
    return this.refresh();
  }

  /** Mint unconditionally, discarding any cached token. The on-401 path — see the class comment. */
  async refresh(): Promise<TokenResult> {
    const res = await fetch(this.#tokenUrl, {
      method: "POST",
      // RFC 6749 §4 mandates form encoding. Auth0 tolerates JSON, which is why a JSON mint stayed
      // green for as long as Auth0 was the only issuer any client faced; temper's AS reads the body
      // with `req.formData()` and a JSON mint never reaches its grant branch.
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: this.#requestBody(),
    });

    if (!res.ok) {
      throw new TokenMintError(`token mint failed (${res.status}): ${await res.text()}`, res.status);
    }

    const body = (await res.json()) as { access_token: string; expires_in: number };
    // Absolute, not relative: a duration cannot survive being cached.
    this.#cached = {
      token: body.access_token,
      expiresAt: this.#now() + body.expires_in * 1000,
    };
    return this.#cached;
  }

  #requestBody(): URLSearchParams {
    const params = new URLSearchParams({
      grant_type: "client_credentials",
      client_id: this.#clientId,
      client_secret: this.#clientSecret,
    });
    if (this.#audience !== undefined) {
      params.set("audience", this.#audience);
    }
    return params;
  }
}

function requireNonEmpty(value: string, name: string): string {
  if (typeof value !== "string" || value === "") {
    throw new TypeError(`${name} must be a non-empty string`);
  }
  return value;
}
