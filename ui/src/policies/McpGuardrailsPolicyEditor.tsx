import { Braces, Plus, Server, Trash2 } from 'lucide-react';
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
type Kind = 'remote' | 'expression';
type Action = 'reject' | 'transform';

type ProcessorDraft = {
	kind: Kind;
	methods: Record<string, Phase>;
	host: string;
	failureMode: FailureMode;
	metadata: Record<string, string>;
	condition: string;
	action: Action;
	reject: string;
	transform: string;
	// Fields of a loaded processor the form does not show (e.g. requestHeaders),
	// written back only while it keeps its loaded kind.
	loaded?: { kind: Kind; unmodeled: Record<string, unknown> };
};

const modeledFields = new Set([
	'kind',
	'methods',
	'host',
	'failureMode',
	'metadata',
	'condition',
	'reject',
	'transform'
]);

const kindOptions = [
	{ value: 'remote', label: 'Remote', description: 'External MCP policy service' },
	{ value: 'expression', label: 'Expression', description: 'In-process CEL' }
];

const actionOptions = [
	{ value: 'reject', label: 'Reject', description: 'Deny matching requests' },
	{ value: 'transform', label: 'Transform', description: 'Rewrite params or result' }
];

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
		return (
			<UnsupportedInForm supports="expression processors and remote processors with a host target" />
		);
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
			const problem = processorProblem(processor);
			if (problem) nextErrors[index] = problem;
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
					description="Add a remote policy service or CEL expression to inspect MCP requests and responses."
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
									{processor.kind === 'remote' ? <Server size={16} /> : <Braces size={16} />}
								</span>
								<div className="mcp-processor-title">
									<strong>Processor {index + 1}</strong>
									<code>{processorSummary(processor)}</code>
								</div>
								{processor.kind === 'remote' ? (
									<span className={processor.failureMode === 'failOpen' ? 'badge warn' : 'badge'}>
										{processor.failureMode === 'failOpen' ? 'Fail open' : 'Fail closed'}
									</span>
								) : (
									<span className={processor.action === 'reject' ? 'badge warn' : 'badge'}>
										{processor.action === 'reject' ? 'Reject' : 'Transform'}
									</span>
								)}
								<button
									className="table-action danger"
									type="button"
									onClick={() => removeProcessor(index)}
								>
									<Trash2 size={14} />
									Delete
								</button>
							</div>

							<FieldGroup label="Type">
								<EnumSelector
									ariaLabel={`Processor ${index + 1} type`}
									value={processor.kind}
									options={kindOptions}
									onChange={value => updateProcessor(index, { kind: value as Kind })}
								/>
							</FieldGroup>

							{processor.kind === 'remote' ? (
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
							) : (
								<ExpressionControls
									index={index}
									processor={processor}
									help={props.help}
									onChange={patch => updateProcessor(index, patch)}
								/>
							)}

							<MethodPhaseEditor
								methods={processor.methods}
								help={props.help}
								onChange={methods => updateProcessor(index, { methods })}
							/>

							{processor.kind === 'remote' ? (
								<KeyValueEditor
									label="Metadata"
									tooltip={props.help.field<Processor>('Processor', 'metadata')}
									values={processor.metadata}
									keyPlaceholder="metadata key"
									valuePlaceholder="CEL expression"
									valueKind="cel"
									onChange={metadata => updateProcessor(index, { metadata })}
								/>
							) : null}

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

