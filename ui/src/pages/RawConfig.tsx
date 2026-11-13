import { useNavigate } from '@tanstack/react-router';
import { Button, Table, Tabs } from 'antd';
import { Clipboard, Download, FileText, RotateCcw, Save, Trash2 } from 'lucide-react';
import { lazy, Suspense, useEffect, useRef, useState } from 'react';

import type { ConfigResource } from '@/api/configResourcesApi';
import { ConfigDiffDrawer, ConfigSaveButton } from '@/components/ConfigDiffDrawer';
import {
	ConfirmDialog,
	JsonBlock,
	PageHeader,
	Panel,
	StatusBanner,
	Tooltip,
	tablePagination
} from '@/components/Primitives';
import { validateGatewayConfig } from '@/configValidation';
import { maskKey } from '@/credentialDisplay';
import {
	useConfigDumpMode,
	useConfigResources,
	useDeleteConfigResource,
	useHybridFileWriteOverrideKeys,
	useRawConfigText,
	useRuntimeInfo,
	useUpdateRawConfig
} from '@/hooks';
import { detectLineEnding } from '@/lineEnding';
import { parseYamlText } from '@/policies/policyUtils';
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
				<StatusBanner state="loading" title="Detecting configuration mode" />
			</div>
		);
	}
	if (mode.data?.mode === 'dump') return null;
	return <RawConfigEditorPage />;
}

function RawConfigEditorPage() {
	const config = useRawConfigText();
	const runtime = useRuntimeInfo();
	const hybrid = runtime.data?.ui.configStoreMode === 'hybrid';
	const resources = useConfigResources({ enabled: hybrid });
	const [view, setView] = useState<'file' | 'database'>('file');
	const update = useUpdateRawConfig();
	const initialText = config.data ?? '';
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
				throw new Error('Configuration must be a YAML object.');
			}
			await validateGatewayConfig(parsed as GatewayConfig);
			await update.mutateAsync(text);
			setSavedText(text);
		} catch (err) {
			setError(err instanceof Error ? err.message : 'Invalid configuration YAML.');
		}
	}

	return (
		<div className="page-stack">
			<PageHeader
				title="Raw Configuration"
				description={
					view === 'file'
						? 'Edit the full gateway YAML.'
						: 'Inspect configuration resources stored in the database.'
				}
				actions={
					view === 'file' ? (
						<div className="button-row">
							<Button
								icon={<Clipboard size={16} />}
								disabled={!text}
								onClick={() => void copyConfig(text)}
							>
								Copy
							</Button>
							<Button
								icon={<Download size={16} />}
								disabled={!text}
								onClick={() => downloadConfig(text)}
							>
								Download
							</Button>
							<Button
								icon={<RotateCcw size={16} />}
								disabled={!dirty || update.isPending}
								onClick={() => updateText(initialText)}
							>
								Reset
							</Button>
							<Button
								icon={<FileText size={16} />}
								disabled={!dirty || update.isPending}
								onClick={() => setDiffOpen(true)}
							>
								View diff
							</Button>
							<ConfigSaveButton disabled={!dirty || update.isPending} onClick={() => void save()}>
								<Save size={16} />
								Save
							</ConfigSaveButton>
						</div>
					) : null
				}
			/>

			{hybrid ? (
				<Tabs
					className="raw-config-view-tabs"
					activeKey={view}
					onChange={key => setView(key as 'file' | 'database')}
					items={[
						{ key: 'file', label: 'File' },
						{ key: 'database', label: 'Database' }
					]}
				/>
			) : null}

			{view === 'file' && config.isError ? (
				<StatusBanner state="bad" title="Configuration API unavailable">
					{config.error.message}
				</StatusBanner>
			) : null}
			{view === 'file' && error ? (
				<StatusBanner state="bad" title="Save failed">
					{error}
				</StatusBanner>
			) : null}
			{view === 'file' && showSaved ? (
				<StatusBanner state="ok" title="Configuration saved" />
			) : null}

			{view === 'file' ? (
				<Panel>
					<Suspense
						fallback={
							<div className="editor-wrap raw-config-editor loading-panel">Loading editor...</div>
						}
					>
						<LazyRawConfigEditor
							invalid={Boolean(error)}
							lineEnding={detectLineEnding(initialText)}
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
					title="Raw configuration diff"
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
					<StatusBanner state="bad" title="Delete failed">
						{deleteResource.error.message}
					</StatusBanner>
				) : null}
				{props.loading ? (
					<StatusBanner state="loading" title="Loading database resources" />
				) : props.error ? (
					<StatusBanner state="bad" title="Configuration database unavailable">
						{props.error}
					</StatusBanner>
				) : props.resources.length === 0 ? (
					<StatusBanner state="info" title="No database resources" />
				) : (
					<Table
						className="raw-config-resource-table"
						size="small"
						rowKey={resourceKey}
						dataSource={props.resources}
						pagination={tablePagination}
						expandable={{
							showExpandColumn: false,
							expandedRowKeys: expanded ? [expanded] : [],
							expandedRowRender: resource => <JsonBlock value={redactedResourceValue(resource)} />
						}}
						columns={[
							{ title: 'Kind', dataIndex: 'kind', render: (kind: string) => <code>{kind}</code> },
							{ title: 'ID', dataIndex: 'id', render: (id: string) => <code>{id}</code> },
							{
								title: 'Revision',
								dataIndex: 'revision',
								render: (revision?: number) => revision ?? '—'
							},
							{
								title: 'Updated',
								dataIndex: 'updatedAt',
								render: (updatedAt?: string) =>
									updatedAt ? new Date(updatedAt).toLocaleString() : '—'
							},
							{
								title: 'Value',
								key: 'value',
								render: (_, resource) => {
									const key = resourceKey(resource);
									const open = expanded === key;
									return (
										<div className="button-row compact">
											<Button
												size="small"
												aria-expanded={open}
												onClick={() => setExpanded(open ? null : key)}
											>
												{open ? 'Hide JSON' : 'View JSON'}
											</Button>
											{dangerousActionsVisible ? (
												<Tooltip content="Delete database resource">
													<Button
														danger
														size="small"
														aria-label="Delete database resource"
														icon={<Trash2 size={15} />}
														onClick={() => setDeleting(resource)}
													/>
												</Tooltip>
											) : null}
										</div>
									);
								}
							}
						]}
					/>
				)}
			</Panel>
			{deleting ? (
				<ConfirmDialog
					title="Delete database resource?"
					destructive
					confirmLabel="Delete resource"
					confirmDisabled={deleteResource.isPending || !dangerousActionsVisible}
					onCancel={() => setDeleting(null)}
					onConfirm={() => {
						if (!dangerousActionsVisible) return;
						const key = resourceKey(deleting);
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
					<p>
						Delete <strong>{deleting.kind}</strong>/<strong>{deleting.id}</strong>? This cannot be
						undone.
					</p>
				</ConfirmDialog>
			) : null}
		</>
	);
}

function resourceKey(resource: ConfigResource) {
	return `${resource.kind}:${resource.id}`;
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
