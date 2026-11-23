import { Link } from '@tanstack/react-router';
import { Button, Checkbox, Input, Table, Tag } from 'antd';
import { Info, Pencil, Plus, Server, ShieldCheck, SlidersHorizontal, Trash2 } from 'lucide-react';
import { useMemo, useState } from 'react';

import type { McpSettingsResource } from '@/api/configResourcesApi';
import { ConfigDiffSaveActions } from '@/components/ConfigDiffDrawer';
import { LostFieldsWarning, useChangedFields, useMergedEdits } from '@/components/EditorContracts';
import { EnumSelector } from '@/components/EnumSelector';
import { FreeformCombobox } from '@/components/FreeformCombobox';
import { GatewayBindingEditor, type GatewayBindingValue } from '@/components/GatewayBindingEditor';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import {
	ConfirmDialog,
	Drawer,
	EmptyState,
	Field,
	FieldGroup,
	PageHeader,
	Panel,
	SegmentedControl,
	StatusBanner,
	Tooltip
} from '@/components/Primitives';
import {
	ensureMcp,
	fileOwnedMcpSettingFields,
	isDatabaseConfigResource,
	makeEmptyMcpTarget,
	mcpPolicies,
	mcpSettingsFields,
	upsertMcpTarget
} from '@/config';
import { useStickyQueryParam } from '@/drawerRouteState';
import { useDeleteConfigResource, useMcpConfigData, useUpsertConfigResource } from '@/hooks';
import {
	type McpServerEndpoint,
	mcpServerEndpoints,
	splitMcpResourceId,
	trimTrailingSlash
} from '@/mcp';
import { BackendPoliciesField } from '@/policies/PolicyFormControls';
import { CollapsiblePolicySection, PolicySection } from '@/policies/PolicyLayout';
import { parseYamlText, toYamlMappingText } from '@/policies/policyUtils';
import { type SchemaHelp, useSchemaHelp } from '@/schemaHelp';
import type {
	GatewayConfig,
	McpConfig,
	McpEndpoint,
	McpFailureMode,
	McpPrefixMode,
	McpStatefulMode,
	McpTarget,
	McpTargetKind
} from '@/types';

const targetKinds: McpTargetKind[] = ['mcp', 'sse', 'stdio', 'openapi'];

type McpSettingsPatch = Partial<
	Omit<McpConfig, 'gateways' | 'port' | 'statefulMode' | 'dnsRebindingProtection'>
> & {
	statefulMode?: McpStatefulMode | null;
	gateways?: McpConfig['gateways'] | null;
	port?: number | null;
	dnsRebindingProtection?: boolean | null;
};

