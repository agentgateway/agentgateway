import { Button, Table, Tag } from 'antd';
import { Eye } from 'lucide-react';

import {
	Drawer,
	EmptyState,
	Panel,
	Tooltip,
	tablePagination,
	YamlBlock
} from '@/components/Primitives';
import { useStickyQueryParam } from '@/drawerRouteState';
import type { DumpModel, VirtualModelRouting } from '@/gateway-admin';
import { routeBackendLabel } from '@/pages/traffic/TrafficConfigDumpPanel';

export function DumpModelsView(props: { models: DumpModel[] }) {
	const [selectedKey, setSelectedKey] = useStickyQueryParam('model');
	const selectedModel = props.models.find(model => model.key === selectedKey);

	return (
		<>
			<Panel>
				{!props.models.length ? (
					<EmptyState
						title="No models"
						description="No models are present in the active gateway dump."
					/>
				) : (
					<div className="table-wrap">
						<Table
							size="middle"
							className="dump-models-table"
							rowKey="key"
							dataSource={props.models}
							pagination={tablePagination}
							columns={[
								{
									title: 'Name',
									key: 'name',
									render: (_, model) => (
										<div className="resource-name-cell">
											<strong>{model.name}</strong>
											<small>{model.key}</small>
										</div>
									)
								},
								{
									title: 'Type',
									key: 'type',
									render: (_, model) => <Tag>{modelTypeLabel(model)}</Tag>
								},
								{
									title: 'Visibility',
									key: 'visibility',
									render: (_, model) => modelVisibilityLabel(model)
								},
								{
									title: 'Target',
									key: 'target',
									render: (_, model) => {
										const target = modelTarget(model);
										return (
											<div className="resource-name-cell">
												<strong>{target.summary}</strong>
												{target.detail ? <small>{target.detail}</small> : null}
											</div>
										);
									}
								},
								{ title: 'Listener', dataIndex: 'listenerKey' },
								{
									key: 'actions',
									onHeaderCell: () => ({ 'aria-label': 'Actions' }),
									align: 'right',
									render: (_, model) => (
										<Tooltip content="View model">
											<Button
												aria-label={`View ${model.name}`}
												icon={<Eye size={16} />}
												onClick={() => setSelectedKey(model.key)}
											/>
										</Tooltip>
									)
								}
							]}
						/>
					</div>
				)}
			</Panel>

			{selectedModel ? (
				<Drawer
					title={selectedModel.name}
					headerActions={<Tag>{modelTypeLabel(selectedModel)}</Tag>}
					onClose={() => setSelectedKey(null)}
				>
					<div className="drawer-summary-list">
						<div>
							<span>Visibility</span>
							<strong>{modelVisibilityLabel(selectedModel)}</strong>
						</div>
						<div>
							<span>Target</span>
							<strong>{modelTarget(selectedModel).summary}</strong>
						</div>
						<div>
							<span>Listener</span>
							<strong>{selectedModel.listenerKey}</strong>
						</div>
					</div>
					<span className="field-label">Model YAML</span>
					<YamlBlock value={selectedModel} />
				</Drawer>
			) : null}
		</>
	);
}

function modelTypeLabel(model: DumpModel) {
	return 'concrete' in model.kind ? 'Concrete' : 'Virtual';
}

function modelVisibilityLabel(model: DumpModel) {
	return 'concrete' in model.kind ? model.kind.concrete.visibility : '—';
}

function modelTarget(model: DumpModel): { summary: string; detail?: string } {
	if ('concrete' in model.kind) {
		return { summary: routeBackendLabel(model.kind.concrete.backend) };
	}
	return virtualRoutingTarget(model.kind.virtual.routing);
}

function virtualRoutingTarget(routing: VirtualModelRouting): { summary: string; detail?: string } {
	if ('weighted' in routing) {
		const count = routing.weighted.length;
		return {
			summary: `${count} weighted ${count === 1 ? 'target' : 'targets'}`,
			detail: routing.weighted
				.map(target => `${target.model} (${target.weight})${target.invalid ? ' invalid' : ''}`)
				.join(', ')
		};
	}
	if ('conditional' in routing) {
		const rules = routing.conditional.filter(target => target.when?.trim()).length;
		const hasFallback = routing.conditional.some(target => !target.when?.trim());
		return {
			summary: `${rules} ${rules === 1 ? 'rule' : 'rules'}${hasFallback ? ', fallback' : ''}`,
			detail: routing.conditional
				.map(target => `${target.model}${target.invalid ? ' invalid' : ''}`)
				.join(', ')
		};
	}
	if ('callout' in routing) {
		const { failureMode } = routing.callout;
		return {
			summary: 'Callout',
			detail: typeof failureMode === 'object' ? `fallback ${failureMode.fallback}` : undefined
		};
	}
	return { summary: 'Failover', detail: routeBackendLabel(routing.failover.backend) };
}
