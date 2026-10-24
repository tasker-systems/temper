<script lang="ts">
	/**
	 * The chrome's standing menu, at the sidebar's head: what the reader can set about their own
	 * session from any page — the appearance, and who they are signed in as, with Sign out. The
	 * desktop's `ChromeMenu` sits in the same corner and behaves the same way: it opens only when
	 * the reader opens it, and closes on Escape or a pointer outside it.
	 *
	 * The sidebar head renders it in both rail states, so a collapsed rail still reaches every
	 * entry. The panel is `fixed` at the trigger's corner because the rail clips its overflow, and
	 * a collapsed rail is narrower than the panel.
	 */
	import ThemeSwitch from './ThemeSwitch.svelte';

	interface Props {
		user: { display_name: string; email: string } | null;
	}

	let { user }: Props = $props();

	let open = $state(false);
	let root: HTMLDivElement | undefined = $state();
	let trigger: HTMLButtonElement | undefined = $state();
	let at = $state({ top: 0, left: 0 });

	function toggle(): void {
		if (!open && trigger) {
			const r = trigger.getBoundingClientRect();
			at = { top: r.bottom + 4, left: r.left };
		}
		open = !open;
	}

	function onDocumentPointerDown(event: PointerEvent): void {
		if (root && !root.contains(event.target as Node)) open = false;
	}

	function onDocumentKeydown(event: KeyboardEvent): void {
		if (event.key === 'Escape') open = false;
	}

	$effect(() => {
		document.addEventListener('pointerdown', onDocumentPointerDown);
		document.addEventListener('keydown', onDocumentKeydown);
		return () => {
			document.removeEventListener('pointerdown', onDocumentPointerDown);
			document.removeEventListener('keydown', onDocumentKeydown);
		};
	});
</script>

<div bind:this={root} class="flex">
	<button
		bind:this={trigger}
		type="button"
		onclick={toggle}
		aria-expanded={open}
		aria-label="Menu"
		title="Menu"
		class="px-3 py-2 font-mono text-sm leading-none text-tp-accent hover:text-tp-text"
	>
		≡
	</button>
	{#if open}
		<div
			class="fixed z-50 w-56 rounded border border-tp-rule-strong bg-tp-surface-raised py-1"
			style="top: {at.top}px; left: {at.left}px"
			data-testid="chrome-menu"
		>
			<div class="px-3 pt-1.5 pb-0.5 text-[10px] uppercase tracking-widest text-tp-text-subtle">
				Appearance
			</div>
			<ThemeSwitch />
			<div class="mt-1 border-t border-tp-rule pt-1">
				{#if user}
					<div class="flex items-center gap-2 px-3 py-1.5 text-xs text-tp-text-subtle">
						<div class="h-5 w-5 flex-shrink-0 rounded-full bg-tp-text-faint"></div>
						<span class="truncate" title={user.email || undefined}>{user.display_name}</span>
					</div>
				{/if}
				<a
					href="/auth/logout"
					class="block px-3 py-1.5 text-sm text-tp-text-muted hover:text-tp-text"
				>
					Sign out
				</a>
			</div>
		</div>
	{/if}
</div>
