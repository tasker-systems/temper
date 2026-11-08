import type { TeamDetail } from '$lib/types';

/**
 * What the team page renders, and nothing else.
 *
 * The page's data is serialized into the HTML it ships, so whatever `load` returns is in the
 * payload whether or not the template shows it. `GET /api/teams/{id}` already withholds what the
 * reader may not see — `public_within` is answered only to the team's own members and system
 * admins, `public_within_in` and `children` only to its managers — and this projection keeps the
 * page to the fields it names, so a field the API grows later never rides along unrendered.
 *
 * A public_within row carries its anchor, whether it confers (a boolean only: nothing on the wire
 * says which forbid, dead team or pool team stops it, and nothing here guesses), when it was
 * opened, and its audience as counts. `instance_wide` is deliberately not carried.
 */
export type TeamView = {
	id: string;
	slug: string;
	name: string;
	description: string | null;
	members: { profileId: string; handle: string; role: string }[];
	parents: { id: string; slug: string; name: string }[];
	children: { id: string; slug: string; name: string }[];
	publicWithin: {
		anchor: { id: string; slug: string; name: string };
		effective: boolean;
		openedAt: string;
		audience: { teams: number; principals: number };
	}[];
	publicWithinIn: { team: { id: string; slug: string; name: string }; effective: boolean }[];
};

const ref = (t: { id: string; slug: string; name: string }) => ({
	id: t.id,
	slug: t.slug,
	name: t.name,
});

export function toTeamView(d: TeamDetail): TeamView {
	return {
		id: d.id,
		slug: d.slug,
		name: d.name,
		description: d.description ?? null,
		members: d.members.map((m) => ({ profileId: m.profile_id, handle: m.handle, role: m.role })),
		parents: (d.parents ?? []).map(ref),
		children: (d.children ?? []).map(ref),
		publicWithin: (d.public_within ?? []).map((r) => ({
			anchor: ref(r.anchor),
			effective: r.effective,
			openedAt: r.opened_at,
			audience: { teams: Number(r.audience.teams), principals: Number(r.audience.principals) },
		})),
		publicWithinIn: (d.public_within_in ?? []).map((r) => ({
			team: ref(r.team),
			effective: r.effective,
		})),
	};
}

/** `/teams/<slug>` — the one place the team page's address is spelled. */
export function teamHref(slug: string): string {
	return `/teams/${encodeURIComponent(slug)}`;
}
