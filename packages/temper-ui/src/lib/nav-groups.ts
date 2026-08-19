import type { ContextRowWithCounts, TeamRow } from '$lib/types';

/**
 * The nav's grouping model: which **groups** a reader belongs to, and which
 * **places** within each hold work — both legible at once.
 *
 * The sidebar used to split contexts on `kb_owner_table` and render every
 * team-owned context flat under one literal "Teams" heading, with `owner_ref`
 * never rendered at all. Which team a context belonged to was therefore
 * invisible: the reader got the places and not the groups.
 *
 * Pure so the grouping and the three-way availability state are unit-testable
 * without a DOM; the components below only render what this returns.
 */

/** A group's owner sigil decides how it is labelled and ordered. */
export type NavGroupKind = 'self' | 'team' | 'profile';

export interface NavGroup {
	/**
	 * The group's `owner_ref` (`@handle` / `+team-slug`). Stable per owner and
	 * per reader, so it is also the persisted collapse key (`sidebar.svelte.ts`).
	 */
	key: string;
	/** Team display name where known, else the bare ref with its sigil dropped. */
	label: string;
	kind: NavGroupKind;
	contexts: ContextRowWithCounts[];
	/** Sum of the group's readable places' counts — what a collapsed group still reports. */
	resourceCount: number;
}

/**
 * What the nav has to render, as three distinct facts rather than one list.
 *
 * `unavailable` is NOT `empty`. The layout surfaces `null` on a failed
 * `/api/contexts` read precisely so the two stay apart (`+layout.server.ts:38-42`);
 * an empty nav says "you belong to nothing", which is a claim about the reader
 * that a fetch which never answered cannot support.
 */
export type NavContextsState =
	| { kind: 'unavailable' }
	| { kind: 'empty' }
	| { kind: 'groups'; groups: NavGroup[] };

/**
 * The reader's own group. Their handle is the one label in this list that says
 * nothing they don't already know, and "Contexts" (what the heading used to
 * read) no longer distinguishes it now that another profile's shared places can
 * sit beside it under their own heading.
 */
export const SELF_GROUP_LABEL = 'My contexts';

/** `+acme-eng` → `acme-eng`, `@alice` → `alice`. The fallback when no name is known. */
function bareRef(ownerRef: string): string {
	return ownerRef.replace(/^[@+]/, '');
}

/** A team's display name, falling back to its slug when unknown or blank. */
function teamLabel(name: string | undefined, slug: string): string {
	return name?.trim() || slug;
}

/**
 * Group the reader's readable contexts by the owner that holds them.
 *
 * **Grouping is derived from the contexts themselves, never from `teams`.**
 * `/api/contexts` is scoped by `context_visible_to`, whereas `/api/teams`
 * returns only the teams the caller is a *member* of — so a team-owned context
 * can be readable without membership. Keying groups off the teams list would
 * silently drop those places. `teams` does exactly two things: it upgrades a
 * heading from `+team-slug` to the team's display name, and it contributes the
 * groups a reader belongs to that hold no readable place.
 *
 * A team a reader belongs to whose places they cannot read renders as an empty
 * group — the places are absent, and their absence stays indistinguishable
 * from their nonexistence.
 *
 * `teams` is `null` on a failed read, which degrades labels to the bare slug
 * and drops empty groups. Never the places: those come from `contexts`.
 */
export function navContextsState(
	contexts: ContextRowWithCounts[] | null,
	teams: TeamRow[] | null,
	selfProfileId: string,
): NavContextsState {
	if (contexts === null) return { kind: 'unavailable' };

	const nameBySlug = new Map((teams ?? []).map((t) => [t.slug, t.name]));

	const byKey = new Map<string, NavGroup>();

	// Seed the groups the reader belongs to, so a team holding no readable place
	// is still visibly a group they are in rather than simply missing.
	for (const team of teams ?? []) {
		const key = `+${team.slug}`;
		byKey.set(key, {
			key,
			label: teamLabel(team.name, team.slug),
			kind: 'team',
			contexts: [],
			resourceCount: 0,
		});
	}

	for (const ctx of contexts) {
		let group = byKey.get(ctx.owner_ref);
		if (!group) {
			const bare = bareRef(ctx.owner_ref);
			const isTeam = ctx.owner_ref.startsWith('+');
			const isSelf = !isTeam && ctx.kb_owner_id === selfProfileId;
			group = {
				key: ctx.owner_ref,
				label: isTeam ? teamLabel(nameBySlug.get(bare), bare) : isSelf ? SELF_GROUP_LABEL : bare,
				kind: isTeam ? 'team' : isSelf ? 'self' : 'profile',
				contexts: [],
				resourceCount: 0,
			};
			byKey.set(group.key, group);
		}
		group.contexts.push(ctx);
		// `resource_count` is `bigint` in the ts-rs binding; normalize the way
		// `vault-list.ts:70` does rather than summing across two numeric types.
		group.resourceCount += Number(ctx.resource_count);
	}

	if (byKey.size === 0) return { kind: 'empty' };

	// The reader's own places first, then the groups they share, each block
	// alphabetical so the nav does not reorder itself between loads.
	const rank: Record<NavGroupKind, number> = { self: 0, team: 1, profile: 2 };
	const groups = [...byKey.values()].sort(
		(a, b) => rank[a.kind] - rank[b.kind] || a.label.localeCompare(b.label),
	);

	return { kind: 'groups', groups };
}
