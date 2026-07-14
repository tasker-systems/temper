import createClient, { type Client } from "openapi-fetch";

import { createAuthedFetch, type FetchLike } from "./auth-fetch.js";
import type { Credentials } from "./credentials.js";
import type { paths } from "./generated/schema.js";

export interface TemperClientOptions {
  /** The instance origin — e.g. `https://temperkb.io`. No trailing path. */
  baseUrl: string;
  credentials: Credentials;
  /** The fetch to wrap — compose here to keep a caller-specific retry. Default: global `fetch`. */
  fetch?: FetchLike;
}

/**
 * A fully typed client over every path in the contract.
 *
 * There are deliberately NO per-endpoint methods here. The gem hand-writes `Resources`,
 * `Contexts`, `CognitiveMaps` because Ruby has no type inference — someone must write
 * `create(title:, context:)` by hand. TypeScript infers: `createClient<paths>` types every path,
 * its params, and its responses PER STATUS, straight off the generated schema. A hand-written
 * `resources.create()` would be a second, worse spelling of something already correct — and a
 * place for the two to drift.
 *
 * HTTP errors are the contract's too — but ONLY those. `openapi-fetch` does not throw for a
 * status: it returns `{ data, error, response }` with `error` typed from the spec's own error
 * responses per status, so a 404 or a 422 is a value, not an exception, and no hand-written error
 * hierarchy (the gem's `errors.rb`, which exists because Faraday raises) belongs alongside it.
 *
 * What DOES throw is everything the contract never described. The authed fetch is the injected
 * fetch, and openapi-fetch rethrows whatever its fetch throws — there is no `onError` middleware
 * catching it. So a bad `clientSecret` throws `TokenMintError` before a request is ever sent, and
 * an unreachable host throws a network `TypeError`. Both are the client's own plumbing failing,
 * which is exactly the thing that has no per-status entry in the spec. Wrap the call in a `try`;
 * `if (error)` alone will not see them.
 */
export function createTemperClient(opts: TemperClientOptions): Client<paths> {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    fetch: createAuthedFetch({
      credentials: opts.credentials,
      fetch: opts.fetch,
    }),
  });
}
