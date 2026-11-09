<script lang="ts">
	import type { ReachVia } from '$lib/types/generated/reach';

	/**
	 * How the reader reaches the object, as its single-object read answered it. Only
	 * `public_within` renders: the reader reads this because a team opened its own contexts to the
	 * reader's part of the team tree, so they may read and reference it, never act on it. The
	 * marker names no team and no person, because the read does not.
	 *
	 * `membership`, absent (an older server, a list row) and `null` all render nothing: nothing
	 * about the reader's route is claimed unless the read said so.
	 */
	let { via }: { via: ReachVia | null | undefined } = $props();
</script>

{#if via === 'public_within'}
	<span
		class="marker"
		title="You read this because its team opened its own contexts to your part of the team tree. You can read it and reference it from your own places; writing, sharing or subscribing to it needs membership."
		>◇ REACHED THROUGH PUBLIC WITHIN · READ ONLY</span
	>
{/if}

<style>
	.marker {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: 0.08em;
		padding: 2px 7px;
		border-radius: 2px;
		border: 1px dashed var(--tp-rule-strong);
		color: var(--tp-text-muted);
		background: var(--tp-surface-raised);
	}
</style>
