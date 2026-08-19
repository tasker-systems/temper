import { describe, expect, it } from 'vitest';
import { ROW_ID as ID, makeRow, withoutKey } from '../test/fixtures';
import {
	contextGraphHref,
	contextHref,
	isCogmapHomed,
	isContextGraphLocation,
	isContextLocation,
	resourceHref,
	searchHref,
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
	it('points at the Atlas context door', () => {
		expect(contextGraphHref('@me', 'temper')).toBe('/graph/@me?context=temper');
	});

	it('keeps the owner sigil but encodes the slug scope', () => {
		expect(contextGraphHref('+acme-team', 'ops team')).toBe('/graph/+acme-team?context=ops%20team');
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

	it('matches the Atlas door, where the context is the ?context= scope', () => {
		expect(
			isContextLocation({ owner: '@me' }, at('/graph/@me?context=temper'), '@me', 'temper'),
		).toBe(true);
	});

	it('does not match a different owner or a different context', () => {
		const door = at('/graph/@me?context=temper');
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
				at('/vault/@me/temper?context=writing'),
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
	it('is true on the Atlas door for that context', () => {
		expect(
			isContextGraphLocation({ owner: '@me' }, at('/graph/@me?context=temper'), '@me', 'temper'),
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
