// TODO(Task 6): Implement Vercel Workflow for content processing.
// This stub exists so api/content-ingest.ts compiles before Task 6 is built.

export async function processContentIngest(
  _resourceId: string,
  _content: string,
  _replace: boolean,
  _profileId: string,
): Promise<void> {
  "use workflow";
  throw new Error("process-content-ingest workflow not yet implemented");
}
