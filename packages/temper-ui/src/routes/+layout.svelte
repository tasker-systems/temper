<script>
	import '../app.css';

	let { children } = $props();
</script>

<div class="min-h-screen bg-chalk text-ink font-sans">
	<header class="border-b border-temper-200 px-6 py-4">
		<nav class="mx-auto flex max-w-6xl items-center justify-between">
			<a href="/" class="text-xl font-semibold text-temper-700">temper</a>
			<div class="flex gap-6 text-sm">
				<a href="/docs" class="text-temper-600 hover:text-temper-800">Docs</a>
				<a href="/dashboard" class="text-temper-600 hover:text-temper-800">Dashboard</a>
			</div>
		</nav>
	</header>

	<main>
		{@render children()}
	</main>
</div>
