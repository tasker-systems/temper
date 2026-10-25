import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { BODY_LINK_CAP, extractVaultLinks, vaultLinkId } from './links.js';
import { MAX_SOURCE_LENGTH } from './markdown.js';

const U1 = '019d4add-f49d-7c43-a87d-dda470e5dd9c';
const U2 = '01a16e2a-570d-77fa-9ca5-a905fe467e9a';

describe('vaultLinkId', () => {
	it('accepts exactly ./<uuid>, with an optional trailing slash, held lowercase', () => {
		expect(vaultLinkId(`./${U1}`)).toBe(U1);
		expect(vaultLinkId(`./${U1}/`)).toBe(U1);
		expect(vaultLinkId(`./${U1.toUpperCase()}`)).toBe(U1);
	});

	it('refuses every looser form', () => {
		for (const href of [
			`/${U1}`,
			U1,
			`./${U1}/x`,
			`./some-note-${U1}`,
			`https://temperkb.io/vault/r/${U1}`,
			`./${U1.slice(1)}`,
			`./${U1}?x=1`,
			`./${U1}#h`,
		]) {
			expect(vaultLinkId(href), href).toBeNull();
		}
	});
});

describe('extractVaultLinks', () => {
	it('returns the distinct ids in first-occurrence order', () => {
		const md = `[b](./${U2}) then [a](./${U1}) and [b again](./${U2.toUpperCase()}/)`;
		expect(extractVaultLinks(md)).toEqual({ ids: [U2, U1], dropped: 0 });
	});

	it('reaches links in tables, list items and reference-style links', () => {
		const md = [
			`| a |\n|---|\n| [t](./${U1}) |`,
			'',
			`- item [l](./${U2})`,
			'',
			'[ref][r]',
			'',
			`[r]: ./${U1.replace('9c', '9d')}`,
		].join('\n');
		expect(extractVaultLinks(md).ids).toEqual([U1, U2, U1.replace('9c', '9d')]);
	});

	it('skips code spans and fences, and leaves non-vault links out', () => {
		const md = [
			`\`[c](./${U1})\``,
			'',
			'```',
			`[f](./${U1})`,
			'```',
			'',
			'[web](https://example.com)',
		].join('\n');
		expect(extractVaultLinks(md)).toEqual({ ids: [], dropped: 0 });
	});

	it('caps at BODY_LINK_CAP and counts the rest', () => {
		const ids = Array.from(
			{ length: BODY_LINK_CAP + 3 },
			(_, i) => `00000000-0000-7000-8000-${i.toString(16).padStart(12, '0')}`,
		);
		const md = ids.map((id) => `[x](./${id})`).join(' ');
		const got = extractVaultLinks(md);
		expect(got.ids).toHaveLength(BODY_LINK_CAP);
		expect(got.ids[0]).toBe(ids[0]);
		expect(got.dropped).toBe(3);
	});

	it('returns nothing past the source bound', () => {
		const md = `[a](./${U1}) ${'x'.repeat(MAX_SOURCE_LENGTH)}`;
		expect(extractVaultLinks(md)).toEqual({ ids: [], dropped: 0 });
	});

	it('is total: a body the lexer throws on returns nothing rather than throwing', () => {
		// A non-string reaches the lexer the way a corrupt read would; marked throws on it.
		expect(extractVaultLinks(42 as unknown as string)).toEqual({ ids: [], dropped: 0 });
	});
});

describe('BODY_LINK_CAP', () => {
	it('is the id-set bound the server publishes', () => {
		// The server refuses an `ids` list over this bound, so a body asks for no more than one
		// request can carry. Read from the published contract rather than restated by hand.
		const openapi = JSON.parse(
			readFileSync(new URL('../../../openapi.json', import.meta.url), 'utf8'),
		);
		expect(openapi.components.schemas.IdSet.properties.ids.maxItems).toBe(BODY_LINK_CAP);
	});
});
