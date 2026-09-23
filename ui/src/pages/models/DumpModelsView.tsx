import { Eye } from 'lucide-react';

import { Drawer, EmptyState, Panel, Tooltip, YamlBlock } from '@/components/Primitives';
import { useStickyQueryParam } from '@/drawerRouteState';
import type { DumpModel, VirtualModelRouting } from '@/gateway-admin';
import { tr } from '@/i18n';
import { routeBackendLabel } from '@/pages/traffic/TrafficConfigDumpPanel';

export function DumpModelsView(props: { models: DumpModel[] }) {
	const [selectedKey, setSelectedKey] = useStickyQueryParam('model');
	const selectedModel = props.models.find(model => model.key === selectedKey);

	return (
		<>
			<Panel>
				{!props.models.length ? (
					<EmptyState
						title={tr('copy.noModels')}
						description={tr('copy.noModelsInActiveGatewayDump')}
					/>
				) : (
					<div className="table-wrap">
						<table className="dump-models-table">
							<thead>
								<tr>
									<th>{tr('copy.name')}</th>
									<th>{tr('copy.type')}</th>
									<th>{tr('copy.visibility')}</th>
									<th>{tr('copy.target')}</th>
									<th>{tr('copy.listener')}</th>
									<th aria-label={tr('copy.actions')} />
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
												<Tooltip content={tr('copy.viewModel')}>
													<button
														className="icon-button"
														type="button"
														aria-label={tr('copy.viewValue', model.name)}
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
							<span>{tr('copy.visibility')}</span>
							<strong>{modelVisibilityLabel(selectedModel)}</strong>
						</div>
						<div>
							<span>{tr('copy.target')}</span>
							<strong>{modelTarget(selectedModel).summary}</strong>
						</div>
						<div>
							<span>{tr('copy.listener')}</span>
							<strong>{selectedModel.listenerKey}</strong>
						</div>
					</div>
					<span className="field-label">{tr('copy.modelYaml')}</span>
					<YamlBlock value={selectedModel} />
				</Drawer>
			) : null}
		</>
	);
}

function modelTypeLabel(model: DumpModel) {
	return 'concrete' in model.kind ? tr('copy.concrete') : tr('copy.virtualModel');
}

function modelVisibilityLabel(model: DumpModel) {
	if (!('concrete' in model.kind)) return '—';
	return model.kind.concrete.visibility === 'public'
		? tr('copy.publicModelVisibility')
		: tr('copy.internalModelVisibility');
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
			summary: tr('copy.valueWeightedTargets', { count }),
			detail: routing.weighted
				.map(
					target =>
						`${target.model} (${target.weight})${target.invalid ? ` ${tr('copy.invalid')}` : ''}`
				)
				.join(', ')
		};
	}
	if ('conditional' in routing) {
		const rules = routing.conditional.filter(target => target.when?.trim()).length;
		const hasFallback = routing.conditional.some(target => !target.when?.trim());
		return {
			summary: hasFallback
				? tr('copy.valueRulesWithFallback', { count: rules })
				: tr('copy.valueRules', { count: rules }),
			detail: routing.conditional
				.map(target => `${target.model}${target.invalid ? ` ${tr('copy.invalid')}` : ''}`)
				.join(', ')
		};
	}
	return { summary: tr('copy.failover'), detail: routeBackendLabel(routing.failover.backend) };
}
