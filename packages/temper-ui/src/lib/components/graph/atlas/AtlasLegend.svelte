<script lang="ts">
	import { legendModel } from '$lib/graph/atlas/legend';

	const m = legendModel();
	let open = $state(false);
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
			<div class="lbl">EDGE KIND</div>
			{#each m.edgeKinds as k (k.kind)}
				<div class="row">
					<svg class="sample" width="24" height="8" aria-hidden="true">
						<line x1="1" y1="4" x2="23" y2="4" stroke={k.color} stroke-width="2" stroke-dasharray={k.dash ?? undefined} />
					</svg>{k.kind}
				</div>
			{/each}
		</div>
		<div class="sec">
			<div class="lbl">EDGE COLOR</div>
			{#each m.edgeColors as e (e.label)}
				<div class="row"><span class="line" style="background:{e.color}"></span>{e.label}</div>
			{/each}
		</div>
		<div class="sec">
			<div class="lbl">POLARITY</div>
			{#each m.polarity as p (p.label)}
				<div class="row">
					<svg class="sample" width="24" height="8" aria-hidden="true">
						<defs>
							<marker id="legend-arrow-end-{p.label}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
								<path d="M0,0 L10,5 L0,10 z" fill={p.color} />
							</marker>
							<marker id="legend-arrow-start-{p.label}" viewBox="0 0 10 10" refX="1" refY="5" markerWidth="6" markerHeight="6" orient="auto">
								<path d="M10,0 L0,5 L10,10 z" fill={p.color} />
							</marker>
						</defs>
						<line
							x1="1"
							y1="4"
							x2="23"
							y2="4"
							stroke={p.color}
							stroke-width="2"
							marker-end={p.marker === 'end' ? `url(#legend-arrow-end-${p.label})` : undefined}
							marker-start={p.marker === 'start' ? `url(#legend-arrow-start-${p.label})` : undefined}
						/>
					</svg>{p.label}
				</div>
			{/each}
		</div>
		<div class="sec">
			<div class="lbl">WEIGHT</div>
			{#each m.weight as w (w.label)}
				<div class="row">
					<svg class="sample" width="24" height="8" aria-hidden="true">
						<line x1="1" y1="4" x2="23" y2="4" stroke={w.color} stroke-width={w.width} />
					</svg>{w.label}
				</div>
			{/each}
		</div>
		<div class="sec">
			<div class="lbl">BRIDGE</div>
			<div class="row"><span class="line thick" style="background:#e8cf8f"></span>shared-edge count · thicker = stronger</div>
		</div>
	{/if}
</div>

<style>
	.legend {
		padding: 6px 12px;
		font-size: 12px;
		color: var(--color-quiet-ink, #c9d1d9);
		display: flex;
		align-items: flex-start;
		gap: 18px;
		flex-wrap: wrap;
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
		padding: 2px 0;
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
	.line.thick {
		height: 4px;
	}
	.sample {
		flex: 0 0 auto;
		overflow: visible;
	}
</style>
