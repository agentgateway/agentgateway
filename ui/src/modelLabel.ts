import type { LlmModel } from '@/types';

type HeaderValue = NonNullable<
	NonNullable<LlmModel['matches']>[number]['headers']
>[number]['value'];

// Label shown in the UI; requests always send model.name.
export function modelLabel(model: LlmModel): string {
	return model.displayName?.trim() || model.name;
}

// Short text that tells apart models sharing a match pattern, such as several `*` rows.
export function modelDetail(model: LlmModel): string {
	const params = model.params ?? {};
	const headers = (model.matches ?? []).flatMap(match =>
		(match.headers ?? []).map(header => `${header.name}: ${headerValueText(header.value)}`)
	);
	return [
		params.vertexProject,
		params.vertexRegion,
		params.awsRegion,
		params.azureResourceName,
		params.baseUrl ?? params.hostOverride,
		...headers
	]
		.filter(Boolean)
		.join(' · ');
}

// Secondary line under a label: the pattern when a display name hides it, then the detail.
export function modelSubtitle(model: LlmModel): string {
	return [model.displayName?.trim() ? model.name : '', modelDetail(model)]
		.filter(Boolean)
		.join(' · ');
}

// A picker value: the model name, plus its id or detail when an earlier model has the same name.
// The index is the last resort, as it shifts when models are added or removed.
export function modelOptionValue(models: LlmModel[], index: number): string {
	const model = models[index];
	const sameName = models.filter(other => other.name === model.name);
	if (sameName[0] === model) return model.name;
	if (model.id) return `${model.name}#${model.id}`;
	const detail = modelDetail(model);
	const unique = detail && sameName.filter(other => modelDetail(other) === detail).length === 1;
	return `${model.name}#${unique ? detail : index}`;
}

// Headers from the first match that a client can send as-is (every header exact, no pseudo-headers).
// Authorization and Content-Type are left out, as the caller sets them.
export function modelMatchHeaders(model: LlmModel): Record<string, string> {
	const match = model.matches?.find(item =>
		(item.headers ?? []).every(
			header =>
				typeof header.value === 'object' && 'exact' in header.value && !header.name.startsWith(':')
		)
	);
	return Object.fromEntries(
		(match?.headers ?? []).flatMap(header => {
			const name = header.name.toLowerCase();
			return typeof header.value === 'object' && 'exact' in header.value && !callerHeaders.has(name)
				? [[name, header.value.exact]]
				: [];
		})
	);
}

const callerHeaders = new Set(['authorization', 'content-type']);

function headerValueText(value: HeaderValue) {
	if (typeof value === 'string') return value;
	return 'exact' in value ? value.exact : `/${value.regex}/`;
}
