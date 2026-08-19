<script lang="ts">
	import Sidebar from '$lib/components/Sidebar.svelte';
	import { sidebarCollapsed, sidebarGroups } from '$lib/stores/sidebar.svelte';
	import type { ContextRowWithCounts, TeamRow } from '$lib/types';

	const SELF = '00000000-0000-0000-0000-0000000000aa';
	const OTHER = '00000000-0000-0000-0000-0000000000bb';

	function ctx(
		name: string,
		ownerRef: string,
		count: number,
		ownerId = ownerRef.startsWith('+') ? 'team' : SELF,
	): ContextRowWithCounts {
		return {
			id: `${ownerRef}/${name}`,
			name,
			kb_owner_table: ownerRef.startsWith('+') ? 'kb_teams' : 'kb_profiles',
			kb_owner_id: ownerId,
			created: '2026-01-01T00:00:00Z',
			updated: '2026-01-01T00:00:00Z',
			resource_count: count as unknown as bigint,
			slug: name,
			owner_ref: ownerRef,
		};
	}

	const team = (slug: string, name: string): TeamRow => ({
		id: slug,
		slug,
		name,
		description: null,
		created: '2026-01-01T00:00:00Z',
		auto_join_role: null,
	});

	const CONTEXTS = [
		ctx('temper', '@operator', 2314),
		ctx('writing', '@operator', 188),
		ctx('infra', '+platform', 412),
		ctx('runbooks', '+platform', 37),
		ctx('papers', '+research', 91),
		ctx('handbook', '+outside-team', 12),
		ctx('shared-notes', '@colleague', 8, OTHER),
	];

	const TEAMS = [
		team('platform', 'Platform Group'),
		team('research', 'Research Group'),
		team('quiet', 'Quiet Group'),
	];

	type Scenario = 'groups' | 'no-teams-read' | 'empty' | 'unavailable';

	const SCENARIOS: Record<Scenario, { contexts: ContextRowWithCounts[] | null; teams: TeamRow[] | null; note: string }> = {
		groups: {
			contexts: CONTEXTS,
			teams: TEAMS,
			note: 'Both reads answered. "Quiet Group" is a team the reader belongs to that holds no readable place; "outside-team" is a team-owned place readable WITHOUT membership, so it has no display name.',
		},
		'no-teams-read': {
			contexts: CONTEXTS,
			teams: null,
			note: 'The teams read failed. Labels fall back to the bare slug and the empty group drops — no place is lost.',
		},
		empty: {
			contexts: [],
			teams: [],
			note: 'The read answered with nothing.',
		},
		unavailable: {
			contexts: null,
			teams: null,
			note: 'The context read failed. Distinct from empty: the nav cannot claim the reader belongs to nothing.',
		},
	};

	let scenario = $state<Scenario>('groups');
	let current = $derived(SCENARIOS[scenario]);

	// The real app seeds both stores from `(app)/+layout.svelte`, which does not
	// wrap this route. Without this the harness renders every group expanded on
	// load and silently fails to exercise the persistence it exists to show.
	$effect(() => {
		sidebarGroups.init();
	});
</script>

<div class="flex h-screen bg-zinc-950 text-zinc-100">
	<Sidebar
		contexts={current.contexts}
		teams={current.teams}
		selfProfileId={SELF}
		user={{ display_name: 'Operator', email: 'operator@example.com' }}
		instanceName={null}
		collapsed={sidebarCollapsed.value}
		onToggle={() => sidebarCollapsed.toggle()}
	/>
	<main class="flex-1 overflow-y-auto p-6">
		<h1 class="font-mono text-sm tracking-widest text-zinc-400 uppercase">Nav render harness</h1>
		<div class="mt-4 flex flex-wrap gap-2">
			{#each Object.keys(SCENARIOS) as key (key)}
				<button
					type="button"
					onclick={() => (scenario = key as Scenario)}
					class="rounded border px-2.5 py-1 text-xs {scenario === key
						? 'border-quiet-accent text-zinc-100'
						: 'border-zinc-700 text-zinc-400 hover:text-zinc-200'}"
				>
					{key}
				</button>
			{/each}
		</div>
		<p class="mt-4 max-w-xl text-sm text-zinc-400">{current.note}</p>
		<p class="mt-6 max-w-xl text-xs text-zinc-600">
			Group collapse persists to <code>temper.sidebar.groups.collapsed</code>, the same store the
			rail's own collapse uses. Active-place marking is NOT exercised here — it reads the real
			route params, which this harness route does not carry.
		</p>
	</main>
</div>
