<script lang="ts">
	/**
	 * The graph surface's shell: the ask bar, the canvas, the readout, the bound line.
	 *
	 * The layout carries a claim the reader can check by looking: **everything in the canvas is
	 * their own material, and everything a machine decided is in the panel beside it.** Nothing
	 * derived is drawn as a mark, and the readout is not a mark either — it is a panel that reads
	 * as reasoning about the answer.
	 *
	 * The bound line spans the full width beneath both, present whether or not the view is
	 * partial, and there is nothing that dismisses it.
	 */
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { GraphViewData } from '$lib/graph/view';
	import BoundLine from './BoundLine.svelte';
	import GraphA11yList from './GraphA11yList.svelte';
	import GraphCanvas from './GraphCanvas.svelte';
	import NodeRail from './NodeRail.svelte';
	import WhyThese from './WhyThese.svelte';
	import { withGraphQuestion, withGraphSelection } from '$lib/vault-url';

	let { data }: { data: GraphViewData } = $props();

	// The input is seeded from the URL and re-seeded whenever it changes, so the box always shows
	// what the answer on screen was actually asked — never a half-typed question the graph does
	// not reflect.
	// Seeded by the effect rather than the initialiser, which would capture only the first
	// `data` and leave the box showing a question the graph no longer reflects.
	let asked = $state('');
	$effect(() => {
		asked = data.question ?? '';
	});

	const selectedNode = $derived(
		data.selected ? (data.model.nodes.find((n) => n.id === data.selected) ?? null) : null,
	);

	// `q` PUSHES: asking a different question is a step the reader can walk back out of, and Back
	// must walk their path rather than leave the site. `sel` REPLACES: a selection is ephemeral
	// panel state and does not belong in the history the Back button walks.
	function ask(event: SubmitEvent) {
		event.preventDefault();
		goto(withGraphQuestion($page.url, asked));
	}
	const select = (id: string) => goto(withGraphSelection($page.url, id), { replaceState: true });

	const emptyMessage = $derived(
		data.question
			? 'Nothing in the places you asked about answers this yet.'
			: 'There is nothing in these places yet.',
	);
</script>

<div class="graph-page">
	<form class="ask" onsubmit={ask}>
		<label class="sr-only" for="graph-question">Ask about your work</label>
		<input
			id="graph-question"
			bind:value={asked}
			placeholder="Ask about your work — or leave this empty to see everything"
			autocomplete="off"
		/>
		<button type="submit">Ask</button>
		{#if data.question}
			<a class="clear" href={withGraphQuestion($page.url, null)}>Clear</a>
		{/if}
	</form>

	{#if data.borrowedFrom}
		<p class="borrowed">
			No question was asked, so this is surveying under
			<a href={`/vault/r/${data.borrowedFrom.telosResourceId}`}>{data.borrowedFrom.name}</a>’s own
			charter.
		</p>
	{/if}

	{#if data.refusal}
		<!-- A refusal names what the reader asked for and nothing about why a place is absent: a
		     place they cannot read is absent, never hinted at, and its absence is not
		     distinguishable from its nonexistence. -->
		<div class="refusal" role="status">
			{#if data.refusal.kind === 'no-place-resolved'}
				<h2>Nothing to show for {data.refusal.named === 1 ? 'that place' : 'those places'}</h2>
				<p>
					{data.refusal.named === 1 ? 'The place' : 'None of the places'} this link names
					{data.refusal.named === 1 ? 'is' : 'are'} available to you now. Nothing was answered in
					{data.refusal.named === 1 ? 'its' : 'their'} place.
				</p>
				<p><a href={`/graph/${data.owner}`}>See everything you can read →</a></p>
			{:else}
				<h2>There is nothing here yet</h2>
				<p>Once you have a place with work in it, this is where its shape appears.</p>
			{/if}
		</div>
	{:else}
		<div class="instrument">
			<div class="stage">
				<GraphA11yList model={data.model} url={$page.url} />
				<GraphCanvas
					model={data.model}
					selected={data.selected}
					onSelect={select}
					{emptyMessage}
				/>
				{#if selectedNode}
					<NodeRail
						node={selectedNode}
						model={data.model}
						excerpt={data.selectedExcerpt}
						trail={data.selectedTrail}
					/>
				{/if}
			</div>

			{#if data.readout}
				<WhyThese
					readout={data.readout}
					question={data.question}
					owner={data.owner}
					places={data.placesAsked}
				/>
			{/if}
		</div>

		{#if data.bound}
			<BoundLine bound={data.bound} />
		{/if}
	{/if}
</div>

<style>
	.graph-page {
		display: grid;
		grid-template-rows: auto 1fr auto;
		height: 100%;
		min-height: 0;
	}
	.ask {
		display: flex;
		gap: 8px;
		align-items: center;
		padding: 8px 14px;
		border-bottom: 1px solid rgba(255, 255, 255, 0.06);
	}
	.ask input {
		flex: 1 1 auto;
		min-width: 0;
		padding: 7px 11px;
		border: 1px solid rgba(255, 255, 255, 0.12);
		border-radius: 7px;
		background: rgba(255, 255, 255, 0.03);
		color: #dbe2ea;
		font-size: 13px;
	}
	.ask button,
	.ask .clear {
		padding: 7px 12px;
		border: 1px solid rgba(255, 255, 255, 0.14);
		border-radius: 7px;
		background: none;
		color: #c9d1d9;
		font-size: 12px;
		text-decoration: none;
		cursor: pointer;
	}
	.borrowed {
		grid-row: auto;
		margin: 0;
		padding: 6px 14px;
		border-bottom: 1px solid rgba(255, 255, 255, 0.06);
		color: #9aa3b0;
		font-size: 12.5px;
	}
	.instrument {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 20rem;
		min-height: 0;
		min-width: 0;
	}
	@media (max-width: 900px) {
		.instrument {
			grid-template-columns: 1fr;
		}
	}
	.stage {
		position: relative;
		display: flex;
		min-width: 0;
		min-height: 0;
		overflow: hidden;
	}
	.refusal {
		display: grid;
		align-content: center;
		justify-items: start;
		gap: 8px;
		padding: 40px 14px;
		max-width: 34rem;
		color: #9aa3b0;
	}
	.refusal h2 {
		margin: 0;
		font-family: Georgia, serif;
		font-size: 19px;
		color: #dbe2ea;
	}
	.refusal p {
		margin: 0;
		font-size: 13.5px;
		line-height: 1.55;
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip: rect(0 0 0 0);
		white-space: nowrap;
	}
</style>
