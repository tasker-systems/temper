<script lang="ts">
  import type { Snippet } from 'svelte';
  import Footer from '$lib/components/landing/Footer.svelte';

  let { children }: { children: Snippet } = $props();
</script>

{@render children()}

<Footer />
