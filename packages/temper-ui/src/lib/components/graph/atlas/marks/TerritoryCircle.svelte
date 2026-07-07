<script lang="ts">
	import type { Territory } from '$lib/types/generated/graph_territory';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import { wrapLabel, fieldStyle } from '$lib/graph/atlas/labels';
	import RegionHoverCard from './RegionHoverCard.svelte';

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
		/** Salience-gated labeling: only the salient regions draw an in-panorama label;
		 * un-labeled circles still reveal the full title in <title>. */
		showLabel?: boolean;
		/** Salience-driven field intensity (0..1): brighter fill + stronger glow when salient. */
		intensity?: number;
		/** Region metadata for the hover card (regions only). */
		salience?: number | null;
		coherence?: number | null;
	}
	let {
		x, y, r, kind, label, memberCount = 0, onEnter,
		ghost = false, showLabel = true, intensity = 0.5,
		salience = null, coherence = null
	}: Props = $props();

	let hovered = $state(false);
	const isRegion = $derived(kind === 'region' && !!onEnter);

	const tint = $derived(TERRITORY_TINTS[kind]);
	const radius = $derived(ghost ? r * 0.85 : r);
	const style = $derived(fieldStyle(intensity, ghost));
	const glow = $derived(style.glowPx > 0 ? `drop-shadow(0 0 ${style.glowPx}px ${tint})` : 'none');
	const baseLabel = $derived(label ?? (memberCount > 0 ? `Region · ${memberCount}` : null));
	const displayLabel = $derived(ghost && baseLabel ? `${baseLabel} · empty` : baseLabel);
	// Force-separated layout: label sits BELOW the circle, mixed-case, ≤ 2 lines, width-aware.
	const perLineCap = $derived(Math.max(14, Math.floor(r / 2.4)));
	const lines = $derived(displayLabel ? wrapLabel(displayLabel, perLineCap) : []);
	const FONT = 11;
	const LINE_H = 12;

	// Accessible name: for a region, fold the hover-card metadata (resources · salience ·
	// coherence) into the label so a keyboard/screen-reader user gets the non-spatial
	// equivalent of the field — the hover card itself is pointer-events:none decoration.
	const ariaLabel = $derived(
		isRegion && displayLabel
			? `${displayLabel} — ${memberCount} ${memberCount === 1 ? 'resource' : 'resources'}` +
				(salience != null ? `, salience ${salience.toFixed(2)}` : '') +
				(coherence != null ? `, coherence ${Math.round(coherence * 100)}%` : '')
			: (displayLabel ?? kind)
	);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<g
	class="territory atlas-focusable"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	aria-label={ariaLabel}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	onmouseenter={() => (hovered = true)}
	onmouseleave={() => (hovered = false)}
	onfocus={() => (hovered = true)}
	onblur={() => (hovered = false)}
	style={onEnter ? 'cursor:pointer' : undefined}
>
	<!-- Full title on hover/focus for every territory, labeled or not. -->
	{#if displayLabel}<title>{displayLabel}</title>{/if}
	<circle
		cx={x}
		cy={y}
		r={radius}
		fill={tint}
		fill-opacity={style.fillOpacity}
		stroke={tint}
		stroke-opacity={style.strokeOpacity}
		stroke-width="1.5"
		stroke-dasharray={ghost ? '3 5' : '6 4'}
		style={`filter:${glow}`}
	/>
	{#if showLabel && lines.length > 0}
		<text
			x={x}
			y={y + radius + 11}
			text-anchor="middle"
			fill={tint}
			fill-opacity={ghost ? '0.6' : '1'}
			font-size={FONT}
			font-weight="600"
		>
			{#each lines as line, i (i)}<tspan x={x} dy={i === 0 ? 0 : LINE_H}>{line}</tspan>{/each}
		</text>
	{/if}
	<circle class="focus-ring" cx={x} cy={y} r={radius + 4} stroke-width="2" />
	{#if hovered && isRegion}
		<RegionHoverCard {x} {y} r={radius} label={displayLabel} {memberCount} {salience} {coherence} />
	{/if}
</g>
