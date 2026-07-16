<script lang="ts">
	import type { EventTrail } from '$lib/types/generated/element_trail';
	import { trailModel } from '$lib/graph/atlas/trail';
	import { summarizeEvent } from '$lib/graph/atlas/eventSummary';
	import { flattenPayload } from '$lib/graph/atlas/payloadRows';
	import { relativeTime } from '$lib/graph/atlas/relativeTime';

	let { trail }: { trail: EventTrail | null } = $props();

	// trailModel takes a non-null EventTrail — guard here, don't widen it.
	let rows = $derived(trail ? trailModel(trail) : []);
	let openEvent = $state<string | null>(null);
</script>

<section>
	<div class="label">History · {rows.length}</div>
	{#if rows.length === 0}
		<p class="empty">No recorded history.</p>
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
	.empty {
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 11px;
		color: var(--color-quiet-dim);
		margin: 0;
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
