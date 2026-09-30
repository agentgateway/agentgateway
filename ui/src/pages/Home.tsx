import { Link } from '@tanstack/react-router';
import { Bot, Network, Server, Settings } from 'lucide-react';
import type { ReactNode } from 'react';
import { useEffect, useState } from 'react';

import type { McpSettingsResource } from '@/api/configResourcesApi';
import { PageHeader, StatusBanner } from '@/components/Primitives';
import { ensureLlm, fileOwnedMcpSettingFields } from '@/config';
import {
	useConfigDumpMode,
	useEnableSurface,
	useLlmConfigData,
	useMcpConfigData,
	useTrafficConfigData,
	useUpdateConfig,
	useUpsertConfigResource
} from '@/hooks';
import { currentLanguage, tr, translateText } from '@/i18n';
import { McpSettingsDrawer } from '@/pages/McpServers';
import { LlmSettingsDrawer } from '@/pages/models/LlmSettingsDrawer';
import { ReadonlyModeBanner, TrafficDumpOverview } from '@/pages/traffic/TrafficConfigDumpPanel';
import { useSchemaHelp } from '@/schemaHelp';
import { trafficStats } from '@/traffic';
import type { GatewayConfig } from '@/types';

const uiAuthPolicyKeys = ['oidc', 'jwtAuth', 'extAuthz', 'basicAuth', 'apiKey', 'authorization'];

