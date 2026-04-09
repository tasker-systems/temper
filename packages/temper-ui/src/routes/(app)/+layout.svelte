<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { LayoutData } from './$types';
	import Wordmark from '$lib/components/Wordmark.svelte';

	let { data, children }: { data: LayoutData; children: Snippet } = $props();
</script>

<div class="flex min-h-screen flex-col">
	<div class="mx-auto flex w-full max-w-7xl flex-1 gap-8 px-6 py-10">
		<aside class="w-56 shrink-0">
			<div class="mb-8 text-quiet-accent">
				<Wordmark size="md" href="/dashboard" />
			</div>

			<div class="mb-6 border-b border-quiet-rule pb-4">
				<div class="text-sm font-medium text-quiet-fg">{data.profile.display_name}</div>
				{#if data.user.email}
					<div class="truncate text-xs text-quiet-dim">{data.user.email}</div>
				{/if}
			</div>

			<nav class="space-y-1 text-sm">
				<a
					href="/dashboard"
					class="block rounded px-3 py-2 text-quiet-mid hover:bg-quiet-card hover:text-quiet-fg"
				>
					Dashboard
				</a>
				<a
					href="/resources"
					class="block rounded px-3 py-2 text-quiet-mid hover:bg-quiet-card hover:text-quiet-fg"
				>
					Resources
				</a>
				<a
					href="/contexts"
					class="block rounded px-3 py-2 text-quiet-mid hover:bg-quiet-card hover:text-quiet-fg"
				>
					Contexts
				</a>
				<a
					href="/search"
					class="block rounded px-3 py-2 text-quiet-mid hover:bg-quiet-card hover:text-quiet-fg"
				>
					Search
				</a>
				<a
					href="/teams"
					class="block rounded px-3 py-2 text-quiet-mid hover:bg-quiet-card hover:text-quiet-fg"
				>
					Teams
				</a>
				{#if data.entitlements.is_admin}
					<a
						href="/admin/access"
						class="block rounded px-3 py-2 text-quiet-mid hover:bg-quiet-card hover:text-quiet-fg"
					>
						Admin
					</a>
				{/if}
			</nav>

			<div class="mt-6 border-t border-quiet-rule pt-4">
				<a
					href="/auth/logout"
					class="block rounded px-3 py-2 text-sm text-quiet-dim hover:bg-quiet-card hover:text-quiet-fg"
				>
					Sign out
				</a>
			</div>
		</aside>

		<div class="flex-1">
			{@render children()}
		</div>
	</div>

	<footer class="border-t border-quiet-rule">
		<div
			class="mx-auto flex max-w-7xl items-center justify-between gap-4 px-6 py-6 text-quiet-dim"
		>
			<div class="text-quiet-dim">
				<Wordmark size="sm" href={null} />
			</div>
			<div class="flex gap-5 font-mono text-[0.65rem] uppercase tracking-widest">
				<a href="/docs" class="hover:text-quiet-fg">Docs</a>
				<a
					href="https://github.com/tasker-systems/temper"
					rel="noreferrer"
					class="hover:text-quiet-fg"
				>
					GitHub
				</a>
				<span class="text-quiet-dim/60">© temper</span>
			</div>
		</div>
	</footer>
</div>
