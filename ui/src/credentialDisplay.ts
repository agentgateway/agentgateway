import type { VirtualApiKey } from '@/types';

export const keyHintMetadata = 'agentgateway.dev/keyHint';

export function maskKey(key: string) {
	if (key.length <= 10) return key;
	return `${key.slice(0, 7)}...${key.slice(-4)}`;
}

export function hasKeyValue<T extends { metadata?: unknown }>(key: T): key is T & { key: string } {
	return typeof (key as { key?: unknown }).key === 'string';
}

export function keyValue(key: VirtualApiKey) {
	return hasKeyValue(key) ? key.key : (key.keyHash ?? '');
}

export function keyDisplay(key: VirtualApiKey) {
	if (hasKeyValue(key)) return maskKey(key.key);
	const hint = metadataRecord(key.metadata)[keyHintMetadata];
	return typeof hint === 'string' && hint ? hint : '****';
}

export function keyLabel(key: VirtualApiKey) {
	const metadata = metadataRecord(key.metadata);
	const name =
		typeof metadata.name === 'string' && metadata.name.trim() ? metadata.name.trim() : '';
	return name ? `${name} (${keyDisplay(key)})` : keyDisplay(key);
}

function metadataRecord(value: unknown): Record<string, unknown> {
	return value && typeof value === 'object' && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}
