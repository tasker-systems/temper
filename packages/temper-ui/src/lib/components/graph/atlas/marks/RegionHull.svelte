<script lang="ts">
	interface Props {
		d: string;
		label: string | null;
		tint: string;
	}
	let { d, label, tint }: Props = $props();
</script>

<g class="region-hull">
	<path {d} fill={tint} fill-opacity="0.05" stroke={tint} stroke-opacity="0.4" stroke-width="1.5" stroke-dasharray="7 5" />
	{#if label}
		<text fill={tint} font-size="11" font-weight="600" letter-spacing="1" style="text-transform:uppercase" dx="6" dy="-6">
			<textPath href="">{label}</textPath>
		</text>
	{/if}
</g>
