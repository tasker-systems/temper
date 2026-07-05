<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { AtlasSearchHit } from '$lib/types/generated/graph_atlas';
	import { docTypeHue } from '$lib/graph/atlas/palette';
	import { buildDrillNodeUrl } from '$lib/graph/atlas/nav';

	interface Props {
		teamId: string;
	}
	let { teamId }: Props = $props();

	let q = $state('');
	let hits = $state<AtlasSearchHit[]>([]);
	let timer: ReturnType<typeof setTimeout> | null = null;

	function onInput() {
		if (timer) clearTimeout(timer);
		const term = q.trim();
		if (term.length === 0) {
			hits = [];
			return;
		}
		timer = setTimeout(async () => {
			const res = await fetch(`/graph/_search?team=${teamId}&q=${encodeURIComponent(term)}`);
			hits = res.ok ? await res.json() : [];
		}, 180);
	}

	function jump(hit: AtlasSearchHit) {
		goto(buildDrillNodeUrl($page.url, hit.node_id), { replaceState: true });
		q = '';
		hits = [];
	}
</script>

<div class="search" data-testid="atlas-search">
	<input placeholder="Find a node…" bind:value={q} oninput={onInput} />
	{#if hits.length}
		<ul>
			{#each hits as h (h.node_id)}
				<li>
					<button type="button" onclick={() => jump(h)}>
						<span
							class="dot"
							style="background: {h.home === 'cogmap'
								? docTypeHue(h.doc_type)
								: 'transparent'}; border-color: {docTypeHue(h.doc_type)}"
						></span>
						<span class="t">{h.title}</span>
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</div>

<style>
	.search {
		padding: 10px 12px;
	}
	input {
		width: 100%;
		box-sizing: border-box;
		background: #14171d;
		border: 1px solid #2a2f38;
		color: var(--color-quiet-ink, #c9d1d9);
		border-radius: 6px;
		padding: 6px 9px;
		font-size: 13px;
	}
	ul {
		list-style: none;
		margin: 6px 0 0;
		padding: 0;
		max-height: 220px;
		overflow-y: auto;
	}
	li button {
		display: flex;
		gap: 8px;
		align-items: center;
		width: 100%;
		text-align: left;
		background: none;
		border: 0;
		padding: 5px 4px;
		cursor: pointer;
		color: var(--color-quiet-ink, #c9d1d9);
		font-size: 12px;
	}
	li button:hover {
		background: rgba(255, 255, 255, 0.03);
	}
	.dot {
		width: 9px;
		height: 9px;
		border-radius: 50%;
		border: 2px solid;
		flex: 0 0 auto;
	}
</style>
