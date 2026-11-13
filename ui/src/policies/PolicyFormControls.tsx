import { Button, Checkbox, Input, type InputRef, Radio } from 'antd';
import { Plus } from 'lucide-react';
import { type ReactNode, useRef, useState } from 'react';

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
					<Input
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
	const valueRef = useRef<InputRef>(null);
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
							<Button
								size="small"
								danger
								type="text"
								onClick={() => {
									const next = { ...props.values };
									delete next[key];
									props.onChange(next);
								}}
							>
								Remove
							</Button>
						</div>
					))}
				</div>
			) : (
				<div className="empty-inline">No values configured.</div>
			)}
			{props.quickKeys?.length ? (
				<div className="kv-quick-row">
					{props.quickKeys.map(key => (
						<Button
							size="small"
							shape="round"
							key={key}
							disabled={key in props.values}
							onClick={() => {
								setKeyDraft(key);
								window.requestAnimationFrame(() => valueRef.current?.focus());
							}}
						>
							{key}
						</Button>
					))}
				</div>
			) : null}
			<div className="kv-add-row">
				<Input
					value={keyDraft}
					onChange={event => setKeyDraft(event.target.value)}
					placeholder={props.keyPlaceholder ?? 'name'}
				/>
				<Input
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
				<Button onClick={add}>Add</Button>
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

// A bordered checkbox row with a bold title and an optional muted description.
export function OptionCheckbox(props: {
	checked: boolean;
	onChange: (checked: boolean) => void;
	title: string;
	description?: ReactNode;
}) {
	return (
		<Checkbox
			className="config-option-row"
			checked={props.checked}
			onChange={event => props.onChange(event.target.checked)}
		>
			<strong>{props.title}</strong>
			{props.description ? <small>{props.description}</small> : null}
		</Checkbox>
	);
}

// A radio group drawn as cards, each with a title and description.
export function OptionCards<T extends string>(props: {
	value: T;
	options: Array<{ value: T; label: string; description: string }>;
	onChange: (value: T) => void;
}) {
	return (
		<Radio.Group
			className="option-card-grid"
			value={props.value}
			onChange={event => props.onChange(event.target.value as T)}
		>
			{props.options.map(option => (
				<Radio className="option-card" value={option.value} key={option.value}>
					<strong>{option.label}</strong>
					<small>{option.description}</small>
				</Radio>
			))}
		</Radio.Group>
	);
}
