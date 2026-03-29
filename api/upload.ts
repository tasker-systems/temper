import { put } from "@vercel/blob";
import { verifyToken, getJwksVerifier, getIssuer, type AuthClaims } from "../packages/temper-cloud/src/auth.js";
import { buildBlobPathname, buildInsertBlobFileQuery } from "../packages/temper-cloud/src/upload.js";
import { getDb } from "../packages/temper-cloud/src/db.js";
import { processUpload } from "./workflows/process-upload.js";

export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  if (req.method !== "POST") {
    return new Response(JSON.stringify({ error: "Method not allowed" }), {
      status: 405,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Authenticate
  const authHeader = req.headers.get("authorization");
  if (!authHeader?.startsWith("Bearer ")) {
    return new Response(
      JSON.stringify({ error: { code: "UNAUTHORIZED", message: "Missing Authorization header" } }),
      { status: 401, headers: { "Content-Type": "application/json" } }
    );
  }

  let claims: AuthClaims;
  try {
    claims = await verifyToken(authHeader.slice(7), getJwksVerifier(), getIssuer());
  } catch {
    return new Response(
      JSON.stringify({ error: { code: "UNAUTHORIZED", message: "Invalid token" } }),
      { status: 401, headers: { "Content-Type": "application/json" } }
    );
  }

  // Parse multipart form data
  const formData = await req.formData();
  const file = formData.get("file") as File | null;
  const resourceId = formData.get("resource_id") as string | null;

  if (!file) {
    return new Response(JSON.stringify({ error: "file is required" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }
  if (!resourceId) {
    return new Response(JSON.stringify({ error: "resource_id is required" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }

  const db = getDb();

  // Get profile_id and verify resource access
  const profileRows = await db`
    SELECT id FROM kb_profiles WHERE auth_provider_sub = ${claims.sub} LIMIT 1
  `;
  if (profileRows.length === 0) {
    return new Response(
      JSON.stringify({ error: "Profile not found" }),
      { status: 404, headers: { "Content-Type": "application/json" } }
    );
  }
  const profileId = profileRows[0].id as string;

  const visibleResources = await db`
    SELECT resource_id FROM resources_visible_to(${profileId}::uuid)
    WHERE resource_id = ${resourceId}::uuid
  `;
  if (visibleResources.length === 0) {
    return new Response(
      JSON.stringify({ error: "Resource not found or not accessible" }),
      { status: 404, headers: { "Content-Type": "application/json" } }
    );
  }

  // Store in Vercel Blob
  const pathname = buildBlobPathname(profileId, resourceId, file.name);
  const blob = await put(pathname, file, { access: "private" });

  // Insert blob_files record
  const { sql, params } = buildInsertBlobFileQuery({
    profileId,
    resourceId,
    blobUrl: blob.url,
    pathname: blob.pathname,
    contentType: file.type || null,
    fileSizeBytes: file.size,
  });
  const insertResult = await db(sql, params);
  const blobFileId = insertResult[0].id as string;

  // Trigger the processing workflow. The "use workflow" directive makes this
  // a durable invocation — Vercel executes the steps asynchronously.
  // If the workflow fails to start, we still return 202 since the file is stored.
  try {
    await processUpload(blobFileId, blob.url, resourceId);
  } catch (err) {
    console.error("Failed to trigger processing workflow:", err);
  }

  return new Response(
    JSON.stringify({
      blob_file_id: blobFileId,
      status: "pending",
    }),
    { status: 202, headers: { "Content-Type": "application/json" } }
  );
}
