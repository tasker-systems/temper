import { describe, expect, it } from 'vitest';
import { navContextsState, SELF_GROUP_LABEL } from './nav-groups';
import type { ContextRowWithCounts, TeamRow } from './types';

const SELF = '019d4add-0000-7000-0000-000000000001';
const OTHER = '019d4add-0000-7000-0000-000000000002';

function ctx(
	slug: string,
	ownerRef: string,
	opts: { ownerId?: string; count?: number } = {},
): ContextRowWithCounts {
	const isTeam = ownerRef.startsWith('+');
	return {
		id: `ctx-${slug}`,
		name: slug,
		kb_owner_table: isTeam ? 'kb_teams' : 'kb_profiles',
		kb_owner_id: opts.ownerId ?? (isTeam ? 'team-id' : SELF),
		created: '2026-01-01T00:00:00Z',
		updated: '2026-01-01T00:00:00Z',
		// The wire delivers a JSON number; ts-rs declares `bigint`. The cast is the
		// honest fixture — see the sum test below, which pins that both survive.
		resource_count: (opts.count ?? 0) as unknown as bigint,
		slug,
		owner_ref: ownerRef,
	};
}

function team(slug: string, name: string): TeamRow {
	return {
		id: `team-${slug}`,
		slug,
		name,
		description: null,
		created: '2026-01-01T00:00:00Z',
		auto_join_role: null,
	};
}

describe('navContextsState — availability is three facts, not two', () => {
	it('reports a failed contexts read as unavailable, never as empty', () => {
		expect(navContextsState(null, [], SELF)).toEqual({ kind: 'unavailable' });
	});

	it('reports a read that answered with nothing as empty', () => {
		expect(navContextsState([], null, SELF)).toEqual({ kind: 'empty' });
	});

	it('keeps the two apart — a null read is not the same value as an empty one', () => {
		expect(navContextsState(null, null, SELF)).not.toEqual(navContextsState([], null, SELF));
	});
});

describe('navContextsState — groups and their places', () => {
	// The bite: the sidebar this replaces put BOTH of these under one flat "Teams"
	// heading with `owner_ref` unrendered, so which team held which place was
	// invisible. Any implementation that keeps a single team bucket fails here.
	it('separates two teams into their own groups, each holding only its own places', () => {
		const state = navContextsState(
			[ctx('infra', '+platform'), ctx('papers', '+research'), ctx('runbooks', '+platform')],
			[team('platform', 'Platform Group'), team('research', 'Research Group')],
			SELF,
		);
		if (state.kind !== 'groups') throw new Error(`expected groups, got ${state.kind}`);

		expect(state.groups.map((g) => g.key)).toEqual(['+platform', '+research']);
		expect(state.groups[0].contexts.map((c) => c.slug)).toEqual(['infra', 'runbooks']);
		expect(state.groups[1].contexts.map((c) => c.slug)).toEqual(['papers']);
	});

	it('labels a team group with its display name, not its ref', () => {
		const state = navContextsState(
			[ctx('infra', '+platform')],
			[team('platform', 'Platform Group')],
			SELF,
		);
		if (state.kind !== 'groups') throw new Error('expected groups');
		expect(state.groups[0].label).toBe('Platform Group');
	});

	it('renders a team the reader belongs to that holds no readable place', () => {
		const state = navContextsState(
			[ctx('infra', '+platform')],
			[team('platform', 'Platform Group'), team('research', 'Research Group')],
			SELF,
		);
		if (state.kind !== 'groups') throw new Error('expected groups');

		const research = state.groups.find((g) => g.key === '+research');
		expect(research?.label).toBe('Research Group');
		expect(research?.contexts).toEqual([]);
		expect(research?.resourceCount).toBe(0);
	});

	it('groups a team-owned context the reader reads without membership', () => {
		// `/api/contexts` is scoped by `context_visible_to`; `/api/teams` returns only
		// teams the caller is a MEMBER of. Keying groups off the teams list would drop
		// this place entirely.
		const state = navContextsState([ctx('shared', '+outside')], [], SELF);
		if (state.kind !== 'groups') throw new Error('expected groups');

		expect(state.groups.map((g) => g.key)).toEqual(['+outside']);
		expect(state.groups[0].contexts.map((c) => c.slug)).toEqual(['shared']);
	});
});

describe('navContextsState — whose group is whose', () => {
	it("names the reader's own group apart from another profile's", () => {
		const state = navContextsState(
			[ctx('mine', '@me'), ctx('theirs', '@alice', { ownerId: OTHER })],
			[],
			SELF,
		);
		if (state.kind !== 'groups') throw new Error('expected groups');

		expect(state.groups.map((g) => [g.kind, g.label])).toEqual([
			['self', SELF_GROUP_LABEL],
			['profile', 'alice'],
		]);
	});

	it('orders the reader first, then teams, then other profiles — each block alphabetical', () => {
		const state = navContextsState(
			[
				ctx('theirs', '@zoe', { ownerId: OTHER }),
				ctx('papers', '+research'),
				ctx('mine', '@me'),
				ctx('infra', '+platform'),
			],
			[team('platform', 'Platform Group'), team('research', 'Research Group')],
			SELF,
		);
		if (state.kind !== 'groups') throw new Error('expected groups');
		expect(state.groups.map((g) => g.label)).toEqual([
			SELF_GROUP_LABEL,
			'Platform Group',
			'Research Group',
			'zoe',
		]);
	});
});

describe('navContextsState — what a collapsed group still reports', () => {
	it('sums its places’ counts across the wire’s numbers and the binding’s bigints', () => {
		const rows = [ctx('infra', '+platform', { count: 12 }), ctx('runbooks', '+platform')];
		rows[1].resource_count = 30n;

		const state = navContextsState(rows, [], SELF);
		if (state.kind !== 'groups') throw new Error('expected groups');
		expect(state.groups[0].resourceCount).toBe(42);
	});
});

describe('navContextsState — a failed teams read degrades labels, never places', () => {
	it('keeps every place and falls back to the slug', () => {
		const state = navContextsState(
			[ctx('infra', '+platform'), ctx('papers', '+research')],
			null,
			SELF,
		);
		if (state.kind !== 'groups') throw new Error('expected groups');

		expect(state.groups.map((g) => g.label)).toEqual(['platform', 'research']);
		expect(state.groups.flatMap((g) => g.contexts.map((c) => c.slug))).toEqual(['infra', 'papers']);
	});
});
