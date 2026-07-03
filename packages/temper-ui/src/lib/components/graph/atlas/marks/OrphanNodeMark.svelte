<script lang="ts">
	import { docTypeHue } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		title: string;
		docType: string | null;
	}
	let { x, y, title, docType }: Props = $props();

	const color = $derived(docTypeHue(docType));
</script>

<g class="orphan">
	<circle cx={x} cy={y} r="5" fill={color} />
	<text x={x + 10} y={y + 3} fill="#c7d0da" font-size="11">{title}</text>
</g>
