import { requestJson, requestText } from '@/api/base';
import type { GatewayConfig } from '@/types';

export function getConfig() {
	return requestJson<GatewayConfig>('/api/config');
}

export function getEffectiveConfig() {
	return requestJson<GatewayConfig>('/api/config/effective');
}

export function writeConfig(config: GatewayConfig) {
	return requestJson<{ status: string; message: string }>('/api/config', {
		method: 'POST',
		body: JSON.stringify(config)
	});
}

/** The configuration file as it is on disk, comments included. */
export function getRawConfig() {
	return requestText('/api/config/raw');
}

/** Writes the YAML text verbatim; the server validates it before replacing the file. */
export function writeRawConfig(text: string) {
	return requestJson<{ status: string; message: string }>('/api/config/raw', {
		method: 'POST',
		headers: { 'Content-Type': 'application/yaml' },
		body: text
	});
}
