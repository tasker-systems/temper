import { stroke as strokeOf } from '@tasker-systems/temper-design/graph';
import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Edge from './Edge.svelte';

const draw = (
	over: Partial<{ stroke: ReturnType<typeof strokeOf>; relations: string[]; label: boolean }> = {},
) => {
	const { container } = render(Edge, {
		props: {
			x1: 0,
			y1: 0,
			x2: 100,
			y2: 40,
			heads: ['M100.00,40.00 L94.00,37.00 L95.00,43.00 Z'],
			stroke: strokeOf({ edgeKind: 'leads_to' }),
			relations: ['leads_to · weight not reported'],
			...over,
		},
	});
	return container;
};

describe('the edge mark', () => {
	it('draws a cross at the midpoint of a contradicts edge, painted by its role', () => {
		const c = draw({ stroke: strokeOf({ label: 'contradicts' }) });
		const cross = c.querySelector('.graph-cross');
		expect(cross).not.toBeNull();
		expect(cross?.getAttribute('transform')).toBe('translate(50 20)');
		expect(c.querySelector('.graph-edge')?.getAttribute('data-role')).toBe('contradicts');
	});

	it('draws no cross on any other edge', () => {
		expect(draw().querySelector('.graph-cross')).toBeNull();
		expect(
			draw({ stroke: strokeOf({ label: 'derived_from' }) }).querySelector('.graph-cross'),
		).toBeNull();
	});

	it('draws its heads as shapes, so the canvas holds no marker ids', () => {
		const c = draw();
		expect(c.querySelectorAll('.graph-head')).toHaveLength(1);
		expect(c.querySelector('[marker-end], [marker-start]')).toBeNull();
	});

	it('names every relation on the pair when hovered, not only the first', () => {
		const relations = ['leads_to · weight 0.40', 'contradicts · weight not reported'];
		const c = draw({ relations, label: true });
		expect([...c.querySelectorAll('text')].map((t) => t.textContent)).toEqual(relations);
		expect(c.querySelector('.graph-edge')?.getAttribute('aria-label')).toBe(relations.join('; '));
		expect(draw({ relations }).querySelectorAll('text')).toHaveLength(0);
	});
});
