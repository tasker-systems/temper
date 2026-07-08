<script lang="ts">
	// Non-spatial accessible mirror of the Atlas Home field (Beat B). The Home field
	// is drawn inside `<svg role="img">`, whose subtree is opaque to screen readers, so
	// this HTML `<nav>` is the accessible equivalent: the same build contexts + research
	// cogmaps as real links, with empty states. Visually hidden, but revealed on
	// keyboard focus so it is not a dead trap. Sighted-pointer users use the field;
	// screen-reader + keyboard users get this.
	//
	// Beat C mirrors TierHome's scope-filter chip-row + recency text as real, natively
	// interactive controls: a `<button>` group (native Enter/Space activation, no custom
	// keydown handling needed) with `aria-pressed` on the active chip, and a "last active
	// …" string per build row. Only one chip-row is ever shown — same as TierHome, keyed
	// off whichever Home lens is committed (`?home`) — and it narrows only that lens's own
	// list, exactly mirroring `buildPosFiltered`/`researchPosFiltered`.
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import type { AtlasHome } from '$lib/types/generated/graph_home';
	import {
		parseHomeLens,
		parseScopeFilter,
		buildScopeFilterUrl,
		clearScopeFilterUrl
	} from '$lib/graph/atlas/nav';
	import { deriveScopeChips } from '$lib/graph/atlas/scopeChips';
	import { relativeTime } from '$lib/graph/atlas/relativeTime';

	interface Props {
		home: AtlasHome;
	}
	let { home }: Props = $props();

	const cogmapHref = (id: string) => `${$page.url.pathname}?cogmap=${id}`;

	const committed = $derived(parseHomeLens($page.url));
	const scope = $derived(parseScopeFilter($page.url));

	// Same derivation as TierHome: chips come off the COMMITTED lens's own bodies, so
	// the chip-row never offers a scope that isn't actually in the list on screen.
	const chipRefs = $derived(
		committed === 'research'
			? deriveScopeChips(home.research)
			: committed === 'build'
				? deriveScopeChips(home.build)
				: []
	);

	// Narrow only the committed lens's own list — the uncommitted lens is unaffected,
	// same as the field (where it's simply not the one being viewed).
	const buildFiltered = $derived(
		home.build.filter((c) => committed !== 'build' || scope == null || c.owner_ref === scope)
	);
	const researchFiltered = $derived(
		home.research.filter((m) => committed !== 'research' || scope == null || m.owner_ref === scope)
	);

	// Toggle-off affordance: re-activating the active chip (or "All") clears the narrow.
	function selectScope(ref: string | null) {
		if (ref === null || ref === scope) {
			goto(clearScopeFilterUrl($page.url), { keepFocus: true, noScroll: true });
		} else {
			goto(buildScopeFilterUrl($page.url, ref), { keepFocus: true, noScroll: true });
		}
	}

	const lastActiveText = (iso: string | null) => (iso ? relativeTime(iso) : '—');
</script>

{#snippet chipRow(label: string)}
	<div class="chip-row" role="group" aria-label={`Filter ${label} by scope`}>
		<button type="button" aria-pressed={scope === null} onclick={() => selectScope(null)}>All</button>
		{#each chipRefs as ref (ref)}
			<button
				type="button"
				aria-pressed={scope === ref}
				aria-label={`filter to ${ref}`}
				onclick={() => selectScope(ref)}>{ref}</button
			>
		{/each}
	</div>
{/snippet}

<nav class="home-a11y" aria-label="Atlas home — your work and the knowledge you can explore">
	<h2>Build — your work, across your teams and personal space</h2>
	{#if home.build.length}
		{#if committed === 'build' && chipRefs.length}
			{@render chipRow('build contexts')}
		{/if}
		{#if buildFiltered.length}
			<ul>
				{#each buildFiltered as c (c.id)}
					<li>
						<a href={`/vault/${c.owner_ref}`}
							>{c.name} — {c.owner_ref} · {c.resource_count} resources · last active {lastActiveText(
								c.last_active_at
							)}</a
						>
					</li>
				{/each}
			</ul>
		{:else}
			<p>No contexts in "{scope}" — try "All".</p>
		{/if}
	{:else}
		<p>You don't have any contexts to build in yet.</p>
	{/if}

	<h2>Research — the knowledge you can explore</h2>
	{#if home.research.length}
		{#if committed === 'research' && chipRefs.length}
			{@render chipRow('research cogmaps')}
		{/if}
		{#if researchFiltered.length}
			<ul>
				{#each researchFiltered as m (m.id)}
					<li><a href={cogmapHref(m.id)}>{m.name} — {m.region_count} regions</a></li>
				{/each}
			</ul>
		{:else}
			<p>No cognitive maps in "{scope}" — try "All".</p>
		{/if}
	{:else}
		<p>There are no cognitive maps you can reach yet.</p>
	{/if}
</nav>

<style>
	/* Visually hidden until focused (standard sr-only + reveal-on-focus). */
	.home-a11y {
		position: absolute;
		width: 1px;
		height: 1px;
		margin: -1px;
		padding: 0;
		overflow: hidden;
		clip: rect(0 0 0 0);
		white-space: nowrap;
		border: 0;
	}
	.home-a11y:focus-within {
		position: absolute;
		top: 8px;
		left: 8px;
		z-index: 5;
		width: auto;
		height: auto;
		margin: 0;
		padding: 12px 16px;
		overflow: auto;
		clip: auto;
		white-space: normal;
		background: rgba(20, 23, 29, 0.97);
		border: 1px solid rgba(255, 255, 255, 0.12);
		border-radius: 10px;
		color: #c9ced9;
		font-size: 13px;
	}
	.home-a11y h2 {
		font-size: 12px;
		letter-spacing: 0.04em;
		margin: 6px 0 4px;
	}
	.home-a11y a {
		color: #9fc4d6;
	}
	.home-a11y .chip-row {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		margin: 2px 0 6px;
	}
	.home-a11y .chip-row button {
		background: none;
		border: 1px solid rgba(255, 255, 255, 0.2);
		border-radius: 999px;
		color: inherit;
		font: inherit;
		font-size: 12px;
		padding: 1px 8px;
		cursor: pointer;
	}
	.home-a11y .chip-row button[aria-pressed='true'] {
		background: rgba(159, 196, 214, 0.25);
		border-color: #9fc4d6;
		font-weight: 700;
	}
</style>
