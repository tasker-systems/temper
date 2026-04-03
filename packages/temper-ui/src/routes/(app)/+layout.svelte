<script>
	let { children } = $props();
</script>

<div class="mx-auto flex max-w-7xl gap-6 px-6 py-8">
	<aside class="w-56 shrink-0">
		<nav class="space-y-1 text-sm">
			<a href="/dashboard" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Dashboard
			</a>
			<a href="/resources" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Resources
			</a>
			<a href="/contexts" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Contexts
			</a>
			<a href="/search" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Search
			</a>
			<a href="/teams" class="block rounded px-3 py-2 text-temper-700 hover:bg-temper-50">
				Teams
			</a>
		</nav>
	</aside>

	<div class="flex-1">
		{@render children()}
	</div>
</div>
