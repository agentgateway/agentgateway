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

export function corsNeedsUpdate(
	cors: CorsPolicy | null | undefined,
	target: CorsTarget,
	origin = currentOrigin()
) {
	if (!cors) return true;
	return (
		!hasValue(cors.allowOrigins, origin) ||
		!hasValue(cors.allowMethods, 'GET') ||
		!hasValue(cors.allowMethods, 'POST') ||
		!hasValue(cors.allowHeaders, '*') ||
		(target === 'mcp' && !hasValue(cors.exposeHeaders, 'Mcp-Session-Id'))
	);
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

function hasValue(values: string[] | undefined, value: string) {
	return Boolean(values?.some(item => item.toLowerCase() === value.toLowerCase()));
}
