<script lang="ts">
	/**
	 * The first view (`$lib/first-view`): the work the reader last changed, with its opening; the
	 * places they work in, their standing ways in, and what was recently updated.
	 * Each region renders its own read's state — arriving, failed, given up, empty — so one list's
	 * failure never reads as another's absence, and every list ends with the shared omission
	 * sentence and the order it is read in.
	 */
	import type { Snippet } from 'svelte';
	import { page } from '$app/stores';
	import { omissionSentence } from '@tasker-systems/temper-design/omission';
	import { WAY_IN_BOUND, WAY_IN_ORDER } from '@tasker-systems/temper-design/ways-in';
	import RegionState from '$lib/components/RegionState.svelte';
	import MarkdownRenderer from '$lib/components/MarkdownRenderer.svelte';
	import ResourceLink from '$lib/components/ResourceLink.svelte';
	import {
		FIRST_RUN_HELP_URL,
		type FirstViewList,
		isFirstRun,
		PLACES_ORDER,
		PLACES_SCOPE,
		type PlacesList,
		placesInUse,
		RECENT_EMPTY_DETAIL,
		RECENT_SCOPE,
		RESUME_LABEL,
		WAY_IN_EMPTY_DETAIL,
		WAY_IN_MAX,
		wayInMoreHref,
	} from '$lib/first-view';
	import { relativeTime } from '$lib/graph/relativeTime';
	import { regionStateFor } from '$lib/region';
	import type { ResourceView } from '$lib/types';
	import { contextHref } from '$lib/vault-url';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();

	/**
	 * The places, named from the contexts the layout read. A failed contexts read (`null`) fails this
	 * region through `placesInUse`, rather than leaving it to claim the reader changed nothing.
	 *
	 * `.then` makes a new promise, which nothing has subscribed to while the server renders the
	 * pending state, so it carries its own catch, as `derive` in `$lib/server/bounded` does for a
	 * load. The `{:catch}` below still sees the rejection.
	 */
	const places: Promise<PlacesList> = $derived.by(() => {
		const joined = data.places.then((activity) => placesInUse(activity, data.contexts));
		joined.catch(() => {});
		return joined;
	});

	/** Whether the reader has nothing yet, from the same reads the regions draw (`isFirstRun`). */
	const firstRun: Promise<boolean> = $derived(
		isFirstRun(data.resume, places, [...data.ways.map((w) => w.list), data.recent]),
	);
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

