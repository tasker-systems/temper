<script lang="ts">
	/**
	 * The graph render harness. Pick a scenario, pick a viewport, look at the real page.
	 *
	 * The viewport is a first-class knob rather than a convenience: two of this surface's live
	 * rulings are about width and height specifically — `CANVAS_FLOOR_PX` yields *Why these* below a
	 * threshold nobody has measured, and `.instrument`'s 900px stacking rule has never once fired.
	 * Neither is observable in jsdom, which computes no layout, so the presets below are the
	 * instrument for both.
	 */
	import GraphPage from '$lib/components/graph/GraphPage.svelte';
	import { scenarioNames, viewFor } from '$lib/graph/harness';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();

	const scenarios = $derived(scenarioNames(data.bundle));
	let scenario = $state('entry');
	const view = $derived(viewFor(data.bundle, scenario));
	const why = $derived((data.bundle[scenario] as { _why?: string })?._why ?? '');
	const cannot = $derived((data.bundle[scenario] as { _does_not_witness?: string })?._does_not_witness);

	interface Preset {
		label: string;
		w: number;
		h: number;
	}
	// Each preset is a threshold something on this page is ruled against, not a device.
	const presets: Preset[] = [
		{ label: 'roomy 1440×900', w: 1440, h: 900 },
		{ label: 'above the floor 1280×770', w: 1280, h: 770 },
		{ label: 'at the floor 1280×704', w: 1280, h: 704 },
		{ label: 'below the floor 1280×610', w: 1280, h: 610 },
		{ label: 'at the stack rule 900×700', w: 900, h: 700 },
		{ label: 'below it 760×700', w: 760, h: 700 }
	];
	let w = $state(1280);
	let h = $state(770);
</script>

<svelte:head><title>Graph render harness</title></svelte:head>

<div class="harness">
	<header class="controls">
		<span class="brand">⚙ Graph render harness</span>
		<div class="group">
			<span class="cap">scenario</span>
			{#each scenarios as key (key)}
				<button type="button" class:active={scenario === key} onclick={() => (scenario = key)}>
					{key}
				</button>
			{/each}
		</div>
		<div class="group">
			<span class="cap">viewport</span>
			{#each presets as p (p.label)}
				<button
					type="button"
					class:active={w === p.w && h === p.h}
					onclick={() => {
						w = p.w;
						h = p.h;
					}}>{p.label}</button
				>
			{/each}
			<label class="num">w <input type="number" min="320" max="2400" step="10" bind:value={w} /></label>
			<label class="num">h <input type="number" min="240" max="1600" step="10" bind:value={h} /></label>
		</div>
		<span class="meta">{w}×{h}px</span>
	</header>

	<p class="why">{why}</p>
	{#if cannot}
		<!-- What the fixture cannot witness travels WITH it, so a reader of this screen is not left
		     to infer coverage from the fact that something rendered. -->
		<p class="cannot"><strong>This fixture cannot witness:</strong> {cannot}</p>
	{/if}

	<div class="stage">
		<div class="frame" style={`width:${w}px;height:${h}px`}>
			{#key scenario}
				<GraphPage data={view} />
			{/key}
		</div>
	</div>
</div>

<style>
	.harness {
		display: flex;
		flex-direction: column;
		min-height: 100vh;
		background: #0b0d11;
		color: #c9ced9;
		font: 12px/1.4 system-ui, sans-serif;
	}
	.controls {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 16px;
		padding: 8px 14px;
		border-bottom: 1px solid rgba(255, 255, 255, 0.08);
		background: #12151b;
	}
	.brand {
		font-weight: 600;
		color: #e6e9ef;
	}
	.group {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px;
	}
	.cap {
		text-transform: uppercase;
		letter-spacing: 0.08em;
		font-size: 10px;
		color: #6f7886;
		margin-right: 4px;
	}
	button {
		background: #1b2028;
		border: 1px solid rgba(255, 255, 255, 0.1);
		color: #c9ced9;
		border-radius: 4px;
		padding: 3px 8px;
		font: inherit;
		cursor: pointer;
	}
	button.active {
		background: #2d3947;
		border-color: #4a6480;
		color: #e6e9ef;
	}
	.num input {
		width: 62px;
		background: #1b2028;
		border: 1px solid rgba(255, 255, 255, 0.1);
		color: inherit;
		border-radius: 4px;
		padding: 2px 4px;
		font: inherit;
	}
	.meta {
		margin-left: auto;
		color: #6f7886;
		font-variant-numeric: tabular-nums;
	}
	.why {
		margin: 0;
		padding: 8px 14px;
		color: #9aa3b0;
		border-bottom: 1px solid rgba(255, 255, 255, 0.06);
	}
	.cannot {
		margin: 0;
		padding: 8px 14px;
		color: #e6c07b;
		background: rgba(230, 192, 123, 0.07);
		border-bottom: 1px solid rgba(230, 192, 123, 0.2);
	}
	.stage {
		padding: 16px;
		overflow: auto;
	}
	.frame {
		overflow: hidden;
		resize: both;
		border: 1px solid rgba(255, 255, 255, 0.14);
		background: #0f1116;
	}
</style>