export function McpServersPage() {
	const mcpData = useMcpConfigData();
	const rawConfig = mcpData.rawConfig;
	const hybrid = mcpData.hybrid;
	const upsertResource = useUpsertConfigResource();
	const deleteResource = useDeleteConfigResource();
	const help = useSchemaHelp();
	const resources = mcpData.resources;
	const effectiveConfig = mcpData.data;
	const mcp = effectiveConfig?.mcp;
	const endpoints = useMemo(() => mcpServerEndpoints(mcp), [mcp]);
	const endpointPaths = endpoints.flatMap(endpoint =>
		endpoint.endpoint && endpoint.endpoint !== '/mcp' ? [endpoint.endpoint] : []
	);
	const mcpEndpointHasTargets = Boolean(
		mcp?.endpoints?.some(item => trimTrailingSlash(item.path) === '/mcp' && item.targets?.length)
	);
	const targets = endpoints.flatMap(endpoint =>
		endpoint.targets.map((target, index) => ({
			target,
			endpoint: splitMcpResourceId(endpoint.ids[index]).endpoint,
			id: endpoint.ids[index]
		}))
	);
	const fileOwnedSettingFields = fileOwnedMcpSettingFields(rawConfig.data, hybrid);
	const saving = upsertResource.isPending || deleteResource.isPending;
	const saveError = upsertResource.error?.message ?? deleteResource.error?.message ?? null;
	const [editing, setEditing] = useState<{
		previousId?: string;
		endpoint?: string;
		target: McpTarget;
	} | null>(null);
	const [deletingServer, setDeletingServer] = useState<string | null>(null);
	const [settingsEndpoint, setSettingsEndpoint] = useState<string | null>(null);
	const [serverDrawer, setServerDrawer] = useStickyQueryParam('server');
	const linkedTarget =
		serverDrawer && serverDrawer !== 'new' && serverDrawer !== 'settings'
			? targets.find(target => target.id === serverDrawer)
			: null;
	const activeEditing =
		editing ??
		(serverDrawer === 'new'
			? { target: makeEmptyMcpTarget() }
			: linkedTarget
				? {
						previousId: linkedTarget.id,
						endpoint: linkedTarget.endpoint,
						target: structuredClone(linkedTarget.target)
					}
				: null);
	const settingsOpen = serverDrawer === 'settings';
	const showEndpointHeaders = endpoints.some(endpoint => endpoint.endpoint);

	const canDelete = (id: string) =>
		!hybrid || isDatabaseConfigResource(resources, 'mcp.target', id);

	function openNewServer(endpoint?: string) {
		setEditing({ target: makeEmptyMcpTarget(), endpoint });
		setServerDrawer('new');
	}

	function openEditServer(id: string) {
		setEditing(null);
		setServerDrawer(id);
	}

	function closeServerDrawer() {
		setEditing(null);
		setServerDrawer(null, 'replace');
	}

	return (
		<div className="page-stack">
			<PageHeader
				title="MCP Servers"
				description="Configure MCP targets served by the gateway."
				actions={
					<div className="button-row">
						<Button
							icon={<SlidersHorizontal size={16} />}
							onClick={() => setServerDrawer('settings')}
						>
							Settings
						</Button>
						<Button type="primary" icon={<Plus size={16} />} onClick={() => openNewServer()}>
							Add server
						</Button>
					</div>
				}
			/>

			{saveError && !activeEditing && !settingsOpen && !settingsEndpoint ? (
				<StatusBanner state="bad" title="Save failed">
					{saveError}
				</StatusBanner>
			) : null}
			{upsertResource.isSuccess || deleteResource.isSuccess ? (
				<StatusBanner state="ok" title="Configuration saved" />
			) : null}

			{mcpData.isLoading || mcpData.error || targets.length === 0 || !showEndpointHeaders ? (
				<Panel>
					{mcpData.isLoading ? (
						<StatusBanner state="loading" title="Loading MCP servers" />
					) : mcpData.error ? (
						<StatusBanner state="bad" title="Configuration API unavailable">
							{mcpData.error.message}
						</StatusBanner>
					) : targets.length === 0 ? (
						<EmptyState
							title="No MCP servers configured"
							description="Add a target so the gateway can expose MCP traffic."
							action={
								<Button type="primary" icon={<Server size={16} />} onClick={() => openNewServer()}>
									Add server
								</Button>
							}
						/>
					) : (
						<McpServerTable
							endpoint={endpoints[0]}
							canDelete={canDelete}
							saving={saving}
							onEdit={openEditServer}
							onDelete={setDeletingServer}
						/>
					)}
				</Panel>
			) : (
				endpoints.map(endpoint => {
					const overrides = endpoint.endpoint
						? Object.keys(mcpPolicies(mcp, endpoint.endpoint) ?? {}).length
						: 0;
					const entry = mcp?.endpoints?.find(
						item => trimTrailingSlash(item.path) === endpoint.path
					);
					const settingOverrides = mcpSettingsFields.filter(
						field => (entry as McpSettingsResource | undefined)?.[field] != null
					).length;
					return (
						<Panel key={endpoint.path} className="mcp-endpoint-panel">
							<div className="mcp-endpoint-header">
								<div>
									<h3>{endpoint.path}</h3>
									<p>
										{endpoint.targets.length} {endpoint.targets.length === 1 ? 'server' : 'servers'}
									</p>
								</div>
								<div className="page-actions">
									<Link
										to="/mcp/policies"
										search={endpoint.endpoint ? { endpoint: endpoint.endpoint } : {}}
									>
										<Button size="small" icon={<ShieldCheck size={14} />}>
											Policies
											{overrides ? (
												<Tag color="purple">
													{overrides} {overrides === 1 ? 'override' : 'overrides'}
												</Tag>
											) : null}
										</Button>
									</Link>
									<Button
										size="small"
										icon={<SlidersHorizontal size={14} />}
										onClick={() => setSettingsEndpoint(endpoint.path)}
									>
										Settings
										{settingOverrides ? (
											<Tag color="purple">
												{settingOverrides} {settingOverrides === 1 ? 'override' : 'overrides'}
											</Tag>
										) : null}
									</Button>
									<Button
										size="small"
										icon={<Plus size={14} />}
										onClick={() =>
											openNewServer(endpoint.path === '/mcp' ? undefined : endpoint.endpoint)
										}
									>
										Add server
									</Button>
								</div>
							</div>
							{endpoint.targets.length ? (
								<McpServerTable
									endpoint={endpoint}
									canDelete={canDelete}
									saving={saving}
									onEdit={openEditServer}
									onDelete={setDeletingServer}
								/>
							) : null}
						</Panel>
					);
				})
			)}

			{activeEditing ? (
				<McpServerEditor
					key={activeEditing.previousId ?? 'new'}
					initial={activeEditing.target}
					initialEndpoint={activeEditing.endpoint}
					endpointOptions={endpointPaths}
					mcpEndpointHasTargets={mcpEndpointHasTargets}
					config={effectiveConfig}
					previousId={activeEditing.previousId}
					databaseBacked={
						hybrid &&
						(!activeEditing.previousId ||
							isDatabaseConfigResource(resources, 'mcp.target', activeEditing.previousId))
					}
					help={help}
					saving={saving}
					saveError={saveError}
					onCancel={closeServerDrawer}
					onSave={(target, endpoint, previousId) => {
						upsertResource.mutate(
							{
								kind: 'mcp.target',
								value: endpoint ? { ...target, endpoint } : target,
								previousId
							},
							{
								onSuccess: closeServerDrawer
							}
						);
					}}
				/>
			) : null}
			{settingsOpen && !mcpData.isLoading && !mcpData.error ? (
				<McpSettingsDrawer
					config={effectiveConfig}
					mcp={mcp}
					databaseBacked={hybrid}
					readOnlyFields={fileOwnedSettingFields}
					help={help}
					saving={saving}
					saveError={saveError}
					onClose={closeServerDrawer}
					onSave={value => {
						upsertResource.mutate(
							{ kind: 'mcp.settings', value },
							{
								onSuccess: closeServerDrawer
							}
						);
					}}
				/>
			) : null}
			{settingsEndpoint ? (
				<McpSettingsDrawer
					endpoint={settingsEndpoint}
					config={effectiveConfig}
					mcp={
						mcp?.endpoints?.find(item => trimTrailingSlash(item.path) === settingsEndpoint) as
							| McpSettingsResource
							| undefined
					}
					databaseBacked={hybrid}
					readOnlyFields={fileOwnedMcpSettingFields(rawConfig.data, hybrid, settingsEndpoint)}
					help={help}
					saving={saving}
					saveError={saveError}
					onClose={() => setSettingsEndpoint(null)}
					onSave={value => {
						const done = { onSuccess: () => setSettingsEndpoint(null) };
						// A stored resource without settings would keep an otherwise empty endpoint alive.
						if (hybrid && Object.keys(value).length === 0) {
							if (isDatabaseConfigResource(resources, 'mcp.settings', settingsEndpoint)) {
								deleteResource.mutate({ kind: 'mcp.settings', id: settingsEndpoint }, done);
							} else {
								setSettingsEndpoint(null);
							}
							return;
						}
						upsertResource.mutate(
							{ kind: 'mcp.settings', value: { ...value, endpoint: settingsEndpoint } },
							done
						);
					}}
				/>
			) : null}
			{deletingServer ? (
				<ConfirmDialog
					title="Delete MCP server?"
					destructive
					confirmLabel="Delete server"
					confirmDisabled={saving}
					onCancel={() => setDeletingServer(null)}
					onConfirm={() => {
						deleteResource.mutate(
							{ kind: 'mcp.target', id: deletingServer },
							{
								onSuccess: () => setDeletingServer(null)
							}
						);
					}}
				>
					<p>
						Delete <strong>{splitMcpResourceId(deletingServer).name}</strong>
						{splitMcpResourceId(deletingServer).endpoint
							? ` from ${splitMcpResourceId(deletingServer).endpoint}`
							: ''}
						? Traffic can no longer be sent to this target.
					</p>
				</ConfirmDialog>
			) : null}
		</div>
	);
}

