import { Link, useNavigate } from '@tanstack/react-router';
import { useEffect, useState } from 'react';

import { gatewayOptions } from '@/components/GatewayBindingEditor';
import { Dropdown, FieldGroup, PageHeader, Panel, StatusBanner } from '@/components/Primitives';
import { startupGatewayRefs } from '@/config';
import {
	useEffectiveGatewayConfig,
	useEnableSurface,
	useMcpConfigData,
	useTrafficConfigData
} from '@/hooks';
import type { GatewayConfig } from '@/types';

type SurfaceKind = 'llm' | 'mcp' | 'traffic';

const surfaceConfig: Record<
	SurfaceKind,
	{
		title: string;
		description: string;
		enabled: (config: GatewayConfig | undefined) => boolean;
		destination: string;
		destinationLabel: string;
	}
> = {
	llm: {
		title: 'Enable LLM',
		description: 'Add LLM settings to the configuration, then set up models.',
		enabled: config => Boolean(config?.llm),
		destination: '/llm/models',
		destinationLabel: 'Continue to models'
	},
	mcp: {
		title: 'Enable MCP',
		description: 'Add MCP settings to the configuration, then connect servers.',
		enabled: config => Boolean(config?.mcp),
		destination: '/mcp/servers',
		destinationLabel: 'Continue to servers'
	},
	traffic: {
		title: 'Enable Traffic',
		description: 'Add traffic settings to the configuration, then set up gateways and routes.',
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
	const enableSurface = useEnableSurface();
	const navigate = useNavigate();
	const surface = surfaceConfig[props.surface];
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
			await enableSurface.mutateAsync({
				surface: props.surface,
				gateway: gateway || undefined
			});
			void navigate({ to: surface.destination });
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
			{enableSurface.isError ? (
				<StatusBanner state="bad" title="Save failed">
					{enableSurface.error?.message}
				</StatusBanner>
			) : null}

			<Panel className="surface-enable-panel">
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
							disabled={loading || enableSurface.isPending}
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
