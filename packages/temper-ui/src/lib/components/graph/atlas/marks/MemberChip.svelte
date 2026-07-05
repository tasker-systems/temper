<script lang="ts">
	import { docTypeHue, isAuthored, CANVAS_BG } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		onEnter: () => void;
	}
	let { x, y, r, title, docType, onEnter }: Props = $props();

	const color = $derived(docTypeHue(docType));
	const filled = $derived(isAuthored(docType));
	// The on-canvas label is truncated to a budget that scales with the chip size,
	// so adjacent members' labels don't collide into an illegible run; the full
	// title is always available via the native SVG tooltip on hover.
	const maxChars = $derived(Math.max(6, Math.round(r / 3)));
	const label = $derived(
		title.length > maxChars ? `${title.slice(0, maxChars - 1).trimEnd()}…` : title
	);
	const fontSize = $derived(r < 22 ? 9 : 10);
</script>

<g
	class="member-chip atlas-focusable"
	role="button"
	tabindex="0"
	aria-label={title}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter()}
	style="cursor:pointer"
>
	<title>{title}</title>
	{#if filled}
		<circle cx={x} cy={y} {r} fill={color} />
	{:else}
		<circle cx={x} cy={y} {r} fill={CANVAS_BG} stroke={color} stroke-width="2.2" />
	{/if}
	<text x={x} y={y + r + 11} text-anchor="middle" fill="#c7d0da" font-size={fontSize}>{label}</text>
	<circle class="focus-ring" cx={x} cy={y} r={r + 4} stroke-width="2" />
</g>
