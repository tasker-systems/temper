import { API_BASE_URL } from '$env/static/private';

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
		headers: { Authorization: `Bearer ${accessToken}` }
	});
	if (!res.ok) {
		const body = await res.json().catch(() => ({}));
		throw new ApiError(res.status, (body as Record<string, unknown>).message as string ?? `HTTP ${res.status}`, body);
	}
	return res.json() as Promise<T>;
}

export async function apiPost<T>(path: string, accessToken: string, body: unknown): Promise<T> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'POST',
		headers: {
			Authorization: `Bearer ${accessToken}`,
			'Content-Type': 'application/json'
		},
		body: JSON.stringify(body)
	});
	if (!res.ok) {
		const errBody = await res.json().catch(() => ({}));
		throw new ApiError(res.status, (errBody as Record<string, unknown>).message as string ?? `HTTP ${res.status}`, errBody);
	}
	return res.json() as Promise<T>;
}

export async function apiPatch<T>(path: string, accessToken: string, body: unknown): Promise<T> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'PATCH',
		headers: {
			Authorization: `Bearer ${accessToken}`,
			'Content-Type': 'application/json'
		},
		body: JSON.stringify(body)
	});
	if (!res.ok) {
		const errBody = await res.json().catch(() => ({}));
		throw new ApiError(res.status, (errBody as Record<string, unknown>).message as string ?? `HTTP ${res.status}`, errBody);
	}
	return res.json() as Promise<T>;
}

export async function apiDelete(path: string, accessToken: string): Promise<void> {
	const res = await fetch(`${API_BASE_URL}${path}`, {
		method: 'DELETE',
		headers: { Authorization: `Bearer ${accessToken}` }
	});
	if (!res.ok) {
		const body = await res.json().catch(() => ({}));
		throw new ApiError(res.status, (body as Record<string, unknown>).message as string ?? `HTTP ${res.status}`, body);
	}
}
