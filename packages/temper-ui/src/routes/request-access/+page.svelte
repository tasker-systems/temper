<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { invalidateAll, goto } from '$app/navigation';
	import { enhance } from '$app/forms';
	import Wordmark from '$lib/components/Wordmark.svelte';
	import type { PageData, ActionData } from './$types';

	let { data, form }: { data: PageData; form: ActionData } = $props();

	let pollHandle: ReturnType<typeof setInterval> | null = null;

	onMount(() => {
		if (data.ownRequest?.status === 'pending') {
			pollHandle = setInterval(() => {
				invalidateAll();
			}, 30_000);
		}
	});

	onDestroy(() => {
		if (pollHandle) clearInterval(pollHandle);
	});

	$effect(() => {
		if (data.ownRequest?.status === 'approved') {
			goto('/vault/all');
		}
	});

	const status = $derived(data.ownRequest?.status ?? null);
	const accountLabel = $derived(data.user?.email ?? data.user?.name ?? data.user?.sub ?? '');

	function fullDate(iso: string): string {
		return new Date(iso).toLocaleDateString('en-US', {
			month: 'long',
			day: 'numeric',
			year: 'numeric'
		});
	}
</script>

<svelte:head>
	<title>Request access — temper</title>
</svelte:head>

<div class="page">
	<div class="page-mark">
		<Wordmark size="md" href="/" />
	</div>

	<header class="ed-strip">
		<span class="ed-strip-em">Account</span>
		<span class="ed-strip-sep">·</span>
		<span class="ed-strip-accent">{accountLabel}</span>
		<span class="ed-strip-spacer"></span>
		<span class="ed-strip-meta">
			{data.settings?.instance_name ?? 'temperkb.io'} · invitation only
		</span>
	</header>

	<article class="ed-masthead">
		{#if status === 'pending'}
			<!-- ─── PENDING ─────────────────────────────────────────── -->
			<header class="ed-hero">
				<div class="ed-hero-eyebrow">Status</div>
				<h1 class="ed-hero-name t-ed-hero-name">Pending review</h1>
				<p class="ed-hero-deck t-ed-deck">
					Your letter was filed on <em>{fullDate(data.ownRequest!.created)}</em> and is awaiting
					a steward's decision. This page will refresh automatically when it's been reviewed.
				</p>
			</header>

			{#if data.ownRequest!.message}
				<section class="ed-rail">
					<div class="ed-rail-marker">
						<div class="ed-rail-label">Your note</div>
						<div class="ed-rail-sub">As submitted</div>
					</div>
					<blockquote class="letter-quote">{data.ownRequest!.message}</blockquote>
				</section>
			{/if}

			<form method="POST" action="?/withdraw" use:enhance class="withdraw-form">
				<button type="submit" class="ed-action ed-action--ghost">
					Withdraw this request
				</button>
			</form>
		{:else if status === 'approved'}
			<!-- ─── APPROVED ────────────────────────────────────────── -->
			<header class="ed-hero">
				<div class="ed-hero-eyebrow">Status</div>
				<h1 class="ed-hero-name t-ed-hero-name">Approved</h1>
				<p class="ed-hero-deck t-ed-deck">
					Welcome to temper. You're being routed to the dashboard now.
				</p>
			</header>
		{:else}
			<!-- ─── FORM (no request, withdrawn, or rejected) ───────── -->
			<header class="ed-hero">
				<div class="ed-hero-eyebrow">A letter of introduction</div>
				<h1 class="ed-hero-name t-ed-hero-name">Request access</h1>
				<p class="ed-hero-deck t-ed-deck">
					This instance is in invitation-only beta. Tell a steward what you're working on,
					accept the terms, and you'll be added once your request is reviewed.
				</p>
			</header>

			{#if status === 'rejected'}
				<aside class="ed-notice">
					<div class="ed-notice-label">Previous request declined</div>
					{#if data.ownRequest!.decision_note}
						<p>"{data.ownRequest!.decision_note}"</p>
					{/if}
					<p>You may submit a new request below.</p>
				</aside>
			{/if}

			<form method="POST" action="?/submit" use:enhance class="letter">
				<div class="letter-field">
					<label for="message" class="letter-label">
						Your note
						<span class="letter-label-aside">optional</span>
					</label>
					<textarea
						id="message"
						name="message"
						rows="6"
						class="letter-body"
						placeholder="What you're working on, what brought you here, anything else useful for the reviewer."
						value={form?.message ?? ''}
					></textarea>
				</div>

				<div class="letter-terms">
					<input
						id="accepted_terms"
						name="accepted_terms"
						type="checkbox"
						class="letter-checkbox"
						required
					/>
					<label for="accepted_terms" class="letter-terms-label">
						I accept the
						{#if data.settings?.terms_resource_uri}
							<a href={data.settings.terms_resource_uri}>terms of service</a>.
						{:else}
							terms of service.
						{/if}
					</label>
				</div>

				{#if data.settings?.terms_version}
					<input type="hidden" name="terms_version" value={data.settings.terms_version} />
				{/if}

				{#if form?.error}
					<aside class="ed-notice">
						<div class="ed-notice-label">Notice</div>
						<p>{form.error}</p>
					</aside>
				{/if}

				<button type="submit" class="letter-submit">
					Submit request <span aria-hidden="true">→</span>
				</button>
			</form>
		{/if}

		<footer class="ed-footnote">
			Signed in as {accountLabel}
			<a href="/auth/logout">Sign out</a>
		</footer>
	</article>
</div>

<style>
	.page {
		max-width: 56rem;
		margin: 0 auto;
		padding: 3rem 2rem 4rem;
	}
	.page-mark {
		color: var(--color-quiet-accent);
		margin-bottom: 2.5rem;
	}

	/* ─── Letter quote (pending state) ───────────────────────────── */

	.letter-quote {
		font-family: var(--font-serif);
		font-size: 1.05rem;
		font-style: italic;
		color: var(--color-quiet-mid);
		line-height: 1.6;
		margin: 0;
		padding-left: 1rem;
		border-left: 1px solid var(--color-quiet-rule);
	}
	.withdraw-form {
		margin-top: 2rem;
	}

	/* ─── Letter form ────────────────────────────────────────────── */

	.letter {
		max-width: 36rem;
	}
	.letter-field {
		margin-bottom: 2rem;
	}
	.letter-label {
		display: block;
		font-family: var(--font-mono);
		font-size: 0.62rem;
		text-transform: uppercase;
		letter-spacing: 0.22em;
		color: var(--color-quiet-accent);
		margin-bottom: 0.7rem;
	}
	.letter-label-aside {
		color: var(--color-quiet-dim);
		margin-left: 0.5rem;
		text-transform: none;
		letter-spacing: 0;
		font-style: italic;
		font-family: var(--font-serif);
		font-size: 0.78rem;
	}
	.letter-body {
		display: block;
		width: 100%;
		font-family: var(--font-serif);
		font-size: 1.05rem;
		line-height: 1.6;
		color: var(--color-quiet-fg);
		background: transparent;
		border: none;
		border-top: 1px solid var(--color-quiet-rule);
		border-bottom: 1px solid var(--color-quiet-rule);
		padding: 1.2rem 0.2rem;
		resize: vertical;
		min-height: 9rem;
		transition: border-color 0.18s ease;
	}
	.letter-body::placeholder {
		color: var(--color-quiet-dim);
		font-style: italic;
	}
	.letter-body:focus {
		outline: none;
		border-top-color: var(--color-quiet-accent);
		border-bottom-color: var(--color-quiet-accent);
	}

	.letter-terms {
		display: flex;
		align-items: baseline;
		gap: 0.7rem;
		margin-bottom: 2.5rem;
	}
	.letter-checkbox {
		accent-color: var(--color-quiet-accent);
		width: 0.9rem;
		height: 0.9rem;
		margin-top: 0.2rem;
		flex-shrink: 0;
	}
	.letter-terms-label {
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 0.95rem;
		color: var(--color-quiet-mid);
		line-height: 1.5;
	}
	.letter-terms-label a {
		color: var(--color-quiet-accent);
		text-decoration: none;
		border-bottom: 1px solid var(--color-quiet-border);
	}
	.letter-terms-label a:hover {
		color: var(--color-quiet-fg);
		border-bottom-color: var(--color-quiet-accent);
	}

	.letter-submit {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		text-transform: uppercase;
		letter-spacing: 0.22em;
		color: var(--color-quiet-fg);
		background: transparent;
		border: 1px solid var(--color-quiet-accent);
		padding: 0.9rem 1.6rem;
		cursor: pointer;
		transition:
			background 0.18s ease,
			color 0.18s ease;
	}
	.letter-submit:hover {
		background: var(--color-quiet-accent);
		color: #0a0a0f;
	}
</style>
