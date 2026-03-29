export function buildBlobPathname(profileId: string, resourceId: string, filename: string): string {
  // Prevent path traversal
  const safeFilename = filename.replace(/\.\./g, "");
  return `${profileId}/${resourceId}/${safeFilename}`;
}

export interface InsertBlobFileParams {
  profileId: string;
  resourceId: string | null;
  blobUrl: string;
  pathname: string;
  contentType: string | null;
  fileSizeBytes: number | null;
}

export function buildInsertBlobFileQuery(params: InsertBlobFileParams): {
  sql: string;
  params: (string | number | null)[];
} {
  return {
    sql: `INSERT INTO blob_files (profile_id, resource_id, blob_url, pathname, content_type, file_size_bytes, status)
VALUES ($1, $2, $3, $4, $5, $6, 'pending')
RETURNING id, status, created_at`,
    params: [
      params.profileId,
      params.resourceId,
      params.blobUrl,
      params.pathname,
      params.contentType,
      params.fileSizeBytes,
    ],
  };
}

export interface UploadResult {
  blobFileId: string;
  status: string;
  createdAt: string;
}
