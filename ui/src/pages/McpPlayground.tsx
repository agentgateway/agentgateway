import { Button, Checkbox, Input, InputNumber, Tag } from 'antd';
import { Braces, Cable, Play, RotateCcw } from 'lucide-react';
import { type KeyboardEvent, useEffect, useMemo, useState } from 'react';

import { sendMcpJsonRpc } from '@/api/playgroundApi';
import { EnumSelector } from '@/components/EnumSelector';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import {
	ButtonLink,
	Dropdown,
	EmptyState,
	Field,
	FieldGroup,
	JsonBlock,
	PageHeader,
	Panel,
	StatusBanner
} from '@/components/Primitives';
import { corsNeedsUpdate, currentOrigin, playgroundCorsPolicy } from '@/cors';
import { mcpPlaygroundEndpoint } from '@/gatewayUrls';
import { useMcpConfigData, useStoredStringState, useUpsertPolicyResource } from '@/hooks';
import { extractMcpTools, type McpTool, nextRpcId, sendInitializedNotification } from '@/mcp';

type JsonSchema = {
	type?: string | string[];
	properties?: Record<string, JsonSchema>;
	required?: string[];
	description?: string;
	enum?: unknown[];
	items?: JsonSchema;
	default?: unknown;
};

type McpResponse = {
	status: number;
	sessionId: string | null;
	body: unknown;
};
const storageKeys = {
	tool: 'mcpPlaygroundTool'
};

