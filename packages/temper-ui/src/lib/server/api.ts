import { activeTraceparent } from '@tasker-systems/temper-telemetry-ts';
import { env } from '$env/dynamic/private';
import { EDGE_PROXY_HEADER, edgeProxySecret } from './edge-proxy';
import { jsonBody } from './json-body';

// Read at runtime (not build-inlined) so the upstream API origin is configured
// purely through env, consistent with the reverse proxy in `proxy.ts`. Both read
// the same `API_BASE_URL`; binding them the same way avoids one taking effect at
// runtime while the other needs a rebuild.
const API_BASE_URL = env.API_BASE_URL ?? '';

/**
 * The headers every loader request to temper-api carries beyond its own.
 *
 * - The active UI request span's `traceparent`, so the loaders propagate the span to the API:
 *   the SSR half of closing the "internal dangle" (the reverse proxy sets it separately). Absent
 *   when span export is disabled. See `temper-telemetry-ts` and task `019fbf24`.
 * - The edge-proxy marker, when configured, so the API's per-IP edge rules exempt these
 *   requests as they exempt the proxy's (see `edge-proxy.ts`).
 */
export function outbound(headers: Record<string, string>): Record<string, string> {
	const out = { ...headers };
	const traceparent = activeTraceparent();
	if (traceparent) out.traceparent = traceparent;
	const marker = edgeProxySecret();
	if (marker) out[EDGE_PROXY_HEADER] = marker;
	return out;
}

export class ApiError extends Error {
	status: number;
	body: unknown;

	constructor(status: number, message: string, body?: unknown) {
		super(message);
		this.name = 'ApiError';
		this.status = status;
		this.body = body;
	}
}

export async function apiGet<T>(path: string, accessToken: string): Promise<T> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		headers: outbound({ Authorization: `Bearer ${accessToken}` }),
	});
	if (!res.ok) {
		const body = await res.json().catch(() => ({}));
		throw new ApiError(
			res.status,
			((body as Record<string, unknown>).message as string) ?? `HTTP ${res.status}`,
			body,
		);
	}
	return res.json() as Promise<T>;
}

export async function apiPost<T>(path: string, accessToken: string, body: unknown): Promise<T> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'POST',
		headers: outbound({
			Authorization: `Bearer ${accessToken}`,
			'Content-Type': 'application/json',
		}),
		body: jsonBody(body),
	});
	if (!res.ok) {
		const errBody = await res.json().catch(() => ({}));
		throw new ApiError(
			res.status,
			((errBody as Record<string, unknown>).message as string) ?? `HTTP ${res.status}`,
			errBody,
		);
	}
	return res.json() as Promise<T>;
}

export async function apiPatch<T>(path: string, accessToken: string, body: unknown): Promise<T> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'PATCH',
		headers: outbound({
			Authorization: `Bearer ${accessToken}`,
			'Content-Type': 'application/json',
		}),
		body: jsonBody(body),
	});
	if (!res.ok) {
		const errBody = await res.json().catch(() => ({}));
		throw new ApiError(
			res.status,
			((errBody as Record<string, unknown>).message as string) ?? `HTTP ${res.status}`,
			errBody,
		);
	}
	return res.json() as Promise<T>;
}

export async function apiDelete(path: string, accessToken: string): Promise<void> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'DELETE',
		headers: outbound({ Authorization: `Bearer ${accessToken}` }),
	});
	if (!res.ok) {
		const body = await res.json().catch(() => ({}));
		throw new ApiError(
			res.status,
			((body as Record<string, unknown>).message as string) ?? `HTTP ${res.status}`,
			body,
		);
	}
}
