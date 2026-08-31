import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { ArtifactView } from '$lib/types/generated/data_artifact';
import ArtifactList from './ArtifactList.svelte';

/**
 * What this component owes, per clause: every artifact viewable whole (metadata plus the
 * content payload), folded distinct from live and still reachable, conformance shown when a
 * shape is in force and never faked when none is. The `data-testid` hooks the page test needs
 * are the section label and the rows' own text; these tests read the component directly.
 */

const artifact = (over: Partial<ArtifactView> = {}): ArtifactView => ({
	artifact_id: '01a00000000000000000000000000001',
	resource_id: 'res-1',
	kind_owner_table: 'kb_profiles',
	kind_owner_id: 'p-1',
	artifact_kind: 'measurement',
	intent: 'member',
	precedence: 0,
	content_hash: 'a'.repeat(64),
	content_bytes: 1536n,
	shape_state: 'never_declared',
	is_folded: false,
	created: '2026-08-20T10:00:00Z',
	content: { run: 2, ms: 412 },
	...over,
});

const getHead = (container: HTMLElement): HTMLElement => {
	const head = container.querySelector('.head');
	if (!(head instanceof HTMLElement)) throw new Error('no artifact row head rendered');
	return head;
};

describe('ArtifactList', () => {
	it('renders one row per artifact, newest first, with family · intent · size', () => {
		const { container } = render(ArtifactList, {
			artifacts: [
				artifact({ artifact_id: 'a-old', created: '2026-08-20T10:00:00Z' }),
				artifact({
					artifact_id: 'a-new',
					artifact_kind: 'extraction',
					created: '2026-08-21T10:00:00Z',
				}),
			],
		});

		const families = [...container.querySelectorAll('.head .family')].map((el) => el.textContent);
		expect(families).toEqual(['extraction', 'measurement']);
		expect(container.querySelector('.label')?.textContent).toContain('Data artifacts · 2');
		expect(container.textContent).toContain('member');
		expect(container.textContent).toContain('1.5 KB');
	});

	it('renders folded artifacts folded — dimmed, labeled, and still present', () => {
		const { container } = render(ArtifactList, {
			artifacts: [artifact({ is_folded: true })],
		});

		expect(container.querySelector('.artifact.folded')).not.toBeNull();
		expect(container.textContent).toContain('folded');
	});

	it('shows a conformance verdict when a shape is in force, and nothing when none is', () => {
		const governed = render(ArtifactList, {
			artifacts: [artifact({ shape_state: 'declared_not_satisfied' })],
		});
		expect(governed.container.textContent).toContain('declared not satisfied');
		expect(governed.container.querySelector('.shape.not-satisfied')).not.toBeNull();
		governed.unmount();

		const satisfied = render(ArtifactList, {
			artifacts: [artifact({ shape_state: 'declared_satisfied' })],
		});
		expect(satisfied.container.querySelector('.shape.satisfied')).not.toBeNull();
		satisfied.unmount();

		const unchecked = render(ArtifactList, {
			artifacts: [artifact({ shape_state: 'declared_not_yet_checked' })],
		});
		// Present and honest, but colored as neither verdict — unchecked never reads as checked.
		expect(
			unchecked.container.querySelector('.shape:not(.satisfied):not(.not-satisfied)'),
		).not.toBeNull();
		unchecked.unmount();

		const ungoverned = render(ArtifactList, {
			artifacts: [artifact({ shape_state: 'never_declared' })],
		});
		expect(ungoverned.container.textContent).not.toContain('never_declared');
		expect(ungoverned.container.textContent).not.toContain('never declared');
	});

	it('opens to the whole metadata and the whole content payload', async () => {
		const { container } = render(ArtifactList, { artifacts: [artifact()] });

		await fireEvent.click(getHead(container));

		expect(container.querySelector('.meta-table')?.textContent).toContain('a'.repeat(64));
		const content = container.querySelector('.content')?.textContent ?? '';
		expect(JSON.parse(content)).toEqual({ run: 2, ms: 412 });
	});

	it('says so when an artifact was committed with no content, rather than showing an empty box', async () => {
		const { container } = render(ArtifactList, {
			artifacts: [artifact({ content: null })],
		});

		await fireEvent.click(getHead(container));

		expect(container.querySelector('.content')).toBeNull();
		expect(container.textContent).toContain('no content');
	});

	it('exposes expand state to assistive tech via aria-expanded', async () => {
		const { container } = render(ArtifactList, { artifacts: [artifact()] });
		const head = getHead(container);

		expect(head.getAttribute('aria-expanded')).toBe('false');
		await fireEvent.click(head);
		expect(head.getAttribute('aria-expanded')).toBe('true');
		expect(screen.getByRole('button', { name: /measurement/ })).toBeDefined();
	});
});
