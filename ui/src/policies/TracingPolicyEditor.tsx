import { Gauge, Send, Tags } from 'lucide-react';
import { useState } from 'react';

import { UnsupportedYamlFallback } from '@/components/EditorContracts';
import { EnumSelector } from '@/components/EnumSelector';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { Field, FieldGroup } from '@/components/Primitives';
import type { StringBoolFloat, TracingConfig } from '@/gateway-config';
import { ListEditor } from '@/policies/ListEditor';
import {
	hasUnsupportedTarget,
	KeyValueEditor,
	TargetEditor,
	targetFrom,
	unsupportedTargetLabel
} from '@/policies/PolicyFormControls';
import { PolicySection } from '@/policies/PolicyLayout';
import { cleanEmpty } from '@/policies/policyUtils';
import { ResultingYaml } from '@/policies/ResultingYaml';
import type { SchemaHelp } from '@/schemaHelp';

type Protocol = 'grpc' | 'http';

export const otlpProtocolOptions: Array<{ value: Protocol; label: string }> = [
	{ value: 'grpc', label: 'gRPC' },
	{ value: 'http', label: 'HTTP' }
];

export function TracingPolicyEditor(props: {
	formId?: string;
	tracing: TracingConfig | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (value: TracingConfig) => void;
}) {
	const original = props.tracing;
	const [target, setTarget] = useState(() => targetFrom(original, 'localhost:4317'));
	const [protocol, setProtocol] = useState<Protocol>(original?.protocol ?? 'grpc');
	const [path, setPath] = useState(original?.path ?? '');
	const [randomSampling, setRandomSampling] = useState(samplingText(original?.randomSampling));
	const [clientSampling, setClientSampling] = useState(samplingText(original?.clientSampling));
	const [parentNotSampled, setParentNotSampled] = useState(
		samplingText(original?.parentNotSampled)
	);
	const [filter, setFilter] = useState(original?.filter ?? '');
	const [attributes, setAttributes] = useState(original?.attributes ?? {});
	const [resources, setResources] = useState(original?.resources ?? {});
	const [remove, setRemove] = useState(original?.remove ?? []);

	if (hasUnsupportedTarget(original)) {
		return (
			<UnsupportedYamlFallback
				title="Unsupported target type"
				value={original ?? {}}
				schema={props.help.node(['$defs', 'TracingConfig'])}
				help={props.help}
			>
				This policy uses a {unsupportedTargetLabel(original)} target. The visual editor currently
				supports host targets only.
			</UnsupportedYamlFallback>
		);
	}

	const policy = cleanEmpty({
		...original,
		...target,
		protocol,
		path: protocol === 'http' ? path.trim() : undefined,
		randomSampling: samplingValue(randomSampling),
		clientSampling: samplingValue(clientSampling),
		parentNotSampled: samplingValue(parentNotSampled),
		filter: filter.trim(),
		attributes: Object.keys(attributes).length ? attributes : undefined,
		resources: Object.keys(resources).length ? resources : undefined,
		remove
	}) as TracingConfig;

	return (
		<form
			id={props.formId}
			className="policy-editor-stack"
			onSubmit={event => {
				event.preventDefault();
				props.onSave(policy);
			}}
		>
			<TargetEditor value={target} tooltip="OTLP collector address." onChange={setTarget} />
			<PolicySection
				icon={<Send size={17} />}
				title="Export"
				description="How spans are sent to the collector."
			>
				<div className="form-grid">
					<FieldGroup
						label="Protocol"
						tooltip={props.help.field<TracingConfig>('TracingConfig', 'protocol')}
					>
						<EnumSelector
							ariaLabel="Protocol"
							value={protocol}
							options={otlpProtocolOptions}
							onChange={setProtocol}
						/>
					</FieldGroup>
					{protocol === 'http' ? (
						<Field label="Path" tooltip={props.help.field<TracingConfig>('TracingConfig', 'path')}>
							<input
								value={path}
								onChange={event => setPath(event.target.value)}
								placeholder="/v1/traces"
							/>
						</Field>
					) : null}
				</div>
			</PolicySection>
			<PolicySection
				icon={<Gauge size={17} />}
				title="Sampling"
				description="Each value is a ratio, true/false, or a CEL expression."
			>
				<div className="form-grid">
					<Field
						label="Random sampling"
						tooltip={props.help.field<TracingConfig>('TracingConfig', 'randomSampling')}
					>
						<input
							className="mono-input"
							value={randomSampling}
							onChange={event => setRandomSampling(event.target.value)}
							placeholder="0.1"
						/>
					</Field>
					<Field
						label="Client sampling"
						tooltip={props.help.field<TracingConfig>('TracingConfig', 'clientSampling')}
					>
						<input
							className="mono-input"
							value={clientSampling}
							onChange={event => setClientSampling(event.target.value)}
							placeholder="true"
						/>
					</Field>
					<Field
						label="Parent not sampled"
						tooltip={props.help.field<TracingConfig>('TracingConfig', 'parentNotSampled')}
					>
						<input
							className="mono-input"
							value={parentNotSampled}
							onChange={event => setParentNotSampled(event.target.value)}
							placeholder="false"
						/>
					</Field>
				</div>
				<FieldGroup
					label="Filter"
					tooltip={props.help.field<TracingConfig>('TracingConfig', 'filter')}
				>
					<MiniMonacoEditor
						className="micro"
						language="cel"
						value={filter}
						onChange={setFilter}
						placeholder="response.code >= 500"
					/>
				</FieldGroup>
			</PolicySection>
			<PolicySection
				icon={<Tags size={17} />}
				title="Attributes"
				description="Values are CEL expressions evaluated per request."
			>
				<KeyValueEditor
					label="Span attributes"
					tooltip={props.help.field<TracingConfig>('TracingConfig', 'attributes')}
					values={attributes}
					keyPlaceholder="http.route"
					valuePlaceholder="request.path"
					valueKind="cel"
					onChange={setAttributes}
				/>
				<KeyValueEditor
					label="Resource attributes"
					tooltip={props.help.field<TracingConfig>('TracingConfig', 'resources')}
					values={resources}
					keyPlaceholder="service.name"
					valuePlaceholder={'"agentgateway"'}
					valueKind="cel"
					quickKeys={['service.name', 'deployment.environment']}
					onChange={setResources}
				/>
				<ListEditor
					label="Remove attributes"
					tooltip={props.help.field<TracingConfig>('TracingConfig', 'remove')}
					values={remove}
					placeholder="http.user_agent"
					onChange={setRemove}
				/>
			</PolicySection>
			<ResultingYaml value={policy} />
		</form>
	);
}

function samplingText(value: StringBoolFloat | null | undefined) {
	return value === null || value === undefined ? '' : String(value);
}

function samplingValue(text: string): StringBoolFloat | undefined {
	const value = text.trim();
	if (!value) return undefined;
	if (value === 'true') return true;
	if (value === 'false') return false;
	const number = Number(value);
	return Number.isFinite(number) ? number : value;
}
