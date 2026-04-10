// Re-export all generated types from temper-core
export * from './generated/access.ts';
export * from './generated/context.ts';
export * from './generated/event.ts';
export * from './generated/invitation.ts';
export * from './generated/profile.ts';
export * from './generated/resource.ts';
export * from './generated/search.ts';
export * from './generated/team.ts';
export * from './generated/transfer.ts';
export * from './generated/serde_json/JsonValue.ts';

import type { Profile } from './generated/profile.ts';
import type { Entitlements } from './generated/access.ts';

/**
 * Wire shape of `GET /api/profile` — Profile fields flattened with an
 * `entitlements` sibling. Mirrors the Rust handler's `ProfileWithEntitlements`
 * which uses `#[serde(flatten)]` on the embedded Profile.
 *
 * Defined as a TypeScript intersection rather than a generated type because
 * ts-rs doesn't represent `serde(flatten)` cleanly, and the Rust shape is
 * load-bearing for `temper-client` which deserializes the same response as
 * a bare `Profile` (extra fields ignored).
 */
export type ProfileWithEntitlements = Profile & { entitlements: Entitlements };
