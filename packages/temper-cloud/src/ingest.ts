import { randomUUID } from "node:crypto";
import type { AuthClaims } from "./auth.js";
import type { NeonClient } from "./db.js";
import type { ChunkRow } from "./processing/index.js";
import {
  buildStoreChunksQuery,
  buildVersionBumpQuery,
  chunkText,
  embedTexts,
} from "./processing/index.js";

// ---------------------------------------------------------------------------
// Public interfaces
// ---------------------------------------------------------------------------

export interface IngestMetadata {
  title: string;
  kb_context_id: string; // UUID or resolved from name
  kb_doc_type_id: string; // UUID or resolved from name
  uri: string;
  slug?: string;
  mimetype?: string;
  tags?: string[];
  metadata?: Record<string, unknown>;
  context_name?: string; // Resolve to UUID server-side
  doc_type_name?: string; // Resolve to UUID server-side
}

export interface ResourceRecord {
  id: string;
  kb_context_id: string;
  kb_doc_type_id: string;
  uri: string;
  title: string;
  slug: string | null;
  content_hash: string | null;
  mimetype: string | null;
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
 * Check for an existing active resource with the same content hash owned by
 * the same profile. Returns the matching ResourceRecord or null.
 */
export async function findByContentHash(
  db: NeonClient,
  contentHash: string,
  profileId: string,
): Promise<ResourceRecord | null> {
  const rows = await db`
    SELECT id, kb_context_id, kb_doc_type_id, uri, title, slug, content_hash,
           mimetype, originator_profile_id, owner_profile_id, is_active, created, updated
    FROM resources
    WHERE content_hash = ${contentHash}
      AND owner_profile_id = ${profileId}::uuid
      AND is_active = true
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
  const mimetype = meta.mimetype ?? null;

  const rows = await db`
    INSERT INTO resources (
      id, kb_context_id, kb_doc_type_id, uri, title, slug, content_hash,
      mimetype, originator_profile_id, owner_profile_id, is_active, created, updated
    ) VALUES (
      ${newId}::uuid,
      ${contextId}::uuid,
      ${docTypeId}::uuid,
      ${meta.uri},
      ${meta.title},
      ${slug},
      ${contentHash},
      ${mimetype},
      ${profileId}::uuid,
      ${profileId}::uuid,
      true,
      now(),
      now()
    )
    RETURNING id, kb_context_id, kb_doc_type_id, uri, title, slug, content_hash,
              mimetype, originator_profile_id, owner_profile_id, is_active, created, updated
  `;

  return rows[0] as unknown as ResourceRecord;
}

// ---------------------------------------------------------------------------
// Resource hash update
// ---------------------------------------------------------------------------

/**
 * Update the content_hash on an existing resource (used when re-ingesting
 * updated content for the same resource).
 */
export async function updateResourceHash(
  db: NeonClient,
  resourceId: string,
  contentHash: string,
): Promise<void> {
  await db`
    UPDATE resources
    SET content_hash = ${contentHash}, updated = now()
    WHERE id = ${resourceId}::uuid
  `;
}

// ---------------------------------------------------------------------------
// Inline content processing pipeline
// ---------------------------------------------------------------------------

/**
 * Full inline processing pipeline:
 *   1. Chunk the markdown text via chunkText()
 *   2. Embed all chunks via embedTexts()
 *   3. Determine next version number from existing chunks
 *   4. Mark old chunks as non-current (version bump)
 *   5. Insert new chunk rows
 *
 * Returns the number of chunks stored.
 */
export async function processContentInline(
  db: NeonClient,
  resourceId: string,
  markdown: string,
): Promise<number> {
  const chunks = chunkText(markdown);
  if (chunks.length === 0) return 0;

  // Embed all chunk contents in a single batch
  const texts = chunks.map((c) => c.content);
  const embeddings = await embedTexts(texts);

  // Determine the next version number
  const versionRows = await db`
    SELECT COALESCE(MAX(version), 0) AS max_version
    FROM kb_chunks
    WHERE resource_id = ${resourceId}::uuid
  `;
  const nextVersion = (Number(versionRows[0].max_version) + 1) as number;

  // Build ChunkRow records with UUIDs
  const chunkRows: ChunkRow[] = chunks.map((chunk, i) => ({
    id: randomUUID(),
    resource_id: resourceId,
    chunk_index: chunk.chunk_index,
    version: nextVersion,
    header_path: chunk.header_path,
    content: chunk.content,
    content_hash: chunk.content_hash,
    embedding: embeddings[i],
  }));

  // Version bump: mark previous current chunks as non-current
  const versionBump = buildVersionBumpQuery(resourceId, nextVersion);
  if (versionBump.sql) {
    await db.query(versionBump.sql, versionBump.params);
  }

  // Store new chunks
  const storeQuery = buildStoreChunksQuery(chunkRows);
  if (storeQuery.sql) {
    await db.query(storeQuery.sql, storeQuery.params);
  }

  return chunkRows.length;
}
