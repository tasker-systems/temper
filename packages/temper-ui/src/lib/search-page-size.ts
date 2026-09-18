/**
 * The palette's page size — the ONE constant behind both the request the UI-server proxy
 * makes (`limit`, in `vault-search.ts`) and the bound the palette states under a full page
 * (D6, "Showing the first 10"). Client-safe on purpose: the component reads it too, so it
 * cannot live beside the token-carrying server modules.
 */
export const SEARCH_PAGE_SIZE = 10;
