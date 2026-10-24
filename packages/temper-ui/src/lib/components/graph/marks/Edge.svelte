<script lang="ts">
	import type { StrokeRole } from '@tasker-systems/temper-design/graph';

	/**
	 * One line per PAIR of nodes, carrying every relation between them
	 * (`@tasker-systems/temper-design/graph`). The geometry and the stroke are the package's: the
	 * line is trimmed to the marks' rims and its heads are shapes, so a head is never under the mark
	 * it points at and the canvas holds no marker ids. Its colour is the contract's `graph.css`
	 * recipe, by role — this mark paints none of its own.
	 */
	interface Props {
		x1: number;
		y1: number;
		x2: number;
		y2: number;
		heads: string[];
		stroke: { role: StrokeRole; dash: string | null; width: number; cross: boolean };
		/** Each relation on the pair, in words: what a hover names. */
		relations: string[];
		label?: boolean;
		onSelect?: () => void;
	}
	let { x1, y1, x2, y2, heads, stroke, relations, label = false, onSelect }: Props = $props();

	const midX = $derived((x1 + x2) / 2);
	const midY = $derived((y1 + y2) / 2);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<g
	class="edge graph-edge atlas-focusable"
	data-role={stroke.role}
	role={onSelect ? 'button' : undefined}
	tabindex={onSelect ? 0 : undefined}
	aria-label={relations.join('; ')}
	onclick={onSelect}
	onkeydown={(e) => e.key === 'Enter' && onSelect?.()}
	style={onSelect ? 'cursor:pointer' : undefined}
>
	<line {x1} {y1} {x2} {y2} stroke="transparent" stroke-width="10" />
	<!-- Focus halo behind the visible stroke so the edge colour stays on top. -->
	<line class="focus-ring" {x1} {y1} {x2} {y2} stroke-width={stroke.width + 5} stroke-linecap="round" />
	<line
		class="graph-line"
		{x1}
		{y1}
		{x2}
		{y2}
		stroke-width={stroke.width}
		stroke-dasharray={stroke.dash ?? undefined}
	/>
	{#each heads as d (d)}
		<path class="graph-head" {d} />
	{/each}
	{#if stroke.cross}
		<!-- contradicts' channel beyond colour (the package's `Stroke.cross`) -->
		<g class="graph-cross" transform="translate({midX} {midY})">
			<line x1="-3.5" y1="-3.5" x2="3.5" y2="3.5" />
			<line x1="-3.5" y1="3.5" x2="3.5" y2="-3.5" />
		</g>
	{/if}
	{#if label}
		{#each relations as r, i (i)}
			<text
				x={midX}
				y={midY - 3 - (relations.length - 1 - i) * 11}
				text-anchor="middle"
				style="fill: var(--tp-accent)"
				font-size="9">{r}</text
			>
		{/each}
	{/if}
</g>
