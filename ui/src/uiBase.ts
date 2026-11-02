export function uiBaseFrom(viteBase: string, baseElementHref: string | undefined) {
	// Router base is wherever <base> points, falling back to /ui/ if there isn't one.
	if (viteBase !== './') return viteBase;
	return baseElementHref ? new URL(baseElementHref).pathname : '/ui/';
}

export function apiBaseFrom(viteBase: string, baseElementHref: string | undefined) {
	// The admin API lives one level up from the UI.
	if (viteBase !== './' || !baseElementHref) return '';
	return new URL('..', baseElementHref).pathname.replace(/\/+$/, '');
}
