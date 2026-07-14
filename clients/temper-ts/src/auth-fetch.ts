import type { Credentials } from "./credentials.js";

/**
 * `openapi-fetch`'s `ClientOptions["fetch"]` — a `Request` in, a `Response` out. NOT the
 * `(url, init)` shape of global `fetch`. Matching it exactly is what lets an authed fetch drop
 * straight into `createClient` with no adapter.
 *
 * It is also the seam a caller wraps to keep its own retry/backoff underneath the auth layer. The
 * steward has such a retry (`agent/lib/temper-auth.ts`'s `fetchWithRetry`, for Vercel cold starts)
 * and does NOT yet compose it here — that migration is its own task, and it is not a rename: the
 * steward's is `(url, init)`-shaped and must be reshaped to this signature first. Until then the
 * steward keeps its own hand-rolled 401 re-mint, which is one of the two things temper-ts exists to
 * end.
 */
export type FetchLike = (input: Request) => Promise<Response>;

/**
 * The attribution marker, sent on every request. It names the KIND of surface, never the client's
 * language — the gem sends this same `sdk` (clients/temper-rb/lib/temper/connection.rb), and so
 * will temper-py. There is deliberately no override: the server trusts `{sdk, cli}` and attributes
 * a `cli` write to the `<handle>@cli` emitter, so a knob here would be a knob for writing a lie
 * into the event ledger. There is no TypeScript CLI to tell the truth with.
 */
const SURFACE = "sdk";

export interface AuthedFetchOptions {
  credentials: Credentials;
  /** The fetch to wrap. Default: global `fetch`. */
  fetch?: FetchLike;
}

function authorize(request: Request, token: string): Request {
  const headers = new Headers(request.headers);
  headers.set("authorization", `Bearer ${token}`);
  headers.set("x-temper-surface", SURFACE);
  return new Request(request, { headers });
}

/**
 * `fetch` against temper, authenticated, with a single re-mint on 401.
 *
 * The 401 branch is not belt-and-braces. A caller resolves ONE token and then holds it across N
 * parallel requests, so a token that dies mid-flight takes them all down and nothing recovers;
 * refresh-ahead-of-expiry cannot help, because the token was live when it was checked. Temper's
 * own AS mints 900-second tokens by default, which makes outliving one ordinary rather than exotic.
 *
 * ONE dead token buys ONE replacement, and that is enforced HERE, not in `Credentials`.
 * `refresh()` mints UNCONDITIONALLY; its in-flight memo coalesces only the callers that overlap
 * the mint itself — which staggered 401s, the ordinary fan-out shape, do not. So this remembers
 * the token it actually SENT and re-mints only if that same dead token is still the cached one.
 * A sibling request that already re-minted leaves a fresh token behind; this rides it.
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
  const inner: FetchLike = opts.fetch ?? ((input) => fetch(input));

  return async (input: Request): Promise<Response> => {
    // Clone BEFORE the send. Sending consumes the body, and a retry built from the consumed
    // request would carry an empty one — silently writing nothing, on the exact path the 401
    // recovery exists to save.
    const pristine = input.clone();

    const used = await credentials.token();
    const response = await inner(authorize(input, used));
    if (response.status !== 401 || !credentials.canRefresh) {
      return response;
    }

    // Nothing on this path ever reads the abandoned 401's body, and undici holds the socket open
    // until someone does.
    void response.body?.cancel();

    const current = await credentials.tokenResult();
    const token = current.token !== used ? current.token : (await credentials.refresh()).token;
    return inner(authorize(pristine, token));
  };
}
