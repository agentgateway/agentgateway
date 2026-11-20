import { Link } from '@tanstack/react-router';
import { Button, Collapse, Tag } from 'antd';
import { Shield } from 'lucide-react';
import type { ComponentType, ReactNode } from 'react';
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';

import type { PolicyResourceKind } from '@/api/configResourcesApi';
import { Dropdown, PageHeader, StatusBanner, YamlBlock } from '@/components/Primitives';
import { ensureLlm, ensureMcpPolicies, mcpPolicies } from '@/config';
import { useStickyQueryParam } from '@/drawerRouteState';
import {
	resetFinishedSaves,
	useDeleteConfigResource,
	useEffectiveGatewayConfig,
	useRawGatewayConfig,
	useRuntimeInfo,
	useUpsertPolicyResource
} from '@/hooks';
import { mcpResourceId, mcpServerEndpoints } from '@/mcp';
import { PolicyDrawer } from '@/policies/PolicyDrawer';
import { policyEnabled, policySummary, titleFromKey } from '@/policies/policyUtils';
import { policyUi } from '@/policies/registry';
import type { PolicyKey } from '@/policies/types';
import { useSchemaHelp } from '@/schemaHelp';
import type { GatewayConfig } from '@/types';

const llmPolicySections: Array<{ title: string; keys: PolicyKey[] }> = [
	{
		title: 'Access',
		keys: [
			'cors',
			'apiKey',
			'basicAuth',
			'jwtAuth',
			'oidc',
			'authorization',
			'extAuthz'
		] as PolicyKey[]
	},
	{ title: 'Safety', keys: ['guardrails'] as PolicyKey[] },
	{
		title: 'Traffic Shaping',
		keys: ['localRateLimit', 'remoteRateLimit'] as PolicyKey[]
	},
	{ title: 'Mutation', keys: ['transformations', 'extProc'] as PolicyKey[] }
];

export function PoliciesPage() {
	return (
		<PolicyCatalogPage
			title="LLM Policies"
			description="Configure top-level behavior that applies before model-specific routing."
			schemaRoot="LocalLLMPolicy"
			resourceKind="llm.policy"
			sections={llmPolicySections}
			yamlDescription="Read-only view of effective LLM policies, including database-backed resources in hybrid mode."
			policies={config => config.data?.llm?.policies as Record<string, unknown> | null | undefined}
			managedLinks={{
				apiKey: { to: '/llm/keys', summary: 'Managed on Virtual API Keys' },
				guardrails: { to: '/llm/guardrails', summary: 'Managed on Guardrails' }
			}}
			onSavePolicy={(next, key, value) => {
				const llm = ensureLlm(next);
				llm.policies ??= {};
				(llm.policies as Record<string, unknown>)[key] = value;
			}}
			onDisablePolicy={(next, key) => {
				const llm = ensureLlm(next);
				if (llm.policies)
					(llm.policies as Record<string, unknown>)[key] =
						key === 'localRateLimit' ? undefined : null;
			}}
		/>
	);
}

const mcpPolicySections: Array<{ title: string; keys: PolicyKey[] }> = [
	{
		title: 'MCP',
		keys: ['mcpAuthentication', 'mcpAuthorization', 'mcpGuardrails'] as PolicyKey[]
	},
	{
		title: 'Access',
		keys: ['authorization', 'cors', 'extAuthz', 'jwtAuth'] as PolicyKey[]
	},
	{
		title: 'Traffic Shaping',
		keys: ['localRateLimit', 'remoteRateLimit'] as PolicyKey[]
	},
	{ title: 'Mutation', keys: ['transformations', 'extProc'] as PolicyKey[] }
];

