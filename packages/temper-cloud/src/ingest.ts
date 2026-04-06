import { randomUUID } from "node:crypto";
import { uuidv7 } from "uuidv7";
import type { AuthClaims } from "./auth.js";
import type { NeonClient } from "./db.js";
import { DEVICE_ID_CLOUD, insertEventAndAudit } from "./events.js";
import { canonicalJsonHash } from "./hash.js";

// ---------------------------------------------------------------------------
// Public interfaces
// ---------------------------------------------------------------------------

export interface IngestMetadata {
  title: string;
  kb_context_id: string; // UUID or resolved from name
  kb_doc_type_id: string; // UUID or resolved from name
  origin_uri: string;
  slug?: string;
  tags?: string[];
  metadata?: Record<string, unknown>;
  context_name?: string; // Resolve to UUID server-side
  doc_type_name?: string; // Resolve to UUID server-side
}

export interface ResourceRecord {
  id: string;
  kb_context_id: string;
  kb_doc_type_id: string;
  origin_uri: string;
  title: string;
  slug: string | null;
  originator_profile_id: string;
  owner_profile_id: string;
  is_active: boolean;
  created: string;
  updated: string;
}

// ---------------------------------------------------------------------------
// Profile lookup
// ---------------------------------------------------------------------------

/**
 * Look up the profile_id from auth claims.
 * Joins through kb_profile_auth_links using claims.sub as the external identity.
 * Returns null if no matching profile is found.
 */
export async function getProfileId(db: NeonClient, claims: AuthClaims): Promise<string | null> {
  const rows = await db`
    SELECT p.id
    FROM kb_profiles p
    JOIN kb_profile_auth_links pal ON pal.profile_id = p.id
    WHERE pal.auth_provider_user_id = ${claims.sub}
      AND p.is_active = true
    LIMIT 1
  `;
  if (rows.length === 0) return null;
  return rows[0].id as string;
}

// ---------------------------------------------------------------------------
// Content hash deduplication
// ---------------------------------------------------------------------------

/**
 * Check for an existing active resource with the same body hash owned by
 * the same profile. Returns the matching ResourceRecord or null.
 */
export async function findByBodyHash(
  db: NeonClient,
  bodyHash: string,
  profileId: string,
): Promise<ResourceRecord | null> {
  const rows = await db`
    SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title, r.slug,
           r.originator_profile_id, r.owner_profile_id, r.is_active, r.created, r.updated
    FROM kb_resources r
    JOIN kb_resource_manifests m ON m.resource_id = r.id
    WHERE m.body_hash = ${bodyHash}
      AND r.owner_profile_id = ${profileId}::uuid
      AND r.is_active = true
    LIMIT 1
  `;
  if (rows.length === 0) return null;
  return rows[0] as unknown as ResourceRecord;
}

// ---------------------------------------------------------------------------
// Context resolution
// ---------------------------------------------------------------------------

/**
 * Resolve a context name to its UUID for the given profile owner.
 * Auto-creates the context if it does not already exist under this profile.
 */
export async function resolveContextId(
  db: NeonClient,
  name: string,
  profileId: string,
): Promise<string> {
  // Look up existing context owned by this profile
  const existing = await db`
    SELECT id FROM kb_contexts
    WHERE name = ${name}
      AND kb_owner_table = 'kb_profiles'
      AND kb_owner_id = ${profileId}::uuid
    LIMIT 1
  `;
  if (existing.length > 0) {
    return existing[0].id as string;
  }

  // Auto-create
  const newId = randomUUID();
  await db`
    INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
    VALUES (${newId}::uuid, ${name}, 'kb_profiles', ${profileId}::uuid)
  `;

  const eventId = uuidv7();
  await db`
    INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, event_type, payload, created)
    VALUES (${eventId}::uuid, ${profileId}::uuid, ${"vercel-cloud"}, ${newId}::uuid, 'context_created', '{}', now())
  `;

  return newId;
}

