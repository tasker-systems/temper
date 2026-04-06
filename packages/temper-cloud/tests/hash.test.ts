import { describe, expect, it } from "vitest";
import { canonicalJsonHash } from "../src/hash.js";

describe("canonicalJsonHash", () => {
  it("hashes empty object", () => {
    expect(canonicalJsonHash({})).toBe(
      "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
    );
  });

  it("sorts keys lexicographically", () => {
    const result = canonicalJsonHash({ b: 2, a: 1 });
    expect(result).toBe(canonicalJsonHash({ a: 1, b: 2 }));
  });

  it("sorts nested object keys recursively", () => {
    const result = canonicalJsonHash({
      z: { b: 2, a: 1 },
      a: "first",
    });
    const reversed = canonicalJsonHash({
      a: "first",
      z: { a: 1, b: 2 },
    });
    expect(result).toBe(reversed);
  });

  it("preserves array order", () => {
    const a = canonicalJsonHash({ items: [3, 1, 2] });
    const b = canonicalJsonHash({ items: [1, 2, 3] });
    expect(a).not.toBe(b);
  });

  it("handles null, boolean, and numeric values", () => {
    const result = canonicalJsonHash({
      flag: true,
      count: 42,
      empty: null,
    });
    expect(result).toMatch(/^sha256:[0-9a-f]{64}$/);
  });

  it("matches Rust hash_json_value() for shared fixture", () => {
    // This fixture hash was verified against the Rust implementation.
    // Rust: cargo nextest run -p temper-api hash_json_shared_fixture --no-capture
    const result = canonicalJsonHash({
      "temper-type": "task",
      "temper-stage": "in-progress",
      "temper-seq": 42,
      title: "Test task",
    });
    expect(result).toBe(
      "sha256:d39e1380d3b0ce969fe93f1df8b2da5d1caabf90b33e2e30f01d661f2c3c4895",
    );
  });
});
