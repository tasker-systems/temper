/**
 * Encrypted session cookie management.
 *
 * Sessions are stored as a JWE (JSON Web Encryption) cookie using
 * `dir`/`A256GCM`. The symmetric key is derived from `SESSION_SECRET` via
 * SHA-256, so the secret must be at least 32 random bytes' worth of entropy
 * (typically a 64-character hex string or a 44-character base64 string).
 *
 * The session payload holds the access_token, refresh_token (if granted),
 * id_token claims, and the expiry. We do NOT store the temper Profile in the
 * cookie — it's fetched fresh on every request via `hooks.server.ts` to keep
 * entitlements current. The cookie's job is just to remember "who is this
 * person" between requests.
 */

import type { Cookies } from '@sveltejs/kit';
import { CompactEncrypt, compactDecrypt } from 'jose';
import { createHash } from 'node:crypto';
import { SESSION_SECRET } from '$env/static/private';
import type { OidcIdTokenClaims } from './oidc';

const COOKIE_NAME = 'temper_session';
const PKCE_COOKIE_NAME = 'temper_pkce';
const COOKIE_MAX_AGE_SECONDS = 60 * 60 * 24 * 30; // 30 days
const PKCE_COOKIE_MAX_AGE_SECONDS = 60 * 10; // 10 minutes — login flow window

/**
 * Persistent session payload — survives across requests.
 */
export interface SessionData {
	accessToken: string;
	refreshToken: string | null;
	idTokenClaims: OidcIdTokenClaims;
	/** Unix seconds when the access_token expires. */
	expiresAt: number;
}

/**
 * Short-lived state held during the redirect dance between /auth/login and
 * /auth/callback. Contains the CSRF state, PKCE verifier, and where to send
 * the user after a successful login.
 */
export interface PkceData {
	state: string;
	verifier: string;
	returnTo: string;
}

const sessionKey = createHash('sha256').update(SESSION_SECRET).digest();

async function encrypt(payload: object): Promise<string> {
	const json = new TextEncoder().encode(JSON.stringify(payload));
	return new CompactEncrypt(json)
		.setProtectedHeader({ alg: 'dir', enc: 'A256GCM' })
		.encrypt(sessionKey);
}

async function decrypt<T>(token: string): Promise<T> {
	const { plaintext } = await compactDecrypt(token, sessionKey);
	return JSON.parse(new TextDecoder().decode(plaintext)) as T;
}

// ---------------------------------------------------------------------------
// Main session cookie
// ---------------------------------------------------------------------------

export async function readSession(cookies: Cookies): Promise<SessionData | null> {
	const raw = cookies.get(COOKIE_NAME);
	if (!raw) return null;
	try {
		return await decrypt<SessionData>(raw);
	} catch {
		// Tampered or rotated key — drop the cookie and treat as logged out.
		cookies.delete(COOKIE_NAME, { path: '/' });
		return null;
	}
}

export async function writeSession(cookies: Cookies, data: SessionData): Promise<void> {
	const encrypted = await encrypt(data);
	cookies.set(COOKIE_NAME, encrypted, {
		path: '/',
		httpOnly: true,
		secure: true,
		sameSite: 'lax',
		maxAge: COOKIE_MAX_AGE_SECONDS
	});
}

export function clearSession(cookies: Cookies): void {
	cookies.delete(COOKIE_NAME, { path: '/' });
}

// ---------------------------------------------------------------------------
// PKCE / state cookie (set in /auth/login, consumed in /auth/callback)
// ---------------------------------------------------------------------------

export async function readPkce(cookies: Cookies): Promise<PkceData | null> {
	const raw = cookies.get(PKCE_COOKIE_NAME);
	if (!raw) return null;
	try {
		return await decrypt<PkceData>(raw);
	} catch {
		cookies.delete(PKCE_COOKIE_NAME, { path: '/' });
		return null;
	}
}

export async function writePkce(cookies: Cookies, data: PkceData): Promise<void> {
	const encrypted = await encrypt(data);
	cookies.set(PKCE_COOKIE_NAME, encrypted, {
		path: '/',
		httpOnly: true,
		secure: true,
		sameSite: 'lax',
		maxAge: PKCE_COOKIE_MAX_AGE_SECONDS
	});
}

export function clearPkce(cookies: Cookies): void {
	cookies.delete(PKCE_COOKIE_NAME, { path: '/' });
}
