<script lang="ts">
	/**
	 * The analysis render harness — the receiver half.
	 *
	 * Three anchors, all from one untrimmed capture taken **2026-08-20**: a context with 501
	 * groupings, a cogmap with 406 and an analytics row, and a cogmap that has **never materialized
	 * a region**. The third is the one worth having a harness for: a table of nothing, above a
	 * legend that has to say why, is the screen `displaced-structure-remains-reachable` is judged
	 * on and the hardest to get right without looking at it.
	 *
	 * **`[2026-08-25]` The capture date is what the context's missing analytics row means.** A
	 * context has an anchor-level readout now — `/api/contexts/{id}/analytics` answers the
	 * staleness half — and the bundle carries none for its context only because it predates that
	 * door. So the context here renders the declined branch: a property of the CAPTURE, not of the
	 * world. `$lib/graph/harness.ts` carries the same remainder where it infers the anchor kind
	 * from that row's presence.
	 */
	import AnalysisPage from '$lib/components/graph/AnalysisPage.svelte';
	import { analysisScenarioNames, analysisViewFor } from '$lib/graph/harness';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();

	const anchors = $derived(analysisScenarioNames(data.bundle));
	let anchor = $state('context');
	const view = $derived(analysisViewFor(data.bundle, anchor));

	let w = $state(1280);
	let h = $state(860);
</script>

<svelte:head><title>Analysis render harness</title></svelte:head>

<div class="harness">
	<header class="controls">
		<span class="brand">⚙ Analysis render harness</span>
		<div class="group">
			<span class="cap">anchor</span>
			{#each anchors as key (key)}
				<button type="button" class:active={anchor === key} onclick={() => (anchor = key)}>
					{key}
				</button>
			{/each}
		</div>
		<label class="num">w <input type="number" min="320" max="2400" step="10" bind:value={w} /></label>
		<label class="num">h <input type="number" min="240" max="1600" step="10" bind:value={h} /></label>
		<span class="meta">{w}×{h}px</span>
	</header>

	<div class="stage">
		<div class="frame" style={`width:${w}px;height:${h}px`}>
			{#key anchor}
				<AnalysisPage data={view} />
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
	.stage {
		padding: 16px;
		overflow: auto;
	}
	.frame {
		overflow: auto;
		resize: both;
		border: 1px solid rgba(255, 255, 255, 0.14);
		background: #0f1116;
	}
</style>
