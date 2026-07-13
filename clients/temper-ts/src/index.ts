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

/**
 * The deploy gate's evidence. The steward logs this on every dispatch tick
 * (`agent/schedules/steward.ts`), which is how we confirm from PRODUCTION logs that the deployed
 * bundle actually resolved the `file:` dependency on this package — a `file:` dep that silently
 * failed to bundle is exactly the failure the log line exists to catch.
 *
 * Hand-synced with `package.json`'s `version`; reconcile the two (or generate this from it) at first
 * publish, when the string starts meaning something to a consumer.
 */
export const TEMPER_TS_VERSION = "0.0.0";
