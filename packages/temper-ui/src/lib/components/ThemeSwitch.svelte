<script lang="ts">
	/**
	 * The appearance control: text actions, worded as the desktop's `ThemeSwitch` words them —
	 * "Follow system", then each theme the design package publishes, by title. The choice is kept
	 * by `themeStore` (a cookie the server renders from). The `(app)` layout initialises it for
	 * every authed page; this initialises it too, so the pressed action is the reader's choice
	 * wherever the switch is rendered. `init` is idempotent.
	 */
	import { THEMES, type ThemePreference } from '@tasker-systems/temper-design/theme';
	import { onMount } from 'svelte';
	import { page } from '$app/stores';
	import { themeStore } from '$lib/stores/theme.svelte';

	// Until the store starts, the pressed action is the one the server read from the cookie (the
	// `(app)` layout's `themePreference`); the store's own default would press "Follow system" for
	// a reader who fixed a theme, for a frame.
	let started = $state(false);
	onMount(() => {
		themeStore.init();
		started = true;
	});
	const served = $derived($page.data.themePreference as ThemePreference | undefined);
	const pref = $derived<ThemePreference>(started ? themeStore.pref : (served ?? themeStore.pref));

	const action =
		'block w-full px-3 py-1 text-left text-xs text-tp-text-subtle hover:text-tp-text aria-pressed:text-tp-text';
</script>

<div role="group" aria-label="Theme" class="py-1" data-testid="theme-switch">
	<button
		type="button"
		class={action}
		aria-pressed={pref.follow === 'system'}
		onclick={() => themeStore.choose({ follow: 'system', family: themeStore.active })}
		>Follow system</button
	>
	{#each THEMES as theme (theme.name)}
		<button
			type="button"
			class={action}
			aria-pressed={pref.follow === 'fixed' && pref.name === theme.name}
			onclick={() => themeStore.choose({ follow: 'fixed', name: theme.name })}>{theme.title}</button
		>
	{/each}
</div>
