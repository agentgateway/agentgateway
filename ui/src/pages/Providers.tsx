import { Button, Input, Table, Tag } from 'antd';
import { Bot, Pencil, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';

import { ConfigDiffSaveActions } from '@/components/ConfigDiffDrawer';
import { LostFieldsWarning, useMergedEdits } from '@/components/EditorContracts';
import {
	ButtonLink,
	ConfirmDialog,
	Drawer,
	EmptyState,
	Field,
	PageHeader,
	Panel,
	StatusBanner,
	Tooltip,
	tablePagination
} from '@/components/Primitives';
import { ProviderIcon } from '@/components/ProviderIcon';
import {
	invalidProviderApiKey,
	isDatabaseConfigResource,
	makeEmptyLlmProvider,
	providerDisplayName,
	providerLabel,
	upsertLlmProvider
} from '@/config';
import { useStickyQueryParam } from '@/drawerRouteState';
import {
	resetFinishedSaves,
	useDeleteConfigResource,
	useLlmConfigData,
	useUpsertConfigResource
} from '@/hooks';
import { ProviderConfigEditor } from '@/pages/models/ProviderConfigEditor';
import { cleanEmpty } from '@/policies/policyUtils';
import { type SchemaHelp, useSchemaHelp } from '@/schemaHelp';
import type { GatewayConfig, LlmModel, LlmProvider, ProviderName } from '@/types';

export function ProvidersPage() {
	const { config, hybrid, resources, models, providers, isLoading, error } = useLlmConfigData();
	const upsertResource = useUpsertConfigResource();
	const deleteResource = useDeleteConfigResource();
	const help = useSchemaHelp();
	const [editing, setEditing] = useState<{
		previousName?: string;
		provider: LlmProvider;
	} | null>(null);
	const [deletingProvider, setDeletingProvider] = useState<string | null>(null);
	const [providerDrawer, setProviderDrawer] = useStickyQueryParam('provider');
	const linkedProvider =
		providerDrawer && providerDrawer !== 'new'
			? providers.find(provider => provider.name === providerDrawer)
			: null;
	const activeEditing =
		editing ??
		(providerDrawer === 'new'
			? { provider: makeEmptyLlmProvider() }
			: linkedProvider
				? {
						previousName: linkedProvider.name,
						provider: structuredClone(linkedProvider)
					}
				: null);
	const editingDatabaseProvider = Boolean(
		hybrid &&
			activeEditing &&
			(!activeEditing.previousName ||
				isDatabaseConfigResource(resources, 'llm.provider', activeEditing.previousName))
	);

	function openNewProvider() {
		resetSaves();
		setEditing(null);
		setProviderDrawer('new');
	}

	function openEditProvider(provider: LlmProvider) {
		resetSaves();
		setEditing(null);
		setProviderDrawer(provider.name);
	}

	function closeProviderEditor() {
		resetSaves();
		setEditing(null);
		setProviderDrawer(null, 'replace');
	}

	const saving = upsertResource.isPending || deleteResource.isPending;
	const saveError = upsertResource.error?.message ?? deleteResource.error?.message ?? null;
	const saved = upsertResource.isSuccess || deleteResource.isSuccess;

	function resetSaves() {
		resetFinishedSaves(upsertResource, deleteResource);
	}

	function databaseBacked(provider: LlmProvider) {
		return isDatabaseConfigResource(resources, 'llm.provider', provider.name);
	}

	function saveProvider(provider: LlmProvider, previousName?: string) {
		upsertResource.mutate(
			{ kind: 'llm.provider', value: provider, previousId: previousName },
			{ onSuccess: closeProviderEditor }
		);
	}

	function deleteProvider(name: string) {
		deleteResource.mutate(
			{ kind: 'llm.provider', id: name },
			{
				onSuccess: () => setDeletingProvider(null)
			}
		);
	}

	return (
		<div className="page-stack">
			<PageHeader
				title="LLM Providers"
				description="Define reusable provider credentials and connection settings for models."
				actions={
					<Button type="primary" icon={<Plus size={16} />} onClick={openNewProvider}>
						Add provider
					</Button>
				}
			/>

			{saveError && !activeEditing ? (
				<StatusBanner state="bad" title="Save failed">
					{saveError}
				</StatusBanner>
			) : null}
			{saved ? <StatusBanner state="ok" title="Configuration saved" /> : null}

			<Panel>
				{isLoading ? (
					<StatusBanner state="loading" title="Loading providers" />
				) : error ? (
					<StatusBanner state="bad" title="Configuration API unavailable">
						{error.message}
					</StatusBanner>
				) : providers.length === 0 ? (
					<EmptyState
						title="No shared providers configured"
						description="Add a provider when multiple models should share the same credentials or upstream connection settings."
						action={
							<Button type="primary" icon={<Plus size={16} />} onClick={openNewProvider}>
								Add provider
							</Button>
						}
					/>
				) : (
					<div className="table-wrap">
						<Table<LlmProvider>
							size="middle"
							rowKey="name"
							dataSource={providers}
							pagination={tablePagination}
							columns={[
								{ title: 'Name', dataIndex: 'name', className: 'strong' },
								...(hybrid
									? [
											{
												title: 'Source',
												key: 'source',
												render: (_: unknown, provider: LlmProvider) => (
													<Tag>{databaseBacked(provider) ? 'Database' : 'File'}</Tag>
												)
											}
										]
									: []),
								{
									title: 'Provider',
									key: 'provider',
									render: (_, provider) => (
										<ProviderBadge provider={providerLabel(provider.provider) as ProviderName} />
									)
								},
								{
									title: 'Upstream model',
									key: 'model',
									render: (_, provider) => provider.params?.model || 'Incoming model'
								},
								{
									title: 'Used by',
									key: 'usage',
									render: (_, provider) => {
										const usage = providerUsage(provider.name, models).length;
										return usage ? (
											<Tag color="success">
												{usage} {usage === 1 ? 'model' : 'models'}
											</Tag>
										) : (
											<Tag>unused</Tag>
										);
									}
								},
								{
									key: 'actions',
									align: 'right',
									onHeaderCell: () => ({ 'aria-label': 'Actions' }),
									render: (_, provider) => {
										const used = providerUsage(provider.name, models).length > 0;
										const fileOwned = hybrid && !databaseBacked(provider);
										return (
											<div className="row-actions">
												<Tooltip content="Add model using this provider">
													<ButtonLink
														color="default"
														variant="outlined"
														aria-label="Add model using provider"
														icon={<Bot size={16} />}
														to="/llm/models"
														search={{ provider: provider.name }}
													/>
												</Tooltip>
												<Tooltip content="Edit provider">
													<Button
														aria-label="Edit provider"
														icon={<Pencil size={16} />}
														onClick={() => openEditProvider(provider)}
													/>
												</Tooltip>
												<Tooltip
													content={
														used
															? 'Provider is referenced by models'
															: fileOwned
																? 'File-owned providers cannot be deleted here'
																: 'Delete provider'
													}
												>
													<Button
														danger
														aria-label="Delete provider"
														icon={<Trash2 size={16} />}
														disabled={used || saving || fileOwned}
														onClick={() => setDeletingProvider(provider.name)}
													/>
												</Tooltip>
											</div>
										);
									}
								}
							]}
						/>
					</div>
				)}
			</Panel>

			{activeEditing ? (
				<ProviderEditor
					key={activeEditing.previousName ?? 'new'}
					initial={activeEditing.provider}
					config={config.data}
					databaseBacked={editingDatabaseProvider}
					previousName={activeEditing.previousName}
					help={help}
					saving={saving}
					saveError={upsertResource.error?.message}
					onCancel={closeProviderEditor}
					onSave={saveProvider}
				/>
			) : null}
			{deletingProvider ? (
				<ConfirmDialog
					title="Delete provider?"
					destructive
					confirmLabel="Delete provider"
					confirmDisabled={saving}
					onCancel={() => setDeletingProvider(null)}
					onConfirm={() => deleteProvider(deletingProvider)}
				>
					<p>
						Delete <strong>{deletingProvider}</strong>? This cannot be undone.
					</p>
				</ConfirmDialog>
			) : null}
		</div>
	);
}

function ProviderEditor(props: {
	initial: LlmProvider;
	config?: GatewayConfig;
	databaseBacked: boolean;
	previousName?: string;
	help: SchemaHelp;
	saving: boolean;
	saveError?: string;
	onCancel: () => void;
	onSave: (provider: LlmProvider, previousName?: string) => void;
}) {
	const [provider, setProvider] = useState<LlmProvider>(props.initial);
	const [initialDraft] = useState(() => JSON.stringify(props.initial));
	const [saveAttempted, setSaveAttempted] = useState(false);
	const merged = useMergedEdits(
		props.previousName ? props.initial : undefined,
		cleanEmpty(provider) as LlmProvider | undefined,
		['$defs', 'LocalLLMProvider']
	);
	const preview = merged.value;
	const invalidApiKey = invalidProviderApiKey(provider.params?.apiKey);
	const providerApiKeyError =
		saveAttempted && invalidApiKey ? 'Enter a value, or choose Unset.' : null;

	function save() {
		setSaveAttempted(true);
		if (invalidApiKey) return;
		props.onSave(preview ?? provider, props.previousName);
	}

	function validateBeforeDiff() {
		setSaveAttempted(true);
		if (!provider.name.trim()) return false;
		if (invalidApiKey) return false;
		return true;
	}

	return (
		<Drawer
			title={props.previousName ? 'Edit provider' : 'Add provider'}
			onClose={props.onCancel}
			dirty={JSON.stringify(provider) !== initialDraft}
			saving={props.saving}
			error={props.saveError}
			raw={{
				schemaPath: ['$defs', 'LocalLLMProvider'],
				value: () => preview ?? provider,
				onChange: value => {
					setProvider(value as LlmProvider);
					merged.reset(value);
				},
				empty: makeEmptyLlmProvider()
			}}
			footer={requestClose => (
				<ConfigDiffSaveActions
					config={props.config}
					resourceDiff={
						props.databaseBacked
							? {
									original: props.previousName ? props.initial : {},
									modified: preview ?? {}
								}
							: undefined
					}
					diffTitle={props.databaseBacked ? 'Provider resource diff' : 'Provider config diff'}
					saveLabel="Save provider"
					saving={props.saving}
					onCancel={requestClose}
					onSave={save}
					beforeDiff={validateBeforeDiff}
					applyDiff={next => upsertLlmProvider(next, preview ?? provider, props.previousName)}
				/>
			)}
		>
			<LostFieldsWarning lost={merged.lost} />
			<div className="form-grid">
				<Field
					label="Provider name"
					path="name"
					tooltip={props.help.field<LlmProvider>(
						'LocalLLMProvider',
						'name',
						'Models reference this name from their provider field.'
					)}
				>
					<Input
						value={provider.name}
						onChange={event => setProvider({ ...provider, name: event.target.value })}
						placeholder="openai-prod"
					/>
				</Field>
			</div>

			<ProviderConfigEditor
				provider={provider.provider}
				params={provider.params}
				auth={provider.defaults?.auth}
				help={props.help}
				apiKeyError={providerApiKeyError}
				onProviderChange={(nextProvider, params) =>
					setProvider(current => ({
						...current,
						provider: nextProvider,
						params
					}))
				}
				onParamsChange={params => setProvider(current => ({ ...current, params }))}
				onAuthChange={auth =>
					setProvider(current => ({
						...current,
						defaults: auth
							? { ...(current.defaults ?? {}), auth }
							: removeProviderAuth(current.defaults)
					}))
				}
			/>
		</Drawer>
	);
}

function removeProviderAuth(defaults: LlmProvider['defaults']) {
	if (!defaults) return null;
	const next = { ...defaults, auth: null };
	return Object.values(next).some(value => value !== null && value !== undefined) ? next : null;
}

function ProviderBadge(props: { provider: ProviderName }) {
	return (
		<Tag className="provider-badge" icon={<ProviderIcon provider={props.provider} />}>
			{providerDisplayName(props.provider)}
		</Tag>
	);
}

function providerUsage(providerName: string, models: LlmModel[]) {
	return models.filter(
		model =>
			typeof model.provider === 'object' &&
			'reference' in model.provider &&
			model.provider.reference === providerName
	);
}
