import { ApiError, requestJson } from '@/api/base';
import type { LocalAttachedRoute, LocalAttachedTCPRoute } from '@/gateway-config';
import type {
	LlmConfig,
	LlmModel,
	LlmProvider,
	LlmVirtualModel,
	McpConfig,
	McpTarget,
	TrafficGateway,
	VirtualApiKey
} from '@/types';

export type ConfigResourceKind =
	| 'modelCatalog'
	| 'llm.provider'
	| 'llm.model'
	| 'llm.virtualModel'
	| 'llm.apiKey'
	| 'llm.policy'
	| 'mcp.target'
	| 'mcp.policy'
	| 'llm.settings'
	| 'mcp.settings'
	| 'traffic.gateway'
	| 'traffic.route'
	| 'traffic.tcpRoute'
	| 'ui.policy';

export type PolicyResourceKind = Extract<ConfigResourceKind, `${string}.policy`>;

export type LlmSettingsResource = Pick<LlmConfig, 'gateways' | 'port' | 'tls'>;
export type McpSettingsResource = Partial<Omit<McpConfig, 'targets' | 'policies'>>;
export type TrafficGatewayResource = TrafficGateway & { name: string };
export type TrafficRouteResource = LocalAttachedRoute & { name: string };
export type TrafficTcpRouteResource = LocalAttachedTCPRoute & { name: string };

export type ConfigResourceValue<K extends ConfigResourceKind> = K extends 'modelCatalog'
	? { base?: unknown; custom?: unknown }
	: K extends 'llm.settings'
		? LlmSettingsResource
		: K extends 'llm.provider'
			? LlmProvider
			: K extends 'llm.model'
				? LlmModel
				: K extends 'llm.virtualModel'
					? LlmVirtualModel
					: K extends 'llm.apiKey'
						? VirtualApiKey
						: K extends 'mcp.target'
							? McpTarget
							: K extends 'mcp.settings'
								? McpSettingsResource
								: K extends 'traffic.gateway'
									? TrafficGatewayResource
									: K extends 'traffic.route'
										? TrafficRouteResource
										: K extends 'traffic.tcpRoute'
											? TrafficTcpRouteResource
											: K extends 'llm.policy' | 'mcp.policy' | 'ui.policy'
												? unknown
												: never;

export interface ConfigResource<K extends ConfigResourceKind = ConfigResourceKind> {
	kind: K;
	id: string;
	value: ConfigResourceValue<K>;
	revision?: number;
	createdAt?: string;
	updatedAt?: string;
}

export interface ConfigResourcesResponse<K extends ConfigResourceKind = ConfigResourceKind> {
	resources: ConfigResource<K>[];
	generation?: number | null;
}

export class ConfigConflictError extends Error {
	constructor(message: string) {
		super(message);
		this.name = 'ConfigConflictError';
	}
}

let observedGeneration: number | null = null;

function rememberGeneration(response: { generation?: number | null }) {
	if (typeof response.generation === 'number') observedGeneration = response.generation;
	return response;
}

export function forgetConfigGeneration() {
	observedGeneration = null;
}

async function writeConfig<T>(path: string, init: RequestInit): Promise<T> {
	const pin: Record<string, string> =
		observedGeneration === null ? {} : { 'If-Match': String(observedGeneration) };
	try {
		const response = await requestJson<T>(path, {
			...init,
			headers: { ...((init.headers as Record<string, string>) ?? {}), ...pin }
		});
		rememberGeneration(response as { generation?: number | null });
		return response;
	} catch (error) {
		if (error instanceof ApiError && error.status === 409) {
			// Drop the pin so the next write revalidates server-side rather than wedging on a
			// generation this client cannot refresh by itself.
			observedGeneration = null;
			throw new ConfigConflictError(error.message);
		}
		throw error;
	}
}

export async function listConfigResources() {
	const response = await requestJson<ConfigResourcesResponse>('/api/config/resources');
	rememberGeneration(response);
	return response;
}

export function putConfigResources<K extends ConfigResourceKind>(
	kind: K,
	resources: ConfigResourceValue<K>[]
) {
	return writeConfig<ConfigResourcesResponse<K>>(
		`/api/config/resources/${encodeURIComponent(kind)}`,
		{
			method: 'PUT',
			body: JSON.stringify({
				resources: resources.map(value => ({ value }))
			})
		}
	);
}

export function updateConfigResource<K extends ConfigResourceKind>(
	kind: K,
	id: string,
	value: ConfigResourceValue<K>
) {
	return writeConfig<ConfigResourcesResponse<K>>(
		`/api/config/resources/${encodeURIComponent(kind)}/${encodeURIComponent(id)}`,
		{
			method: 'PUT',
			body: JSON.stringify({ value })
		}
	);
}

export function deleteConfigResource(kind: ConfigResourceKind, id: string) {
	return writeConfig<{ status: string; message: string; generation?: number | null }>(
		`/api/config/resources/${encodeURIComponent(kind)}/${encodeURIComponent(id)}`,
		{ method: 'DELETE' }
	);
}
