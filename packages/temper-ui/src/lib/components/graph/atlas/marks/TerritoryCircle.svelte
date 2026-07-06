<script lang="ts">
	import type { Territory } from '$lib/types/generated/graph_territory';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import { truncateLabel } from '$lib/graph/atlas/labels';

	interface Props {
		x: number;
		y: number;
		r: number;
		kind: Territory['kind'];
		label: string | null;
		/** Member count fallback: when `label` is null, renders "Region · N" instead of a blank circle. */
		memberCount?: number;
		onEnter?: () => void;
		/** Empty territory (no members) — render as a de-emphasized ghost (L3). Still drillable. */
		ghost?: boolean;
	}
	let { x, y, r, kind, label, memberCount = 0, onEnter, ghost = false }: Props = $props();

	// Region = warm-neutral tint; context = cool tint; cogmap = warm tint. Low-opacity
	// washes with a dashed hull outline, cartographic style.
	const tint = $derived(TERRITORY_TINTS[kind]);
	const radius = $derived(ghost ? r * 0.85 : r);
	const baseLabel = $derived(label ?? (memberCount > 0 ? `Region · ${memberCount}` : null));
	const displayLabel = $derived(ghost && baseLabel ? `${baseLabel} · empty` : baseLabel);
	// radius-proportional char budget so a long derived title fits the circle
	const shownLabel = $derived(
		displayLabel ? truncateLabel(displayLabel, Math.max(6, Math.floor(r / 4))) : null
	);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<g
	class="territory atlas-focusable"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	aria-label={displayLabel ?? kind}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	style={onEnter ? 'cursor:pointer' : undefined}
>
	<circle
		cx={x}
		cy={y}
		r={radius}
		fill={tint}
		fill-opacity={ghost ? '0.04' : '0.09'}
		stroke={tint}
		stroke-opacity={ghost ? '0.2' : '0.4'}
		stroke-width="1.5"
		stroke-dasharray={ghost ? '3 5' : '6 4'}
	/>
	{#if shownLabel}
		<text
			x={x}
			y={y}
			text-anchor="middle"
			fill={tint}
			fill-opacity={ghost ? '0.6' : '1'}
			font-size="11"
			font-weight="600"
			letter-spacing="1"
			style="text-transform:uppercase"
		>
			{shownLabel}
		</text>
	{/if}
	<circle class="focus-ring" cx={x} cy={y} r={radius + 4} stroke-width="2" />
</g>
