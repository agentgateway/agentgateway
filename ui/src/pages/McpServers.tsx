import { Pencil, Plus, Server, SlidersHorizontal, Trash2 } from 'lucide-react';
import { useMemo, useState } from 'react';

import type { McpSettingsResource } from '@/api/configResourcesApi';
import { ConfigDiffSaveActions } from '@/components/ConfigDiffDrawer';
import { EnumSelector } from '@/components/EnumSelector';
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
	upsertMcpTarget
} from '@/config';
import { useStickyQueryParam } from '@/drawerRouteState';
import { useDeleteConfigResource, useMcpConfigData, useUpsertConfigResource } from '@/hooks';
import { tr } from '@/i18n';
import { PolicySection } from '@/policies/PolicyLayout';
import { parseYamlText, toYamlMappingText } from '@/policies/policyUtils';
import { type SchemaHelp, useSchemaHelp } from '@/schemaHelp';
import type {
	GatewayConfig,
	McpConfig,
	McpFailureMode,
	McpPrefixMode,
	McpStatefulMode,
	McpTarget,
	McpTargetKind
} from '@/types';

const targetKinds: McpTargetKind[] = ['mcp', 'sse', 'stdio'];

type McpSettingsPatch = Partial<Omit<McpConfig, 'gateways' | 'port'>> & {
	gateways?: McpConfig['gateways'] | null;
	port?: number | null;
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
	const targets = useMemo(() => mcp?.targets ?? [], [mcp]);
	const fileOwnedSettingFields = fileOwnedMcpSettingFields(rawConfig.data, hybrid);
	const saving = upsertResource.isPending || deleteResource.isPending;
	const saveError = upsertResource.error?.message ?? deleteResource.error?.message ?? null;
	const [editing, setEditing] = useState<{
		previousName?: string;
		target: McpTarget;
	} | null>(null);
	const [deletingServer, setDeletingServer] = useState<string | null>(null);
	const [serverDrawer, setServerDrawer] = useStickyQueryParam('server');
	const linkedTarget =
		serverDrawer && serverDrawer !== 'new' && serverDrawer !== 'settings'
			? targets.find(target => target.name === serverDrawer)
			: null;
	const activeEditing =
		editing ??
		(serverDrawer === 'new'
			? { target: makeEmptyMcpTarget() }
			: linkedTarget
				? {
						previousName: linkedTarget.name,
						target: structuredClone(linkedTarget)
					}
				: null);
	const settingsOpen = serverDrawer === 'settings';

	function openNewServer() {
		setEditing(null);
		setServerDrawer('new');
	}

	function openEditServer(target: McpTarget) {
		setEditing(null);
		setServerDrawer(target.name);
	}

	function closeServerDrawer() {
		setEditing(null);
		setServerDrawer(null, 'replace');
	}

	return (
		<div className="page-stack">
			<PageHeader
				title={tr('copy.mcpServers')}
				description={tr('copy.configureMcpTargetsServedByTheGateway')}
				actions={
					<div className="button-row">
						<button className="button" type="button" onClick={() => setServerDrawer('settings')}>
							<SlidersHorizontal size={16} />
							{tr('copy.settings')}
						</button>
						<button className="button primary" type="button" onClick={openNewServer}>
							<Plus size={16} />
							{tr('copy.addServer')}
						</button>
					</div>
				}
			/>

			{saveError && !activeEditing && !settingsOpen ? (
				<StatusBanner state="bad" title={tr('copy.saveFailed')}>
					{saveError}
				</StatusBanner>
			) : null}
			{upsertResource.isSuccess || deleteResource.isSuccess ? (
				<StatusBanner state="ok" title={tr('copy.configurationSaved')} />
			) : null}

			<Panel>
				{mcpData.isLoading ? (
					<StatusBanner state="loading" title={tr('copy.loadingMcpServers')} />
				) : mcpData.error ? (
					<StatusBanner state="bad" title={tr('copy.configurationApiUnavailable')}>
						{mcpData.error.message}
					</StatusBanner>
				) : targets.length === 0 ? (
					<EmptyState
						title={tr('copy.noMcpServersConfigured')}
						description={tr('copy.addATargetSoTheGatewayCanExposeMcpTraffic')}
						action={
							<button className="button primary" type="button" onClick={openNewServer}>
								<Server size={16} />
								{tr('copy.addServer')}
							</button>
						}
					/>
				) : (
					<div className="table-wrap">
						<table>
							<thead>
								<tr>
									<th>{tr('copy.name')}</th>
									<th>{tr('copy.type')}</th>
									<th>{tr('copy.endpoint')}</th>
									<th>{tr('copy.state')}</th>
									<th />
								</tr>
							</thead>
							<tbody>
								{targets.map(target => {
									const kind = targetKind(target);
									const warnings = targetWarnings(target);
									const databaseBacked = isDatabaseConfigResource(
										resources,
										'mcp.target',
										target.name
									);
									return (
										<tr key={target.name}>
											<td className="strong">{target.name}</td>
											<td>
												<span className="badge">{transportLabel(kind)}</span>
											</td>
											<td>
												<code>{targetEndpoint(target)}</code>
											</td>
											<td>
												{warnings.length ? (
													<span className="badge warn">
														{warnings.length}
														{tr('copy.warnings')}
													</span>
												) : (
													<span className="badge ok">{tr('copy.ready')}</span>
												)}
											</td>
											<td className="row-actions">
												<Tooltip content={tr('copy.editServer')}>
													<button
														className="icon-button"
														aria-label={tr('copy.editServer')}
														type="button"
														onClick={() => openEditServer(target)}
													>
														<Pencil size={16} />
													</button>
												</Tooltip>
												<Tooltip
													content={
														hybrid && !databaseBacked
															? tr('copy.fileOwnedServersCannotBeDeletedHere')
															: tr('copy.deleteServer')
													}
												>
													<button
														className="icon-button danger"
														aria-label={tr('copy.deleteServer')}
														type="button"
														disabled={saving || (hybrid && !databaseBacked)}
														onClick={() => setDeletingServer(target.name)}
													>
														<Trash2 size={16} />
													</button>
												</Tooltip>
											</td>
										</tr>
									);
								})}
							</tbody>
						</table>
					</div>
				)}
			</Panel>

			{activeEditing ? (
				<McpServerEditor
					key={activeEditing.previousName ?? 'new'}
					initial={activeEditing.target}
					config={effectiveConfig}
					previousName={activeEditing.previousName}
					databaseBacked={
						hybrid &&
						(!activeEditing.previousName ||
							isDatabaseConfigResource(resources, 'mcp.target', activeEditing.previousName))
					}
					help={help}
					saving={saving}
					saveError={saveError}
					onCancel={closeServerDrawer}
					onSave={(target, previousName) => {
						upsertResource.mutate(
							{ kind: 'mcp.target', value: target, previousId: previousName },
							{
								onSuccess: closeServerDrawer
							}
						);
					}}
				/>
			) : null}
			{settingsOpen ? (
				<McpSettingsDrawer
					config={effectiveConfig}
					mcp={mcp}
					databaseBacked={hybrid}
					readOnlyFields={fileOwnedSettingFields}
					help={help}
					saving={saving}
					saveError={saveError}
					onClose={closeServerDrawer}
					onSave={settings => {
						const value = Object.fromEntries(
							Object.entries(settings).filter(([, field]) => field != null)
						) as McpSettingsResource;
						upsertResource.mutate(
							{ kind: 'mcp.settings', value },
							{
								onSuccess: closeServerDrawer
							}
						);
					}}
				/>
			) : null}
			{deletingServer ? (
				<ConfirmDialog
					title={tr('copy.deleteMcpServer')}
					destructive
					confirmLabel={tr('copy.deleteServer')}
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
					<p>{tr('copy.deleteTargetQuestion', [deletingServer])}</p>
				</ConfirmDialog>
			) : null}
		</div>
	);
}

