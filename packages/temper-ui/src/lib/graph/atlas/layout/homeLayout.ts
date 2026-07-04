/**
 * Canonical @me home layout (spec C2-D2, grown by the Atlas Home chunk): you →
 * teams → visible cogmaps, a three-column membership graph. Deterministic; pure.
 */
import type { HomeCogmap, HomeTeam } from '$lib/types/generated/graph_home';

export interface HomeNode {
	id: string;
	name: string;
	x: number;
	y: number;
	kind: 'you' | 'team' | 'cogmap';
}

export interface HomeEdge {
	fromX: number;
	fromY: number;
	toX: number;
	toY: number;
}

export interface HomeGraph {
	you: HomeNode;
	teams: HomeNode[];
	cogmaps: HomeNode[];
	edges: HomeEdge[];
	cogmapEdges: HomeEdge[];
}

function columnNodes(
	items: { id: string; name: string }[],
	x: number,
	size: { width: number; height: number },
	kind: 'team' | 'cogmap'
): HomeNode[] {
	const top = size.height * 0.12;
	const span = size.height * 0.76;
	const step = items.length > 1 ? span / (items.length - 1) : 0;
	return items.map((it, i) => ({
		id: it.id,
		name: it.name,
		x,
		y: items.length === 1 ? size.height / 2 : top + i * step,
		kind
	}));
}

export function layoutHome(
	teams: HomeTeam[],
	cogmaps: HomeCogmap[],
	size: { width: number; height: number }
): HomeGraph {
	const you: HomeNode = { id: 'you', name: 'you', x: size.width * 0.16, y: size.height / 2, kind: 'you' };
	const teamX = size.width * 0.52;
	const cogmapX = size.width * 0.86;

	const teamNodes = columnNodes(teams, teamX, size, 'team');
	const cogmapNodes = columnNodes(cogmaps, cogmapX, size, 'cogmap');

	const edges: HomeEdge[] = teamNodes.map((t) => ({ fromX: you.x, fromY: you.y, toX: t.x, toY: t.y }));

	const teamById = new Map(teamNodes.map((t) => [t.id, t]));
	const cogmapById = new Map(cogmapNodes.map((c) => [c.id, c]));
	const cogmapEdges: HomeEdge[] = [];
	for (const c of cogmaps) {
		const cNode = cogmapById.get(c.id)!;
		for (const teamId of c.team_ids) {
			const t = teamById.get(teamId);
			if (t) cogmapEdges.push({ fromX: t.x, fromY: t.y, toX: cNode.x, toY: cNode.y });
		}
	}

	return { you, teams: teamNodes, cogmaps: cogmapNodes, edges, cogmapEdges };
}
