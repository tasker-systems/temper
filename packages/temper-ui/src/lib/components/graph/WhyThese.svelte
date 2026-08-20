<script lang="ts">
	/**
	 * The *why-these* readout — machine reasoning about the answer, and never a thing in the graph.
	 *
	 * **This panel is the only place derived structure appears, and a reader can confirm that by
	 * looking**: everything on the canvas is a resource they wrote and an edge that exists in the
	 * database, and everything a machine decided is in here. That is
	 * `no-derived-thing-poses-as-authored` as a property of the layout, not a rule someone applies.
	 *
	 * Every sentence is built by `readout.ts`, which is pure and tested — including the rule that
	 * none of them may contain the words the substrate uses for these things, and the rule that a
	 * grouping the lookup could not find reads as *re-derived* rather than as an error.
	 */
	import {
		type Readout,
		describeGrouping,
		describeReadout,
		describeWithheld,
		listGroupings,
	} from '$lib/graph/readout';

	let { readout, question }: { readout: Readout; question: string | null } = $props();

	const lead = $derived(describeReadout(readout));
	const listed = $derived(listGroupings(readout));
</script>

<aside class="why" aria-label="Why these">
	<h2>Why these</h2>

	{#if question}
		<p class="asked">You asked: <em>{question}</em></p>
	{:else}
		<p class="asked">No question was asked, so everything in the places you named is the answer.</p>
	{/if}

	<p class="lead">{lead}</p>

	{#if listed.shown.length}
		<ul class="groupings">
			{#each listed.shown as g (g.id)}
				<li class:absent={g.name.state !== 'named'}>{describeGrouping(g)}</li>
			{/each}
		</ul>
		{#if listed.withheld > 0}
			<p class="withheld">{describeWithheld(listed.withheld)}</p>
		{/if}
	{/if}

	<details class="accounting">
		<summary>What each step was handed</summary>
		<ul>
			{#each readout.stages as s (s.stage)}
				<li>
					<span class="act">{s.act}</span>
					<span class="n">{s.handed} handed</span>
					{#if s.unusable > 0}<span class="n unusable">{s.unusable} did not contribute</span>{/if}
				</li>
			{/each}
		</ul>
	</details>
</aside>

<style>
	.why {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 14px;
		overflow: auto;
		border-left: 1px solid rgba(255, 255, 255, 0.07);
		background: rgba(255, 255, 255, 0.015);
		color: #9aa3b0;
		font-size: 13px;
		line-height: 1.5;
	}
	h2 {
		margin: 0;
		font:
			9px/1 ui-monospace,
			Menlo,
			monospace;
		letter-spacing: 0.18em;
		text-transform: uppercase;
		color: #6f7886;
	}
	p {
		margin: 0;
	}
	.asked {
		color: #c3cbd6;
	}
	.asked em {
		font-family: Georgia, serif;
		font-style: italic;
	}
	.lead {
		color: #8b95a3;
	}
	.groupings {
		margin: 0;
		padding: 0;
		list-style: none;
		display: grid;
		gap: 6px;
	}
	.groupings li {
		padding: 7px 9px;
		border-left: 2px solid rgba(180, 120, 200, 0.55);
		background: rgba(180, 120, 200, 0.06);
		border-radius: 0 5px 5px 0;
		color: #c3cbd6;
		font-size: 12.5px;
	}
	/* A grouping that is gone or unchecked is stated quietly — it is information about the
	   machine's own bookkeeping, never a fault the reader is being shown. */
	.groupings li.absent {
		border-left-color: rgba(255, 255, 255, 0.12);
		background: rgba(255, 255, 255, 0.02);
		color: #79828f;
		font-style: italic;
	}
	.withheld {
		color: #79828f;
		font-size: 12px;
		font-style: italic;
	}
	.accounting {
		margin-top: 2px;
		font-size: 12px;
	}
	.accounting summary {
		cursor: pointer;
		color: #6f7886;
		font:
			9px/1.6 ui-monospace,
			Menlo,
			monospace;
		letter-spacing: 0.14em;
		text-transform: uppercase;
	}
	.accounting ul {
		margin: 8px 0 0;
		padding: 0;
		list-style: none;
		display: grid;
		gap: 4px;
	}
	.accounting li {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 10px;
		align-items: baseline;
	}
	.act {
		color: #c3cbd6;
		font-family: ui-monospace, Menlo, monospace;
		font-size: 11px;
	}
	.n {
		color: #79828f;
		font-size: 11px;
		font-variant-numeric: tabular-nums;
	}
	.unusable {
		color: #b58b6a;
	}
</style>