export function HomePage() {
	const mode = useConfigDumpMode();
	const dumpMode = mode.data?.mode === 'dump';
	const {
		config,
		rawConfig,
		runtime,
		hybrid,
		models,
		virtualModels,
		providers,
		warnings,
		isLoading: configDataLoading,
		error: configDataError
	} = useLlmConfigData({
		enabled: Boolean(mode.data && mode.data.mode !== 'dump')
	});
	const mcpData = useMcpConfigData({
		enabled: Boolean(mode.data && mode.data.mode !== 'dump')
	});
	const trafficData = useTrafficConfigData({
		enabled: Boolean(mode.data && mode.data.mode !== 'dump')
	});
	const update = useUpdateConfig();
	const enable = useEnableSurface();
	const upsertResource = useUpsertConfigResource();
	const help = useSchemaHelp();
	const [locallyEnabled, setLocallyEnabled] = useState<Set<StartupSurface>>(() => new Set());
	const hasLlm = Boolean(
		config.data?.llm || models.length || virtualModels.length || providers.length
	);
	const hasMcp = Boolean(mcpData.data?.mcp);
	const hasTraffic = Boolean(
		trafficData.data &&
			(Boolean(trafficData.data.binds?.length) ||
				'gateways' in trafficData.data ||
				'routes' in trafficData.data ||
				'tcpRoutes' in trafficData.data)
	);
	const hasBinds = Boolean(config.data?.binds?.length);
	const mcpServers = mcpData.data?.mcp?.targets ?? [];
	const fileOwnedMcpSettings = fileOwnedMcpSettingFields(rawConfig.data, hybrid);
	const pageDataLoading = configDataLoading || mcpData.isLoading || trafficData.isLoading;
	const pageDataError = configDataError ?? mcpData.error ?? trafficData.error;
	const uiGatewayNeedsAuthWarning =
		!runtime.isLoading && !runtime.isError && uiExposedWithoutAuth(config.data);
	const callableModels = models.length + virtualModels.length;
	const traffic = trafficStats(trafficData.data);
	const [startupEvaluated, setStartupEvaluated] = useState(false);
	const [startupFlow, setStartupFlow] = useState(false);
	const [llmSettingsOpen, setLlmSettingsOpen] = useState(false);
	const [mcpSettingsOpen, setMcpSettingsOpen] = useState(false);
	const showStartup = Boolean(config.data && startupFlow);
	const anySurfaceEnabled = hasLlm || hasMcp || hasTraffic || locallyEnabled.size > 0;

	useEffect(() => {
		if (!config.data || pageDataLoading || pageDataError || startupEvaluated) return;
		setStartupFlow(!hasLlm && !hasMcp && (!hasTraffic || isDefaultUiGatewayScaffold(config.data)));
		setStartupEvaluated(true);
	}, [config.data, pageDataError, pageDataLoading, hasLlm, hasMcp, hasTraffic, startupEvaluated]);

	async function enableSurface(surface: StartupSurface) {
		try {
			await enable.mutateAsync({
				surface: surface === 'apis' ? 'traffic' : surface
			});
			setLocallyEnabled(current => new Set(current).add(surface));
		} catch {
			// The enable mutation exposes the save error.
		}
	}

	if (mode.isLoading || (!dumpMode && pageDataLoading)) {
		return (
			<div className="page-stack">
				<StatusBanner state="loading" title={tr('copy.loadingGatewayConfiguration')} />
			</div>
		);
	}

	if (dumpMode) {
		return (
			<div className="page-stack">
				<PageHeader title={tr('copy.gatewayOverview')} />
				<ReadonlyModeBanner />
				<TrafficDumpOverview dump={mode.data?.dump} />
			</div>
		);
	}

	if (showStartup) {
		return (
			// biome-ignore lint/a11y/noStaticElementInteractions: Existing lint violation; remove this suppression when the underlying issue is fixed.
			// biome-ignore lint/a11y/useKeyWithClickEvents: Existing lint violation; remove this suppression when the underlying issue is fixed.
			<div className="startup-shell" onClick={() => setStartupFlow(false)}>
				{/** biome-ignore lint/a11y/useKeyWithClickEvents: Existing lint violation; remove this suppression when the underlying issue is fixed. */}
				<section
					className="startup-panel"
					role="dialog"
					aria-modal="true"
					aria-labelledby="startup-title"
					onClick={event => event.stopPropagation()}
				>
					<div className="startup-copy">
						<h2 id="startup-title">{tr('copy.welcomeToAgentgateway')}</h2>
						<p>{tr('copy.chooseWhatThisGatewayWillServeAnythingSkippedCanBeEnabledLater')}</p>
					</div>

					{pageDataError ? (
						<StatusBanner state="bad" title={tr('copy.configurationApiUnavailable')}>
							{pageDataError.message}
						</StatusBanner>
					) : null}
					{enable.isError || update.isError ? (
						<StatusBanner state="bad" title={tr('copy.saveFailed')}>
							{enable.error?.message ?? update.error?.message}
						</StatusBanner>
					) : null}
					<div className="startup-chip-grid">
						<StartupChip
							label="LLM"
							description={tr('copy.modelsProvidersAndApiKeys')}
							enabled={hasLlm || locallyEnabled.has('llm')}
							disabled={enable.isPending || update.isPending}
							icon={<Bot size={24} />}
							onClick={() => void enableSurface('llm')}
						/>
						<StartupChip
							label="MCP"
							description={tr('copy.mcpServersAndTools')}
							enabled={hasMcp || locallyEnabled.has('mcp')}
							disabled={enable.isPending || update.isPending}
							icon={<Server size={24} />}
							onClick={() => void enableSurface('mcp')}
						/>
						<StartupChip
							label={tr('copy.traffic')}
							description={tr('copy.httpAndTcpRoutesAndBackends')}
							enabled={hasTraffic || locallyEnabled.has('apis')}
							disabled={enable.isPending || update.isPending}
							icon={<Network size={24} />}
							onClick={() => void enableSurface('apis')}
						/>
					</div>

					{anySurfaceEnabled ? (
						<div className="startup-actions">
							<button
								className="button primary"
								type="button"
								onClick={() => setStartupFlow(false)}
							>
								{tr('copy.continue')}
							</button>
						</div>
					) : (
						<div className="startup-actions">
							<button className="button" type="button" onClick={() => setStartupFlow(false)}>
								{tr('copy.skipSetup')}
							</button>
						</div>
					)}
				</section>
			</div>
		);
	}

	return (
		<div className="page-stack">
			<PageHeader title={tr('copy.gatewayOverview')} />

			{enable.isError || update.isError ? (
				<StatusBanner state="bad" title={tr('copy.saveFailed')}>
					{enable.error?.message ?? update.error?.message}
				</StatusBanner>
			) : null}

			{pageDataLoading ? (
				<StatusBanner state="loading" title={tr('copy.loadingGatewayConfiguration')} />
			) : pageDataError ? (
				<StatusBanner state="bad" title={tr('copy.configurationApiUnavailable')}>
					{pageDataError.message}
				</StatusBanner>
			) : warnings.length ? (
				<StatusBanner state="warn" title={tr('copy.valueWarningValue', { count: warnings.length })}>
					<ul className="banner-warning-list">
						{warnings.map(warning => (
							<li key={warning}>{configWarningLabel(warning)}</li>
						))}
					</ul>
				</StatusBanner>
			) : null}
			{uiGatewayNeedsAuthWarning ? (
				<StatusBanner
					state="warn"
					title={tr('copy.uiIsExposedWithoutAuthentication')}
					action={
						<Link className="button" to="/settings">
							{tr('copy.configureUiPolicies')}
						</Link>
					}
				>
					{tr(
						'copy.unauthenticatedUsersCanAccessTheUiConsiderAddingAuthenticationOrAuthorizationPol_qnhsta'
					)}
				</StatusBanner>
			) : null}

			<section className="surface-overview-list" aria-label={tr('copy.gatewaySurfaces')}>
				<SurfaceRow
					title="LLM"
					icon={<Bot size={18} />}
					enabled={hasLlm}
					disabled={enable.isPending || update.isPending}
					onEnable={() => void enableSurface('llm')}
					setupNeeded={callableModels === 0}
					setupText="Add a model before LLM traffic can be served."
					setupTo="/llm/models"
					setupHash="add=model"
					setupLabel="Set up models"
					overview={[
						`${models.length} ${models.length === 1 ? 'model' : 'models'}`,
						`${virtualModels.length} virtual ${virtualModels.length === 1 ? 'model' : 'models'}`,
						`${providers.length} shared ${providers.length === 1 ? 'provider' : 'providers'}`,
						surfaceEndpointLabel(config.data?.llm?.gateways, config.data?.llm?.port ?? 4000)
					]}
					actions={
						<button
							className="button"
							type="button"
							disabled={enable.isPending || update.isPending}
							onClick={() => setLlmSettingsOpen(true)}
						>
							<Settings size={16} />
							{tr('copy.settings')}
						</button>
					}
				/>
				<SurfaceRow
					title="MCP"
					icon={<Server size={18} />}
					enabled={hasMcp}
					disabled={enable.isPending || update.isPending}
					onEnable={() => void enableSurface('mcp')}
					setupNeeded={mcpServers.length === 0}
					setupText="Add an MCP target before tools are available."
					setupTo="/mcp/servers"
					setupLabel="Set up servers"
					overview={[
						tr('copy.valueConfiguredServers', { count: mcpServers.length }),
						surfaceEndpointLabel(mcpData.data?.mcp?.gateways, mcpData.data?.mcp?.port ?? 3000)
					]}
					actions={
						<button
							className="button"
							type="button"
							disabled={enable.isPending || update.isPending}
							onClick={() => setMcpSettingsOpen(true)}
						>
							<Settings size={16} />
							{tr('copy.settings')}
						</button>
					}
				/>
				<SurfaceRow
					title={tr('copy.traffic')}
					icon={<Network size={18} />}
					enabled={hasTraffic}
					disabled={enable.isPending || update.isPending}
					onEnable={() => void enableSurface('apis')}
					setupNeeded={hasBinds ? traffic.listeners === 0 : traffic.gateways === 0}
					setupText={
						hasBinds
							? 'Add a listener before HTTP or TCP traffic can be served.'
							: 'Add a gateway before HTTP traffic can be served.'
					}
					setupTo={hasBinds ? '/traffic/listeners' : '/traffic/gateways'}
					setupLabel={hasBinds ? 'Set up listeners' : 'Set up gateways'}
					overview={
						hasBinds
							? [
									tr('copy.valueBinds', { count: traffic.binds }),
									tr('copy.valueListeners', { count: traffic.listeners }),
									tr('copy.valueRoutes', {
										count: traffic.httpRoutes + traffic.tcpRoutes
									})
								]
							: [
									tr('copy.valueGateways', { count: traffic.gateways }),
									tr('copy.valueRoutes', { count: traffic.httpRoutes })
								]
					}
				/>
			</section>
			{llmSettingsOpen ? (
				<LlmSettingsDrawer
					config={config.data}
					llm={config.data?.llm}
					help={help}
					saving={update.isPending}
					saveError={update.isError ? update.error.message : null}
					onClose={() => setLlmSettingsOpen(false)}
					onSave={settings =>
						update.mutate(
							next => {
								Object.assign(ensureLlm(next), settings);
							},
							{
								onSuccess: () => setLlmSettingsOpen(false)
							}
						)
					}
				/>
			) : null}
			{mcpSettingsOpen ? (
				<McpSettingsDrawer
					config={mcpData.data}
					mcp={mcpData.data?.mcp}
					databaseBacked={hybrid}
					readOnlyFields={fileOwnedMcpSettings}
					help={help}
					saving={update.isPending || upsertResource.isPending}
					saveError={update.error?.message ?? upsertResource.error?.message ?? null}
					onClose={() => setMcpSettingsOpen(false)}
					onSave={settings => {
						const value = Object.fromEntries(
							Object.entries(settings).filter(([, field]) => field != null)
						) as McpSettingsResource;
						upsertResource.mutate(
							{ kind: 'mcp.settings', value },
							{ onSuccess: () => setMcpSettingsOpen(false) }
						);
					}}
				/>
			) : null}
		</div>
	);
}

