import { describe, expect, it } from 'vitest';
import type { TeamDetail } from '$lib/types';
import { toTeamView } from './team-view';

const team = (id: string, slug: string) => ({ id, slug, name: slug.toUpperCase() });

describe('toTeamView', () => {
	it('carries only the fields the page renders: no instance_wide, no unnamed extras', () => {
		const detail = {
			...team('t', 'eng-sec'),
			description: null,
			created: '2026-11-01T00:00:00Z',
			auto_join_role: null,
			members: [{ profile_id: 'p', handle: 'ana', role: 'owner', source: 'direct' }],
			children: [],
			parents: [team('a', 'eng')],
			public_within: [
				{
					anchor: team('a', 'eng'),
					effective: false,
					instance_wide: true,
					opened_at: '2026-11-07T12:00:00Z',
					audience: { teams: 3, principals: 9 },
				},
			],
			public_within_in: [],
			// A field the API might grow later never rides into the page's payload.
			forbidding_team: team('x', 'secret'),
		} as unknown as TeamDetail;

		const view = toTeamView(detail);
		expect(view.publicWithin).toEqual([
			{
				anchor: team('a', 'eng'),
				effective: false,
				openedAt: '2026-11-07T12:00:00Z',
				audience: { teams: 3, principals: 9 },
			},
		]);
		const serialized = JSON.stringify(view);
		expect(serialized).not.toContain('instance_wide');
		expect(serialized).not.toContain('secret');
		expect(serialized).not.toContain('source');
	});

	it('an answer without the public_within fields (an older server) renders none', () => {
		const detail = {
			...team('t', 'eng'),
			description: null,
			created: '2026-11-01T00:00:00Z',
			auto_join_role: null,
			members: [],
		} as unknown as TeamDetail;
		const view = toTeamView(detail);
		expect(view.publicWithin).toEqual([]);
		expect(view.publicWithinIn).toEqual([]);
		expect(view.children).toEqual([]);
	});
});
