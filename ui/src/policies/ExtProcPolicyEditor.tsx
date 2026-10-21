import { SlidersHorizontal } from 'lucide-react';
import { useState } from 'react';

import { LostFieldsWarning, UnsupportedInForm, useMergedEdits } from '@/components/EditorContracts';
import { EnumSelector, type EnumSelectorOption } from '@/components/EnumSelector';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { FieldGroup } from '@/components/Primitives';
import type { ExtProc, ProcessingOptions } from '@/gateway-config';
import {
	hasUnsupportedTarget,
	KeyValueEditor,
	TargetEditor,
	targetFrom
} from '@/policies/PolicyFormControls';
import { PolicySection } from '@/policies/PolicyLayout';
import { cleanEmpty, parseYamlText, toYamlMappingText } from '@/policies/policyUtils';
import type { ExtProcDraft } from '@/policies/types';
import type { DeepPath, SchemaHelp } from '@/schemaHelp';

type BodyMode = 'none' | 'buffered' | 'bufferedPartial' | 'fullDuplexStreamed';
type SendMode = 'send' | 'skip';

const bodyModes: Array<EnumSelectorOption<BodyMode>> = [
	{ value: 'fullDuplexStreamed', label: 'Full duplex streamed' },
	{ value: 'buffered', label: 'Buffered' },
	{ value: 'bufferedPartial', label: 'Buffered partial' },
	{ value: 'none', label: 'None' }
];

const sendModes: Array<EnumSelectorOption<SendMode>> = [
	{ value: 'send', label: 'Send' },
	{ value: 'skip', label: 'Skip' }
];

