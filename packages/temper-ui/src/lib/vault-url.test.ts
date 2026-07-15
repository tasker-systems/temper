import { describe, expect, it } from 'vitest';
import {
	contextHref,
	contextGraphHref,
	isContextGraphLocation,
	isContextLocation,
	resourceHref,
	searchHref
} from './vault-url';
import type { ResourceRow } from './types/generated/resource';

const ID = '019f420c-cf01-7bc1-87c9-09684b0fa69e';

function makeRow(partial: Partial<ResourceRow>): ResourceRow {
	return {
		id: ID,
		kb_context_id: '00000000-0000-0000-0003-000000000001',
		origin_uri: '',
		title: 'T',
		originator_profile_id: '00000000-0000-0000-0000-000000000001',
		owner_profile_id: '00000000-0000-0000-0000-000000000001',
		is_active: true,
		created: '2026-07-08T00:00:00Z',
		updated: '2026-07-08T00:00:00Z',
		context_name: 'Temper',
		doc_type_name: 'task',
		owner_handle: 'j-cole-taylor',
		context_slug: 'temper',
		context_owner_ref: '@j-cole-taylor',
		cogmap_id: null,
		cogmap_name: null,
		stage: null,
		seq: null,
		mode: null,
		effort: null,
		body_hash: null,
		ingest_state: 'complete',
		body_storage: 'derived',
		...partial
	};
}

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
		expect(contextGraphHref('+acme-team', 'ops team')).toBe(
			'/graph/+acme-team?context=ops%20team'
		);
	});
});

describe('resourceHref', () => {
	it('builds the full resource path for a context-homed resource', () => {
		expect(resourceHref(makeRow({}))).toBe(`/vault/@j-cole-taylor/temper/task/${ID}`);
	});

	it('uses the exact doc_type and the bare id (no decorated ref)', () => {
		expect(resourceHref(makeRow({ doc_type_name: 'session' }))).toBe(
			`/vault/@j-cole-taylor/temper/session/${ID}`
		);
	});

	it('percent-encodes the doc_type segment', () => {
		expect(resourceHref(makeRow({ doc_type_name: 'a b' }))).toBe(
			`/vault/@j-cole-taylor/temper/a%20b/${ID}`
		);
	});

	it('returns null for a cogmap-homed resource (null context fields)', () => {
		expect(
			resourceHref(
				makeRow({ context_owner_ref: null, context_slug: null, cogmap_id: 'x', cogmap_name: 'Map' })
			)
		).toBe(null);
	});
});

describe('searchHref', () => {
	it('encodes the query', () => {
		expect(searchHref('auth flow')).toBe('/vault/search?q=auth%20flow');
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
				'temper'
			)
		).toBe(true);
	});

	it('matches the Atlas door, where the context is the ?context= scope', () => {
		expect(isContextLocation({ owner: '@me' }, at('/graph/@me?context=temper'), '@me', 'temper')).toBe(
			true
		);
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
				'writing'
			)
		).toBe(false);
	});

	it('round-trips both builders, so the inverse cannot drift from them', () => {
		expect(
			isContextLocation({ owner: '@me', context: 'ops team' }, at(contextHref('@me', 'ops team')), '@me', 'ops team')
		).toBe(true);
		expect(
			isContextLocation({ owner: '@me' }, at(contextGraphHref('@me', 'ops team')), '@me', 'ops team')
		).toBe(true);
	});
});

describe('isContextGraphLocation', () => {
	it('is true on the Atlas door for that context', () => {
		expect(
			isContextGraphLocation({ owner: '@me' }, at('/graph/@me?context=temper'), '@me', 'temper')
		).toBe(true);
	});

	it('is false on the context vault page, which links to the door rather than being it', () => {
		expect(
			isContextGraphLocation(
				{ owner: '@me', context: 'temper' },
				at('/vault/@me/temper'),
				'@me',
				'temper'
			)
		).toBe(false);
	});

	it('is false on the door for some other context', () => {
		expect(
			isContextGraphLocation({ owner: '@me' }, at('/graph/@me?context=writing'), '@me', 'temper')
		).toBe(false);
	});
});
