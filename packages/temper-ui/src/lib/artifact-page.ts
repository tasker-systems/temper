/**
 * One bounded page of a resource's data artifacts, and the links that move between pages.
 *
 * `GET /api/resources/{id}/artifacts` with `limit` answers the bare array, in `(created, id)`
 * order (oldest first), and names the next page in a `Link: <target>; rel="next"` header that is
 * absent on the last page. The resource page asks for one such page and never the whole history.
 *
 * The page's position lives in the PAGE's URL, as the vault list's `offset` does, so a page is
 * an address the load reads server-side: a link, a reload and the browser's Back all mean the
 * same page. The keys are prefixed because the artifacts region is one region of several on the
 * resource page and the position is that region's alone.
 */

import type { ArtifactCursor, ArtifactView } from '$lib/types/generated/data_artifact';

/**
 * Rows the resource page asks for at once. Within the server ceiling `MAX_ARTIFACT_PAGE_ROWS`,
 * above which the read is refused, and the same size the CLI and MCP drain the list with
 * (`ARTIFACT_DRAIN_PAGE_ROWS`), since each row carries its content whole. The vault lists'
 * `DEFAULT_LIMIT` is the same number.
 */
export const ARTIFACT_PAGE_ROWS = 50;

/** The resource page's own query keys for the artifacts region's position. */
export const PAGE_AFTER_CREATED = 'artifacts_after_created';
export const PAGE_AFTER_ID = 'artifacts_after_id';

/** What the artifacts region renders: one page, and where the reader can go from it. */
export interface ArtifactRegion {
	artifacts: ArtifactView[];
	/** The page after this one, or `null` when this is the last. */
	laterHref: string | null;
	/** The first page, or `null` when this IS the first page. */
	earliestHref: string | null;
}

/**
 * Whether a link entry's parameters carry a `rel` whose relation types include `next`, read as
 * RFC 8288 allows: name and type case-insensitively, the value quoted or bare, and the value a
 * whitespace-separated list. Only the first `rel` counts.
 */
function namesNext(params: string): boolean {
	for (const param of params.split(';')) {
		const eq = param.indexOf('=');
		if (eq < 0 || param.slice(0, eq).trim().toLowerCase() !== 'rel') continue;
		let value = param.slice(eq + 1).trim();
		if (value.length >= 2 && value.startsWith('"') && value.endsWith('"')) {
			value = value.slice(1, -1);
		}
		return value.split(/\s+/).some((rel) => rel.toLowerCase() === 'next');
	}
	return false;
}

/**
 * The position a `Link` header's next entry continues after, or `null` when there is no header
 * or it names no next page.
 *
 * Throws on a header it cannot read — an entry that is not `<target>`-shaped, or a next target
 * without both `after_created` and `after_id`. Reading an unreadable continuation as "no more"
 * would show the reader one page as the whole list; thrown, it renders as a failed region.
 */
export function nextFromLink(header: string | null): ArtifactCursor | null {
	if (header === null) return null;
	for (const raw of header.split(',')) {
		const entry = raw.trim();
		const close = entry.indexOf('>');
		if (!entry.startsWith('<') || close < 0) {
			throw new Error(`malformed Link header on the artifact list: ${header}`);
		}
		if (!namesNext(entry.slice(close + 1))) continue;
		const target = new URL(entry.slice(1, close), 'http://link.invalid/');
		const created = target.searchParams.get('after_created');
		const artifactId = target.searchParams.get('after_id');
		if (!created || !artifactId) {
			throw new Error(`malformed Link header on the artifact list (no position): ${header}`);
		}
		return { created, artifact_id: artifactId };
	}
	return null;
}

/**
 * The API read for the page the resource page's URL names: always bounded, folded artifacts
 * included, continuing after the URL's position when it carries one.
 *
 * Whatever part of a position the URL holds is passed on as is. Half a position is the API's to
 * refuse (it names the missing half), which renders as a failed region, never as a first page.
 */
export function artifactListPath(id: string, pageQuery: URLSearchParams): string {
	const query = new URLSearchParams({
		include_folded: 'true',
		limit: String(ARTIFACT_PAGE_ROWS),
	});
	const created = pageQuery.get(PAGE_AFTER_CREATED);
	const artifactId = pageQuery.get(PAGE_AFTER_ID);
	if (created !== null) query.set('after_created', created);
	if (artifactId !== null) query.set('after_id', artifactId);
	return `/api/resources/${id}/artifacts?${query}`;
}

/** Whether the resource page's URL names a position, i.e. is past the first page. */
export function pastFirstPage(pageQuery: URLSearchParams): boolean {
	return pageQuery.has(PAGE_AFTER_CREATED) || pageQuery.has(PAGE_AFTER_ID);
}

/**
 * This page's URL at another artifacts position: after `position`, or the first page when it is
 * `null`. Every other query key is kept, so nothing else the URL says moves.
 */
export function artifactPageHref(pageUrl: URL, position: ArtifactCursor | null): string {
	const url = new URL(pageUrl);
	if (position) {
		url.searchParams.set(PAGE_AFTER_CREATED, position.created);
		url.searchParams.set(PAGE_AFTER_ID, position.artifact_id);
	} else {
		url.searchParams.delete(PAGE_AFTER_CREATED);
		url.searchParams.delete(PAGE_AFTER_ID);
	}
	return `${url.pathname}${url.search}`;
}
