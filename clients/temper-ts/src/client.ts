import createClient, { type Client } from "openapi-fetch";

import { createAuthedFetch, type FetchLike, type Surface } from "./auth-fetch.js";
import type { Credentials } from "./credentials.js";
import type { paths } from "./generated/schema.js";

export interface TemperClientOptions {
  /** The instance origin — e.g. `https://temperkb.io`. No trailing path. */
  baseUrl: string;
  credentials: Credentials;
  /** Default `sdk`. */
  surface?: Surface;
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
 * Errors are the contract's too. `openapi-fetch` does not throw; it returns
 * `{ data, error, response }` with `error` typed from the spec's own error responses. The gem
 * needs `errors.rb` because Faraday raises. Inventing a hierarchy alongside a typed one would be
 * inventing drift.
 */
export function createTemperClient(opts: TemperClientOptions): Client<paths> {
  return createClient<paths>({
    baseUrl: opts.baseUrl,
    fetch: createAuthedFetch({
      credentials: opts.credentials,
      surface: opts.surface,
      fetch: opts.fetch,
    }),
  });
}
