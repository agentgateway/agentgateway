import { requestJson } from '@/api/base';

export interface RuntimeInfo {
	build: {
		version: string;
		gitRevision: string;
		rustVersion: string;
		buildProfile: string;
		buildTarget: string;
	};
	ui: {
		gatewayMode: 'standalone' | 'xds';
		configStoreMode: 'file' | 'hybrid' | 'readOnly';
	};
}

export function getRuntimeInfo() {
	return requestJson<RuntimeInfo>('/api/runtime');
}
