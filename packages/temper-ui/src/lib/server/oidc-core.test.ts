import { describe, expect, it } from 'vitest';
import {
	resolveOidcConfig,
	parseDiscovery,
	buildAuthorizeUrl,
	buildLogoutUrl,
	decodeIdToken
} from './oidc-core';

describe('resolveOidcConfig', () => {
	it('prefers OIDC_* over AUTH0_* when both are present', () => {
		const cfg = resolveOidcConfig({
			OIDC_ISSUER: 'https://org.okta.com/oauth2/abc',
			OIDC_CLIENT_ID: 'oidc-client',
			OIDC_CLIENT_SECRET: 'oidc-secret',
			OIDC_AUDIENCE: 'oidc-aud',
			AUTH0_DOMAIN: 'tenant.us.auth0.com',
			AUTH0_CLIENT_ID: 'auth0-client',
			AUTH0_CLIENT_SECRET: 'auth0-secret',
			AUTH0_AUDIENCE: 'auth0-aud'
		});
		expect(cfg).toEqual({
			issuer: 'https://org.okta.com/oauth2/abc',
			clientId: 'oidc-client',
			clientSecret: 'oidc-secret',
			audience: 'oidc-aud'
		});
	});

	it('falls back to AUTH0_* (canonical deployment) when OIDC_* is absent', () => {
		const cfg = resolveOidcConfig({
			AUTH0_DOMAIN: 'temperkb.us.auth0.com',
			AUTH0_CLIENT_ID: 'auth0-client',
			AUTH0_CLIENT_SECRET: 'auth0-secret',
			AUTH0_AUDIENCE: 'https://api.temperkb.io'
		});
		expect(cfg).toEqual({
			issuer: 'https://temperkb.us.auth0.com',
			clientId: 'auth0-client',
			clientSecret: 'auth0-secret',
			audience: 'https://api.temperkb.io'
		});
	});

	it('treats audience as optional and omits it when unset', () => {
		const cfg = resolveOidcConfig({
			OIDC_ISSUER: 'https://org.okta.com/oauth2/abc',
			OIDC_CLIENT_ID: 'c',
			OIDC_CLIENT_SECRET: 's'
		});
		expect(cfg.audience).toBeUndefined();
	});

	it('treats an empty-string audience as unset', () => {
		const cfg = resolveOidcConfig({
			OIDC_ISSUER: 'https://org.okta.com/oauth2/abc',
			OIDC_CLIENT_ID: 'c',
			OIDC_CLIENT_SECRET: 's',
			OIDC_AUDIENCE: ''
		});
		expect(cfg.audience).toBeUndefined();
	});

	it('strips a trailing slash from the issuer', () => {
		const cfg = resolveOidcConfig({
			OIDC_ISSUER: 'https://org.okta.com/oauth2/abc/',
			OIDC_CLIENT_ID: 'c',
			OIDC_CLIENT_SECRET: 's'
		});
		expect(cfg.issuer).toBe('https://org.okta.com/oauth2/abc');
	});

	it('throws when no issuer can be resolved', () => {
		expect(() =>
			resolveOidcConfig({ OIDC_CLIENT_ID: 'c', OIDC_CLIENT_SECRET: 's' })
		).toThrow(/issuer/i);
	});

	it('throws when client id is missing', () => {
		expect(() =>
			resolveOidcConfig({ OIDC_ISSUER: 'https://x', OIDC_CLIENT_SECRET: 's' })
		).toThrow(/client id/i);
	});

	it('resolves clientSecret as undefined (does not throw) for a public PKCE client', () => {
		const cfg = resolveOidcConfig({ OIDC_ISSUER: 'https://x', OIDC_CLIENT_ID: 'c' });
		expect(cfg.clientSecret).toBeUndefined();
	});

	it('resolves discoveryUrl from OIDC_DISCOVERY_URL, trimming a trailing slash', () => {
		const cfg = resolveOidcConfig({
			OIDC_ISSUER: 'https://as.example.com',
			OIDC_CLIENT_ID: 'temper-ui',
			OIDC_DISCOVERY_URL: 'https://as.example.com/.well-known/oauth-authorization-server/'
		});
		expect(cfg.discoveryUrl).toBe('https://as.example.com/.well-known/oauth-authorization-server');
	});

	it('leaves discoveryUrl undefined when OIDC_DISCOVERY_URL is unset', () => {
		const cfg = resolveOidcConfig({
			OIDC_ISSUER: 'https://org.okta.com/oauth2/abc',
			OIDC_CLIENT_ID: 'c',
			OIDC_CLIENT_SECRET: 's'
		});
		expect(cfg.discoveryUrl).toBeUndefined();
	});
});

