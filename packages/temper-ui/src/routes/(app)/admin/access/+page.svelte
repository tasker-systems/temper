<script lang="ts">
	import { enhance } from '$app/forms';
	import type { PageData, ActionData } from './$types';

	let { data, form }: { data: PageData; form: ActionData } = $props();

	function fullDate(iso: string): string {
		return new Date(iso).toLocaleDateString('en-US', {
			month: 'short',
			day: 'numeric',
			year: 'numeric'
		});
	}
</script>

<svelte:head>
	<title>Access requests — temper</title>
</svelte:head>

<article class="ed-masthead">
	<header class="ed-hero">
		<div class="ed-hero-eyebrow">Stewardship · Access</div>
		<h1 class="ed-hero-name">Requests</h1>
		<p class="ed-hero-deck">
			<span class="ed-hero-num">{data.requests.length}</span>
			{data.requests.length === 1 ? 'pending request' : 'pending requests'} awaiting review.
			Approving inserts a watcher membership; declining allows the petitioner to write again.
		</p>
	</header>

	{#if form?.error}
		<aside class="ed-notice">
			<div class="ed-notice-label">Notice</div>
			<p>{form.error}</p>
		</aside>
	{/if}

	<section class="ed-rail">
		<div class="ed-rail-marker">
			<div class="ed-rail-label">The queue</div>
			<div class="ed-rail-sub">Pending join requests, oldest first</div>
		</div>

		{#if data.requests.length === 0}
			<div class="ed-rail-empty">
				The queue is clear. New requests will appear here when petitioners write in.
			</div>
		{:else}
			<ol class="queue">
				{#each data.requests as req, i (req.id)}
					<li class="queue-entry">
						<span class="queue-num">{String(i + 1).padStart(2, '0')}</span>
						<div class="queue-body">
							<div class="queue-head">
								<div class="queue-name">{req.display_name}</div>
								<div class="queue-meta">
									<span>{req.email ?? 'no email on file'}</span>
									<span class="queue-meta-sep">·</span>
									<span>via {req.source}</span>
									<span class="queue-meta-sep">·</span>
									<time datetime={req.created}>{fullDate(req.created)}</time>
								</div>
							</div>

							{#if req.message}
								<blockquote class="queue-message">{req.message}</blockquote>
							{:else}
								<div class="queue-message queue-message--silent">
									No accompanying note.
								</div>
							{/if}

							<div class="queue-actions">
								<form method="POST" action="?/approve" use:enhance>
									<input type="hidden" name="id" value={req.id} />
									<button type="submit" class="ed-action ed-action--primary">
										Approve →
									</button>
								</form>
								<span class="ed-action-sep">·</span>
								<form method="POST" action="?/reject" use:enhance class="queue-decline">
									<input type="hidden" name="id" value={req.id} />
									<input
										type="text"
										name="decision_note"
										placeholder="Reason (optional)"
										class="queue-decline-input"
									/>
									<button type="submit" class="ed-action ed-action--ghost">Decline</button>
								</form>
							</div>
						</div>
					</li>
				{/each}
			</ol>
		{/if}
	</section>
</article>

<style>
	.queue {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.queue-entry {
		display: grid;
		grid-template-columns: 2.4rem 1fr;
		gap: 1.2rem;
		padding: 1.8rem 0;
		border-bottom: 1px solid var(--color-quiet-rule);
	}
	.queue-entry:last-child {
		border-bottom: none;
	}
	.queue-num {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		color: var(--color-quiet-dim);
		letter-spacing: 0.05em;
		font-feature-settings: 'tnum';
		padding-top: 0.4rem;
	}
	.queue-head {
		margin-bottom: 1rem;
	}
	.queue-name {
		font-family: var(--font-serif);
		font-size: 1.4rem;
		color: var(--color-quiet-fg);
		margin-bottom: 0.4rem;
		line-height: 1.2;
	}
	.queue-meta {
		font-family: var(--font-mono);
		font-size: 0.6rem;
		text-transform: uppercase;
		letter-spacing: 0.18em;
		color: var(--color-quiet-dim);
		display: flex;
		gap: 0.5rem;
		flex-wrap: wrap;
		align-items: baseline;
	}
	.queue-meta-sep {
		opacity: 0.4;
	}
	.queue-message {
		font-family: var(--font-serif);
		font-size: 1rem;
		font-style: italic;
		color: var(--color-quiet-mid);
		line-height: 1.55;
		padding-left: 1rem;
		border-left: 1px solid var(--color-quiet-rule);
		margin: 0 0 1.4rem 0;
	}
	.queue-message--silent {
		color: var(--color-quiet-dim);
		font-size: 0.9rem;
	}
	.queue-actions {
		display: flex;
		align-items: center;
		gap: 0;
		flex-wrap: wrap;
	}
	.queue-decline {
		display: flex;
		align-items: center;
		gap: 0.8rem;
	}
	.queue-decline-input {
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 0.85rem;
		background: transparent;
		border: none;
		border-bottom: 1px solid var(--color-quiet-rule);
		color: var(--color-quiet-fg);
		padding: 0.3rem 0.4rem;
		width: 13rem;
		transition: border-color 0.18s ease;
	}
	.queue-decline-input::placeholder {
		color: var(--color-quiet-dim);
	}
	.queue-decline-input:focus {
		outline: none;
		border-bottom-color: var(--color-quiet-accent);
	}

	@media (max-width: 38rem) {
		.queue-entry {
			grid-template-columns: 1fr;
		}
		.queue-num {
			padding-top: 0;
		}
		.queue-decline {
			width: 100%;
			margin-top: 0.5rem;
		}
		.queue-decline-input {
			flex: 1;
			width: auto;
		}
	}
</style>
