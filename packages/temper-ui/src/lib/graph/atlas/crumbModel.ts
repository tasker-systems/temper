import { focusToken, type Focus } from './nav';

export interface CrumbSegment {
	label: string;
	kind: 'home' | 'cogmap' | 'context' | 'territory' | 'node' | 'container' | 'bucket' | 'scope';
	/** The `?focus=` value this segment navigates to; null for home/scope/context segments. */
	focusPath: string | null;
}

export interface CrumbInput {
	cogmapName: string | null;
	/** Beat E — the `?context` scope slug, labelling the context root segment. Null off the
	 *  context door. Mutually exclusive with a cogmap in practice (distinct doors). */
	contextSlug: string | null;
	focusPath: Focus[];
	crumbTerritory: { id: string; label: string | null } | null;
	seedTitle: string | null;
	/** The committed Home `?scope` narrow (Beat C), or null when un-narrowed.
	 *  Only meaningful on the Home branch — suppressed once a cogmap is entered. */
	scopeFilter: string | null;
}

/** Focus entries that actually appear in a drill path; `{ kind: 'none' }` never reaches
 *  here (guarded below), but the union includes it so we narrow explicitly to stay
 *  typesafe. Note a `bucket` focus carries no `id` — it is addressed by
 *  `(groupKey, value)` — which is why the token comes from `focusToken`, not from
 *  interpolating `f.id`. */
type DrillFocus = Exclude<Focus, { kind: 'none' }>;

const encode = (path: DrillFocus[]): string => path.map(focusToken).join(',');

/** Derive the ordered breadcrumb segments from URL/loaded state. Pure. */
export function crumbModel(input: CrumbInput): CrumbSegment[] {
	const segs: CrumbSegment[] = [{ label: '⌂ Atlas', kind: 'home', focusPath: null }];

	if (input.cogmapName) {
		segs.push({ label: input.cogmapName, kind: 'cogmap', focusPath: null });
	} else if (input.contextSlug) {
		segs.push({ label: input.contextSlug, kind: 'context', focusPath: null });
	} else if (input.scopeFilter) {
		segs.push({ label: input.scopeFilter, kind: 'scope', focusPath: null });
	}

	// Build cumulative focus paths so each drill segment links to its own depth.
	const walked: DrillFocus[] = [];
	for (const f of input.focusPath) {
		if (f.kind === 'none') continue;
		walked.push(f);
		if (f.kind === 'territory') {
			const label = input.crumbTerritory?.id === f.id ? input.crumbTerritory.label : null;
			segs.push({ label: label ?? 'Region', kind: 'territory', focusPath: encode(walked) });
		} else if (f.kind === 'container') {
			// A goal container's leaf is its title (resolved from the drill subgraph seed).
			segs.push({ label: input.seedTitle ?? 'Container', kind: 'container', focusPath: encode(walked) });
		} else if (f.kind === 'bucket') {
			// A residual bucket carries no id/title — its label comes from the group value.
			segs.push({ label: `Unfiled · ${f.value}`, kind: 'bucket', focusPath: encode(walked) });
		} else {
			segs.push({ label: input.seedTitle ?? 'Node', kind: 'node', focusPath: encode(walked) });
		}
	}
	return segs;
}