describe('parseDiscovery', () => {
	it('extracts the endpoints from a discovery document', () => {
		const endpoints = parseDiscovery({
			authorization_endpoint: 'https://idp/authorize',
			token_endpoint: 'https://idp/token',
			end_session_endpoint: 'https://idp/logout',
			jwks_uri: 'https://idp/jwks',
			extra_field_we_ignore: true
		});
		expect(endpoints).toEqual({
			authorization_endpoint: 'https://idp/authorize',
			token_endpoint: 'https://idp/token',
			end_session_endpoint: 'https://idp/logout',
			jwks_uri: 'https://idp/jwks'
		});
	});

	it('tolerates a missing end_session_endpoint (provider has no RP-initiated logout)', () => {
		const endpoints = parseDiscovery({
			authorization_endpoint: 'https://idp/authorize',
			token_endpoint: 'https://idp/token'
		});
		expect(endpoints.end_session_endpoint).toBeUndefined();
	});

	it('throws when a required endpoint is missing', () => {
		expect(() => parseDiscovery({ token_endpoint: 'https://idp/token' })).toThrow(
			/authorization_endpoint/
		);
	});

	it('throws when the payload is not an object', () => {
		expect(() => parseDiscovery('nope')).toThrow();
	});
});

describe('buildAuthorizeUrl', () => {
	const endpoint = 'https://idp/authorize';
	const base = { clientId: 'client-123', redirectUri: 'https://app/auth/callback' };

	it('builds the standard Authorization Code + PKCE request', () => {
		const url = new URL(buildAuthorizeUrl(endpoint, base, 'state-xyz', 'challenge-abc'));
		expect(url.origin + url.pathname).toBe(endpoint);
		const p = url.searchParams;
		expect(p.get('response_type')).toBe('code');
		expect(p.get('client_id')).toBe('client-123');
		expect(p.get('redirect_uri')).toBe('https://app/auth/callback');
		expect(p.get('scope')).toBe('openid profile email offline_access');
		expect(p.get('state')).toBe('state-xyz');
		expect(p.get('code_challenge')).toBe('challenge-abc');
		expect(p.get('code_challenge_method')).toBe('S256');
	});

	it('includes audience when configured (Auth0)', () => {
		const url = new URL(
			buildAuthorizeUrl(endpoint, { ...base, audience: 'https://api' }, 's', 'c')
		);
		expect(url.searchParams.get('audience')).toBe('https://api');
	});

	it('omits audience when not configured (Okta custom auth server)', () => {
		const url = new URL(buildAuthorizeUrl(endpoint, base, 's', 'c'));
		expect(url.searchParams.has('audience')).toBe(false);
	});
});

describe('buildLogoutUrl', () => {
	it('builds an RP-initiated logout URL from end_session_endpoint', () => {
		const url = new URL(
			buildLogoutUrl('https://idp/logout', {
				clientId: 'client-123',
				returnTo: 'https://app/'
			})
		);
		expect(url.origin + url.pathname).toBe('https://idp/logout');
		expect(url.searchParams.get('post_logout_redirect_uri')).toBe('https://app/');
		expect(url.searchParams.get('client_id')).toBe('client-123');
		expect(url.searchParams.has('id_token_hint')).toBe(false);
	});

	it('includes id_token_hint when an id token is supplied', () => {
		const url = new URL(
			buildLogoutUrl('https://idp/logout', {
				clientId: 'client-123',
				returnTo: 'https://app/',
				idToken: 'the.id.token'
			})
		);
		expect(url.searchParams.get('id_token_hint')).toBe('the.id.token');
	});

	it('falls back to returnTo when the provider has no end_session_endpoint', () => {
		expect(
			buildLogoutUrl(undefined, { clientId: 'client-123', returnTo: 'https://app/' })
		).toBe('https://app/');
	});
});

describe('decodeIdToken', () => {
	it('decodes the claims from a JWT id_token without verifying the signature', () => {
		const claims = { sub: 'auth0|123', email: 'a@b.com', name: 'A B', exp: 1, iat: 0 };
		const b64url = (obj: object) =>
			Buffer.from(JSON.stringify(obj))
				.toString('base64')
				.replace(/=/g, '')
				.replace(/\+/g, '-')
				.replace(/\//g, '_');
		const idToken = `${b64url({ alg: 'RS256' })}.${b64url(claims)}.sig`;
		expect(decodeIdToken(idToken)).toMatchObject({ sub: 'auth0|123', email: 'a@b.com' });
	});

	it('throws on a malformed token', () => {
		expect(() => decodeIdToken('not-a-jwt')).toThrow();
	});
});