function surfaceEndpointLabel(gateways: string | string[] | undefined, port: number) {
	if (gateways == null || gateways.length === 0) return `Port ${port}`;
	return `Gateway ${Array.isArray(gateways) ? gateways.join(', ') : gateways}`;
}

function configWarningLabel(warning: string) {
	const virtualKeyMode = warning.match(
		/^Virtual API key mode is ([^;]+); unauthenticated requests may be accepted\.$/
	);
	if (virtualKeyMode) {
		return tr(
			'copy.virtualApiKeyModeIsValueUnauthenticatedRequestsMayBeAccepted',
			translateText(virtualKeyMode[1])
		);
	}
	const modelWarning = warning.match(/^([^:]+): (.+)$/);
	if (modelWarning) {
		const separator = currentLanguage() === 'zh-CN' ? '：' : ': ';
		return `${modelWarning[1]}${separator}${translateText(modelWarning[2])}`;
	}
	return translateText(warning);
}

function uiExposedWithoutAuth(config: GatewayConfig | null | undefined) {
	if (!uiGateway(config)) return false;
	const policies = config?.ui?.policies as Record<string, unknown> | undefined;
	return !uiAuthPolicyKeys.some(key => Boolean(policies?.[key]));
}

function uiGateway(config: GatewayConfig | null | undefined) {
	const gateways = config?.ui?.gateways;
	if (Array.isArray(gateways)) return gateways[0];
	if (gateways) return gateways;
	return config?.ui && config.gateways?.default ? 'default' : undefined;
}

