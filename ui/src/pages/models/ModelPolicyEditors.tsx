import { Checkbox, Input, InputNumber } from 'antd';

import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { Field, FieldGroup } from '@/components/Primitives';
import type {
	HeaderModifier,
	LocalEviction,
	LocalHealthPolicy,
	PromptCachingConfig
} from '@/gateway-config';
import { ListEditor } from '@/policies/ListEditor';
import { KeyValueEditor } from '@/policies/PolicyFormControls';
import type { SchemaHelp } from '@/schemaHelp';
import type { LlmModel } from '@/types';

export function HealthPolicyEditor(props: {
	health: LlmModel['health'] | null | undefined;
	help: SchemaHelp;
	onChange: (value: LlmModel['health'] | null) => void;
}) {
	const health = props.health ?? {};
	const eviction = health.eviction ?? {};

	function patchHealth(value: Partial<NonNullable<LlmModel['health']>>) {
		props.onChange({ ...health, ...value });
	}

	function patchEviction(value: Partial<NonNullable<NonNullable<LlmModel['health']>['eviction']>>) {
		patchHealth({ eviction: { ...eviction, ...value } });
	}

	return (
		<div className="policy-editor-stack compact">
			<FieldGroup
				label="Unhealthy expression"
				tooltip={props.help.field<LocalHealthPolicy>('LocalHealthPolicy', 'unhealthyExpression')}
				hint="Leave empty to use default 5xx and connection failure handling."
			>
				<MiniMonacoEditor
					language="cel"
					value={health.unhealthyExpression ?? ''}
					onChange={value => patchHealth({ unhealthyExpression: value || null })}
					placeholder="response.code >= 500"
				/>
			</FieldGroup>
			<div className="form-grid">
				<Field
					label="Eviction duration"
					tooltip={props.help.field<LocalEviction>('LocalEviction', 'duration')}
				>
					<Input
						value={eviction.duration ?? ''}
						onChange={event => patchEviction({ duration: event.target.value || null })}
						placeholder="30s"
					/>
				</Field>
				<Field
					label="Consecutive failures"
					tooltip={props.help.field<LocalEviction>('LocalEviction', 'consecutiveFailures')}
				>
					<InputNumber
						min={1}
						value={eviction.consecutiveFailures ?? null}
						onChange={value => patchEviction({ consecutiveFailures: value })}
						placeholder="3"
					/>
				</Field>
				<Field
					label="Health threshold"
					tooltip={props.help.field<LocalEviction>('LocalEviction', 'healthThreshold')}
				>
					<InputNumber
						value={eviction.healthThreshold ?? null}
						onChange={value => patchEviction({ healthThreshold: value })}
						placeholder="0.5"
					/>
				</Field>
				<Field
					label="Restore health"
					tooltip={props.help.field<LocalEviction>('LocalEviction', 'restoreHealth')}
				>
					<InputNumber
						value={eviction.restoreHealth ?? null}
						onChange={value => patchEviction({ restoreHealth: value })}
						placeholder="1"
					/>
				</Field>
			</div>
		</div>
	);
}

export function YamlMappingEditor(props: {
	label: string;
	tooltip?: string;
	value: string;
	placeholder: string;
	onChange: (value: string) => void;
}) {
	return (
		<FieldGroup label={props.label} tooltip={props.tooltip}>
			<MiniMonacoEditor
				language="yaml"
				value={props.value}
				onChange={props.onChange}
				placeholder={props.placeholder}
			/>
		</FieldGroup>
	);
}