// ---------------------------------------------------------------------------
// Doc type resolution
// ---------------------------------------------------------------------------

/**
 * Resolve a doc_type name to its UUID.
 * kb_doc_types are system-level — returns null if not found (no auto-create).
 */
export async function resolveDocTypeId(db: NeonClient, name: string): Promise<string | null> {
  const rows = await db`
    SELECT id FROM kb_doc_types WHERE name = ${name} LIMIT 1
  `;
  if (rows.length === 0) return null;
  return rows[0].id as string;
}

// ---------------------------------------------------------------------------
// Resource insert
// ---------------------------------------------------------------------------

/**
 * Insert a new resource record.
 * If context_name is provided it takes precedence over kb_context_id (resolved
 * to UUID via resolveContextId). Same for doc_type_name / kb_doc_type_id.
 * Returns the newly created ResourceRecord.
 */
export async function insertResource(
  db: NeonClient,
  meta: IngestMetadata,
  contentHash: string,
  profileId: string,
): Promise<ResourceRecord> {
  // Resolve context
  let contextId = meta.kb_context_id;
  if (meta.context_name) {
    contextId = await resolveContextId(db, meta.context_name, profileId);
  }

  // Resolve doc type
  let docTypeId = meta.kb_doc_type_id;
  if (meta.doc_type_name) {
    const resolved = await resolveDocTypeId(db, meta.doc_type_name);
    if (!resolved) {
      throw new Error(`Unknown doc_type_name: ${meta.doc_type_name}`);
    }
    docTypeId = resolved;
  }

  const newId = randomUUID();
  const slug = meta.slug ?? null;

  const rows = await db`
    INSERT INTO kb_resources (
      id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
      originator_profile_id, owner_profile_id, is_active, created, updated
    ) VALUES (
      ${newId}::uuid,
      ${contextId}::uuid,
      ${docTypeId}::uuid,
      ${meta.origin_uri},
      ${meta.title},
      ${slug},
      ${profileId}::uuid,
      ${profileId}::uuid,
      true,
      now(),
      now()
    )
    RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
              originator_profile_id, owner_profile_id, is_active, created, updated
  `;

  // Create manifest entry for body hash tracking
  await db`
    INSERT INTO kb_resource_manifests (resource_id, body_hash, updated)
    VALUES (${newId}::uuid, ${contentHash}, now())
  `;

  const emptyHash = canonicalJsonHash({});

  await insertEventAndAudit(db, {
    profileId,
    deviceId: DEVICE_ID_CLOUD,
    contextId: contextId,
    resourceId: newId,
    eventType: "resource_created",
    action: "create",
    bodyHash: contentHash,
    managedHash: emptyHash,
    openHash: emptyHash,
  });

  return rows[0] as unknown as ResourceRecord;
}

// ---------------------------------------------------------------------------
// Resource hash update
// ---------------------------------------------------------------------------

/**
 * Update the body_hash on an existing resource's manifest entry (used when
 * re-ingesting updated content for the same resource).
 */
export async function updateResourceHash(
  db: NeonClient,
  resourceId: string,
  bodyHash: string,
  profileId: string,
  contextId: string,
): Promise<void> {
  await db`
    INSERT INTO kb_resource_manifests (resource_id, body_hash, updated)
    VALUES (${resourceId}::uuid, ${bodyHash}, now())
    ON CONFLICT (resource_id)
    DO UPDATE SET body_hash = ${bodyHash}, updated = now()
  `;
  await db`
    UPDATE kb_resources SET updated = now() WHERE id = ${resourceId}::uuid
  `;

  const emptyHash = canonicalJsonHash({});

  await insertEventAndAudit(db, {
    profileId,
    deviceId: DEVICE_ID_CLOUD,
    contextId,
    resourceId,
    eventType: "body_updated",
    action: "update_body",
    bodyHash,
    managedHash: emptyHash,
    openHash: emptyHash,
  });
}
