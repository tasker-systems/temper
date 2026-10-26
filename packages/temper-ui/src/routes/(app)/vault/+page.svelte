<script lang="ts">
	/**
	 * The first view (`$lib/first-view`): the reader's latest handoff, the places they work in, their
	 * standing ways in, and what was recently updated. Each region renders its own read's state — arriving, failed, given
	 * up, empty — so one list's failure never reads as another's absence, and every list ends with
	 * the shared omission sentence and the order it is read in.
	 */
	import { page } from '$app/stores';
	import { HANDOFF_LABEL } from '@tasker-systems/temper-design/handoff';
	import { omissionSentence } from '@tasker-systems/temper-design/omission';
	import { WAY_IN_BOUND, WAY_IN_ORDER } from '@tasker-systems/temper-design/ways-in';
	import RegionState from '$lib/components/RegionState.svelte';
	import ResourceLink from '$lib/components/ResourceLink.svelte';
	import {
		type FirstViewList,
		PLACES_ORDER,
		PLACES_SCOPE,
		RECENT_SCOPE,
		WAY_IN_MAX,
		wayInMoreHref,
	} from '$lib/first-view';
	import { relativeTime } from '$lib/graph/relativeTime';
	import { regionStateFor } from '$lib/region';
	import type { ResourceView } from '$lib/types';
	import { contextHref } from '$lib/vault-url';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();

	/** The handoff is the reader's own; every state of its region says so. */
	const handoffRegion = `your ${HANDOFF_LABEL}`;
</script>

<svelte:head>
	<title>Home — temper</title>
</svelte:head>

