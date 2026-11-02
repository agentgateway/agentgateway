export function uiBaseFrom(viteBase: string, baseElementHref: string | undefined) {
	if (viteBase !== './') return viteBase;
	return baseElementHref ? new URL(baseElementHref).pathname : '/ui/';
}

export function apiBaseFrom(viteBase: string, baseElementHref: string | undefined) {
	if (viteBase !== './' || !baseElementHref) return '';
	return new URL('..', baseElementHref).pathname.replace(/\/+$/, '');
}
