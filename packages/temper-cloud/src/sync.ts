import { z } from "zod";
import type { NeonClient } from "./db.js";

// ---------------------------------------------------------------------------
// Zod schemas — request body validation
// ---------------------------------------------------------------------------

const SyncManifestEntrySchema = z.object({
  uri: z.string().startsWith("kb://"),
  local_hash: z.string().min(1),
  remote_hash: z.string().min(1),
});

const SyncContextEntriesSchema = z.object({
  name: z.string().min(1),
  entries: z.array(SyncManifestEntrySchema),
});

export const SyncStatusBodySchema = z.object({
  contexts: z.array(SyncContextEntriesSchema).min(1),
});

export const SyncCompleteBodySchema = z.object({
  client_id: z.string().min(1),
  merged_resources: z
    .array(
      z.object({
        resource_id: z.string().uuid(),
        content_hash: z.string().min(1),
      }),
    )
    .default([]),
});

export type SyncStatusBody = z.infer<typeof SyncStatusBodySchema>;
export type SyncCompleteBody = z.infer<typeof SyncCompleteBodySchema>;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

export interface SyncPushItem {
  uri: string;
  resource_id: string | null;
}

export interface SyncPullItem {
  uri: string;
  resource_id: string;
  content_hash: string;
}

export interface SyncConflictItem {
  uri: string;
  resource_id: string;
  server_hash: string;
}

export interface SyncRemovedItem {
  uri: string;
  resource_id: string;
}

export interface SyncDiffResult {
  to_push: SyncPushItem[];
  to_pull: SyncPullItem[];
  conflicts: SyncConflictItem[];
  removed: SyncRemovedItem[];
}

export interface SyncCompleteResult {
  last_sync_at: string;
  updated_count: number;
}

// ---------------------------------------------------------------------------
// Row categorization (pure function, no DB)
// ---------------------------------------------------------------------------

interface DiffRow {
  resource_id: string | null;
  kb_uri: string;
  content_hash: string;
  updated: string | null;
  diff_type: string;
}

/**
 * Categorize raw rows from sync_diff_for_device() into typed buckets.
 * Pure function — no DB access, fully unit-testable.
 */
export function categorizeDiffRows(rows: DiffRow[]): SyncDiffResult {
  const to_push: SyncPushItem[] = [];
  const to_pull: SyncPullItem[] = [];
  const conflicts: SyncConflictItem[] = [];
  const removed: SyncRemovedItem[] = [];

  for (const row of rows) {
    switch (row.diff_type) {
      case "to_push":
        to_push.push({ uri: row.kb_uri, resource_id: row.resource_id });
        break;
      case "to_pull":
        to_pull.push({
          uri: row.kb_uri,
          resource_id: row.resource_id as string,
          content_hash: row.content_hash,
        });
        break;
      case "conflict":
        conflicts.push({
          uri: row.kb_uri,
          resource_id: row.resource_id as string,
          server_hash: row.content_hash,
        });
        break;
      case "removed":
        removed.push({
          uri: row.kb_uri,
          resource_id: row.resource_id as string,
        });
        break;
    }
  }

  return { to_push, to_pull, conflicts, removed };
}

// ---------------------------------------------------------------------------
// Business logic (DB functions)
// ---------------------------------------------------------------------------

/**
 * Compute the sync diff by calling sync_diff_for_device() and categorizing results.
 */
export async function computeSyncDiff(
  db: NeonClient,
  profileId: string,
  body: SyncStatusBody,
): Promise<SyncDiffResult> {
  const contextNames: string[] = [];
  const manifestEntries: Array<{
    uri: string;
    local_hash: string;
    remote_hash: string;
  }> = [];

  for (const ctx of body.contexts) {
    contextNames.push(ctx.name);
    for (const entry of ctx.entries) {
      manifestEntries.push(entry);
    }
  }

  const rows = await db`
    SELECT resource_id, kb_uri, content_hash, updated, diff_type
    FROM sync_diff_for_device(
      ${profileId}::uuid,
      ${contextNames}::text[],
      ${JSON.stringify(manifestEntries)}::jsonb
    )
  `;

  return categorizeDiffRows(rows as unknown as DiffRow[]);
}

/**
 * Finalize a sync round: update content hashes and upsert device sync state.
 */
export async function completeSyncRound(
  db: NeonClient,
  profileId: string,
  body: SyncCompleteBody,
): Promise<SyncCompleteResult> {
  let updatedCount = 0;

  for (const mr of body.merged_resources) {
    await db`
      UPDATE kb_resources
      SET content_hash = ${mr.content_hash}, updated = now()
      WHERE id = ${mr.resource_id}::uuid
    `;
    updatedCount++;
  }

  await db`
    INSERT INTO kb_device_sync_state (id, profile_id, client_id, last_sync_at)
    VALUES (gen_random_uuid(), ${profileId}::uuid, ${body.client_id}, now())
    ON CONFLICT (profile_id, client_id)
    DO UPDATE SET last_sync_at = now()
  `;

  return {
    last_sync_at: new Date().toISOString(),
    updated_count: updatedCount,
  };
}