function isDefaultUiGatewayScaffold(config: GatewayConfig) {
	if (!config.ui || uiGateway(config) !== 'default') return false;
	if (config.binds?.length || config.routes?.length || config.tcpRoutes?.length) {
		return false;
	}
	const gatewayNames = Object.keys(config.gateways ?? {});
	return gatewayNames.length === 1 && gatewayNames[0] === 'default';
}

type StartupSurface = 'llm' | 'mcp' | 'apis';

function StartupChip(props: {
	description: string;
	disabled: boolean;
	enabled: boolean;
	icon: ReactNode;
	label: string;
	onClick: () => void;
}) {
	return (
		<button
			className={props.enabled ? 'startup-chip enabled' : 'startup-chip'}
			type="button"
			disabled={props.disabled || props.enabled}
			onClick={props.onClick}
		>
			{props.icon}
			<strong>
				{props.enabled
					? tr('copy.valueEnabled', [props.label])
					: tr('copy.enableValue', [props.label])}
			</strong>
			<span>{props.description}</span>
		</button>
	);
}

function SurfaceRow(props: {
	disabled: boolean;
	enabled: boolean;
	icon: ReactNode;
	actions?: ReactNode;
	links?: Array<{ label: string; to: string }>;
	onEnable: () => void;
	overview: string[];
	setupLabel: string;
	setupNeeded: boolean;
	setupText: string;
	setupHash?: string;
	setupTo: string;
	title: string;
}) {
	if (!props.enabled) {
		return (
			<div className="surface-row compact">
				<div className="surface-row-title">
					{props.icon}
					<strong>{props.title}</strong>
				</div>
				<button className="button" type="button" disabled={props.disabled} onClick={props.onEnable}>
					{tr('copy.enableValue', [props.title])}
				</button>
			</div>
		);
	}

	return (
		<div className={props.setupNeeded ? 'surface-row needs-setup' : 'surface-row'}>
			<div className="surface-row-main">
				<div className="surface-row-title">
					{props.icon}
					<strong>{props.title}</strong>
				</div>
				{props.setupNeeded ? (
					<p>{translateText(props.setupText)}</p>
				) : (
					<div className="surface-metrics">
						{props.overview.map(item => (
							<span key={item}>{item}</span>
						))}
					</div>
				)}
			</div>
			<div className="surface-row-actions">
				{!props.setupNeeded
					? (props.actions ??
						props.links?.map(link => (
							<Link key={link.to} className="button" to={link.to}>
								{translateText(link.label)}
							</Link>
						)))
					: null}
				<Link className="button primary" to={props.setupTo} hash={props.setupHash}>
					{translateText(props.setupLabel)}
				</Link>
			</div>
		</div>
	);
}
