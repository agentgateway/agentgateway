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
	/** Standalone config reload state; mirrors the config_synchronized metric. */
	configReload: {
		/** False when the most recent reload failed and the runtime kept the previous config. */
		synchronized: boolean;
		/** Error of the most recent failed reload, if any. */
		lastError: string | null;
	};
}

export function getRuntimeInfo() {
	return requestJson<RuntimeInfo>('/api/runtime');
}
