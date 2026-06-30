import { describe, expect, it } from 'vitest';
import { isProxiedPath, buildUpstreamUrl } from './proxy';

describe('isProxiedPath', () => {
	it('matches the MCP entrypoint (exact and subpaths)', () => {
		expect(isProxiedPath('/mcp')).toBe(true);
		expect(isProxiedPath('/mcp/messages')).toBe(true);
	});

	it('matches OAuth, discovery, and API prefixes', () => {
		expect(isProxiedPath('/oauth/token')).toBe(true);
		expect(isProxiedPath('/.well-known/openid-configuration')).toBe(true);
		expect(isProxiedPath('/.well-known/oauth-authorization-server')).toBe(true);
		expect(isProxiedPath('/api/profile')).toBe(true);
		expect(isProxiedPath('/api/resources?q=x')).toBe(true);
	});

	it('does NOT match the app, auth, or marketing routes the UI owns', () => {
		expect(isProxiedPath('/')).toBe(false);
		expect(isProxiedPath('/vault/all')).toBe(false);
		expect(isProxiedPath('/auth/login')).toBe(false);
		expect(isProxiedPath('/manifesto')).toBe(false);
	});

	it('does not over-match prefixes that merely share a leading substring', () => {
		expect(isProxiedPath('/mcpfoo')).toBe(false);
		expect(isProxiedPath('/apilike')).toBe(false);
		expect(isProxiedPath('/oauthish')).toBe(false);
	});
});

describe('buildUpstreamUrl', () => {
	it('joins the upstream base with the request path and query', () => {
		expect(buildUpstreamUrl('https://api.example.com', '/api/profile', '?q=x')).toBe(
			'https://api.example.com/api/profile?q=x'
		);
	});

	it('preserves an empty query string', () => {
		expect(buildUpstreamUrl('https://api.example.com', '/mcp', '')).toBe(
			'https://api.example.com/mcp'
		);
	});

	it('tolerates a trailing slash on the upstream base', () => {
		expect(buildUpstreamUrl('https://api.example.com/', '/oauth/token', '')).toBe(
			'https://api.example.com/oauth/token'
		);
	});
});
