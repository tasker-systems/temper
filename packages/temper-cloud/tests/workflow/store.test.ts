import { describe, expect, it } from "vitest";
import { buildStatusUpdateQuery } from "../../src/workflow/store.js";

describe("buildStatusUpdateQuery", () => {
  it("generates UPDATE SQL for kb_blob_files status", () => {
    const { sql, params } = buildStatusUpdateQuery("file-001", "processed", null);
    expect(sql).toContain("UPDATE kb_blob_files");
    expect(sql).toContain("status");
    expect(params).toContain("file-001");
    expect(params).toContain("processed");
  });

  it("includes error_message for failed status", () => {
    const { params } = buildStatusUpdateQuery("file-001", "failed", "ONNX load error");
    expect(params).toContain("ONNX load error");
  });
});
