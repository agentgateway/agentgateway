import { Link, useNavigate } from '@tanstack/react-router';
import { Bot, Network, Server } from 'lucide-react';
import { useEffect, useState } from 'react';

import { refreshBaseCosts } from '@/api/costsApi';
import { gatewayOptions } from '@/components/GatewayBindingEditor';
import { Dropdown, FieldGroup, PageHeader, Panel, StatusBanner } from '@/components/Primitives';
import { startupGatewayRefs } from '@/config';
import { refreshBaseCostsAndConfigure } from '@/costs';
import {
	useEffectiveGatewayConfig,
	useEnableSurface,
	useMcpConfigData,
	useTrafficConfigData,
	useUpdateConfig
} from '@/hooks';
import { tr } from '@/i18n';
import type { GatewayConfig } from '@/types';

type SurfaceKind = 'llm' | 'mcp' | 'traffic';

const surfaceConfig: Record<
	SurfaceKind,
	{
		title: string;
		name: string;
		description: string;
		icon: typeof Bot;
		enabled: (config: GatewayConfig | undefined) => boolean;
		destination: string;
		destinationLabel: string;
	}
> = {
	llm: {
		get title() {
			return tr('copy.enableLlm');
		},
		get name() {
			return tr('copy.models');
		},
		get description() {
			return tr(
				'copy.createTheLlmConfigurationSectionSoModelsProvidersKeysGuardrailsLogsAndPlayground_197f4qj'
			);
		},
		icon: Bot,
		enabled: config => Boolean(config?.llm),
		destination: '/llm/models',
		get destinationLabel() {
			return tr('copy.continueToValue', [tr('copy.models')]);
		}
	},
	mcp: {
		get title() {
			return tr('copy.enableMcp');
		},
		get name() {
			return tr('copy.servers');
		},
		get description() {
			return tr(
				'copy.createTheMcpConfigurationSectionSoServersAndMcpPlaygroundToolsCanBeConfigured'
			);
		},
		icon: Server,
		enabled: config => Boolean(config?.mcp),
		destination: '/mcp/servers',
		get destinationLabel() {
			return tr('copy.continueToValue', [tr('copy.servers')]);
		}
	},
	traffic: {
		get title() {
			return tr('copy.enableTraffic');
		},
		get name() {
			return tr('copy.gateways');
		},
		get description() {
			return tr(
				'copy.createTheTrafficConfigurationSectionSoHttpGatewaysRoutesBackendsAndPoliciesCanBeConfigured'
			);
		},
		icon: Network,
		enabled: config =>
			Boolean(config && ('gateways' in config || 'routes' in config || 'binds' in config)),
		destination: '/traffic/gateways',
		get destinationLabel() {
			return tr('copy.continueToValue', [tr('copy.gateways')]);
		}
	}
};

export function LlmGetStartedPage() {
	return <GetStartedPage surface="llm" />;
}

export function McpGetStartedPage() {
	return <GetStartedPage surface="mcp" />;
}

export function TrafficGetStartedPage() {
	return <GetStartedPage surface="traffic" />;
}

function GetStartedPage(props: { surface: SurfaceKind }) {
	const config = useEffectiveGatewayConfig();
	const mcpData = useMcpConfigData();
	const trafficData = useTrafficConfigData();
	const update = useUpdateConfig();
	const enableSurface = useEnableSurface();
	const navigate = useNavigate();
	const surface = surfaceConfig[props.surface];
	const Icon = surface.icon;
	const effectiveConfig =
		props.surface === 'mcp'
			? mcpData.data
			: props.surface === 'traffic'
				? trafficData.data
				: config.data;
	const loading =
		config.isLoading ||
		(props.surface === 'mcp' && mcpData.isLoading) ||
		(props.surface === 'traffic' && trafficData.isLoading);
	const configError =
		config.error ??
		(props.surface === 'mcp'
			? mcpData.error
			: props.surface === 'traffic'
				? trafficData.error
				: null);
	const enabled = surface.enabled(effectiveConfig);
	const [gateway, setGateway] = useState('');
	const options = gatewayOptions(trafficData.data ?? config.data);
	const defaultGateways = startupGatewayRefs(trafficData.data ?? config.data);

	useEffect(() => {
		if (!loading && !configError && enabled) {
			void navigate({ to: surface.destination, replace: true });
		}
	}, [configError, enabled, loading, navigate, surface.destination]);

	async function enable() {
		if (enabled) {
			void navigate({ to: surface.destination });
			return;
		}
		try {
			const { hybrid } = await enableSurface.mutateAsync({
				surface: props.surface,
				gateway: gateway || undefined
			});
			void navigate({ to: surface.destination });
			if (props.surface === 'llm') {
				void (hybrid ? refreshBaseCosts() : refreshBaseCostsAndConfigure(update)).catch(
					() => undefined
				);
			}
		} catch {
			// The enable mutation exposes the save error.
		}
	}

	if (!loading && !configError && enabled) {
		return (
			<div className="page-stack">
				<StatusBanner state="loading" title={tr('copy.openingValue', [surface.destinationLabel])} />
			</div>
		);
	}

	return (
		<div className="page-stack">
			<PageHeader title={surface.title} description={surface.description} />

			{loading ? (
				<StatusBanner state="loading" title={tr('copy.loadingGatewayConfiguration')} />
			) : null}
			{configError ? (
				<StatusBanner state="bad" title={tr('copy.configurationApiUnavailable')}>
					{configError.message}
				</StatusBanner>
			) : null}
			{enableSurface.isError || update.isError ? (
				<StatusBanner state="bad" title={tr('copy.saveFailed')}>
					{enableSurface.error?.message ?? update.error?.message}
				</StatusBanner>
			) : null}

			<Panel className="surface-enable-panel">
				<div className="surface-enable-heading">
					<span className="policy-form-section-icon">
						<Icon size={18} />
					</span>
					<div>
						<h3>{enabled ? tr('copy.valueEnabled', [surface.name]) : surface.title}</h3>
						<p>
							{enabled ? tr('copy.topLevelConfigurationSectionAlreadyExists') : surface.description}
						</p>
					</div>
				</div>

				{!enabled && (props.surface === 'llm' || props.surface === 'mcp') ? (
					<details className="schema-details">
						<summary>{tr('copy.advanced')}</summary>
						<FieldGroup label={tr('copy.gateway')}>
							<Dropdown
								ariaLabel={tr('copy.gateway')}
								value={gateway}
								onChange={setGateway}
								options={[
									{
										value: '',
										label: tr('copy.automaticValue', [
											Array.isArray(defaultGateways) ? defaultGateways.join(', ') : defaultGateways
										]),
										description: options.length
											? tr('copy.useTheConfiguredGateway')
											: tr('copy.createADefaultGateway')
									},
									...options
								]}
							/>
						</FieldGroup>
					</details>
				) : null}

				<div className="button-row">
					{enabled ? (
						<Link className="button primary" to={surface.destination}>
							{surface.destinationLabel}
						</Link>
					) : (
						<button
							className="button primary"
							type="button"
							disabled={loading || enableSurface.isPending || update.isPending}
							onClick={() => void enable()}
						>
							{tr('copy.enable')}
						</button>
					)}
					<Link className="button" to="/">
						{tr('copy.backToHome')}
					</Link>
				</div>
			</Panel>
		</div>
	);
}
