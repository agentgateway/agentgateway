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
import type { GatewayConfig } from '@/types';

type SurfaceKind = 'llm' | 'mcp' | 'traffic';

const surfaceConfig: Record<
	SurfaceKind,
	{
		title: string;
		description: string;
		icon: typeof Bot;
		enabled: (config: GatewayConfig | undefined) => boolean;
		destination: string;
		destinationLabel: string;
	}
> = {
	llm: {
		title: 'Enable LLM',
		description:
			'Create the LLM configuration section so models, providers, keys, guardrails, logs, and playground tools can be configured.',
		icon: Bot,
		enabled: config => Boolean(config?.llm),
		destination: '/llm/models',
		destinationLabel: 'Continue to models'
	},
	mcp: {
		title: 'Enable MCP',
		description:
			'Create the MCP configuration section so servers and MCP playground tools can be configured.',
		icon: Server,
		enabled: config => Boolean(config?.mcp),
		destination: '/mcp/servers',
		destinationLabel: 'Continue to servers'
	},
	traffic: {
		title: 'Enable Traffic',
		description:
			'Create the traffic configuration section so HTTP gateways, routes, backends, and policies can be configured.',
		icon: Network,
		enabled: config =>
			Boolean(config && ('gateways' in config || 'routes' in config || 'binds' in config)),
		destination: '/traffic/gateways',
		destinationLabel: 'Continue to gateways'
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
				<StatusBanner state="loading" title={`Opening ${surface.destinationLabel.toLowerCase()}`} />
			</div>
		);
	}

	return (
		<div className="page-stack">
			<PageHeader title={surface.title} description={surface.description} />

			{loading ? <StatusBanner state="loading" title="Loading gateway configuration" /> : null}
			{configError ? (
				<StatusBanner state="bad" title="Configuration API unavailable">
					{configError.message}
				</StatusBanner>
			) : null}
			{enableSurface.isError || update.isError ? (
				<StatusBanner state="bad" title="Save failed">
					{enableSurface.error?.message ?? update.error?.message}
				</StatusBanner>
			) : null}

			<Panel className="surface-enable-panel">
				<div className="surface-enable-heading">
					<span className="policy-form-section-icon">
						<Icon size={18} />
					</span>
					<div>
						<h3>
							{enabled ? `${surface.title.replace('Enable ', '')} is enabled` : surface.title}
						</h3>
						<p>
							{enabled
								? 'The top-level configuration section already exists.'
								: surface.description}
						</p>
					</div>
				</div>

				{!enabled && (props.surface === 'llm' || props.surface === 'mcp') ? (
					<details className="schema-details">
						<summary>Advanced</summary>
						<FieldGroup label="Gateway">
							<Dropdown
								ariaLabel="Gateway"
								value={gateway}
								onChange={setGateway}
								options={[
									{
										value: '',
										label: `Automatic (${Array.isArray(defaultGateways) ? defaultGateways.join(', ') : defaultGateways})`,
										description: options.length
											? 'Use the configured gateway.'
											: 'Create a default gateway.'
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
							Enable
						</button>
					)}
					<Link className="button" to="/">
						Back to home
					</Link>
				</div>
			</Panel>
		</div>
	);
}
