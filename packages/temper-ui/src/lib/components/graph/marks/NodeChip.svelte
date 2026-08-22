<script lang="ts">
	/**
	 * A single mark. **It no longer draws its own hover card**, and that is a paint-order fix
	 * rather than a refactor.
	 *
	 * `[found on production — 2026-08-22]` The card used to render inside this component's `<g>`,
	 * which is inside the canvas's node loop — so every mark drawn after it painted over it.
	 * **SVG has no `z-index`; stacking is document order**, which is why the symptom was reported
	 * for years as a z-index problem and why setting one never helped. The canvas draws the card
	 * once, after every mark; this component only reports that the pointer is over it.
	 */
	import { docTypeHue, CANVAS_BG } from '$lib/graph/palette';
	import { truncateLabel } from '$lib/graph/labels';
	import { nodeMarkShape } from '$lib/graph/marks';

	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		home: 'context' | 'cogmap';
		/**
		 * Draw the arm ring around this mark.
		 *
		 * Named for what it DOES, not for the arm it used to stand for: whether an arm is worth a
		 * ring is a judgment about the whole view — a canvas whose marks all share one arm has no
		 * contrast to encode — and the mark cannot make it. Called `seed` until `[2026-08-21]`,
		 * when a view that ringed all 130 of its marks made the old name read as a fact about the
		 * node rather than a decision about the screen.
		 */
		ringed?: boolean;
		anchored?: boolean;
		/** Visual-only doc-type filter dimming (Task 8) — never affects the read. */
		dim?: boolean;
		/**
		 * Whether the pointer is over this mark — **owned by the canvas**, not by this component.
		 *
		 * One source of truth: the canvas needs it to decide which card to draw last, and this
		 * component needs it to suppress its own small label. Two copies of the same fact, updated
		 * from the same two events, is the drift worth avoiding.
		 */
		hovered?: boolean;
		onHover?: (hovered: boolean) => void;
		onEnter?: () => void;
	}
	let {
		x,
		y,
		r,
		title,
		docType,
		home,
		ringed = false,
		anchored = false,
		dim = false,
		hovered = false,
		onHover,
		onEnter
	}: Props = $props();

	const color = $derived(docTypeHue(docType));
	const shape = $derived(nodeMarkShape(home));
	const style = $derived(`${onEnter ? 'cursor:pointer;' : ''}opacity:${dim ? 0.15 : 1};`);
	// The small anchored label is a lightweight always-on cue; the hover card
	// (N2) is richer and takes over the moment the pointer is over the node,
	// anchored or not.
	const showLabel = $derived(anchored && !hovered);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<g
	class="node-chip atlas-focusable"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	aria-label={title}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	onmouseenter={() => onHover?.(true)}
	onmouseleave={() => onHover?.(false)}
	{style}
>
	{#if ringed}
		<!-- Classed so a test can assert its ABSENCE: a channel that fires on every mark encodes
		     nothing, and nothing in the type system could have caught that. Not a mark — the
		     vocabulary test counts `<g class>`, and this is a bare circle inside one. -->
		<circle class="arm-ring" cx={x} cy={y} r={r + 6} fill="none" stroke="#cfd6e2" stroke-width="1.5" />
	{/if}
	{#if shape === 'circle'}
		<!-- cogmap facet = an idea in the map -->
		<circle cx={x} cy={y} {r} fill={color} />
	{:else}
		<!-- context resource = the work it was derived_from — a document-square.
		     Shape carries the axis; color still carries doc-type. -->
		<rect
			x={x - r}
			y={y - r}
			width={2 * r}
			height={2 * r}
			rx={Math.max(2, r * 0.32)}
			fill={color}
			stroke={CANVAS_BG}
			stroke-width="1.5"
		/>
	{/if}
	{#if showLabel}
		<text x={x} y={y + r + 13} text-anchor="middle" fill="#c7d0da" font-size="10">{truncateLabel(title, 22)}</text>
	{/if}
	<circle class="focus-ring" cx={x} cy={y} r={r + 4} stroke-width="2" />
</g>
