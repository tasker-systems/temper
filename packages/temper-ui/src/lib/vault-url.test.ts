import { describe, expect, it } from 'vitest';
import { ROW_ID as ID, makeRow, withoutKey } from '../test/fixtures';
import {
	contextGraphHref,
	contextHref,
	graphHref,
	isCogmapHomed,
	isContextGraphLocation,
	isContextLocation,
	parseGraphAddress,
	resourceHref,
	searchHref,
	withGraphQuestion,
	withGraphSeed,
	withGraphSelection,
} from './vault-url';

describe('contextHref', () => {
	it('builds /vault/{ownerRef}/{slug} without encoding the sigil', () => {
		expect(contextHref('@j-cole-taylor', 'temper')).toBe('/vault/@j-cole-taylor/temper');
		expect(contextHref('+acme-team', 'ops')).toBe('/vault/+acme-team/ops');
	});

	it('encodes the slug defensively', () => {
		expect(contextHref('@me', 'my context')).toBe('/vault/@me/my%20context');
	});
});

describe('contextGraphHref', () => {
	it('addresses the context as an `in` anchor, not a `?context=` scope', () => {
		expect(contextGraphHref('@me', 'temper')).toBe('/graph/@me?in=ctx%3A%40me%2Ftemper');
	});

	it('keeps the route owner sigil, and carries the anchor ref whole', () => {
		// The route's `[owner]` is the reader whose graph this is; the ANCHOR carries its own
		// owner, which is what makes a team context expressible at a personal door at all.
		expect(contextGraphHref('+acme-team', 'ops team')).toBe(
			'/graph/+acme-team?in=ctx%3A%2Bacme-team%2Fops+team',
		);
	});

	it('round-trips through the parser, so the two spellings cannot drift', () => {
		const url = new URL(contextGraphHref('+acme-team', 'ops team'), 'https://temperkb.io');

		expect(parseGraphAddress(url).anchors).toEqual([
			{ kind: 'context', ref: '+acme-team/ops team' },
		]);
	});
});

describe('resourceHref', () => {
	it('returns a ref route for a cogmap-homed row (the 533-resource fix)', () => {
		const row = makeRow({
			context_owner_ref: null,
			context_slug: null,
			cogmap_id: 'x',
			cogmap_name: 'Map',
			doc_type_name: 'concept',
		});
		expect(resourceHref(row)).toBe(`/vault/r/${ID}`);
	});

	it('returns the same ref route for a context-homed row', () => {
		const row = makeRow({
			context_owner_ref: '@j-cole-taylor',
			context_slug: 'temper',
			doc_type_name: 'task',
		});
		expect(resourceHref(row)).toBe(`/vault/r/${ID}`);
	});

	it('never returns null, whatever the home', () => {
		expect(resourceHref(makeRow({ context_owner_ref: null, context_slug: null }))).toBeTruthy();
	});

	it('ignores doc_type — the route resolves on the id alone', () => {
		// The old path carried an encoded doc_type segment. Resolution was always
		// trailing-UUID-only, so it never disambiguated anything.
		expect(resourceHref(makeRow({ doc_type_name: 'a b' }))).toBe(`/vault/r/${ID}`);
	});
});

describe('searchHref', () => {
	it('encodes the query', () => {
		expect(searchHref('auth flow')).toBe('/vault/search?q=auth%20flow');
	});
});