export function McpPoliciesPage() {
	const config = useEffectiveGatewayConfig();
	const endpoints = mcpServerEndpoints(config.data?.mcp).filter(endpoint => endpoint.endpoint);
	const [endpointParam, setEndpointParam] = useStickyQueryParam('endpoint');
	const endpoint = endpoints.find(item => item.endpoint === endpointParam)?.endpoint;
	return (
		<PolicyCatalogPage
			key={endpoint ?? ''}
			title="MCP Policies"
			description={
				endpoint
					? `Policies for ${endpoint}. They replace shared policies; authorization rules add to them.`
					: endpoints.length
						? 'Policies shared by all MCP endpoints.'
						: 'Configure top-level behavior for MCP gateway traffic.'
			}
			actions={
				endpoints.length ? (
					<div className="policy-scope">
						<span>Applies to</span>
						<Dropdown
							ariaLabel="Applies to"
							value={endpoint ?? ''}
							searchable={endpoints.length > 8}
							options={[
								{ value: '', label: 'All endpoints' },
								...endpoints.map(item => ({
									value: item.endpoint ?? '',
									label: item.endpoint
								}))
							]}
							onChange={value => setEndpointParam(value || null)}
						/>
					</div>
				) : undefined
			}
			schemaRoot="FilterOrPolicy"
			resourceKind="mcp.policy"
			resourceId={key => mcpResourceId(key, endpoint)}
			policyNote={key => {
				const mcp = config.data?.mcp;
				// Authorization rules from both scopes apply; other policies replace the shared one.
				const combines = key === 'authorization' || key === 'mcpAuthorization';
				const has = (scope?: string) => Object.hasOwn(mcpPolicies(mcp, scope) ?? {}, key);
				if (endpoint) {
					if (!has()) return undefined;
					if (!has(endpoint)) return 'shared';
					return combines ? 'adds to shared' : 'overrides shared';
				}
				if (!has()) return undefined;
				const count = endpoints.filter(item => has(item.endpoint)).length;
				if (!count) return undefined;
				return `${count} ${combines ? (count === 1 ? 'addition' : 'additions') : count === 1 ? 'override' : 'overrides'}`;
			}}
			sections={mcpPolicySections}
			policyKeys={mcpPolicySections.flatMap(section => section.keys)}
			yamlDescription={
				endpoint
					? `Read-only view of the ${endpoint} endpoint policies.`
					: 'Read-only view of mcp.policies.'
			}
			policies={config => mcpPolicies(config.data?.mcp, endpoint)}
			onSavePolicy={(next, key, value) => {
				ensureMcpPolicies(next, endpoint)[key] = value;
			}}
			onDisablePolicy={(next, key) => {
				delete ensureMcpPolicies(next, endpoint)[key];
			}}
		/>
	);
}