export function HeaderModifierEditor(props: {
	value: LlmModel['requestHeaders'] | LlmModel['responseHeaders'] | null | undefined;
	help: SchemaHelp;
	onChange: (value: LlmModel['requestHeaders'] | null) => void;
}) {
	const value = props.value ?? {};
	return (
		<div className="policy-editor-stack compact">
			<KeyValueEditor
				label="Add headers"
				tooltip={props.help.field<HeaderModifier>('HeaderModifier', 'add')}
				values={value.add ?? {}}
				keyPlaceholder="x-header"
				valuePlaceholder="value"
				onChange={add => props.onChange({ ...value, add })}
			/>
			<KeyValueEditor
				label="Set headers"
				tooltip={props.help.field<HeaderModifier>('HeaderModifier', 'set')}
				values={value.set ?? {}}
				keyPlaceholder="x-header"
				valuePlaceholder="value"
				onChange={set => props.onChange({ ...value, set })}
			/>
			<ListEditor
				label="Remove headers"
				tooltip={props.help.field<HeaderModifier>('HeaderModifier', 'remove')}
				values={value.remove ?? []}
				placeholder="x-header"
				onChange={remove => props.onChange({ ...value, remove })}
			/>
		</div>
	);
}

export function PromptCachingEditor(props: {
	value: LlmModel['promptCaching'] | null | undefined;
	help: SchemaHelp;
	onChange: (value: LlmModel['promptCaching'] | null) => void;
}) {
	const value = props.value ?? {};

	function patch(next: Partial<NonNullable<LlmModel['promptCaching']>>) {
		props.onChange({ ...value, ...next });
	}

	return (
		<div className="policy-editor-stack compact">
			<div className="form-grid">
				{(
					[
						['cacheSystem', 'System prompt'],
						['cacheMessages', 'Messages'],
						['cacheTools', 'Tools']
					] as const
				).map(([key, label]) => (
					<Checkbox
						key={key}
						className="prompt-cache-option"
						checked={Boolean(value[key])}
						onChange={event => patch({ [key]: event.target.checked || undefined })}
					>
						<strong>{label}</strong>
						<small>{props.help.field<PromptCachingConfig>('PromptCachingConfig', key)}</small>
					</Checkbox>
				))}
			</div>
			<div className="form-grid">
				<Field
					label="Minimum tokens"
					tooltip={props.help.field<PromptCachingConfig>('PromptCachingConfig', 'minTokens')}
				>
					<InputNumber
						value={value.minTokens ?? null}
						onChange={minTokens => patch({ minTokens })}
						placeholder="1024"
					/>
				</Field>
				<Field
					label="Message offset"
					tooltip={props.help.field<PromptCachingConfig>(
						'PromptCachingConfig',
						'cacheMessageOffset'
					)}
				>
					<InputNumber
						value={value.cacheMessageOffset ?? null}
						onChange={offset => patch({ cacheMessageOffset: offset ?? undefined })}
						placeholder="0"
					/>
				</Field>
			</div>
		</div>
	);
}

export function healthSummary(health: LlmModel['health'] | null | undefined) {
	if (!health) return 'No health policy configured';
	const parts = [
		health.unhealthyExpression ? 'custom expression' : null,
		health.eviction?.duration ? `evict ${health.eviction.duration}` : null,
		health.eviction?.consecutiveFailures ? `${health.eviction.consecutiveFailures} failures` : null
	].filter(Boolean);
	return parts.join(', ') || 'Default unhealthy detection configured';
}

export function headerModifierSummary(
	value: LlmModel['requestHeaders'] | LlmModel['responseHeaders'] | null | undefined,
	label: 'request' | 'response'
) {
	const count =
		Object.keys(value?.add ?? {}).length +
		Object.keys(value?.set ?? {}).length +
		(value?.remove?.length ?? 0);
	if (count === 0) return `No ${label} header changes configured`;
	return `${count} ${count === 1 ? 'header change' : 'header changes'} configured`;
}

export function promptCachingSummary(value: LlmModel['promptCaching'] | null | undefined) {
	if (!value) return 'No prompt caching configured';
	const scopes = [
		value.cacheSystem ? 'system' : null,
		value.cacheMessages ? 'messages' : null,
		value.cacheTools ? 'tools' : null
	].filter(Boolean);
	return scopes.length ? `Cache ${scopes.join(', ')}` : 'Prompt caching configured';
}