function McpServerTable(props: {
	endpoint: McpServerEndpoint;
	canDelete: (id: string) => boolean;
	saving: boolean;
	onEdit: (id: string) => void;
	onDelete: (id: string) => void;
}) {
	const rows = props.endpoint.targets.map((target, index) => ({
		target,
		id: props.endpoint.ids[index]
	}));
	return (
		<Table<(typeof rows)[number]>
			className="mcp-server-table"
			rowKey="id"
			dataSource={rows}
			pagination={false}
			tableLayout="fixed"
			columns={[
				{
					title: 'Name',
					key: 'name',
					width: '24%',
					className: 'strong',
					render: (_, { target }) => {
						const warnings = targetWarnings(target);
						return (
							<>
								{target.name}
								{warnings.length ? (
									<Tooltip content={warnings.join(' ')}>
										<Tag color="warning">
											{warnings.length} {warnings.length === 1 ? 'warning' : 'warnings'}
										</Tag>
									</Tooltip>
								) : null}
							</>
						);
					}
				},
				{
					title: 'Transport',
					key: 'transport',
					width: 160,
					render: (_, { target }) => <Tag>{transportLabel(targetKind(target))}</Tag>
				},
				{
					title: 'Target',
					key: 'target',
					ellipsis: true,
					render: (_, { target }) => <code>{targetEndpoint(target)}</code>
				},
				{
					key: 'actions',
					align: 'right',
					width: 104,
					onHeaderCell: () => ({ 'aria-label': 'Actions' }),
					render: (_, { id }) => {
						const canDelete = props.canDelete(id);
						return (
							<div className="row-actions">
								<Tooltip content="Edit server">
									<Button
										aria-label="Edit server"
										icon={<Pencil size={16} />}
										onClick={() => props.onEdit(id)}
									/>
								</Tooltip>
								<Tooltip
									content={
										canDelete ? 'Delete server' : 'File-owned servers cannot be deleted here'
									}
								>
									<Button
										danger
										aria-label="Delete server"
										icon={<Trash2 size={16} />}
										disabled={props.saving || !canDelete}
										onClick={() => props.onDelete(id)}
									/>
								</Tooltip>
							</div>
						);
					}
				}
			]}
		/>
	);
}

export function McpSettingsDrawer(props: {
	config?: GatewayConfig | null;
	/** Edits the overrides of this endpoint rather than the shared settings. */
	endpoint?: string;
	mcp?: McpSettingsResource | null;
	databaseBacked?: boolean;
	readOnlyFields?: ReadonlySet<string>;
	help: SchemaHelp;
	saving: boolean;
	saveError?: string | null;
	onClose: () => void;
	onSave: (settings: McpSettingsResource) => void;
}) {
	return (
		<Drawer
			title={props.endpoint ? `Settings for ${props.endpoint}` : 'Settings'}
			onClose={props.onClose}
			error={props.saveError}
		>
			<McpSettings
				config={props.config}
				endpoint={props.endpoint}
				mcp={props.mcp}
				databaseBacked={props.databaseBacked}
				readOnlyFields={props.readOnlyFields}
				help={props.help}
				saving={props.saving}
				onSave={props.onSave}
			/>
		</Drawer>
	);
}

