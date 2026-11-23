import { sendMcpJsonRpc } from '@/api/playgroundApi';
import type { McpConfig, McpTarget } from '@/types';

export type McpTool = {
	name: string;
	description?: string;
	inputSchema?: unknown;
};

export function extractMcpTools(body: unknown): McpTool[] {
	const payload = Array.isArray(body) ? body[0] : body;
	if (!payload || typeof payload !== 'object') return [];
	const result = (payload as { result?: unknown }).result;
	if (!result || typeof result !== 'object') return [];
	const tools = (result as { tools?: unknown }).tools;
	if (!Array.isArray(tools)) return [];
	return tools.filter(isMcpTool);
}

export async function initializeMcpSession(
	baseUrl: string,
	clientName: string,
	existingSessionId?: string,
	bearerToken?: string
) {
	if (existingSessionId) return existingSessionId;
	const response = await sendMcpJsonRpc({
		baseUrl,
		bearerToken,
		body: {
			jsonrpc: '2.0',
			id: nextRpcId(),
			method: 'initialize',
			params: {
				protocolVersion: '2025-03-26',
				capabilities: {},
				clientInfo: {
					name: clientName,
					version: '0.1.0'
				}
			}
		}
	});
	const sessionId = response.sessionId ?? '';
	await sendInitializedNotification(baseUrl, sessionId, bearerToken);
	return sessionId;
}

export async function sendInitializedNotification(
	baseUrl: string,
	sessionId?: string | null,
	bearerToken?: string
) {
	if (!sessionId) return;
	await sendMcpJsonRpc({
		baseUrl,
		sessionId,
		bearerToken,
		body: {
			jsonrpc: '2.0',
			method: 'notifications/initialized',
			params: {}
		}
	});
}

let rpcId = 0;

export function nextRpcId() {
	rpcId += 1;
	return rpcId;
}

function isMcpTool(value: unknown): value is McpTool {
	return Boolean(
		value && typeof value === 'object' && typeof (value as { name?: unknown }).name === 'string'
	);
}

/** A set of MCP targets served together at one path. */
export type McpServerEndpoint = {
	/** The endpoint path, or undefined for the top-level targets when there are no endpoints. */
	endpoint?: string;
	/** Path clients connect to. */
	path: string;
	targets: McpTarget[];
	/** Resource ID of each target. */
	ids: string[];
};

export function mcpServerEndpoints(mcp: McpConfig | null | undefined): McpServerEndpoint[] {
	const topLevel = mcp?.targets ?? [];
	const topLevelIds = topLevel.map(t => t.name);
	const endpoints = (mcp?.endpoints ?? []).map(endpoint => {
		const path = trimTrailingSlash(endpoint.path);
		const targets = endpoint.targets ?? [];
		return { endpoint: path, path, targets, ids: targets.map(t => mcpResourceId(t.name, path)) };
	});
	if (endpoints.length === 0) return [{ path: '/mcp', targets: topLevel, ids: topLevelIds }];
	// The top-level targets are the targets of the /mcp endpoint.
	if (topLevel.length === 0) return endpoints;
	const defaultEndpoint = endpoints.find(endpoint => endpoint.path === '/mcp');
	if (defaultEndpoint) {
		defaultEndpoint.targets = [...topLevel, ...defaultEndpoint.targets];
		defaultEndpoint.ids = [...topLevelIds, ...defaultEndpoint.ids];
		return endpoints;
	}
	return [{ endpoint: '/mcp', path: '/mcp', targets: topLevel, ids: topLevelIds }, ...endpoints];
}

export function mcpEndpointLabel(endpoint: McpServerEndpoint) {
	return endpoint.endpoint ?? 'Default (/mcp)';
}

/** Resource ID of an MCP target or policy: `<endpoint path>:<name>` in an endpoint, otherwise the name. */
export function mcpResourceId(name: string, endpoint?: string) {
	return endpoint ? `${endpoint}:${name}` : name;
}

export function splitMcpResourceId(id: string): { endpoint?: string; name: string } {
	const index = id.lastIndexOf(':');
	return id.startsWith('/') && index > 0
		? { endpoint: id.slice(0, index), name: id.slice(index + 1) }
		: { name: id };
}

export function trimTrailingSlash(path: string) {
	return path.replace(/\/+$/, '');
}
