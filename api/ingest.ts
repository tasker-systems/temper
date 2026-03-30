import type { AuthClaims } from "../packages/temper-cloud/src/auth.js";
import type { IngestMetadata } from "../packages/temper-cloud/src/ingest.js";

export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  if (req.method !== "POST") {
    return new Response(JSON.stringify({ error: "Method not allowed" }), {
      status: 405,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Dynamic imports to avoid ESM/CJS conflict in Vercel's hybrid runtime
  const { verifyToken, getJwksVerifier, getIssuer } = await import("../packages/temper-cloud/src/auth.js");
  const { getDb } = await import("../packages/temper-cloud/src/db.js");
  const { getProfileId, findByContentHash, insertResource } = await import("../packages/temper-cloud/src/ingest.js");
  const { processIngest } = await import("./workflows/process-ingest.js");
  const { createHash } = await import("node:crypto");

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
  const metadataRaw = formData.get("metadata") as string | null;
  const content = formData.get("content") as string | null;

  if (!metadataRaw) {
    return new Response(JSON.stringify({ error: "metadata is required" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }
  if (!content) {
    return new Response(JSON.stringify({ error: "content is required" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }

  let metadata: IngestMetadata;
  try {
    metadata = JSON.parse(metadataRaw) as IngestMetadata;
  } catch {
    return new Response(JSON.stringify({ error: "metadata must be valid JSON" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
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

  // Compute SHA-256 content hash
  const contentHash = createHash("sha256").update(content).digest("hex");

  // Check idempotency — return existing resource if content hash matches
  const existing = await findByContentHash(db, contentHash, profileId);
  if (existing) {
    return new Response(JSON.stringify(existing), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }

  // Insert new resource record
  const resource = await insertResource(db, metadata, contentHash, profileId);

  // Trigger async workflow: chunk → embed → store
  await processIngest(resource.id, content);

  return new Response(JSON.stringify(resource), {
    status: 202,
    headers: { "Content-Type": "application/json" },
  });
}