function McpSettings(props: {
	config?: GatewayConfig | null;
	/** Edits the overrides of this endpoint rather than the shared settings. */
	endpoint?: string;
	mcp?: McpSettingsResource | null;
	databaseBacked?: boolean;
	readOnlyFields?: ReadonlySet<string>;
	help: SchemaHelp;
	saving: boolean;
	onSave: (settings: McpSettingsResource) => void;
}) {
	const [binding, setBinding] = useState<GatewayBindingValue>({
		gateways: props.mcp?.gateways ?? null,
		port: props.mcp?.port ?? null
	});
	// Settings an endpoint leaves unset come from the shared settings.
	const shared = props.endpoint ? props.config?.mcp : undefined;
	const inheritLabel = (value?: string | null) => (value ? `Inherit (${value})` : 'Inherit');
	const inheritDescription = 'Use the shared MCP setting.';
	const overrideHint = props.endpoint ? 'Overrides the shared MCP setting.' : undefined;
	const [statefulMode, setStatefulMode] = useState<McpStatefulMode | 'inherit'>(
		props.mcp?.statefulMode ?? (props.endpoint ? 'inherit' : 'stateful')
	);
	const [prefixMode, setPrefixMode] = useState<McpPrefixMode | 'inherit'>(
		props.mcp?.prefixMode ?? (props.endpoint ? 'inherit' : 'conditional')
	);
	const [failureMode, setFailureMode] = useState<McpFailureMode | 'inherit'>(
		props.mcp?.failureMode ?? (props.endpoint ? 'inherit' : 'failClosed')
	);
	const [sseKeepAlive, setSseKeepAlive] = useState(props.mcp?.sseKeepAlive ?? '');
	const [server, setServer] = useState({
		name: props.mcp?.server?.name ?? '',
		version: props.mcp?.server?.version ?? '',
		title: props.mcp?.server?.title ?? '',
		instructions: props.mcp?.server?.instructions ?? ''
	});
	const serverOverrides = Object.fromEntries(
		Object.entries(server).filter(([, value]) => value.trim())
	);
	const serverReadOnly = props.readOnlyFields?.has('server');
	const patch: McpSettingsPatch = {
		...(props.endpoint
			? {}
			: {
					gateways: binding.gateways ?? [],
					port: binding.gateways != null && binding.gateways.length > 0 ? null : binding.port
				}),
		statefulMode: statefulMode === 'inherit' ? null : statefulMode,
		prefixMode: prefixMode === 'inherit' ? null : prefixMode,
		failureMode: failureMode === 'inherit' ? null : failureMode,
		sseKeepAlive: sseKeepAlive.trim() || null,
		dnsRebindingProtection: props.mcp?.dnsRebindingProtection ?? null,
		server: Object.keys(serverOverrides).length > 0 ? serverOverrides : null
	};
	// The settings resource is replaced as a whole. Fields the form was not used to change keep
	// their configured value, so defaults the form displays are not written.
	const writable = (settings: object) =>
		Object.fromEntries(
			Object.entries(settings).filter(
				([field, value]) =>
					value != null &&
					mcpSettingsFields.some(known => known === field) &&
					!props.readOnlyFields?.has(field)
			)
		) as McpSettingsResource;
	const original = writable(props.mcp ?? {});
	const settings = useChangedFields(original, writable(patch));
	const bindingReadOnly = Boolean(
		props.readOnlyFields?.has('gateways') || props.readOnlyFields?.has('port')
	);

	return (
		<form
			className="policy-editor-stack"
			onSubmit={event => {
				event.preventDefault();
				props.onSave(settings);
			}}
		>
			{props.readOnlyFields?.size ? (
				<StatusBanner state="warn" title="Some settings are file-owned">
					Disabled fields are managed by the file configuration. Other MCP settings can still be
					saved to the database.
				</StatusBanner>
			) : null}
			{props.endpoint ? null : (
				<PolicySection
					icon={<Server size={17} />}
					title="Gateway binding"
					description="Choose how MCP is exposed."
				>
					<div className="form-grid">
						<GatewayBindingEditor
							config={props.config}
							value={binding}
							defaultPort={3000}
							portLabel="Port"
							portPlaceholder="3000"
							portTooltip={props.help.field<McpConfig>(
								'LocalSimpleMcpConfig',
								'port',
								'Gateway port for MCP traffic.'
							)}
							disabled={bindingReadOnly}
							onChange={setBinding}
						/>
					</div>
				</PolicySection>
			)}
			<CollapsiblePolicySection
				icon={<SlidersHorizontal size={17} />}
				title="MCP behavior"
				description="Choose session, tool-prefix, and failure behavior."
			>
				<div className="form-grid">
					<FieldGroup
						label="State mode"
						hint={statefulMode === 'inherit' ? undefined : overrideHint}
						tooltip={props.help.field<McpConfig>(
							'LocalSimpleMcpConfig',
							'statefulMode',
							'Controls whether MCP sessions are preserved by the gateway.'
						)}
					>
						<EnumSelector
							ariaLabel="State mode"
							value={statefulMode}
							options={[
								...(props.endpoint
									? [
											{
												value: 'inherit' as const,
												label: inheritLabel(shared?.statefulMode ?? 'stateful'),
												description: inheritDescription
											}
										]
									: []),
								{
									value: 'stateless',
									label: 'Stateless',
									description: 'Do not preserve MCP session state between requests.'
								},
								{
									value: 'stateful',
									label: 'Stateful',
									description: 'Preserve MCP sessions so targets can keep per-session context.'
								}
							]}
							schema={props.help.node([
								'$defs',
								'LocalSimpleMcpConfig',
								'properties',
								'statefulMode'
							])}
							disabled={props.readOnlyFields?.has('statefulMode')}
							onChange={setStatefulMode}
						/>
					</FieldGroup>
					<FieldGroup
						label="Prefix mode"
						hint={prefixMode === 'inherit' ? undefined : overrideHint}
						tooltip={props.help.field<McpConfig>(
							'LocalSimpleMcpConfig',
							'prefixMode',
							'Controls whether target names are prefixed when exposing tools.'
						)}
					>
						<EnumSelector
							ariaLabel="Prefix mode"
							value={prefixMode}
							options={[
								...(props.endpoint
									? [
											{
												value: 'inherit' as const,
												label: inheritLabel(shared?.prefixMode ?? 'conditional'),
												description: inheritDescription
											}
										]
									: []),
								{ value: 'conditional', label: 'Conditional' },
								{ value: 'always', label: 'Always' },
								{ value: 'never', label: 'Never' }
							]}
							schema={props.help.node([
								'$defs',
								'LocalSimpleMcpConfig',
								'properties',
								'prefixMode'
							])}
							disabled={props.readOnlyFields?.has('prefixMode')}
							onChange={setPrefixMode}
						/>
					</FieldGroup>
					<FieldGroup
						label="Failure mode"
						hint={failureMode === 'inherit' ? undefined : overrideHint}
						tooltip={props.help.field<McpConfig>('LocalSimpleMcpConfig', 'failureMode')}
					>
						<EnumSelector
							ariaLabel="Failure mode"
							value={failureMode}
							options={[
								...(props.endpoint
									? [
											{
												value: 'inherit' as const,
												label: inheritLabel(shared?.failureMode),
												description: inheritDescription
											}
										]
									: []),
								{ value: 'failClosed', label: 'Fail closed' },
								{ value: 'failOpen', label: 'Fail open' }
							]}
							schema={props.help.node(['$defs', 'McpBackendFailureMode'])}
							disabled={props.readOnlyFields?.has('failureMode')}
							onChange={setFailureMode}
						/>
					</FieldGroup>
					<Field
						label="SSE keep-alive"
						hint={sseKeepAlive.trim() ? overrideHint : undefined}
						tooltip={props.help.field<McpConfig>('LocalSimpleMcpConfig', 'sseKeepAlive')}
					>
						<Input
							value={sseKeepAlive}
							disabled={props.readOnlyFields?.has('sseKeepAlive')}
							onChange={event => setSseKeepAlive(event.target.value)}
							placeholder={shared?.sseKeepAlive ?? '15s'}
						/>
					</Field>
				</div>
			</CollapsiblePolicySection>
			<CollapsiblePolicySection
				icon={<Info size={17} />}
				title="Server info"
				description={
					props.endpoint
						? 'Setting any field replaces the shared server info and instructions as a whole.'
						: 'Override the server info and instructions reported to clients when multiplexing targets.'
				}
				summary={
					props.endpoint && Object.keys(serverOverrides).length > 0 ? (
						<Tag color="purple">override</Tag>
					) : undefined
				}
			>
				<div className="form-grid">
					<Field
						label="Name"
						tooltip={props.help.field<NonNullable<McpConfig['server']>>(
							'McpServerOverrides',
							'name'
						)}
					>
						<Input
							value={server.name}
							disabled={serverReadOnly}
							onChange={event => setServer({ ...server, name: event.target.value })}
							placeholder="agentgateway"
						/>
					</Field>
					<Field
						label="Version"
						tooltip={props.help.field<NonNullable<McpConfig['server']>>(
							'McpServerOverrides',
							'version'
						)}
					>
						<Input
							value={server.version}
							disabled={serverReadOnly}
							onChange={event => setServer({ ...server, version: event.target.value })}
						/>
					</Field>
					<Field
						label="Title"
						tooltip={props.help.field<NonNullable<McpConfig['server']>>(
							'McpServerOverrides',
							'title'
						)}
					>
						<Input
							value={server.title}
							disabled={serverReadOnly}
							onChange={event => setServer({ ...server, title: event.target.value })}
						/>
					</Field>
				</div>
				<Field
					label="Instructions"
					tooltip={props.help.field<NonNullable<McpConfig['server']>>(
						'McpServerOverrides',
						'instructions'
					)}
				>
					<Input.TextArea
						value={server.instructions}
						disabled={serverReadOnly}
						onChange={event => setServer({ ...server, instructions: event.target.value })}
					/>
				</Field>
			</CollapsiblePolicySection>
			<ConfigDiffSaveActions
				config={props.config}
				diffTitle="MCP settings config diff"
				saveLabel="Save settings"
				saving={props.saving}
				saveDisabled={mcpSettingsFields.every(field => props.readOnlyFields?.has(field))}
				onSave={() => props.onSave(settings)}
				resourceDiff={props.databaseBacked ? { original, modified: settings } : undefined}
				applyDiff={next => {
					const mcp = ensureMcp(next);
					let target: Record<string, unknown> = mcp;
					if (props.endpoint) {
						mcp.endpoints ??= [];
						let entry = mcp.endpoints.find(item => trimTrailingSlash(item.path) === props.endpoint);
						if (!entry) {
							entry = { path: props.endpoint };
							mcp.endpoints.push(entry);
						}
						target = entry;
					}
					for (const field of mcpSettingsFields) {
						if (props.readOnlyFields?.has(field)) continue;
						if (field in settings) target[field] = settings[field];
						else delete target[field];
					}
				}}
			/>
		</form>
	);
}