export function PolicyCatalogPage(props: {
	title: string;
	description: string;
	actions?: ReactNode;
	beforePolicies?: ReactNode;
	schemaRoot: string;
	resourceKind: PolicyResourceKind;
	/** Maps a policy key to its resource ID; defaults to the key. */
	resourceId?: (key: PolicyKey) => string;
	/** A short note shown by the enabled chip, for example how the policy relates to another scope. */
	policyNote?: (key: PolicyKey) => string | undefined;
	sections: Array<{ title: string; keys: PolicyKey[] }>;
	policyKeys?: PolicyKey[];
	yamlDescription: string;
	policies: (
		config: ReturnType<typeof useEffectiveGatewayConfig>
	) => Record<string, unknown> | null | undefined;
	policiesDisabled?: (config: ReturnType<typeof useEffectiveGatewayConfig>) => boolean;
	policiesDisabledReason?: string;
	managedLinks?: Partial<Record<PolicyKey, { to: string; summary: string }>>;
	onSavePolicy: (config: GatewayConfig, key: PolicyKey, value: unknown) => void;
	onDisablePolicy: (config: GatewayConfig, key: PolicyKey) => void;
}) {
	const rawConfig = useRawGatewayConfig();
	const config = useEffectiveGatewayConfig();
	const runtime = useRuntimeInfo();
	const hybrid = runtime.data?.ui.configStoreMode === 'hybrid';
	const upsertPolicy = useUpsertPolicyResource();
	const deleteResource = useDeleteConfigResource();
	const filePolicies = props.policies(rawConfig);
	const policies = props.policies(config);
	const policyDataLoading = config.isLoading || rawConfig.isLoading || runtime.isLoading;
	const policyDataUnavailable =
		policyDataLoading || config.isError || rawConfig.isError || runtime.isError;
	const policiesDisabled = props.policiesDisabled?.(config) ?? false;
	const [selected, setSelected] = useState<PolicyKey | null>(() => policyKeyFromHash());
	const pendingScrollRestore = useRef<{ x: number; y: number } | null>(null);
	const help = useSchemaHelp();
	const policyCatalog = useMemo(() => {
		const schemaKeys = help.objectProperties(['$defs', props.schemaRoot]) as PolicyKey[];
		const keys =
			props.policyKeys ??
			(schemaKeys.length > 0 ? schemaKeys : (Object.keys(policyUi) as PolicyKey[]));
		return keys.map(key => {
			const policyKey = key as PolicyKey;
			const ui = policyUi[policyKey];
			return {
				key: policyKey,
				title: ui?.title ?? titleFromKey(policyKey),
				description:
					help.propertyDescription(props.schemaRoot, [policyKey], 'Configured from schema.') ??
					'Configured from schema.',
				icon: ui?.icon ?? Shield,
				customEditor: ui?.customEditor
			};
		});
	}, [help, props.policyKeys, props.schemaRoot]);
	const selectedMeta = policyCatalog.find(policy => policy.key === selected);
	const selectedFileOwned = Boolean(
		selected && filePolicies && Object.hasOwn(filePolicies, selected)
	);
	const saving = upsertPolicy.isPending || deleteResource.isPending;
	const saveError = upsertPolicy.error?.message ?? deleteResource.error?.message ?? null;

	const policyItems = useMemo(() => {
		return policyCatalog.map(meta => ({
			...meta,
			enabled: policyEnabled(policies, meta.key),
			summary: policySummary(policies, meta.key),
			note: props.policyNote?.(meta.key)
		}));
	}, [policies, policyCatalog, props.policyNote]);
	const groupedPolicyItems = useMemo(() => {
		const byKey = new Map(policyItems.map(policy => [policy.key, policy]));
		const known = new Set(props.sections.flatMap(section => section.keys));
		const sections = props.sections
			.map(section => ({
				...section,
				policies: section.keys
					.map(key => byKey.get(key))
					.filter((policy): policy is NonNullable<typeof policy> => Boolean(policy))
					.sort(
						(a, b) =>
							Number(b.enabled) - Number(a.enabled) ||
							section.keys.indexOf(a.key) - section.keys.indexOf(b.key)
					)
			}))
			.filter(section => section.policies.length > 0);
		const otherPolicies = policyItems
			.filter(policy => !known.has(policy.key))
			.sort((a, b) => Number(b.enabled) - Number(a.enabled) || a.title.localeCompare(b.title));
		return otherPolicies.length
			? [...sections, { title: 'Other', keys: [], policies: otherPolicies }]
			: sections;
	}, [policyItems, props.sections]);

	useEffect(() => {
		function syncSelectedFromUrl() {
			resetFinishedSaves(upsertPolicy, deleteResource);
			setSelected(policyKeyFromHash());
		}
		window.addEventListener('hashchange', syncSelectedFromUrl);
		window.addEventListener('popstate', syncSelectedFromUrl);
		return () => {
			window.removeEventListener('hashchange', syncSelectedFromUrl);
			window.removeEventListener('popstate', syncSelectedFromUrl);
		};
	}, [deleteResource, upsertPolicy]);

	useLayoutEffect(() => {
		if (selected !== null) return;
		const scroll = pendingScrollRestore.current;
		if (!scroll) return;
		pendingScrollRestore.current = null;
		window.scrollTo(scroll.x, scroll.y);
	}, [selected]);

	function openPolicy(policyKey: PolicyKey) {
		resetFinishedSaves(upsertPolicy, deleteResource);
		setSelected(policyKey);
		setPolicyHash(policyKey, 'push');
	}

	function closePolicy() {
		resetFinishedSaves(upsertPolicy, deleteResource);
		pendingScrollRestore.current = { x: window.scrollX, y: window.scrollY };
		setSelected(null);
		setPolicyHash(null, 'replace');
	}

	return (
		<div className="page-stack">
			<PageHeader title={props.title} description={props.description} actions={props.actions} />
			{config.isError || rawConfig.isError || runtime.isError ? (
				<StatusBanner state="bad" title="Configuration API unavailable">
					{config.error?.message ?? rawConfig.error?.message ?? runtime.error?.message}
				</StatusBanner>
			) : null}
			{policyDataLoading ? <StatusBanner state="loading" title="Loading policies" /> : null}
			{saveError && !selected ? (
				<StatusBanner state="bad" title="Save failed">
					{saveError}
				</StatusBanner>
			) : null}
			{props.beforePolicies}
			{policiesDisabled ? (
				<StatusBanner state="warn" title={props.policiesDisabledReason ?? 'Policies disabled'} />
			) : null}

			<div className="policy-section-list">
				{groupedPolicyItems.map(section => (
					<section className="policy-page-section" key={section.title}>
						<h3>{section.title}</h3>
						<div className="policy-page-grid">
							{section.policies.map(policy => (
								<PolicyTile
									key={policy.key}
									policy={policy}
									managedLink={props.managedLinks?.[policy.key]}
									disabled={policiesDisabled || policyDataUnavailable}
									onOpen={openPolicy}
								/>
							))}
						</div>
					</section>
				))}
			</div>

			<Collapse
				ghost
				styles={{
					root: { marginBottom: 8 },
					header: {
						padding: '10px 0 8px',
						color: 'var(--muted)',
						fontSize: 13,
						fontWeight: 700,
						lineHeight: 'normal'
					}
				}}
				items={[
					{
						key: 'yaml',
						label: 'Current top-level policy YAML',
						children: (
							<>
								<p className="muted-copy">{props.yamlDescription}</p>
								<YamlBlock value={policies ?? {}} />
							</>
						)
					}
				]}
			/>

			{selected && selectedMeta && !policiesDisabled && !policyDataUnavailable ? (
				<PolicyDrawer
					key={selected}
					policyKey={selected}
					title={selectedMeta.title}
					customEditor={selectedMeta.customEditor}
					policyValue={policies?.[selected] ?? null}
					policies={policies as Record<string, unknown> | null | undefined}
					help={help}
					schemaRoot={props.schemaRoot}
					saving={saving}
					saveError={saveError}
					config={config.data}
					databaseBacked={hybrid && !selectedFileOwned}
					onClose={closePolicy}
					applySaveDiff={(next, value) => {
						props.onSavePolicy(next, selected, value);
					}}
					onSave={value => {
						upsertPolicy.mutate(
							{ kind: props.resourceKind, id: props.resourceId?.(selected) ?? selected, value },
							{ onSuccess: closePolicy }
						);
					}}
					onDisable={() => {
						deleteResource.mutate(
							{ kind: props.resourceKind, id: props.resourceId?.(selected) ?? selected },
							{ onSuccess: closePolicy }
						);
					}}
				/>
			) : null}
		</div>
	);
}

