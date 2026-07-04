<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TeamRow } from '$lib/types/generated/team';
	import { layoutHome } from '$lib/graph/atlas/layout/homeLayout';
	import { buildScopeUrl } from '$lib/graph/atlas/nav';

	interface Props {
		teams: TeamRow[];
		width: number;
		height: number;
	}
	let { teams, width, height }: Props = $props();

	const g = $derived(layoutHome(teams, { width, height }));

	function enterTeam(teamId: string) {
		goto(buildScopeUrl($page.url, teamId), { replaceState: true });
	}
</script>

<text x={width / 2} y="28" text-anchor="middle" fill="#5f7686" font-size="11" letter-spacing="1">YOUR TEAMS</text>

{#each g.edges as e, i (i)}
	<line x1={e.fromX} y1={e.fromY} x2={e.toX} y2={e.toY} stroke="#8b93a5" stroke-opacity="0.5" />
{/each}

<circle cx={g.you.x} cy={g.you.y} r="22" fill="#cfd6e2" fill-opacity="0.14" stroke="#cfd6e2" stroke-width="1.5" />
<text x={g.you.x} y={g.you.y + 4} text-anchor="middle" fill="#cfd6e2" font-size="11">you</text>

{#each g.teams as t (t.id)}
	<g role="button" tabindex="0" onclick={() => enterTeam(t.id)} onkeydown={(e) => e.key === 'Enter' && enterTeam(t.id)} style="cursor:pointer">
		<rect x={t.x - 90} y={t.y - 19} width="180" height="38" rx="8" fill="#3a8ae8" fill-opacity="0.12" stroke="#6fa8c7" stroke-opacity="0.6" />
		<text x={t.x} y={t.y + 4} text-anchor="middle" fill="#9fc4d6" font-size="11" font-weight="600">{t.name} ↵</text>
	</g>
{/each}
