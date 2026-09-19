import { Eye } from 'lucide-react';

import { Drawer, EmptyState, Panel, Tooltip, YamlBlock } from '@/components/Primitives';
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
						<table className="dump-models-table">
							<thead>
								<tr>
									<th>Name</th>
									<th>Type</th>
									<th>Visibility</th>
									<th>Target</th>
									<th>Listener</th>
									<th aria-label="Actions" />
								</tr>
							</thead>
							<tbody>
								{props.models.map(model => {
									const target = modelTarget(model);
									return (
										<tr key={model.key}>
											<td>
												<div className="resource-name-cell">
													<strong>{model.name}</strong>
													<small>{model.key}</small>
												</div>
											</td>
											<td>
												<span className="badge">{modelTypeLabel(model)}</span>
											</td>
											<td>{modelVisibilityLabel(model)}</td>
											<td>
												<div className="resource-name-cell">
													<strong>{target.summary}</strong>
													{target.detail ? <small>{target.detail}</small> : null}
												</div>
											</td>
											<td>{model.listenerKey}</td>
											<td className="row-actions">
												<Tooltip content="View model">
													<button
														className="icon-button"
														type="button"
														aria-label={`View ${model.name}`}
														onClick={() => setSelectedKey(model.key)}
													>
														<Eye size={16} />
													</button>
												</Tooltip>
											</td>
										</tr>
									);
								})}
							</tbody>
						</table>
					</div>
				)}
			</Panel>

			{selectedModel ? (
				<Drawer
					title={selectedModel.name}
					headerActions={<span className="badge">{modelTypeLabel(selectedModel)}</span>}
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
	return { summary: 'Failover', detail: routeBackendLabel(routing.failover.backend) };
}
