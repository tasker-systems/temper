import { env } from '$env/dynamic/private';
import { activeTraceparent } from './telemetry/context';

// Read at runtime (not build-inlined) so the upstream API origin is configured
// purely through env, consistent with the reverse proxy in `proxy.ts`. Both read
// the same `API_BASE_URL`; binding them the same way avoids one taking effect at
// runtime while the other needs a rebuild.
const API_BASE_URL = env.API_BASE_URL ?? '';

/**
 * Merge the active UI request span's `traceparent` into an outbound header set, so the
 * server-side data loaders propagate the span to temper-api. This is the SSR half of
 * closing the "internal dangle": unlike the reverse proxy, these loader fetches carried
 * no trace context at all. No-op when span export is disabled. See `./telemetry/context`
 * and task `019fbf24`.
 */
function traced(headers: Record<string, string>): Record<string, string> {
	const traceparent = activeTraceparent();
	return traceparent ? { ...headers, traceparent } : headers;
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
		headers: traced({ Authorization: `Bearer ${accessToken}` }),
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
		headers: traced({
			Authorization: `Bearer ${accessToken}`,
			'Content-Type': 'application/json',
		}),
		body: JSON.stringify(body),
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
		headers: traced({
			Authorization: `Bearer ${accessToken}`,
			'Content-Type': 'application/json',
		}),
		body: JSON.stringify(body),
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
		headers: traced({ Authorization: `Bearer ${accessToken}` }),
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