function PolicyTile(props: {
	policy: {
		key: PolicyKey;
		title: string;
		enabled: boolean;
		summary: string;
		description: string;
		icon: ComponentType<{ size?: number }>;
	};
	managedLink?: { to: string; summary: string };
	disabled?: boolean;
	onOpen: (policyKey: PolicyKey) => void;
}) {
	const className = props.policy.enabled ? 'policy-tile enabled' : 'policy-tile';
	if (props.managedLink) {
		return (
			<Link className={className} to={props.managedLink.to}>
				<PolicyTileContent policy={props.policy} summary={props.managedLink.summary} />
			</Link>
		);
	}
	return (
		<Button
			className={className}
			disabled={props.disabled}
			onClick={() => props.onOpen(props.policy.key)}
		>
			<PolicyTileContent policy={props.policy} />
		</Button>
	);
}

function policyKeyFromHash(): PolicyKey | null {
	const raw = decodeURIComponent(window.location.hash.replace(/^#/, ''));
	if (!raw) return null;
	const policy = raw.startsWith('policy=') ? raw.slice('policy='.length) : raw;
	return policy ? (policy as PolicyKey) : null;
}

function setPolicyHash(policyKey: PolicyKey | null, mode: 'push' | 'replace') {
	const nextUrl = `${window.location.pathname}${window.location.search}${policyKey ? `#${encodeURIComponent(policyKey)}` : ''}`;
	if (nextUrl === `${window.location.pathname}${window.location.search}${window.location.hash}`)
		return;
	const scrollX = window.scrollX;
	const scrollY = window.scrollY;
	if (mode === 'push') {
		History.prototype.pushState.call(window.history, null, '', nextUrl);
	} else {
		History.prototype.replaceState.call(window.history, null, '', nextUrl);
	}
	window.scrollTo(scrollX, scrollY);
}

function PolicyTileContent(props: {
	policy: {
		title: string;
		enabled: boolean;
		summary: string;
		note?: string;
		description: string;
		icon: ComponentType<{ size?: number }>;
	};
	summary?: string;
}) {
	const Icon = props.policy.icon;
	const summary = props.summary ?? props.policy.summary;
	return (
		<>
			<div className="policy-tile-header">
				<span className="policy-icon">
					<Icon size={18} />
				</span>
				<span className="policy-tile-badges">
					{props.policy.note ? <Tag color="purple">{props.policy.note}</Tag> : null}
					{props.policy.enabled || !props.policy.note ? (
						<Tag color={props.policy.enabled ? 'success' : undefined}>
							{props.policy.enabled ? 'enabled' : 'disabled'}
						</Tag>
					) : null}
				</span>
			</div>
			<strong>{props.policy.title}</strong>
			{summary ? <span>{summary}</span> : null}
			<small>{props.policy.description}</small>
		</>
	);
}
