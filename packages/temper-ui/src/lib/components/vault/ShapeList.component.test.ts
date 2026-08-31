import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { ShapeView } from '$lib/types/generated/data_artifact_shape';
import ShapeList from './ShapeList.svelte';

/**
 * What this component owes, per clause: which families a context governs, each one's
 * enforcement posture, and the schema viewable — with an `enforcing` posture findable but
 * never rendered as a refusal aimed at the reader.
 */

const shape = (over: Partial<ShapeView> = {}): ShapeView => ({
	shape_id: '01a00000000000000000000000000002',
	home_anchor_table: 'kb_contexts',
	home_anchor_id: 'ctx-1',
	kind_owner_table: 'kb_profiles',
	kind_owner_id: 'p-1',
	artifact_kind: 'measurement',
	schema: { type: 'object', required: ['ms'] },
	enforcement: 'advisory',
	shape_version: 1,
	is_folded: false,
	created: '2026-08-21T10:00:00Z',
	...over,
});

const getHead = (container: HTMLElement): HTMLElement => {
	const head = container.querySelector('.head');
	if (!(head instanceof HTMLElement)) throw new Error('no shape row head rendered');
	return head;
};

describe('ShapeList', () => {
	it('renders one row per governed family with its posture and chain depth', () => {
		const { container } = render(ShapeList, {
			shapes: [
				shape(),
				shape({
					shape_id: '01a00000000000000000000000000003',
					artifact_kind: 'extraction',
					shape_version: 3,
				}),
			],
		});

		const families = [...container.querySelectorAll('.family')].map((el) => el.textContent);
		expect(families).toEqual(['extraction', 'measurement']);
		expect(container.querySelector('.label')?.textContent).toContain('Governed families · 2');
		expect(container.textContent).toContain('v3');
	});

	it('makes an enforcing posture findable without alarming — advisory stays quiet', () => {
		const { container } = render(ShapeList, { shapes: [shape({ enforcement: 'enforcing' })] });
		expect(container.querySelector('.enforcing')?.textContent).toBe('enforcing');
		expect(container.querySelector('.advisory')).toBeNull();

		const quiet = render(ShapeList, { shapes: [shape()] });
		expect(quiet.container.querySelector('.advisory')?.textContent).toBe('advisory');
	});

	it('opens to the schema itself, whole', async () => {
		const schema = { type: 'object', required: ['ms'], properties: { ms: { type: 'number' } } };
		const { container } = render(ShapeList, { shapes: [shape({ schema })] });

		expect(container.querySelector('.schema')).toBeNull();
		await fireEvent.click(getHead(container));

		const shown = JSON.parse(container.querySelector('.schema')?.textContent ?? 'null');
		expect(shown).toEqual(schema);
		expect(getHead(container).getAttribute('aria-expanded')).toBe('true');
	});
});
