import type { ResourceRow } from './generated/resource';
import type { ManagedMeta } from './generated/managed_meta';
import type { JsonValue } from './generated/serde_json/JsonValue';

/**
 * What `GET /api/resources/{id}` actually returns: the row plus both meta tiers.
 *
 * Hand-composed, deliberately. The Rust `ResourceDetail`
 * (`crates/temper-workflow/src/types/resource.rs:187`) uses `#[serde(flatten)]`,
 * which ts-rs cannot generate. Both halves here ARE generated — only the join is
 * by hand — so this does not re-declare a wire shape.
 *
 * Both tiers are `#[serde(skip_serializing_if = "Option::is_none")]`, so they are
 * **absent** from the JSON rather than null — hence optional, not `| null` alone.
 *
 * The page previously typed this response as `ResourceRow`, which silently
 * discarded both tiers: a type assertion on a fetch result gets no excess-property
 * check. That is why the vault has never rendered frontmatter.
 */
export type ResourceDetail = ResourceRow & {
	managed_meta?: ManagedMeta | null;
	open_meta?: JsonValue | null;
};
