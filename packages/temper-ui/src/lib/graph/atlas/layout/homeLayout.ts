/**
 * Canonical @me home layout (spec C2-D2): you → teams, the membership-graph half
 * C2 ships. Deterministic two-column layout; the Atlas Home chunk grows a third
 * (cogmap) column onto this same shape. Pure.
 */
import type { TeamRow } from '$lib/types/generated/team';

export interface HomeNode {
	id: string;
	name: string;
	x: number;
	y: number;
	kind: 'you' | 'team';
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
	edges: HomeEdge[];
}

export function layoutHome(teams: TeamRow[], size: { width: number; height: number }): HomeGraph {
	const you: HomeNode = { id: 'you', name: 'you', x: size.width * 0.16, y: size.height / 2, kind: 'you' };
	const teamX = size.width * 0.58;
	const top = size.height * 0.12;
	const span = size.height * 0.76;
	const step = teams.length > 1 ? span / (teams.length - 1) : 0;

	const teamNodes: HomeNode[] = teams.map((t, i) => ({
		id: t.id,
		name: t.name,
		x: teamX,
		y: teams.length === 1 ? size.height / 2 : top + i * step,
		kind: 'team'
	}));

	const edges: HomeEdge[] = teamNodes.map((t) => ({ fromX: you.x, fromY: you.y, toX: t.x, toY: t.y }));

	return { you, teams: teamNodes, edges };
}
