import { SEARCH_PAGE_SIZE } from '$lib/search-page-size';
import type { SearchResponse } from '$lib/types/generated/search';
import { apiPost } from './api';

/**
 * The asking door — the palette's single question, answered over the real search read.
 *
 * The request carries `{ query, limit }` and **nothing else**: no `arms` selector, no embedding.
 * That is the whole point of this task being UI-only — the wire shape the server answers today
 * already computes and returns **both** arms (exact FTS, wide vector), each with its own
 * disposition, and the palette's job is to display one of them at a time. A request shaped like
 * this is byte-identical to what any current client sends, so the display-side selection never
 * reaches the wire.
 *
 * The session token stays server-side, exactly like the graph page's composition read — the
 * browser's fetch hits this module, never the upstream door.
 *
 * @see CommandPalette.svelte — the arm the reader sees is chosen there, from the answer returned here
 */
export const runSearch = (
	token: string,
	q: string,
	limit = SEARCH_PAGE_SIZE,
): Promise<SearchResponse> => apiPost<SearchResponse>('/api/search', token, { query: q, limit });
