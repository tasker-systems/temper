<script lang="ts">
	import { legendModel } from '$lib/graph/atlas/legend';

	const m = legendModel();
	let open = $state(true);
</script>

<div class="legend" data-testid="atlas-legend">
	<button type="button" class="head" onclick={() => (open = !open)}>
		▦ Legend {open ? '▾' : '▸'}
	</button>
	{#if open}
		<div class="sec">
			<div class="lbl">DOC TYPE</div>
			{#each m.docTypes as d (d.docType)}
				<div class="row">
					<span
						class="sw"
						style="background:{d.authored ? d.hue : 'transparent'}; border-color:{d.hue}"
					></span>{d.docType}
				</div>
			{/each}
		</div>
		<div class="sec">
			<div class="lbl">HOME</div>
			{#each m.home as h (h.label)}
				<div class="row">
					<span
						class="sw"
						style="background:{h.filled ? '#c9d1d9' : 'transparent'}; border-color:#c9d1d9"
					></span>{h.label}
				</div>
			{/each}
		</div>
		<div class="sec">
			<div class="lbl">EDGES</div>
			{#each m.edges as e (e.label)}
				<div class="row"><span class="line" style="background:{e.color}"></span>{e.label}</div>
			{/each}
		</div>
	{/if}
</div>

<style>
	.legend {
		padding: 8px 12px;
		font-size: 12px;
		color: var(--color-quiet-ink, #c9d1d9);
	}
	.head {
		background: none;
		border: 0;
		color: var(--color-quiet-ink, #c9d1d9);
		cursor: pointer;
		font-size: 12px;
		padding: 4px 0;
	}
	.sec {
		padding: 6px 0;
	}
	.lbl {
		font: 8.5px monospace;
		letter-spacing: 0.2em;
		color: #6a727e;
		margin-bottom: 4px;
	}
	.row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 2px 0;
	}
	.sw {
		width: 10px;
		height: 10px;
		border-radius: 50%;
		border: 2px solid;
		flex: 0 0 auto;
	}
	.line {
		width: 16px;
		height: 2px;
		flex: 0 0 auto;
	}
</style>
