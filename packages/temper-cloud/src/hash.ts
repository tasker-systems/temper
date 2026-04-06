import { createHash } from "node:crypto";

/**
 * Compute a `sha256:<hex>` hash of a JSON value with canonicalized key ordering.
 *
 * Algorithm matches Rust `hash_json_value()` in ingest_service.rs:
 * 1. Sort object keys recursively (lexicographic, depth-first)
 * 2. JSON.stringify with no spacing (compact form)
 * 3. SHA-256 the UTF-8 bytes
 * 4. Return "sha256:<hex>"
 */
export function canonicalJsonHash(value: Record<string, unknown>): string {
  const canonical = canonicalize(value);
  const serialized = JSON.stringify(canonical);
  const hash = createHash("sha256").update(serialized, "utf8").digest("hex");
  return `sha256:${hash}`;
}

function canonicalize(value: unknown): unknown {
  if (value === null || typeof value !== "object") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map(canonicalize);
  }
  const sorted: Record<string, unknown> = {};
  for (const key of Object.keys(value).sort()) {
    sorted[key] = canonicalize((value as Record<string, unknown>)[key]);
  }
  return sorted;
}
