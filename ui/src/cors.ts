import type { CorsPolicy, GatewayConfig } from '@/types';

export type CorsTarget = 'llm' | 'mcp';

export function currentOrigin() {
	return window.location.origin;
}

export async function waitForLlmCors(baseUrl: string) {
	const signal = AbortSignal.timeout(10_000);
	while (!signal.aborted) {
		try {
			// An empty model request checks POST CORS without invoking a provider.
			const response = await fetch(`${baseUrl.replace(/\/$/, '')}/v1/models`, {
				method: 'POST',
				credentials: 'omit',
				headers: { 'Content-Type': 'application/json', Authorization: 'Bearer cors-probe' },
				body: '{}',
				cache: 'no-store',
				signal
			});
			await response.body?.cancel();
			return;
		} catch {
			if (!signal.aborted) await new Promise(resolve => setTimeout(resolve, 250));
		}
	}
	throw new Error('The CORS policy was saved, but the LLM listener is not reachable. Try again.');
}

// Non-safelisted headers sent by sendChatCompletion, sendMcpJsonRpc and waitForLlmCors.
export const playgroundRequestHeaders: Record<CorsTarget, string[]> = {
	llm: ['content-type', 'authorization'],
	mcp: ['content-type', 'authorization', 'mcp-session-id']
};

// Mirrors the gateway's CORS policy for the playground's uncredentialed POSTs.
// POST is a safelisted method, so allowMethods doesn't matter.
export function corsNeedsUpdate(
	cors: CorsPolicy | null | undefined,
	target: CorsTarget,
	origin = currentOrigin(),
	extraHeaders: string[] = []
) {
	if (!cors) return true;
	const headers = [...playgroundRequestHeaders[target], ...extraHeaders];
	return (
		!originAllowed(cors.allowOrigins, origin) ||
		!allows(cors.allowHeaders, headers) ||
		(target === 'mcp' && !allows(cors.exposeHeaders, ['Mcp-Session-Id']))
	);
}

// Same rules as the gateway: scheme, host and port must match, ports default by scheme,
// and `*` in the host matches any run of characters.
function originAllowed(allowOrigins: string[] | undefined, origin: string) {
	if (hasValue(allowOrigins, '*')) return true;
	const request = parseOrigin(origin);
	return Boolean(
		request &&
			allowOrigins?.some(value => {
				const allowed = parseOrigin(value);
				if (allowed?.scheme !== request.scheme || allowed.port !== request.port) return false;
				const host = allowed.host.split('*').map(escapeRegExp).join('.*');
				return new RegExp(`^${host}$`).test(request.host);
			})
	);
}

function parseOrigin(value: string) {
	const match = /^(https?):\/\/([^/?#:]+)(?::(\d+))?$/i.exec(value);
	if (!match) return null;
	const scheme = match[1].toLowerCase();
	const port = Number(match[3] ?? (scheme === 'https' ? 443 : 80));
	return { scheme, host: match[2].toLowerCase(), port };
}

function escapeRegExp(value: string) {
	return value.replace(/[.+?^${}()|[\]\\]/g, '\\$&');
}

export function applyPlaygroundCors(
	config: GatewayConfig,
	target: CorsTarget,
	origin = currentOrigin()
) {
	if (target === 'llm') {
		config.llm ??= { models: [] };
		config.llm.policies ??= {};
		config.llm.policies.cors = playgroundCorsPolicy(config.llm.policies.cors, target, origin);
		return;
	}

	config.mcp ??= { targets: [] };
	config.mcp.policies ??= {};
	config.mcp.policies.cors = playgroundCorsPolicy(config.mcp.policies.cors, target, origin);
}

export function playgroundCorsPolicy(
	cors: CorsPolicy | null | undefined,
	target: CorsTarget,
	origin = currentOrigin()
): CorsPolicy {
	return {
		...(cors ?? {}),
		allowOrigins: appendUnique(cors?.allowOrigins, origin),
		allowHeaders: appendUnique(cors?.allowHeaders, '*'),
		allowMethods: appendUnique(appendUnique(cors?.allowMethods, 'GET'), 'POST'),
		exposeHeaders:
			target === 'mcp' ? appendUnique(cors?.exposeHeaders, 'Mcp-Session-Id') : cors?.exposeHeaders
	};
}

function appendUnique(values: string[] | undefined, value: string) {
	const next = values ? [...values] : [];
	if (!hasValue(next, value)) next.push(value);
	return next;
}

function allows(values: string[] | undefined, required: string[]) {
	return hasValue(values, '*') || required.every(value => hasValue(values, value));
}

function hasValue(values: string[] | undefined, value: string) {
	return Boolean(values?.some(item => item.toLowerCase() === value.toLowerCase()));
}