export function ExtProcPolicyEditor(props: {
	formId?: string;
	extProc: ExtProcDraft | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (value: ExtProcDraft) => void;
}) {
	const unsupportedTarget = hasUnsupportedTarget(props.extProc);
	const [target, setTarget] = useState(() => targetFrom(props.extProc));
	const [failureMode, setFailureMode] = useState<'failClosed' | 'failOpen'>(
		props.extProc?.failureMode ?? 'failClosed'
	);
	const options = props.extProc?.processingOptions ?? {};
	const [requestBodyMode, setRequestBodyMode] = useState<BodyMode>(
		options.requestBodyMode ?? 'fullDuplexStreamed'
	);
	const [responseBodyMode, setResponseBodyMode] = useState<BodyMode>(
		options.responseBodyMode ?? 'fullDuplexStreamed'
	);
	const [requestHeaderMode, setRequestHeaderMode] = useState<SendMode>(
		options.requestHeaderMode ?? 'send'
	);
	const [responseHeaderMode, setResponseHeaderMode] = useState<SendMode>(
		options.responseHeaderMode ?? 'send'
	);
	const [requestTrailerMode, setRequestTrailerMode] = useState<SendMode>(
		options.requestTrailerMode ?? 'send'
	);
	const [responseTrailerMode, setResponseTrailerMode] = useState<SendMode>(
		options.responseTrailerMode ?? 'send'
	);
	const [allowModeOverride, setAllowModeOverride] = useState(Boolean(options.allowModeOverride));
	const [requestAttributes, setRequestAttributes] = useState(
		props.extProc?.requestAttributes ?? {}
	);
	const [responseAttributes, setResponseAttributes] = useState(
		props.extProc?.responseAttributes ?? {}
	);
	const [metadataText, setMetadataText] = useState(
		toYamlMappingText(props.extProc?.metadataContext)
	);
	const [metadataError, setMetadataError] = useState<string | null>(null);
	const preview = buildExtProc();
	const merged = useMergedEdits(props.extProc, preview);

	if (unsupportedTarget) {
		return <UnsupportedInForm supports="host targets" />;
	}

	function buildExtProc() {
		let metadataContext: unknown;
		try {
			metadataContext = metadataText.trim() ? parseYamlText(metadataText) : undefined;
		} catch {
			metadataContext = undefined;
		}
		return cleanEmpty({
			...target,
			failureMode,
			processingOptions: {
				requestBodyMode,
				responseBodyMode,
				requestHeaderMode,
				responseHeaderMode,
				requestTrailerMode,
				responseTrailerMode,
				allowModeOverride: allowModeOverride ? true : undefined
			},
			requestAttributes: Object.keys(requestAttributes).length ? requestAttributes : undefined,
			responseAttributes: Object.keys(responseAttributes).length ? responseAttributes : undefined,
			metadataContext
		}) as ExtProcDraft;
	}

	function save() {
		try {
			if (metadataText.trim()) parseYamlText(metadataText);
			setMetadataError(null);
			props.onSave(merged.value);
		} catch (err) {
			setMetadataError(err instanceof Error ? err.message : 'Invalid metadata YAML');
		}
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
			<TargetEditor
				value={target}
				help={props.help}
				tooltip={props.help.field<ExtProc>('ExtProc', 'host')}
				onChange={setTarget}
			/>
			<PolicySection
				icon={<SlidersHorizontal size={17} />}
				title="Processing behavior"
				description="Choose failure behavior and which request/response phases are sent."
			>
				<FieldGroup
					label="Failure mode"
					tooltip={props.help.field<ExtProc>('ExtProc', 'failureMode')}
				>
					<EnumSelector
						ariaLabel="Failure mode"
						value={failureMode}
						options={[
							{ value: 'failClosed', label: 'Fail closed' },
							{ value: 'failOpen', label: 'Fail open' }
						]}
						schema={props.help.fieldNode<ExtProc>('ExtProc', 'failureMode')}
						onChange={setFailureMode}
					/>
				</FieldGroup>
				<div className="form-grid">
					<ModeSelect
						label="Request body"
						help={props.help}
						field="requestBodyMode"
						value={requestBodyMode}
						options={bodyModes}
						onChange={setRequestBodyMode}
					/>
					<ModeSelect
						label="Response body"
						help={props.help}
						field="responseBodyMode"
						value={responseBodyMode}
						options={bodyModes}
						onChange={setResponseBodyMode}
					/>
					<ModeSelect
						label="Request headers"
						help={props.help}
						field="requestHeaderMode"
						value={requestHeaderMode}
						options={sendModes}
						onChange={setRequestHeaderMode}
					/>
					<ModeSelect
						label="Response headers"
						help={props.help}
						field="responseHeaderMode"
						value={responseHeaderMode}
						options={sendModes}
						onChange={setResponseHeaderMode}
					/>
					<ModeSelect
						label="Request trailers"
						help={props.help}
						field="requestTrailerMode"
						value={requestTrailerMode}
						options={sendModes}
						onChange={setRequestTrailerMode}
					/>
					<ModeSelect
						label="Response trailers"
						help={props.help}
						field="responseTrailerMode"
						value={responseTrailerMode}
						options={sendModes}
						onChange={setResponseTrailerMode}
					/>
				</div>
				<label className="config-option-row">
					<input
						type="checkbox"
						checked={allowModeOverride}
						onChange={event => setAllowModeOverride(event.target.checked)}
					/>
					<span>
						<strong>Allow mode override</strong>
						<small>
							{props.help.field<ProcessingOptions>('ProcessingOptions', 'allowModeOverride')}
						</small>
					</span>
				</label>
			</PolicySection>
			<PolicySection
				icon={<SlidersHorizontal size={17} />}
				title="Attributes"
				description="CEL expressions sent as attributes to the processor."
			>
				<KeyValueEditor
					label="Request attributes"
					tooltip={props.help.field<ExtProc>('ExtProc', 'requestAttributes')}
					values={requestAttributes}
					keyPlaceholder="key"
					valuePlaceholder="CEL expression"
					valueKind="cel"
					onChange={setRequestAttributes}
				/>
				<KeyValueEditor
					label="Response attributes"
					tooltip={props.help.field<ExtProc>('ExtProc', 'responseAttributes')}
					values={responseAttributes}
					keyPlaceholder="key"
					valuePlaceholder="CEL expression"
					valueKind="cel"
					onChange={setResponseAttributes}
				/>
				<FieldGroup
					label="Metadata context YAML"
					tooltip={props.help.field<ExtProc>('ExtProc', 'metadataContext')}
					className={metadataError ? 'invalid' : undefined}
					hint={metadataError ?? undefined}
				>
					<MiniMonacoEditor
						language="yaml"
						value={metadataText}
						onChange={setMetadataText}
						placeholder={'namespace:\n  key: CEL expression'}
					/>
				</FieldGroup>
			</PolicySection>
		</form>
	);
}

function ModeSelect<T extends string>(props: {
	label: string;
	help: SchemaHelp;
	field: DeepPath<ProcessingOptions>;
	value: T;
	options: Array<EnumSelectorOption<T>>;
	onChange: (value: T) => void;
}) {
	return (
		<FieldGroup
			label={props.label}
			tooltip={props.help.field<ProcessingOptions>('ProcessingOptions', props.field)}
		>
			<EnumSelector
				ariaLabel={props.label}
				value={props.value}
				options={props.options}
				schema={props.help.fieldNode<ProcessingOptions>('ProcessingOptions', props.field)}
				onChange={props.onChange}
			/>
		</FieldGroup>
	);
}
