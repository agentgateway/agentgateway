import { Database, ScrollText, Send } from 'lucide-react';
import { useState } from 'react';

import { UnsupportedYamlFallback } from '@/components/EditorContracts';
import { EnumSelector } from '@/components/EnumSelector';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { Field, FieldGroup } from '@/components/Primitives';
import type { LoggingPolicy, OtlpLoggingConfig } from '@/gateway-config';
import { ListEditor } from '@/policies/ListEditor';
import {
	hasUnsupportedTarget,
	KeyValueEditor,
	targetFrom,
	unsupportedTargetLabel
} from '@/policies/PolicyFormControls';
import { PolicySection } from '@/policies/PolicyLayout';
import { cleanEmpty } from '@/policies/policyUtils';
import { ResultingYaml } from '@/policies/ResultingYaml';
import { otlpProtocolOptions } from '@/policies/TracingPolicyEditor';
import type { SchemaHelp } from '@/schemaHelp';

type Preset = 'default' | 'otel';
type LlmMode = 'default' | 'metadata' | 'full';

export function AccessLogPolicyEditor(props: {
	formId?: string;
	accessLog: LoggingPolicy | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (value: LoggingPolicy) => void;
}) {
	const original = props.accessLog;
	const [preset, setPreset] = useState<Preset>(original?.preset ?? 'default');
	const [filter, setFilter] = useState(original?.filter ?? '');
	const [add, setAdd] = useState(original?.add ?? {});
	const [remove, setRemove] = useState(original?.remove ?? []);
	const [otlpEnabled, setOtlpEnabled] = useState(Boolean(original?.otlp));
	const [otlpHost, setOtlpHost] = useState(() => targetFrom(original?.otlp, 'localhost:4317').host);
	const [otlpProtocol, setOtlpProtocol] = useState(original?.otlp?.protocol ?? 'grpc');
	const [otlpPath, setOtlpPath] = useState(original?.otlp?.path ?? '');
	const [otlpFilter, setOtlpFilter] = useState(original?.otlp?.filter ?? '');
	const [llmMode, setLlmMode] = useState<LlmMode>(original?.database?.llm ?? 'default');
	const [databaseAdd, setDatabaseAdd] = useState(original?.database?.add ?? {});

	if (original?.otlp && hasUnsupportedTarget(original.otlp)) {
		return (
			<UnsupportedYamlFallback
				title="Unsupported OTLP target type"
				value={original}
				schema={props.help.node(['$defs', 'LoggingPolicy'])}
				help={props.help}
			>
				OTLP export uses a {unsupportedTargetLabel(original.otlp)} target. The visual editor
				currently supports host targets only.
			</UnsupportedYamlFallback>
		);
	}

	const policy = cleanEmpty({
		...original,
		preset: preset === 'otel' ? 'otel' : undefined,
		filter: filter.trim(),
		add: Object.keys(add).length ? add : undefined,
		remove,
		otlp: otlpEnabled
			? ({
					...original?.otlp,
					host: otlpHost.trim(),
					protocol: otlpProtocol,
					path: otlpProtocol === 'http' ? otlpPath.trim() : undefined,
					filter: otlpFilter.trim()
				} as OtlpLoggingConfig)
			: undefined,
		database:
			llmMode !== 'default' || Object.keys(databaseAdd).length
				? {
						...original?.database,
						llm: llmMode === 'default' ? undefined : llmMode,
						add: Object.keys(databaseAdd).length ? databaseAdd : undefined
					}
				: undefined
	}) as LoggingPolicy;

	return (
		<form
			id={props.formId}
			className="policy-editor-stack"
			onSubmit={event => {
				event.preventDefault();
				props.onSave(policy);
			}}
		>
			<PolicySection
				icon={<ScrollText size={17} />}
				title="Log fields"
				description="Which requests are logged and what each entry contains."
			>
				<FieldGroup
					label="Stdout preset"
					tooltip={props.help.field<LoggingPolicy>('LoggingPolicy', 'preset')}
				>
					<EnumSelector
						ariaLabel="Stdout preset"
						value={preset}
						options={[
							{ value: 'default', label: 'Default', description: 'Human-oriented fields.' },
							{
								value: 'otel',
								label: 'OpenTelemetry',
								description: 'OTel-aligned HTTP field names.'
							}
						]}
						onChange={setPreset}
					/>
				</FieldGroup>
				<FieldGroup
					label="Filter"
					tooltip={props.help.field<LoggingPolicy>('LoggingPolicy', 'filter')}
				>
					<MiniMonacoEditor
						className="micro"
						language="cel"
						value={filter}
						onChange={setFilter}
						placeholder="response.code >= 400"
					/>
				</FieldGroup>
				<KeyValueEditor
					label="Add fields"
					tooltip={props.help.field<LoggingPolicy>('LoggingPolicy', 'add')}
					values={add}
					keyPlaceholder="user"
					valuePlaceholder="jwt.sub"
					valueKind="cel"
					onChange={setAdd}
				/>
				<ListEditor
					label="Remove fields"
					tooltip={props.help.field<LoggingPolicy>('LoggingPolicy', 'remove')}
					values={remove}
					placeholder="http.user_agent"
					onChange={setRemove}
				/>
			</PolicySection>
			<PolicySection
				icon={<Send size={17} />}
				title="OTLP export"
				description="Send access logs to an OpenTelemetry collector."
			>
				<label className="config-option-row">
					<input
						type="checkbox"
						checked={otlpEnabled}
						onChange={event => setOtlpEnabled(event.target.checked)}
					/>
					<span>
						<strong>Export logs over OTLP</strong>
					</span>
				</label>
				{otlpEnabled ? (
					<>
						<div className="form-grid">
							<Field label="Collector host">
								<input
									value={otlpHost}
									onChange={event => setOtlpHost(event.target.value)}
									placeholder="localhost:4317"
								/>
							</Field>
							<FieldGroup
								label="Protocol"
								tooltip={props.help.field<OtlpLoggingConfig>('OtlpLoggingConfig', 'protocol')}
							>
								<EnumSelector
									ariaLabel="OTLP protocol"
									value={otlpProtocol}
									options={otlpProtocolOptions}
									onChange={setOtlpProtocol}
								/>
							</FieldGroup>
							{otlpProtocol === 'http' ? (
								<Field
									label="Path"
									tooltip={props.help.field<OtlpLoggingConfig>('OtlpLoggingConfig', 'path')}
								>
									<input
										value={otlpPath}
										onChange={event => setOtlpPath(event.target.value)}
										placeholder="/v1/logs"
									/>
								</Field>
							) : null}
						</div>
						<FieldGroup
							label="OTLP filter"
							tooltip={props.help.field<OtlpLoggingConfig>('OtlpLoggingConfig', 'filter')}
						>
							<MiniMonacoEditor
								className="micro"
								language="cel"
								value={otlpFilter}
								onChange={setOtlpFilter}
								placeholder="true"
							/>
						</FieldGroup>
					</>
				) : null}
			</PolicySection>
			<PolicySection
				icon={<Database size={17} />}
				title="Database"
				description="Logs stored for the UI's Logs and Analytics pages."
			>
				<FieldGroup
					label="LLM detail"
					tooltip={props.help.field<LoggingPolicy>('LoggingPolicy', 'database.llm')}
				>
					<EnumSelector
						ariaLabel="LLM detail"
						value={llmMode}
						options={[
							{ value: 'default', label: 'Default' },
							{
								value: 'metadata',
								label: 'Metadata',
								description: 'Usage, timing, and cost without prompts or completions.'
							},
							{
								value: 'full',
								label: 'Full',
								description: 'Also store prompt and completion content.'
							}
						]}
						onChange={setLlmMode}
					/>
				</FieldGroup>
				<KeyValueEditor
					label="Database fields"
					tooltip={props.help.field<LoggingPolicy>('LoggingPolicy', 'database.add')}
					values={databaseAdd}
					keyPlaceholder="team"
					valuePlaceholder={'request.headers["x-team"]'}
					valueKind="cel"
					onChange={setDatabaseAdd}
				/>
			</PolicySection>
			<ResultingYaml value={policy} />
		</form>
	);
}
