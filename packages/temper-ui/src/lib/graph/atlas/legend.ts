// legend.ts
import { DOC_TYPE_HUES, docTypeHue, isAuthored, EDGE_COLORS, type AtlasDocType } from './palette';

export interface LegendSwatch {
	docType: string;
	hue: string;
	authored: boolean;
}
export interface LegendModel {
	docTypes: LegendSwatch[];
	home: { label: string; filled: boolean }[];
	edges: { label: string; color: string }[];
}

/** Derive the legend entirely from palette.ts — the single source of truth.
 *  A colocated test asserts docType coverage stays in sync (guards drift). */
export function legendModel(): LegendModel {
	const docTypes = (Object.keys(DOC_TYPE_HUES) as AtlasDocType[]).map((dt) => ({
		docType: dt,
		hue: docTypeHue(dt),
		authored: isAuthored(dt)
	}));
	return {
		docTypes,
		home: [
			{ label: 'cogmap-homed', filled: true },
			{ label: 'context-homed', filled: false }
		],
		edges: Object.entries(EDGE_COLORS).map(([label, color]) => ({ label, color }))
	};
}
