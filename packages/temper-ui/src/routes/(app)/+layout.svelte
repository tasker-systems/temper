<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { LayoutData } from './$types';
	import Sidebar from '$lib/components/Sidebar.svelte';

	let { data, children }: { data: LayoutData; children: Snippet } = $props();
</script>

<div class="flex h-screen bg-zinc-950 text-zinc-100">
	<Sidebar
		contexts={data.contexts ?? []}
		user={data.profile
			? { display_name: data.profile.display_name, email: data.profile.email ?? '' }
			: null}
		isAdmin={data.entitlements?.is_admin ?? false}
	/>
	<main class="flex-1 overflow-y-auto">
		{@render children()}
	</main>
</div>
