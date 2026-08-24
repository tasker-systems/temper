<script lang="ts">
	/**
	 * The analysis surface — and the first thing on it says what it is.
	 *
	 * **Everything here is the machine's arithmetic about the reader's work, and nothing here is
	 * the work.** That is the exception that proves the successor's thesis rather than a breach of
	 * it: derived structure is never a navigational mark, so when it needs a place, the place says
	 * out loud what it holds. `surface-declares-its-kind` is a judged clause and no test can settle
	 * it — what a test *can* pin is that the declaration is present, unconditional and first, which
	 * is what `AnalysisPage.component.test.ts` does.
	 *
	 * **Not one number on this page is normalised, and none is drawn as a bar.** The quantities are
	 * unbounded, they are `Option<f64>`, and their ranges differ by an order of magnitude between
	 * places. A figure that merely *looked* calibrated would settle an open ruling silently, so
	 * every figure appears raw, beside the span this place actually measures. `analysis.ts` carries
	 * the measurements and the reasoning.
	 *
	 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §4 (Beat C)
	 */
	import RegionState from '$lib/components/RegionState.svelte';
	import GroupingStrips from './GroupingStrips.svelte';
	import {
		CONTEXT_HAS_NO_MAP_READOUT,
		MAP_READOUT_UNAVAILABLE,
		METRICS_UNAVAILABLE,
		describeConstant,
		describeGroupingCount,
		describeNulls,
		describeRange,
		describeRegulation,
		describeStaleness,
		formatValue,
		reportMetrics,
	} from '$lib/graph/analysis';
	import type { AnalysisViewData } from '$lib/graph/view';
	import { regionStateFor } from '$lib/region';
	import { graphAnalysisHref, graphHref, resourceHref } from '$lib/vault-url';

	let { data }: { data: AnalysisViewData } = $props();

	/**
	 * **One read, one arrival.** The map-level section and the groupings section are two views of a
	 * single streamed read, so they are awaited together — two arriving markers for one read would
	 * tell the reader those regions could disagree about whether it answered, and they cannot.
	 *
	 * The `.catch()` is spec §5.3's *other* catch, at the one place this page creates a promise that
	 * did not come through `bounded`: `Promise.all` is a new promise, and during SSR the `{#await}`
	 * below renders its pending branch without subscribing to it. `.catch()` consumes nothing, so
	 * `{:catch}` still sees the failure.
	 */
	const measurements = $derived.by(() => {
		const all = Promise.all([data.regions, data.metricsAvailable, data.map, data.emptiness]);
		all.catch(() => {});
		return all;
	});

	const placeHref = $derived(
		data.place
			? graphHref(data.owner, { anchors: [{ kind: data.place.kind, ref: data.place.ref }] })
			: `/graph/${data.owner}`,
	);
</script>

