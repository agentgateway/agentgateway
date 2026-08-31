import { useNavigate } from '@tanstack/react-router';
import { Clipboard, Download, FileText, RotateCcw, Save, Trash2 } from 'lucide-react';
import { Fragment, lazy, Suspense, useEffect, useMemo, useRef, useState } from 'react';

import type { ConfigResource } from '@/api/configResourcesApi';
import { ConfigDiffDrawer, ConfigSaveButton } from '@/components/ConfigDiffDrawer';
import {
	ConfirmDialog,
	JsonBlock,
	PageHeader,
	Panel,
	StatusBanner,
	Tooltip
} from '@/components/Primitives';
import { validateGatewayConfig } from '@/configValidation';
import { maskKey } from '@/credentialDisplay';
import {
	useConfigDumpMode,
	useConfigResources,
	useDeleteConfigResource,
	useHybridFileWriteOverrideKeys,
	useRawGatewayConfig,
	useRuntimeInfo,
	useUpdateConfig
} from '@/hooks';
import { currentLanguage, tr } from '@/i18n';
import { parseYamlText, toYamlText } from '@/policies/policyUtils';
import type { GatewayConfig } from '@/types';

const LazyRawConfigEditor = lazy(() =>
	import('@/components/RawConfigEditor').then(module => ({
		default: module.RawConfigEditor
	}))
);

export function RawConfigPage() {
	const mode = useConfigDumpMode();
	const navigate = useNavigate();

	useEffect(() => {
		if (mode.data?.mode === 'dump') void navigate({ to: '/' });
	}, [mode.data?.mode, navigate]);

	if (mode.isLoading) {
		return (
			<div className="page-stack">
				<StatusBanner state="loading" title={tr('copy.detectingConfigurationMode')} />
			</div>
		);
	}
	if (mode.data?.mode === 'dump') return null;
	return <RawConfigEditorPage />;
}

