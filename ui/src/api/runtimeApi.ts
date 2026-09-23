import { requestJson } from '@/api/base';

export interface RuntimeUser {
	/** Identity resolved by config.standardAttributes.user, including its default mapping. */
	subject: string | null;
	/** Optional profile details from validated JWT claims. */
	name: string | null;
	email: string | null;
	/** True only for a validated browser session with logout available. */
	canLogout: boolean;
}

export interface RuntimeInfo {
	user?: RuntimeUser | null;
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