<div class="analysis">
	<!-- Unconditional and first. It is not conditioned on there being anything to show, because a
	     page that declares its kind only when it has content declares nothing. -->
	<p class="declaration" data-testid="kind-declaration">
		Everything on this page is the system's own measurement of your work. None of it is something
		you wrote — your own material is on <a href={placeHref}>the graph</a>.
	</p>

	{#if data.refusal}
		<div class="refusal" role="status">
			{#if data.refusal.kind === 'no-place-resolved'}
				<h1>
					Nothing to measure for {data.refusal.named === 1 ? 'that place' : 'those places'}
				</h1>
				<p>
					<!--
						`is not` / `is`, NOT `is` / `is`. The plural carries its negation in "None of
						the places"; the singular has none of its own, so with both arms spelling `is`
						this read "The place named in this link IS readable by you, so there is nothing
						here to measure" — the opposite of the truth, in the one sentence about the
						reader's own access, on the commonest case (a link naming one place).
					-->
					{data.refusal.named === 1 ? 'The place' : 'None of the places'} named in this link
					{data.refusal.named === 1 ? 'is not' : 'is'} readable by you, so there is nothing here
					to measure. It may have been removed, or never shared with you.
				</p>
			{:else}
				<h1>There is nothing here yet</h1>
				<p>Once you have a place with work in it, this is where its measurements appear.</p>
			{/if}
		</div>
	{:else if !data.place}
		<!-- The index. Arriving with no address is answered with what the reader can read, never
		     refused for not knowing the grammar. -->
		<h1>Measure one of your places</h1>
		<p class="lead">
			Each place is measured on its own, because the same quantity runs to very different sizes
			in different places and putting two in one list would invite a comparison that is not
			there.
		</p>
		<ul class="choices">
			{#each data.choices as c (c.ref)}
				<li>
					<a href={graphAnalysisHref(data.owner, { kind: c.kind, ref: c.ref })}>{c.title}</a>
				</li>
			{/each}
		</ul>
	{:else}
		<!-- Named once, where `data.place` is narrowed. The awaited block below reads `place` rather
		     than re-narrowing inside a branch that has nothing to do with the read. -->
		{@const place = data.place}
		<h1>{place.title}</h1>

		{#if data.alsoNamed.length > 0}
			<p class="also" data-testid="also-named">
				You named {data.alsoNamed.length + 1} places. This measures one at a time — the others are
				{#each data.alsoNamed as p, i (p.ref)}<a
						href={graphAnalysisHref(data.owner, { kind: p.kind, ref: p.ref })}>{p.title}</a
					>{i < data.alsoNamed.length - 1 ? ', ' : '.'}{/each}
			</p>
		{/if}

		<!--
			Everything above this line — the declaration, the title and the also-named line — renders
			OUTSIDE every await. That is C1 for this route, and it is structural rather than a promise
			anyone has to keep: none of it reads a measurement.

			ONE await for both sections. They are two views of a single read, so they arrive together.
		-->
		{#await measurements}
			<div class="region-slot">
				<RegionState state="arriving" label="measurements" />
			</div>
		{:then [regions, metricsAvailable, map, emptiness]}
			{@const reports = reportMetrics(regions)}
			{@const columns = reports.filter((r) => r.asColumn)}
			<section class="map-level" aria-labelledby="map-level-h">
				<h2 id="map-level-h">What this place says it is for</h2>
				{#if map}
					<p>
						Its charter is <a href={resourceHref({ id: map.telos.id })}
							>{map.telos.title ?? 'the charter resource'}</a
						>.
					</p>
					<p data-testid="staleness">{describeStaleness(map.staleness)}</p>
					<p data-testid="regulation">{describeRegulation(map.regulation.length)}</p>
					{#if map.regulation.length > 0}
						<ul class="regulation">
							{#each map.regulation as r (r.resource_id)}
								<li><a href={resourceHref({ id: r.resource_id })}>{r.title}</a></li>
							{/each}
						</ul>
					{/if}
				{:else if place.kind === 'context'}
					<!-- Declared, not fabricated. D6 is unshipped and a context has no charter and no
					     regulation set even in principle; inventing a peer field is what the task
					     explicitly forbids. -->
					<p class="declared-absent" data-testid="map-absent">{CONTEXT_HAS_NO_MAP_READOUT}</p>
				{:else}
					<!-- A map whose analytics read was DECLINED — the read answered, and this is what it
					     said. A read that FAILED is the third state and renders in `{:catch}` below. -->
					<p class="declared-absent" data-testid="map-absent">{MAP_READOUT_UNAVAILABLE}</p>
				{/if}
			</section>

			<section class="groupings" aria-labelledby="groupings-h">
				<h2 id="groupings-h">How its work has been grouped</h2>
				<!--
					`[reviewed — 2026-08-21]` This is the one settled-empty state that does NOT route
					through `RegionState`, and that is deliberate. It is more specific than the shared
					vocabulary's "No measurements.", and swapping it for the generic wording would
					satisfy a consistency argument by making the page say less.

					The clause it has to meet is that no two states present alike, and it does: these
					sentences and the `{:catch}`'s "Measurements unavailable — nothing here was read"
					share no words. What `RegionState` protects against is drift between the ARRIVING
					and FAILED spellings across surfaces, and both of those still come from it.

					`[2026-08-24]` The empty spelling was ONE sentence — "This place has no groupings
					yet." — for all four causes, and its *yet* asserted `never_clustered` on a read
					that may have meant any of them. It now takes the cause the read carried. This was
					the last door still claiming a cause it could not know; `16a9e357` fixed the CLI.
				-->
				{#if regions.length === 0}
					<!--
						The empty spelling gets `.declared-absent` and `role="status"`, and neither is
						cosmetic. `.lead` is sized for the one-line row count this used to be; these
						sentences are declarations of the same species as `CONTEXT_HAS_NO_MAP_READOUT`,
						which already uses that treatment. And this is the WHOLE content of the section
						for a screen-reader user, arriving after a wait they were told about — the
						`{#await}` pending branch's `RegionState` is `role="status"`, and it is torn down
						when the read settles, so without this nothing is announced at all.

						Stated honestly: this is a HALF fix for the announcement. The node is inserted
						together with its text rather than mutated inside a region that already existed,
						and AT behaviour there varies. The reliable shape is a live region mounted before
						the await settles; that is a change to how this route streams, not to this line.
					-->
					<p class="lead declared-absent" data-testid="grouping-count" role="status">
						{describeGroupingCount(0, emptiness)}
					</p>
				{:else}
					<p class="lead" data-testid="grouping-count">
						{describeGroupingCount(regions.length, emptiness)}
					</p>
				{/if}

				{#if !metricsAvailable}
					<p class="unavailable" role="status" data-testid="metrics-unavailable">
						{METRICS_UNAVAILABLE}
					</p>
				{/if}

				{#if regions.length > 0}
					<dl class="legend" data-testid="metric-legend">
						{#each reports as r (r.spec.key)}
							<div class="metric" class:collapsed={!r.asColumn}>
								<dt>{r.spec.label} <code>{r.spec.field}</code></dt>
								<dd class="gloss">{r.spec.gloss}</dd>
								{#if r.distribution.constant}
									<dd class="finding">{describeConstant(r.distribution)}</dd>
								{:else}
									<dd class="range">{describeRange(r.distribution)}</dd>
								{/if}
								{#if describeNulls(r.distribution)}
									<dd class="nulls">{describeNulls(r.distribution)}</dd>
								{/if}
							</div>
						{/each}
					</dl>

					<GroupingStrips {regions} />

					<!-- The table is kept, and moved BEHIND a disclosure rather than deleted.
					     The strips answer "what is this telling me"; the figures answer "what exactly
					     does this one measure", and a reader who wants the second should not have to
					     ask anyone for it. `<details>` keeps every row in the document whether or not
					     it is open, so nothing is withheld and nothing is lazily built — the
					     no-top-N property is unchanged, and so is every assertion that counts rows. -->
					<details class="figures">
						<summary>Show the figures — every grouping, every quantity</summary>
					<div class="table-scroll">
						<table>
							<thead>
								<tr>
									<th scope="col">Grouping</th>
									{#each columns as c (c.spec.key)}
										<th scope="col">{c.spec.label}</th>
									{/each}
								</tr>
							</thead>
							<tbody>
								{#each regions as r (`${r.regionId}|${r.lensId}`)}
									<tr>
										<th scope="row" class:unnamed={r.label === null}
											>{r.label ?? 'An unnamed grouping'}</th
										>
										{#each columns as c (c.spec.key)}
											<td class:absent={r.values[c.spec.key] === null}
												>{formatValue(r.values[c.spec.key])}</td
											>
										{/each}
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
					</details>
				{/if}
			</section>
		{:catch error}
			<!-- Never the second state held open: nothing here says "arriving". The measurements are
			     not late — they were not read, or the system stopped waiting for them — and this page
			     keeps saying which place it was measuring, because that never depended on the read. -->
			<div class="region-slot">
				<RegionState state={regionStateFor(error)} label="measurements" />
			</div>
		{/await}
	{/if}
</div>

<style>
	.analysis {
		display: flex;
		flex-direction: column;
		gap: 16px;
		padding: 20px 24px 48px;
		overflow: auto;
		min-height: 0;
	}
	.declaration {
		margin: 0;
		padding: 10px 14px;
		border-left: 3px solid #6f7a90;
		background: rgba(111, 122, 144, 0.1);
		color: #c3ccd9;
		font-size: 13px;
		line-height: 1.5;
	}
	h1 {
		margin: 4px 0 0;
		font-size: 20px;
		color: #e6edf5;
	}
	h2 {
		margin: 0 0 8px;
		font-size: 14px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: #8b94a5;
	}
	.lead,
	.also,
	.declared-absent {
		margin: 0;
		color: #aab4c0;
		font-size: 13px;
		line-height: 1.55;
	}
	.declared-absent {
		color: #8b94a5;
		font-style: italic;
	}
	.unavailable {
		margin: 0;
		color: #d9b26a;
		font-size: 13px;
	}
	/* The one region marker this page has, given the section rule above it so an arrival sits where
	   the sections it stands in for will sit. */
	.region-slot {
		padding-top: 12px;
		border-top: 1px solid #2b3140;
	}
	section {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding-top: 12px;
		border-top: 1px solid #2b3140;
	}
	.choices,
	.regulation {
		margin: 0;
		padding-left: 18px;
		color: #c3ccd9;
		font-size: 13px;
		line-height: 1.8;
	}
	.figures > summary {
		cursor: pointer;
		padding: 8px 0;
		color: #8b94a5;
		font-size: 12.5px;
	}
	.figures > summary:hover {
		color: #c3ccd9;
	}
	.legend {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: 10px;
		margin: 4px 0 8px;
	}
	.metric {
		padding: 8px 10px;
		border: 1px solid #2b3140;
		border-radius: 4px;
	}
	.metric.collapsed {
		background: rgba(43, 49, 64, 0.35);
	}
	.metric dt {
		color: #d6dde8;
		font-size: 12px;
		font-weight: 600;
	}
	.metric dt code {
		color: #7d8496;
		font-weight: 400;
		font-size: 11px;
	}
	.metric dd {
		margin: 4px 0 0;
		font-size: 11px;
		line-height: 1.5;
		color: #8b94a5;
	}
	.metric .range {
		color: #aab4c0;
		font-variant-numeric: tabular-nums;
	}
	.metric .finding {
		color: #d9b26a;
	}
	.table-scroll {
		overflow-x: auto;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 12px;
	}
	th,
	td {
		padding: 5px 10px;
		text-align: left;
		border-bottom: 1px solid #232838;
	}
	thead th {
		position: sticky;
		top: 0;
		background: #1b1e26;
		color: #8b94a5;
		font-weight: 600;
		white-space: nowrap;
	}
	tbody th {
		font-weight: 400;
		color: #d6dde8;
		max-width: 460px;
	}
	tbody th.unnamed {
		color: #7d8496;
		font-style: italic;
	}
	td {
		color: #aab4c0;
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
	}
	td.absent {
		color: #5d6474;
	}
	a {
		color: #8fb6e8;
	}
</style>