function ExpressionControls(props: {
	index: number;
	processor: ProcessorDraft;
	help: SchemaHelp;
	onChange: (patch: Partial<ProcessorDraft>) => void;
}) {
	const { processor } = props;
	return (
		<div className="policy-editor-stack">
			<Field
				label="Condition"
				hint="Optional. When empty, the action applies to every matched method."
				tooltip={props.help.propertyDescription('Processor', ['condition'])}
			>
				<input
					className="mono-input"
					value={processor.condition}
					onChange={event => props.onChange({ condition: event.target.value })}
					placeholder={'mcp.tool.name == "delete_repo"'}
				/>
			</Field>
			<FieldGroup label="Action">
				<EnumSelector
					ariaLabel={`Processor ${props.index + 1} action`}
					value={processor.action}
					options={actionOptions}
					onChange={value => props.onChange({ action: value as Action })}
				/>
			</FieldGroup>
			{processor.action === 'reject' ? (
				<Field
					label="Reject message"
					tooltip={props.help.propertyDescription('Processor', ['reject'])}
				>
					<input
						value={processor.reject}
						onChange={event => props.onChange({ reject: event.target.value })}
						placeholder="This tool is not allowed"
					/>
				</Field>
			) : (
				<Field
					label="Transform"
					tooltip={props.help.propertyDescription('Processor', ['transform'])}
				>
					<textarea
						className="mono-input"
						rows={3}
						value={processor.transform}
						onChange={event => props.onChange({ transform: event.target.value })}
						placeholder={'merge(mcp.params, {"arguments": {...}})'}
					/>
				</Field>
			)}
		</div>
	);
}

function processorProblem(processor: ProcessorDraft): string | undefined {
	if (processor.kind === 'remote')
		return processor.host.trim() ? undefined : 'Processor host is required.';
	if (processor.action === 'reject')
		return processor.reject.trim() ? undefined : 'Reject message is required.';
	return processor.transform.trim() ? undefined : 'Transform expression is required.';
}

function processorSummary(processor: ProcessorDraft) {
	if (processor.kind === 'remote') return processor.host || 'No host configured';
	return processor.condition.trim() || 'Always';
}

// The form edits expression processors and remote processors with a host; any
// other processor makes the whole policy unsupported (undefined).
function initialProcessors(
	guardrails: McpGuardrailsDraft | null | undefined
): ProcessorDraft[] | undefined {
	const processors = Array.isArray(guardrails?.processors) ? guardrails.processors : [];
	const drafts = processors.map((processor: unknown) => {
		if (!isRecord(processor)) return undefined;
		const base = {
			...newProcessor(),
			methods: phaseMap(processor.methods),
			loaded: {
				kind: processor.kind === 'expression' ? 'expression' : 'remote',
				unmodeled: Object.fromEntries(
					Object.entries(processor).filter(([key]) => !modeledFields.has(key))
				)
			}
		} satisfies ProcessorDraft;
		if (processor.kind === 'remote' && typeof processor.host === 'string') {
			return {
				...base,
				host: processor.host,
				failureMode: processor.failureMode === 'failOpen' ? 'failOpen' : 'failClosed',
				metadata: stringMap(processor.metadata)
			} satisfies ProcessorDraft;
		}
		if (processor.kind === 'expression') {
			const condition = typeof processor.condition === 'string' ? processor.condition : '';
			if (typeof processor.reject === 'string')
				return {
					...base,
					kind: 'expression',
					condition,
					reject: processor.reject
				} satisfies ProcessorDraft;
			if (typeof processor.transform === 'string')
				return {
					...base,
					kind: 'expression',
					condition,
					action: 'transform',
					transform: processor.transform
				} satisfies ProcessorDraft;
		}
		return undefined;
	});
	return drafts.every(Boolean) ? (drafts as ProcessorDraft[]) : undefined;
}

function newProcessor(): ProcessorDraft {
	return {
		kind: 'remote',
		methods: { 'tools/call': 'request' },
		host: '',
		failureMode: 'failClosed',
		metadata: {},
		condition: '',
		action: 'reject',
		reject: '',
		transform: ''
	};
}

function buildGuardrails(processors: ProcessorDraft[]): McpGuardrailsDraft {
	return {
		processors: processors.map(processor => {
			const unmodeled = processor.loaded?.kind === processor.kind ? processor.loaded.unmodeled : {};
			return processor.kind === 'remote'
				? cleanEmpty({
						...unmodeled,
						kind: 'remote',
						host: processor.host.trim(),
						failureMode: processor.failureMode,
						methods: processor.methods,
						metadata: Object.keys(processor.metadata).length ? processor.metadata : undefined
					})
				: cleanEmpty({
						...unmodeled,
						kind: 'expression',
						methods: processor.methods,
						condition: processor.condition.trim() || undefined,
						...(processor.action === 'reject'
							? { reject: processor.reject }
							: { transform: processor.transform.trim() })
					});
		})
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
