import { Link, useNavigate } from '@tanstack/react-router';
import { Bot, Network, Server } from 'lucide-react';
import { useEffect, useState } from 'react';

import { Field, PageHeader, Panel, StatusBanner } from '@/components/Primitives';
import {
	enableTrafficConfig,
	ensureLlmFrontendDefaults,
	startupLlmConfig,
	startupMcpConfig,
	usesUiGateways
} from '@/config';
import { refreshBaseCostsAndConfigure } from '@/costs';
import {
	useEffectiveGatewayConfig,
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
	const useGateways = usesUiGateways(trafficData.data ?? config.data);
	const [port, setPort] = useState(() => String(defaultSurfacePort(props.surface)));

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
			await update.mutateAsync(next => {
				if (props.surface === 'llm') {
					next.llm = next.llm ?? startupLlmConfig(next, parsePort(port, 4000));
					ensureLlmFrontendDefaults(next);
				} else if (props.surface === 'mcp') {
					next.mcp =
						next.mcp ?? startupMcpConfig(next, parsePort(port, defaultSurfacePort(props.surface)));
				} else {
					enableTrafficConfig(next, parsePort(port, defaultSurfacePort(props.surface)));
				}
			});
			void navigate({ to: surface.destination });
			if (props.surface === 'llm') {
				void refreshBaseCostsAndConfigure(update).catch(() => undefined);
			}
		} catch {
			// useUpdateConfig exposes the save error through update.isError.
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
			{update.isError ? (
				<StatusBanner state="bad" title={tr('copy.saveFailed')}>
					{update.error.message}
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

				{!enabled && !useGateways && (props.surface === 'llm' || props.surface === 'mcp') ? (
					<details className="schema-details">
						<summary>{tr('copy.advanced')}</summary>
						<Field label={tr('copy.port')}>
							<input
								value={port}
								inputMode="numeric"
								onChange={event => setPort(event.target.value)}
								placeholder={String(defaultSurfacePort(props.surface))}
							/>
						</Field>
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
							disabled={loading || update.isPending}
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

function parsePort(value: string, fallback: number) {
	const parsed = Number.parseInt(value, 10);
	return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function defaultSurfacePort(surface: SurfaceKind) {
	if (surface === 'llm') return 4000;
	if (surface === 'traffic') return 8080;
	return 3000;
}
