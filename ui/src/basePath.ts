import { apiBaseFrom, uiBaseFrom } from '@/uiBase';

function baseElementHref() {
	// document.baseURI always has a value, so check the server actually injected a <base>.
	return document.querySelector('base') ? document.baseURI : undefined;
}

export function defaultApiBase() {
	return apiBaseFrom(import.meta.env.BASE_URL, baseElementHref());
}

export function routerBasePath() {
	const configured = import.meta.env.VITE_ROUTER_BASE_PATH;
	const base = configured || uiBaseFrom(import.meta.env.BASE_URL, baseElementHref());
	if (!base || base === './' || base === '/') return undefined;
	return `/${base.replace(/^\/+|\/+$/g, '')}`;
}
