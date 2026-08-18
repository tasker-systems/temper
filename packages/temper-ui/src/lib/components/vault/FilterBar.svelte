<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { untrack } from 'svelte';
	import type { ContextRowWithCounts } from '$lib/types';
	import {
		draftNavigationSettled,
		draftNavigationStarted,
		draftTyped,
		draftUrlChanged,
		newDraft
	} from '$lib/vault-draft';
	import { buildFilterUrl, STAGES, STATUSES, type VaultFilters } from '$lib/vault-filters';

	interface Props {
		filters: VaultFilters;
		revealed: string | null;
		fixedContext: boolean;
		/**
		 * Contexts for the Context select, or `null` when the layout could not read them.
		 * `null` is not an empty list: an empty list says "there is nothing to filter by",
		 * which is exactly the claim a failed fetch may not make.
		 */
		contexts: ContextRowWithCounts[] | null;
	}

	let { filters, revealed, fixedContext, contexts }: Props = $props();

	// Local drafts for the free-text controls so keystrokes don't each trigger a navigation.
	// The URL (via `filters`) is still the source of truth, but only when this control has
	// nothing outstanding — see `vault-draft.ts` for why a bare resync ate keystrokes.
	let q = $state(untrack(() => newDraft(filters.q ?? '')));
	let tagDraft = $state('');
	let qDebounce: ReturnType<typeof setTimeout>;

	$effect(() => {
		const incoming = filters.q ?? '';
		q = draftUrlChanged(untrack(() => q), incoming);
	});

	// Returns the navigation so the `q` draft can tell when its own round-trip has settled —
	// until then an incoming URL value is this control's own echo, not an external change.
	function navigate(patch: Partial<VaultFilters>): Promise<void> {
		return goto(buildFilterUrl($page.url, patch), { replaceState: true });
	}

	function onQInput(event: Event) {
		q = draftTyped(q, (event.currentTarget as HTMLInputElement).value);
		clearTimeout(qDebounce);
		qDebounce = setTimeout(() => {
			q = draftNavigationStarted(q);
			navigate({ q: q.requested || null }).finally(() => {
				q = draftNavigationSettled(q);
			});
		}, 300);
	}

	function onContextChange(event: Event) {
		const value = (event.target as HTMLSelectElement).value;
		navigate({ contextRef: value || null });
	}

	function onStageChange(event: Event) {
		const value = (event.target as HTMLSelectElement).value;
		navigate({ stage: value || null });
	}

	function onStatusChange(event: Event) {
		const value = (event.target as HTMLSelectElement).value;
		navigate({ status: value || null });
	}

	function addTagDraft() {
		const name = tagDraft.trim();
		tagDraft = '';
		if (!name || filters.tags.includes(name)) return;
		navigate({ tags: [...filters.tags, name] });
	}

	function onTagKeydown(event: KeyboardEvent) {
		if (event.key !== 'Enter') return;
		event.preventDefault();
		addTagDraft();
	}

	function removeTag(name: string) {
		navigate({ tags: filters.tags.filter((t) => t !== name) });
	}
</script>

<div class="flex flex-wrap items-end gap-4">
	<div class="flex flex-col gap-1">
		<label for="filter-q" class="text-[10px] uppercase tracking-widest text-zinc-500"
			>title contains</label
		>
		<input
			id="filter-q"
			type="text"
			value={q.draft}
			oninput={onQInput}
			placeholder="title contains…"
			class="w-48 rounded border border-zinc-700/50 bg-zinc-800/50 px-2.5 py-1 text-xs text-zinc-200
			       placeholder:text-zinc-600 outline-none focus:border-quiet-border"
		/>
	</div>

	{#if !fixedContext}
		<div class="flex flex-col gap-1">
			<label for="filter-context" class="text-[10px] uppercase tracking-widest text-zinc-500"
				>context</label
			>
			<!--
				A failed context read renders as unavailable, not as "no contexts". An empty
				select reads as "there is nothing to filter by" — a claim about the vault that a
				fetch which never answered cannot support. The sidebar can degrade to an empty
				list (a context that doesn't render is simply not offered); a filter control
				cannot, because absence there is an assertion.
			-->
			{#if contexts === null}
				<select
					id="filter-context"
					disabled
					title="The context list could not be read — reload to try again."
					class="rounded border border-amber-500/30 bg-zinc-800/50 px-2.5 py-1 text-xs text-zinc-500
					       outline-none"
				>
					<option value="">contexts unavailable</option>
				</select>
			{:else}
				<select
					id="filter-context"
					value={filters.contextRef ?? ''}
					onchange={onContextChange}
					class="rounded border border-zinc-700/50 bg-zinc-800/50 px-2.5 py-1 text-xs text-zinc-200
					       outline-none focus:border-quiet-border"
				>
					<option value="">All contexts</option>
					{#each contexts as ctx (ctx.id)}
						<option value={`${ctx.owner_ref}/${ctx.slug}`}>{ctx.name}</option>
					{/each}
				</select>
			{/if}
		</div>
	{/if}

	<div class="flex flex-col gap-1">
		<label for="filter-tags" class="text-[10px] uppercase tracking-widest text-zinc-500">tags</label>
		<div
			class="flex min-w-48 flex-wrap items-center gap-1.5 rounded border border-zinc-700/50 bg-zinc-800/50 px-2 py-1"
		>
			{#each filters.tags as tag (tag)}
				<span
					class="inline-flex items-center gap-1 rounded bg-quiet-accent/15 px-1.5 py-0.5 text-[11px] font-mono text-quiet-accent"
				>
					{tag}
					<button
						type="button"
						class="text-quiet-accent/70 hover:text-quiet-accent"
						onclick={() => removeTag(tag)}
						aria-label={`Remove tag ${tag}`}
					>
						×
					</button>
				</span>
			{/each}
			<input
				id="filter-tags"
				type="text"
				bind:value={tagDraft}
				onkeydown={onTagKeydown}
				onblur={addTagDraft}
				placeholder={filters.tags.length ? '' : 'add tag…'}
				class="min-w-16 flex-1 bg-transparent text-xs text-zinc-200 placeholder:text-zinc-600 outline-none"
			/>
		</div>
	</div>

	{#if revealed === 'task'}
		<div class="flex flex-col gap-1">
			<label for="filter-stage" class="text-[10px] uppercase tracking-widest text-zinc-500"
				>stage</label
			>
			<select
				id="filter-stage"
				value={filters.stage ?? ''}
				onchange={onStageChange}
				class="rounded border border-zinc-700/50 bg-zinc-800/50 px-2.5 py-1 text-xs text-zinc-200
				       outline-none focus:border-quiet-border"
			>
				<option value="">Any stage</option>
				{#each STAGES as s}
					<option value={s}>{s}</option>
				{/each}
			</select>
		</div>
	{:else if revealed === 'goal'}
		<div class="flex flex-col gap-1">
			<label for="filter-status" class="text-[10px] uppercase tracking-widest text-zinc-500"
				>status</label
			>
			<select
				id="filter-status"
				value={filters.status ?? ''}
				onchange={onStatusChange}
				class="rounded border border-zinc-700/50 bg-zinc-800/50 px-2.5 py-1 text-xs text-zinc-200
				       outline-none focus:border-quiet-border"
			>
				<option value="">Any status</option>
				{#each STATUSES as s}
					<option value={s}>{s}</option>
				{/each}
			</select>
		</div>
	{/if}
</div>
