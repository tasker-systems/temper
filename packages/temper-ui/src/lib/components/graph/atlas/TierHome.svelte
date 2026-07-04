<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { HomeCogmap, HomeTeam } from '$lib/types/generated/graph_home';
	import { layoutHome } from '$lib/graph/atlas/layout/homeLayout';
	import { buildCogmapUrl, buildScopeUrl } from '$lib/graph/atlas/nav';
	import { COGMAP_DOOR, TEAM_DOOR } from '$lib/graph/atlas/palette';

	interface Props {
		teams: HomeTeam[];
		cogmaps: HomeCogmap[];
		width: number;
		height: number;
	}
	let { teams, cogmaps, width, height }: Props = $props();

	const g = $derived(layoutHome(teams, cogmaps, { width, height }));
	const teamById = $derived(new Map(teams.map((t) => [t.id, t])));
	const cogmapById = $derived(new Map(cogmaps.map((c) => [c.id, c])));

	function enterTeam(teamId: string) {
		goto(buildScopeUrl($page.url, teamId), { replaceState: true });
	}
	function enterCogmap(cogmapId: string) {
		goto(buildCogmapUrl($page.url, cogmapId), { replaceState: true });
	}

	// D4-threshold fix: `onpointerup` alone (see comment below) also fires at the
	// END of a pan gesture — d3-zoom's camera sees the down/up pair as a drag, but
	// the pointerup still lands on whichever door is under the cursor on release,
	// so panning across a door and letting go on top of it used to navigate.
	// Fix: remember where the pointer went down and only treat pointerup as an
	// activation if it released within POINTER_MOVE_THRESHOLD px (euclidean) of
	// that point — a stationary click is ~0px and always passes; a real pan
	// exceeds it and is ignored here (d3-zoom handles the pan itself).
	const POINTER_MOVE_THRESHOLD = 6;
	let downPt = $state<{ x: number; y: number } | null>(null);

	function onDoorPointerDown(e: PointerEvent) {
		downPt = { x: e.clientX, y: e.clientY };
	}
	function onDoorPointerUp(e: PointerEvent, activate: () => void) {
		if (!downPt) return;
		const dx = e.clientX - downPt.x;
		const dy = e.clientY - downPt.y;
		downPt = null;
		if (Math.hypot(dx, dy) < POINTER_MOVE_THRESHOLD) activate();
	}
</script>

<text x={width * 0.34} y="28" text-anchor="middle" fill="#5f7686" font-size="11" letter-spacing="1">YOUR TEAMS</text>
<text x={width * 0.86} y="28" text-anchor="middle" fill="#5f7686" font-size="11" letter-spacing="1">COGMAPS</text>

{#each g.edges as e, i (i)}
	<line x1={e.fromX} y1={e.fromY} x2={e.toX} y2={e.toY} stroke="#8b93a5" stroke-opacity="0.5" />
{/each}
{#each g.cogmapEdges as e, i (i)}
	<line x1={e.fromX} y1={e.fromY} x2={e.toX} y2={e.toY} stroke="#8b93a5" stroke-opacity="0.35" />
{/each}

<circle cx={g.you.x} cy={g.you.y} r="22" fill="#cfd6e2" fill-opacity="0.14" stroke="#cfd6e2" stroke-width="1.5" />
<text x={g.you.x} y={g.you.y + 4} text-anchor="middle" fill="#cfd6e2" font-size="11">you</text>

<!--
	D4 fix: the C2 door only entered a team on a SECOND click. Root cause is
	d3-zoom's camera, attached to the whole canvas <svg> (see camera.ts) — every
	mousedown/mouseup on ANY child (including these doors) is first captured by
	d3-zoom's internal pan/drag machinery, which — if it sees any pointer jitter
	between down and up (routine on trackpads/real mice) — installs a one-shot,
	capturing `click` listener on `window` that swallows the very next native
	`click` event to suppress click-through after a real pan gesture. That eats
	this door's first click; by the second click the one-shot listener is gone.
	Fix: activate on `pointerup` (fires before d3-zoom's click-swallow listener
	can intercept it) rather than `onclick`, so a single click always enters.
	`onclick` stays wired too (harmless/idempotent — same URL) so activation via
	assistive tech that synthesizes a `click` without a preceding pointer event
	still works. `onkeydown` (Enter) remains the keyboard path.

	Follow-up: `pointerup` alone also fires at the end of a PAN — releasing the
	pointer on top of a door after dragging across the canvas used to navigate.
	`onDoorPointerDown`/`onDoorPointerUp` (above) gate activation on the pointer
	having moved less than `POINTER_MOVE_THRESHOLD` px between down and up, so a
	pan is ignored here (d3-zoom drives the pan) while a stationary click
	(~0px movement) still enters.
-->
{#each g.teams as t (t.id)}
	{@const team = teamById.get(t.id)}
	<g
		role="button"
		tabindex="0"
		aria-label={t.name}
		onclick={() => enterTeam(t.id)}
		onpointerdown={onDoorPointerDown}
		onpointerup={(e) => onDoorPointerUp(e, () => enterTeam(t.id))}
		onkeydown={(e) => e.key === 'Enter' && enterTeam(t.id)}
		style="cursor:pointer"
	>
		<rect x={t.x - 90} y={t.y - 22} width="180" height="46" rx="8" fill={TEAM_DOOR.fill} stroke={TEAM_DOOR.stroke} stroke-opacity="0.6" />
		<text x={t.x} y={t.y - 2} text-anchor="middle" fill={TEAM_DOOR.ink} font-size="11" font-weight="600">{t.name} ↵</text>
		{#if team}
			<text x={t.x} y={t.y + 14} text-anchor="middle" fill={TEAM_DOOR.ink} font-size="9" opacity="0.75">
				{team.resource_count} res · {team.cogmap_count} maps
			</text>
		{/if}
	</g>
{/each}

{#each g.cogmaps as c (c.id)}
	{@const cogmap = cogmapById.get(c.id)}
	<g
		role="button"
		tabindex="0"
		aria-label={c.name}
		onclick={() => enterCogmap(c.id)}
		onpointerdown={onDoorPointerDown}
		onpointerup={(e) => onDoorPointerUp(e, () => enterCogmap(c.id))}
		onkeydown={(e) => e.key === 'Enter' && enterCogmap(c.id)}
		style="cursor:pointer"
	>
		<rect x={c.x - 90} y={c.y - 22} width="180" height="46" rx="8" fill={COGMAP_DOOR.fill} stroke={COGMAP_DOOR.stroke} stroke-opacity="0.6" />
		<text x={c.x} y={c.y - 2} text-anchor="middle" fill={COGMAP_DOOR.ink} font-size="11" font-weight="600">{c.name} ↵</text>
		{#if cogmap}
			<text x={c.x} y={c.y + 14} text-anchor="middle" fill={COGMAP_DOOR.ink} font-size="9" opacity="0.75">
				{cogmap.region_count} regions · {cogmap.facet_count} facets
			</text>
		{/if}
	</g>
{/each}
