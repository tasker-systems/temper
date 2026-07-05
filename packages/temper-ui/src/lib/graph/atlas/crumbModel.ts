import type { Focus } from './nav';
import type { TeamScopeView } from '$lib/types/generated/graph_scope';

export interface CrumbSegment {
	label: string;
	kind: 'home' | 'ancestor' | 'team' | 'cogmap' | 'territory' | 'node';
	/** The `?focus=` value this segment navigates to; null for home/scope segments. */
	focusPath: string | null;
}

export interface CrumbInput {
	scope: TeamScopeView | null;
	cogmapName: string | null;
	focusPath: Focus[];
	crumbTerritory: { id: string; label: string | null } | null;
	seedTitle: string | null;
}

/** Focus entries that actually appear in a drill path always carry an id;
 *  `{ kind: 'none' }` never reaches here (guarded below), but the union includes
 *  it so we narrow explicitly to keep this typesafe. */
type DrillFocus = Exclude<Focus, { kind: 'none' }>;

const encode = (path: DrillFocus[]): string => path.map((f) => `${f.kind}:${f.id}`).join(',');

/** Derive the ordered breadcrumb segments from URL/loaded state. Pure. */
export function crumbModel(input: CrumbInput): CrumbSegment[] {
	const segs: CrumbSegment[] = [{ label: '⌂ Atlas', kind: 'home', focusPath: null }];

	if (input.scope) {
		for (const a of input.scope.ancestors) segs.push({ label: a.name, kind: 'ancestor', focusPath: null });
		segs.push({ label: input.scope.team.name, kind: 'team', focusPath: null });
	} else if (input.cogmapName) {
		segs.push({ label: input.cogmapName, kind: 'cogmap', focusPath: null });
	}

	// Build cumulative focus paths so each drill segment links to its own depth.
	const walked: DrillFocus[] = [];
	for (const f of input.focusPath) {
		if (f.kind === 'none') continue;
		walked.push(f);
		if (f.kind === 'territory') {
			const label = input.crumbTerritory?.id === f.id ? input.crumbTerritory.label : null;
			segs.push({ label: label ?? 'Region', kind: 'territory', focusPath: encode(walked) });
		} else {
			segs.push({ label: input.seedTitle ?? 'Node', kind: 'node', focusPath: encode(walked) });
		}
	}
	return segs;
}
