<script lang="ts">
	/**
	 * The bound declaration — always on screen, plain, never dismissible.
	 *
	 * **Chrome, not a warning.** It is present whether or not the view is partial, so *complete*
	 * is something the reader is TOLD rather than something they infer from silence. The cheaper
	 * design — show a marker when something was dropped — makes the absence of a marker the
	 * signal, and a bug that suppresses it invisible.
	 *
	 * Every string comes from `renderBoundLine`, which is pure and tested. This component's whole
	 * job is to put that sentence somewhere it cannot be dismissed.
	 */
	import type { BoundDeclaration } from '$lib/graph/bound';
	import { renderBoundLine } from '$lib/graph/bound';

	let { bound }: { bound: BoundDeclaration } = $props();

	const line = $derived(renderBoundLine(bound));
</script>

<!-- `role="status"` and not `alert`: this is a standing description of the view, not an event. -->
<p class="bound" role="status" data-testid="bound-line">{line}</p>

<style>
	.bound {
		margin: 0;
		padding: 8px 14px;
		border-top: 1px solid rgba(255, 255, 255, 0.07);
		background: rgba(255, 255, 255, 0.02);
		color: #98a2b0;
		font:
			12px/1.5 ui-monospace,
			SFMono-Regular,
			Menlo,
			monospace;
		font-variant-numeric: tabular-nums;
		letter-spacing: 0.01em;
	}
</style>
