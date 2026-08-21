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
	import type { NamedPlace } from '$lib/graph/entry';
	import { graphAnalysisHref } from '$lib/vault-url';

	/**
	 * `readout` is `null` on a **traversed** view, and the panel becomes PROVENANCE rather than
	 * disappearing — §7.2's recommended option of three.
	 *
	 * *"Declare itself as the grounding that the current view descends from… It stops claiming to
	 * explain the current screen and becomes provenance for it."* Disappearing was the second-best,
	 * and it was rejected because it *"loses the reader's route back to how they got here."*
	 *
	 * `[ruled — 2026-08-21, Pete]` **Provenance only.** The question, the places and the route back
	 * survive; the stage accounting and the grouping list do not — they were measured for a screen
	 * the reader has left, and no composition ran for this one.
	 */
	let {
		readout,
		question,
		owner,
		places,
		backHref = null,
	}: {
		readout: Readout | null;
		question: string | null;
		owner: string;
		places: NamedPlace[];
		/** The grounding this view descends from. Non-null exactly when this is a traversal. */
		backHref?: string | null;
	} = $props();

	const lead = $derived(readout ? describeReadout(readout) : null);
	const listed = $derived(readout ? listGroupings(readout) : null);
</script>

<!--
	The heading is part of the claim, so it changes with the rest. `Why these` asserts that what
	follows explains the marks on screen; on a traversed view it does not, and it would be false in
	exactly the way `REACHED` was — a string asserting a reader act that did not happen on the view
	rendering it.
-->
<aside class="why" aria-label={readout ? 'Why these' : 'Where you started'}>
	<h2>{readout ? 'Why these' : 'Where you started'}</h2>

	{#if readout}
		{#if question}
			<p class="asked">You asked: <em>{question}</em></p>
		{:else}
			<p class="asked">
				No question was asked, so everything in the places you named is the answer.
			</p>
		{/if}
	{:else if question}
		<!--
			Past tense, and deliberately so. §4: the walk is NOT confined to the grounding's result
			set — `traversal_slice` runs over the reader's whole visible corpus — so `q` is where the
			reader STARTED and never a filter still in force. "You asked" in the present would say
			the question is still narrowing, which is the thing this panel exists to stop saying.
		-->
		<p class="asked">You started from this question: <em>{question}</em></p>
	{:else}
		<p class="asked">You started from the shape of your work, without asking a question.</p>
	{/if}

	{#if !readout}
		<p class="lead">
			These marks were reached by following edges from where you were — not by asking again.
		</p>
	{/if}

	{#if backHref}
		<p class="back"><a href={backHref}>← Back to where you started</a></p>
	{/if}

	{#if lead}<p class="lead">{lead}</p>{/if}

	{#if listed && listed.shown.length}
		<ul class="groupings">
			{#each listed.shown as g (g.id)}
				<li class:absent={g.name.state !== 'named'}>{describeGrouping(g)}</li>
			{/each}
		</ul>
		{#if listed.withheld > 0}
			<p class="withheld">{describeWithheld(listed.withheld)}</p>
		{/if}
	{/if}

	{#if places.length > 0}
		<!-- The receiver, reached. Beat B took the per-region measurements off the canvas; a place
		     they are merely stored in is not what `displaced-structure-remains-reachable` asks for,
		     so every place this answer drew on links to its own measurements. One link per place
		     rather than one per grouping, because a grouping's id is resolved from a flat set and
		     carries no anchor — deliberately, so the readout needs no per-kind branch. -->
		<p class="measured" data-testid="measured-links">
			<!--
				On a traversal these describe the GROUNDING, not the marks on screen, and they survive
				"under a sentence that says so" (§6) rather than being dropped. Dropping them is what
				would break `displaced-structure-remains-reachable`: the analysis door has to stay
				reachable without the reader being told a URL.
			-->
			{readout ? 'How these were measured:' : 'How your starting places were measured:'}
			{#each places as p, i (`${p.kind}:${p.ref}`)}<a
					href={graphAnalysisHref(owner, { kind: p.kind, ref: p.ref })}>{p.title}</a
				>{i < places.length - 1 ? ' · ' : ''}{/each}
		</p>
	{/if}

	<!--
		The stage accounting is composition-only, and not merely irrelevant on a traversal — there
		are no stages. Rendering an empty `What each step was handed` would be a disclosure about a
		pipeline that never ran.
	-->
	{#if readout}
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
	{/if}
</aside>

<style>
	.measured {
		margin: 0;
		font-size: 11px;
		line-height: 1.7;
		color: #8b94a5;
	}
	.measured a {
		color: #8fb6e8;
	}
	.back {
		margin: 0;
		font-size: 12px;
	}
	.back a {
		color: #8fb6e8;
	}
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
