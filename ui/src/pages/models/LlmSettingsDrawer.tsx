import { Server } from 'lucide-react';
import { useState } from 'react';

import { ConfigDiffSaveActions } from '@/components/ConfigDiffDrawer';
import { useChangedFields } from '@/components/EditorContracts';
import { GatewayBindingEditor, type GatewayBindingValue } from '@/components/GatewayBindingEditor';
import { Drawer, StatusBanner } from '@/components/Primitives';
import { ensureLlm, llmSettingsFields } from '@/config';
import { PolicySection } from '@/policies/PolicyLayout';
import type { SchemaHelp } from '@/schemaHelp';
import type { GatewayConfig, LlmConfig } from '@/types';

export type LlmSettingsPatch = Partial<Omit<LlmConfig, 'gateways' | 'port'>> & {
	gateways?: LlmConfig['gateways'] | null;
	port?: number | null;
};

export function LlmSettingsDrawer(props: {
	config?: GatewayConfig | null;
	llm?: LlmConfig | null;
	databaseBacked?: boolean;
	readOnlyFields?: ReadonlySet<string>;
	help: SchemaHelp;
	saving: boolean;
	saveError?: string | null;
	onClose: () => void;
	onSave: (settings: LlmSettingsPatch) => void;
}) {
	const [binding, setBinding] = useState<GatewayBindingValue>({
		gateways: props.llm?.gateways ?? null,
		port: props.llm?.port ?? null
	});
	const usesGateway = binding.gateways != null && binding.gateways.length > 0;
	const patch: LlmSettingsPatch = {
		gateways: binding.gateways ?? [],
		port: usesGateway ? null : binding.port
	};
	// The settings are replaced as a whole: carry tls through (it can't sit beside gateways) and
	// leave out file-owned fields. Fields the form was not used to change keep their configured value.
	const original = settingsResource(props.llm ?? {}, props.readOnlyFields);
	const settings = useChangedFields(
		original,
		settingsResource({ ...patch, tls: usesGateway ? null : props.llm?.tls }, props.readOnlyFields)
	);
	const bindingReadOnly = Boolean(
		props.readOnlyFields?.has('gateways') || props.readOnlyFields?.has('port')
	);

	return (
		<Drawer title="Settings" onClose={props.onClose}>
			<form
				className="policy-editor-stack"
				onSubmit={event => {
					event.preventDefault();
					props.onSave(settings);
				}}
			>
				<PolicySection
					icon={<Server size={17} />}
					title="Gateway binding"
					description="Choose how LLM traffic is exposed."
				>
					<div className="form-grid">
						<GatewayBindingEditor
							config={props.config}
							value={binding}
							defaultPort={4000}
							portLabel="Port"
							portPlaceholder="4000"
							portTooltip={props.help.field<LlmConfig>(
								'LocalLLMConfig',
								'port',
								'Gateway port for LLM traffic.'
							)}
							disabled={bindingReadOnly}
							onChange={setBinding}
						/>
					</div>
				</PolicySection>
				<ConfigDiffSaveActions
					config={props.config}
					diffTitle="LLM settings config diff"
					saveLabel="Save settings"
					saving={props.saving}
					saveDisabled={props.databaseBacked && bindingReadOnly}
					onSave={() => props.onSave(settings)}
					resourceDiff={props.databaseBacked ? { original, modified: settings } : undefined}
					applyDiff={next => applyLlmSettings(next, settings, props.readOnlyFields)}
				/>
			</form>
			{props.saveError ? (
				<StatusBanner state="bad" title="Save failed">
					{props.saveError}
				</StatusBanner>
			) : null}
		</Drawer>
	);
}

/** Replaces the LLM settings in `config`, leaving file-owned fields as they are. */
export function applyLlmSettings(
	config: GatewayConfig,
	settings: LlmSettingsPatch,
	readOnlyFields?: ReadonlySet<string>
) {
	const llm = ensureLlm(config) as Record<string, unknown>;
	for (const field of llmSettingsFields) {
		if (readOnlyFields?.has(field)) continue;
		if (field in settings) llm[field] = settings[field];
		else delete llm[field];
	}
}

function settingsResource(
	settings: Pick<LlmSettingsPatch, 'gateways' | 'port' | 'tls'>,
	readOnlyFields?: ReadonlySet<string>
): LlmSettingsPatch {
	return Object.fromEntries(
		llmSettingsFields.flatMap(field => {
			const value = settings[field];
			const empty = value == null || (Array.isArray(value) && value.length === 0);
			return empty || readOnlyFields?.has(field) ? [] : [[field, value]];
		})
	);
}
