import { Plus } from 'lucide-react';
import { useRef, useState } from 'react';

import { Field, FieldGroup, StatusBanner } from '@/components/Primitives';
import { parseSchemaYamlEditorValue, SchemaYamlEditor } from '@/components/SchemaYamlEditor';
import { isRecord, toYamlMappingText } from '@/policies/policyUtils';
import type { TargetDraft } from '@/policies/types';
import type { SchemaHelp } from '@/schemaHelp';

export type TargetMode = 'host' | 'service' | 'backend';

export function targetMode(value: unknown): TargetMode {
	if (value && typeof value === 'object' && !Array.isArray(value)) {
		if ('service' in value) return 'service';
		if ('backend' in value) return 'backend';
	}
	return 'host';
}

export function hasUnsupportedTarget(value: unknown) {
	return targetMode(value) !== 'host';
}

export function TargetEditor(props: {
	value: TargetDraft;
	help: SchemaHelp;
	tooltip?: string;
	onChange: (value: TargetDraft) => void;
}) {
	return (
		<div className="policy-form-section">
			<div className="policy-form-section-header">
				<span className="policy-form-section-icon">
					<Plus size={17} />
				</span>
				<div>
					<h4>Target</h4>
					<p>External service the gateway calls for this policy.</p>
				</div>
			</div>
			<div className="policy-form-section-body">
				<Field label="Host" tooltip={props.tooltip} path="host">
					<input
						value={props.value.host}
						onChange={event => props.onChange({ ...props.value, host: event.target.value })}
						placeholder="host:port or unix:/path/to/socket"
					/>
				</Field>
				<BackendPoliciesField
					help={props.help}
					value={props.value.policies}
					onChange={policies => props.onChange({ ...props.value, policies })}
				/>
			</div>
		</div>
	);
}

export function KeyValueEditor(props: {
	label?: string;
	tooltip?: string;
	values: Record<string, string>;
	keyPlaceholder?: string;
	valuePlaceholder?: string;
	valueKind?: 'text' | 'cel';
	quickKeys?: string[];
	onChange: (values: Record<string, string>) => void;
}) {
	const [keyDraft, setKeyDraft] = useState('');
	const [valueDraft, setValueDraft] = useState('');
	const valueRef = useRef<HTMLInputElement>(null);
	const entries = Object.entries(props.values);

	function add() {
		const key = keyDraft.trim();
		if (!key) return;
		props.onChange({ ...props.values, [key]: valueDraft.trim() });
		setKeyDraft('');
		setValueDraft('');
	}

	const editor = (
		<div className="kv-editor">
			{entries.length ? (
				<div className="kv-list">
					{entries.map(([key, value]) => (
						<div className="kv-row" key={key}>
							<code>{key}</code>
							<span>{value}</span>
							<button
								className="table-action danger"
								type="button"
								onClick={() => {
									const next = { ...props.values };
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
				<div className="empty-inline">No values configured.</div>
			)}
			{props.quickKeys?.length ? (
				<div className="kv-quick-row">
					{props.quickKeys.map(key => (
						<button
							className="choice-pill compact"
							type="button"
							key={key}
							disabled={key in props.values}
							onClick={() => {
								setKeyDraft(key);
								window.requestAnimationFrame(() => valueRef.current?.focus());
							}}
						>
							{key}
						</button>
					))}
				</div>
			) : null}
			<div className="kv-add-row">
				<input
					value={keyDraft}
					onChange={event => setKeyDraft(event.target.value)}
					placeholder={props.keyPlaceholder ?? 'name'}
				/>
				<input
					ref={valueRef}
					className={props.valueKind === 'cel' ? 'mono-input' : undefined}
					value={valueDraft}
					onChange={event => setValueDraft(event.target.value)}
					onKeyDown={event => {
						if (event.key === 'Enter') {
							event.preventDefault();
							add();
						}
					}}
					placeholder={
						props.valuePlaceholder ?? (props.valueKind === 'cel' ? 'request.path' : 'value')
					}
				/>
				<button className="button" type="button" onClick={add}>
					Add
				</button>
			</div>
		</div>
	);
	return props.label ? (
		<FieldGroup label={props.label} tooltip={props.tooltip}>
			{editor}
		</FieldGroup>
	) : (
		editor
	);
}

export function targetFrom(value: unknown, fallbackHost = '127.0.0.1:9000'): TargetDraft {
	const target = isRecord(value) ? value : {};
	return {
		host: typeof target.host === 'string' ? target.host : fallbackHost,
		policies: target.policies
	};
}

// Edits SimpleLocalBackendPolicies (backendAuth, backendTLS, ...) used when the
// gateway calls an external service. Invalid YAML keeps the last valid value.
export function BackendPoliciesField(props: {
	help: SchemaHelp;
	value: unknown;
	onChange: (value: unknown) => void;
}) {
	const [text, setText] = useState(() => toYamlMappingText(props.value));
	const [error, setError] = useState<string | null>(null);

	function update(next: string) {
		setText(next);
		try {
			const parsed = parseSchemaYamlEditorValue(next);
			if (!isRecord(parsed)) throw new Error('Backend policies must be a YAML mapping.');
			setError(null);
			props.onChange(Object.keys(parsed).length ? parsed : undefined);
		} catch (err) {
			setError(err instanceof Error ? err.message : 'Invalid YAML');
		}
	}

	return (
		<FieldGroup
			label="Backend policies"
			tooltip={props.help.definition(
				'SimpleLocalBackendPolicies',
				'Policies applied when connecting to this service.'
			)}
			hint="Optional. For example, backendAuth.key.value for an API key, or backendTLS."
		>
			{error ? (
				<StatusBanner state="bad" title="Invalid YAML">
					{error}
				</StatusBanner>
			) : null}
			<SchemaYamlEditor
				className="short"
				path="agentgateway-backend-policies.yaml"
				schema={props.help.node(['$defs', 'SimpleLocalBackendPolicies']) ?? {}}
				showLineNumbers={false}
				invalid={Boolean(error)}
				value={text}
				onChange={update}
			/>
		</FieldGroup>
	);
}
