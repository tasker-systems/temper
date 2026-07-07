<script lang="ts">
	// Non-spatial accessible mirror of the Atlas Home field (Beat B). The Home field
	// is drawn inside `<svg role="img">`, whose subtree is opaque to screen readers, so
	// this HTML `<nav>` is the accessible equivalent: the same build contexts + research
	// cogmaps as real links, with empty states. Visually hidden, but revealed on
	// keyboard focus so it is not a dead trap. Sighted-pointer users use the field;
	// screen-reader + keyboard users get this.
	import { page } from '$app/stores';
	import type { AtlasHome } from '$lib/types/generated/graph_home';

	interface Props {
		home: AtlasHome;
	}
	let { home }: Props = $props();

	const cogmapHref = (id: string) => `${$page.url.pathname}?cogmap=${id}`;
</script>

<nav class="home-a11y" aria-label="Atlas home — your work and the knowledge you can explore">
	<h2>Build — your work, across your teams and personal space</h2>
	{#if home.build.length}
		<ul>
			{#each home.build as c (c.id)}
				<li>
					<a href={`/vault/${c.owner_ref}`}>{c.name} — {c.owner_ref} · {c.resource_count} resources</a>
				</li>
			{/each}
		</ul>
	{:else}
		<p>You don't have any contexts to build in yet.</p>
	{/if}

	<h2>Research — the knowledge you can explore</h2>
	{#if home.research.length}
		<ul>
			{#each home.research as m (m.id)}
				<li><a href={cogmapHref(m.id)}>{m.name} — {m.region_count} regions</a></li>
			{/each}
		</ul>
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
</style>
