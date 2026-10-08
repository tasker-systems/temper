import { beforeEach, describe, expect, it, vi } from 'vitest';

// The deployment's environment, mutable per test. `edgeProxySecret` reads it on every call.
const env: Record<string, string | undefined> = {};
vi.mock('$env/dynamic/private', () => ({ env }));

const { EDGE_PROXY_HEADER, edgeProxySecret } = await import('./edge-proxy');
const { outbound } = await import('./api');

beforeEach(() => {
	delete env.TEMPER_EDGE_PROXY_SECRET;
});

describe('edgeProxySecret', () => {
	it('is the configured value, trimmed', () => {
		env.TEMPER_EDGE_PROXY_SECRET = '  deploy-secret\n';
		expect(edgeProxySecret()).toBe('deploy-secret');
	});

	it('is absent when unset or blank, so nothing is sent', () => {
		expect(edgeProxySecret()).toBeUndefined();
		env.TEMPER_EDGE_PROXY_SECRET = '   ';
		expect(edgeProxySecret()).toBeUndefined();
	});

	// A wrapped paste would otherwise make every forwarded request throw, and the runtime's
	// error message quotes the value into the logs.
	it('is not sent when it cannot be a header value, and the log never quotes it', () => {
		const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
		env.TEMPER_EDGE_PROXY_SECRET = 'TOPSECRET\nVALUE';
		expect(edgeProxySecret()).toBeUndefined();
		expect(logged).toHaveBeenCalledTimes(1);
		expect(JSON.stringify(logged.mock.calls)).not.toContain('TOPSECRET');
		logged.mockRestore();
	});
});

// The loaders reach the API from the same few addresses as the proxy; without the marker the
// API's per-IP rules would put every server-side render in one bucket.
describe('outbound (the loaders’ request headers)', () => {
	it('carries the edge-proxy marker when configured', () => {
		env.TEMPER_EDGE_PROXY_SECRET = 'deploy-secret';
		expect(outbound({ Authorization: 'Bearer tok' })).toMatchObject({
			Authorization: 'Bearer tok',
			[EDGE_PROXY_HEADER]: 'deploy-secret',
		});
	});

	it('carries no marker when unconfigured', () => {
		expect(outbound({ Authorization: 'Bearer tok' })).not.toHaveProperty(EDGE_PROXY_HEADER);
	});
});
