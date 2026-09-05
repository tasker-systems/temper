// harness.ts — committed, personal-data-free fixtures for the vault render harness.
//
// Everything here is invented: ids, hashes, titles, numbers. The vocabulary they exercise is
// real — these fixtures deliberately cover the whole closed vocabulary the three shipped
// surfaces render, because a harness that shows only the happy path is a harness that has
// never seen the states a reader will actually ask about.
import type { ArtifactView } from '$lib/types/generated/data_artifact';
import type { ShapeView } from '$lib/types/generated/data_artifact_shape';
import type { ElementEvent, EventTrail } from '$lib/types/generated/element_trail';
import type { GraphEdgeRow } from '$lib/types/generated/graph';

const HASH = '9f2c0a81e4d7b6a35c8f1e0d2a4b7c9e6f3a5d8b1c7e4f2a9d6b3c8e5f1a7d4b';

export const resource = {
	id: '01a0000000000000000000000000harness',
	title: 'Quarterly latency measurements',
	doc_type_name: 'task',
	context_name: 'Temper',
	slug: 'quarterly-latency-measurements',
	managed_meta: {
		'temper-stage': 'done',
		'temper-mode': null,
		'temper-effort': null,
		'temper-status': null,
		'temper-seq': null,
		'temper-branch': null,
		'temper-pr': null,
		'temper-llm-model': null,
		'temper-llm-run': null,
		'temper-provenance': null,
	},
	open_meta: { owner: 'vault-harness', tags: ['latency', 'fixture'] },
};

export const trail: EventTrail = {
	element_kind: 'node',
	element_id: resource.id,
	events: [
		{
			event_id: 'ev-1',
			kind: 'resource_created',
			actor_entity_id: 'ent-1',
			actor_name: 'vault-harness',
			occurred_at: '2026-08-30T10:00:00Z',
			confidence: null,
			payload: {},
		},
		{
			event_id: 'ev-2',
			kind: 'property_set',
			actor_entity_id: 'ent-1',
			actor_name: 'vault-harness',
			occurred_at: '2026-08-30T10:05:00Z',
			confidence: null,
			payload: { property_key: 'temper-stage', value: 'done' },
		},
		{
			event_id: 'ev-3',
			kind: 'data_artifact_committed',
			actor_entity_id: 'ent-2',
			actor_name: 'measure-agent',
			occurred_at: '2026-08-31T09:12:00Z',
			confidence: null,
			payload: {
				artifact_id: '01a0000000000000000000000000art0001',
				artifact_kind: 'measurement',
				intent: 'member',
				precedence: 2,
				content_hash: HASH,
				content_bytes: 1229n,
				supersedes: [],
			},
		},
		{
			event_id: 'ev-4',
			kind: 'data_artifact_committed',
			actor_entity_id: 'ent-2',
			actor_name: 'measure-agent',
			occurred_at: '2026-08-31T09:14:00Z',
			confidence: null,
			payload: {
				artifact_id: '01a0000000000000000000000000art0002',
				artifact_kind: 'measurement',
				intent: 'current',
				precedence: 0,
				content_hash: 'b'.repeat(64),
				content_bytes: 1048576,
				supersedes: ['01a0000000000000000000000000art0001'],
			},
		},
		{
			event_id: 'ev-5',
			kind: 'relationship_asserted',
			actor_entity_id: 'ent-1',
			actor_name: 'vault-harness',
			occurred_at: '2026-08-31T09:15:00Z',
			confidence: null,
			payload: { label: 'relates_to', target: { id: 'absent-from-this-subgraph' } },
		},
	] as unknown as ElementEvent[],
};

