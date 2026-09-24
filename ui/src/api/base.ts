export const apiBase = import.meta.env.VITE_AGENTGATEWAY_API ?? '';

let redirecting = false;
let hasSuccessfulRequest = false;

export async function requestApi(path: string, init?: RequestInit): Promise<Response> {
	// Authentication must use document navigation; fetch cannot follow cross-origin login redirects.
	const response = await fetch(`${apiBase}${path}`, {
		...init,
		credentials: 'include',
		redirect: 'manual'
	});
	if (response.status === 401 || response.type === 'opaqueredirect') {
		const location = response.headers.get('location');
		const uiLogin =
			location?.startsWith('/') && !location.startsWith('//') && !location.includes('\\');
		if (!uiLogin && !hasSuccessfulRequest) {
			throw new Error('Authentication required. Please sign in and reload the page.');
		}
		if (!redirecting) {
			redirecting = true;
			if (!uiLogin) {
				window.location.reload();
				return new Promise<Response>(() => {});
			}
			const returnTo = window.location.pathname + window.location.search + window.location.hash;
			const destination = new URL(`${apiBase}${location}`, window.location.href);
			destination.searchParams.set('returnTo', returnTo);
			window.location.replace(destination.href);
		}
		return new Promise<Response>(() => {});
	}
	if (response.ok) hasSuccessfulRequest = true;
	return response;
}

export class ApiError extends Error {
	readonly status: number;

	constructor(status: number, message: string) {
		super(message);
		this.name = 'ApiError';
		this.status = status;
	}
}

export async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
	// `headers` last: spreading `init` after it drops Content-Type for callers that pass headers.
	const response = await requestApi(path, {
		...init,
		headers: { 'Content-Type': 'application/json', ...(init?.headers ?? {}) }
	});
	if (!response.ok) {
		let message = `${response.status} ${response.statusText}`;
		try {
			const text = await response.text();
			if (text) {
				try {
					const body = JSON.parse(text);
					message = typeof body === 'string' ? body : JSON.stringify(body);
				} catch {
					message = text;
				}
			}
		} catch {
			// Keep the status text fallback when the body cannot be read.
		}
		throw new ApiError(response.status, message || 'request failed');
	}
	return response.json() as Promise<T>;
}

export function parseJsonOrText(text: string) {
	try {
		return JSON.parse(text) as unknown;
	} catch {
		return text;
	}
}
