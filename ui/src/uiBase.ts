export function uiBaseFrom(viteBase: string, baseElementHref: string | undefined) {
	// Router base comes from the server's <base>, else /ui/.
	if (viteBase !== './') return viteBase;
	return baseElementHref ? new URL(baseElementHref).pathname : '/ui/';
}

export function apiBaseFrom(viteBase: string, baseElementHref: string | undefined) {
	// The admin API lives one level up from the UI.
	if (viteBase !== './' || !baseElementHref) return '';
	return new URL('..', baseElementHref).pathname.replace(/\/+$/, '');
}

export function withBasePath(path: string, apiBase: string) {
	// The proxy stripped the prefix, so add it back before redirecting the browser.
	if (!apiBase.startsWith('/') || path === apiBase || path.startsWith(`${apiBase}/`)) return path;
	return `${apiBase}${path}`;
}