describe('isCogmapHomed', () => {
	it('is false for a context-homed row AS THE WIRE SENDS IT — with no cogmap_id key at all', () => {
		// The bite test. `skip_serializing_if = "Option::is_none"` omits the key, so the value is
		// `undefined`, and `undefined !== null` is true — the previous rule classified every
		// context-homed resource as a cogmap and rendered it as dead, unlinked text.
		const row = withoutKey(withoutKey(makeRow({}), 'cogmap_id'), 'cogmap_name');
		expect(row).not.toHaveProperty('cogmap_id');
		expect(isCogmapHomed(row)).toBe(false);
	});

	it('is true for a cogmap-homed row', () => {
		// The positive control: without it, a rule that always returned false would pass above.
		const row = withoutKey(
			makeRow({ cogmap_id: 'ac1d0000-0000-0000-0000-00000000c0de', cogmap_name: 'Map' }),
			'context_name',
		);
		expect(isCogmapHomed(row)).toBe(true);
	});

	it('agrees with itself on an explicit null, so both wire shapes classify alike', () => {
		// A null key reaches this from any hand-built fixture or an older serializer. Absent and
		// null mean the same thing — "not homed here" — and must never diverge.
		expect(isCogmapHomed(makeRow({ cogmap_id: null }))).toBe(false);
	});
});

const at = (href: string) => new URL(href, 'https://temperkb.io');

describe('isContextLocation', () => {
	it('matches the vault route, where the context is a path param', () => {
		expect(
			isContextLocation(
				{ owner: '@me', context: 'temper' },
				at('/vault/@me/temper'),
				'@me',
				'temper',
			),
		).toBe(true);
	});

	it('matches the graph door, where the context is an `in` anchor', () => {
		expect(
			isContextLocation({ owner: '@me' }, at(contextGraphHref('@me', 'temper')), '@me', 'temper'),
		).toBe(true);
	});

	it('matches a TEAM context at a personal door, because the anchor carries its own owner', () => {
		// The route `[owner]` is deliberately not consulted on this door. `/graph/@me` asking about
		// `+acme-team/ops` is the cross-owner reach the anchor grammar exists to provide, and
		// checking the route owner would mark the place inactive on the screen showing it.
		expect(
			isContextLocation(
				{ owner: '@me' },
				at(`/graph/@me?in=ctx%3A%2Bacme-team%2Fops`),
				'+acme-team',
				'ops',
			),
		).toBe(true);
	});

	it('matches one anchor among several', () => {
		const many = at('/graph/@me?in=ctx%3A%40me%2Ftemper&in=map%3Aabc&in=ctx%3A%40me%2Fwriting');

		expect(isContextLocation({ owner: '@me' }, many, '@me', 'writing')).toBe(true);
		expect(isContextLocation({ owner: '@me' }, many, '@me', 'absent')).toBe(false);
	});

	it('does not match a different owner or a different context', () => {
		const door = at(contextGraphHref('@me', 'temper'));
		expect(isContextLocation({ owner: '@me' }, door, '+acme-team', 'temper')).toBe(false);
		expect(isContextLocation({ owner: '@me' }, door, '@me', 'writing')).toBe(false);
	});

	it('does not match a route that addresses no context', () => {
		expect(isContextLocation({}, at('/vault/search?q=temper'), '@me', 'temper')).toBe(false);
		expect(isContextLocation({ owner: '@me' }, at('/graph/@me'), '@me', 'temper')).toBe(false);
	});

	it('prefers the path param when a route somehow carries both', () => {
		expect(
			isContextLocation(
				{ owner: '@me', context: 'temper' },
				at('/vault/@me/temper?in=ctx%3A%40me%2Fwriting'),
				'@me',
				'writing',
			),
		).toBe(false);
	});

	it('round-trips both builders, so the inverse cannot drift from them', () => {
		expect(
			isContextLocation(
				{ owner: '@me', context: 'ops team' },
				at(contextHref('@me', 'ops team')),
				'@me',
				'ops team',
			),
		).toBe(true);
		expect(
			isContextLocation(
				{ owner: '@me' },
				at(contextGraphHref('@me', 'ops team')),
				'@me',
				'ops team',
			),
		).toBe(true);
	});
});

describe('isContextGraphLocation', () => {
	it('is true on the graph door for that context', () => {
		expect(
			isContextGraphLocation(
				{ owner: '@me' },
				at(contextGraphHref('@me', 'temper')),
				'@me',
				'temper',
			),
		).toBe(true);
	});

	it('is false on the context vault page, which links to the door rather than being it', () => {
		expect(
			isContextGraphLocation(
				{ owner: '@me', context: 'temper' },
				at('/vault/@me/temper'),
				'@me',
				'temper',
			),
		).toBe(false);
	});

	it('is false on the door for some other context', () => {
		expect(
			isContextGraphLocation({ owner: '@me' }, at('/graph/@me?context=writing'), '@me', 'temper'),
		).toBe(false);
	});
});

