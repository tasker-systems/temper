import { describe, expect, it } from 'vitest';
import {
	buildStylesheet,
	edgeStrokeDasharray,
	EMPHASIS_TRANSITION_MS,
	nodeColor,
	nodeFontSize,
	nodeHeightPx,
	nodeWidthPx,
	SESSION_GLYPH_COLOR
} from './styling';
import type { GraphNode } from '../types/generated/graph';

function node(partial: Partial<GraphNode>): GraphNode {
	return {
		id: '00000000-0000-0000-0000-000000000000',
		slug: 'x',
		title: 'X',
		doc_type: 'research',
		aggregator: false,
		edge_count: 0,
		session_count: 0,
		excerpt: null,
		stage: null,
		...partial
	};
}

describe('nodeColor', () => {
	it('maps every known doctype to a distinct color', () => {
		const colors = new Set(
			['research', 'task', 'session', 'concept', 'goal', 'decision', 'memory'].map(nodeColor)
		);
		expect(colors.size).toBe(7);
	});

	it('returns a gray fallback for unknown doc types', () => {
		expect(nodeColor('unknown')).toMatch(/^#[0-9a-f]{6}$/i);
	});
});

describe('SESSION_GLYPH_COLOR', () => {
	it('is the session hue (green) so the ⌊N⌋ glyph reads as an annotation', () => {
		expect(SESSION_GLYPH_COLOR).toBe(nodeColor('session'));
	});
});

describe('nodeFontSize', () => {
	it('aggregators render at 19px', () => {
		expect(nodeFontSize(node({ aggregator: true }))).toBe(19);
	});

	it('participants with <10 edges render at 13px', () => {
		expect(nodeFontSize(node({ edge_count: 5 }))).toBe(13);
	});

	it('highly-connected participants (>=10 edges) scale up to 14px', () => {
		expect(nodeFontSize(node({ edge_count: 10 }))).toBe(14);
		expect(nodeFontSize(node({ edge_count: 50 }))).toBe(14);
	});
});

describe('nodeWidthPx', () => {
	it('aggregators get a 180px floor + padding', () => {
		expect(nodeWidthPx(node({ aggregator: true }), 3)).toBe(220);
	});

	it('participants tight-fit around the label with a 60px floor', () => {
		expect(nodeWidthPx(node({ aggregator: false }), 2)).toBe(60);
		expect(nodeWidthPx(node({ aggregator: false }), 20)).toBe(160);
	});
});

describe('nodeHeightPx', () => {
	it('aggregators 70, participants 22', () => {
		expect(nodeHeightPx(node({ aggregator: true }))).toBe(70);
		expect(nodeHeightPx(node({ aggregator: false }))).toBe(22);
	});
});

describe('buildStylesheet', () => {
	it('has the base node rule first', () => {
		const sheet = buildStylesheet();
		expect(sheet[0].selector).toBe('node');
	});

	it('has one selector per edge type', () => {
		const sheet = buildStylesheet();
		const edgeTypeSelectors = sheet
			.map((r) => r.selector)
			.filter((s) => s.startsWith('edge.etype-'));
		const types: string[] = [
			'depends_on',
			'extends',
			'parent_of',
			'derived_from',
			'preceded_by',
			'relates_to',
			'references'
		];
		for (const t of types) {
			expect(edgeTypeSelectors).toContain(`edge.etype-${t}`);
		}
	});

	it('aggregator rule wraps label with a translucent ellipse', () => {
		const aggRule = buildStylesheet().find((r) => r.selector === 'node.aggregator');
		expect(aggRule?.style['shape']).toBe('ellipse');
		expect(aggRule?.style['background-opacity'] as number).toBeLessThan(1);
	});
});

describe('emphasis classes', () => {
	it('exposes a 180ms transition budget that matches kg-handoff', () => {
		expect(EMPHASIS_TRANSITION_MS).toBe(180);
	});

	it('base node rule animates opacity/text-opacity at the transition budget', () => {
		const base = buildStylesheet().find((r) => r.selector === 'node');
		expect(base?.style['transition-duration']).toBe(EMPHASIS_TRANSITION_MS);
		expect(String(base?.style['transition-property'])).toContain('opacity');
		expect(String(base?.style['transition-property'])).toContain('text-opacity');
	});

	it('base edge rule animates opacity/width/line-color at the transition budget', () => {
		const base = buildStylesheet().find((r) => r.selector === 'edge');
		expect(base?.style['transition-duration']).toBe(EMPHASIS_TRANSITION_MS);
		const props = String(base?.style['transition-property']);
		expect(props).toContain('opacity');
		expect(props).toContain('width');
		expect(props).toContain('line-color');
	});

	it('node.hovered wraps the label in a low-alpha wash of its own hue', () => {
		const rule = buildStylesheet().find((r) => r.selector === 'node.hovered');
		expect(rule?.style['text-background-color']).toBe('data(fill)');
		expect(rule?.style['text-background-opacity'] as number).toBeLessThan(1);
	});

	it('node.dim fades non-neighbor nodes toward 0.35 opacity', () => {
		const rule = buildStylesheet().find((r) => r.selector === 'node.dim');
		expect(rule?.style.opacity).toBe(0.35);
	});

	it('edge.incident lifts to 1.1px in the source hue at full opacity', () => {
		const rule = buildStylesheet().find((r) => r.selector === 'edge.incident');
		expect(rule?.style.width).toBe(1.1);
		expect(rule?.style['line-color']).toBe('data(sourceFill)');
		expect(rule?.style.opacity).toBe(1);
	});

	it('edge.quiet fades unrelated edges to 0.03 alpha', () => {
		const rule = buildStylesheet().find((r) => r.selector === 'edge.quiet');
		expect(rule?.style.opacity).toBe(0.03);
	});
});

describe('zoom-tier selectors', () => {
	it('hides participant labels and renders tick marks at the overview tier', () => {
		const rule = buildStylesheet().find(
			(r) => r.selector === 'node.tier-overview.participant'
		);
		expect(rule, 'participant overview rule exists').toBeDefined();
		expect(rule?.style['text-opacity']).toBe(0);
		expect(rule?.style['background-opacity']).toBe(1);
		expect(rule?.style.shape).toBe('rectangle');
	});

	it('does NOT add an overview-tier rule for aggregators (they stay labeled)', () => {
		const sheet = buildStylesheet();
		const hit = sheet.find((r) => r.selector === 'node.tier-overview.aggregator');
		expect(hit, 'aggregators should have no overview override').toBeUndefined();
	});

	it('swaps the detail-tier label to the multiline labelWithDate when dateStrip exists', () => {
		const rule = buildStylesheet().find(
			(r) => r.selector === 'node.tier-detail[dateStrip]'
		);
		expect(rule, 'detail dateStrip rule exists').toBeDefined();
		expect(rule?.style.label).toBe('data(labelWithDate)');
		expect(rule?.style['text-wrap']).toBe('wrap');
	});

	it('stacks the stage tag under tasks at the detail tier', () => {
		const rule = buildStylesheet().find(
			(r) => r.selector === 'node.tier-detail.type-task[stage]'
		);
		expect(rule, 'detail task-stage rule exists').toBeDefined();
		expect(rule?.style.label).toBe('data(labelWithDateAndStage)');
		expect(rule?.style['text-wrap']).toBe('wrap');
	});

	it('has NO tier-mid rule (mid IS the steady state)', () => {
		const hits = buildStylesheet().filter((r) => r.selector.includes('tier-mid'));
		expect(hits).toEqual([]);
	});
});

describe('edgeStrokeDasharray', () => {
	it('returns a nonempty dash for dashed edge types', () => {
		expect(edgeStrokeDasharray('preceded_by')).not.toBe('');
		expect(edgeStrokeDasharray('relates_to')).not.toBe('');
		expect(edgeStrokeDasharray('references')).not.toBe('');
	});

	it('returns empty (solid) for solid edge types', () => {
		expect(edgeStrokeDasharray('depends_on')).toBe('');
		expect(edgeStrokeDasharray('extends')).toBe('');
		expect(edgeStrokeDasharray('parent_of')).toBe('');
	});
});
