import { describe, expect, it } from 'vitest';
import { resourceHref } from './navigation';
import type { GraphNode } from '../types/generated/graph';

function makeNode(partial: Partial<GraphNode>): GraphNode {
	return {
		id: '00000000-0000-0000-0000-000000000000',
		slug: 'my-slug',
		title: 'T',
		doc_type: 'research',
		aggregator: false,
		edge_count: 0,
		session_count: 0,
		excerpt: null,
		stage: null,
		...partial
	};
}

describe('resourceHref', () => {
	it('builds a vault path using owner, context, doc_type, and slug', () => {
		const href = resourceHref(
			'@me',
			'temper',
			makeNode({ doc_type: 'concept', slug: 'idempotency-keys' })
		);
		expect(href).toBe('/vault/@me/temper/concept/idempotency-keys');
	});

	it('uses the exact doc_type from the node', () => {
		expect(
			resourceHref('@me', 'temper', makeNode({ doc_type: 'task', slug: 'auth' }))
		).toBe('/vault/@me/temper/task/auth');
		expect(
			resourceHref('@me', 'temper', makeNode({ doc_type: 'session', slug: 's1' }))
		).toBe('/vault/@me/temper/session/s1');
	});

	it('URL-encodes owner and context params', () => {
		// Defensive: if the context ever contains a space, we don't break the URL.
		const href = resourceHref('alice', 'my context', makeNode({ slug: 'foo' }));
		expect(href).toBe('/vault/alice/my%20context/research/foo');
	});
});
