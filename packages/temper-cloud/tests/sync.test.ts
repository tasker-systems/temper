import { describe, expect, it } from "vitest";
import { categorizeDiffRows, SyncCompleteBodySchema, SyncStatusBodySchema } from "../src/sync.js";

describe("SyncStatusBodySchema", () => {
  it("accepts valid request body", () => {
    const body = {
      contexts: [
        {
          name: "temper",
          entries: [
            { uri: "kb://temper/task/abc", local_hash: "sha256:aaa", remote_hash: "sha256:bbb" },
          ],
        },
      ],
    };
    const result = SyncStatusBodySchema.safeParse(body);
    expect(result.success).toBe(true);
  });

  it("accepts body with multiple contexts", () => {
    const body = {
      contexts: [
        { name: "temper", entries: [] },
        {
          name: "tasker",
          entries: [{ uri: "kb://tasker/note/x", local_hash: "a", remote_hash: "b" }],
        },
      ],
    };
    const result = SyncStatusBodySchema.safeParse(body);
    expect(result.success).toBe(true);
  });

  it("rejects missing contexts", () => {
    const result = SyncStatusBodySchema.safeParse({});
    expect(result.success).toBe(false);
  });

  it("rejects empty contexts array", () => {
    const result = SyncStatusBodySchema.safeParse({ contexts: [] });
    expect(result.success).toBe(false);
  });

  it("rejects empty context name", () => {
    const body = { contexts: [{ name: "", entries: [] }] };
    const result = SyncStatusBodySchema.safeParse(body);
    expect(result.success).toBe(false);
  });

  it("rejects entries with missing fields", () => {
    const body = {
      contexts: [{ name: "temper", entries: [{ uri: "kb://a/b/c" }] }],
    };
    const result = SyncStatusBodySchema.safeParse(body);
    expect(result.success).toBe(false);
  });

  it("rejects uri that does not start with kb://", () => {
    const body = {
      contexts: [
        {
          name: "temper",
          entries: [{ uri: "https://example.com", local_hash: "a", remote_hash: "b" }],
        },
      ],
    };
    const result = SyncStatusBodySchema.safeParse(body);
    expect(result.success).toBe(false);
  });
});

describe("SyncCompleteBodySchema", () => {
  it("accepts valid request body", () => {
    const body = {
      client_id: "device-abc",
      merged_resources: [
        { resource_id: "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11", content_hash: "sha256:abc" },
      ],
    };
    const result = SyncCompleteBodySchema.safeParse(body);
    expect(result.success).toBe(true);
  });

  it("accepts body with empty merged_resources", () => {
    const body = { client_id: "device-abc", merged_resources: [] };
    const result = SyncCompleteBodySchema.safeParse(body);
    expect(result.success).toBe(true);
  });

  it("defaults merged_resources when omitted", () => {
    const body = { client_id: "device-abc" };
    const result = SyncCompleteBodySchema.safeParse(body);
    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.data.merged_resources).toEqual([]);
    }
  });

  it("rejects missing client_id", () => {
    const result = SyncCompleteBodySchema.safeParse({ merged_resources: [] });
    expect(result.success).toBe(false);
  });

  it("rejects invalid UUID in resource_id", () => {
    const body = {
      client_id: "device-abc",
      merged_resources: [{ resource_id: "not-a-uuid", content_hash: "abc" }],
    };
    const result = SyncCompleteBodySchema.safeParse(body);
    expect(result.success).toBe(false);
  });
});

describe("categorizeDiffRows", () => {
  it("categorizes rows by diff_type", () => {
    const rows = [
      {
        resource_id: "id-1",
        kb_uri: "kb://a/b/1",
        content_hash: "h1",
        updated: null,
        diff_type: "to_push",
      },
      {
        resource_id: "id-2",
        kb_uri: "kb://a/b/2",
        content_hash: "h2",
        updated: null,
        diff_type: "to_pull",
      },
      {
        resource_id: "id-3",
        kb_uri: "kb://a/b/3",
        content_hash: "h3",
        updated: null,
        diff_type: "conflict",
      },
      {
        resource_id: "id-4",
        kb_uri: "kb://a/b/4",
        content_hash: "h4",
        updated: null,
        diff_type: "removed",
      },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.to_push).toHaveLength(1);
    expect(result.to_pull).toHaveLength(1);
    expect(result.conflicts).toHaveLength(1);
    expect(result.removed).toHaveLength(1);
  });

  it("maps fields correctly for to_push", () => {
    const rows = [
      {
        resource_id: "id-1",
        kb_uri: "kb://temper/task/uuid1",
        content_hash: "h1",
        updated: null,
        diff_type: "to_push",
      },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.to_push[0]).toEqual({ uri: "kb://temper/task/uuid1", resource_id: "id-1" });
  });

  it("maps fields correctly for to_pull", () => {
    const rows = [
      {
        resource_id: "id-2",
        kb_uri: "kb://temper/note/uuid2",
        content_hash: "sha256:abc",
        updated: "2026-01-01",
        diff_type: "to_pull",
      },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.to_pull[0]).toEqual({
      uri: "kb://temper/note/uuid2",
      resource_id: "id-2",
      content_hash: "sha256:abc",
    });
  });

  it("maps fields correctly for conflict", () => {
    const rows = [
      {
        resource_id: "id-3",
        kb_uri: "kb://a/b/3",
        content_hash: "server-hash",
        updated: null,
        diff_type: "conflict",
      },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.conflicts[0]).toEqual({
      uri: "kb://a/b/3",
      resource_id: "id-3",
      server_hash: "server-hash",
    });
  });

  it("maps fields correctly for removed", () => {
    const rows = [
      {
        resource_id: "id-4",
        kb_uri: "kb://a/b/4",
        content_hash: "h4",
        updated: null,
        diff_type: "removed",
      },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.removed[0]).toEqual({ uri: "kb://a/b/4", resource_id: "id-4" });
  });

  it("returns empty arrays for no rows", () => {
    const result = categorizeDiffRows([]);
    expect(result.to_push).toHaveLength(0);
    expect(result.to_pull).toHaveLength(0);
    expect(result.conflicts).toHaveLength(0);
    expect(result.removed).toHaveLength(0);
  });

  it("handles null resource_id for to_push (new local resource)", () => {
    const rows = [
      {
        resource_id: null,
        kb_uri: "kb://a/b/new",
        content_hash: "h1",
        updated: null,
        diff_type: "to_push",
      },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.to_push[0].resource_id).toBeNull();
  });

  it("ignores rows with unknown diff_type", () => {
    const rows = [
      {
        resource_id: "id-1",
        kb_uri: "kb://a/b/1",
        content_hash: "h1",
        updated: null,
        diff_type: "unknown_type",
      },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.to_push).toHaveLength(0);
    expect(result.to_pull).toHaveLength(0);
    expect(result.conflicts).toHaveLength(0);
    expect(result.removed).toHaveLength(0);
  });
});