export function McpPlaygroundPage() {
	const mcpData = useMcpConfigData();
	const effectiveConfig = mcpData.data;
	const upsertPolicy = useUpsertPolicyResource();
	const targets = useMemo(() => effectiveConfig?.mcp?.targets ?? [], [effectiveConfig]);
	const mcpEndpoint = mcpPlaygroundEndpoint(effectiveConfig);
	const derivedBaseUrl = mcpEndpoint.baseUrl;
	const baseUrl = derivedBaseUrl;
	const [initialized, setInitialized] = useState(false);
	const [sessionId, setSessionId] = useState('');
	const [tools, setTools] = useState<McpTool[]>([]);
	const [toolName, setToolName] = useStoredStringState(storageKeys.tool, '');
	const [argumentValues, setArgumentValues] = useState<Record<string, unknown>>({});
	const [argumentsJson, setArgumentsJson] = useState('{}');
	const [bearerToken, setBearerToken] = useState('');
	const [result, setResult] = useState<McpResponse | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState<'initialize' | 'call' | null>(null);

	const selectedTool = tools.find(tool => tool.name === toolName);
	const needsCors =
		effectiveConfig && !mcpEndpoint.sameOrigin
			? corsNeedsUpdate(effectiveConfig.mcp?.policies?.cors, 'mcp')
			: false;
	const fileCorsOwned = Boolean(
		mcpData.rawConfig.data?.mcp?.policies &&
			Object.hasOwn(mcpData.rawConfig.data.mcp.policies, 'cors')
	);

	useEffect(() => {
		localStorage.removeItem('mcpPlaygroundArgs');
	}, []);

	const [argumentTool, setArgumentTool] = useState(selectedTool);
	if (argumentTool !== selectedTool) {
		setArgumentTool(selectedTool);
		setArgumentValues(
			selectedTool?.inputSchema ? defaultArgumentsFromSchema(selectedTool.inputSchema) : {}
		);
	}

	async function initialize() {
		setLoading('initialize');
		setError(null);
		try {
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
							name: 'agentgateway-ui',
							version: '0.1.0'
						}
					}
				}
			});
			setResult(response);
			const nextSessionId = response.sessionId ?? sessionId;
			if (response.sessionId) setSessionId(response.sessionId);
			await sendInitializedNotification(baseUrl, nextSessionId, bearerToken);
			const toolsResponse = await sendMcpJsonRpc({
				baseUrl,
				sessionId: nextSessionId,
				bearerToken,
				body: {
					jsonrpc: '2.0',
					id: nextRpcId(),
					method: 'tools/list',
					params: {}
				}
			});
			setResult(toolsResponse);
			if (toolsResponse.sessionId) setSessionId(toolsResponse.sessionId);
			setInitialized(true);
			const nextTools = extractMcpTools(toolsResponse.body);
			setTools(nextTools);
			if (nextTools.length > 0 && !nextTools.some(tool => tool.name === toolName)) {
				setToolName(nextTools[0].name);
			}
		} catch (err) {
			setError(err instanceof Error ? err.message : 'MCP initialize failed');
		} finally {
			setLoading(null);
		}
	}

	async function callTool() {
		setLoading('call');
		setError(null);
		try {
			const toolArguments =
				selectedTool?.inputSchema && schemaHasSimpleProperties(selectedTool.inputSchema)
					? argumentsFromForm(selectedTool.inputSchema, argumentValues)
					: parseArguments(argumentsJson);
			const response = await sendMcpJsonRpc({
				baseUrl,
				sessionId,
				bearerToken,
				body: {
					jsonrpc: '2.0',
					id: nextRpcId(),
					method: 'tools/call',
					params: {
						name: toolName.trim(),
						arguments: toolArguments
					}
				}
			});
			setResult(response);
			if (response.sessionId) setSessionId(response.sessionId);
		} catch (err) {
			setError(err instanceof Error ? err.message : 'MCP tools/call failed');
		} finally {
			setLoading(null);
		}
	}

	return (
		<div className="page-stack">
			<PageHeader
				title="MCP Playground"
				description="List and call tools through the MCP gateway."
			/>
			{needsCors ? (
				<StatusBanner
					state="warn"
					title="Browser access is not allowed"
					action={
						mcpData.hybrid && fileCorsOwned ? (
							<ButtonLink to="/mcp/policies" hash="cors">
								Configure CORS
							</ButtonLink>
						) : (
							<Button
								disabled={upsertPolicy.isPending}
								onClick={() => {
									upsertPolicy.mutate({
										kind: 'mcp.policy',
										id: 'cors',
										value: playgroundCorsPolicy(effectiveConfig?.mcp?.policies?.cors, 'mcp')
									});
								}}
							>
								Apply CORS
							</Button>
						)
					}
				>
					Add {currentOrigin()} to the MCP CORS policy and expose Mcp-Session-Id so this playground
					can keep a browser session.
				</StatusBanner>
			) : null}
			{targets.length === 0 ? (
				<StatusBanner state="warn" title="No MCP servers">
					Create an MCP server before testing MCP traffic.
				</StatusBanner>
			) : null}
			{error ? (
				<StatusBanner state="bad" title="MCP request failed">
					{error}
				</StatusBanner>
			) : null}
			<section className="two-column wide-left playground-layout">
				<Panel>
					<div className="mcp-session-bar">
						<div>
							<span>Session</span>
							<strong className="mono">
								{initialized ? sessionId || 'initialized' : 'not initialized'}
							</strong>
						</div>
						{initialized ? (
							<Button
								icon={<RotateCcw size={16} />}
								onClick={() => {
									setInitialized(false);
									setSessionId('');
									setTools([]);
									setToolName('');
									setResult(null);
									setError(null);
								}}
							>
								Reset
							</Button>
						) : (
							<Button
								type="primary"
								icon={<Cable size={16} />}
								loading={loading === 'initialize'}
								disabled={loading !== null || !baseUrl.trim()}
								onClick={initialize}
							>
								Initialize
							</Button>
						)}
					</div>

					<details className="schema-details mcp-auth-details">
						<summary>Authorization header</summary>
						<Field label="Bearer token">
							<Input
								value={bearerToken}
								className="masked-secret-input"
								autoComplete="off"
								autoCorrect="off"
								autoCapitalize="none"
								data-1p-ignore="true"
								data-lpignore="true"
								data-form-type="other"
								name="agw-mcp-playground-bearer-token"
								spellCheck={false}
								onChange={event => setBearerToken(event.target.value)}
								placeholder="Optional token"
							/>
						</Field>
					</details>

					<FieldGroup label="Tool">
						<Dropdown
							ariaLabel="Tool"
							value={toolName}
							placeholder={initialized ? 'No tools returned' : 'Initialize first'}
							searchable
							options={tools.map(tool => ({
								value: tool.name,
								label: tool.description ? `${tool.name} - ${tool.description}` : tool.name,
								icon: <Braces size={16} />,
								searchText: `${tool.name} ${tool.description ?? ''}`
							}))}
							onChange={setToolName}
						/>
					</FieldGroup>

					{selectedTool?.description ? (
						<div className="tool-description">{selectedTool.description}</div>
					) : null}
					{selectedTool?.inputSchema ? (
						<details className="schema-details">
							<summary>Input schema</summary>
							<JsonBlock value={selectedTool.inputSchema} />
						</details>
					) : null}

					{!selectedTool ? (
						<div className="empty-inline">
							Initialize the session and select a tool to configure arguments.
						</div>
					) : selectedTool.inputSchema && schemaHasSimpleProperties(selectedTool.inputSchema) ? (
						<ToolArgumentsForm
							schema={selectedTool.inputSchema}
							values={argumentValues}
							onChange={setArgumentValues}
							onSubmit={callTool}
						/>
					) : (
						<FieldGroup label="Arguments JSON">
							<MiniMonacoEditor
								language="json"
								value={argumentsJson}
								onChange={setArgumentsJson}
								onSubmit={() => void callTool()}
							/>
						</FieldGroup>
					)}
					<div className="button-row mcp-call-actions">
						<Button
							type="primary"
							icon={<Play size={16} />}
							loading={loading === 'call'}
							disabled={loading !== null || !initialized || !baseUrl.trim() || !toolName.trim()}
							onClick={callTool}
						>
							Call tool
						</Button>
					</div>
				</Panel>
				<Panel className="playground-response-panel">
					<div className="section-heading">
						<h3>Result</h3>
					</div>
					{result ? (
						<McpResultView response={result} />
					) : (
						<EmptyState
							title="No response yet"
							description="Initialize or send a tool request to inspect MCP behavior."
						/>
					)}
				</Panel>
			</section>
		</div>
	);
}