export function McpSettingsDrawer(props: {
	config?: GatewayConfig | null;
	mcp?: McpConfig | null;
	databaseBacked?: boolean;
	readOnlyFields?: ReadonlySet<string>;
	help: SchemaHelp;
	saving: boolean;
	saveError?: string | null;
	onClose: () => void;
	onSave: (settings: McpSettingsPatch) => void;
}) {
	return (
		<Drawer title={tr('copy.settings')} onClose={props.onClose}>
			<McpSettings
				config={props.config}
				mcp={props.mcp}
				databaseBacked={props.databaseBacked}
				readOnlyFields={props.readOnlyFields}
				help={props.help}
				saving={props.saving}
				onSave={props.onSave}
			/>
			{props.saveError ? (
				<StatusBanner state="bad" title={tr('copy.saveFailed')}>
					{props.saveError}
				</StatusBanner>
			) : null}
		</Drawer>
	);
}

function McpSettings(props: {
	config?: GatewayConfig | null;
	mcp?: McpConfig | null;
	databaseBacked?: boolean;
	readOnlyFields?: ReadonlySet<string>;
	help: SchemaHelp;
	saving: boolean;
	onSave: (settings: McpSettingsPatch) => void;
}) {
	const [binding, setBinding] = useState<GatewayBindingValue>({
		gateways: props.mcp?.gateways ?? null,
		port: props.mcp?.port ?? null
	});
	const [statefulMode, setStatefulMode] = useState<McpStatefulMode>(
		props.mcp?.statefulMode ?? 'stateless'
	);
	const [prefixMode, setPrefixMode] = useState<McpPrefixMode | 'none'>(
		props.mcp?.prefixMode ?? 'none'
	);
	const [failureMode, setFailureMode] = useState<McpFailureMode>(
		props.mcp?.failureMode ?? 'failClosed'
	);
	const patch: McpSettingsPatch = {
		gateways: binding.gateways ?? [],
		port: binding.gateways != null && binding.gateways.length > 0 ? null : binding.port,
		statefulMode,
		prefixMode: prefixMode === 'none' ? null : prefixMode,
		failureMode
	};
	const originalResourceValue = Object.fromEntries(
		Object.entries({
			gateways: props.mcp?.gateways,
			port: props.mcp?.port,
			statefulMode: props.mcp?.statefulMode,
			prefixMode: props.mcp?.prefixMode,
			failureMode: props.mcp?.failureMode
		}).filter(([field, value]) => value != null && !props.readOnlyFields?.has(field))
	);
	const resourceValue = Object.fromEntries(
		Object.entries(patch).filter(
			([field, value]) => value != null && !props.readOnlyFields?.has(field)
		)
	);
	const writablePatch = Object.fromEntries(
		Object.entries(patch).filter(([field]) => !props.readOnlyFields?.has(field))
	) as McpSettingsPatch;
	const bindingReadOnly = Boolean(
		props.readOnlyFields?.has('gateways') || props.readOnlyFields?.has('port')
	);

	return (
		<form
			className="policy-editor-stack"
			onSubmit={event => {
				event.preventDefault();
				props.onSave(writablePatch);
			}}
		>
			{props.readOnlyFields?.size ? (
				<StatusBanner state="warn" title={tr('copy.someSettingsAreFileOwned')}>
					{tr(
						'copy.disabledFieldsAreManagedByTheFileConfigurationOtherMcpSettingsCanStillBeSavedToTheDatabase'
					)}
				</StatusBanner>
			) : null}
			<PolicySection
				icon={<Server size={17} />}
				title={tr('copy.gatewayBinding')}
				description={tr('copy.chooseHowMcpIsExposed')}
			>
				<div className="form-grid">
					<GatewayBindingEditor
						config={props.config}
						value={binding}
						defaultPort={3000}
						portLabel={tr('copy.port')}
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
			<PolicySection
				icon={<SlidersHorizontal size={17} />}
				title={tr('copy.mcpBehavior')}
				description={tr('copy.chooseSessionToolPrefixAndFailureBehavior')}
			>
				<div className="form-grid">
					<FieldGroup
						label={tr('copy.stateMode')}
						tooltip={props.help.field<McpConfig>(
							'LocalSimpleMcpConfig',
							'statefulMode',
							'Controls whether MCP sessions are preserved by the gateway.'
						)}
					>
						<EnumSelector
							ariaLabel={tr('copy.stateMode')}
							value={statefulMode}
							options={[
								{
									value: 'stateless',
									label: tr('copy.stateless'),
									description: tr('copy.doNotPreserveMcpSessionStateBetweenRequests')
								},
								{
									value: 'stateful',
									label: tr('copy.stateful'),
									description: tr('copy.preserveMcpSessionsSoTargetsCanKeepPerSessionContext')
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
						label={tr('copy.prefixMode')}
						tooltip={props.help.field<McpConfig>(
							'LocalSimpleMcpConfig',
							'prefixMode',
							'Controls whether target names are prefixed when exposing tools.'
						)}
					>
						<EnumSelector
							ariaLabel={tr('copy.prefixMode')}
							value={prefixMode}
							options={[
								{
									value: 'none',
									label: tr('copy.none_deku7v'),
									description: tr('copy.exposeToolNamesWithoutAddingTheTargetName')
								},
								{
									value: 'always',
									label: tr('copy.always'),
									description: tr('copy.alwaysPrefixExposedToolNamesWithTheTargetName')
								},
								{
									value: 'conditional',
									label: tr('copy.conditional'),
									description: tr('copy.prefixOnlyWhenNeededToAvoidToolNameConflicts')
								},
								{
									value: 'never',
									label: tr('copy.never'),
									description: tr(
										'copy.neverPrefixCallsAreRoutedByToolNameWhichMustBeUniqueAcrossTargets'
									)
								}
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
						label={tr('copy.failureMode')}
						tooltip={props.help.field<McpConfig>('LocalSimpleMcpConfig', 'failureMode')}
					>
						<EnumSelector
							ariaLabel={tr('copy.failureMode')}
							value={failureMode}
							options={[
								{ value: 'failClosed', label: tr('copy.failClosed') },
								{ value: 'failOpen', label: tr('copy.failOpen') }
							]}
							schema={props.help.node(['$defs', 'McpBackendFailureMode'])}
							disabled={props.readOnlyFields?.has('failureMode')}
							onChange={setFailureMode}
						/>
					</FieldGroup>
				</div>
			</PolicySection>
			<ConfigDiffSaveActions
				config={props.config}
				diffTitle={tr('copy.mcpSettingsConfigDiff')}
				saveLabel={tr('copy.saveSettings')}
				saving={props.saving}
				saveDisabled={Object.keys(writablePatch).length === 0}
				onSave={() => props.onSave(writablePatch)}
				resourceDiff={
					props.databaseBacked
						? {
								original: originalResourceValue,
								modified: resourceValue
							}
						: undefined
				}
				applyDiff={next => {
					Object.assign(ensureMcp(next), patch);
				}}
			/>
		</form>
	);
}

function McpServerEditor(props: {
	initial: McpTarget;
	config?: GatewayConfig | null;
	previousName?: string;
	databaseBacked?: boolean;
	help: SchemaHelp;
	saving: boolean;
	saveError?: string | null;
	onCancel: () => void;
	onSave: (target: McpTarget, previousName?: string) => void;
}) {
	const [name, setName] = useState(props.initial.name);
	const [kind, setKind] = useState<McpTargetKind>(() => {
		const kind = targetKind(props.initial);
		return kind === 'openapi' ? 'mcp' : kind;
	});
	const network = networkTarget(props.initial);
	const stdio = 'stdio' in props.initial ? props.initial.stdio : undefined;
	const [url, setUrl] = useState(() => networkUrl(network, kind));
	const [cmd, setCmd] = useState(stdio?.cmd ?? '');
	const [args, setArgs] = useState((stdio?.args ?? []).join(' '));
	const [envText, setEnvText] = useState(toYamlMappingText(stdio?.env));
	const [clearEnv, setClearEnv] = useState(Boolean(stdio?.clear_env));
	const [error, setError] = useState<string | null>(null);
	const draft = JSON.stringify({
		name,
		kind,
		url,
		cmd,
		args,
		envText,
		clearEnv
	});
	const [initialDraft] = useState(() => draft);

	function targetPreview() {
		const base = {
			...props.initial,
			name: name.trim(),
			policies: props.initial.policies
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
		const target = {
			host: url.trim() || null
		};
		return kind === 'sse' ? { ...base, sse: target } : { ...base, mcp: target };
	}

	function validTargetPreview() {
		try {
			setError(null);
			return targetPreview();
		} catch (err) {
			setError(err instanceof Error ? err.message : tr('copy.invalidServerConfiguration'));
			return null;
		}
	}

	function save() {
		const target = validTargetPreview();
		if (!target) return;
		props.onSave(target, props.previousName);
	}

	return (
		<Drawer
			title={props.previousName ? tr('copy.editMcpServer') : tr('copy.addMcpServer')}
			onClose={props.onCancel}
			dirty={draft !== initialDraft}
			saving={props.saving}
			footer={requestClose => (
				<ConfigDiffSaveActions
					config={props.config}
					diffTitle={tr('copy.mcpServerConfigDiff')}
					saveLabel={tr('copy.saveServer')}
					saving={props.saving}
					saveDisabled={!name.trim() || (kind === 'stdio' && !cmd.trim())}
					onCancel={requestClose}
					onSave={save}
					beforeDiff={() => Boolean(validTargetPreview())}
					resourceDiff={
						props.databaseBacked
							? () => ({
									original: props.previousName ? props.initial : {},
									modified: targetPreview()
								})
							: undefined
					}
					applyDiff={next => {
						const target = targetPreview();
						upsertMcpTarget(next, target, props.previousName);
					}}
				/>
			)}
		>
			<div className="form-grid">
				<Field
					label={tr('copy.serverName')}
					tooltip={props.help.field<McpTarget>(
						'LocalMcpTarget',
						'name',
						'Name used to identify this MCP target.'
					)}
				>
					<input
						value={name}
						onChange={event => setName(event.target.value)}
						placeholder="weather"
					/>
				</Field>
			</div>
			<FieldGroup
				label={tr('copy.transport')}
				tooltip={tr('copy.howTheGatewayConnectsToThisMcpTarget')}
			>
				<SegmentedControl
					ariaLabel={tr('copy.transport')}
					value={kind}
					className="mcp-transport-control"
					options={targetKinds.map(value => ({
						value,
						label: transportLabel(value)
					}))}
					onChange={value => {
						setKind(value);
						if (!url.trim())
							setUrl(value === 'sse' ? 'http://localhost:3001/sse' : 'http://localhost:3001/mcp');
					}}
				/>
			</FieldGroup>

			{kind === 'stdio' ? (
				<>
					<Field
						label={tr('copy.command')}
						tooltip={props.help.field<McpTarget>(
							'LocalMcpTarget1',
							'stdio.cmd',
							'Command to launch for command-line MCP servers.'
						)}
					>
						<input value={cmd} onChange={event => setCmd(event.target.value)} placeholder="npx" />
					</Field>
					<Field
						label={tr('copy.arguments')}
						tooltip={props.help.field<McpTarget>(
							'LocalMcpTarget1',
							'stdio.args',
							'Command arguments passed to the MCP server process.'
						)}
					>
						<input
							value={args}
							onChange={event => setArgs(event.target.value)}
							placeholder={tr('copy.yModelcontextprotocolServerFilesystemTmp')}
						/>
					</Field>
					<FieldGroup
						label={tr('copy.environmentYaml')}
						tooltip={props.help.field<McpTarget>(
							'LocalMcpTarget1',
							'stdio.env',
							'Environment variables set for the MCP server process.'
						)}
					>
						<MiniMonacoEditor language="yaml" value={envText} onChange={setEnvText} />
					</FieldGroup>
					<label className="toggle-row">
						<input
							type="checkbox"
							checked={clearEnv}
							onChange={event => setClearEnv(event.target.checked)}
						/>
						{tr('copy.clearEnvironment')}
					</label>
				</>
			) : (
				<Field
					label={tr('copy.url')}
					tooltip={
						kind === 'sse'
							? props.help.field<McpTarget>(
									'LocalMcpTarget1',
									'sse.host',
									'URL of the MCP server endpoint.'
								)
							: props.help.field<McpTarget>(
									'LocalMcpTarget1',
									'mcp.host',
									'URL of the MCP server endpoint.'
								)
					}
				>
					<input
						value={url}
						onChange={event => setUrl(event.target.value)}
						placeholder={kind === 'sse' ? 'http://localhost:3001/sse' : 'http://localhost:3001/mcp'}
					/>
				</Field>
			)}
			{error ? (
				<StatusBanner state="bad" title={tr('copy.invalidServer')}>
					{error}
				</StatusBanner>
			) : null}
			{props.saveError ? (
				<StatusBanner state="bad" title={tr('copy.saveFailed')}>
					{props.saveError}
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
	if ('stdio' in target && !target.stdio.cmd.trim()) warnings.push('Command is required.');
	if (!('stdio' in target)) {
		const network = networkTarget(target);
		if (!network?.host) warnings.push('URL should be set.');
	}
	return warnings;
}

function splitArgs(value: string) {
	return value.trim() ? value.trim().split(/\s+/) : [];
}

function parseEnvYaml(value: string) {
	const parsed = parseYamlText(value);
	if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
		throw new Error(tr('copy.environmentMustBeAYamlMapping'));
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
	const path = network.path ?? (kind === 'sse' ? '/sse' : '/mcp');
	return `http://${host}${port}${path}`;
}
