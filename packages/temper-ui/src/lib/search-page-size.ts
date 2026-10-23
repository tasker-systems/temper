/**
 * The palette's page size: the `limit` the UI-server proxy sends (`vault-search.ts`). The palette
 * does not read it: whether more matched is each search arm's own `truncated` (D6), because a full
 * page is not the answer's word that more matched.
 */
export const SEARCH_PAGE_SIZE = 10;