describe('the graph surface address', () => {
	const at = (path: string) => new URL(path, 'https://temperkb.io');

	describe('parseGraphAddress', () => {
		it('reads a question, anchors, seeds and a selection', () => {
			const a = parseGraphAddress(
				at('/graph/@me?q=what+now&in=ctx:@me/temper&from=aaaa-1&sel=bbbb-2'),
			);

			expect(a.question).toBe('what now');
			expect(a.anchors).toEqual([{ kind: 'context', ref: '@me/temper' }]);
			expect(a.seeds).toEqual(['aaaa-1']);
			expect(a.selection).toBe('bbbb-2');
		});

		it('is empty, not malformed, at the unaddressed door', () => {
			const a = parseGraphAddress(at('/graph/@me'));

			expect(a).toEqual({ question: null, anchors: [], seeds: [], selection: null });
		});

		it('carries a WHOLE ref, so a team context is expressible at all', () => {
			const a = parseGraphAddress(at('/graph/@me?in=ctx:+acme/ops'));

			expect(a.anchors).toEqual([{ kind: 'context', ref: '+acme/ops' }]);
		});

		it('reads a cogmap anchor, which needs no owner', () => {
			const a = parseGraphAddress(at('/graph/@me?in=map:019f2391-e001-7933-b88a-28fb92e56ac1'));

			expect(a.anchors).toEqual([{ kind: 'cogmap', ref: '019f2391-e001-7933-b88a-28fb92e56ac1' }]);
		});

		it('spans both anchor kinds in one address', () => {
			const a = parseGraphAddress(at('/graph/@me?in=ctx:@me/temper&in=map:abc-1&in=ctx:+t/ops'));

			expect(a.anchors).toEqual([
				{ kind: 'context', ref: '@me/temper' },
				{ kind: 'cogmap', ref: 'abc-1' },
				{ kind: 'context', ref: '+t/ops' },
			]);
		});

		it('drops an anchor with no kind prefix — a bare slug is not the grammar', () => {
			const a = parseGraphAddress(at('/graph/@me?in=temper&in=ctx:@me/temper'));

			expect(a.anchors).toEqual([{ kind: 'context', ref: '@me/temper' }]);
		});

		it('drops an unknown kind prefix rather than guessing at it', () => {
			expect(parseGraphAddress(at('/graph/@me?in=region:abc')).anchors).toEqual([]);
		});

		it('drops a kind prefix carrying no ref', () => {
			expect(parseGraphAddress(at('/graph/@me?in=ctx:')).anchors).toEqual([]);
		});

		it('treats a blank question as absent, so ?q= is not a search for nothing', () => {
			expect(parseGraphAddress(at('/graph/@me?q=+++')).question).toBeNull();
		});

		it('reads every from seed, in order', () => {
			expect(parseGraphAddress(at('/graph/@me?from=a&from=b&from=c')).seeds).toEqual([
				'a',
				'b',
				'c',
			]);
		});
	});

	describe('graphHref', () => {
		it('is bare at the unaddressed door', () => {
			expect(graphHref('@me', {})).toBe('/graph/@me');
		});

		it('keeps the owner sigil unencoded, as every other builder here does', () => {
			expect(graphHref('+acme', {})).toBe('/graph/+acme');
		});

		it('emits each part under its own param', () => {
			const href = graphHref('@me', {
				question: 'what now',
				anchors: [
					{ kind: 'context', ref: '@me/temper' },
					{ kind: 'cogmap', ref: 'abc-1' },
				],
				seeds: ['s1'],
				selection: 'r1',
			});

			expect(href).toBe(
				'/graph/@me?q=what+now&in=ctx%3A%40me%2Ftemper&in=map%3Aabc-1&from=s1&sel=r1',
			);
		});

		it('round-trips every part through the parser', () => {
			const address = {
				question: 'a question with spaces & an ampersand',
				anchors: [
					{ kind: 'context' as const, ref: '+acme/ops' },
					{ kind: 'cogmap' as const, ref: '019f2391-e001-7933-b88a-28fb92e56ac1' },
				],
				seeds: ['aaaa-1', 'bbbb-2'],
				selection: 'cccc-3',
			};

			expect(parseGraphAddress(at(graphHref('@me', address)))).toEqual(address);
		});

		it('round-trips a context slug carrying characters a URL would eat', () => {
			const address = {
				question: null,
				anchors: [{ kind: 'context' as const, ref: '@me/a b&c' }],
				seeds: [],
				selection: null,
			};

			expect(parseGraphAddress(at(graphHref('@me', address)))).toEqual(address);
		});
	});
});

