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
 * The wire contract, generated from the repo-root `openapi.json` — itself a product of the Axum
 * router. NEVER hand-edited: `cargo make openapi` regenerates it, and `openapi-ts-drift` (in
 * `cargo make check`, and in CI) fails if the committed copy has fallen behind the contract.
 *
 * Public from day one on purpose. temper-ui types its API surface from ts-rs today, and 103 of
 * those 133 types are ALSO OpenAPI schemas — two TypeScript renderings of the same Rust structs.
 * The exit is temper-ui importing these instead; an export it cannot reach would not be a
 * direction, only an intention.
 */
export type { components, operations, paths } from "./generated/schema.js";

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
