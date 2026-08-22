<script lang="ts">
	import type { EventTrail } from '$lib/types/generated/element_trail';
	import { trailModel } from '$lib/graph/trail';
	import { summarizeEvent } from '$lib/graph/eventSummary';
	import { flattenPayload } from '$lib/graph/payloadRows';
	import { relativeTime } from '$lib/graph/relativeTime';
	import RegionState from '$lib/components/RegionState.svelte';

	/**
	 * `trail` is no longer nullable. It used to be, because a failed read degraded to `null` in the
	 * load — and that made a failure indistinguishable from a resource with genuinely no history.
	 * Failure now travels to the page's `{:catch}`, so the only value that reaches here is one a
	 * read actually returned.
	 */
	let { trail }: { trail: EventTrail } = $props();

	let rows = $derived(trailModel(trail));
	let openEvent = $state<string | null>(null);
</script>

<section>
	<div class="label">History · {rows.length}</div>
	<!--
		The emptiness verdict stays HERE rather than in the page, because the count beside it comes
		from the same `rows`: one derivation decides both what this heading says and whether the
		region is empty, so they cannot disagree. Deciding emptiness in the page would be a second
		predicate over the same data, and two predicates drift.

		`[corrected — 2026-08-21]` This comment used to justify itself by saying `trailModel` filters,
		so a trail whose events all filter out would still be empty. **It does not filter** —
		`trail.ts:23` reverses and maps 1:1, so `rows.length === trail.events.length` always, and that
		state is unreachable. The rule survives its wrong reason; the reason is replaced rather than
		quietly dropped, because it is the kind of claim that gets built on.

		The words come from `RegionState` so this region cannot drift away from every other one.
	-->
	{#if rows.length === 0}
		<RegionState state="empty" label="history" />
	{:else}
		{#each rows.slice(0, 50) as row (row.id)}
			<!-- summarizeEvent resolves relationship targets through an optional node
			     map; the vault page loads no subgraph, so it is omitted and the
			     summary line is skipped for the events that would need it. -->
			{@const summary = summarizeEvent(row.rawKind, row.payload)}
			<div class="event">
				<button
					class="head"
					aria-expanded={openEvent === row.id}
					onclick={() => (openEvent = openEvent === row.id ? null : row.id)}
				>
					<span class="kind">{row.kind}</span>
					<span class="chev">{openEvent === row.id ? '⌄' : '›'}</span>
				</button>
				{#if summary}<div class="summary">{summary}</div>{/if}
				<div class="meta">
					{row.actorName} · {relativeTime(row.occurredAt)}{#if row.confidence}
						· <span class="conf">{row.confidence}</span>{/if}
				</div>
				{#if openEvent === row.id}
					<dl class="payload">
						{#each flattenPayload(row.payload) as pr (pr.key)}
							<div><dt>{pr.key}</dt><dd>{pr.value}</dd></div>
						{/each}
					</dl>
				{/if}
			</div>
		{/each}
	{/if}
</section>

<style>
	section {
		padding: 12px 14px;
		border-top: 1px solid var(--color-quiet-rule);
	}
	section:first-child {
		border-top: 0;
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 6px;
	}
	.event {
		padding: 4px 0;
	}
	.head {
		display: flex;
		justify-content: space-between;
		align-items: center;
		width: 100%;
		background: none;
		border: 0;
		padding: 0;
		cursor: pointer;
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: color-mix(in srgb, var(--hue) 70%, white);
	}
	.chev {
		color: var(--color-quiet-dim);
	}
	.summary {
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 11px;
		color: var(--color-quiet-mid);
		margin: 1px 0;
	}
	.meta {
		font-family: var(--font-mono);
		font-size: 9px;
		color: var(--color-quiet-dim);
	}
	.conf {
		color: #8fd8a8;
	}
	.payload {
		margin: 4px 0 0;
		border-left: 1px solid color-mix(in srgb, var(--hue) 25%, transparent);
		padding-left: 8px;
	}
	.payload div {
		display: grid;
		grid-template-columns: 84px 1fr;
		gap: 6px;
	}
	.payload dt,
	.payload dd {
		font-family: var(--font-mono);
		font-size: 9px;
		margin: 0;
		word-break: break-word;
	}
	.payload dt {
		color: var(--color-quiet-dim);
	}
	.payload dd {
		color: var(--color-quiet-mid);
	}
</style>