type McpServerForm = ReturnType<typeof formFrom>;

// The form state for a target: from the saved config, or from raw mode.
function formFrom(target: McpTarget) {
	const kind = targetKind(target);
	const stdio = 'stdio' in target ? target.stdio : undefined;
	const schema = 'openapi' in target ? target.openapi?.schema : undefined;
	const schemaMode = schemaSourceMode(schema);
	return {
		// The target itself, for the fields the form does not edit.
		source: target,
		name: target.name ?? '',
		kind,
		url: networkUrl(networkTarget(target), kind),
		cmd: stdio?.cmd ?? '',
		args: (stdio?.args ?? []).join(' '),
		envText: toYamlMappingText(stdio?.env),
		clearEnv: Boolean(stdio?.clear_env),
		schemaMode,
		schemaValue: schemaSourceValue(schema, schemaMode),
		policies: target.policies
	};
}

function McpServerEditor(props: {
	initial: McpTarget;
	initialEndpoint?: string;
	endpointOptions: string[];
	mcpEndpointHasTargets: boolean;
	config?: GatewayConfig | null;
	previousId?: string;
	databaseBacked?: boolean;
	help: SchemaHelp;
	saving: boolean;
	saveError?: string | null;
	onCancel: () => void;
	onSave: (target: McpTarget, endpoint: string | undefined, previousId?: string) => void;
}) {
	const [form, setForm] = useState(() => formFrom(props.initial));
	const [initialForm] = useState(form);
	const [endpointText, setEndpointText] = useState(props.initialEndpoint ?? '');
	// An existing server stays where it is until the endpoint is edited. Otherwise servers for /mcp
	// go in the top-level list, unless an explicit /mcp endpoint already lists them.
	const typedEndpoint = trimTrailingSlash(endpointText.trim());
	const endpoint =
		props.previousId && endpointText === (props.initialEndpoint ?? '')
			? props.initialEndpoint
			: typedEndpoint && typedEndpoint !== '/mcp'
				? typedEndpoint
				: props.mcpEndpointHasTargets
					? '/mcp'
					: undefined;
	const {
		source,
		name,
		kind,
		url,
		cmd,
		args,
		envText,
		clearEnv,
		schemaMode,
		schemaValue,
		policies
	} = form;
	const network = networkTarget(source);
	const [error, setError] = useState<string | null>(null);
	// A target naming a backend takes that backend's policies; editing the URL
	// connects it directly instead.
	const backendReference =
		network && 'backend' in network && url.trim() === networkUrl(network, targetKind(source))
			? String(network.backend)
			: null;

	const merged = useMergedEdits(props.previousId ? props.initial : undefined, safeTargetPreview(), [
		'$defs',
		'LocalMcpTarget'
	]);

	function targetPreview() {
		const base = {
			...source,
			name: name.trim(),
			// The gateway rejects target policies on stdio and backend references.
			policies: kind === 'stdio' || backendReference ? undefined : policies
		} as McpTarget;
		delete (base as Record<string, unknown>).mcp;
		delete (base as Record<string, unknown>).sse;
		delete (base as Record<string, unknown>).stdio;
		delete (base as Record<string, unknown>).openapi;
		if (kind === 'stdio') {
			const env = envText.trim() ? parseEnvYaml(envText) : {};
			return {
				...base,
				stdio: {
					cmd: cmd.trim(),
					args: splitArgs(args),
					env,
					clear_env: clearEnv
				}
			};
		}
		// The URL field folds host, port, and path into one value. Until it is
		// edited, emit the original address so an edit replaces all of them.
		const target =
			network && url.trim() === networkUrl(network, targetKind(source))
				? { ...network }
				: { host: url.trim() || null };
		if (kind === 'sse') return { ...base, sse: target };
		if (kind === 'openapi')
			return { ...base, openapi: { ...target, schema: schemaFromSource(schemaValue, schemaMode) } };
		return { ...base, mcp: target };
	}

	function update(patch: Partial<McpServerForm>) {
		setForm(current => ({ ...current, ...patch }));
	}

	function safeTargetPreview() {
		try {
			return targetPreview();
		} catch {
			return null;
		}
	}

	function validTargetPreview() {
		try {
			setError(null);
			if (endpoint && !endpoint.startsWith('/'))
				throw new Error('Endpoint path must start with /.');
			return targetPreview();
		} catch (err) {
			setError(err instanceof Error ? err.message : 'Invalid server configuration');
			return null;
		}
	}

	function save() {
		if (!validTargetPreview()) return;
		props.onSave(merged.value as McpTarget, endpoint, props.previousId);
	}

	return (
		<Drawer
			title={props.previousId ? 'Edit MCP server' : 'Add MCP server'}
			onClose={props.onCancel}
			error={props.saveError}
			raw={{
				schemaPath: ['$defs', 'LocalMcpTarget'],
				value: () => merged.value ?? undefined,
				onChange: value => {
					setForm(formFrom(value as McpTarget));
					merged.reset(value);
				},
				empty: makeEmptyMcpTarget()
			}}
			dirty={
				JSON.stringify(form) !== JSON.stringify(initialForm) ||
				endpointText !== (props.initialEndpoint ?? '')
			}
			saving={props.saving}
			footer={requestClose => (
				<ConfigDiffSaveActions
					config={props.config}
					diffTitle="MCP server config diff"
					saveLabel="Save server"
					saving={props.saving}
					onCancel={requestClose}
					onSave={save}
					beforeDiff={() => Boolean(validTargetPreview())}
					resourceDiff={
						props.databaseBacked
							? () => ({
									original: props.previousId
										? { ...props.initial, endpoint: props.initialEndpoint }
										: {},
									modified: { ...(merged.value as McpTarget), endpoint }
								})
							: undefined
					}
					applyDiff={next =>
						upsertMcpTarget(next, merged.value as McpTarget, endpoint, props.previousId)
					}
				/>
			)}
		>
			<LostFieldsWarning lost={merged.lost} />
			<div className="form-grid">
				<Field
					label="Server name"
					path="name"
					tooltip={props.help.field<McpTarget>(
						'LocalMcpTarget',
						'name',
						'Name used to identify this MCP target.'
					)}
				>
					<Input
						value={name}
						onChange={event => update({ name: event.target.value })}
						placeholder="weather"
					/>
				</Field>
				<Field
					label="Endpoint"
					path="endpoint"
					tooltip={props.help.field<McpEndpoint>(
						'LocalMcpEndpoint',
						'path',
						'Path of the MCP endpoint serving this target. Leave empty to serve it at /mcp.'
					)}
				>
					<FreeformCombobox
						ariaLabel="Endpoint"
						value={endpointText}
						options={props.endpointOptions}
						onChange={setEndpointText}
						placeholder="Default (/mcp)"
					/>
				</Field>
			</div>
			<FieldGroup label="Transport" tooltip="How the gateway connects to this MCP target.">
				<SegmentedControl
					ariaLabel="Transport"
					value={kind}
					className="mcp-transport-control"
					options={targetKinds.map(value => ({
						value,
						label: transportLabel(value)
					}))}
					onChange={value =>
						update({
							kind: value,
							url:
								url.trim() ||
								(value === 'sse' ? 'http://localhost:3001/sse' : 'http://localhost:3001/mcp')
						})
					}
				/>
			</FieldGroup>

			{kind === 'stdio' ? (
				<>
					<Field
						label="Command"
						path="stdio.cmd"
						tooltip={props.help.field<McpTarget>(
							'LocalMcpTarget1',
							'stdio.cmd',
							'Command to launch for command-line MCP servers.'
						)}
					>
						<Input
							value={cmd}
							onChange={event => update({ cmd: event.target.value })}
							placeholder="npx"
						/>
					</Field>
					<Field
						label="Arguments"
						tooltip={props.help.field<McpTarget>(
							'LocalMcpTarget1',
							'stdio.args',
							'Command arguments passed to the MCP server process.'
						)}
					>
						<Input
							value={args}
							onChange={event => update({ args: event.target.value })}
							placeholder="-y @modelcontextprotocol/server-filesystem /tmp"
						/>
					</Field>
					<FieldGroup
						label="Environment YAML"
						tooltip={props.help.field<McpTarget>(
							'LocalMcpTarget1',
							'stdio.env',
							'Environment variables set for the MCP server process.'
						)}
					>
						<MiniMonacoEditor
							language="yaml"
							value={envText}
							onChange={value => update({ envText: value })}
						/>
					</FieldGroup>
					<Checkbox
						className="toggle-row"
						checked={clearEnv}
						onChange={event => update({ clearEnv: event.target.checked })}
					>
						Clear environment
					</Checkbox>
				</>
			) : (
				<>
					<Field
						label="URL"
						path={`${kind}.host`}
						tooltip={
							kind === 'sse'
								? props.help.field<McpTarget>(
										'LocalMcpTarget1',
										'sse.host',
										'URL of the MCP server endpoint.'
									)
								: kind === 'openapi'
									? props.help.field<McpTarget>(
											'LocalMcpTarget1',
											'openapi.host',
											'URL of the MCP server endpoint.'
										)
									: props.help.field<McpTarget>(
											'LocalMcpTarget1',
											'mcp.host',
											'URL of the MCP server endpoint.'
										)
						}
					>
						<Input
							value={url}
							onChange={event => update({ url: event.target.value })}
							placeholder={
								kind === 'sse' ? 'http://localhost:3001/sse' : 'http://localhost:3001/mcp'
							}
						/>
					</Field>
					{kind === 'openapi' ? (
						<>
							<FieldGroup
								label="Schema source"
								tooltip={props.help.field<McpTarget>(
									'LocalMcpTarget1',
									'openapi.schema',
									'Where to load the OpenAPI schema document from.'
								)}
							>
								<SegmentedControl
									ariaLabel="Schema source"
									value={schemaMode}
									options={[
										{ value: 'url', label: 'URL' },
										{ value: 'file', label: 'File' },
										{ value: 'inline', label: 'Inline' }
									]}
									onChange={value => update({ schemaMode: value })}
								/>
							</FieldGroup>
							{schemaMode === 'inline' ? (
								<FieldGroup
									label="Schema"
									missing={!schemaValue.trim()}
									tooltip={props.help.field<McpTarget>(
										'LocalMcpTarget1',
										'openapi.schema',
										'Inline OpenAPI schema document.'
									)}
								>
									<MiniMonacoEditor
										language="yaml"
										value={schemaValue}
										onChange={value => update({ schemaValue: value })}
									/>
								</FieldGroup>
							) : (
								<Field
									label="Schema"
									missing={!schemaValue.trim()}
									tooltip={props.help.field<McpTarget>(
										'LocalMcpTarget1',
										'openapi.schema',
										schemaMode === 'file'
											? 'Path to the OpenAPI schema document on disk.'
											: 'URL to fetch the OpenAPI schema document from.'
									)}
								>
									<Input
										value={schemaValue}
										onChange={event => update({ schemaValue: event.target.value })}
										placeholder={
											schemaMode === 'file'
												? '/etc/agentgateway/openapi.yaml'
												: 'https://example.com/openapi.json'
										}
									/>
								</Field>
							)}
						</>
					) : null}
					{backendReference ? null : (
						<BackendPoliciesField
							help={props.help}
							value={policies}
							onChange={value => update({ policies: value as McpTarget['policies'] })}
						/>
					)}
				</>
			)}
			{error ? (
				<StatusBanner state="bad" title="Invalid server">
					{error}
				</StatusBanner>
			) : null}
		</Drawer>
	);
}