<!-- Resume's region: drawn only when it has something to say, see below. -->
{#snippet resumeRegion(content: Snippet)}
	<section
		aria-labelledby="resume-heading"
		data-testid="resume"
		class="min-w-0 rounded border border-tp-rule bg-tp-surface p-4"
	>
		<h2 id="resume-heading" class="mb-2 text-[10px] uppercase tracking-widest text-tp-text-subtle">
			{RESUME_LABEL}
		</h2>
		{@render content()}
	</section>
{/snippet}

{#snippet foot(list: { rows: unknown[]; total: number }, scope: string, order: string = WAY_IN_ORDER)}
	<p class="mt-2 text-xs text-tp-text-subtle" data-testid="omission">
		{omissionSentence(list.total, list.rows.length, scope)}
		<span>Ordered {order}.</span>
	</p>
{/snippet}

<div class="mx-auto max-w-5xl px-6 py-8 space-y-8">
	<header>
		<h1 class="text-lg text-tp-text">Home</h1>
		<p class="text-xs text-tp-text-subtle">Where your own work stands.</p>
	</header>

	<!-- A reader with nothing yet is told how work arrives, since nothing on this page can make
	     any. Drawn only once every read has settled empty: while one is arriving, or when one
	     failed, the regions below speak for themselves, and claiming "nothing yet" over a failed
	     read would contradict them. No pending arm and no wrapper, so for every other reader this
	     adds nothing to the page. -->
	{#await firstRun then nothingYet}
		{#if nothingYet}
			<section
				aria-labelledby="first-run-heading"
				data-testid="first-run"
				class="rounded border border-tp-rule bg-tp-surface p-4 text-sm text-tp-text"
			>
				<h2 id="first-run-heading" class="mb-1 text-base text-tp-text">
					Nothing here yet — temper fills as you and your agents work.
				</h2>
				<p class="text-tp-text-muted">
					Goals, tasks and sessions are written by an agent working with temper: Claude Code with
					the temper skill, the temper desktop app, or the <code>temper</code> CLI. Each one appears
					here the moment it is saved.
				</p>
				<a
					href={FIRST_RUN_HELP_URL}
					class="mt-2 inline-block text-xs text-tp-accent hover:text-tp-text"
				>
					How to connect an agent →
				</a>
			</section>
		{/if}
	{/await}

	<!-- Resume: the work the reader last changed, when they changed it — never `updated`, which a
	     teammate's write moves — and its opening. A reader who has changed nothing gets no region at
	     all; only a read in flight or a failed one draws a state here. The read in flight does draw
	     one, and it vanishes for that reader: drawing nothing instead would push every region below
	     down when the return lands, for nearly every reader, and would make a slow read look like
	     having changed nothing. -->
	{#snippet resumeArriving()}<RegionState state="arriving" label={RESUME_LABEL} />{/snippet}
	{#await data.resume}
		{@render resumeRegion(resumeArriving)}
	{:then resume}
		{#if resume}
			{#snippet resumeShown()}
				<p class="mb-1 text-[10px] text-tp-text-subtle" data-testid="resume-changed">
					{resume.resource.doc_type_name} · you changed {relativeTime(resume.changedAt)}
				</p>
				<ResourceLink
					id={resume.resource.id}
					title={resume.resource.title}
					class="text-base text-tp-text hover:text-tp-accent"
				/>
				{#await data.opening}
					<RegionState state="arriving" label="opening" />
				{:then opening}
					{#if opening}
						<div class="mt-3 text-sm text-tp-text" data-testid="resume-opening">
							<MarkdownRenderer markdown={opening.markdown} />
							{#if opening.truncated}
								<p class="mt-1 text-xs italic text-tp-text-subtle">
									… continues in the {resume.resource.doc_type_name}.
								</p>
							{/if}
						</div>
					{/if}
				{:catch error}
					<RegionState state={regionStateFor(error)} label="opening" />
				{/await}
			{/snippet}
			{@render resumeRegion(resumeShown)}
		{/if}
	{:catch error}
		{#snippet resumeFailed()}<RegionState state={regionStateFor(error)} label={RESUME_LABEL} />{/snippet}
		{@render resumeRegion(resumeFailed)}
	{/await}

	<!-- Ordered by the reader's own changes, and only here: the nav's places keep their order. The
	     list is read again whenever this page's load reruns, including when a way in grows, so a
	     change made meanwhile can move a place. -->
	<section aria-labelledby="places-heading" data-testid="places">
		<h2 id="places-heading" class="mb-2 text-[10px] uppercase tracking-widest text-tp-text-subtle">
			places you work in
		</h2>
		{#await places}
			<RegionState state="arriving" label={PLACES_SCOPE} />
		{:then got}
			{#if got.total === 0}
				<RegionState
					state="empty"
					label={PLACES_SCOPE}
					detail="Places where you, or an agent working as you, change something appear here."
				/>
			{:else}
				<ul class="divide-y divide-tp-rule">
					{#each got.rows as { context, lastChangedAt } (context.id)}
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
				{@render foot(got, PLACES_SCOPE, PLACES_ORDER)}
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
						<RegionState state="empty" label={way.scope} detail={WAY_IN_EMPTY_DETAIL[way.id]} />
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
				<RegionState state="empty" label={RECENT_SCOPE} detail={RECENT_EMPTY_DETAIL} />
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
