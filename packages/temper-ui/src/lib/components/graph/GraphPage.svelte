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
	import RegionState from '$lib/components/RegionState.svelte';
	import type { GraphViewData } from '$lib/graph/view';
	import { regionStateFor } from '$lib/region';
	import BoundLine from './BoundLine.svelte';
	import GraphA11yList from './GraphA11yList.svelte';
	import GraphCanvas from './GraphCanvas.svelte';
	import NodeRail from './NodeRail.svelte';
	import WhyThese from './WhyThese.svelte';
	import { withGraphQuestion, withGraphSelection, withoutGraphWalk } from '$lib/vault-url';

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

	/** The five settled values one read produces, in the order the `{#await}` below unwraps them. */
	type Answer = [
		Awaited<GraphViewData['model']>,
		Awaited<GraphViewData['selected']>,
		Awaited<GraphViewData['readout']>,
		Awaited<GraphViewData['bound']>,
		Awaited<GraphViewData['tooLittleStructure']>,
	];

	/**
	 * One arrival of the answer, **with the address it answers for stamped on it.**
	 *
	 * The two rail reads travel here rather than being read off `data` at the point of use, so a
	 * generation swaps whole. Held apart, the rail could show the node the canvas has drawn beside a
	 * paragraph read for a different one — which is the same species of false claim as the marks
	 * this block exists to keep.
	 */
	interface Arrival {
		key: string;
		answer: Promise<Answer>;
		excerpt: GraphViewData['selectedExcerpt'];
		trail: GraphViewData['selectedTrail'];
	}

	/**
	 * **One read, one arrival.** The canvas, the panel beside it and the bound line are three views
	 * of a single streamed read, so they are awaited together — three arriving markers for one read
	 * would tell the reader those regions could disagree about whether it answered, and they cannot.
	 * `selected` joins them because the rail is resolved against the model and cannot precede it.
	 *
	 * The `.catch()` is spec §5.3's *other* catch, at the one place this page creates a promise that
	 * did not come through `bounded`: `Promise.all` is a new promise, and during SSR the `{#await}`
	 * below renders its pending branch without subscribing to it. `.catch()` consumes nothing, so
	 * `{:catch}` still sees the failure.
	 *
	 * `key` is **the whole address with the selection taken off**, built by the one function that
	 * owns that param. It is what the load's read actually depends on: `q`, `in`, `from` and `depth`
	 * all decide which read runs and what it returns, and `sel` decides none of them —
	 * `withGraphSelection` "rewrites the URL in place … so `q`, `in` and `from` travel untouched".
	 * The URL is the only complete source for it: `data` cannot express `from`, so a key read off
	 * `data.question` and `data.placesAsked` would call a hop the same answer as the screen it
	 * hopped from. Both are captured in ONE derived so they can only change together, which is how
	 * SvelteKit lands `page.url` and `data` — a key stamped in a later tick would belong to neither.
	 */
	const arriving = $derived.by((): Arrival => {
		const all: Promise<Answer> = Promise.all([
			data.model,
			data.selected,
			data.readout,
			data.bound,
			data.tooLittleStructure,
		]);
		all.catch(() => {});
		return {
			key: withGraphSelection($page.url, null),
			answer: all,
			excerpt: data.selectedExcerpt,
			trail: data.selectedTrail,
		};
	});

	/**
	 * The last arrival that SETTLED — written here, never read here, so this cannot re-trigger
	 * itself. `$state.raw` because an arrival is replaced whole and nothing inside it is mutated.
	 *
	 * It stays `null` through SSR, where effects do not run: the server therefore awaits the
	 * incoming promise exactly as it always did, and renders the pending branch.
	 */
	let landed = $state.raw<Arrival | null>(null);

	/**
	 * **The marks the reader is looking at, when a new read cannot change them.**
	 *
	 * Non-null only while all three hold: something has landed, it answers for the same address as
	 * the read now in flight, and that read has not landed yet. Then the `{#await}` below is handed
	 * the promise it is *already* showing — so it never re-enters its pending branch, and the canvas
	 * is not rebuilt at all. When the key differs this is null and the marks come down, because the
	 * incoming read is a different answer and marks left under it would be a false claim.
	 *
	 * Note what this is NOT: it is not a stale-while-revalidate window. Nothing here decides to show
	 * an older answer than the one available — the older answer and the newer one are the same
	 * model, because `sel` cannot reach the read.
	 */
	const held = $derived(
		landed !== null && landed.key === arriving.key && landed.answer !== arriving.answer
			? landed
			: null,
	);
	const answer = $derived(held?.answer ?? arriving.answer);

	$effect(() => {
		const arrival = arriving;
		let live = true;
		// Settled either way. A read that FAILED must take the held marks down and say so, exactly
		// as a first read that fails does — `{:catch}` is reached by handing it the rejected promise.
		const land = () => {
			if (live) landed = arrival;
		};
		arrival.answer.then(land, land);
		return () => {
			live = false;
		};
	});

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
		<!--
			Everything above this line — the ask form, the borrowed-charter line, the refusal and the
			page chrome — renders OUTSIDE every await. That is C1 for this route, and it is
			structural rather than a promise anyone has to keep: none of it reads `data.model`.
		-->
		{#await answer}
			<div class="instrument">
				<div class="stage region">
					<RegionState state="arriving" label="graph" />
				</div>
			</div>
		{:then [model, selected, readout, bound, tooLittle]}
			{#if tooLittle}
				<!-- Rung 2. NOT a refusal in the reader's terms and it must not read like one: they
				     have material, it simply has no relationships to draw. So the screen says which
				     instrument is the right one and hands them the door to it, rather than showing
				     an empty canvas and letting them conclude they have nothing.

				     It renders HERE, inside the read, because it is a verdict about the answer that
				     came back rather than about the address the reader arrived on. -->
				<div class="refusal" role="status">
					<h2>A graph is not the right view for this yet</h2>
					<p>
						You can read {tooLittle.inScope}
						{tooLittle.inScope === 1 ? 'resource' : 'resources'} here, but nothing is linked to
						anything else — so there is no shape to draw. The list view is the better instrument
						until things start connecting.
					</p>
					<p><a href={`/vault/${data.owner}`}>Browse them as a list →</a></p>
				</div>
			{:else}
				<!-- Resolved against what was DRAWN, on the client as on the server: a `sel` this
				     answer does not contain opens nothing. -->
				{@const node = selected ? (model.nodes.find((n) => n.id === selected) ?? null) : null}
				<div class="instrument">
					<div class="stage">
						<GraphA11yList {model} url={$page.url} />
						<GraphCanvas {model} {selected} onSelect={select} {emptyMessage} />
						{#if node}
							<!-- From the SAME arrival as `model`, so the rail cannot describe a node
							     beside a paragraph read for a different one. -->
							<NodeRail
								{node}
								{model}
								excerpt={held ? held.excerpt : data.selectedExcerpt}
								trail={held ? held.trail : data.selectedTrail}
							/>
						{/if}

						{#if held}
							<!-- The marks stayed, and the reader is still told a read is in flight —
							     "keep the marks" is not "hide that anything is loading". Same region
							     vocabulary as the pending branch, because it is the same state: this
							     read has not answered. It sits over the canvas rather than replacing
							     it, which is the whole difference.

							     Only here, over the marks. Rung 2 replaces the canvas, and nothing on
							     that screen is clickable, so the one navigation that reaches this
							     without changing the key cannot start from it. -->
							<div class="updating">
								<RegionState state="arriving" label="graph" />
							</div>
						{/if}
					</div>

					<!--
						Rendered on a traversal too, where `readout` is null. Until D2 the condition was
						`{#if data.readout}` alone, which is composition-only — so a hop got §7.2's
						"disappear", explicitly named there as the second-best of three because it "loses the
						reader's route back to how they got here."

						Both halves are read from the SETTLED values the await unwrapped. `{#if data.readout}`
						over a promise is always true, which would render this panel unconditionally and pass
						every test in this file.
					-->
					{#if readout || bound?.traversed}
						<WhyThese
							{readout}
							question={data.question}
							owner={data.owner}
							places={data.placesAsked}
							backHref={bound?.traversed ? withoutGraphWalk($page.url) : null}
						/>
					{/if}
				</div>

				{#if bound}
					<BoundLine {bound} />
				{/if}
			{/if}
		{:catch error}
			<!-- Never the second state held open: nothing here says "arriving". The marks are not late
			     — either they were not read, or the system stopped waiting for them, and
			     `regionStateFor` is what tells those two apart on this side of the boundary. -->
			<div class="instrument">
				<div class="stage region">
					<RegionState state={regionStateFor(error)} label="graph" />
				</div>
			</div>
		{/await}
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
	/* The stage while the read is in flight, or after it failed. The region keeps its own
	   appearance; this only gives it the room the canvas would have had. */
	.stage.region {
		align-items: flex-start;
		padding: 24px 14px;
	}
	/* The same region, over marks that are staying. Placed rather than laid out, so keeping the
	   canvas costs it no room and nothing on it moves when the new read lands. */
	.updating {
		position: absolute;
		top: 10px;
		left: 12px;
		z-index: 1;
		background: rgba(20, 23, 29, 0.92);
		border-radius: 0 5px 5px 0;
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
