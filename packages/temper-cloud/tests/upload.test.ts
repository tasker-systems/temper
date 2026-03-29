import { describe, expect, it } from "vitest";
import { buildBlobPathname, buildInsertBlobFileQuery } from "../src/upload.js";

describe("buildBlobPathname", () => {
  it("constructs pathname from profile, resource, and filename", () => {
    const pathname = buildBlobPathname("profile-123", "resource-456", "document.md");
    expect(pathname).toBe("profile-123/resource-456/document.md");
  });

  it("sanitizes filename by removing path traversal", () => {
    const pathname = buildBlobPathname("p", "r", "../../../etc/passwd");
    expect(pathname).not.toContain("..");
  });
});

describe("buildInsertBlobFileQuery", () => {
  it("generates INSERT SQL with all fields", () => {
    const { sql, params } = buildInsertBlobFileQuery({
      profileId: "profile-123",
      resourceId: "resource-456",
      blobUrl: "https://blob.vercel-storage.com/abc",
      pathname: "profile-123/resource-456/doc.md",
      contentType: "text/markdown",
      fileSizeBytes: 1024,
    });

    expect(sql).toContain("INSERT INTO blob_files");
    expect(params).toContain("profile-123");
    expect(params).toContain("resource-456");
    expect(params).toContain("https://blob.vercel-storage.com/abc");
    expect(params).toContain("text/markdown");
    expect(params).toContain(1024);
  });
});