export const edges: GraphEdgeRow[] = [
	{
		edge_id: 'edge-1',
		peer_table: 'kb_resources',
		peer_id: 'peer-1',
		peer_title: 'A peer the harness invented',
		peer_slug: 'a-peer-the-harness-invented',
		edge_kind: 'near',
		polarity: 'forward',
		label: 'relates to',
		direction: 'outgoing',
		weight: 0.5,
		created: '2026-08-30T10:00:00Z',
	},
	{
		// A blob peer: addressed by id alone, no title — and the list renders it
		// without a resource-route link.
		edge_id: 'edge-2',
		peer_table: 'kb_blobs',
		peer_id: 'blob-harness-invented',
		peer_title: null,
		peer_slug: null,
		edge_kind: 'express',
		polarity: 'forward',
		label: 'derivation_source',
		direction: 'outgoing',
		weight: 1.0,
		created: '2026-08-30T10:05:00Z',
	},
];

const artifact = (over: Partial<ArtifactView>): ArtifactView => ({
	artifact_id: '01a0000000000000000000000000art0001',
	resource_id: resource.id,
	kind_owner_table: 'kb_profiles',
	kind_owner_id: '00000000-0000-0000-0005-0000000000harness',
	artifact_kind: 'measurement',
	intent: 'member',
	precedence: 0,
	content_hash: HASH,
	content_bytes: 1229n,
	shape_state: 'never_declared',
	is_folded: false,
	created: '2026-08-31T09:12:00Z',
	content: { p50_ms: 412, p95_ms: 880, runs: 12 },
	...over,
});

/** The whole closed vocabulary, one row each — this is what the section must render legibly. */
export const artifacts: ArtifactView[] = [
	artifact({
		artifact_id: '01a0000000000000000000000000art0002',
		artifact_kind: 'measurement',
		intent: 'current',
		shape_state: 'declared_satisfied',
		content_bytes: 1536n,
		created: '2026-08-31T09:14:00Z',
		content: { p50_ms: 398, p95_ms: 845, runs: 20 },
	}),
	artifact({
		artifact_id: '01a0000000000000000000000000art0003',
		artifact_kind: 'extraction',
		intent: 'current',
		shape_state: 'declared_not_satisfied',
		content_bytes: 245760n,
		created: '2026-08-31T10:02:00Z',
		content: { fields: 14, expected: 15, missing: ['region'] },
	}),
	artifact({
		artifact_id: '01a0000000000000000000000000art0004',
		artifact_kind: 'summary',
		intent: 'pinned',
		shape_state: 'declared_not_yet_checked',
		content_bytes: 512n,
		created: '2026-08-31T10:20:00Z',
		content: null,
	}),
	artifact({
		artifact_id: '01a0000000000000000000000000art0001',
		artifact_kind: 'measurement',
		intent: 'member',
		precedence: 2,
		is_folded: true,
		created: '2026-08-31T09:12:00Z',
	}),
];

/** The absence contract's arm: the section renders NOTHING for this — page reads unchanged. */
export const emptyArtifacts: ArtifactView[] = [];

export const shapes: ShapeView[] = [
	{
		shape_id: '01a0000000000000000000000000shp0001',
		home_anchor_table: 'kb_contexts',
		home_anchor_id: '00000000-0000-0000-0003-0000000000harness',
		kind_owner_table: 'kb_profiles',
		kind_owner_id: '00000000-0000-0000-0005-0000000000harness',
		artifact_kind: 'extraction',
		schema: {
			type: 'object',
			required: ['fields'],
			properties: { fields: { type: 'integer', minimum: 0 } },
		},
		enforcement: 'enforcing',
		shape_version: 3,
		is_folded: false,
		created: '2026-08-22T10:00:00Z',
	},
	{
		shape_id: '01a0000000000000000000000000shp0002',
		home_anchor_table: 'kb_contexts',
		home_anchor_id: '00000000-0000-0000-0003-0000000000harness',
		kind_owner_table: 'kb_profiles',
		kind_owner_id: '00000000-0000-0000-0005-0000000000harness',
		artifact_kind: 'measurement',
		schema: { type: 'object', required: ['p50_ms'], properties: { p50_ms: { type: 'number' } } },
		enforcement: 'advisory',
		shape_version: 1,
		is_folded: false,
		created: '2026-08-21T10:00:00Z',
	},
];