describe('the graph URL mutators change one part and leave the rest alone', () => {
	const at = (search: string) => new URL(`https://x.test/graph/@me${search}`);

	it('selects a bare resource uuid — there is no kind prefix to give', () => {
		// The vocabulary has one selectable kind. An edge is a `ViaEntry` and carries no id at
		// all, so a `node:` prefix would name a distinction that does not exist.
		expect(withGraphSelection(at('?q=x'), 'abc')).toBe('/graph/@me?q=x&sel=abc');
	});

	it('clearing the selection leaves the question and the places untouched', () => {
		const url = at('?q=x&in=ctx%3A%40me%2Ftemper&from=r1&sel=abc');

		expect(withGraphSelection(url, null)).toBe('/graph/@me?q=x&in=ctx%3A%40me%2Ftemper&from=r1');
	});

	it('a selection never changes what was asked', () => {
		const url = at('?q=how&in=map%3A019f&from=r1');
		const next = new URL(`https://x.test${withGraphSelection(url, 'n1')}`);

		expect(next.searchParams.get('q')).toBe('how');
		expect(next.searchParams.getAll('in')).toEqual(['map:019f']);
		expect(next.searchParams.getAll('from')).toEqual(['r1']);
	});

	it('asking a new question drops the selection, which named a node in the old answer', () => {
		expect(withGraphQuestion(at('?in=ctx%3A%40me%2Ftemper&sel=abc'), 'why')).toBe(
			'/graph/@me?in=ctx%3A%40me%2Ftemper&q=why',
		);
	});

	it('clearing the question keeps the places — the door is still addressed', () => {
		expect(withGraphQuestion(at('?q=x&in=ctx%3A%40me%2Ftemper'), null)).toBe(
			'/graph/@me?in=ctx%3A%40me%2Ftemper',
		);
	});

	it('a blank question is the same as no question, not a question that is empty', () => {
		expect(withGraphQuestion(at('?q=x'), '   ')).toBe('/graph/@me');
	});

	it('walking from a seed replaces the question, which no longer decides the answer', () => {
		// `from` REPLACES the upstream stage as what the walk grows from, so a stale `q` would
		// leave a question on screen that decides nothing about what is drawn.
		expect(withGraphSeed(at('?q=x&in=ctx%3A%40me%2Ftemper&sel=abc'), 'r9')).toBe(
			'/graph/@me?in=ctx%3A%40me%2Ftemper&from=r9',
		);
	});

	it('walking from a seed replaces any previous seed rather than accumulating', () => {
		expect(withGraphSeed(at('?from=r1&from=r2'), 'r9')).toBe('/graph/@me?from=r9');
	});

	it('every mutator round-trips through the parser it is the inverse of', () => {
		const url = new URL(`https://x.test${withGraphSeed(at('?in=ctx%3A%2Bteam%2Fops'), 'r9')}`);
		const address = parseGraphAddress(url);

		expect(address.anchors).toEqual([{ kind: 'context', ref: '+team/ops' }]);
		expect(address.seeds).toEqual(['r9']);
		expect(address.question).toBeNull();
	});
});
