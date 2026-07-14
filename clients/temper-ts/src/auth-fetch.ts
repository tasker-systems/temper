import type { Credentials } from "./credentials.js";

/**
 * `openapi-fetch`'s `ClientOptions["fetch"]` — a `Request` in, a `Response` out. NOT the
 * `(url, init)` shape of global `fetch`. Matching it exactly is what lets an authed fetch
 * drop straight into `createClient` with no adapter, and what lets a caller compose one
 * (the steward wraps its 5xx cold-start retry this way).
 */
export type FetchLike = (input: Request) => Promise<Response>;

/**
 * The attribution marker, sent on every request. It names the KIND of surface, never the
 * client's language — the gem sends the same `sdk` (clients/temper-rb/lib/temper/connection.rb).
 * Provenance, never authorization: the server's `{sdk, cli}` allowlist degrades anything else
 * to `web`.
 */
export type Surface = "sdk" | "cli";

export interface AuthedFetchOptions {
  credentials: Credentials;
  /** Default `sdk`. */
  surface?: Surface;
  /** The fetch to wrap. Default: global `fetch`. */
  fetch?: FetchLike;
}

/**
 * `fetch` against temper, authenticated, with a single re-mint on 401.
 *
 * The 401 branch is not belt-and-braces. A caller resolves ONE token and then holds it across N
 * parallel requests, so a token that dies mid-flight takes them all down and nothing recovers;
 * refresh-ahead-of-expiry cannot help, because the token was live when it was checked. Temper's
 * own AS mints 900-second tokens by default, which makes outliving one ordinary rather than
 * exotic. `ClientCredentials.refresh()` coalesces concurrent callers onto ONE mint, so N
 * simultaneous 401s buy one token, not N.
 *
 * Exactly ONE retry: a 401 that survives a fresh token is a real authorization failure — a
 * revoked credential, missing reach — and retrying it would only bury the error.
 *
 * A strategy that cannot mint gets its 401 back UNTOUCHED. `BearerToken.refresh()` throws, and
 * throwing here would replace temper's real answer — the response body a human is trying to
 * read — with a message about the client's own plumbing.
 */
export function createAuthedFetch(opts: AuthedFetchOptions): FetchLike {
  const { credentials } = opts;
  const surface: Surface = opts.surface ?? "sdk";
  const inner: FetchLike = opts.fetch ?? ((input) => fetch(input));

  const authorize = async (request: Request, token: string): Promise<Request> => {
    const headers = new Headers(request.headers);
    headers.set("authorization", `Bearer ${token}`);
    headers.set("x-temper-surface", surface);
    return new Request(request, { headers });
  };

  return async (input: Request): Promise<Response> => {
    // Clone BEFORE the send. Sending consumes the body, and a retry built from the consumed
    // request would carry an empty one — silently writing nothing, on the exact path the 401
    // recovery exists to save.
    const pristine = input.clone();

    const response = await inner(await authorize(input, await credentials.token()));
    if (response.status !== 401 || !credentials.canRefresh) {
      return response;
    }

    const refreshed = await credentials.refresh();
    return inner(await authorize(pristine, refreshed.token));
  };
}
