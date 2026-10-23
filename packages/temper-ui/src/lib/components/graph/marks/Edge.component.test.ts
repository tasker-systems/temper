import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { EdgeMark } from '$lib/graph/palette';
import Edge from './Edge.svelte';

const edge = (over: Partial<EdgeMark> = {}): EdgeMark => ({
	edge_kind: 'leads_to',
	polarity: 'forward',
	label: null,
	...over,
});

const draw = (e: EdgeMark) => {
	const { container } = render(Edge, { props: { x1: 0, y1: 0, x2: 100, y2: 40, edge: e } });
	return container;
};

describe('the edge mark', () => {
	it('draws a cross at the midpoint of a contradicts edge, on danger', () => {
		const cross = draw(edge({ label: 'contradicts' })).querySelector('.contradicts-cross');
		expect(cross).not.toBeNull();
		expect(cross?.getAttribute('transform')).toBe('translate(50 20)');
		expect(cross?.getAttribute('style')).toContain('var(--tp-danger)');
	});

	it('draws no cross on any other edge', () => {
		expect(draw(edge()).querySelector('.contradicts-cross')).toBeNull();
		expect(draw(edge({ label: 'derived_from' })).querySelector('.contradicts-cross')).toBeNull();
	});
});