function targetKind(target: McpTarget): McpTargetKind {
	if ('sse' in target) return 'sse';
	if ('stdio' in target) return 'stdio';
	if ('openapi' in target) return 'openapi';
	return 'mcp';
}

function networkTarget(target: McpTarget) {
	if ('sse' in target) return target.sse;
	if ('mcp' in target) return target.mcp;
	if ('openapi' in target) return target.openapi;
	return undefined;
}

function targetEndpoint(target: McpTarget) {
	if ('stdio' in target) return stdioCommandLine(target.stdio);
	const network = networkTarget(target);
	if (!network) return 'n/a';
	const host = network.host ?? 'localhost';
	const port = network.port ? `:${network.port}` : '';
	const path = network.path ?? '';
	return `${host}${port}${path}`;
}

function stdioCommandLine(stdio: { cmd: string; args?: string[] }) {
	const parts = [stdio.cmd, ...(stdio.args ?? [])].filter(part => part.trim());
	return parts.map(shellDisplayArg).join(' ');
}

function shellDisplayArg(value: string) {
	return /\s/.test(value) ? JSON.stringify(value) : value;
}

function targetWarnings(target: McpTarget) {
	const warnings: string[] = [];
	if (!target.name.trim()) warnings.push('Server name is required.');
	if (target.name.includes(':')) warnings.push('Server name must not contain ":".');
	if ('stdio' in target && !target.stdio.cmd.trim()) warnings.push('Command is required.');
	if (!('stdio' in target)) {
		const network = networkTarget(target);
		if (!network?.host) warnings.push('URL should be set.');
	}
	if (
		'openapi' in target &&
		!schemaSourceValue(target.openapi.schema, schemaSourceMode(target.openapi.schema)).trim()
	)
		warnings.push('Schema is required.');
	return warnings;
}

