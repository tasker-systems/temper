import type { AuthClaims } from "../../packages/temper-cloud/src/auth.js";

export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  if (req.method !== "PUT") {
    return new Response(JSON.stringify({ error: "Method not allowed" }), {
      status: 405,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Dynamic imports to avoid ESM/CJS conflict in Vercel's hybrid runtime
  const { verifyToken, getJwksVerifier, getIssuer } = await import("../../packages/temper-cloud/src/auth.js");
  const { getDb } = await import("../../packages/temper-cloud/src/db.js");
  const { getProfileId, processContentInline, updateResourceHash } = await import("../../packages/temper-cloud/src/ingest.js");
  const { createHash } = await import("node:crypto");

  // Extract resource ID from URL path (/api/ingest/<id>)
  const url = new URL(req.url);
  const pathParts = url.pathname.split("/").filter(Boolean);
  const resourceId = pathParts[pathParts.length - 1];

  if (!resourceId) {
    return new Response(JSON.stringify({ error: "Resource ID is required" }), {
      status: 400,
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

  const db = getDb();

  // Get profile_id from auth claims
  const profileId = await getProfileId(db, claims);
  if (!profileId) {
    return new Response(
      JSON.stringify({ error: "Profile not found" }),
      { status: 404, headers: { "Content-Type": "application/json" } }
    );
  }

  // Verify resource exists and caller owns it
  const resourceRows = await db`
    SELECT id, kb_context_id, kb_doc_type_id, uri, title, slug, content_hash,
           mimetype, originator_profile_id, owner_profile_id, is_active, created, updated
    FROM resources
    WHERE id = ${resourceId}::uuid
      AND (owner_profile_id = ${profileId}::uuid OR originator_profile_id = ${profileId}::uuid)
      AND is_active = true
    LIMIT 1
  `;
  if (resourceRows.length === 0) {
    return new Response(
      JSON.stringify({ error: "Resource not found or not accessible" }),
      { status: 404, headers: { "Content-Type": "application/json" } }
    );
  }
  const resource = resourceRows[0];

  // Parse content from form data
  const formData = await req.formData();
  const content = formData.get("content") as string | null;

  if (!content) {
    return new Response(JSON.stringify({ error: "content is required" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Compute SHA-256 content hash
  const contentHash = createHash("sha256").update(content).digest("hex");

  // If content is unchanged, return early
  if (resource.content_hash === contentHash) {
    return new Response(JSON.stringify(resource), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Process content inline: chunk → embed → store
  await processContentInline(db, resourceId, content);

  // Update the resource's content hash
  await updateResourceHash(db, resourceId, contentHash);

  // Return updated resource
  const updatedRows = await db`
    SELECT id, kb_context_id, kb_doc_type_id, uri, title, slug, content_hash,
           mimetype, originator_profile_id, owner_profile_id, is_active, created, updated
    FROM resources
    WHERE id = ${resourceId}::uuid
    LIMIT 1
  `;

  return new Response(JSON.stringify(updatedRows[0]), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}
