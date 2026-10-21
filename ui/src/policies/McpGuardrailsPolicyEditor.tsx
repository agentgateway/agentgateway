import { Plus, Server, Trash2 } from 'lucide-react';
import { useState } from 'react';

import { LostFieldsWarning, UnsupportedInForm, useMergedEdits } from '@/components/EditorContracts';
import { EnumSelector } from '@/components/EnumSelector';
import { EmptyState, Field, FieldGroup, StatusBanner } from '@/components/Primitives';
import type { Processor } from '@/gateway-config';
import { KeyValueEditor } from '@/policies/PolicyFormControls';
import { cleanEmpty } from '@/policies/policyUtils';
import type { McpGuardrailsDraft } from '@/policies/types';
import type { SchemaHelp } from '@/schemaHelp';

type Phase = 'off' | 'request' | 'response' | 'full';
type FailureMode = 'failOpen' | 'failClosed';

type ProcessorDraft = {
	host: string;
	failureMode: FailureMode;
	methods: Record<string, Phase>;
	metadata: Record<string, string>;
};

const phaseOptions = [
	{ value: 'off', label: 'Off' },
	{ value: 'request', label: 'Request' },
	{ value: 'response', label: 'Response' },
	{ value: 'full', label: 'Full' }
];

const failureOptions = [
	{ value: 'failClosed', label: 'Fail closed' },
	{ value: 'failOpen', label: 'Fail open' }
];

export function McpGuardrailsPolicyEditor(props: {
	formId?: string;
	guardrails: McpGuardrailsDraft | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (guardrails: McpGuardrailsDraft) => void;
}) {
	const [initial] = useState(() => initialProcessors(props.guardrails));
	const [processors, setProcessors] = useState<ProcessorDraft[]>(initial ?? []);
	const [errors, setErrors] = useState<Record<number, string>>({});
	const [error, setError] = useState<string | null>(null);
	const preview = buildGuardrails(processors);
	const merged = useMergedEdits(props.guardrails, preview);

	if (!initial) {
		return <UnsupportedInForm supports="remote processors with a host target" />;
	}

	function addProcessor() {
		setProcessors(current => [...current, newProcessor()]);
		setError(null);
	}

	function updateProcessor(index: number, patch: Partial<ProcessorDraft>) {
		setProcessors(current =>
			current.map((processor, processorIndex) =>
				processorIndex === index ? { ...processor, ...patch } : processor
			)
		);
		setErrors(current => {
			if (!current[index]) return current;
			const next = { ...current };
			delete next[index];
			return next;
		});
		setError(null);
	}

	function removeProcessor(index: number) {
		setProcessors(current => current.filter((_, processorIndex) => processorIndex !== index));
		setErrors({});
		setError(null);
	}

	function save() {
		const nextErrors: Record<number, string> = {};
		if (!processors.length) nextErrors[0] = 'At least one processor is required.';
		processors.forEach((processor, index) => {
			if (!processor.host.trim()) nextErrors[index] = 'Processor host is required.';
			if (!Object.keys(processor.methods).length)
				nextErrors[index] = 'Add at least one MCP method match.';
		});
		setErrors(nextErrors);
		if (Object.keys(nextErrors).length) {
			setError('Fix the highlighted processors before saving.');
			return;
		}
		setError(null);
		props.onSave(merged.value);
	}

	return (
		<form
			id={props.formId}
			className="policy-editor-stack"
			onSubmit={event => {
				event.preventDefault();
				save();
			}}
		>
			<LostFieldsWarning lost={merged.lost} />
			<div className="authz-rule-toolbar">
				<div>
					<strong>
						{processors.length} {processors.length === 1 ? 'processor' : 'processors'}
					</strong>
					<small>Processors run in order; the first rejection stops the request.</small>
				</div>
				<button className="button" type="button" onClick={addProcessor}>
					<Plus size={16} />
					Add processor
				</button>
			</div>

			{processors.length === 0 ? (
				<EmptyState
					title="No MCP guardrail processors"
					description="Add a remote policy processor to inspect MCP requests and responses."
					action={
						<button className="button primary" type="button" onClick={addProcessor}>
							<Plus size={16} />
							Add processor
						</button>
					}
				/>
			) : (
				<div className="mcp-processor-list">
					{processors.map((processor, index) => (
						<section
							className={errors[index] ? 'mcp-processor-card invalid' : 'mcp-processor-card'}
							// biome-ignore lint/suspicious/noArrayIndexKey: Existing lint violation; remove this suppression when the underlying issue is fixed.
							key={index}
						>
							<div className="mcp-processor-header">
								<span className="policy-form-section-icon compact">
									<Server size={16} />
								</span>
								<div className="mcp-processor-title">
									<strong>Processor {index + 1}</strong>
									<code>{processor.host || 'No host configured'}</code>
								</div>
								<span className={processor.failureMode === 'failOpen' ? 'badge warn' : 'badge'}>
									{processor.failureMode === 'failOpen' ? 'Fail open' : 'Fail closed'}
								</span>
								<button
									className="table-action danger"
									type="button"
									onClick={() => removeProcessor(index)}
								>
									<Trash2 size={14} />
									Delete
								</button>
							</div>

							<div className="mcp-processor-controls">
								<Field label="Host" tooltip={props.help.field<Processor>('Processor', 'host')}>
									<input
										value={processor.host}
										onChange={event => updateProcessor(index, { host: event.target.value })}
										placeholder="guardrails.example.com:9000"
									/>
								</Field>
								<FieldGroup
									label="Failure mode"
									tooltip={props.help.field<Processor>('Processor', 'failureMode')}
								>
									<EnumSelector
										ariaLabel={`Processor ${index + 1} failure mode`}
										value={processor.failureMode}
										options={failureOptions}
										schema={props.help.fieldNode<Processor>('Processor', 'failureMode')}
										onChange={value =>
											updateProcessor(index, {
												failureMode: value as FailureMode
											})
										}
									/>
								</FieldGroup>
							</div>

							<MethodPhaseEditor
								methods={processor.methods}
								help={props.help}
								onChange={methods => updateProcessor(index, { methods })}
							/>

							<KeyValueEditor
								label="Metadata"
								tooltip={props.help.field<Processor>('Processor', 'metadata')}
								values={processor.metadata}
								keyPlaceholder="metadata key"
								valuePlaceholder="CEL expression"
								valueKind="cel"
								onChange={metadata => updateProcessor(index, { metadata })}
							/>

							{errors[index] ? <small className="field-error">{errors[index]}</small> : null}
						</section>
					))}
				</div>
			)}

			{error ? (
				<StatusBanner state="bad" title="Invalid MCP guardrails policy">
					{error}
				</StatusBanner>
			) : null}
		</form>
	);
}

