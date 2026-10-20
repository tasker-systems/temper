import { describe, expect, it } from 'vitest';
import {
	ARTIFACT_PAGE_ROWS,
	artifactListPath,
	artifactPageHref,
	nextFromLink,
	PAGE_AFTER_CREATED,
	PAGE_AFTER_ID,
	pastFirstPage,
} from './artifact-page';

const AFTER_ID = '00000000-0000-0000-0000-0000000000aa';
const AFTER_CREATED = '2026-10-19T12:30:45.123456Z';

/** The server's form: a relative target, `serde_urlencoded`'s percent-encoding, `rel="next"`. */
const serverLink =
	'</api/resources/r1/artifacts?include_folded=true&limit=50' +
	`&after_created=2026-10-19T12%3A30%3A45.123456Z&after_id=${AFTER_ID}>; rel="next"`;

describe('nextFromLink', () => {
	it("reads the position out of the server's next link", () => {
		expect(nextFromLink(serverLink)).toEqual({ created: AFTER_CREATED, artifact_id: AFTER_ID });
	});

	it('reads no next page from no header, or from a link that is not the next page', () => {
		expect(nextFromLink(null)).toBeNull();
		expect(nextFromLink('</api/resources/r1/artifacts>; rel="prev"')).toBeNull();
		expect(nextFromLink('</a>; rel="nextish"')).toBeNull();
		expect(nextFromLink('</a>; title="next"')).toBeNull();
		// Only the first `rel` counts.
		expect(nextFromLink('</a>; rel="prev"; rel="next"')).toBeNull();
	});

	it('reads `rel` as RFC 8288 allows: any case, quoted or bare, a list of types', () => {
		const target = `</a?after_created=${encodeURIComponent(AFTER_CREATED)}&after_id=${AFTER_ID}>`;
		for (const params of [
			'; rel=next',
			'; REL="next"',
			'; Rel=NEXT',
			'; rel="next last"',
			'; rel="prev  next"',
			'; title="page"; rel="next"',
		]) {
			expect(nextFromLink(`${target}${params}`), params).toEqual({
				created: AFTER_CREATED,
				artifact_id: AFTER_ID,
			});
		}
	});

	it('finds the next entry among several', () => {
		expect(nextFromLink(`</first>; rel="first", ${serverLink}`)?.artifact_id).toBe(AFTER_ID);
	});

	it('throws on a header it cannot read, rather than reading one page as the whole list', () => {
		for (const bad of [
			'',
			'not a link',
			'<unterminated; rel="next"',
			'</a>; rel="next"',
			`</a?after_id=${AFTER_ID}>; rel="next"`,
			`</a?after_created=${encodeURIComponent(AFTER_CREATED)}>; rel="next"`,
		]) {
			expect(() => nextFromLink(bad), bad).toThrow(/malformed Link/);
		}
	});
});

describe('artifactListPath', () => {
	it('asks for one bounded first page, folded artifacts included', () => {
		expect(artifactListPath('r1', new URLSearchParams())).toBe(
			`/api/resources/r1/artifacts?include_folded=true&limit=${ARTIFACT_PAGE_ROWS}`,
		);
	});

	it("continues after the page URL's position", () => {
		const page = new URLSearchParams({
			[PAGE_AFTER_CREATED]: AFTER_CREATED,
			[PAGE_AFTER_ID]: AFTER_ID,
		});
		const asked = new URL(artifactListPath('r1', page), 'http://api.invalid');
		expect(asked.searchParams.get('limit')).toBe(String(ARTIFACT_PAGE_ROWS));
		expect(asked.searchParams.get('include_folded')).toBe('true');
		expect(asked.searchParams.get('after_created')).toBe(AFTER_CREATED);
		expect(asked.searchParams.get('after_id')).toBe(AFTER_ID);
	});

	it('passes half a position on for the API to refuse, never dropping it into a first page', () => {
		const asked = new URL(
			artifactListPath('r1', new URLSearchParams({ [PAGE_AFTER_ID]: AFTER_ID })),
			'http://api.invalid',
		);
		expect(asked.searchParams.get('after_id')).toBe(AFTER_ID);
		expect(asked.searchParams.has('after_created')).toBe(false);
	});
});

describe('artifactPageHref and pastFirstPage', () => {
	it('puts the position in the page URL, keeping every other key', () => {
		const href = artifactPageHref(new URL('http://ui.invalid/vault/r/r1?tab=x'), {
			created: AFTER_CREATED,
			artifact_id: AFTER_ID,
		});
		const url = new URL(href, 'http://ui.invalid');
		expect(url.pathname).toBe('/vault/r/r1');
		expect(url.searchParams.get('tab')).toBe('x');
		expect(url.searchParams.get(PAGE_AFTER_CREATED)).toBe(AFTER_CREATED);
		expect(url.searchParams.get(PAGE_AFTER_ID)).toBe(AFTER_ID);
		expect(pastFirstPage(url.searchParams)).toBe(true);
	});

	it('takes the position off for the first page', () => {
		const at = new URL(
			`http://ui.invalid/vault/r/r1?tab=x&${PAGE_AFTER_CREATED}=c&${PAGE_AFTER_ID}=i`,
		);
		expect(artifactPageHref(at, null)).toBe('/vault/r/r1?tab=x');
		expect(pastFirstPage(new URL('http://ui.invalid/vault/r/r1').searchParams)).toBe(false);
	});
});