function splitArgs(value: string) {
	return value.trim() ? value.trim().split(/\s+/) : [];
}

function parseEnvYaml(value: string) {
	const parsed = parseYamlText(value);
	if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
		throw new Error('Environment must be a YAML mapping.');
	}
	return Object.fromEntries(Object.entries(parsed).map(([key, item]) => [key, String(item)]));
}

function transportLabel(kind: McpTargetKind) {
	if (kind === 'mcp') return 'Streamable HTTP';
	if (kind === 'sse') return 'Legacy SSE';
	if (kind === 'stdio') return 'Command Line';
	return 'OpenAPI';
}

function networkUrl(network: ReturnType<typeof networkTarget>, kind: McpTargetKind) {
	if (!network) return kind === 'sse' ? 'http://localhost:3001/sse' : 'http://localhost:3001/mcp';
	if (network.host?.startsWith('http://') || network.host?.startsWith('https://'))
		return network.host;
	const host = network.host ?? 'localhost';
	const port = network.port ? `:${network.port}` : '';
	const path = network.path ?? (kind === 'openapi' ? '' : kind === 'sse' ? '/sse' : '/mcp');
	return `http://${host}${port}${path}`;
}

type SchemaSourceMode = 'file' | 'url' | 'inline';

function schemaSourceMode(value: unknown): SchemaSourceMode {
	if (value && typeof value === 'object' && 'file' in value) return 'file';
	if (value && typeof value === 'object' && 'url' in value) return 'url';
	if (typeof value === 'string') return 'inline';
	return 'url';
}

function schemaSourceValue(value: unknown, mode: SchemaSourceMode) {
	if (mode === 'file' && value && typeof value === 'object' && 'file' in value)
		return typeof value.file === 'string' ? value.file : '';
	if (mode === 'url' && value && typeof value === 'object' && 'url' in value)
		return typeof value.url === 'string' ? value.url : '';
	if (mode === 'inline' && typeof value === 'string') return value;
	return '';
}

function schemaFromSource(value: string, mode: SchemaSourceMode): unknown {
	const trimmed = value.trim();
	if (mode === 'file') return { file: trimmed };
	if (mode === 'url') return { url: trimmed };
	return value;
}