function RawConfigEditorPage() {
	const config = useRawGatewayConfig();
	const runtime = useRuntimeInfo();
	const hybrid = runtime.data?.ui.configStoreMode === 'hybrid';
	const resources = useConfigResources({ enabled: hybrid });
	const [view, setView] = useState<'file' | 'database'>('file');
	const update = useUpdateConfig();
	const initialText = useMemo(() => (config.data ? toYamlText(config.data) : ''), [config.data]);
	const [text, setText] = useState(initialText);
	const [error, setError] = useState<string | null>(null);
	const [savedText, setSavedText] = useState<string | null>(null);
	const [diffOpen, setDiffOpen] = useState(false);
	const previousInitialText = useRef(initialText);
	const dirty = text !== initialText;
	const showSaved = Boolean(savedText && text === savedText && initialText === savedText);

	useEffect(() => {
		if (previousInitialText.current !== initialText) {
			if (!text || text === previousInitialText.current) setText(initialText);
			previousInitialText.current = initialText;
		}
	}, [initialText, text]);

	function updateText(next: string) {
		setText(next);
		setError(null);
		setSavedText(null);
		setDiffOpen(false);
		update.reset();
	}

	async function save() {
		setError(null);
		setSavedText(null);
		try {
			const parsed = parseYamlText(text);
			if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
				throw new Error(tr('copy.configurationMustBeAYamlObject'));
			}
			await validateGatewayConfig(parsed as GatewayConfig);
			await update.mutateAsync(() => parsed as GatewayConfig);
			setSavedText(text);
		} catch (err) {
			setError(err instanceof Error ? err.message : tr('copy.invalidConfigurationYaml'));
		}
	}

	return (
		<div className="page-stack">
			<PageHeader
				title={tr('copy.rawConfiguration')}
				description={
					view === 'file'
						? tr('copy.editTheFullGatewayYaml')
						: tr('copy.inspectConfigurationResourcesStoredInTheDatabase')
				}
				actions={
					view === 'file' ? (
						<div className="button-row">
							<button
								className="button"
								type="button"
								disabled={!text}
								onClick={() => void copyConfig(text)}
							>
								<Clipboard size={16} />
								{tr('copy.copy')}
							</button>
							<button
								className="button"
								type="button"
								disabled={!text}
								onClick={() => downloadConfig(text)}
							>
								<Download size={16} />
								{tr('copy.download')}
							</button>
							<button
								className="button"
								type="button"
								disabled={!dirty || update.isPending}
								onClick={() => updateText(initialText)}
							>
								<RotateCcw size={16} />
								{tr('copy.reset')}
							</button>
							<button
								className="button"
								type="button"
								disabled={!dirty || update.isPending}
								onClick={() => setDiffOpen(true)}
							>
								<FileText size={16} />
								{tr('copy.viewDiff')}
							</button>
							<ConfigSaveButton disabled={!dirty || update.isPending} onClick={() => void save()}>
								<Save size={16} />
								{tr('copy.save')}
							</ConfigSaveButton>
						</div>
					) : null
				}
			/>

			{hybrid ? (
				<div className="segmented-control compact raw-config-view-tabs" role="tablist">
					<button
						className={view === 'file' ? 'active' : ''}
						type="button"
						role="tab"
						aria-selected={view === 'file'}
						onClick={() => setView('file')}
					>
						{tr('copy.file')}
					</button>
					<button
						className={view === 'database' ? 'active' : ''}
						type="button"
						role="tab"
						aria-selected={view === 'database'}
						onClick={() => setView('database')}
					>
						{tr('copy.database')}
					</button>
				</div>
			) : null}

			{view === 'file' && config.isError ? (
				<StatusBanner state="bad" title={tr('copy.configurationApiUnavailable')}>
					{config.error.message}
				</StatusBanner>
			) : null}
			{view === 'file' && error ? (
				<StatusBanner state="bad" title={tr('copy.saveFailed')}>
					{error}
				</StatusBanner>
			) : null}
			{view === 'file' && showSaved ? (
				<StatusBanner state="ok" title={tr('copy.configurationSaved')} />
			) : null}

			{view === 'file' ? (
				<Panel>
					<Suspense
						fallback={
							<div className="editor-wrap raw-config-editor loading-panel">
								{tr('copy.loadingEditor')}
							</div>
						}
					>
						<LazyRawConfigEditor
							invalid={Boolean(error)}
							value={text}
							onChange={updateText}
							onSave={() => void save()}
						/>
					</Suspense>
				</Panel>
			) : (
				<DatabaseResourcesPanel
					loading={resources.isLoading}
					error={resources.error?.message}
					resources={resources.data?.resources ?? []}
				/>
			)}
			{view === 'file' && diffOpen ? (
				<ConfigDiffDrawer
					title={tr('copy.rawConfigurationDiff')}
					original={initialText}
					modified={text}
					saving={update.isPending}
					onClose={() => setDiffOpen(false)}
					onSave={() => void save()}
				/>
			) : null}
		</div>
	);
}

