/**
 * Two strategies behind one interface. Precedence is the CALLER's explicit choice, never discovered
 * from the environment — that is how the steward's schedules went Connect-first while its MCP
 * connection went M2M-first, so on the Auth0-fronted instance the schedules' REST calls silently
 * failed while MCP worked.
 *
 * This is `Temper::Credentials` (clients/temper-rb/lib/temper/credentials.rb) transliterated. That
 * Ruby module was itself ported FROM the steward's hand-rolled mint, and fixed two bugs in the
 * process; this brings the fixed version home so the two first-party clients cannot drift again.
 * Both of the gem's divergences are reproduced here, one of them in JavaScript's own idiom:
 *
 *   - `refresh()` — KEPT. Refresh-ahead-of-expiry alone is insufficient: the steward resolves a
 *     token once per tick, so a tick outliving its cached token takes a 401 that nothing recovers.
 *     Temper's own AS mints 900-second tokens by default, which makes that ordinary rather than
 *     exotic. Re-mint ON 401.
 *   - The mutex — REPLACED, not dropped. The hazard the gem's mutex guards is CONCURRENCY, not OS
 *     threads, and `Promise.all` supplies plenty of it on a single thread: the steward fans N maps
 *     out over one token, and a token that dies mid-tick 401s all N at once. Each would then call
 *     `refresh()` — N concurrent mints, N billed tokens, last-writer-wins on the cache. The JS
 *     equivalent of the mutex is memoizing the in-flight mint promise: concurrent callers await the
 *     SAME mint, and the memo clears when it settles (success or failure) so a failed mint never
 *     poisons the next attempt.
 */

/** `expiresAt` is ABSOLUTE (ms since epoch). A duration cannot survive being cached — and eve's connection auth wants exactly this shape. */
export interface TokenResult {
  token: string;
  expiresAt: number;
}

export interface Credentials {
  /**
   * Whether `refresh()` can actually mint. A caller recovering from a 401 must ASK before it
   * re-mints: a strategy holding a token it did not mint (BearerToken) can only throw, and a
   * `refresh()`-throws-on-401 path replaces the server's real 401 — the response a human is trying
   * to read — with "BearerToken cannot refresh". Capability, not `instanceof`: the steward composes
   * its Vercel Connect strategy as a plain object, which no `instanceof` check would ever see.
   */
  readonly canRefresh: boolean;
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
  readonly canRefresh = false;
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

/** The `/oauth/token` success body both issuers promise (tests/contracts/m2m-token-request.json). */
interface TokenResponseBody {
  access_token: string;
  expires_in: number;
}

/**
 * Parse, don't validate. A cast would let a malformed 200 through: `access_token: undefined` puts
 * `Bearer undefined` on every subsequent request, and `expires_in: undefined` makes `expiresAt`
 * NaN — and since every NaN comparison is false, the cache would then be judged expired forever and
 * re-mint on EVERY call. The gem gets this for free from `body.fetch('access_token')`, which raises.
 */
function isTokenResponseBody(body: unknown): body is TokenResponseBody {
  if (typeof body !== "object" || body === null) {
    return false;
  }
  const fields = body as Record<string, unknown>;
  return (
    typeof fields.access_token === "string" &&
    fields.access_token !== "" &&
    typeof fields.expires_in === "number" &&
    Number.isFinite(fields.expires_in)
  );
}

/** A `client_credentials` machine principal. Works against BOTH issuers a temper instance can be fronted by. */
export class ClientCredentials implements Credentials {
  /** Re-mint this far AHEAD of expiry rather than racing it. */
  static readonly SKEW_MS = 60_000;

  readonly canRefresh = true;

  readonly #tokenUrl: string;
  readonly #clientId: string;
  readonly #clientSecret: string;
  readonly #audience: string | undefined;
  readonly #now: () => number;
  #cached: TokenResult | undefined;
  /** The mutex, in JS idiom — see the class comment. Non-undefined exactly while a mint is in flight. */
  #inFlight: Promise<TokenResult> | undefined;

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

  /**
   * Mint unconditionally, discarding any cached token. The on-401 path — see the class comment.
   *
   * Concurrent callers COALESCE onto one mint. N parallel fetches sharing a token that dies mid-tick
   * all take a 401 and all land here at once; without this they would buy N tokens to answer one
   * expiry.
   */
  async refresh(): Promise<TokenResult> {
    this.#inFlight ??= this.#mint().finally(() => {
      // Clear on failure too: a mint that failed must not wedge every later attempt onto its
      // already-rejected promise.
      this.#inFlight = undefined;
    });
    return this.#inFlight;
  }

  async #mint(): Promise<TokenResult> {
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

    let body: unknown;
    try {
      body = await res.json();
    } catch {
      throw new TokenMintError(`token mint returned a non-JSON body (${res.status})`, res.status);
    }
    if (!isTokenResponseBody(body)) {
      throw new TokenMintError(
        `token mint returned a ${res.status} without a usable access_token/expires_in`,
        res.status,
      );
    }

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