function parseArguments(value: string) {
	const trimmed = value.trim();
	if (!trimmed) return {};
	const parsed = JSON.parse(trimmed) as unknown;
	if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
		throw new Error('Arguments must be a JSON object.');
	}
	return parsed;
}

function ToolArgumentsForm(props: {
	schema: unknown;
	values: Record<string, unknown>;
	onChange: (values: Record<string, unknown>) => void;
	onSubmit: () => void;
}) {
	const schema = props.schema as JsonSchema;
	const properties = schema.properties ?? {};
	const required = new Set(schema.required ?? []);
	const entries = Object.entries(properties);
	if (entries.length === 0) {
		return <div className="empty-inline">This tool does not declare arguments.</div>;
	}
	return (
		<div className="tool-arguments-form">
			{entries.map(([name, property]) => {
				const type = schemaType(property);
				const label = required.has(name) ? `${name} *` : name;
				const value = props.values[name];
				if (property.enum?.length) {
					return (
						<FieldGroup label={label} key={name}>
							<EnumSelector
								ariaLabel={name}
								value={value === undefined ? String(property.enum[0] ?? '') : String(value)}
								options={property.enum.map(option => ({
									value: String(option),
									label: String(option)
								}))}
								onChange={next => props.onChange({ ...props.values, [name]: next })}
							/>
							{property.description ? <small>{property.description}</small> : null}
						</FieldGroup>
					);
				}
				if (type === 'boolean') {
					return (
						<Checkbox
							className="option-checkbox"
							key={name}
							checked={Boolean(value)}
							onChange={event =>
								props.onChange({
									...props.values,
									[name]: event.target.checked
								})
							}
							onKeyDown={event => submitOnModEnter(event, props.onSubmit)}
						>
							<span>
								<strong>{label}</strong>
								{property.description ? <small>{property.description}</small> : null}
							</span>
						</Checkbox>
					);
				}
				if (type === 'number' || type === 'integer') {
					return (
						<Field label={label} hint={property.description} key={name}>
							<InputNumber
								style={{ width: '100%' }}
								value={typeof value === 'number' ? value : null}
								onChange={next =>
									props.onChange({ ...props.values, [name]: typeof next === 'number' ? next : '' })
								}
								onKeyDown={event => submitOnModEnter(event, props.onSubmit)}
							/>
						</Field>
					);
				}
				if (type === 'array' || type === 'object') {
					return (
						<FieldGroup label={label} hint={property.description} key={name}>
							<MiniMonacoEditor
								language="json"
								value={value === undefined ? '' : JSON.stringify(value, null, 2)}
								onChange={next =>
									props.onChange({
										...props.values,
										[name]: parseJsonDraft(next)
									})
								}
								onSubmit={props.onSubmit}
								placeholder={type === 'array' ? '[]' : '{}'}
							/>
						</FieldGroup>
					);
				}
				return (
					<Field label={label} hint={property.description} key={name}>
						<Input
							value={typeof value === 'string' ? value : ''}
							onChange={event => props.onChange({ ...props.values, [name]: event.target.value })}
							onKeyDown={event => submitOnModEnter(event, props.onSubmit)}
						/>
					</Field>
				);
			})}
		</div>
	);
}

function submitOnModEnter(event: KeyboardEvent, onSubmit: () => void) {
	if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
		event.preventDefault();
		onSubmit();
	}
}

function schemaHasSimpleProperties(schema: unknown): schema is JsonSchema {
	return Boolean(
		schema &&
			typeof schema === 'object' &&
			!Array.isArray(schema) &&
			(schema as JsonSchema).properties &&
			typeof (schema as JsonSchema).properties === 'object'
	);
}

function schemaType(schema: JsonSchema) {
	if (Array.isArray(schema.type)) return schema.type.find(type => type !== 'null') ?? 'string';
	return schema.type ?? 'string';
}