function DatabaseResourcesPanel(props: {
	loading: boolean;
	error?: string;
	resources: ConfigResource[];
}) {
	const [expanded, setExpanded] = useState<string | null>(null);
	const [deleting, setDeleting] = useState<ConfigResource | null>(null);
	const deleteResource = useDeleteConfigResource();
	const dangerousActionsVisible = useHybridFileWriteOverrideKeys();

	useEffect(() => {
		if (!dangerousActionsVisible && deleting) setDeleting(null);
	}, [dangerousActionsVisible, deleting]);

	return (
		<>
			<Panel>
				{deleteResource.isError ? (
					<StatusBanner state="bad" title={tr('copy.deleteFailed')}>
						{deleteResource.error.message}
					</StatusBanner>
				) : null}
				{props.loading ? (
					<StatusBanner state="loading" title={tr('copy.loadingDatabaseResources')} />
				) : props.error ? (
					<StatusBanner state="bad" title={tr('copy.configurationDatabaseUnavailable')}>
						{props.error}
					</StatusBanner>
				) : props.resources.length === 0 ? (
					<StatusBanner state="info" title={tr('copy.noDatabaseResources')} />
				) : (
					<div className="table-wrap">
						<table className="data-table raw-config-resource-table">
							<thead>
								<tr>
									<th>{tr('copy.kind')}</th>
									<th>ID</th>
									<th>{tr('copy.revision')}</th>
									<th>{tr('copy.updated')}</th>
									<th>{tr('schema.value')}</th>
								</tr>
							</thead>
							<tbody>
								{props.resources.map(resource => {
									const key = `${resource.kind}:${resource.id}`;
									const open = expanded === key;
									return (
										<Fragment key={key}>
											<tr>
												<td>
													<code>{resource.kind}</code>
												</td>
												<td>
													<code>{resource.id}</code>
												</td>
												<td>{resource.revision ?? '—'}</td>
												<td>
													{resource.updatedAt
														? new Intl.DateTimeFormat(currentLanguage(), {
																dateStyle: 'medium',
																timeStyle: 'medium'
															}).format(new Date(resource.updatedAt))
														: '—'}
												</td>
												<td>
													<div className="button-row compact">
														<button
															className="button compact"
															type="button"
															aria-expanded={open}
															onClick={() => setExpanded(open ? null : key)}
														>
															{open ? tr('copy.hideJson') : tr('copy.viewJson')}
														</button>
														{dangerousActionsVisible ? (
															<Tooltip content={tr('copy.deleteDatabaseResource')}>
																<button
																	className="icon-button danger"
																	type="button"
																	aria-label={tr('copy.deleteDatabaseResource')}
																	onClick={() => setDeleting(resource)}
																>
																	<Trash2 size={15} />
																</button>
															</Tooltip>
														) : null}
													</div>
												</td>
											</tr>
											{open ? (
												<tr className="raw-config-resource-detail">
													<td colSpan={5}>
														<JsonBlock value={redactedResourceValue(resource)} />
													</td>
												</tr>
											) : null}
										</Fragment>
									);
								})}
							</tbody>
						</table>
					</div>
				)}
			</Panel>
			{deleting ? (
				<ConfirmDialog
					title={tr('copy.deleteDatabaseResourceQuestion')}
					destructive
					confirmLabel={tr('copy.deleteResource')}
					confirmDisabled={deleteResource.isPending || !dangerousActionsVisible}
					onCancel={() => setDeleting(null)}
					onConfirm={() => {
						if (!dangerousActionsVisible) return;
						const key = `${deleting.kind}:${deleting.id}`;
						deleteResource.mutate(
							{ kind: deleting.kind, id: deleting.id },
							{
								onSuccess: () => {
									if (expanded === key) setExpanded(null);
									setDeleting(null);
								}
							}
						);
					}}
				>
					<p>{tr('copy.deleteDatabaseResourceValue', [deleting.kind, deleting.id])}</p>
				</ConfirmDialog>
			) : null}
		</>
	);
}

function redactedResourceValue(resource: ConfigResource) {
	if (resource.kind !== 'llm.apiKey') return resource.value;
	const value = structuredClone(resource.value) as Record<string, unknown>;
	if (typeof value.key === 'string') value.key = maskKey(value.key);
	if (typeof value.keyHash === 'string') value.keyHash = maskKey(value.keyHash);
	return value;
}

async function copyConfig(value: string) {
	await navigator.clipboard.writeText(value);
}

function downloadConfig(value: string) {
	const blob = new Blob([value.endsWith('\n') ? value : `${value}\n`], {
		type: 'application/yaml;charset=utf-8'
	});
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement('a');
	anchor.href = url;
	anchor.download = 'agentgateway-config.yaml';
	anchor.click();
	URL.revokeObjectURL(url);
}
