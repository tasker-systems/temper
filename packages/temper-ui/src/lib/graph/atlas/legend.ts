// legend.ts
import {
	DOC_TYPE_HUES,
	docTypeHue,
	isAuthored,
	EDGE_COLORS,
	EDGE_KINDS,
	edgeStyle,
	type AtlasDocType
} from './palette';
import type { AtlasEdge } from '$lib/types/generated/graph_atlas';

export interface LegendSwatch {
	docType: string;
	hue: string;
	authored: boolean;
}

export interface EdgeKindSwatch {
	kind: AtlasEdge['edge_kind'];
	dash: string | null;
	color: string;
}

export interface EdgeColorSwatch {
	label: string;
	color: string;
}

export interface PolaritySwatch {
	label: string;
	marker: 'start' | 'end' | 'none';
	color: string;
}

export interface WeightSwatch {
	label: string;
	width: number;
	color: string;
}

export interface LegendModel {
	docTypes: LegendSwatch[];
	home: { label: string; filled: boolean }[];
	/** kind = line style (dash) — the axis the ScopeBar edge-kind filter operates on. */
	edgeKinds: EdgeKindSwatch[];
	/** color-by-label — structural/contradicts/derived (`derived_from` = dashed provenance bridge). */
	edgeColors: EdgeColorSwatch[];
	/** polarity = arrowhead; `near` is symmetric (no marker). */
	polarity: PolaritySwatch[];
	/** weight = stroke thickness. */
	weight: WeightSwatch[];
}

/** A neutral (label: null) synthetic edge for deriving legend samples through `edgeStyle` —
 *  never hand-roll a dash/color/marker here, always read it back off the real style fn. */
function legendEdge(overrides: Partial<AtlasEdge> = {}): AtlasEdge {
	return {
		id: 'legend',
		source: 'legend-a',
		target: 'legend-b',
		edge_kind: 'contains',
		polarity: 'forward',
		label: null,
		weight: 1,
		...overrides
	};
}

/** Derive the legend entirely from palette.ts — the single source of truth.
 *  A colocated test asserts docType + edge-kind coverage stays in sync (guards drift). */
export function legendModel(): LegendModel {
	const docTypes = (Object.keys(DOC_TYPE_HUES) as AtlasDocType[]).map((dt) => ({
		docType: dt,
		hue: docTypeHue(dt),
		authored: isAuthored(dt)
	}));

	const edgeKinds: EdgeKindSwatch[] = EDGE_KINDS.map((kind) => {
		const s = edgeStyle(legendEdge({ edge_kind: kind }));
		return { kind, dash: s.dash, color: s.color };
	});

	const edgeColors: EdgeColorSwatch[] = Object.entries(EDGE_COLORS).map(([label, color]) => ({
		label,
		color
	}));

	const forward = edgeStyle(legendEdge({ polarity: 'forward' }));
	const inverse = edgeStyle(legendEdge({ polarity: 'inverse' }));
	const near = edgeStyle(legendEdge({ edge_kind: 'near', polarity: 'forward' }));
	const polarity: PolaritySwatch[] = [
		{ label: 'forward', marker: forward.markerEnd ? 'end' : 'none', color: forward.color },
		{ label: 'inverse', marker: inverse.markerStart ? 'start' : 'none', color: inverse.color },
		{
			label: 'near (symmetric)',
			marker: near.markerStart || near.markerEnd ? 'end' : 'none',
			color: near.color
		}
	];

	const weight: WeightSwatch[] = [1, 3, 5].map((w) => {
		const s = edgeStyle(legendEdge({ weight: w }));
		return { label: `weight ${w}`, width: s.width, color: s.color };
	});

	return {
		docTypes,
		home: [
			{ label: 'cogmap-homed', filled: true },
			{ label: 'context-homed', filled: false }
		],
		edgeKinds,
		edgeColors,
		polarity,
		weight
	};
}