function defaultArgumentsFromSchema(schema: unknown) {
	if (!schemaHasSimpleProperties(schema)) return {};
	const values: Record<string, unknown> = {};
	for (const [name, property] of Object.entries(schema.properties ?? {})) {
		if (property.default !== undefined) {
			values[name] = property.default;
			continue;
		}
		if (property.enum?.length) {
			values[name] = String(property.enum[0]);
			continue;
		}
		const type = schemaType(property);
		if (type === 'boolean') values[name] = false;
		else if (type === 'array') values[name] = [];
		else if (type === 'object') values[name] = {};
		else values[name] = '';
	}
	return values;
}

function argumentsFromForm(schema: unknown, values: Record<string, unknown>) {
	if (!schemaHasSimpleProperties(schema)) return values;
	const next: Record<string, unknown> = {};
	for (const [name, property] of Object.entries(schema.properties ?? {})) {
		const value = values[name];
		const type = schemaType(property);
		if (value === '' || value === undefined) continue;
		if (
			(type === 'number' || type === 'integer') &&
			typeof value === 'number' &&
			Number.isFinite(value)
		) {
			next[name] = value;
		} else if (type !== 'number' && type !== 'integer') {
			next[name] = value;
		}
	}
	return next;
}

function parseJsonDraft(value: string) {
	const trimmed = value.trim();
	if (!trimmed) return '';
	try {
		return JSON.parse(trimmed) as unknown;
	} catch {
		return value;
	}
}

function McpResultView(props: { response: McpResponse }) {
	const payload = responsePayload(props.response.body);
	const result =
		payload && typeof payload === 'object' ? (payload as { result?: unknown }).result : undefined;
	const error =
		payload && typeof payload === 'object' ? (payload as { error?: unknown }).error : undefined;
	const toolError =
		result && typeof result === 'object' && (result as { isError?: unknown }).isError === true;
	const tools = extractMcpTools(props.response.body);
	const content =
		result && typeof result === 'object' && Array.isArray((result as { content?: unknown }).content)
			? (result as { content: unknown[] }).content
			: [];
	const structuredContent =
		result && typeof result === 'object'
			? (result as { structuredContent?: unknown }).structuredContent
			: undefined;

	return (
		<div className="mcp-result-view">
			<div className="mcp-result-status">
				<Tag color={error || toolError ? 'error' : 'success'}>
					{error ? 'error' : toolError ? 'tool error' : `HTTP ${props.response.status}`}
				</Tag>
				{props.response.sessionId ? <span className="mono">{props.response.sessionId}</span> : null}
			</div>

			{error ? (
				<div className="mcp-result-card">
					<strong>Error</strong>
					<JsonBlock value={error} />
				</div>
			) : tools.length > 0 ? (
				<div className="mcp-result-card">
					<strong>{tools.length} tools discovered</strong>
					<div className="mcp-tool-list">
						{tools.map(tool => (
							<div className="mcp-tool-row" key={tool.name}>
								<Tag>{tool.name}</Tag>
								{tool.description ? <small>{tool.description}</small> : null}
							</div>
						))}
					</div>
				</div>
			) : content.length > 0 || structuredContent !== undefined ? (
				<div className="mcp-result-card">
					<strong>{toolError ? 'Tool error' : 'Tool output'}</strong>
					{content.map((item, index) => (
						// biome-ignore lint/suspicious/noArrayIndexKey: Existing lint violation; remove this suppression when the underlying issue is fixed.
						<ContentBlock block={item} key={index} />
					))}
					{structuredContent !== undefined ? (
						<details className="schema-details" open>
							<summary>Structured content</summary>
							<JsonBlock value={structuredContent} />
						</details>
					) : null}
				</div>
			) : (
				<div className="mcp-result-card">
					<strong>{toolError ? 'Tool error' : 'Response'}</strong>
					<JsonBlock value={result ?? props.response.body} />
				</div>
			)}

			<details className="schema-details">
				<summary>Raw JSON</summary>
				<JsonBlock value={props.response} />
			</details>
		</div>
	);
}

function ContentBlock(props: { block: unknown }) {
	if (!props.block || typeof props.block !== 'object') return <JsonBlock value={props.block} />;
	const block = props.block as {
		type?: unknown;
		text?: unknown;
		data?: unknown;
		mimeType?: unknown;
	};
	if (block.type === 'text' && typeof block.text === 'string') {
		return <div className="mcp-text-output">{block.text}</div>;
	}
	if (block.type === 'image' && typeof block.data === 'string') {
		const mimeType = typeof block.mimeType === 'string' ? block.mimeType : 'image/png';
		return (
			<img
				className="mcp-image-output"
				src={`data:${mimeType};base64,${block.data}`}
				alt="MCP tool output"
			/>
		);
	}
	if (block.type === 'resource') {
		return (
			<details className="schema-details" open>
				<summary>Resource</summary>
				<JsonBlock value={props.block} />
			</details>
		);
	}
	return <JsonBlock value={props.block} />;
}

function responsePayload(body: unknown) {
	return Array.isArray(body) ? body[0] : body;
}
