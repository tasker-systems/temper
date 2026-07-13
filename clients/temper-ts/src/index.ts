/**
 * The TypeScript client for the Temper knowledge base API. Sibling of `temper-rb`
 * (and the coming `temper-py`); the three are pinned to the same wire contracts.
 */

export {
  BearerToken,
  ClientCredentials,
  type ClientCredentialsOptions,
  type Credentials,
  TokenMintError,
  type TokenResult,
} from "./credentials.js";

/** Identifies the client in server logs, and — being a value — keeps the bundler from tree-shaking this package out of a consumer's build. */
export const TEMPER_TS_VERSION = "0.0.0";
