import type { Dispatch, SetStateAction } from 'react';

import type { ProviderFormatConfig } from '@/gateway-config';
import { tr } from '@/i18n';
import type { SchemaHelp } from '@/schemaHelp';
import type { CustomProvider, LlmModel, ModelProvider, ProviderFormat } from '@/types';

const formats: ProviderFormat[] = [
	'completions',
	'messages',
	'responses',
	'embeddings',
	'anthropicTokenCount',
	'generateContent',
	'geminiCountTokens',
	'realtime',
	'rerank'
];

const formatLabelKeys: Record<ProviderFormat, string> = {
	completions: 'chatCompletionsFormat',
	messages: 'anthropicMessagesFormat',
	responses: 'responsesFormat',
	embeddings: 'embeddingsFormat',
	anthropicTokenCount: 'anthropicTokenCountFormat',
	generateContent: 'geminiChatModelsModelGenerateContent',
	geminiCountTokens: 'geminiTokenCountModelsModelCountTokens',
	realtime: 'realtimeFormat',
	rerank: 'rerankFormat'
};

export function CustomFormats(props: {
	model: LlmModel;
	help: SchemaHelp;
	setModel: Dispatch<SetStateAction<LlmModel>>;
}) {
	const custom = customProvider(props.model.provider);

	function toggle(type: ProviderFormat, checked: boolean) {
		props.setModel(current => {
			const currentCustom = customProvider(current.provider);
			const nextFormats = checked
				? [...currentCustom.formats, { type }]
				: currentCustom.formats.filter((format: ProviderFormatConfig) => format.type !== type);
			return {
				...current,
				provider: { custom: { ...currentCustom, formats: nextFormats } }
			};
		});
	}

	function setPath(type: ProviderFormat, path: string) {
		props.setModel(current => {
			if (typeof current.provider === 'string' || !('custom' in current.provider)) return current;
			return {
				...current,
				provider: {
					custom: {
						...current.provider.custom,
						formats: current.provider.custom.formats.map((format: ProviderFormatConfig) =>
							format.type === type ? { ...format, path: path || null } : format
						)
					}
				}
			};
		});
	}

	return (
		<div className="format-grid">
			{formats.map(type => {
				const selected = custom.formats.find(
					(format: ProviderFormatConfig) => format.type === type
				);
				return (
					<div className="format-row" key={type}>
						<label className={selected ? 'format-toggle selected' : 'format-toggle'}>
							<input
								type="checkbox"
								checked={Boolean(selected)}
								onChange={event => toggle(type, event.target.checked)}
							/>
							<span className="format-toggle-box" aria-hidden="true" />
							<span>{formatLabel(type)}</span>
						</label>
						<input
							aria-label={tr('copy.valuePathOverride', [formatLabel(type)])}
							disabled={!selected}
							value={selected?.path ?? ''}
							placeholder={props.help.field<ProviderFormatConfig>(
								'ProviderFormatConfig',
								'path',
								'optional path override'
							)}
							onChange={event => setPath(type, event.target.value)}
						/>
					</div>
				);
			})}
		</div>
	);
}

function formatLabel(type: ProviderFormat) {
	return tr(`copy.${formatLabelKeys[type]}`);
}

function customProvider(provider: ModelProvider): CustomProvider {
	if (typeof provider === 'object' && 'custom' in provider) return provider.custom;
	return { formats: [] };
}