{#snippet rows(list: ResourceView[])}
	<ul class="divide-y divide-tp-rule">
		{#each list as row (row.id)}
			<li class="flex items-baseline justify-between gap-3 py-1.5 min-w-0">
				<ResourceLink
					id={row.id}
					title={row.title}
					class="truncate text-sm text-tp-text hover:text-tp-accent"
				/>
				<span class="flex-none font-mono text-[10px] text-tp-text-subtle">{row.doc_type_name}</span>
			</li>
		{/each}
	</ul>
{/snippet}

{#snippet foot(list: FirstViewList, scope: string)}
	<p class="mt-2 text-xs text-tp-text-subtle" data-testid="omission">
		{omissionSentence(list.total, list.rows.length, scope)}
		<span>Ordered {WAY_IN_ORDER}.</span>
	</p>
{/snippet}

<div class="mx-auto max-w-5xl px-6 py-8 space-y-8">
	<header>
		<h1 class="text-lg text-tp-text">Home</h1>
		<p class="text-xs text-tp-text-subtle">Where your own work stands.</p>
	</header>

	<section aria-labelledby="handoff-heading" class="rounded border border-tp-rule bg-tp-surface p-4">
		<h2 id="handoff-heading" class="mb-2 text-[10px] uppercase tracking-widest text-tp-text-subtle">
			{handoffRegion}
		</h2>
		{#await data.session}
			<RegionState state="arriving" label={handoffRegion} />
		{:then session}
			{#if session}
				<!-- "Latest" is by last update, which is what the read orders by; the age says when. -->
				<p class="mb-1 text-[10px] text-tp-text-subtle" data-testid="handoff-age">
					session · updated {relativeTime(session.updated)}
				</p>
				<ResourceLink id={session.id} title={session.title} class="text-sm text-tp-text hover:text-tp-accent" />
				{#await data.next}
					<RegionState state="arriving" label="next steps" />
				{:then next}
					{#if next}
						<figure class="mt-3" data-testid="next-steps">
							<figcaption class="mb-1 text-[10px] uppercase tracking-widest text-tp-text-subtle">
								excerpt · {next.heading}
							</figcaption>
							<blockquote class="whitespace-pre-line border-l-2 border-tp-accent-line pl-3 text-sm italic text-tp-text">
								{next.text}
							</blockquote>
							{#if next.truncated}
								<p class="mt-1 text-xs italic text-tp-text-subtle">… continues in the session.</p>
							{/if}
						</figure>
					{/if}
				{:catch error}
					<RegionState state={regionStateFor(error)} label="next steps" />
				{/await}
			{:else}
				<RegionState
					state="empty"
					label="sessions recorded yet"
					detail="Sessions you record with an agent appear here, and their next steps with them."
				/>
			{/if}
		{:catch error}
			<RegionState state={regionStateFor(error)} label={handoffRegion} />
		{/await}
	</section>

	<!-- Resume — the work you last changed — belongs beside the handoff. It is not drawn until a
	     read of what the reader changed exists; see `$lib/first-view`. -->

	<!-- Ordered by the reader's own changes, and only here: the nav's places keep their order. The
	     list is drawn once per load, so it never moves under a reader using it. -->
	<section aria-labelledby="places-heading" data-testid="places">
		<h2 id="places-heading" class="mb-2 text-[10px] uppercase tracking-widest text-tp-text-subtle">
			places you work in
		</h2>
		{#await data.places}
			<RegionState state="arriving" label={PLACES_SCOPE} />
		{:then places}
			{#if places.total === 0}
				<RegionState
					state="empty"
					label={PLACES_SCOPE}
					detail="Places where you, or an agent working as you, change something appear here."
				/>
			{:else}
				<ul class="divide-y divide-tp-rule">
					{#each places.rows as { context, lastChangedAt } (context.id)}
						<li class="flex items-baseline justify-between gap-3 py-1.5 min-w-0">
							<a
								href={contextHref(context.owner_ref, context.slug)}
								class="truncate text-sm text-tp-text hover:text-tp-accent"
							>
								{context.name}
							</a>
							<span class="flex-none text-[10px] text-tp-text-subtle" data-testid="place-changed">
								you changed {relativeTime(lastChangedAt)}
							</span>
						</li>
					{/each}
				</ul>
				<p class="mt-2 text-xs text-tp-text-subtle" data-testid="omission">
					{omissionSentence(places.total, places.rows.length, PLACES_SCOPE)}
					<span>Ordered {PLACES_ORDER}.</span>
				</p>
			{/if}
		{:catch error}
			<RegionState state={regionStateFor(error)} label={PLACES_SCOPE} />
		{/await}
	</section>

	<section aria-label="Ways in" class="grid gap-6 md:grid-cols-3">
		{#each data.ways as { way, shown, list } (way.id)}
			<div class="min-w-0" data-testid="way-{way.id}">
				<h2 class="mb-2 text-[10px] uppercase tracking-widest text-tp-text-subtle">{way.label}</h2>
				{#await list}
					<RegionState state="arriving" label={way.scope} />
				{:then got}
					{#if got.total === 0}
						<RegionState state="empty" label={way.scope} />
					{:else}
						{@render rows(got.rows)}
						{@render foot(got, way.scope)}
						{#if got.total > got.rows.length && shown < WAY_IN_MAX}
							<a
								href={wayInMoreHref($page.url, way, shown)}
								data-sveltekit-noscroll
								class="mt-1 inline-block text-xs text-tp-accent hover:text-tp-text"
							>
								Show {Math.min(WAY_IN_BOUND, got.total - got.rows.length, WAY_IN_MAX - shown)} more
							</a>
						{:else if got.total > got.rows.length}
							<p class="mt-1 text-xs text-tp-text-subtle">
								The first view shows at most {WAY_IN_MAX}.
							</p>
						{/if}
					{/if}
				{:catch error}
					<RegionState state={regionStateFor(error)} label={way.scope} />
				{/await}
			</div>
		{/each}
	</section>

	<section aria-labelledby="recent-heading" data-testid="recent">
		<h2 id="recent-heading" class="mb-2 text-[10px] uppercase tracking-widest text-tp-text-subtle">
			recently updated
		</h2>
		{#await data.recent}
			<RegionState state="arriving" label="recently updated" />
		{:then recent}
			{#if recent.total === 0}
				<RegionState state="empty" label="resources you can read" />
			{:else}
				{@render rows(recent.rows)}
				{@render foot(recent, RECENT_SCOPE)}
				<a href="/vault/all" class="mt-1 inline-block text-xs text-tp-accent hover:text-tp-text">
					All resources →
				</a>
			{/if}
		{:catch error}
			<RegionState state={regionStateFor(error)} label="recently updated" />
		{/await}
	</section>
</div>
