<script lang="ts">
	import { docTypeHue } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r?: number;
		title: string;
		docType: string | null;
		onEnter?: () => void;
	}
	let { x, y, r = 5, title, docType, onEnter }: Props = $props();

	const color = $derived(docTypeHue(docType));
	let hovered = $state(false);
</script>

<g
	class="orphan"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	aria-label={title}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	onmouseenter={() => (hovered = true)}
	onmouseleave={() => (hovered = false)}
	style={onEnter ? 'cursor:pointer' : undefined}
>
	<circle cx={x} cy={y} {r} fill={color} />
	{#if hovered}
		<text x={x + r + 4} y={y + 3} fill="#e6edf5" font-size="10">{title}</text>
	{/if}
</g>
