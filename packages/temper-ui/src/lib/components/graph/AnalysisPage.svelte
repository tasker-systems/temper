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
	import { graphAnalysisHref, graphHref, resourceHref } from '$lib/vault-url';

	let { data }: { data: AnalysisViewData } = $props();

	const reports = $derived(reportMetrics(data.regions));
	const columns = $derived(reports.filter((r) => r.asColumn));
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
					{data.refusal.named === 1 ? 'The place' : 'None of the places'} named in this link
					{data.refusal.named === 1 ? 'is' : 'is'} readable by you, so there is nothing here to
					measure. It may have been removed, or never shared with you.
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
		<h1>{data.place.title}</h1>

		{#if data.alsoNamed.length > 0}
			<p class="also" data-testid="also-named">
				You named {data.alsoNamed.length + 1} places. This measures one at a time — the others are
				{#each data.alsoNamed as p, i (p.ref)}<a
						href={graphAnalysisHref(data.owner, { kind: p.kind, ref: p.ref })}>{p.title}</a
					>{i < data.alsoNamed.length - 1 ? ', ' : '.'}{/each}
			</p>
		{/if}

		<section class="map-level" aria-labelledby="map-level-h">
			<h2 id="map-level-h">What this place says it is for</h2>
			{#if data.map}
				<p>
					Its charter is <a href={resourceHref({ id: data.map.telos.id })}
						>{data.map.telos.title ?? 'the charter resource'}</a
					>.
				</p>
				<p data-testid="staleness">{describeStaleness(data.map.staleness)}</p>
				<p data-testid="regulation">{describeRegulation(data.map.regulation.length)}</p>
				{#if data.map.regulation.length > 0}
					<ul class="regulation">
						{#each data.map.regulation as r (r.resource_id)}
							<li><a href={resourceHref({ id: r.resource_id })}>{r.title}</a></li>
						{/each}
					</ul>
				{/if}
			{:else if data.place.kind === 'context'}
				<!-- Declared, not fabricated. D6 is unshipped and a context has no charter and no
				     regulation set even in principle; inventing a peer field is what the task
				     explicitly forbids. -->
				<p class="declared-absent" data-testid="map-absent">{CONTEXT_HAS_NO_MAP_READOUT}</p>
			{:else}
				<p class="declared-absent" data-testid="map-absent">{MAP_READOUT_UNAVAILABLE}</p>
			{/if}
		</section>

		<section class="groupings" aria-labelledby="groupings-h">
			<h2 id="groupings-h">How its work has been grouped</h2>
			<p class="lead" data-testid="grouping-count">{describeGroupingCount(data.regions.length)}</p>

			{#if !data.metricsAvailable}
				<p class="unavailable" role="status" data-testid="metrics-unavailable">
					{METRICS_UNAVAILABLE}
				</p>
			{/if}

			{#if data.regions.length > 0}
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
							{#each data.regions as r (`${r.regionId}|${r.lensId}`)}
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
			{/if}
		</section>
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
