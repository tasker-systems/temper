<script lang="ts">
	import AtlasPage from '$lib/components/graph/atlas/AtlasPage.svelte';
	import type { AtlasViewData } from '$lib/graph/atlas/viewData';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();

	// Scenario keys are every fixture entry except the `_meta` provenance stamp.
	const scenarios = $derived(Object.keys(data.fixtures).filter((k) => k !== '_meta'));
	let scenario = $state('home');
	const view = $derived(data.fixtures[scenario] as AtlasViewData);

	// Viewport axis — the two Beat-2a regressions (legend grows into canvas /
	// legend squeezed off-screen) only surface at constrained heights, so the
	// harness makes width AND height first-class knobs with worst-case presets.
	interface Preset {
		label: string;
		w: number;
		h: number;
	}
	const presets: Preset[] = [
		{ label: 'short 1280×380', w: 1280, h: 380 },
		{ label: 'laptop 1366×640', w: 1366, h: 640 },
		{ label: 'interstitial 1024×560', w: 1024, h: 560 },
		{ label: 'tall 1440×900', w: 1440, h: 900 },
		{ label: 'narrow 720×760', w: 720, h: 760 }
	];
	let w = $state(1280);
	let h = $state(560);

	function applyPreset(p: Preset) {
		w = p.w;
		h = p.h;
	}
</script>

<svelte:head><title>Atlas render harness</title></svelte:head>

<div class="harness">
	<header class="controls">
		<span class="brand">⚙ Atlas render harness</span>

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
				<button type="button" class:active={w === p.w && h === p.h} onclick={() => applyPreset(p)}>
					{p.label}
				</button>
			{/each}
			<label class="num">w <input type="number" min="320" max="2400" step="10" bind:value={w} /></label>
			<label class="num">h <input type="number" min="240" max="1600" step="10" bind:value={h} /></label>
		</div>

		<span class="meta">{w}×{h}px · {data.fixtures._meta?.team ?? 'fixtures'}</span>
	</header>

	<div class="stage">
		<div class="frame" style={`width:${w}px;height:${h}px`}>
			{#key scenario}
				<!-- Replay the fixture's captured selection: the harness renders at a static
				     URL with no `?sel=`, so `?sel=`-driven selections (context-node + edge
				     rails) only surface when passed explicitly. -->
				<AtlasPage data={view} selectionOverride={view.selection} />
			{/key}
		</div>
	</div>
</div>

<style>
	.harness {
		display: flex;
		flex-direction: column;
		height: 100vh;
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
		letter-spacing: 0.02em;
	}
	.group {
		display: flex;
		align-items: center;
		gap: 4px;
		flex-wrap: wrap;
	}
	.cap {
		text-transform: uppercase;
		letter-spacing: 0.14em;
		font-size: 9px;
		color: #6a727e;
		margin-right: 2px;
	}
	button {
		background: #1b1f27;
		border: 1px solid rgba(255, 255, 255, 0.1);
		color: #c9ced9;
		border-radius: 6px;
		padding: 3px 8px;
		cursor: pointer;
		font: inherit;
	}
	button:hover {
		border-color: rgba(255, 255, 255, 0.28);
	}
	button.active {
		background: #2b3648;
		border-color: #5b7aa8;
		color: #fff;
	}
	.num {
		display: inline-flex;
		align-items: center;
		gap: 3px;
		color: #6a727e;
	}
	.num input {
		width: 56px;
		background: #1b1f27;
		border: 1px solid rgba(255, 255, 255, 0.1);
		color: #c9ced9;
		border-radius: 6px;
		padding: 3px 5px;
		font: inherit;
	}
	.meta {
		margin-left: auto;
		color: #6a727e;
	}
	/* The stage scrolls so oversized frames are inspectable; the frame itself
	   clips (overflow:hidden) so it behaves like a real bounded viewport — the
	   whole point of the height regressions. */
	.stage {
		flex: 1;
		overflow: auto;
		padding: 24px;
		display: flex;
		justify-content: center;
		align-items: flex-start;
	}
	.frame {
		flex: 0 0 auto;
		overflow: hidden;
		border: 1px solid rgba(255, 255, 255, 0.14);
		border-radius: 8px;
		box-shadow: 0 8px 40px rgba(0, 0, 0, 0.5);
		background: var(--obsidian, #0b0d11);
		resize: both;
	}
</style>