function MethodPhaseEditor(props: {
	methods: Record<string, Phase>;
	help: SchemaHelp;
	onChange: (methods: Record<string, Phase>) => void;
}) {
	const [method, setMethod] = useState('');
	const [phase, setPhase] = useState<Phase>('request');
	const entries = Object.entries(props.methods);

	function add() {
		const key = method.trim();
		if (!key) return;
		props.onChange({ ...props.methods, [key]: phase });
		setMethod('');
	}

	return (
		<FieldGroup label="Method phases" tooltip={props.help.field<Processor>('Processor', 'methods')}>
			<div className="mcp-method-editor">
				{entries.length ? (
					<div className="mcp-method-list">
						{entries.map(([key, value]) => (
							<div className="mcp-method-row" key={key}>
								<code>{key}</code>
								<span className={`badge mcp-phase ${value}`}>{phaseLabel(value)}</span>
								<button
									className="table-action danger"
									type="button"
									onClick={() => {
										const next = { ...props.methods };
										delete next[key];
										props.onChange(next);
									}}
								>
									Remove
								</button>
							</div>
						))}
					</div>
				) : (
					<div className="empty-inline">No MCP methods configured.</div>
				)}
				<div className="mcp-method-add-row">
					<input
						value={method}
						onChange={event => setMethod(event.target.value)}
						placeholder="tools/call, prompts/*, or *"
					/>
					<EnumSelector
						ariaLabel="Phase"
						value={phase}
						options={phaseOptions}
						schema={props.help.node(['$defs', 'Phase'])}
						onChange={value => setPhase(value as Phase)}
					/>
					<button className="button" type="button" onClick={add}>
						Add
					</button>
				</div>
			</div>
		</FieldGroup>
	);
}

// The form edits remote processors with a host; any other processor makes the
// whole policy unsupported (undefined).
function initialProcessors(
	guardrails: McpGuardrailsDraft | null | undefined
): ProcessorDraft[] | undefined {
	const processors = Array.isArray(guardrails?.processors) ? guardrails.processors : [];
	const drafts = processors.map((processor: unknown) =>
		isRecord(processor) && processor.kind === 'remote' && typeof processor.host === 'string'
			? {
					host: processor.host,
					failureMode: processor.failureMode === 'failOpen' ? 'failOpen' : 'failClosed',
					methods: phaseMap(processor.methods),
					metadata: stringMap(processor.metadata),
					requestHeaders: ''
				}
			: undefined
	);
	return drafts.every(Boolean) ? (drafts as ProcessorDraft[]) : undefined;
}

function newProcessor(): ProcessorDraft {
	return {
		host: '',
		failureMode: 'failClosed',
		methods: { 'tools/call': 'request' },
		metadata: {}
	};
}

function buildGuardrails(processors: ProcessorDraft[]): McpGuardrailsDraft {
	return {
		processors: processors.map(processor =>
			cleanEmpty({
				kind: 'remote',
				host: processor.host.trim(),
				failureMode: processor.failureMode,
				methods: processor.methods,
				metadata: Object.keys(processor.metadata).length ? processor.metadata : undefined
			})
		)
	} as McpGuardrailsDraft;
}

function phaseMap(value: unknown): Record<string, Phase> {
	if (!isRecord(value)) return {};
	const next: Record<string, Phase> = {};
	Object.entries(value).forEach(([key, phase]) => {
		if (phase === 'off' || phase === 'request' || phase === 'response' || phase === 'full')
			next[key] = phase;
	});
	return next;
}

function stringMap(value: unknown): Record<string, string> {
	if (!isRecord(value)) return {};
	const next: Record<string, string> = {};
	Object.entries(value).forEach(([key, item]) => {
		if (typeof item === 'string') next[key] = item;
	});
	return next;
}

function phaseLabel(value: Phase) {
	return phaseOptions.find(option => option.value === value)?.label ?? value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}
