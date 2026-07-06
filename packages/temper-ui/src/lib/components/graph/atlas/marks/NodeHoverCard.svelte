<script lang="ts">
	/**
	 * Standard node hover card (Beat 2b N2). An SVG `<foreignObject>` floating
	 * above the node so HTML/CSS (serif title, line-clamp, backdrop-filter) can
	 * render inside the canvas. Doctype pill + edge-count + serif title + a
	 * 2-line clamped excerpt snippet + a muted "click → open in rail" hint —
	 * the "Standard" density chosen in brainstorming (see the Task 6 brief).
	 * Purely decorative: pointer-events are disabled so it never steals the
	 * hover/click that `NodeChip`'s `<g>` handles.
	 */
	import { docTypeHue } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		edges: number;
		excerpt: string | null;
	}
	let { x, y, r, title, docType, edges, excerpt }: Props = $props();

	const CARD_WIDTH = 220;
	const CARD_HEIGHT = 128;
	const GAP = 12;

	const hue = $derived(docTypeHue(docType));
	const left = $derived(x - CARD_WIDTH / 2);
	const top = $derived(y - r - GAP - CARD_HEIGHT);
</script>

<foreignObject
	x={left}
	y={top}
	width={CARD_WIDTH}
	height={CARD_HEIGHT}
	style="overflow: visible; pointer-events: none;"
>
	<!-- Bottom-anchor the card within the fixed-height box: a short card (short or
	     absent excerpt) still lands its bottom edge — and the arrow — at y - r - GAP,
	     right above the node, instead of floating up against the reserved height. -->
	<div class="hc-anchor">
		<div class="hover-card" style={`--hue: ${hue};`}>
			<div class="row">
				<span class="pill">{docType ?? '—'}</span>
				<span class="chip">⌷ {edges} {edges === 1 ? 'edge' : 'edges'}</span>
			</div>
			<div class="ctitle">{title}</div>
			{#if excerpt}
				<p class="snip">{excerpt}</p>
			{/if}
			<div class="hint">click → open in rail</div>
		</div>
	</div>
</foreignObject>

<style>
	.hc-anchor {
		height: 100%;
		display: flex;
		align-items: flex-end;
	}
	.hover-card {
		position: relative;
		box-sizing: border-box;
		width: 100%;
		background: rgba(20, 23, 29, 0.97);
		backdrop-filter: blur(8px);
		border: 1px solid color-mix(in srgb, var(--hue) 34%, transparent);
		border-radius: 9px;
		box-shadow: 0 8px 30px rgba(0, 0, 0, 0.55);
		color: #c9d1d9;
		text-align: left;
		padding: 10px 12px;
		font: 12px/1.45 system-ui, sans-serif;
	}
	.hover-card::after {
		content: '';
		position: absolute;
		left: 50%;
		bottom: -6px;
		transform: translateX(-50%) rotate(45deg);
		width: 10px;
		height: 10px;
		background: rgba(20, 23, 29, 0.97);
		border-right: 1px solid color-mix(in srgb, var(--hue) 34%, transparent);
		border-bottom: 1px solid color-mix(in srgb, var(--hue) 34%, transparent);
	}
	.row {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	.pill {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font: 8.5px monospace;
		letter-spacing: 0.14em;
		text-transform: uppercase;
		color: var(--hue);
	}
	.pill::before {
		content: '';
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--hue);
	}
	.chip {
		font: 8.5px monospace;
		letter-spacing: 0.1em;
		color: #8a929e;
	}
	.ctitle {
		font-family: Georgia, serif;
		font-size: 15px;
		color: var(--hue);
		margin: 4px 0 5px;
		line-height: 1.2;
	}
	.snip {
		margin: 0;
		color: #9aa3b0;
		font-size: 11.5px;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}
	.hint {
		margin-top: 7px;
		font: 8px monospace;
		letter-spacing: 0.16em;
		color: #5a6270;
		text-transform: uppercase;
	}
</style>
