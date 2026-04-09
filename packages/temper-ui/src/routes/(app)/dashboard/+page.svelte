<script lang="ts">
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();

	const now = new Date();
	const dateLabel = now
		.toLocaleDateString('en-US', {
			weekday: 'long',
			month: 'long',
			day: 'numeric',
			year: 'numeric'
		})
		.toUpperCase();

	const recent = $derived(
		[...data.recentResources].sort((a, b) => b.updated.localeCompare(a.updated))
	);

	function shortDate(iso: string): string {
		return new Date(iso)
			.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
			.toUpperCase();
	}
</script>

<svelte:head>
	<title>Dashboard — temper</title>
</svelte:head>

<article class="ed-masthead">
	<header class="ed-strip">
		<span class="ed-strip-em">{dateLabel}</span>
		<span class="ed-strip-sep">·</span>
		<span class="ed-strip-accent">@{data.profile.slug}</span>
		{#if data.entitlements.is_admin}
			<span class="ed-strip-sep">·</span>
			<span class="ed-strip-italic">Steward</span>
		{/if}
		<span class="ed-strip-spacer"></span>
		<span class="ed-strip-meta">Personal edition</span>
	</header>

	<header class="ed-hero">
		<div class="ed-hero-eyebrow">Welcome back</div>
		<h1 class="ed-hero-name">{data.profile.display_name}</h1>
		<p class="ed-hero-deck">
			<span class="ed-hero-num">{data.recentResources.length}</span>
			{data.recentResources.length === 1 ? 'entry' : 'entries'} in your throughline
			<span class="ed-hero-sep">·</span>
			<span class="ed-hero-num">{data.contexts.length}</span>
			{data.contexts.length === 1 ? 'context' : 'contexts'}
			<span class="ed-hero-sep">·</span>
			access <em class="ed-hero-state">{data.entitlements.system_access ? 'open' : 'pending'}</em>
		</p>
	</header>

	{#if data.apiErrors.length > 0}
		<aside class="ed-notice">
			<div class="ed-notice-label">Notice</div>
			<ul>
				{#each data.apiErrors as msg}
					<li>{msg}</li>
				{/each}
			</ul>
		</aside>
	{/if}

	<section class="ed-rail">
		<div class="ed-rail-marker">
			<div class="ed-rail-label">The ledger</div>
			<div class="ed-rail-sub">Recent resources, newest first</div>
		</div>

		{#if recent.length === 0}
			<div class="ed-rail-empty">
				The ledger is empty. Run
				<code>temper sync</code>
				from the CLI to push your vault and the entries will appear here.
			</div>
		{:else}
			<ol class="ledger">
				{#each recent as r, i (r.id)}
					<li class="ledger-entry">
						<span class="ledger-num">{String(i + 1).padStart(2, '0')}</span>
						<div class="ledger-body">
							<div class="ledger-title">{r.title}</div>
							<div class="ledger-uri">{r.origin_uri}</div>
						</div>
						<time class="ledger-date" datetime={r.updated}>{shortDate(r.updated)}</time>
					</li>
				{/each}
			</ol>
		{/if}
	</section>

	<section class="ed-rail">
		<div class="ed-rail-marker">
			<div class="ed-rail-label">The library</div>
			<div class="ed-rail-sub">Contexts visible to you</div>
		</div>

		{#if data.contexts.length === 0}
			<div class="ed-rail-empty">No contexts visible to this profile.</div>
		{:else}
			<ul class="library">
				{#each data.contexts as ctx (ctx.id)}
					<li class="library-card">
						<div class="library-name">{ctx.name}</div>
						<div class="library-meta">{ctx.kb_owner_table.replace('kb_', '')}</div>
					</li>
				{/each}
			</ul>
		{/if}
	</section>
</article>

<style>
	/* ─── Ledger (recent resources) ──────────────────────────────── */

	.ledger {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.ledger-entry {
		display: grid;
		grid-template-columns: 2.4rem 1fr auto;
		align-items: baseline;
		gap: 1.2rem;
		padding: 1rem 0;
		border-bottom: 1px solid var(--color-quiet-rule);
		transition: opacity 0.18s ease;
	}
	.ledger-entry:last-child {
		border-bottom: none;
	}
	.ledger-entry:hover {
		opacity: 0.85;
	}
	.ledger-num {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		color: var(--color-quiet-dim);
		letter-spacing: 0.05em;
		font-feature-settings: 'tnum';
		padding-top: 0.15rem;
	}
	.ledger-title {
		font-family: var(--serif);
		font-size: 1.05rem;
		color: var(--color-quiet-fg);
		margin-bottom: 0.25rem;
		line-height: 1.35;
	}
	.ledger-uri {
		font-family: var(--font-mono);
		font-size: 0.66rem;
		color: var(--color-quiet-dim);
		letter-spacing: 0.02em;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 100%;
	}
	.ledger-date {
		font-family: var(--font-mono);
		font-size: 0.6rem;
		text-transform: uppercase;
		letter-spacing: 0.18em;
		color: var(--color-quiet-mid);
		white-space: nowrap;
		padding-top: 0.15rem;
	}

	/* ─── Library (contexts as a card catalog) ───────────────────── */

	.library {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
		border-top: 1px solid var(--color-quiet-rule);
		border-left: 1px solid var(--color-quiet-rule);
	}
	.library-card {
		padding: 1.1rem 1.3rem;
		border-right: 1px solid var(--color-quiet-rule);
		border-bottom: 1px solid var(--color-quiet-rule);
		transition: background 0.18s ease;
	}
	.library-card:hover {
		background: var(--color-quiet-card);
	}
	.library-name {
		font-family: var(--serif);
		font-size: 1rem;
		color: var(--color-quiet-fg);
		margin-bottom: 0.3rem;
		line-height: 1.3;
	}
	.library-meta {
		font-family: var(--font-mono);
		font-size: 0.58rem;
		text-transform: uppercase;
		letter-spacing: 0.18em;
		color: var(--color-quiet-dim);
	}

	@media (max-width: 38rem) {
		.ledger-entry {
			grid-template-columns: 2rem 1fr;
		}
		.ledger-date {
			grid-column: 2 / 3;
			padding-top: 0.4rem;
		}
	}
</style>
